//! 工具列：視窗上方獨立一條，不蓋遊戲畫面。可隱藏，滑鼠可點，對應 F1 到 F4、F10。
//! 版面只由視窗寬度決定，和 SDL 無關，所以命中測試可以單元測試。文字一律 ASCII，用 `font` 的點陣。

use crate::font;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Tween,
    Counter,
    Gfx,
    Scale,
    Feel,
    /// 手機版：開授權頁面。
    About,
    /// 手機版：離開。
    Quit,
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
    /// 玩家機預測與短按保留。
    pub feel: bool,
    /// 手機版的字樣，不帶 F 鍵。
    pub mobile: bool,
}

/// 各按鈕的字元數上限（含最長的狀態文字），按鈕寬度固定，切換狀態時版面不跳動。
const BUTTONS: [(Button, usize); 6] = [
    (Button::Tween, 12),   // "F1 TWEEN OFF"
    (Button::Counter, 12), // "F2 COUNT OFF"
    (Button::Gfx, 8),      // "F3 NO HD"
    (Button::Scale, 5),    // "F4 X3"
    (Button::Feel, 12),    // "F5 FEEL ORIG"
    (Button::Hide, 8),     // "F10 HIDE"
];

/// 手機版的按鈕，字數同樣含最長的狀態文字。倍率在手機上沒有用途。
const MOBILE_BUTTONS: [(Button, usize); 7] = [
    (Button::Tween, 9),   // "TWEEN OFF"
    (Button::Counter, 9), // "COUNT OFF"
    (Button::Gfx, 5),     // "NO HD"
    (Button::Feel, 9),    // "FEEL ORIG"
    (Button::About, 5),
    (Button::Quit, 4),
    (Button::Hide, 4),
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

/// 手機版的字型倍率。按鈕要手指按得到，所以比桌面版大。
pub fn mobile_font_scale(width: u32) -> usize {
    ((width / 360) as usize).max(1)
}

/// 手機版的工具列高度：寬度的 1/16，2400 寬時 150 像素，400 dpi 的手機上是 60 dp。
pub fn mobile_bar_height(width: u32) -> u32 {
    (((font::GLYPH_H + 6) * mobile_font_scale(width)) as u32).max(width / 16)
}

/// 各按鈕的矩形（視窗座標，原點在工具列左上角）。放不下的按鈕不畫，它們的功能鍵仍然有效。
pub fn layout(width: u32) -> Vec<(Button, Rect)> {
    let fs = font_scale(width) as i32;
    layout_with(width, fs, bar_height(width) as i32, &BUTTONS)
}

pub fn mobile_layout(width: u32) -> Vec<(Button, Rect)> {
    layout_with(width, mobile_font_scale(width) as i32, mobile_bar_height(width) as i32, &MOBILE_BUTTONS)
}

fn layout_with(width: u32, fs: i32, h: i32, buttons: &[(Button, usize)]) -> Vec<(Button, Rect)> {
    let pad = 3 * fs;
    let mut x = pad;
    let mut out = Vec::with_capacity(buttons.len());
    for &(b, chars) in buttons {
        let w = font::text_width(&"X".repeat(chars), fs as usize) as i32 + 2 * pad;
        if x + w > width as i32 {
            break;
        }
        out.push((b, Rect { x, y: 0, w, h }));
        x += w + pad;
    }
    out
}

/// 按鈕上的文字。
pub fn label(b: Button, s: &BarState) -> String {
    if s.mobile {
        return mobile_label(b, s);
    }
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
        Button::Feel => format!("F5 FEEL {}", if s.feel { "NEW" } else { "ORIG" }),
        Button::Hide => "F10 HIDE".to_string(),
        Button::About => "ABOUT".to_string(),
        Button::Quit => "QUIT".to_string(),
    }
}

fn mobile_label(b: Button, s: &BarState) -> String {
    let onoff = |v: bool| if v { "ON" } else { "OFF" };
    match b {
        Button::Tween => format!("TWEEN {}", onoff(s.tween)),
        Button::Counter => format!("COUNT {}", onoff(s.counter)),
        Button::Gfx => {
            if !s.hd_available {
                "NO HD".to_string()
            } else if s.hd {
                "HD".to_string()
            } else {
                "ORIG".to_string()
            }
        }
        Button::Scale => format!("X{}", s.scale),
        Button::Feel => format!("FEEL {}", if s.feel { "NEW" } else { "ORIG" }),
        Button::About => "ABOUT".to_string(),
        Button::Quit => "QUIT".to_string(),
        Button::Hide => "HIDE".to_string(),
    }
}

fn label_color(b: Button, s: &BarState) -> [u8; 4] {
    match b {
        Button::Tween => if s.tween { ON } else { OFF },
        Button::Counter => if s.counter { ON } else { OFF },
        Button::Feel => if s.feel { ON } else { OFF },
        Button::Gfx => {
            if !s.hd_available {
                NA
            } else if s.hd {
                ON
            } else {
                OFF
            }
        }
        Button::Scale | Button::Hide | Button::About | Button::Quit => PLAIN,
    }
}

/// 工具列本身：只有顯示與否。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Toolbar {
    pub visible: bool,
    pub mobile: bool,
}

impl Toolbar {
    pub fn new(visible: bool) -> Toolbar {
        Toolbar { visible, mobile: false }
    }

    pub fn mobile(visible: bool) -> Toolbar {
        Toolbar { visible, mobile: true }
    }

    /// 佔用的高度，隱藏時 0。
    pub fn height(&self, width: u32) -> u32 {
        match (self.visible, self.mobile) {
            (false, _) => 0,
            (true, false) => bar_height(width),
            (true, true) => mobile_bar_height(width),
        }
    }

    fn buttons(&self, width: u32) -> Vec<(Button, Rect)> {
        if self.mobile { mobile_layout(width) } else { layout(width) }
    }

    /// 視窗座標 `(x, y)` 的點擊命中哪個按鈕。隱藏時沒有命中。
    pub fn hit(&self, width: u32, x: i32, y: i32) -> Option<Button> {
        if !self.visible {
            return None;
        }
        self.buttons(width).into_iter().find(|(_, r)| r.contains(x, y)).map(|(b, _)| b)
    }

    /// 畫成 `width × bar_height(width)` 的 RGBA。
    pub fn render(&self, width: u32, s: &BarState) -> Vec<u8> {
        let h = if self.mobile { mobile_bar_height(width) } else { bar_height(width) } as usize;
        let w = width as usize;
        let fs = if self.mobile { mobile_font_scale(width) } else { font_scale(width) };
        let mut buf = vec![0u8; w * h * 4];
        font::fill_rect(&mut buf, w, h, 0, 0, w as i32, h as i32, BG);
        for (b, r) in self.buttons(width) {
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
        BarState { tween: true, counter: false, hd: true, hd_available: true, scale: 3, feel: true, mobile: false }
    }

    #[test]
    fn layout_fits_every_supported_window_width() {
        for scale in 1..=8u32 {
            let width = 320 * scale;
            let l = layout(width);
            // 320 寬放不下全部，留前面幾個，其餘靠功能鍵。F4 循環的最小倍率是 2，全部放得下。
            assert_eq!(l.len(), if scale == 1 { 4 } else { BUTTONS.len() }, "{width}");
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
        assert_eq!(order, vec![Button::Tween, Button::Counter, Button::Gfx, Button::Scale, Button::Feel, Button::Hide]);
        let s = st();
        assert!(label(Button::Tween, &s).starts_with("F1 "));
        assert!(label(Button::Counter, &s).starts_with("F2 "));
        assert!(label(Button::Gfx, &s).starts_with("F3 "));
        assert!(label(Button::Scale, &s).starts_with("F4 "));
        assert!(label(Button::Feel, &s).starts_with("F5 "));
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
        assert_eq!(label(Button::Feel, &s), "F5 FEEL NEW");
        s.feel = false;
        assert_eq!(label(Button::Feel, &s), "F5 FEEL ORIG");
        assert_eq!(label(Button::Hide, &s), "F10 HIDE");
    }

    #[test]
    fn every_label_fits_its_button_and_is_ascii() {
        for width in [640u32, 960, 1280, 1600] {
            let fs = font_scale(width);
            for mask in 0..32u32 {
                let s = BarState {
                    tween: mask & 1 != 0,
                    counter: mask & 2 != 0,
                    hd: mask & 4 != 0,
                    hd_available: mask & 8 != 0,
                    scale: 5,
                    feel: mask & 16 != 0,
                    mobile: false,
                };
                for (b, r) in layout(width) {
                    let t = label(b, &s);
                    assert!(t.is_ascii());
                    assert!(font::text_width(&t, fs) as i32 + 2 * 3 * fs as i32 <= r.w, "{t} 超出按鈕 {width}");
                }
            }
        }
    }

    #[test]
    fn mobile_bar_fits_and_is_big_enough_for_a_finger() {
        for width in [1280u32, 1600, 1920, 2400, 3200] {
            let fs = mobile_font_scale(width);
            let tb = Toolbar::mobile(true);
            let l = mobile_layout(width);
            assert_eq!(l.len(), MOBILE_BUTTONS.len(), "{width}");
            for mask in 0..32u32 {
                let s = BarState {
                    tween: mask & 1 != 0,
                    counter: mask & 2 != 0,
                    hd: mask & 4 != 0,
                    hd_available: mask & 8 != 0,
                    scale: 5,
                    feel: mask & 16 != 0,
                    mobile: true,
                };
                for (b, r) in &l {
                    let t = label(*b, &s);
                    assert!(!t.starts_with('F') || t.starts_with("FEEL"), "手機字樣不帶 F 鍵：{t}");
                    assert!(font::text_width(&t, fs) as i32 + 2 * 3 * fs as i32 <= r.w, "{t} 超出按鈕 {width}");
                }
                assert_eq!(tb.render(width, &s).len(), width as usize * mobile_bar_height(width) as usize * 4);
            }
        }
        // 2400 寬、400 dpi 的手機：高度換算 58 dp。
        assert!(mobile_bar_height(2400) as f32 / 2.5 >= 48.0);
        let tb = Toolbar::mobile(true);
        let (_, r) = mobile_layout(2400).into_iter().find(|(b, _)| *b == Button::About).unwrap();
        assert_eq!(tb.hit(2400, r.x + 1, r.y + 1), Some(Button::About));
        assert_eq!(Toolbar::mobile(false).hit(2400, r.x + 1, r.y + 1), None);
        assert_eq!(Toolbar::mobile(false).height(2400), 0);
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
