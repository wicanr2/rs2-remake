//! 工具列與設定畫面共用的繪圖部件：中文點陣字型、字串表、RGBA 畫布。全部不依賴 SDL 子系統。

pub mod font;
pub mod panel;
pub mod text;

/// 整數矩形，視窗座標。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        Rect { x, y, w: (r - x).max(0), h: (b - y).max(0) }
    }

    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, ..*self }
    }

    pub fn inset(&self, d: i32) -> Rect {
        Rect { x: self.x + d, y: self.y + d, w: (self.w - 2 * d).max(0), h: (self.h - 2 * d).max(0) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
}

/// RGBA 畫布。所有繪製都夾在 `clip` 之內。
pub struct Canvas {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>,
    clip: Rect,
}

impl Canvas {
    pub fn new(w: i32, h: i32, bg: [u8; 4]) -> Canvas {
        let (w, h) = (w.max(1), h.max(1));
        let mut px = vec![0u8; (w * h * 4) as usize];
        for p in px.chunks_exact_mut(4) {
            p.copy_from_slice(&bg);
        }
        Canvas { w, h, px, clip: Rect { x: 0, y: 0, w, h } }
    }

    /// 之後的繪製只改 `r` 之內的像素；`None` 取消夾制。
    pub fn set_clip(&mut self, r: Option<Rect>) {
        let full = Rect { x: 0, y: 0, w: self.w, h: self.h };
        self.clip = r.map(|r| r.intersect(&full)).unwrap_or(full);
    }

    /// 填色。alpha 為 255 直接覆蓋，否則疊在原色上。
    pub fn rect(&mut self, r: Rect, c: [u8; 4]) {
        let r = r.intersect(&self.clip);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let o = ((y * self.w + x) * 4) as usize;
                if c[3] == 255 {
                    self.px[o..o + 4].copy_from_slice(&c);
                } else {
                    let a = c[3] as u32;
                    for i in 0..3 {
                        self.px[o + i] = ((c[i] as u32 * a + self.px[o + i] as u32 * (255 - a) + 127) / 255) as u8;
                    }
                    self.px[o + 3] = 255;
                }
            }
        }
    }

    /// 外框，粗 `t`。
    pub fn frame(&mut self, r: Rect, t: i32, c: [u8; 4]) {
        self.rect(Rect { h: t, ..r }, c);
        self.rect(Rect { y: r.bottom() - t, h: t, ..r }, c);
        self.rect(Rect { w: t, ..r }, c);
        self.rect(Rect { x: r.right() - t, w: t, ..r }, c);
    }

    /// 在 `(x, y)` 畫文字，`y` 是字框頂端，字高 `h`。
    pub fn text(&mut self, x: i32, y: i32, s: &str, h: i32, c: [u8; 4]) {
        font::builtin().draw(&mut self.px, self.w, self.h, self.clip, x, y, s, h, c);
    }

    pub fn text_width(&self, s: &str, h: i32) -> i32 {
        font::builtin().text_width(s, h)
    }

    /// 文字放進 `r`：垂直置中，左右各留 `pad`，放不下就截斷加 `…`。
    pub fn text_in(&mut self, r: Rect, s: &str, h: i32, pad: i32, align: Align, c: [u8; 4]) {
        let f = font::builtin();
        let avail = (r.w - 2 * pad).max(0);
        let t = f.fit(s, avail, h);
        let tw = f.text_width(&t, h);
        let x = match align {
            Align::Left => r.x + pad,
            Align::Center => r.x + (r.w - tw) / 2,
        };
        let saved = self.clip;
        self.clip = saved.intersect(&r);
        self.text(x, r.y + (r.h - h) / 2, &t, h, c);
        self.clip = saved;
    }
}

/// 一小塊文字標示（例如工具列收起來時的「已暫停」），半透明深色底。回傳 RGBA 與寬高。
pub fn banner(s: &str, fh: i32) -> (Vec<u8>, i32, i32) {
    let pad = fh / 2;
    let w = font::builtin().text_width(s, fh) + 2 * pad;
    let h = fh + pad;
    let mut c = Canvas::new(w, h, [14, 20, 34, 235]);
    c.text(pad, pad / 2, s, fh, crate::toolbar::INK_WARN);
    (c.px, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_is_sized_to_its_text() {
        let (px, w, h) = banner(text::ST_PAUSED, 24);
        assert_eq!(px.len(), (w * h * 4) as usize);
        assert!(w > font::builtin().text_width(text::ST_PAUSED, 24) && h > 24);
        assert!(px.chunks_exact(4).any(|p| p[0] > 200 && p[3] > 200), "要有字");
        assert!(px.chunks_exact(4).all(|p| p[3] >= 200), "底是半透明但夠深");
    }

    #[test]
    fn rect_helpers() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), Rect::new(5, 5, 5, 5));
        assert_eq!(a.intersect(&Rect::new(20, 20, 5, 5)).w, 0);
        assert!(a.contains(0, 0) && a.contains(9, 9) && !a.contains(10, 5) && !a.contains(-1, 0));
        assert_eq!(a.inset(2), Rect::new(2, 2, 6, 6));
        assert_eq!(a.inset(9).w, 0);
        assert_eq!((a.right(), a.bottom()), (10, 10));
    }

    #[test]
    fn canvas_rect_blends_and_clips() {
        let mut c = Canvas::new(10, 10, [0, 0, 0, 255]);
        c.rect(Rect::new(-5, -5, 20, 20), [100, 100, 100, 255]);
        assert!(c.px.chunks_exact(4).all(|p| p == [100, 100, 100, 255]));
        c.set_clip(Some(Rect::new(2, 2, 3, 3)));
        c.rect(Rect::new(0, 0, 10, 10), [200, 0, 0, 255]);
        let red = c.px.chunks_exact(4).filter(|p| p[0] == 200).count();
        assert_eq!(red, 9);
        c.set_clip(None);
        c.rect(Rect::new(0, 0, 1, 1), [255, 255, 255, 128]);
        assert!(c.px[0] > 170 && c.px[0] < 190 && c.px[3] == 255);
    }

    #[test]
    fn text_in_truncates_and_stays_inside_the_rect() {
        let mut c = Canvas::new(120, 40, [0, 0, 0, 255]);
        let r = Rect::new(10, 5, 80, 30);
        c.text_in(r, "這是一段很長很長的說明文字", 20, 4, Align::Left, [255, 255, 255, 255]);
        for y in 0..40 {
            for x in 0..120 {
                let o = ((y * 120 + x) * 4) as usize;
                if !r.contains(x, y) {
                    assert_eq!(&c.px[o..o + 3], &[0, 0, 0], "({x},{y}) 超出矩形");
                }
            }
        }
        assert!(c.px.chunks_exact(4).any(|p| p[0] > 0));
    }

    // -----------------------------------------------------------------------
    // 規格 005 驗收 E6：介面上的每個字都有點陣。
    // -----------------------------------------------------------------------

    /// Rust 原始碼裡的字串與字元常值（跳脫已還原），略過註解與 `#[cfg(test)]` 之後的內容。
    fn literals(src: &str) -> Vec<String> {
        let src = src.split("#[cfg(test)]").next().unwrap_or(src);
        let cs: Vec<char> = src.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        let read = |mut i: usize, quote: char| -> (String, usize) {
            let mut s = String::new();
            while i < cs.len() {
                let c = cs[i];
                if c == quote {
                    return (s, i + 1);
                }
                if c == '\\' {
                    i += 1;
                    match cs[i] {
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        '0' => s.push('\0'),
                        'u' => {
                            let end = (i..cs.len()).find(|&j| cs[j] == '}').expect("\\u{...}");
                            let hex: String = cs[i + 2..end].iter().collect();
                            s.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                            i = end;
                        }
                        '\n' => {
                            while i + 1 < cs.len() && cs[i + 1].is_whitespace() {
                                i += 1;
                            }
                        }
                        c => s.push(c),
                    }
                    i += 1;
                } else {
                    s.push(c);
                    i += 1;
                }
            }
            panic!("字串沒有結尾");
        };
        while i < cs.len() {
            if cs[i] == '/' && cs.get(i + 1) == Some(&'/') {
                while i < cs.len() && cs[i] != '\n' {
                    i += 1;
                }
            } else if cs[i] == '/' && cs.get(i + 1) == Some(&'*') {
                while i + 1 < cs.len() && !(cs[i] == '*' && cs[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            } else if cs[i] == 'r' && matches!(cs.get(i + 1), Some('"') | Some('#')) && (i == 0 || !(cs[i - 1].is_alphanumeric() || cs[i - 1] == '_')) {
                panic!("介面檔不能用 raw string，掃描器不支援");
            } else if cs[i] == '"' {
                let (s, n) = read(i + 1, '"');
                out.push(s);
                i = n;
            } else if cs[i] == '\'' {
                if cs.get(i + 1) == Some(&'\\') {
                    let (s, n) = read(i + 1, '\'');
                    out.push(s);
                    i = n;
                } else if cs.get(i + 2) == Some(&'\'') {
                    out.push(cs[i + 1].to_string());
                    i += 3;
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        out
    }

    #[test]
    fn literal_scanner_handles_escapes_comments_and_lifetimes() {
        let src = "// \"不算\"\nconst A: &str = \"a\\\"b\\u{9ede}\"; fn f<'a>(x: &'a str) { let c = '分'; let d = '\\''; } /* \"也不算\" */ const B: &str = \"手把\"; #[cfg(test)] mod t { const C: &str = \"測試\"; }";
        let l = literals(src);
        assert_eq!(l, vec!["a\"b點", "分", "'", "手把"]);
    }

    /// 掃描介面檔：非 ASCII 的字串只能在 `ui/text.rs`；全部字串的每個字都要有點陣。
    #[test]
    fn e6_every_ui_character_has_a_bitmap() {
        // keymap.rs 與 settings.rs 的中文是設定檔載入警告（寫到 stderr），不是介面，所以不掃。介面用的標籤都在 text.rs。
        let files: [(&str, &str); 7] = [
            ("ui/text.rs", include_str!("text.rs")),
            ("ui/panel.rs", include_str!("panel.rs")),
            ("toolbar.rs", include_str!("../toolbar.rs")),
            ("touch.rs", include_str!("../touch.rs")),
            ("controls.rs", include_str!("../controls.rs")),
            ("options.rs", include_str!("../options.rs")),
            ("pad.rs", include_str!("../pad.rs")),
        ];
        let f = font::builtin();
        let mut total = 0;
        for (name, src) in files {
            for s in literals(src) {
                total += 1;
                let missing = f.missing(&s);
                assert!(missing.is_empty(), "{name} 的字串 {s:?} 缺點陣：{missing:?}，重跑 tools/ui_font_bake.py");
                if name != "ui/text.rs" {
                    assert!(s.is_ascii(), "{name} 有非 ASCII 字串 {s:?}，介面字串要放進 ui/text.rs");
                }
            }
        }
        assert!(total > 100, "掃到的字串太少（{total}），掃描器可能壞了");
    }

    /// 執行時組出來的字串（格式化函式）也要有點陣。
    #[test]
    fn e6_formatted_strings_have_bitmaps() {
        use crate::ui::text as t;
        let f = font::builtin();
        let mut all: Vec<String> = Vec::new();
        for p in 0..2 {
            all.push(t::pad_added("Xbox Controller"));
            all.push(t::pad_unsupported_added("X"));
            all.push(t::pad_removed("X", Some(p)));
            all.push(t::pad_removed("X", None));
            all.push(t::pad_assign_button(p, true));
            all.push(t::pad_assign_button(p, false));
            all.push(t::pad_unsupported_name("X"));
            all.push(t::pad_player_line(p, Some("X")));
            all.push(t::pad_player_line(p, None));
            all.push(t::msg_pad_assigned("X", p));
            all.push(t::msg_pad_taken("X", p));
            all.push(t::msg_pad_unassigned(p));
            all.push(t::msg_pad_unsupported("X"));
            all.push(t::kb_head(p));
            all.push(t::msg_kb_conflict("A", p, t::ACT_UP));
            all.push(t::msg_kb_set(p, t::ACT_UP, "A"));
            all.push(t::msg_kb_cleared(p, t::ACT_UP));
            all.push(t::player_n(p + 1));
        }
        all.push(t::msg_kb_reserved("F1"));
        all.push(t::msg_padkey_taken("A", t::PADFN_FIRE));
        all.push(t::msg_padkey_set(t::PADFN_FIRE, 1, "A"));
        all.push(t::msg_padkey_cleared(t::PADFN_FIRE, 0));
        all.push(t::msg_in_development(t::OPT_FIELD_SIZE));
        for n in 1..=8 {
            all.push(t::scale_value(n));
        }
        for (k, _, label) in t::KEYS {
            all.push(label.to_string());
            all.push(t::key_label(*k));
        }
        for s in &all {
            assert!(f.missing(s).is_empty(), "{s:?} 缺點陣 {:?}", f.missing(s));
        }
    }
}
