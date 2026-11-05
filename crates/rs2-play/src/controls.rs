//! 前端開關的狀態與動作。F1 到 F4、F10、F11 的熱鍵和工具列按鈕都走這裡，不依賴 SDL。

use crate::input::Hotkey;
use crate::session::ViewOpts;
use crate::settings::Settings;
use crate::toolbar::{BarState, Button, Toolbar};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Tween,
    Counter,
    Gfx,
    Scale,
    Toolbar,
    Fullscreen,
}

impl From<Hotkey> for Action {
    fn from(h: Hotkey) -> Action {
        match h {
            Hotkey::Tween => Action::Tween,
            Hotkey::Counter => Action::Counter,
            Hotkey::Gfx => Action::Gfx,
            Hotkey::Scale => Action::Scale,
            Hotkey::Toolbar => Action::Toolbar,
            Hotkey::Fullscreen => Action::Fullscreen,
        }
    }
}

impl From<Button> for Action {
    fn from(b: Button) -> Action {
        match b {
            Button::Tween => Action::Tween,
            Button::Counter => Action::Counter,
            Button::Gfx => Action::Gfx,
            Button::Scale => Action::Scale,
            Button::Hide => Action::Toolbar,
        }
    }
}

/// 動作造成的後續。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effect {
    /// 設定有變，要存檔。
    pub save: bool,
    /// 視窗大小要跟著變（倍率或工具列）。
    pub resize: bool,
    /// 全螢幕狀態有變。
    pub fullscreen: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Controls {
    pub settings: Settings,
    pub hd_available: bool,
    pub fullscreen: bool,
}

impl Controls {
    pub fn new(settings: Settings, hd_available: bool) -> Controls {
        Controls { settings, hd_available, fullscreen: false }
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
                s.scale = Settings::next_scale(s.scale);
                Effect { save: true, resize: !self.fullscreen, ..Effect::default() }
            }
            Action::Toolbar => {
                s.toolbar = !s.toolbar;
                Effect { save: true, resize: !self.fullscreen, ..Effect::default() }
            }
            Action::Fullscreen => {
                self.fullscreen = !self.fullscreen;
                Effect { fullscreen: true, ..Effect::default() }
            }
        }
    }

    pub fn view(&self) -> ViewOpts {
        ViewOpts { tween: self.settings.tween, counter: self.settings.counter, hd: self.settings.hd }
    }

    pub fn bar_state(&self) -> BarState {
        BarState {
            tween: self.settings.tween,
            counter: self.settings.counter,
            hd: self.settings.hd,
            hd_available: self.hd_available,
            scale: self.settings.scale,
        }
    }

    pub fn toolbar(&self) -> Toolbar {
        Toolbar::new(self.settings.toolbar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(hd: bool) -> Controls {
        Controls::new(Settings::default(), hd)
    }

    #[test]
    fn toggles_flip_and_ask_for_a_save() {
        let mut k = c(true);
        let e = k.apply(Action::Tween);
        assert!(!k.settings.tween && e.save && !e.resize);
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
    fn scale_cycles_2_3_4_5_and_resizes() {
        let mut k = c(true);
        k.settings.scale = 2;
        let seen: Vec<u32> = (0..5)
            .map(|_| {
                let e = k.apply(Action::Scale);
                assert!(e.resize && e.save);
                k.settings.scale
            })
            .collect();
        assert_eq!(seen, vec![3, 4, 5, 2, 3]);
    }

    #[test]
    fn toolbar_toggle_resizes_the_window() {
        let mut k = c(true);
        assert!(k.toolbar().visible);
        let e = k.apply(Action::Toolbar);
        assert!(!k.toolbar().visible && e.resize && e.save);
        k.apply(Action::Toolbar);
        assert!(k.toolbar().visible);
    }

    #[test]
    fn fullscreen_does_not_resize_or_save() {
        let mut k = c(true);
        let e = k.apply(Action::Fullscreen);
        assert!(k.fullscreen && e.fullscreen && !e.save && !e.resize);
        // 全螢幕中改倍率或工具列不動視窗大小。
        assert!(!k.apply(Action::Scale).resize);
        assert!(!k.apply(Action::Toolbar).resize);
    }

    #[test]
    fn hotkeys_and_buttons_map_to_the_same_actions() {
        use crate::input::Hotkey as H;
        let pairs = [
            (H::Tween, Button::Tween),
            (H::Counter, Button::Counter),
            (H::Gfx, Button::Gfx),
            (H::Scale, Button::Scale),
            (H::Toolbar, Button::Hide),
        ];
        for (h, b) in pairs {
            assert_eq!(Action::from(h), Action::from(b));
        }
        assert_eq!(Action::from(H::Fullscreen), Action::Fullscreen);
    }

    #[test]
    fn view_follows_settings() {
        let mut k = c(true);
        k.apply(Action::Counter);
        k.apply(Action::Tween);
        assert_eq!(k.view(), ViewOpts { tween: false, counter: true, hd: true });
    }
}
