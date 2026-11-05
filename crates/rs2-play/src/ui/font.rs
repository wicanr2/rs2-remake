//! 介面字型：讀 `assets/ui-font.bin`（Noto Sans CJK TC 的子集點陣，格式見 `tools/ui_font_bake.py`），
//! 可指定字高畫字，不依賴 SDL 子系統。
//!
//! - 字高 `h` 是字框邊長（像素），CJK 字符的筆畫落在字框內，字框頂端到基線是 0.88 `h`。
//! - 內附 16、20、24、32 四種字高的點陣。字高剛好等於其中一種時 1:1 貼上，最清楚。
//!   其他字高取不小於 `h` 的最小一種縮小（區域平均），比最大的 32 還大就用 32 放大（雙線性）。
//! - 沒有點陣的字畫成 `?`。介面字串都有點陣，由測試掃過原始碼確認（規格 005 驗收 E6）。

use crate::ui::Rect;
use std::collections::HashMap;
use std::sync::OnceLock;

const DATA: &[u8] = include_bytes!("../../assets/ui-font.bin");

#[derive(Debug)]
struct Glyph {
    adv: i32,
    bx: i32,
    by: i32,
    w: i32,
    h: i32,
    alpha: Vec<u8>,
}

#[derive(Debug)]
struct SizeTable {
    px: i32,
    ascent: i32,
    glyphs: Vec<Glyph>,
}

#[derive(Debug)]
pub struct Font {
    index: HashMap<char, usize>,
    sizes: Vec<SizeTable>,
}

/// 內附字型，第一次用時解析。
pub fn builtin() -> &'static Font {
    static F: OnceLock<Font> = OnceLock::new();
    F.get_or_init(|| Font::parse(DATA).expect("assets/ui-font.bin 損壞，重跑 tools/ui_font_bake.py"))
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.p.checked_add(n).filter(|e| *e <= self.d.len()).ok_or("字型檔被截斷")?;
        let s = &self.d[self.p..end];
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

fn unrle(data: &[u8], total: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(total);
    let mut i = 0;
    while out.len() < total {
        let c = *data.get(i).ok_or("點陣資料被截斷")?;
        i += 1;
        if c < 0x80 {
            out.resize(out.len() + c as usize + 1, 0);
        } else {
            let k = (c - 0x80) as usize + 1;
            out.extend_from_slice(data.get(i..i + k).ok_or("點陣資料被截斷")?);
            i += k;
        }
    }
    if out.len() != total {
        return Err("點陣資料長度不符".into());
    }
    Ok(out)
}

impl Font {
    pub fn parse(d: &[u8]) -> Result<Font, String> {
        let mut r = Reader { d, p: 0 };
        if r.take(4)? != b"RS2F" {
            return Err("不是 RS2F 字型檔".into());
        }
        let ver = r.u8()?;
        if ver != 1 {
            return Err(format!("字型檔版本 {ver} 不支援"));
        }
        let nsizes = r.u8()? as usize;
        let nchars = r.u16()? as usize;
        let mut cps = Vec::with_capacity(nchars);
        for _ in 0..nchars {
            let cp = r.u32()?;
            cps.push(char::from_u32(cp).ok_or("字型檔有不合法的碼位")?);
        }
        let mut sizes = Vec::with_capacity(nsizes);
        for _ in 0..nsizes {
            let px = r.u8()? as i32;
            let ascent = r.u8()? as i32;
            r.u16()?;
            let blob_len = r.u32()? as usize;
            let mut recs = Vec::with_capacity(nchars);
            for _ in 0..nchars {
                let adv = r.u8()? as i32;
                let bx = r.u8()? as i8 as i32;
                let by = r.u8()? as i8 as i32;
                let w = r.u8()? as i32;
                let h = r.u8()? as i32;
                let off = r.u32()? as usize;
                recs.push((adv, bx, by, w, h, off));
            }
            let blob = r.take(blob_len)?;
            let mut glyphs = Vec::with_capacity(nchars);
            for (adv, bx, by, w, h, off) in recs {
                let total = (w * h) as usize;
                let alpha = if total == 0 { Vec::new() } else { unrle(blob.get(off..).ok_or("點陣位移超出範圍")?, total)? };
                glyphs.push(Glyph { adv, bx, by, w, h, alpha });
            }
            sizes.push(SizeTable { px, ascent, glyphs });
        }
        if sizes.is_empty() || !sizes.windows(2).all(|w| w[0].px < w[1].px) {
            return Err("字高必須由小到大且至少一種".into());
        }
        let index = cps.iter().enumerate().map(|(i, c)| (*c, i)).collect();
        Ok(Font { index, sizes })
    }

    pub fn has(&self, c: char) -> bool {
        self.index.contains_key(&c)
    }

    /// `text` 裡沒有點陣的字（不重複，依出現順序）。控制字元不算。
    pub fn missing(&self, text: &str) -> Vec<char> {
        let mut out = Vec::new();
        for c in text.chars() {
            if !c.is_control() && !self.has(c) && !out.contains(&c) {
                out.push(c);
            }
        }
        out
    }

    /// 內附的字高。
    pub fn baked_heights(&self) -> Vec<i32> {
        self.sizes.iter().map(|s| s.px).collect()
    }

    fn table_for(&self, h: i32) -> &SizeTable {
        self.sizes.iter().find(|s| s.px >= h).unwrap_or_else(|| &self.sizes[self.sizes.len() - 1])
    }

    fn glyph<'a>(&self, t: &'a SizeTable, c: char) -> &'a Glyph {
        let i = self.index.get(&c).or_else(|| self.index.get(&'?')).copied().unwrap_or(0);
        &t.glyphs[i]
    }

    /// 一段文字的寬度（像素），字高 `h`。
    pub fn text_width(&self, text: &str, h: i32) -> i32 {
        let t = self.table_for(h);
        let r = h as f32 / t.px as f32;
        let mut pen = 0.0f32;
        for c in text.chars() {
            pen += self.glyph(t, c).adv as f32 * r;
        }
        pen.ceil() as i32
    }

    /// 在寬 `max_w` 內放得下的前綴，放不下時尾端換成 `…`。
    pub fn fit(&self, text: &str, max_w: i32, h: i32) -> String {
        if self.text_width(text, h) <= max_w {
            return text.to_string();
        }
        let mut out = String::new();
        for c in text.chars() {
            let mut next = out.clone();
            next.push(c);
            next.push('…');
            if self.text_width(&next, h) > max_w {
                break;
            }
            out.push(c);
        }
        out.push('…');
        out
    }

    /// 依寬度折行。CJK 字隨處可斷，英文與數字盡量在空白處斷。換行符號強制斷行。
    pub fn wrap(&self, text: &str, max_w: i32, h: i32) -> Vec<String> {
        let mut lines = Vec::new();
        for para in text.split('\n') {
            let mut cur = String::new();
            let mut last_break: Option<usize> = None; // cur 裡可以在這個位元組位置之後斷開
            for c in para.chars() {
                let mut probe = cur.clone();
                probe.push(c);
                if self.text_width(&probe, h) > max_w && !cur.is_empty() {
                    let ascii_word = c.is_ascii_alphanumeric() && cur.chars().last().is_some_and(|l| l.is_ascii_alphanumeric());
                    if ascii_word {
                        if let Some(b) = last_break {
                            let rest = cur[b..].to_string();
                            cur.truncate(b);
                            lines.push(cur.trim_end().to_string());
                            cur = rest;
                            last_break = None;
                            cur.push(c);
                            continue;
                        }
                    }
                    lines.push(cur.trim_end().to_string());
                    cur = String::new();
                    last_break = None;
                    if c == ' ' {
                        continue;
                    }
                }
                cur.push(c);
                if c == ' ' || !c.is_ascii() {
                    last_break = Some(cur.len());
                }
            }
            lines.push(cur.trim_end().to_string());
        }
        lines
    }

    /// 在 RGBA 緩衝區（`bw × bh`）的 `(x, y)` 畫文字，`y` 是字框頂端。只改 `clip` 之內的像素。
    /// 顏色的 alpha 與字的覆蓋率相乘後，用非預乘的方式疊在目的像素上。
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, buf: &mut [u8], bw: i32, bh: i32, clip: Rect, x: i32, y: i32, text: &str, h: i32, rgba: [u8; 4]) {
        let t = self.table_for(h);
        let r = h as f32 / t.px as f32;
        let baseline = y + (t.ascent as f32 * r).round() as i32;
        let clip = clip.intersect(&Rect { x: 0, y: 0, w: bw, h: bh });
        if clip.w <= 0 || clip.h <= 0 {
            return;
        }
        let mut pen = 0.0f32;
        for c in text.chars() {
            let g = self.glyph(t, c);
            let ox = x + pen.round() as i32;
            pen += g.adv as f32 * r;
            if g.w == 0 || g.h == 0 {
                continue;
            }
            if (r - 1.0).abs() < 1e-6 {
                blit(buf, bw, clip, ox + g.bx, baseline - g.by, g, rgba);
            } else {
                blit_scaled(buf, bw, clip, ox, baseline, g, r, rgba);
            }
        }
    }
}

fn put(buf: &mut [u8], bw: i32, x: i32, y: i32, cover: u32, rgba: [u8; 4]) {
    let sa = cover * rgba[3] as u32 / 255;
    if sa == 0 {
        return;
    }
    let o = ((y * bw + x) * 4) as usize;
    let da = buf[o + 3] as u32;
    if da == 255 {
        for i in 0..3 {
            buf[o + i] = ((rgba[i] as u32 * sa + buf[o + i] as u32 * (255 - sa) + 127) / 255) as u8;
        }
        return;
    }
    let out_a = sa * 255 + da * (255 - sa);
    for i in 0..3 {
        buf[o + i] = ((rgba[i] as u32 * sa * 255 + buf[o + i] as u32 * da * (255 - sa)) / out_a) as u8;
    }
    buf[o + 3] = ((out_a + 127) / 255) as u8;
}

fn blit(buf: &mut [u8], bw: i32, clip: Rect, gx: i32, gy: i32, g: &Glyph, rgba: [u8; 4]) {
    for row in 0..g.h {
        let y = gy + row;
        if y < clip.y || y >= clip.y + clip.h {
            continue;
        }
        for col in 0..g.w {
            let x = gx + col;
            if x < clip.x || x >= clip.x + clip.w {
                continue;
            }
            let a = g.alpha[(row * g.w + col) as usize] as u32;
            if a != 0 {
                put(buf, bw, x, y, a, rgba);
            }
        }
    }
}

/// 縮放貼上。`ox` 是筆位置，`baseline` 是基線的 y。
#[allow(clippy::too_many_arguments)]
fn blit_scaled(buf: &mut [u8], bw: i32, clip: Rect, ox: i32, baseline: i32, g: &Glyph, r: f32, rgba: [u8; 4]) {
    let x0 = (g.bx as f32 * r).floor() as i32;
    let x1 = ((g.bx + g.w) as f32 * r).ceil() as i32;
    let y0 = (-g.by as f32 * r).floor() as i32;
    let y1 = ((-g.by + g.h) as f32 * r).ceil() as i32;
    let at = |sx: i32, sy: i32| -> f32 {
        if sx < 0 || sy < 0 || sx >= g.w || sy >= g.h { 0.0 } else { g.alpha[(sy * g.w + sx) as usize] as f32 }
    };
    for dy in y0..y1 {
        let y = baseline + dy;
        if y < clip.y || y >= clip.y + clip.h {
            continue;
        }
        for dx in x0..x1 {
            let x = ox + dx;
            if x < clip.x || x >= clip.x + clip.w {
                continue;
            }
            // 目的像素 (dx, dy) 對應的來源區間（來源點陣座標）。
            let sxa = dx as f32 / r - g.bx as f32;
            let sxb = (dx + 1) as f32 / r - g.bx as f32;
            let sya = dy as f32 / r + g.by as f32;
            let syb = (dy + 1) as f32 / r + g.by as f32;
            let v = if r < 1.0 {
                // 縮小：區域平均。
                let (ix0, ix1) = (sxa.floor() as i32, sxb.ceil() as i32);
                let (iy0, iy1) = (sya.floor() as i32, syb.ceil() as i32);
                let (mut sum, mut wsum) = (0.0f32, 0.0f32);
                for sy in iy0..iy1 {
                    let wy = (syb.min((sy + 1) as f32) - sya.max(sy as f32)).max(0.0);
                    for sx in ix0..ix1 {
                        let wx = (sxb.min((sx + 1) as f32) - sxa.max(sx as f32)).max(0.0);
                        sum += at(sx, sy) * wx * wy;
                        wsum += wx * wy;
                    }
                }
                if wsum > 0.0 { sum / wsum } else { 0.0 }
            } else {
                // 放大：雙線性，取目的像素中心。
                let cx = (sxa + sxb) * 0.5 - 0.5;
                let cy = (sya + syb) * 0.5 - 0.5;
                let (fx, fy) = (cx.floor(), cy.floor());
                let (tx, ty) = (cx - fx, cy - fy);
                let (ix, iy) = (fx as i32, fy as i32);
                let top = at(ix, iy) * (1.0 - tx) + at(ix + 1, iy) * tx;
                let bot = at(ix, iy + 1) * (1.0 - tx) + at(ix + 1, iy + 1) * tx;
                top * (1.0 - ty) + bot * ty
            };
            let a = v.round().clamp(0.0, 255.0) as u32;
            if a != 0 {
                put(buf, bw, x, y, a, rgba);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_text_chars() -> String {
        // 用 ui::text 的全部字串常值驗證在 ui/tests 裡做；這裡只量字型本身。
        (0x20u8..0x7F).map(|b| b as char).collect()
    }

    #[test]
    fn parses_and_has_all_ascii_and_four_heights() {
        let f = builtin();
        assert_eq!(f.baked_heights(), vec![16, 20, 24, 32]);
        assert!(f.missing(&all_text_chars()).is_empty());
        assert!(f.has('設') && f.has('手') && f.has('把') && f.has('中'));
        assert!(!f.has('龘'));
    }

    #[test]
    fn widths_scale_with_height_and_cjk_is_one_em() {
        let f = builtin();
        for h in [16, 20, 24, 32] {
            assert_eq!(f.text_width("手把", h), 2 * h, "全形字寬等於字高 {h}");
        }
        // 非內附字高：縮放後仍接近字高。
        let w = f.text_width("手把按鍵", 28);
        assert!((4 * 28 - 4..=4 * 28 + 4).contains(&w), "{w}");
        assert_eq!(f.text_width("", 24), 0);
        assert!(f.text_width("Tab", 24) < f.text_width("手把按", 24));
    }

    fn ink_bbox(buf: &[u8], w: i32, h: i32) -> Option<(i32, i32, i32, i32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for y in 0..h {
            for x in 0..w {
                if buf[((y * w + x) * 4 + 3) as usize] != 0 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        (x0 != i32::MAX).then_some((x0, y0, x1, y1))
    }

    #[test]
    fn glyph_ink_stays_inside_the_em_box_at_every_height() {
        let f = builtin();
        for h in [14, 16, 18, 20, 24, 28, 32, 40, 48] {
            let (bw, bh) = (h * 4, h * 3);
            let mut buf = vec![0u8; (bw * bh * 4) as usize];
            let clip = Rect { x: 0, y: 0, w: bw, h: bh };
            // 畫在 (h, h)，字框是 [h, 2h)。筆畫最多超出 1 像素（反鋸齒與縮放）。
            f.draw(&mut buf, bw, bh, clip, h, h, "設定畫", h, [255, 255, 255, 255]);
            let (x0, y0, x1, y1) = ink_bbox(&buf, bw, bh).expect("有墨跡");
            assert!(x0 >= h - 1 && x1 <= h + 3 * h, "{h}: x {x0}..{x1}");
            assert!(y0 >= h - 1 && y1 <= 2 * h, "{h}: y {y0}..{y1}（字框 {h}..{})", 2 * h);
            assert!(y1 - y0 >= h * 7 / 10, "{h}: 字高 {}", y1 - y0 + 1);
        }
    }

    #[test]
    fn exact_height_draws_without_blur_and_scaled_height_is_smooth() {
        let f = builtin();
        let clip = Rect { x: 0, y: 0, w: 100, h: 60 };
        let mut a = vec![0u8; 100 * 60 * 4];
        f.draw(&mut a, 100, 60, clip, 2, 2, "中", 32, [255, 255, 255, 255]);
        // 總覆蓋量（alpha 總和）。縮放不改變筆畫的粗細比例，所以總覆蓋量約隨面積（字高平方）變化。
        let cover = |v: &Vec<u8>| v.chunks_exact(4).map(|p| p[3] as f64).sum::<f64>();
        let ink = cover(&a);
        assert!(ink > 10_000.0, "{ink}");
        // 放大 1.25 倍與縮小 0.875 倍都有輸出。
        let mut b = vec![0u8; 100 * 60 * 4];
        f.draw(&mut b, 100, 60, clip, 2, 2, "中", 40, [255, 255, 255, 255]);
        let mut c = vec![0u8; 100 * 60 * 4];
        f.draw(&mut c, 100, 60, clip, 2, 2, "中", 28, [255, 255, 255, 255]);
        let (rb, rc) = (cover(&b) / ink, cover(&c) / ink);
        assert!((1.35..1.75).contains(&rb) && (0.65..0.9).contains(&rc), "{rb} {rc}（期望約 1.56 與 0.77）");
    }

    #[test]
    fn unknown_chars_fall_back_to_question_mark_and_clip_is_respected() {
        let f = builtin();
        assert_eq!(f.missing("龘A龘"), vec!['龘']);
        let (w, h) = (40, 30);
        let mut q = vec![0u8; (w * h * 4) as usize];
        let mut u = vec![0u8; (w * h * 4) as usize];
        let clip = Rect { x: 0, y: 0, w, h };
        f.draw(&mut q, w, h, clip, 2, 2, "?", 20, [255, 255, 255, 255]);
        f.draw(&mut u, w, h, clip, 2, 2, "龘", 20, [255, 255, 255, 255]);
        assert_eq!(q, u);
        // 夾在小矩形：外面一個像素都不能動。
        let mut c = vec![0u8; (w * h * 4) as usize];
        let small = Rect { x: 5, y: 5, w: 6, h: 6 };
        f.draw(&mut c, w, h, small, 0, 0, "設定", 24, [255, 255, 255, 255]);
        for y in 0..h {
            for x in 0..w {
                if !(5..11).contains(&x) || !(5..11).contains(&y) {
                    assert_eq!(c[((y * w + x) * 4 + 3) as usize], 0, "({x},{y}) 在夾制框外");
                }
            }
        }
        // 畫在緩衝區之外也不 panic。
        f.draw(&mut c, w, h, clip, -100, -100, "設定畫", 32, [1, 2, 3, 4]);
        f.draw(&mut c, w, h, clip, 100, 100, "設定畫", 32, [1, 2, 3, 4]);
    }

    #[test]
    fn blending_over_opaque_and_transparent_backgrounds() {
        let f = builtin();
        let clip = Rect { x: 0, y: 0, w: 30, h: 30 };
        let mut opaque = vec![0u8; 30 * 30 * 4];
        for p in opaque.chunks_exact_mut(4) {
            p.copy_from_slice(&[10, 20, 30, 255]);
        }
        f.draw(&mut opaque, 30, 30, clip, 1, 1, "中", 24, [250, 200, 100, 255]);
        assert!(opaque.chunks_exact(4).all(|p| p[3] == 255), "不透明底不能變透明");
        assert!(opaque.chunks_exact(4).any(|p| p[0] > 200));
        let mut clear = vec![0u8; 30 * 30 * 4];
        f.draw(&mut clear, 30, 30, clip, 1, 1, "中", 24, [250, 200, 100, 255]);
        // 全覆蓋的像素保持原色，不被自己的 alpha 變暗。
        assert!(clear.chunks_exact(4).any(|p| p[3] == 255 && p[..3] == [250, 200, 100]));
    }

    #[test]
    fn fit_and_wrap_respect_the_width() {
        let f = builtin();
        let t = "尚未偵測到手把。USB／藍牙配對請由作業系統完成。";
        let w = 200;
        let s = f.fit(t, w, 20);
        assert!(f.text_width(&s, 20) <= w && s.ends_with('…'));
        assert_eq!(f.fit("手把", w, 20), "手把");
        for line in f.wrap(t, w, 20) {
            assert!(f.text_width(&line, 20) <= w, "{line}");
        }
        assert_eq!(f.wrap(t, w, 20).concat().replace(' ', ""), t.replace(' ', ""));
        let words = f.wrap("alpha beta gamma delta epsilon", 90, 16);
        assert!(words.len() > 1);
        for l in &words {
            assert!(f.text_width(l, 16) <= 90 && !l.starts_with(' ') && !l.ends_with(' '), "{l:?}");
        }
        assert_eq!(f.wrap("a\nb", 100, 16), vec!["a", "b"]);
    }

    #[test]
    fn corrupt_font_is_rejected() {
        assert!(Font::parse(b"XXXX").is_err());
        assert!(Font::parse(&DATA[..100]).is_err());
        let mut bad = DATA.to_vec();
        bad[4] = 9;
        assert!(Font::parse(&bad).unwrap_err().contains("版本"));
    }
}
