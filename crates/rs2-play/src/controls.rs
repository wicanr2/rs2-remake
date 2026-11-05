//! 前端開關的狀態與動作。F1 到 F6、F10、F11 的熱鍵、工具列按鈕與設定畫面都走這裡，不依賴 SDL。
//!
//! 前端暫停有兩個來源：按了暫停（`paused`），或設定畫面開著（`panel_open`）。前端暫停時 `Pacer` 不再步進，
//! 不送 Esc 給遊戲；鍵盤的 Esc 照原版送進遊戲，那是原版的暫停。

use crate::input::Hotkey;
use crate::mode::DisplayMode;
use crate::options::OPTIONS;
use crate::session::ViewOpts;
use crate::settings::Settings;
use crate::toolbar::{BarState, Button, Toolbar};
use crate::ui::hud::HudFlags;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Tween,
    Counter,
    Gfx,
    Scale,
    Feel,
    /// 顯示模式循環（VGA、EGA、CGA、MGA）。
    Mode,
    /// 工具列顯示或隱藏。
    Toolbar,
    Fullscreen,
    About,
    Quit,
    /// 前端暫停。
    Pause,
    /// 開始／繼續：解除暫停，順便關設定畫面。
    Resume,
    /// 暫停與繼續輪流（手把 Start、觸控的暫停鈕）。
    TogglePause,
    /// 開設定畫面的手把或鍵盤頁。
    OpenControls,
    /// 開設定畫面的遊戲頁。
    OpenGame,
    /// F10：開或關設定畫面。
    TogglePanel,
    ClosePanel,
}

impl Action {
    /// 熱鍵對應的動作。F8 換頁只在設定畫面裡有意義，由設定畫面自己處理。
    pub fn from_hotkey(h: Hotkey) -> Option<Action> {
        Some(match h {
            Hotkey::Tween => Action::Tween,
            Hotkey::Counter => Action::Counter,
            Hotkey::Gfx => Action::Gfx,
            Hotkey::Scale => Action::Scale,
            Hotkey::Feel => Action::Feel,
            Hotkey::Mode => Action::Mode,
            Hotkey::Settings => Action::TogglePanel,
            Hotkey::Fullscreen => Action::Fullscreen,
            Hotkey::Page => return None,
        })
    }

    /// 工具列按鈕對應的動作。
    pub fn from_button(b: Button) -> Option<Action> {
        Some(match b {
            Button::Start => Action::Resume,
            Button::Pause => Action::Pause,
            Button::Controls => Action::OpenControls,
            Button::Game => Action::OpenGame,
            Button::Fullscreen => Action::Fullscreen,
            Button::Gfx => Action::Gfx,
            Button::Mode => Action::Mode,
            Button::About => Action::About,
            Button::Quit => Action::Quit,
            Button::Hide => Action::Toolbar,
        })
    }
}

/// 視窗大小要怎麼跟著變。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Resize {
    #[default]
    None,
    /// 倍率變了：視窗改成新倍率的大小。
    Scale,
    /// 工具列顯示或隱藏：寬度不變，高度加減工具列。
    Toolbar,
}

/// 設定畫面開哪一頁。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelOpen {
    /// 手把頁（有偵測到手把）或鍵盤頁。
    Controls,
    Game,
    /// 上次開的頁。
    Last,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelChange {
    #[default]
    None,
    Open(PanelOpen),
    Close,
}

/// 動作造成的後續。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effect {
    /// 設定有變，要存檔。
    pub save: bool,
    pub resize: Resize,
    /// 全螢幕狀態有變。
    pub fullscreen: bool,
    /// 要開授權頁面。
    pub about: bool,
    /// 要離開。
    pub quit: bool,
    /// 設定畫面要開或關。
    pub panel: PanelChange,
}

impl Effect {
    /// 把 `o` 併進來：旗標取聯集，視窗與設定畫面的變動以後來的為準。
    pub fn merge(&mut self, o: Effect) {
        self.save |= o.save;
        self.fullscreen |= o.fullscreen;
        self.about |= o.about;
        self.quit |= o.quit;
        if o.resize != Resize::None {
            self.resize = o.resize;
        }
        if o.panel != PanelChange::None {
            self.panel = o.panel;
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Controls {
    pub settings: Settings,
    pub hd_available: bool,
    /// 單色圖庫（CGA、MGA）載入成功。為假時循環跳過 CGA、MGA。用 `set_mono_available` 設。
    pub mono_available: bool,
    pub fullscreen: bool,
    /// 手機版的工具列，視窗大小固定。
    pub mobile: bool,
    /// 手機版每 dp 幾個像素。
    pub dp: f32,
    /// 按了暫停。
    pub paused: bool,
    /// 設定畫面開著。
    pub panel_open: bool,
}

impl Controls {
    pub fn new(settings: Settings, hd_available: bool) -> Controls {
        Controls { settings, hd_available, mono_available: true, fullscreen: false, mobile: false, dp: crate::toolbar::DEFAULT_DP, paused: false, panel_open: false }
    }

    /// 告知單色圖庫能不能用。不能用時，設定裡存的 CGA、MGA 退回 VGA（回傳真，呼叫端要說明原因）。
    pub fn set_mono_available(&mut self, ok: bool) -> bool {
        self.mono_available = ok;
        if !ok && self.settings.mode.is_mono() {
            self.settings.mode = DisplayMode::Vga;
            return true;
        }
        false
    }

    /// 前端暫停：`Pacer` 不步進，輸入不送進遊戲。
    pub fn frontend_paused(&self) -> bool {
        self.paused || self.panel_open
    }

    pub fn apply(&mut self, a: Action) -> Effect {
        let s = &mut self.settings;
        match a {
            Action::Tween => {
                s.tween = !s.tween;
                Effect { save: true, ..Effect::default() }
            }
            Action::Counter => {
                s.counter = !s.counter;
                Effect { save: true, ..Effect::default() }
            }
            Action::Gfx => {
                if !self.hd_available {
                    return Effect::default();
                }
                s.hd = !s.hd;
                Effect { save: true, ..Effect::default() }
            }
            Action::Scale => {
                // 手機版沒有倍率。
                if self.mobile {
                    return Effect::default();
                }
                s.scale = Settings::next_scale(s.scale);
                Effect { save: true, resize: if self.fullscreen { Resize::None } else { Resize::Scale }, ..Effect::default() }
            }
            Action::Feel => {
                s.feel = !s.feel;
                Effect { save: true, ..Effect::default() }
            }
            Action::Mode => {
                s.mode = s.mode.next_available(self.mono_available);
                Effect { save: true, ..Effect::default() }
            }
            Action::Toolbar => {
                s.toolbar = !s.toolbar;
                Effect { save: true, resize: if self.fullscreen || self.mobile { Resize::None } else { Resize::Toolbar }, ..Effect::default() }
            }
            Action::About => Effect { about: true, ..Effect::default() },
            Action::Quit => Effect { quit: true, ..Effect::default() },
            Action::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                Effect { fullscreen: true, ..Effect::default() }
            }
            Action::Pause => {
                self.paused = true;
                Effect::default()
            }
            Action::Resume => {
                self.paused = false;
                self.close_panel()
            }
            Action::TogglePause => {
                // 設定畫面開著時暫停鈕不作用，設定畫面自己有關閉的方法。
                if !self.panel_open {
                    self.paused = !self.paused;
                }
                Effect::default()
            }
            Action::OpenControls => self.open_panel(PanelOpen::Controls),
            Action::OpenGame => self.open_panel(PanelOpen::Game),
            Action::TogglePanel => {
                if self.panel_open { self.close_panel() } else { self.open_panel(PanelOpen::Last) }
            }
            Action::ClosePanel => self.close_panel(),
        }
    }

    fn open_panel(&mut self, which: PanelOpen) -> Effect {
        self.panel_open = true;
        Effect { panel: PanelChange::Open(which), ..Effect::default() }
    }

    fn close_panel(&mut self) -> Effect {
        if !self.panel_open {
            return Effect::default();
        }
        self.panel_open = false;
        Effect { panel: PanelChange::Close, ..Effect::default() }
    }

    pub fn view(&self) -> ViewOpts {
        ViewOpts { tween: self.settings.tween, counter: self.settings.counter, hd: self.settings.hd, feel: self.settings.feel, mode: self.settings.mode }
    }

    /// 工具列的狀態。`notice` 由呼叫端填。
    pub fn bar_state(&self) -> BarState {
        BarState {
            hd: self.settings.hd,
            hd_available: self.hd_available,
            paused: self.frontend_paused(),
            rules: self.settings.rules_enhanced(),
            god: self.settings.invincible(),
            mobile: self.mobile,
            mode: self.settings.mode,
            mono_available: self.mono_available,
            notice: String::new(),
            hover: None,
            down: None,
        }
    }

    /// HD 介面的 HUD 要顯示的前端狀態（規格 005「HUD」）：右端標籤與充能條。幀計數的值由繪製端補。
    pub fn hud_flags(&self) -> HudFlags {
        let charge = crate::options::index_of("wave_charge").is_some_and(|i| OPTIONS[i].implemented && self.settings.opt_value(i) != 0);
        HudFlags { rules: self.settings.rules_enhanced(), god: self.settings.invincible(), charge_bar: charge, counter: None }
    }

    pub fn toolbar(&self) -> Toolbar {
        if self.mobile { Toolbar::mobile(self.settings.toolbar).with_dp(self.dp) } else { Toolbar::new(self.settings.toolbar) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::INVINCIBLE_INDEX;

    fn c(hd: bool) -> Controls {
        Controls::new(Settings::default(), hd)
    }

    #[test]
    fn toggles_flip_and_ask_for_a_save() {
        let mut k = c(true);
        let e = k.apply(Action::Tween);
        assert!(!k.settings.tween && e.save && e.resize == Resize::None);
        k.apply(Action::Tween);
        assert!(k.settings.tween);
        let e = k.apply(Action::Counter);
        assert!(k.settings.counter && e.save);
        let e = k.apply(Action::Gfx);
        assert!(!k.settings.hd && e.save);
        assert!(k.apply(Action::Gfx).save && k.settings.hd);
    }

    #[test]
    fn gfx_is_a_no_op_without_hd_assets() {
        let mut k = c(false);
        let before = k.settings;
        assert_eq!(k.apply(Action::Gfx), Effect::default());
        assert_eq!(k.settings, before);
        assert!(!k.bar_state().hd_available);
    }

    #[test]
    fn scale_cycles_2_3_4_5_and_resizes_but_not_on_phones() {
        let mut k = c(true);
        k.settings.scale = 2;
        let seen: Vec<u32> = (0..5)
            .map(|_| {
                let e = k.apply(Action::Scale);
                assert!(e.resize == Resize::Scale && e.save);
                k.settings.scale
            })
            .collect();
        assert_eq!(seen, vec![3, 4, 5, 2, 3]);
        let mut p = c(true);
        p.mobile = true;
        let before = p.settings.scale;
        assert_eq!(p.apply(Action::Scale), Effect::default(), "手機版沒有倍率");
        assert_eq!(p.settings.scale, before);
    }

    #[test]
    fn toolbar_toggle_resizes_the_window() {
        let mut k = c(true);
        assert!(k.toolbar().visible);
        let e = k.apply(Action::Toolbar);
        assert!(!k.toolbar().visible && e.resize == Resize::Toolbar && e.save);
        k.apply(Action::Toolbar);
        assert!(k.toolbar().visible);
    }

    #[test]
    fn fullscreen_does_not_resize_or_save() {
        let mut k = c(true);
        let e = k.apply(Action::Fullscreen);
        assert!(k.fullscreen && e.fullscreen && !e.save && e.resize == Resize::None);
        // 全螢幕中改倍率或工具列不動視窗大小。
        assert_eq!(k.apply(Action::Scale).resize, Resize::None);
        assert_eq!(k.apply(Action::Toolbar).resize, Resize::None);
    }

    #[test]
    fn hotkeys_and_buttons_map_to_the_same_actions() {
        use crate::input::Hotkey as H;
        assert_eq!(Action::from_hotkey(H::Tween), Some(Action::Tween));
        assert_eq!(Action::from_hotkey(H::Counter), Some(Action::Counter));
        assert_eq!(Action::from_hotkey(H::Gfx), Some(Action::Gfx));
        assert_eq!(Action::from_hotkey(H::Gfx), Action::from_button(Button::Gfx));
        assert_eq!(Action::from_hotkey(H::Scale), Some(Action::Scale));
        assert_eq!(Action::from_hotkey(H::Feel), Some(Action::Feel));
        assert_eq!(Action::from_hotkey(H::Mode), Some(Action::Mode), "F6 切換顯示模式");
        assert_eq!(Action::from_hotkey(H::Mode), Action::from_button(Button::Mode), "熱鍵與按鈕同一個動作");
        assert_eq!(Action::from_hotkey(H::Settings), Some(Action::TogglePanel), "F10 開設定畫面");
        assert_eq!(Action::from_hotkey(H::Fullscreen), Some(Action::Fullscreen));
        assert_eq!(Action::from_hotkey(H::Fullscreen), Action::from_button(Button::Fullscreen));
        assert_eq!(Action::from_hotkey(H::Page), None, "F8 由設定畫面處理");
        assert_eq!(Action::from_button(Button::Start), Some(Action::Resume));
        assert_eq!(Action::from_button(Button::Pause), Some(Action::Pause));
        assert_eq!(Action::from_button(Button::Controls), Some(Action::OpenControls));
        assert_eq!(Action::from_button(Button::Game), Some(Action::OpenGame));
        assert_eq!(Action::from_button(Button::Hide), Some(Action::Toolbar));
    }

    #[test]
    fn mobile_toolbar_never_resizes_the_window() {
        let mut k = c(true);
        k.mobile = true;
        let e = k.apply(Action::Toolbar);
        assert!(e.save && e.resize == Resize::None);
        assert!(k.toolbar().mobile);
        assert!(k.bar_state().mobile);
        assert!(k.apply(Action::About).about);
        assert!(k.apply(Action::Quit).quit);
        assert_eq!(Action::from_button(Button::About), Some(Action::About));
        assert_eq!(Action::from_button(Button::Quit), Some(Action::Quit));
        k.dp = 3.0;
        assert_eq!(k.toolbar().dp, 3.0);
    }

    #[test]
    fn view_follows_settings() {
        let mut k = c(true);
        k.apply(Action::Counter);
        k.apply(Action::Tween);
        assert_eq!(k.view(), ViewOpts { tween: false, counter: true, hd: true, feel: true, mode: DisplayMode::Vga });
        let e = k.apply(Action::Feel);
        assert!(e.save && e.resize == Resize::None);
        assert!(!k.view().feel);
        k.apply(Action::Mode);
        assert_eq!(k.view().mode, DisplayMode::Ega, "顯示模式隨設定");
    }

    #[test]
    fn mode_action_cycles_saves_and_reaches_the_view_and_the_bar() {
        let mut k = c(true);
        let mut seen = vec![k.settings.mode];
        for _ in 0..4 {
            let e = k.apply(Action::Mode);
            assert!(e.save && e.resize == Resize::None && !e.quit, "只要存檔，不動視窗");
            seen.push(k.settings.mode);
            assert_eq!(k.view().mode, k.settings.mode);
            assert_eq!(k.bar_state().mode, k.settings.mode);
        }
        use DisplayMode::*;
        assert_eq!(seen, vec![Vga, Ega, Cga, Mga, Vga]);
        // 按鈕與熱鍵走同一條路。
        let via_button = Action::from_button(Button::Mode).unwrap();
        k.apply(via_button);
        assert_eq!(k.settings.mode, Ega);
    }

    #[test]
    fn mode_cycle_skips_mono_when_the_bank_is_unavailable() {
        let mut k = c(true);
        assert!(k.mono_available && k.bar_state().mono_available, "預設單色圖庫可用");
        assert!(!k.set_mono_available(false), "目前是 VGA，不用退回");
        assert!(!k.bar_state().mono_available);
        let seen: Vec<DisplayMode> = (0..4).map(|_| { k.apply(Action::Mode); k.settings.mode }).collect();
        use DisplayMode::*;
        assert_eq!(seen, vec![Ega, Vga, Ega, Vga], "只在 VGA 與 EGA 之間循環");
        // 設定裡存的 CGA 在單色圖庫不能用時退回 VGA。
        let mut m = c(true);
        m.settings.mode = Mga;
        assert!(m.set_mono_available(false), "要回報退回了");
        assert_eq!(m.settings.mode, Vga);
        let mut ok = c(true);
        ok.settings.mode = Cga;
        assert!(!ok.set_mono_available(true) && ok.settings.mode == Cga, "可用時不動");
    }

    #[test]
    fn pause_resume_and_toggle() {
        let mut k = c(true);
        assert!(!k.frontend_paused());
        k.apply(Action::Pause);
        assert!(k.paused && k.frontend_paused() && k.bar_state().paused);
        k.apply(Action::Pause);
        assert!(k.paused, "再按暫停仍是暫停");
        let e = k.apply(Action::Resume);
        assert!(!k.paused && !k.frontend_paused() && e.panel == PanelChange::None);
        k.apply(Action::TogglePause);
        assert!(k.paused);
        k.apply(Action::TogglePause);
        assert!(!k.paused);
    }

    #[test]
    fn panel_pauses_the_front_end_and_restores_the_previous_state() {
        let mut k = c(true);
        let e = k.apply(Action::OpenControls);
        assert_eq!(e.panel, PanelChange::Open(PanelOpen::Controls));
        assert!(k.panel_open && k.frontend_paused() && !k.paused, "設定畫面開著時前端暫停，但不是按暫停");
        assert!(k.bar_state().paused);
        // 設定畫面開著時，暫停鈕（手把 Start、觸控）不作用。
        k.apply(Action::TogglePause);
        assert!(!k.paused);
        let e = k.apply(Action::ClosePanel);
        assert_eq!(e.panel, PanelChange::Close);
        assert!(!k.frontend_paused(), "關閉後回到開啟前的狀態");
        // 先暫停再開設定畫面：關閉後仍然暫停。
        k.apply(Action::Pause);
        k.apply(Action::OpenGame);
        k.apply(Action::ClosePanel);
        assert!(k.frontend_paused() && k.paused);
        // 開始／繼續：解除暫停並關設定畫面。
        k.apply(Action::OpenGame);
        let e = k.apply(Action::Resume);
        assert_eq!(e.panel, PanelChange::Close);
        assert!(!k.frontend_paused());
        // F10 輪流開關。
        assert_eq!(k.apply(Action::TogglePanel).panel, PanelChange::Open(PanelOpen::Last));
        assert_eq!(k.apply(Action::TogglePanel).panel, PanelChange::Close);
        assert_eq!(k.apply(Action::ClosePanel), Effect::default(), "沒開著就沒有後續");
    }

    #[test]
    fn effects_merge_by_union_and_later_window_changes_win() {
        let mut e = Effect { save: true, ..Effect::default() };
        e.merge(Effect { resize: Resize::Scale, ..Effect::default() });
        e.merge(Effect { quit: true, panel: PanelChange::Close, ..Effect::default() });
        e.merge(Effect::default());
        assert!(e.save && e.quit && e.resize == Resize::Scale && e.panel == PanelChange::Close && !e.about && !e.fullscreen);
    }

    #[test]
    fn bar_state_shows_rules_and_god_only_for_implemented_options() {
        let mut k = c(true);
        assert!(!k.bar_state().rules && !k.bar_state().god);
        // 沒實作的選項（波動砲充能）寫成開也不顯示強化規則。
        k.settings.set_opt_value(1, 1);
        assert!(!k.bar_state().rules);
        k.settings.set_opt_value(INVINCIBLE_INDEX, 1);
        let b = k.bar_state();
        assert!(b.rules && b.god, "無敵會改變規則，所以兩個都亮");
        // 手感不改變規則。
        let mut k2 = c(true);
        k2.apply(Action::Feel);
        assert!(!k2.bar_state().rules);
    }
}
