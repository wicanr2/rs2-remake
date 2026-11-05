//! 遊戲選項表（規格 005「選項」）。不依賴 SDL 子系統。
//!
//! 每個選項一列。`implemented` 標示這一版有沒有實作：沒有實作的選項值照樣存進設定、設定畫面照樣列出，
//! 但畫成灰色並標「開發中」，不能切換，也不會影響遊戲。實作完一項就把那一列的 `implemented` 改成 `true`，
//! 不用動設定畫面與設定檔。
//!
//! 現在全部已實作。`field_size`（戰場大小）由 `Settings::rules()` 交給核心的 `Rules::field`，進關時生效，
//! CGA、MGA 不支援加寬（進關用 100%）。
//! 另有：`invincible`（無敵）、`feel`（手感，值存在 `Settings::feel`）、`hit_feedback`（打擊回饋，只在 HD 畫面生效），
//! 以及射擊與操控的四項：`wave_charge`（波動砲充能）、`auto_fire`（按住連射）、`force_pod`（砲座像 Force）、
//! `fine_step`（細步移動）。四項的行為在 `rs2-game` 的 `player/enhanced.rs`，由 `Settings::rules()` 交給核心。
//! 以及 AI 的四項 `ai_aim`、`ai_fire`、`ai_react`、`ai_boss`（核心的 `ai.rs`，由 `Settings::rules()` 交給 `Game::set_rules`）。

use crate::ui::text;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    View,
    Shoot,
    Practice,
    Control,
    Ai,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::View => text::GROUP_VIEW,
            Group::Shoot => text::GROUP_SHOOT,
            Group::Practice => text::GROUP_PRACTICE,
            Group::Control => text::GROUP_CONTROL,
            Group::Ai => text::GROUP_AI,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 關（0）、開（1）。
    Toggle,
    /// 100%（0）、120%（1）、150%（2）。
    Size,
    /// 原版時序（0）、新（1）。值存在 `Settings::feel`。
    Feel,
}

#[derive(Clone, Copy, Debug)]
pub struct OptDef {
    /// 設定檔用的名稱（`opt.<id>`）。
    pub id: &'static str,
    pub group: Group,
    pub label: &'static str,
    pub kind: Kind,
    pub default: u8,
    /// 開著時會改變規則，狀態列要標「強化規則」。
    pub changes_rules: bool,
    /// 這一版有沒有實作。沒有就灰色、標開發中、不能切換、不生效。
    pub implemented: bool,
}

impl OptDef {
    /// 可選的值數。
    pub fn values(&self) -> u8 {
        match self.kind {
            Kind::Toggle | Kind::Feel => 2,
            Kind::Size => 3,
        }
    }

    /// 值的顯示字。
    pub fn value_label(&self, v: u8) -> &'static str {
        match self.kind {
            Kind::Toggle => {
                if v == 0 {
                    text::OFF
                } else {
                    text::ON
                }
            }
            Kind::Size => match v {
                0 => text::VAL_SIZE_100,
                1 => text::VAL_SIZE_120,
                _ => text::VAL_SIZE_150,
            },
            Kind::Feel => {
                if v == 0 {
                    text::VAL_ORIGINAL_TIMING
                } else {
                    text::VAL_NEW
                }
            }
        }
    }

    /// 設定檔的值文字。
    pub fn save_value(&self, v: u8) -> &'static str {
        match self.kind {
            Kind::Toggle | Kind::Feel => {
                if v == 0 {
                    "off"
                } else {
                    "on"
                }
            }
            Kind::Size => match v {
                0 => "100",
                1 => "120",
                _ => "150",
            },
        }
    }

    pub fn parse_value(&self, s: &str) -> Option<u8> {
        match self.kind {
            Kind::Toggle | Kind::Feel => match s.to_ascii_lowercase().as_str() {
                "on" | "1" | "true" | "yes" => Some(1),
                "off" | "0" | "false" | "no" => Some(0),
                _ => None,
            },
            Kind::Size => match s.trim_end_matches('%') {
                "100" => Some(0),
                "120" => Some(1),
                "150" => Some(2),
                _ => None,
            },
        }
    }

    /// 下一個值（循環）。
    pub fn next(&self, v: u8) -> u8 {
        (v + 1) % self.values()
    }

    pub fn prev(&self, v: u8) -> u8 {
        (v + self.values() - 1) % self.values()
    }

    /// 開著（偏離「不改變規則」的預設）。
    pub fn is_active(&self, v: u8) -> bool {
        v != 0
    }
}

macro_rules! opt {
    ($id:expr, $g:expr, $label:expr, $kind:expr, $def:expr, $rules:expr, $impl_:expr) => {
        OptDef { id: $id, group: $g, label: $label, kind: $kind, default: $def, changes_rules: $rules, implemented: $impl_ }
    };
}

/// 規格 005「選項」表，依畫面順序。
pub const OPTIONS: [OptDef; 12] = [
    opt!("field_size", Group::View, text::OPT_FIELD_SIZE, Kind::Size, 0, true, true),
    opt!("wave_charge", Group::Shoot, text::OPT_WAVE_CHARGE, Kind::Toggle, 0, true, true),
    opt!("auto_fire", Group::Shoot, text::OPT_AUTO_FIRE, Kind::Toggle, 0, true, true),
    opt!("force_pod", Group::Shoot, text::OPT_FORCE_POD, Kind::Toggle, 0, true, true),
    opt!("hit_feedback", Group::Shoot, text::OPT_HIT_FEEDBACK, Kind::Toggle, 1, false, true),
    opt!("invincible", Group::Practice, text::OPT_INVINCIBLE, Kind::Toggle, 0, true, true),
    opt!("feel", Group::Control, text::OPT_FEEL, Kind::Feel, 1, false, true),
    opt!("fine_step", Group::Control, text::OPT_FINE_STEP, Kind::Toggle, 0, true, true),
    opt!("ai_aim", Group::Ai, text::OPT_AI_AIM, Kind::Toggle, 0, true, true),
    opt!("ai_fire", Group::Ai, text::OPT_AI_FIRE, Kind::Toggle, 0, true, true),
    opt!("ai_react", Group::Ai, text::OPT_AI_REACT, Kind::Toggle, 0, true, true),
    opt!("ai_boss", Group::Ai, text::OPT_AI_BOSS, Kind::Toggle, 0, true, true),
];

pub const FEEL_INDEX: usize = 6;
pub const INVINCIBLE_INDEX: usize = 5;
pub const HIT_FEEDBACK_INDEX: usize = 4;

pub fn index_of(id: &str) -> Option<usize> {
    OPTIONS.iter().position(|o| o.id == id)
}

/// 全部選項的值。`feel` 那一格不用，它的值在 `Settings::feel`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    vals: [u8; 12],
}

impl Default for Options {
    fn default() -> Options {
        let mut vals = [0u8; 12];
        for (v, d) in vals.iter_mut().zip(OPTIONS.iter()) {
            *v = d.default;
        }
        Options { vals }
    }
}

impl Options {
    pub fn get(&self, i: usize) -> u8 {
        self.vals[i]
    }

    pub fn set(&mut self, i: usize, v: u8) {
        self.vals[i] = v % OPTIONS[i].values();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_spec_005() {
        let ids: Vec<&str> = OPTIONS.iter().map(|o| o.id).collect();
        assert_eq!(
            ids,
            vec!["field_size", "wave_charge", "auto_fire", "force_pod", "hit_feedback", "invincible", "feel", "fine_step", "ai_aim", "ai_fire", "ai_react", "ai_boss"]
        );
        // 規格 005「選項」表：預設值與「改變規則」欄。
        let rules: Vec<bool> = OPTIONS.iter().map(|o| o.changes_rules).collect();
        assert_eq!(rules, vec![true, true, true, true, false, true, false, true, true, true, true, true]);
        for o in &OPTIONS {
            if o.changes_rules {
                assert_eq!(o.default, 0, "{}：改變規則的選項預設關閉", o.id);
            }
        }
        assert_eq!(OPTIONS[index_of("hit_feedback").unwrap()].default, 1, "只影響顯示與聲音的選項預設開啟");
        assert_eq!(OPTIONS[FEEL_INDEX].id, "feel");
        assert_eq!(OPTIONS[INVINCIBLE_INDEX].id, "invincible");
        assert_eq!(OPTIONS[HIT_FEEDBACK_INDEX].id, "hit_feedback");
        assert_eq!(OPTIONS[FEEL_INDEX].default, 1, "手感預設是新");
    }

    #[test]
    fn no_option_is_still_in_development() {
        let todo: Vec<&str> = OPTIONS.iter().filter(|o| !o.implemented).map(|o| o.id).collect();
        assert!(todo.is_empty(), "還在開發中的選項：{todo:?}");
    }

    #[test]
    fn values_cycle_and_roundtrip() {
        for o in &OPTIONS {
            let mut v = o.default;
            for _ in 0..o.values() {
                assert_eq!(o.parse_value(o.save_value(v)), Some(v), "{}", o.id);
                assert_eq!(o.prev(o.next(v)), v);
                assert!(!o.value_label(v).is_empty());
                v = o.next(v);
            }
            assert_eq!(v, o.default, "{} 循環一圈回到原值", o.id);
        }
        let size = &OPTIONS[0];
        assert_eq!(size.parse_value("120%"), Some(1));
        assert_eq!(size.parse_value("130"), None);
        assert_eq!(OPTIONS[1].parse_value("maybe"), None);
        assert_eq!(size.values(), 3);
    }

    #[test]
    fn options_default_and_clamp() {
        let o = Options::default();
        for (i, d) in OPTIONS.iter().enumerate() {
            assert_eq!(o.get(i), d.default);
        }
        let mut o = o;
        o.set(1, 7);
        assert!(o.get(1) < 2);
        assert!(OPTIONS[1].is_active(1) && !OPTIONS[1].is_active(0));
    }
}
