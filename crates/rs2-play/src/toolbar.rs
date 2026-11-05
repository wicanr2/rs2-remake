//! 工具列：視窗上方獨立一條，不蓋遊戲畫面。可隱藏，滑鼠可點，對應 F1 到 F4、F10。
//! 版面只由視窗寬度決定，和 SDL 無關，所以命中測試可以單元測試。文字一律 ASCII，用 `font` 的點陣。

use crate::font;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Tween,
    Counter,
    Gfx,
    Scale,
    Hide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// 工具列顯示的開關狀態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarState {
    pub tween: bool,
    pub counter: bool,
    /// 目前用 HD 圖（只有 `hd_available` 時有意義）。
    pub hd: bool,
    pub hd_available: bool,
    pub scale: u32,
}

/// 各按鈕的字元數上限（含最長的狀態文字），按鈕寬度固定，切換狀態時版面不跳動。
const BUTTONS: [(Button, usize); 5] = [
    (Button::Tween, 12),  // "F1 TWEEN OFF"
    (Button::Counter, 12), // "F2 COUNT OFF"
    (Button::Gfx, 8),      // "F3 NO HD"
    (Button::Scale, 5),    // "F4 X3"
    (Button::Hide, 8),     // "F10 HIDE"
];

const BG: [u8; 4] = [28, 30, 40, 255];
const BTN_BG: [u8; 4] = [52, 56, 76, 255];
const ON: [u8; 4] = [130, 230, 130, 255];
const OFF: [u8; 4] = [200, 200, 200, 255];
const NA: [u8; 4] = [110, 110, 120, 255];
const PLAIN: [u8; 4] = [235, 235, 245, 255];

/// 字型倍率：視窗越寬字越大（640 寬 1 倍，960 起 2 倍，1440 起 3 倍）。
pub fn font_scale(width: u32) -> usize {
    ((width / 480) as usize).max(1)
}

/// 工具列高度（像素），上下各留 3 倍率的邊。
pub fn bar_height(width: u32) -> u32 {
    ((font::GLYPH_H + 6) * font_scale(width)) as u32
}

/// 各按鈕的矩形（視窗座標，原點在工具列左上角）。
pub fn layout(width: u32) -> Vec<(Button, Rect)> {
    let fs = font_scale(width) as i32;
    let pad = 3 * fs;
    let h = bar_height(width) as i32;
    let mut x = pad;
    let mut out = Vec::with_capacity(BUTTONS.len());
    for (b, chars) in BUTTONS {
        let w = font::text_width(&"X".repeat(chars), fs as usize) as i32 + 2 * pad;
        out.push((b, Rect { x, y: 0, w, h }));
        x += w + pad;
    }
    out
}

/// 按鈕上的文字。
pub fn label(b: Button, s: &BarState) -> String {
    match b {
        Button::Tween => format!("F1 TWEEN {}", if s.tween { "ON" } else { "OFF" }),
        Button::Counter => format!("F2 COUNT {}", if s.counter { "ON" } else { "OFF" }),
        Button::Gfx => {
            if !s.hd_available {
                "F3 NO HD".to_string()
            } else if s.hd {
                "F3 HD".to_string()
            } else {
                "F3 ORIG".to_string()
            }
        }
        Button::Scale => format!("F4 X{}", s.scale),
        Button::Hide => "F10 HIDE".to_string(),
    }
}

fn label_color(b: Button, s: &BarState) -> [u8; 4] {
    match b {
        Button::Tween => if s.tween { ON } else { OFF },
        Button::Counter => if s.counter { ON } else { OFF },
        Button::Gfx => {
            if !s.hd_available {
                NA
            } else if s.hd {
                ON
            } else {
                OFF
            }
        }
        Button::Scale | Button::Hide => PLAIN,
    }
}

/// 工具列本身：只有顯示與否。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Toolbar {
    pub visible: bool,
}

impl Toolbar {
    pub fn new(visible: bool) -> Toolbar {
        Toolbar { visible }
    }

    /// 佔用的高度，隱藏時 0。
    pub fn height(&self, width: u32) -> u32 {
        if self.visible { bar_height(width) } else { 0 }
    }

    /// 視窗座標 `(x, y)` 的點擊命中哪個按鈕。隱藏時沒有命中。
    pub fn hit(&self, width: u32, x: i32, y: i32) -> Option<Button> {
        if !self.visible {
            return None;
        }
        layout(width).into_iter().find(|(_, r)| r.contains(x, y)).map(|(b, _)| b)
    }

    /// 畫成 `width × bar_height(width)` 的 RGBA。
    pub fn render(&self, width: u32, s: &BarState) -> Vec<u8> {
        let h = bar_height(width) as usize;
        let w = width as usize;
        let fs = font_scale(width);
        let mut buf = vec![0u8; w * h * 4];
        font::fill_rect(&mut buf, w, h, 0, 0, w as i32, h as i32, BG);
        for (b, r) in layout(width) {
            font::fill_rect(&mut buf, w, h, r.x, r.y + fs as i32, r.w, r.h - 2 * fs as i32, BTN_BG);
            let text = label(b, s);
            let ty = r.y + (r.h - font::text_height(fs) as i32) / 2;
            font::draw_text(&mut buf, w, h, r.x + 3 * fs as i32, ty, &text, fs, label_color(b, s));
        }
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st() -> BarState {
        BarState { tween: true, counter: false, hd: true, hd_available: true, scale: 3 }
    }

    #[test]
    fn layout_fits_every_supported_window_width() {
        for scale in 1..=8u32 {
            let width = 320 * scale;
            let l = layout(width);
            assert_eq!(l.len(), 5);
            let mut right = 0;
            for (_, r) in &l {
                assert!(r.x >= right, "按鈕重疊 {width}");
                assert!(r.w > 0 && r.h == bar_height(width) as i32);
                right = r.x + r.w;
            }
            assert!(right <= width as i32, "工具列寬 {right} 超過視窗 {width}");
        }
    }

    #[test]
    fn font_scale_grows_with_width() {
        assert_eq!(font_scale(320), 1);
        assert_eq!(font_scale(640), 1);
        assert_eq!(font_scale(960), 2);
        assert_eq!(font_scale(1280), 2);
        assert_eq!(font_scale(1600), 3);
        assert_eq!(bar_height(640), 13);
        assert_eq!(bar_height(960), 26);
    }

    #[test]
    fn hit_test_centres_gaps_and_outside() {
        let tb = Toolbar::new(true);
        let width = 960;
        let l = layout(width);
        for (b, r) in &l {
            assert_eq!(tb.hit(width, r.x + r.w / 2, r.h / 2), Some(*b));
            assert_eq!(tb.hit(width, r.x, 0), Some(*b), "左上角");
            assert_eq!(tb.hit(width, r.x + r.w - 1, r.h - 1), Some(*b), "右下角");
            assert_eq!(tb.hit(width, r.x + r.w, r.h / 2), None, "右緣之外是縫");
            assert_eq!(tb.hit(width, r.x + r.w / 2, r.h), None, "工具列下緣之外是遊戲畫面");
            assert_eq!(tb.hit(width, r.x + r.w / 2, -1), None);
        }
        assert_eq!(tb.hit(width, 0, 0), None, "左邊界是縫");
        assert_eq!(tb.hit(width, width as i32 - 1, 5), None);
    }

    #[test]
    fn buttons_map_to_the_function_keys_in_order() {
        let order: Vec<Button> = layout(960).into_iter().map(|(b, _)| b).collect();
        assert_eq!(order, vec![Button::Tween, Button::Counter, Button::Gfx, Button::Scale, Button::Hide]);
        let s = st();
        assert!(label(Button::Tween, &s).starts_with("F1 "));
        assert!(label(Button::Counter, &s).starts_with("F2 "));
        assert!(label(Button::Gfx, &s).starts_with("F3 "));
        assert!(label(Button::Scale, &s).starts_with("F4 "));
        assert!(label(Button::Hide, &s).starts_with("F10 "));
    }

    #[test]
    fn hidden_toolbar_takes_no_space_and_ignores_clicks() {
        let tb = Toolbar::new(false);
        assert_eq!(tb.height(960), 0);
        let (_, r) = layout(960)[0];
        assert_eq!(tb.hit(960, r.x + 1, 1), None);
        assert_eq!(Toolbar::new(true).height(960), 26);
    }

    #[test]
    fn labels_show_state_and_shortcut() {
        let mut s = st();
        assert_eq!(label(Button::Tween, &s), "F1 TWEEN ON");
        s.tween = false;
        assert_eq!(label(Button::Tween, &s), "F1 TWEEN OFF");
        assert_eq!(label(Button::Counter, &s), "F2 COUNT OFF");
        s.counter = true;
        assert_eq!(label(Button::Counter, &s), "F2 COUNT ON");
        assert_eq!(label(Button::Gfx, &s), "F3 HD");
        s.hd = false;
        assert_eq!(label(Button::Gfx, &s), "F3 ORIG");
        s.hd_available = false;
        assert_eq!(label(Button::Gfx, &s), "F3 NO HD");
        assert_eq!(label(Button::Scale, &s), "F4 X3");
        assert_eq!(label(Button::Hide, &s), "F10 HIDE");
    }

    #[test]
    fn every_label_fits_its_button_and_is_ascii() {
        for width in [640u32, 960, 1280, 1600] {
            let fs = font_scale(width);
            for mask in 0..16u32 {
                let s = BarState { tween: mask & 1 != 0, counter: mask & 2 != 0, hd: mask & 4 != 0, hd_available: mask & 8 != 0, scale: 5 };
                for (b, r) in layout(width) {
                    let t = label(b, &s);
                    assert!(t.is_ascii());
                    assert!(font::text_width(&t, fs) as i32 + 2 * 3 * fs as i32 <= r.w, "{t} 超出按鈕 {width}");
                }
            }
        }
    }

    #[test]
    fn render_has_expected_size_and_changes_with_state() {
        let tb = Toolbar::new(true);
        let a = tb.render(640, &st());
        assert_eq!(a.len(), 640 * 13 * 4);
        let mut s2 = st();
        s2.tween = false;
        let b = tb.render(640, &s2);
        assert_ne!(a, b);
        assert!(a.chunks_exact(4).all(|p| p[3] == 255));
    }
}
