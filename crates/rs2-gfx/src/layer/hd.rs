//! HD 引擎：同一批繪製命令合成 RGBA，座標乘 4。
//!
//! 記憶體是四塊畫布：顯示頁 0、1（1280×800）與暫存區 A、B（2560 寬、310 邏輯列）。
//! 顯示頁永遠不透明（alpha 255）；暫存區保留 HD 圖的 alpha，因為原版的「色號 0」在 OR 時等於不動，
//! 換成 RGBA 就是透明，覆寫時才變成黑色。
//!
//! 語意（規格 001「繪製模型」與 `docs/re/12-stages-presentation.md` 第 2 節）：
//! - AND 遮罩：目的乘上 HD 遮罩（255 保留，0 清掉）。
//! - OR：HD 圖以 alpha 疊上。覆寫：先當成不透明黑底，再以 alpha 疊上（原版覆寫連黑色都蓋掉底下）。
//! - 閃白：依遮罩反相的比例，朝色號 15 的顏色填。
//! - 暫存區複製、位移、OR、AND、填色都乘 4。調色盤顏色在畫的當下取，已畫上的像素不會隨後來的調色盤改變。
//!
//! HD 靠人判讀，不要求和 `VgaSink` 逐像素相同：目的欄超出一列時裁掉，不像原版接到下一列。

use super::hd_assets::{HdAssets, HdImg, HdMask, Lib, SCALE};
use super::pal::Pal;
use super::Off;
use crate::assets::Assets;
use crate::font::DIGITS;
use crate::sink::scratch_src;
use rs2_core::{BlitCmd, Op, ShtGroup};

pub const S: i32 = SCALE as i32;
/// 顯示頁寬、高（HD 像素）。
pub const PW: i32 = 320 * S;
pub const PH: i32 = 200 * S;
/// 暫存區寬（640 邏輯像素）與列數（310 列，涵蓋 `A19Ah` 表的 300 多列）。
pub const SW: i32 = 640 * S;
pub const SH: i32 = 310 * S;
/// 遊戲區只有 190 列（`y > 189` 的列不畫）。
const GAME_ROWS: i32 = 190 * S;

#[derive(Clone)]
pub struct Canvas {
    pub px: Vec<u8>,
}

impl Canvas {
    pub fn new(w: usize, h: usize, fill: [u8; 4]) -> Canvas {
        let mut px = vec![0u8; w * h * 4];
        if fill != [0; 4] {
            for p in px.chunks_exact_mut(4) {
                p.copy_from_slice(&fill);
            }
        }
        Canvas { px }
    }

    pub fn empty() -> Canvas {
        Canvas { px: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.px.is_empty()
    }
}

#[derive(Clone)]
pub struct HdMem {
    pub pages: [Canvas; 2],
    pub scratch: [Canvas; 2],
}

impl HdMem {
    pub fn new() -> HdMem {
        let page = || Canvas::new(PW as usize, PH as usize, [0, 0, 0, 255]);
        let scr = || Canvas::new(SW as usize, SH as usize, [0, 0, 0, 0]);
        HdMem { pages: [page(), page()], scratch: [scr(), scr()] }
    }
}

impl Default for HdMem {
    fn default() -> Self {
        HdMem::new()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Or,
    Mov,
}

#[inline]
fn over(d: &mut [u8], s: &[u8]) {
    let a = s[3] as u32;
    if a == 0 {
        return;
    }
    if a == 255 {
        d.copy_from_slice(s);
        return;
    }
    let ia = 255 - a;
    for i in 0..3 {
        d[i] = ((s[i] as u32 * a + d[i] as u32 * ia + 127) / 255) as u8;
    }
    d[3] = 255;
}

#[inline]
fn over_black(d: &mut [u8], s: &[u8]) {
    let a = s[3] as u32;
    if a == 255 {
        d.copy_from_slice(s);
        return;
    }
    if a == 0 {
        d.copy_from_slice(&[0, 0, 0, 255]);
        return;
    }
    for i in 0..3 {
        d[i] = ((s[i] as u32 * a + 127) / 255) as u8;
    }
    d[3] = 255;
}

#[inline]
fn lum(s: &[u8]) -> u32 {
    let l = (s[0] as u32 * 77 + s[1] as u32 * 150 + s[2] as u32 * 29) >> 8;
    (l * s[3] as u32 + 127) / 255
}

#[inline]
fn scale_rgb(d: &mut [u8], f: u32) {
    for v in d.iter_mut().take(3) {
        *v = ((*v as u32 * f + 127) / 255) as u8;
    }
}

#[inline]
fn pg(c: &BlitCmd) -> usize {
    ((c.page >> 15) & 1) as usize
}

#[inline]
fn sx(v: u16) -> i32 {
    v as i16 as i32
}

/// 圖畫到顯示頁。裁切：欄 `[0, 1280)`、列 `[0, 760)`，負座標的列欄跳過。
fn blit_img(page: &mut Canvas, img: &HdImg, x0: i32, y0: i32, mode: Mode) {
    if page.is_empty() {
        return;
    }
    let xs = (-x0).max(0);
    let xe = (img.w as i32).min(PW - x0);
    if xs >= xe {
        return;
    }
    for iy in 0..img.h as i32 {
        let y = y0 + iy;
        if y < 0 {
            continue;
        }
        if y >= GAME_ROWS {
            break;
        }
        let so = iy as usize * img.w;
        let srow = &img.rgba[(so + xs as usize) * 4..(so + xe as usize) * 4];
        let drow = &mut page.px[((y * PW + x0 + xs) as usize) * 4..((y * PW + x0 + xe) as usize) * 4];
        for (d, s) in drow.chunks_exact_mut(4).zip(srow.chunks_exact(4)) {
            match mode {
                Mode::Or => over(d, s),
                Mode::Mov => over_black(d, s),
            }
        }
    }
}

fn mask_apply(page: &mut Canvas, m: &HdMask, x0: i32, y0: i32, white: Option<[u8; 3]>) {
    if page.is_empty() {
        return;
    }
    let xs = (-x0).max(0);
    let xe = (m.w as i32).min(PW - x0);
    if xs >= xe {
        return;
    }
    for iy in 0..m.h as i32 {
        let y = y0 + iy;
        if y < 0 {
            continue;
        }
        if y >= GAME_ROWS {
            break;
        }
        let so = iy as usize * m.w;
        let srow = &m.v[so + xs as usize..so + xe as usize];
        let drow = &mut page.px[((y * PW + x0 + xs) as usize) * 4..((y * PW + x0 + xe) as usize) * 4];
        for (d, &v) in drow.chunks_exact_mut(4).zip(srow) {
            match white {
                None => {
                    if v != 255 {
                        scale_rgb(d, v as u32);
                    }
                }
                Some(w) => {
                    let t = 255 - v as u32;
                    if t != 0 {
                        for i in 0..3 {
                            d[i] = ((d[i] as u32 * (255 - t) + w[i] as u32 * t + 127) / 255) as u8;
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Reg {
    /// 落在哪一頁。複製的來源指到顯示頁時不處理，編號目前只有測試讀。
    #[allow(dead_code)]
    Page(usize),
    Scratch(usize),
}

/// 64 KB 位址落在哪一塊：頁 0（40 位元組一列）、暫存區 A（80 位元組一列）、頁 1、暫存區 B。
fn addr_region(a: u16) -> (Reg, i32, i32) {
    let a = a as i32;
    match a {
        0..0x1F40 => (Reg::Page(0), a / 40, a % 40),
        0x1F40..0x8000 => (Reg::Scratch(0), (a - 0x1F40) / 80, (a - 0x1F40) % 80),
        0x8000..0x9F40 => (Reg::Page(1), (a - 0x8000) / 40, (a - 0x8000) % 40),
        _ => (Reg::Scratch(1), (a - 0x9F40) / 80, (a - 0x9F40) % 80),
    }
}

/// `sub_19ADC`：HD 圖原樣（含 alpha）寫進暫存區。奇數寬度丟掉最後一個位元組，不裁切（只受畫布邊界限制）。
fn scratch_put(sc: &mut Canvas, img: &HdImg, stride: u16, c: &BlitCmd) {
    if sc.is_empty() {
        return;
    }
    let w_eff = ((stride & !1) as usize * 8 * SCALE).min(img.w);
    let x0 = sx(c.x) * 8 * S;
    let y0 = sx(c.y) * S;
    let xs = (-x0).max(0) as usize;
    let xe = (w_eff as i32).min(SW - x0).max(0) as usize;
    if xs >= xe {
        return;
    }
    for iy in 0..img.h as i32 {
        let y = y0 + iy;
        if !(0..SH).contains(&y) {
            continue;
        }
        let so = iy as usize * img.w;
        let (d0, d1) = ((y * SW + x0 + xs as i32) as usize, (y * SW + x0 + xe as i32) as usize);
        sc.px[d0 * 4..d1 * 4].copy_from_slice(&img.rgba[(so + xs) * 4..(so + xe) * 4]);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CopyMode {
    /// 覆寫：透明處變黑。
    Over,
    Or,
    And,
}

/// 暫存區列複製到顯示頁：`sub_19BA4`、`19D08`（`shifted`）、`19DA8`（OR）、`1A09F`（AND）。
fn copy_hd(mem: &mut HdMem, c: &BlitCmd, off: Off, mode: CopyMode, shifted: bool) {
    let HdMem { pages, scratch } = mem;
    let dst = &mut pages[pg(c)];
    if dst.is_empty() {
        return;
    }
    // 位址不含 shift：負的 shift（頭目合成圖）會讓含 shift 的位址落到暫存區之外的頁，欄位移在下面另算。
    let (reg, srow, scol) = addr_region(scratch_src(c).wrapping_sub(c.shift));
    let src = match reg {
        Reg::Scratch(i) => &scratch[i],
        Reg::Page(_) => return,
    };
    if src.is_empty() {
        return;
    }
    let y0 = sx(c.y) * S + off.oy;
    let rows = sx(c.cx) * S;
    if rows <= 0 {
        return;
    }
    let x0 = sx(c.x) * 8 * S + off.ox;
    let col0 = (scol + sx(c.shift)) * 8 * S + off.sox;
    // 位移版每個位元組的計數 `shift + i` 要落在 [0, 80)，換成 HD 像素。
    let shift_dev = sx(c.shift) * 8 * S + off.sox;
    let mut klo = (-x0).max(0);
    let mut khi = PW.min(PW - x0);
    if shifted {
        klo = klo.max(-shift_dev);
        khi = khi.min(640 * S - shift_dev);
    }
    for y in y0.max(0)..(y0 + rows).min(GAME_ROWS) {
        let sr0 = srow * S + (y - y0);
        let mut k = klo;
        while k < khi {
            let sc = col0 + k;
            let sr = sr0 + sc.div_euclid(SW);
            let sc = sc.rem_euclid(SW);
            let seg = (khi - k).min(SW - sc);
            let d0 = ((y * PW + x0 + k) as usize) * 4;
            let drow = &mut dst.px[d0..d0 + seg as usize * 4];
            if (0..SH).contains(&sr) {
                let s0 = ((sr * SW + sc) as usize) * 4;
                let srow_px = &src.px[s0..s0 + seg as usize * 4];
                match mode {
                    CopyMode::Over => {
                        // 來源整段不透明（背景圖的常態）就整段複製。
                        if srow_px.chunks_exact(4).all(|p| p[3] == 255) {
                            drow.copy_from_slice(srow_px);
                        } else {
                            for (d, s) in drow.chunks_exact_mut(4).zip(srow_px.chunks_exact(4)) {
                                over_black(d, s);
                            }
                        }
                    }
                    CopyMode::Or => {
                        for (d, s) in drow.chunks_exact_mut(4).zip(srow_px.chunks_exact(4)) {
                            over(d, s);
                        }
                    }
                    CopyMode::And => {
                        for (d, s) in drow.chunks_exact_mut(4).zip(srow_px.chunks_exact(4)) {
                            // 黑白遮罩（不透明的灰階）的亮度就是灰階值，省掉加權。
                            let f = if s[3] == 255 && s[0] == s[1] && s[1] == s[2] { s[0] as u32 } else { lum(s) };
                            if f != 255 {
                                scale_rgb(d, f);
                            }
                        }
                    }
                }
            } else {
                // 暫存區以外：視為全透明黑。
                for d in drow.chunks_exact_mut(4) {
                    match mode {
                        CopyMode::Over => over_black(d, &[0, 0, 0, 0]),
                        CopyMode::Or => {}
                        CopyMode::And => scale_rgb(d, 0),
                    }
                }
            }
            k += seg;
        }
    }
}

fn fill_rect(page: &mut Canvas, x0: i32, y0: i32, x1: i32, y1: i32, rgb: [u8; 3], row_limit: i32) {
    if page.is_empty() {
        return;
    }
    for y in y0.max(0)..y1.min(row_limit) {
        for x in x0.max(0)..x1.min(PW) {
            let o = ((y * PW + x) as usize) * 4;
            page.px[o..o + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
}

/// `sub_1A287`：填實色，每列一整列寬。
fn fill_hd(mem: &mut HdMem, vis: &[[u8; 3]; 16], c: &BlitCmd, off: Off) {
    let rows = sx(c.cx) * S + off.ocx;
    if rows <= 0 {
        return;
    }
    let y0 = sx(c.y) * S + off.oy;
    let x0 = sx(c.x) * 8 * S + off.ox;
    fill_rect(&mut mem.pages[pg(c)], x0, y0, x0 + PW, y0 + rows, vis[(c.dx & 15) as usize], GAME_ROWS);
}

/// `sub_1929F`：血條的一格，7 列各 8 個像素，兩頁同寫，不裁 y 上限。
fn bar_hd(mem: &mut HdMem, vis: &[[u8; 3]; 16], c: &BlitCmd) {
    let x0 = sx(c.x) * 8 * S;
    let y0 = sx(c.y) * S;
    for p in mem.pages.iter_mut() {
        fill_rect(p, x0, y0, x0 + 8 * S, y0 + 7 * S, vis[(c.dx & 15) as usize], PH);
    }
}

/// `sub_19CF9`：清 190 列成色號 0 的顏色。
fn clear_page_hd(mem: &mut HdMem, vis: &[[u8; 3]; 16], c: &BlitCmd) {
    fill_rect(&mut mem.pages[pg(c)], 0, 0, PW, GAME_ROWS, vis[0], GAME_ROWS);
}

/// `sub_1152D`：暫存區 B 的前 `417Eh` 位元組（209 列又 46 位元組）清成透明。
fn clear_scratch_b_hd(mem: &mut HdMem) {
    let sc = &mut mem.scratch[1];
    if sc.is_empty() {
        return;
    }
    let full = (0x417E / 80) as i32 * S;
    let part = (0x417E % 80) as i32 * 8 * S;
    for y in 0..(full + S).min(SH) {
        let w = if y < full { SW } else { part };
        let o = (y * SW) as usize * 4;
        sc.px[o..o + w as usize * 4].fill(0);
    }
}

/// 數字畫在平面 3，這裡改成把字形的亮點畫成色號 8 的顏色。`both` 為真時兩頁同寫。
pub fn counter(pages: &mut [Canvas; 2], vis: &[[u8; 3]; 16], col: u16, row: u16, value: u16, both: bool) {
    let mut v = value;
    let mut digits = [0u8; 5];
    for (i, d) in [10000u16, 1000, 100, 10, 1].into_iter().enumerate() {
        digits[i] = (v / d) as u8;
        v %= d;
    }
    let rgb = vis[8];
    for (i, &dg) in digits.iter().enumerate() {
        let glyph = DIGITS[(dg % 10) as usize];
        for (r, &bits) in glyph.iter().enumerate() {
            let y = row as i32 + r as i32;
            if y >= 200 {
                continue;
            }
            for bit in 0..8i32 {
                if (bits >> (7 - bit)) & 1 == 0 {
                    continue;
                }
                let x = (col as i32 + i as i32) * 8 + bit;
                if x >= 320 {
                    continue;
                }
                for (pi, page) in pages.iter_mut().enumerate() {
                    if (!both && pi != 0) || page.is_empty() {
                        continue;
                    }
                    fill_rect(page, x * S, y * S, (x + 1) * S, (y + 1) * S, rgb, PH);
                }
            }
        }
    }
}

fn sht_lib(c: &BlitCmd) -> Lib {
    match c.sht_group {
        ShtGroup::Bh1 => Lib::Bh1,
        ShtGroup::Sht1 => Lib::Sht1,
    }
}

/// 套用一個 blit 命令。`S1890D` 的幀計數位置不在這裡處理。
pub fn apply(mem: &mut HdMem, hd: &mut HdAssets, assets: &Assets, pal: &Pal, c: &BlitCmd, off: Off) {
    let x0 = sx(c.x) * 8 * S + off.ox;
    let y0 = sx(c.y) * S + off.oy;
    let img_op = |mem: &mut HdMem, hd: &mut HdAssets, lib: Lib, mode: Mode| {
        if mem.pages[pg(c)].is_empty() {
            return;
        }
        if let Some(img) = hd.image(lib, c.index, pal, assets) {
            blit_img(&mut mem.pages[pg(c)], &img, x0, y0, mode);
        }
    };
    let mask_op = |mem: &mut HdMem, hd: &mut HdAssets, lib: Lib, flash: bool| {
        if mem.pages[pg(c)].is_empty() {
            return;
        }
        if let Some(m) = hd.mask(lib, c.index, assets) {
            let white = flash.then(|| pal.visible8()[15]);
            mask_apply(&mut mem.pages[pg(c)], &m, x0, y0, white);
        }
    };
    match c.op {
        Op::S18C0A => img_op(mem, hd, Lib::My, Mode::Mov),
        Op::S18CE2 | Op::S18EB6 | Op::S18DCC => img_op(mem, hd, Lib::My, Mode::Or),
        Op::S18D46 | Op::S18E30 | Op::S18E73 => mask_op(mem, hd, Lib::My, false),
        Op::S18D89 => mask_op(mem, hd, Lib::My, true),
        Op::S18F1A => mask_op(mem, hd, Lib::Sht1, false),
        Op::S19025 => mask_op(mem, hd, Lib::Sht1, true),
        Op::S18F5D => img_op(mem, hd, sht_lib(c), Mode::Or),
        Op::S18FC1 => img_op(mem, hd, sht_lib(c), Mode::Mov),
        Op::S19ADC => {
            let lib = sht_lib(c);
            let Some(it) = lib.go(assets).planes[0].get(c.index as usize) else { return };
            let stride = it.stride;
            if let Some(img) = hd.image(lib, c.index, pal, assets) {
                scratch_put(&mut mem.scratch[((c.scratch >> 15) & 1) as usize], &img, stride, c);
            }
        }
        Op::S19BA4 => copy_hd(mem, c, off, CopyMode::Over, false),
        Op::S19D08 => copy_hd(mem, c, off, CopyMode::Over, true),
        Op::S19DA8 => copy_hd(mem, c, off, CopyMode::Or, false),
        Op::S1A09F => copy_hd(mem, c, off, CopyMode::And, false),
        Op::S19CF9 => clear_page_hd(mem, &pal.visible8(), c),
        Op::S1A287 => fill_hd(mem, &pal.visible8(), c, off),
        Op::S1929F => bar_hd(mem, &pal.visible8(), c),
        Op::S1152D => clear_scratch_b_hd(mem),
        Op::S1890D => counter(&mut mem.pages, &pal.visible8(), c.text_x, c.text_y, c.cx, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize, px: &[[u8; 4]]) -> HdImg {
        assert_eq!(px.len(), w * h);
        HdImg { w, h, rgba: px.iter().flatten().copied().collect() }
    }

    fn page() -> Canvas {
        Canvas::new(PW as usize, PH as usize, [0, 0, 0, 255])
    }

    fn at(p: &Canvas, x: i32, y: i32) -> [u8; 4] {
        let o = ((y * PW + x) as usize) * 4;
        [p.px[o], p.px[o + 1], p.px[o + 2], p.px[o + 3]]
    }

    #[test]
    fn region_mapping_covers_the_address_space() {
        assert!(matches!(addr_region(0), (Reg::Page(0), 0, 0)));
        assert!(matches!(addr_region(0x1F3F), (Reg::Page(0), 199, 39)));
        assert!(matches!(addr_region(0x1F40), (Reg::Scratch(0), 0, 0)));
        assert!(matches!(addr_region(0x1F40 + 80 * 5 + 7), (Reg::Scratch(0), 5, 7)));
        assert!(matches!(addr_region(0x8000), (Reg::Page(1), 0, 0)));
        assert!(matches!(addr_region(0x9F40), (Reg::Scratch(1), 0, 0)));
        assert!(matches!(addr_region(0xFFFF), (Reg::Scratch(1), 309, 47)));
    }

    #[test]
    fn or_blends_and_mov_covers_with_black() {
        let mut p = page();
        p.px[..4].copy_from_slice(&[100, 100, 100, 255]);
        let im = img(2, 1, &[[200, 0, 0, 128], [0, 200, 0, 0]]);
        blit_img(&mut p, &im, 0, 0, Mode::Or);
        let a = at(&p, 0, 0);
        assert_eq!(a[3], 255);
        assert_eq!(a[0], ((200u32 * 128 + 100 * 127 + 127) / 255) as u8);
        assert_eq!(at(&p, 1, 0), [0, 0, 0, 255], "alpha 0 不動，底色是黑");
        let mut p = page();
        p.px[4..8].copy_from_slice(&[9, 9, 9, 255]);
        blit_img(&mut p, &im, 0, 0, Mode::Mov);
        assert_eq!(at(&p, 1, 0), [0, 0, 0, 255], "覆寫時透明處變黑，蓋掉底下");
    }

    #[test]
    fn clipping_rules() {
        let mut p = page();
        let im = img(2, 2, &[[255, 255, 255, 255]; 4]);
        blit_img(&mut p, &im, -1, -1, Mode::Or);
        assert_eq!(at(&p, 0, 0), [255, 255, 255, 255]);
        assert_eq!(at(&p, 1, 0), [0, 0, 0, 255]);
        blit_img(&mut p, &im, PW - 1, 0, Mode::Or);
        assert_eq!(at(&p, PW - 1, 0), [255, 255, 255, 255]);
        // 第 190 邏輯列起不畫
        blit_img(&mut p, &im, 5, GAME_ROWS - 1, Mode::Or);
        assert_eq!(at(&p, 5, GAME_ROWS - 1), [255, 255, 255, 255]);
        assert_eq!(at(&p, 5, GAME_ROWS), [0, 0, 0, 255]);
    }

    #[test]
    fn mask_and_flash() {
        let mut p = page();
        fill_rect(&mut p, 0, 0, 4, 1, [200, 100, 50], PH);
        let m = HdMask { w: 4, h: 1, v: vec![255, 128, 0, 255] };
        mask_apply(&mut p, &m, 0, 0, None);
        assert_eq!(at(&p, 0, 0)[..3], [200, 100, 50]);
        assert_eq!(at(&p, 1, 0)[0], ((200u32 * 128 + 127) / 255) as u8);
        assert_eq!(at(&p, 2, 0)[..3], [0, 0, 0]);
        let mut p = page();
        fill_rect(&mut p, 0, 0, 4, 1, [200, 100, 50], PH);
        mask_apply(&mut p, &m, 0, 0, Some([255, 255, 255]));
        assert_eq!(at(&p, 0, 0)[..3], [200, 100, 50], "255 保留背景，不閃白");
        assert_eq!(at(&p, 2, 0)[..3], [255, 255, 255], "0 填成白");
    }

    #[test]
    fn scratch_copy_roundtrip_with_wrap_and_shift() {
        let mut mem = HdMem { pages: [page(), Canvas::empty()], scratch: [Canvas::new(SW as usize, SH as usize, [0, 0, 0, 0]), Canvas::empty()] };
        // 暫存區 A 第 0 列：每個 HD 像素的 R 等於欄號低位元組，用來辨認來源欄。
        for y in 0..S {
            for x in 0..SW {
                let o = ((y * SW + x) as usize) * 4;
                mem.scratch[0].px[o..o + 4].copy_from_slice(&[(x & 0xFF) as u8, (x >> 8) as u8, 0, 255]);
            }
        }
        let mut c = BlitCmd {
            op: Op::S19BA4,
            x: 0,
            y: 0,
            index: 0,
            page: 0,
            scratch: 0,
            src_x: 0,
            src_y: 0,
            shift: 0,
            cx: 1,
            dx: 0,
            text_x: 0,
            text_y: 0,
            sht_group: ShtGroup::Bh1,
            obj: None,
        };
        copy_hd(&mut mem, &c, Off::default(), CopyMode::Over, false);
        assert_eq!(at(&mem.pages[0], 0, 0)[..2], [0, 0]);
        assert_eq!(at(&mem.pages[0], 300, 3)[..2], [(300 & 0xFF) as u8, 1]);
        // 來源欄位移 80 HD 像素（20 邏輯像素的 4 倍）：目的第 0 欄取來源第 80 欄。
        copy_hd(&mut mem, &c, Off { sox: 80, ..Off::default() }, CopyMode::Over, false);
        assert_eq!(at(&mem.pages[0], 0, 0)[0], 80);
        // 負的來源位移環繞到前一列（第 -1 列在畫布外，視為黑）。
        copy_hd(&mut mem, &c, Off { sox: -4, ..Off::default() }, CopyMode::Over, false);
        assert_eq!(at(&mem.pages[0], 0, 0), [0, 0, 0, 255]);
        assert_eq!(at(&mem.pages[0], 4, 0)[0], 0);
        // 位移版：shift = -1 位元組（-32 HD 像素）時，目的前 32 欄不寫。
        c.op = Op::S19D08;
        c.shift = 0xFFFF;
        let mut m2 = mem.clone();
        m2.pages[0] = Canvas::new(PW as usize, PH as usize, [7, 7, 7, 255]);
        copy_hd(&mut m2, &c, Off::default(), CopyMode::Over, true);
        assert_eq!(at(&m2.pages[0], 31, 0)[..3], [7, 7, 7]);
        assert_eq!(at(&m2.pages[0], 32, 0)[0], 0, "寫入的第一欄來自來源第 0 欄");
        assert_eq!(at(&m2.pages[0], 33, 0)[0], 1, "之後依序是來源第 1 欄");
    }

    #[test]
    fn and_uses_source_luminance_and_alpha() {
        let mut mem = HdMem { pages: [page(), Canvas::empty()], scratch: [Canvas::new(SW as usize, SH as usize, [0, 0, 0, 0]), Canvas::empty()] };
        fill_rect(&mut mem.pages[0], 0, 0, 8, 1, [100, 100, 100], PH);
        // 來源：欄 0 白不透明、欄 1 黑不透明、欄 2 白透明（等於沒寫）
        mem.scratch[0].px[..12].copy_from_slice(&[255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 0]);
        let c = BlitCmd {
            op: Op::S1A09F,
            x: 0,
            y: 0,
            index: 0,
            page: 0,
            scratch: 0,
            src_x: 0,
            src_y: 0,
            shift: 0,
            cx: 1,
            dx: 0,
            text_x: 0,
            text_y: 0,
            sht_group: ShtGroup::Bh1,
            obj: None,
        };
        copy_hd(&mut mem, &c, Off::default(), CopyMode::And, false);
        assert_eq!(at(&mem.pages[0], 0, 0)[..3], [100, 100, 100]);
        assert_eq!(at(&mem.pages[0], 1, 0)[..3], [0, 0, 0]);
        assert_eq!(at(&mem.pages[0], 2, 0)[..3], [0, 0, 0]);
    }

    #[test]
    fn clear_scratch_b_clears_exactly_the_original_range() {
        let mut mem = HdMem { pages: [Canvas::empty(), Canvas::empty()], scratch: [Canvas::empty(), Canvas::new(SW as usize, SH as usize, [9, 9, 9, 9])] };
        clear_scratch_b_hd(&mut mem);
        let sc = &mem.scratch[1];
        let px = |x: i32, y: i32| sc.px[((y * SW + x) as usize) * 4];
        assert_eq!(px(0, 0), 0);
        assert_eq!(px(SW - 1, 208 * S + 3), 0, "第 208 邏輯列整列清掉");
        assert_eq!(px(0, 209 * S), 0, "第 209 列前 46 位元組清掉");
        assert_eq!(px(46 * 8 * S - 1, 209 * S + 3), 0);
        assert_eq!(px(46 * 8 * S, 209 * S), 9, "第 46 位元組之後保留");
        assert_eq!(px(0, 210 * S), 9);
    }
}
