//! 工具列：視窗上方獨立一塊，不蓋遊戲畫面。上面是中文按鈕，下面一行狀態列。可隱藏，滑鼠與觸控可點。
//! 版面只由視窗寬度（手機版再加每 dp 的像素數）決定，和 SDL 無關，所以命中測試可以單元測試。
//! 文字用 `ui::font` 的點陣字型，字串在 `ui::text`。
//!
//! F1 補間、F2 幀計數、F3 原貌／HD、F4 倍率、F5 手感、F10 設定畫面、F11 全螢幕的快捷鍵照舊，
//! 補間、幀計數、倍率與手感的按鈕移進設定畫面的「顯示」與「遊戲」頁。

pub use crate::ui::Rect;
use crate::ui::{Align, Canvas, font, text};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// 開始／繼續。
    Start,
    Pause,
    /// 開設定畫面的手把或鍵盤頁。
    Controls,
    /// 開設定畫面的遊戲頁。
    Game,
    Fullscreen,
    /// 原貌／HD。
    Gfx,
    /// 顯示模式。目前只有 VGA，按鈕灰掉。
    Mode,
    /// 手機版：開授權頁面。
    About,
    /// 手機版：離開。
    Quit,
    /// 手機版：收起工具列。
    Hide,
}

/// 工具列顯示的狀態。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarState {
    /// 目前用 HD 圖（只有 `hd_available` 時有意義）。
    pub hd: bool,
    pub hd_available: bool,
    /// 前端暫停（按了暫停，或設定畫面開著）。
    pub paused: bool,
    /// 有改變規則的選項開著。
    pub rules: bool,
    /// 無敵開著。
    pub god: bool,
    pub mobile: bool,
    /// 最近一則訊息（手把接上、拔除等），空字串是沒有。
    pub notice: String,
}

impl Default for BarState {
    fn default() -> BarState {
        BarState { hd: false, hd_available: false, paused: false, rules: false, god: false, mobile: false, notice: String::new() }
    }
}

/// 桌面版的按鈕，由左到右。
const DESKTOP_BUTTONS: [Button; 7] =
    [Button::Start, Button::Pause, Button::Controls, Button::Game, Button::Fullscreen, Button::Gfx, Button::Mode];

/// 手機版的按鈕。視窗已是全螢幕，沒有全螢幕鈕。倍率在手機上沒有用途。
const MOBILE_BUTTONS: [Button; 9] =
    [Button::Start, Button::Pause, Button::Controls, Button::Game, Button::Gfx, Button::Mode, Button::About, Button::Quit, Button::Hide];

const BG: [u8; 4] = [14, 20, 34, 255];
const BTN_BG: [u8; 4] = [35, 50, 75, 255];
const BTN_ACTIVE: [u8; 4] = [57, 82, 121, 255];
pub const INK: [u8; 4] = [237, 243, 255, 255];
pub const INK_OFF: [u8; 4] = [120, 139, 165, 255];
pub const INK_NOTE: [u8; 4] = [170, 190, 220, 255];
pub const INK_WARN: [u8; 4] = [255, 220, 110, 255];
const INK_ON: [u8; 4] = [130, 230, 130, 255];

/// 尺寸，單位像素。桌面版由視窗寬度決定，手機版由每 dp 的像素數決定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metrics {
    /// 字高。內附點陣是 16、20、24、32，剛好等於其中之一最清楚。
    pub fh: i32,
    /// 按鈕左右留白。
    pub pad: i32,
    /// 按鈕間距，也是版面的基本間距。
    pub gap: i32,
    /// 按鈕高度。
    pub btn_h: i32,
    /// 一列按鈕佔的高度（含上下間距）。
    pub row_h: i32,
    /// 狀態列高度。
    pub status_h: i32,
}

/// `dp` 是手機每 dp 的像素數，桌面版忽略。
pub fn metrics(width: u32, mobile: bool, dp: f32) -> Metrics {
    if mobile {
        let dp = dp.max(0.5);
        let fh = ((14.0 * dp).round() as i32).clamp(20, 56);
        let btn_h = ((48.0 * dp).ceil() as i32).max(fh * 7 / 4);
        let gap = ((4.0 * dp).round() as i32).max(3);
        return Metrics { fh, pad: fh * 2 / 3, gap, btn_h, row_h: btn_h + 2 * gap, status_h: fh * 3 / 2 };
    }
    let fh = match width {
        0..=799 => 16,
        800..=1099 => 20,
        1100..=1499 => 24,
        1500..=2199 => 32,
        _ => 40,
    };
    let gap = (fh / 5).max(3);
    let btn_h = fh * 3 / 2;
    Metrics { fh, pad: fh * 2 / 3, gap, btn_h, row_h: btn_h + 2 * gap, status_h: fh * 3 / 2 }
}

impl Button {
    /// 按鈕上的字。
    pub fn label(self, s: &BarState) -> String {
        let _ = s;
        match self {
            Button::Start => text::BTN_START.to_string(),
            Button::Pause => text::BTN_PAUSE.to_string(),
            Button::Controls => text::BTN_CONTROLS.to_string(),
            Button::Game => text::BTN_GAME.to_string(),
            Button::Fullscreen => text::BTN_FULLSCREEN.to_string(),
            Button::Gfx => text::BTN_GFX.to_string(),
            Button::Mode => format!("{}{}", text::BTN_MODE_PREFIX, text::MODE_VGA),
            Button::About => text::BTN_ABOUT.to_string(),
            Button::Quit => text::BTN_QUIT.to_string(),
            Button::Hide => text::BTN_HIDE.to_string(),
        }
    }

    /// 決定按鈕寬度的字：同一個按鈕不管狀態怎麼變，寬度都不跳。顯示模式之後會換成 CGA、EGA、MGA，字數相同。
    fn widest(self) -> String {
        self.label(&BarState::default())
    }

    pub fn enabled(self, s: &BarState) -> bool {
        match self {
            Button::Gfx => s.hd_available,
            // 目前只有 VGA，之後才接 CGA、MGA。
            Button::Mode => false,
            _ => true,
        }
    }
}

/// 工具列的版面：按鈕、狀態列與總高度。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarLayout {
    pub buttons: Vec<(Button, Rect)>,
    /// 按鈕有幾列。窗太窄時折到下一列。
    pub rows: i32,
    pub status: Rect,
    pub height: i32,
}

pub fn layout(width: u32, mobile: bool, dp: f32) -> BarLayout {
    let m = metrics(width, mobile, dp);
    let f = font::builtin();
    let list: &[Button] = if mobile { &MOBILE_BUTTONS } else { &DESKTOP_BUTTONS };
    let w = width as i32;
    let mut buttons = Vec::with_capacity(list.len());
    let (mut x, mut row) = (m.gap, 0);
    for &b in list {
        let bw = (f.text_width(&b.widest(), m.fh) + 2 * m.pad).min((w - 2 * m.gap).max(1));
        if x + bw > w - m.gap && x > m.gap {
            row += 1;
            x = m.gap;
        }
        buttons.push((b, Rect { x, y: row * m.row_h + m.gap, w: bw, h: m.btn_h }));
        x += bw + m.gap;
    }
    let rows = row + 1;
    let status = Rect { x: 0, y: rows * m.row_h, w, h: m.status_h };
    BarLayout { buttons, rows, status, height: rows * m.row_h + m.status_h }
}

/// 狀態列的段落：文字與顏色。
pub fn status_parts(s: &BarState) -> Vec<(String, [u8; 4])> {
    let mut v = vec![
        if s.paused { (text::ST_PAUSED.to_string(), INK_WARN) } else { (text::ST_RUNNING.to_string(), INK_NOTE) },
        if s.rules { (text::ST_RULES.to_string(), INK_WARN) } else { (text::ST_ORIGINAL.to_string(), INK_NOTE) },
    ];
    if s.god {
        v.push((text::ST_GOD.to_string(), INK_WARN));
    }
    if !s.notice.is_empty() {
        v.push((s.notice.clone(), INK));
    }
    v
}

/// 工具列本身：顯示與否、手機版與每 dp 的像素數。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Toolbar {
    pub visible: bool,
    pub mobile: bool,
    pub dp: f32,
}

/// 讀不到螢幕 DPI 時的每 dp 像素數。
pub const DEFAULT_DP: f32 = 2.0;

impl Toolbar {
    pub fn new(visible: bool) -> Toolbar {
        Toolbar { visible, mobile: false, dp: 1.0 }
    }

    pub fn mobile(visible: bool) -> Toolbar {
        Toolbar { visible, mobile: true, dp: DEFAULT_DP }
    }

    pub fn with_dp(self, dp: f32) -> Toolbar {
        Toolbar { dp, ..self }
    }

    /// 佔用的高度（按鈕加狀態列），隱藏時 0。
    pub fn height(&self, width: u32) -> u32 {
        if self.visible { self.layout(width).height as u32 } else { 0 }
    }

    pub fn layout(&self, width: u32) -> BarLayout {
        layout(width, self.mobile, self.dp)
    }

    /// 視窗座標 `(x, y)` 的點擊命中哪個按鈕。隱藏時沒有命中。灰掉的按鈕也算命中，由呼叫端決定能不能按。
    pub fn hit(&self, width: u32, x: i32, y: i32) -> Option<Button> {
        if !self.visible {
            return None;
        }
        self.layout(width).buttons.into_iter().find(|(_, r)| r.contains(x, y)).map(|(b, _)| b)
    }

    /// 畫成 `width × height(width)` 的 RGBA。
    pub fn render(&self, width: u32, s: &BarState) -> Vec<u8> {
        let l = self.layout(width);
        let m = metrics(width, self.mobile, self.dp);
        let mut c = Canvas::new(width as i32, l.height, BG);
        for (b, r) in &l.buttons {
            let on = match b {
                Button::Pause => s.paused,
                _ => false,
            };
            c.rect(*r, if on { BTN_ACTIVE } else { BTN_BG });
            let ink = if !b.enabled(s) {
                INK_OFF
            } else if *b == Button::Gfx && s.hd {
                INK_ON
            } else {
                INK
            };
            c.text_in(*r, &b.label(s), m.fh, m.pad / 2, Align::Center, ink);
        }
        // 狀態列：各段用不同顏色，放不下的尾端截掉。
        let fnt = font::builtin();
        let mut x = l.status.x + m.gap * 2;
        let ty = l.status.y + (l.status.h - m.fh) / 2;
        let sep = text::ST_SEP;
        for (i, (t, ink)) in status_parts(s).iter().enumerate() {
            if i > 0 {
                x += fnt.text_width(sep, m.fh);
            }
            let room = width as i32 - x - m.gap;
            if room <= 0 {
                break;
            }
            let t = fnt.fit(t, room, m.fh);
            c.text(x, ty, &t, m.fh, *ink);
            x += fnt.text_width(&t, m.fh);
        }
        c.px
    }
}

/// 桌面版視窗上方工具列的總高度（按鈕加狀態列）。
pub fn desktop_height(width: u32) -> u32 {
    layout(width, false, 1.0).height as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st() -> BarState {
        BarState { hd: true, hd_available: true, ..BarState::default() }
    }

    #[test]
    fn desktop_layout_fits_every_supported_window_width() {
        for scale in 1..=8u32 {
            let width = 320 * scale;
            let l = layout(width, false, 1.0);
            assert_eq!(l.buttons.len(), DESKTOP_BUTTONS.len(), "按鈕一個都不能少 {width}");
            let m = metrics(width, false, 1.0);
            for (i, (_, r)) in l.buttons.iter().enumerate() {
                assert!(r.x >= 0 && r.right() <= width as i32, "按鈕超出視窗 {width}：{r:?}");
                assert!(r.h == m.btn_h && r.y >= 0 && r.bottom() <= l.rows * m.row_h, "{r:?}");
                for (_, o) in &l.buttons[..i] {
                    assert!(r.intersect(o).w == 0 || r.intersect(o).h == 0, "按鈕重疊 {width}");
                }
            }
            assert_eq!(l.status.y, l.rows * m.row_h);
            assert_eq!(l.height, l.status.bottom());
            // 預設的 2 倍以上一列放得下，1 倍（320 寬）才折行。
            if scale >= 2 {
                assert_eq!(l.rows, 1, "{width} 應一列放完");
            }
        }
    }

    #[test]
    fn metrics_tiers_use_baked_heights_on_desktop() {
        let baked = font::builtin().baked_heights();
        for w in [640, 800, 960, 1280, 1600, 1920] {
            let m = metrics(w, false, 1.0);
            assert!(baked.contains(&m.fh), "{w} 的字高 {} 不是內附點陣", m.fh);
        }
        assert_eq!(desktop_height(640), (metrics(640, false, 1.0).row_h + metrics(640, false, 1.0).status_h) as u32);
    }

    #[test]
    fn hit_test_centres_gaps_and_outside() {
        let tb = Toolbar::new(true);
        let width = 960;
        let l = tb.layout(width);
        for (b, r) in &l.buttons {
            assert_eq!(tb.hit(width, r.x + r.w / 2, r.y + r.h / 2), Some(*b));
            assert_eq!(tb.hit(width, r.x, r.y), Some(*b), "左上角");
            assert_eq!(tb.hit(width, r.right() - 1, r.bottom() - 1), Some(*b), "右下角");
            assert_eq!(tb.hit(width, r.right(), r.y + r.h / 2), None, "右緣之外是縫");
            assert_eq!(tb.hit(width, r.x + r.w / 2, l.status.y + 1), None, "狀態列不是按鈕");
            assert_eq!(tb.hit(width, r.x + r.w / 2, l.height), None, "工具列下緣之外是遊戲畫面");
            assert_eq!(tb.hit(width, r.x + r.w / 2, -1), None);
        }
        assert_eq!(tb.hit(width, 0, 0), None, "左邊界是縫");
    }

    #[test]
    fn buttons_are_the_spec_005_set_in_order() {
        let order: Vec<Button> = layout(960, false, 1.0).buttons.into_iter().map(|(b, _)| b).collect();
        assert_eq!(order, DESKTOP_BUTTONS.to_vec());
        let s = st();
        assert_eq!(Button::Start.label(&s), "開始／繼續");
        assert_eq!(Button::Pause.label(&s), "暫停");
        assert_eq!(Button::Controls.label(&s), "控制設定");
        assert_eq!(Button::Game.label(&s), "遊戲設定");
        assert_eq!(Button::Fullscreen.label(&s), "全螢幕");
        assert_eq!(Button::Gfx.label(&s), "原貌／HD");
        assert_eq!(Button::Mode.label(&s), "顯示模式：VGA");
    }

    #[test]
    fn display_mode_button_is_greyed_and_hd_follows_availability() {
        let mut s = st();
        assert!(!Button::Mode.enabled(&s), "顯示模式目前只有 VGA，按鈕灰掉");
        assert!(Button::Gfx.enabled(&s));
        s.hd_available = false;
        assert!(!Button::Gfx.enabled(&s));
        for b in [Button::Start, Button::Pause, Button::Controls, Button::Game, Button::Fullscreen, Button::About, Button::Quit, Button::Hide] {
            assert!(b.enabled(&s));
        }
    }

    #[test]
    fn hidden_toolbar_takes_no_space_and_ignores_clicks() {
        let tb = Toolbar::new(false);
        assert_eq!(tb.height(960), 0);
        let (_, r) = layout(960, false, 1.0).buttons[0];
        assert_eq!(tb.hit(960, r.x + 1, r.y + 1), None);
        assert_eq!(Toolbar::new(true).height(960), desktop_height(960));
        assert!(Toolbar::new(true).height(960) > 40);
    }

    #[test]
    fn every_label_fits_its_button_on_both_layouts() {
        let widths = [(320u32, false, 1.0f32), (640, false, 1.0), (960, false, 1.0), (1280, false, 1.0), (1600, false, 1.0), (1280, true, 2.0), (2400, true, 2.5), (1920, true, 3.0)];
        for (width, mobile, dp) in widths {
            let m = metrics(width, mobile, dp);
            let f = font::builtin();
            for (b, r) in layout(width, mobile, dp).buttons {
                for hd in [false, true] {
                    for avail in [false, true] {
                        let s = BarState { hd, hd_available: avail, mobile, ..BarState::default() };
                        let t = b.label(&s);
                        assert!(f.missing(&t).is_empty());
                        assert!(f.text_width(&t, m.fh) + 2 * (m.pad / 2) <= r.w, "{t} 超出按鈕 {width}（{mobile}）：{} > {}", f.text_width(&t, m.fh), r.w);
                    }
                }
            }
        }
    }

    #[test]
    fn mobile_bar_has_big_buttons_wraps_and_has_no_fullscreen_or_scale() {
        for (width, dp) in [(1280u32, 2.0f32), (1600, 2.0), (1920, 3.0), (2400, 2.5), (3200, 3.5), (800, 1.5)] {
            let l = layout(width, true, dp);
            assert_eq!(l.buttons.len(), MOBILE_BUTTONS.len(), "{width}");
            assert!(l.buttons.iter().all(|(b, _)| *b != Button::Fullscreen));
            let m = metrics(width, true, dp);
            for (i, (_, r)) in l.buttons.iter().enumerate() {
                assert!(r.h as f32 >= 48.0 * dp - 1.0, "按鈕高 {} 小於 48 dp（{width}，dp {dp}）", r.h);
                assert!(r.right() <= width as i32 && r.x >= 0, "{r:?} 超出 {width}");
                assert!(r.bottom() <= l.rows * m.row_h);
                for (_, o) in &l.buttons[..i] {
                    assert!(r.intersect(o).w == 0 || r.intersect(o).h == 0, "按鈕重疊 {width}");
                }
            }
            // 窗越窄，列數越多。
            assert!(l.rows >= 1 && l.height == l.rows * m.row_h + m.status_h);
        }
        assert_eq!(layout(2400, true, 2.5).rows, 1);
        assert!(layout(800, true, 1.5).rows > 1);
        // 2400 寬、400 dpi 的手機：一列放完，總高約 71 dp。
        let tb = Toolbar::mobile(true).with_dp(2.5);
        let (_, r) = tb.layout(2400).buttons.into_iter().find(|(b, _)| *b == Button::About).unwrap();
        assert_eq!(tb.hit(2400, r.x + 1, r.y + 1), Some(Button::About));
        assert_eq!(Toolbar::mobile(false).hit(2400, r.x + 1, r.y + 1), None);
        assert_eq!(Toolbar::mobile(false).height(2400), 0);
        // 手機版的字比桌面版大。
        assert!(metrics(2400, true, 2.5).fh > metrics(960, false, 1.0).fh);
    }

    #[test]
    fn status_line_shows_pause_rules_god_and_notice() {
        let plain = status_parts(&BarState::default());
        assert_eq!(plain.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), vec!["執行中", "原版規則"]);
        let s = BarState { paused: true, rules: true, god: true, notice: "手把已拔除：X".into(), ..BarState::default() };
        let v = status_parts(&s);
        assert_eq!(v.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), vec!["已暫停", "強化規則", "無敵", "手把已拔除：X"]);
        let rules_only = BarState { rules: true, ..BarState::default() };
        assert!(status_parts(&rules_only).iter().any(|p| p.0 == "強化規則") && !status_parts(&rules_only).iter().any(|p| p.0 == "無敵"));
    }

    #[test]
    fn render_has_expected_size_changes_with_state_and_is_opaque() {
        let tb = Toolbar::new(true);
        let a = tb.render(960, &st());
        assert_eq!(a.len(), 960 * tb.height(960) as usize * 4);
        for variant in [
            BarState { paused: true, ..st() },
            BarState { rules: true, ..st() },
            BarState { god: true, ..st() },
            BarState { notice: "x".into(), ..st() },
            BarState { hd: false, ..st() },
            BarState { hd_available: false, ..st() },
        ] {
            assert_ne!(a, tb.render(960, &variant), "{variant:?}");
        }
        assert!(a.chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn status_line_never_draws_outside_its_row() {
        // 很長的訊息在窄視窗也只畫在狀態列內。
        let s = BarState { paused: true, rules: true, god: true, notice: "手把已拔除：一支名字非常非常長的手把裝置，已取消第 1 人的指派。".into(), ..BarState::default() };
        for width in [320u32, 640] {
            let tb = Toolbar::new(true);
            let px = tb.render(width, &s);
            let l = tb.layout(width);
            let blank = tb.render(width, &BarState { paused: true, ..BarState::default() });
            // 狀態列以上的像素與狀態文字不同時相同（暫停鈕的底色隨暫停狀態，所以兩邊都暫停）。
            let above = (l.status.y * width as i32 * 4) as usize;
            assert_eq!(&px[..above], &blank[..above], "{width}");
            assert_eq!(px.len(), blank.len());
        }
    }
}
