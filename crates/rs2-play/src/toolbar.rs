//! 工具列：視窗上方獨立一塊，不蓋遊戲畫面。上面是中文按鈕，下面一行狀態列。可隱藏，滑鼠與觸控可點。
//! 版面只由視窗寬度（手機版再加每 dp 的像素數）決定，和 SDL 無關，所以命中測試可以單元測試。
//! 文字用 `ui::font` 的點陣字型，字串在 `ui::text`。
//!
//! F1 補間、F2 幀計數、F3 原貌／HD、F4 倍率、F5 手感、F6 顯示模式、F10 設定畫面、F11 全螢幕的快捷鍵照舊，
//! 補間、幀計數、倍率與手感的按鈕移進設定畫面的「顯示」與「遊戲」頁。

use crate::mode::DisplayMode;
pub use crate::ui::Rect;
use crate::ui::style::{self, Icon, Rgba, Vis, color};
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
    /// 顯示模式（VGA、EGA、CGA、MGA 輪流）。單色圖庫缺檔時跳過 CGA、MGA，但按鈕照常能按。
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
    /// 目前的顯示模式。
    pub mode: DisplayMode,
    /// 單色圖庫載入成功，CGA、MGA 選得到。為假時狀態列常駐一段說明。
    pub mono_available: bool,
    /// 最近一則訊息（手把接上、拔除等），空字串是沒有。
    pub notice: String,
    /// 滑鼠移過的按鈕（變亮）。觸控沒有。
    pub hover: Option<Button>,
    /// 滑鼠按著的按鈕（變深）。
    pub down: Option<Button>,
}

impl Default for BarState {
    fn default() -> BarState {
        BarState {
            hd: false,
            hd_available: false,
            paused: false,
            rules: false,
            god: false,
            mobile: false,
            mode: DisplayMode::Vga,
            mono_available: true,
            notice: String::new(),
            hover: None,
            down: None,
        }
    }
}

/// 桌面版的按鈕，由左到右。
const DESKTOP_BUTTONS: [Button; 7] =
    [Button::Start, Button::Pause, Button::Controls, Button::Game, Button::Fullscreen, Button::Gfx, Button::Mode];

/// 手機版的按鈕。視窗已是全螢幕，沒有全螢幕鈕。倍率在手機上沒有用途。
const MOBILE_BUTTONS: [Button; 9] =
    [Button::Start, Button::Pause, Button::Controls, Button::Game, Button::Gfx, Button::Mode, Button::About, Button::Quit, Button::Hide];

// 色彩見規格 005「工具列與設定畫面的風格」，定義在 `ui::style::color`。
pub const INK: Rgba = color::TEXT;
pub const INK_OFF: Rgba = color::TEXT3;
pub const INK_NOTE: Rgba = color::TEXT2;
pub const INK_WARN: Rgba = color::WARN;
const INK_ON: Rgba = color::ON;

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
    /// 按鈕與欄位的圓角半徑（規格 005：6 到 8 像素，手機版按比例放大）。
    pub rad: i32,
}

/// `dp` 是手機每 dp 的像素數，桌面版忽略。
pub fn metrics(width: u32, mobile: bool, dp: f32) -> Metrics {
    if mobile {
        let dp = dp.max(0.5);
        let fh = ((14.0 * dp).round() as i32).clamp(20, 56);
        let btn_h = ((48.0 * dp).ceil() as i32).max(fh * 7 / 4);
        let gap = ((4.0 * dp).round() as i32).max(3);
        let rad = ((4.0 * dp).round() as i32).clamp(8, 16);
        return Metrics { fh, pad: fh * 2 / 3, gap, btn_h, row_h: btn_h + 2 * gap, status_h: fh * 3 / 2, rad };
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
    let rad = (btn_h * 3 / 10).clamp(6, 8);
    Metrics { fh, pad: fh * 2 / 3, gap, btn_h, row_h: btn_h + 2 * gap, status_h: fh * 3 / 2, rad }
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
            Button::Mode => format!("{}{}", text::BTN_MODE_PREFIX, s.mode.label()),
            Button::About => text::BTN_ABOUT.to_string(),
            Button::Quit => text::BTN_QUIT.to_string(),
            Button::Hide => text::BTN_HIDE.to_string(),
        }
    }

    /// 這個按鈕在字高 `fh` 下最寬的字寬：同一個按鈕不管狀態怎麼變，寬度都不跳。
    /// 顯示模式四個名稱的字寬不同（M 比 V 寬），所以逐一量。
    fn widest_width(self, f: &font::Font, fh: i32) -> i32 {
        match self {
            Button::Mode => DisplayMode::ALL
                .into_iter()
                .map(|mode| f.text_width(&self.label(&BarState { mode, ..BarState::default() }), fh))
                .max()
                .unwrap_or(0),
            _ => f.text_width(&self.label(&BarState::default()), fh),
        }
    }

    /// 按鈕前面的圖示（程式畫的，不用圖檔）。
    pub fn icon(self) -> Icon {
        match self {
            Button::Start => Icon::Play,
            Button::Pause => Icon::Pause,
            Button::Controls => Icon::Gamepad,
            Button::Game => Icon::Gear,
            Button::Fullscreen => Icon::Fullscreen,
            Button::Gfx => Icon::Image,
            Button::Mode => Icon::Monitor,
            Button::About => Icon::Info,
            Button::Quit => Icon::Power,
            Button::Hide => Icon::ChevronUp,
        }
    }

    pub fn enabled(self, s: &BarState) -> bool {
        match self {
            Button::Gfx => s.hd_available,
            // 顯示模式永遠能按：單色圖庫缺檔時只在 VGA 與 EGA 之間循環，說明在狀態列。
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
    /// 按鈕前面有圖示。桌面版一列放不下時拿掉圖示，字比圖示重要。
    pub icons: bool,
}

/// 按鈕左右的留白。有圖示時收窄一點，讓圖示塞得進同樣的寬度。
fn button_pad(m: &Metrics, mobile: bool, icons: bool) -> i32 {
    if icons && !mobile { m.fh / 2 } else { m.pad }
}

/// 圖示佔的寬度（含圖示與字之間的間距）。
fn icon_block(m: &Metrics, icons: bool) -> i32 {
    if icons { m.fh + m.fh / 3 } else { 0 }
}

fn layout_with(width: u32, mobile: bool, dp: f32, icons: bool) -> BarLayout {
    let m = metrics(width, mobile, dp);
    let f = font::builtin();
    let list: &[Button] = if mobile { &MOBILE_BUTTONS } else { &DESKTOP_BUTTONS };
    let w = width as i32;
    let mut buttons = Vec::with_capacity(list.len());
    let (mut x, mut row) = (m.gap, 0);
    let pad = button_pad(&m, mobile, icons);
    for &b in list {
        let bw = (b.widest_width(f, m.fh) + 2 * pad + icon_block(&m, icons)).min((w - 2 * m.gap).max(1));
        if x + bw > w - m.gap && x > m.gap {
            row += 1;
            x = m.gap;
        }
        buttons.push((b, Rect { x, y: row * m.row_h + m.gap, w: bw, h: m.btn_h }));
        x += bw + m.gap;
    }
    let rows = row + 1;
    let status = Rect { x: 0, y: rows * m.row_h, w, h: m.status_h };
    BarLayout { buttons, rows, status, height: rows * m.row_h + m.status_h, icons }
}

pub fn layout(width: u32, mobile: bool, dp: f32) -> BarLayout {
    let l = layout_with(width, mobile, dp, true);
    if !mobile && l.rows > 1 {
        let plain = layout_with(width, mobile, dp, false);
        if plain.rows < l.rows {
            return plain;
        }
    }
    l
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
    if !s.mono_available {
        v.push((text::ST_MONO_MISSING.to_string(), INK_WARN));
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
        let mut c = Canvas::new(width as i32, l.height, color::BG);
        let rad = m.rad as f32;
        for (b, r) in &l.buttons {
            let enabled = b.enabled(s);
            let vis = Vis { hover: enabled && s.hover == Some(*b), down: enabled && s.down == Some(*b), focus: false, enabled };
            let accent = *b == Button::Pause && s.paused;
            style::button(&mut c, *r, rad, vis, accent);
            let ink = if !enabled {
                INK_OFF
            } else if (*b == Button::Gfx && s.hd) || (*b == Button::Mode && s.mode != DisplayMode::Vga) {
                INK_ON
            } else if accent {
                color::PRIMARY
            } else {
                INK
            };
            let label = b.label(s);
            let pad = button_pad(&m, self.mobile, l.icons);
            let block = icon_block(&m, l.icons);
            let fnt = font::builtin();
            let tw = fnt.text_width(&fnt.fit(&label, (r.w - 2 * pad - block).max(1), m.fh), m.fh);
            // 圖示加字當成一組置中。
            let x0 = r.x + (r.w - tw - block) / 2;
            if l.icons {
                style::draw_icon(&mut c, b.icon(), Rect::new(x0, r.y + (r.h - m.fh) / 2, m.fh, m.fh), ink);
            }
            let text_rect = Rect::new(x0 + block, r.y, tw + 2, r.h);
            c.text_in(text_rect, &label, m.fh, 0, Align::Left, ink);
        }
        // 狀態列：各段是半透明的小膠囊，放不下的尾端截掉。
        let fnt = font::builtin();
        let mut x = l.status.x + m.gap * 2;
        let ch = (l.status.h - 4).max(m.fh + 2);
        let cy = l.status.y + (l.status.h - ch) / 2;
        let tx = |r: &Rect| r.y + (r.h - m.fh) / 2;
        for (i, (t, ink)) in status_parts(s).iter().enumerate() {
            let room = width as i32 - x - m.gap;
            if room <= 0 {
                break;
            }
            // 第三段起的訊息（手把接上、拔除）是一般文字，不做膠囊。
            let chip = i < 2 || (s.god && i == 2 && t.as_str() == text::ST_GOD);
            if chip {
                let dot = m.fh / 3;
                let padx = m.fh / 2;
                let want = fnt.text_width(t, m.fh) + 2 * padx + dot + m.fh / 4;
                let w = want.min(room);
                let r = Rect::new(x, cy, w, ch);
                style::fill_rrect(&mut c, r, ch as f32 / 2.0, style::with_alpha(*ink, 30));
                style::stroke_rrect(&mut c, r, ch as f32 / 2.0, 1.0, style::with_alpha(*ink, 70));
                style::fill_circle(&mut c, (r.x + padx) as f32 + dot as f32 / 2.0 - 1.0, (r.y + r.h / 2) as f32, dot as f32 / 2.0, *ink);
                let tr = Rect::new(r.x + padx + dot + m.fh / 4, tx(&r), (r.w - 2 * padx - dot - m.fh / 4).max(1), m.fh + 2);
                c.text_in(tr, t, m.fh, 0, Align::Left, *ink);
                x += w + m.gap;
            } else {
                let t = fnt.fit(t, room, m.fh);
                c.text(x + m.gap, l.status.y + (l.status.h - m.fh) / 2, &t, m.fh, *ink);
                x += fnt.text_width(&t, m.fh) + m.gap * 3;
            }
        }
        // 工具列與遊戲畫面之間一條細線。
        c.rect(Rect::new(0, l.height - 1, width as i32, 1), [255, 255, 255, 26]);
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
    fn display_mode_button_is_always_enabled_and_hd_follows_availability() {
        let mut s = st();
        assert!(Button::Mode.enabled(&s), "顯示模式能按");
        assert!(Button::Gfx.enabled(&s));
        s.hd_available = false;
        assert!(!Button::Gfx.enabled(&s));
        // 單色圖庫缺檔時 CGA、MGA 選不到，按鈕仍能按（VGA 與 EGA 之間循環）。
        s.mono_available = false;
        for b in [Button::Mode, Button::Start, Button::Pause, Button::Controls, Button::Game, Button::Fullscreen, Button::About, Button::Quit, Button::Hide] {
            assert!(b.enabled(&s), "{b:?}");
        }
    }

    #[test]
    fn display_mode_label_follows_the_mode_and_keeps_the_prefix() {
        for (mode, want) in DisplayMode::ALL.into_iter().zip(["顯示模式：VGA", "顯示模式：EGA", "顯示模式：CGA", "顯示模式：MGA"]) {
            assert_eq!(Button::Mode.label(&BarState { mode, ..BarState::default() }), want);
        }
    }

    #[test]
    fn mode_button_width_does_not_jump_when_the_mode_changes() {
        // 寬度由四個名稱最寬的決定，和目前的模式無關；每個名稱都放得進去。
        let f = font::builtin();
        for width in [320u32, 640, 960, 1280, 1600] {
            let m = metrics(width, false, 1.0);
            let (_, r) = layout(width, false, 1.0).buttons.into_iter().find(|(b, _)| *b == Button::Mode).unwrap();
            for mode in DisplayMode::ALL {
                let t = Button::Mode.label(&BarState { mode, ..BarState::default() });
                assert!(f.text_width(&t, m.fh) + 2 * (m.pad / 2) <= r.w, "{t} 超出按鈕 {width}");
            }
        }
    }

    #[test]
    fn mono_missing_adds_one_persistent_status_part() {
        let ok = status_parts(&BarState::default());
        assert!(!ok.iter().any(|p| p.0 == text::ST_MONO_MISSING));
        let missing = status_parts(&BarState { mono_available: false, ..BarState::default() });
        assert_eq!(missing.len(), ok.len() + 1);
        assert!(missing.iter().any(|p| p.0 == text::ST_MONO_MISSING && p.1 == INK_WARN));
        assert!(font::builtin().missing(text::ST_MONO_MISSING).is_empty());
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
                        for mode in DisplayMode::ALL {
                            let s = BarState { hd, hd_available: avail, mobile, mode, ..BarState::default() };
                            let t = b.label(&s);
                            assert!(f.missing(&t).is_empty());
                            assert!(f.text_width(&t, m.fh) + 2 * (m.pad / 2) <= r.w, "{t} 超出按鈕 {width}（{mobile}）：{} > {}", f.text_width(&t, m.fh), r.w);
                        }
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
            BarState { mode: DisplayMode::Ega, ..st() },
            BarState { mode: DisplayMode::Cga, ..st() },
            BarState { mode: DisplayMode::Mga, ..st() },
            BarState { mono_available: false, ..st() },
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

    fn px_at(img: &[u8], w: i32, x: i32, y: i32) -> [u8; 4] {
        let o = ((y * w + x) * 4) as usize;
        [img[o], img[o + 1], img[o + 2], img[o + 3]]
    }

    #[test]
    fn hover_brightens_and_press_darkens_only_that_button_and_disabled_ones_ignore_both() {
        let tb = Toolbar::new(true);
        let width = 1280;
        let l = tb.layout(width);
        let base = tb.render(width, &st());
        let rect_of = |b: Button| l.buttons.iter().find(|(x, _)| *x == b).unwrap().1;
        let sum = |p: [u8; 4]| p[0] as u32 + p[1] as u32 + p[2] as u32;
        let g = rect_of(Button::Game);
        // 按鈕左上角附近、字與圖示之外的位置。
        let probe = (g.x + 6, g.y + g.h / 2);
        let hover = tb.render(width, &BarState { hover: Some(Button::Game), ..st() });
        let down = tb.render(width, &BarState { hover: Some(Button::Game), down: Some(Button::Game), ..st() });
        let (b, h, d) = (px_at(&base, width as i32, probe.0, probe.1), px_at(&hover, width as i32, probe.0, probe.1), px_at(&down, width as i32, probe.0, probe.1));
        assert!(sum(h) > sum(b), "移過變亮：{b:?} {h:?}");
        assert!(sum(d) < sum(b), "按下變深：{b:?} {d:?}");
        // 只有那顆按鈕（含 1 像素外框）的像素變。
        let area = g.inset(-2);
        for y in 0..l.height {
            for x in 0..width as i32 {
                if !area.contains(x, y) {
                    assert_eq!(px_at(&base, width as i32, x, y), px_at(&hover, width as i32, x, y), "({x},{y}) 在按鈕之外卻變了");
                }
            }
        }
        // 灰掉的鈕（沒有 HD 資料時的原貌／HD）不理會移過與按下。
        let no_hd = BarState { hd: false, hd_available: false, ..BarState::default() };
        let off = tb.render(width, &BarState { hover: Some(Button::Gfx), down: Some(Button::Gfx), ..no_hd.clone() });
        assert_eq!(tb.render(width, &no_hd), off);
    }

    #[test]
    fn icons_fit_when_there_is_room_and_drop_before_the_row_would_wrap() {
        for w in [960u32, 1280, 1600, 1920] {
            let l = layout(w, false, 1.0);
            assert!(l.icons && l.rows == 1, "{w} 放得下圖示：icons {} rows {}", l.icons, l.rows);
        }
        let narrow = layout(640, false, 1.0);
        assert!(!narrow.icons && narrow.rows == 1, "640 寬拿掉圖示也要一列放完");
        assert!(layout(320, false, 1.0).rows > 1);
        // 手機版一律有圖示。
        assert!(layout(2400, true, 2.5).icons && layout(1280, true, 2.0).icons);
    }

    #[test]
    fn every_enabled_button_draws_its_icon_in_front_of_the_text_at_960() {
        let tb = Toolbar::new(true);
        let l = tb.layout(960);
        assert!(l.icons);
        let m = metrics(960, false, 1.0);
        let img = tb.render(960, &st());
        for (b, r) in &l.buttons {
            // 圖示和字當成一組置中：圖示在組的最左邊，那個正方形裡要有亮像素。
            let f = font::builtin();
            let pad = button_pad(&m, false, true);
            let block = icon_block(&m, true);
            let tw = f.text_width(&f.fit(&b.label(&st()), (r.w - 2 * pad - block).max(1), m.fh), m.fh);
            let x0 = r.x + (r.w - tw - block) / 2;
            let cell = Rect::new(x0, r.y + (r.h - m.fh) / 2, m.fh, m.fh);
            let ink = (cell.y..cell.bottom()).flat_map(|y| (cell.x..cell.right()).map(move |x| (x, y))).filter(|&(x, y)| {
                let p = px_at(&img, 960, x, y);
                p[0] as u32 + p[1] as u32 + p[2] as u32 > 330
            });
            if b.enabled(&st()) {
                assert!(ink.count() > 6, "{b:?} 的圖示不見了");
            }
        }
    }

    #[test]
    fn corner_radius_is_6_to_8_px_on_desktop() {
        for w in [640u32, 800, 960, 1280, 1600, 1920, 2560] {
            let m = metrics(w, false, 1.0);
            assert!((6..=8).contains(&m.rad), "{w}：{}", m.rad);
        }
        // 圓角真的畫出來：按鈕左上角那一格不是按鈕色。
        let tb = Toolbar::new(true);
        let l = tb.layout(960);
        let r = l.buttons[0].1;
        let img = tb.render(960, &st());
        let corner = px_at(&img, 960, r.x, r.y);
        let inner = px_at(&img, 960, r.x + 8, r.y + r.h / 2);
        assert!(corner[2] < inner[2], "角落{corner:?} 比內部{inner:?}更接近背景");
    }

    #[test]
    fn status_chips_use_warning_colour_for_paused_rules_and_god() {
        let tb = Toolbar::new(true);
        let l = tb.layout(960);
        let warm = |s: &BarState| {
            let img = tb.render(960, s);
            let mut n = 0;
            for y in l.status.y..l.status.bottom() {
                for x in 0..960 {
                    let p = px_at(&img, 960, x, y);
                    if p[0] > 200 && p[0] as i32 > p[2] as i32 + 60 {
                        n += 1;
                    }
                }
            }
            n
        };
        let calm = warm(&st());
        assert!(warm(&BarState { paused: true, ..st() }) > calm + 40);
        assert!(warm(&BarState { rules: true, ..st() }) > calm + 40);
        assert!(warm(&BarState { god: true, rules: true, ..st() }) > warm(&BarState { rules: true, ..st() }) + 20);
    }

    /// 存圖（`RS2_SHOTS` 指定目錄才存）：工具列各狀態，桌面與手機版。
    #[test]
    fn ui_shots_toolbar_states() {
        let Some(dir) = std::env::var("RS2_SHOTS").ok().map(std::path::PathBuf::from) else { return };
        std::fs::create_dir_all(&dir).unwrap();
        let stack = |tb: &Toolbar, width: u32, states: &[BarState]| -> (Vec<u8>, usize, usize) {
            let h = tb.height(width) as usize;
            let mut out = Vec::new();
            for s in states {
                out.extend_from_slice(&tb.render(width, s));
                // 兩張之間 8 像素的深色間隔。
                out.extend(std::iter::repeat_n([5u8, 8, 14, 255], width as usize * 8).flatten());
            }
            (out, width as usize, (h + 8) * states.len())
        };
        let hd = BarState { hd: true, hd_available: true, ..BarState::default() };
        let states = [
            hd.clone(),
            BarState { hover: Some(Button::Game), ..hd.clone() },
            BarState { hover: Some(Button::Pause), down: Some(Button::Pause), ..hd.clone() },
            BarState { paused: true, rules: true, god: true, notice: "偵測到手把：Xbox Controller，請到控制設定指派。".into(), ..hd.clone() },
            BarState { hd: false, hd_available: false, ..BarState::default() },
        ];
        let (px, w, h) = stack(&Toolbar::new(true), 1280, &states);
        std::fs::write(dir.join("toolbar_states_desktop.png"), rs2_gfx::png::encode_rgba(w, h, &px)).unwrap();
        let (px, w, h) = stack(&Toolbar::new(true), 800, &states[..2]);
        std::fs::write(dir.join("toolbar_states_desktop_narrow.png"), rs2_gfx::png::encode_rgba(w, h, &px)).unwrap();
        let mstates = [BarState { mobile: true, ..hd.clone() }, BarState { mobile: true, paused: true, ..hd }];
        let (px, w, h) = stack(&Toolbar::mobile(true).with_dp(2.5), 2400, &mstates);
        std::fs::write(dir.join("toolbar_states_touch.png"), rs2_gfx::png::encode_rgba(w, h, &px)).unwrap();
    }
}
