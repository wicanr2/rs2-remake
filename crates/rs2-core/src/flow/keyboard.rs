//! 鍵盤中斷（INT 9，`1A83Dh`）：按鍵狀態表、開火與轉向旗標、開發者秘技、搖桿旗標、音效開關。
//!
//! 按鍵狀態表與最近按下的鍵（ASCII）仍在資料段，標題與片尾直接讀寫，經 `mirror.rs` 的 `key_down`、`last_key`
//! 一族存取。表的內容：0 沒按過，1 按住，2 剛放開。其餘輸入狀態（開火旗標、秘技、搖桿旗標、最後的掃描碼）在 [`Controls`]。

use crate::game::Core;
use crate::mirror::Key;

/// 旗標的兩個值：`Y` 成立，`N` 不成立。
pub(crate) const FLAG_ON: u8 = b'Y';
pub(crate) const FLAG_OFF: u8 = b'N';

/// 掃描碼的最高位元是 1 表示放開。
const BREAK_BIT: u8 = 0x80;
/// 開音效開關之前要累積的 tick 數。
const SOUND_TOGGLE_MIN_TICKS: u16 = 5;

/// 每位玩家三個旗標，INT 9 設成 `Y`，玩家邏輯處理後清成 `N`。位元組寬，值不限於 `Y`、`N`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FireFlags {
    /// 開火鍵放開過，下一次按下算新按（玩家 1 `byte_273A7`，玩家 2 `byte_273A8`）。
    pub new_press: [u8; 2],
    /// 本幀有開火鍵事件，含按住時的自動重複（`byte_273A9`、`byte_273AB`）。
    pub event: [u8; 2],
    /// 轉向鍵被按下（`byte_273AA`、`byte_273AC`）。
    pub turn: [u8; 2],
}

/// 開發者秘技。按 1、2、3 各一次再按小鍵盤 5 解鎖，解鎖後按空白鍵補滿血、按小鍵盤 `*` 開無敵，Alt 清除。
/// 欄位是字組寬，值不限於 `Y`、`N`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Cheat {
    /// `word_2739D`：已解鎖（低位元組 `Y`）。
    pub unlocked: u16,
    /// `word_2739F`、`word_273A1`、`word_273A3`：1、2、3 鍵各按了幾次。
    pub presses: [u16; 3],
    /// `word_273A5`：無敵（低位元組 `Y`）。`Game::set_invincible` 也寫它。
    pub invincible: u16,
}

/// 輸入領域的狀態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Controls {
    /// `byte_2742C`：最後一個按下（非放開）的鍵的掃描碼，放開時歸 0。沒有人讀。
    pub last_scan: u8,
    pub fire: FireFlags,
    pub cheat: Cheat,
    /// `byte_1DAAC`：搖桿與滑鼠輪詢開關（Ctrl+J 開、Ctrl+K 關）。重製不支援輪詢，旗標照原版維護。
    pub joymouse: u8,
}

impl Core {
    /// 鍵盤中斷處理一個掃描碼。
    pub(crate) fn handle_scancode(&mut self, scan: u8) {
        self.flow.controls.last_scan = scan;
        if scan & BREAK_BIT == 0 {
            let ascii = self.ascii_of(scan);
            self.set_last_key(ascii);
            self.note_key_down(scan);
        } else {
            self.flow.controls.last_scan = 0;
            self.note_key_up(scan & !BREAK_BIT);
        }
        self.update_cheats();
        self.latch_fire_flags();
        self.toggle_joymouse();
        self.toggle_sound();
    }

    /// `sub_1A8F4`。
    fn update_cheats(&mut self) {
        if self.take_key_press(Key::Alt) {
            let cheat = &mut self.flow.controls.cheat;
            cheat.presses = [0; 3];
            cheat.invincible = u16::from(FLAG_OFF);
        }
        for (n, key) in [Key::Digit1, Key::Digit2, Key::Digit3].into_iter().enumerate() {
            if self.take_key_press(key) {
                self.flow.controls.cheat.presses[n] += 1;
            }
        }
        if self.take_key_press(Key::Keypad5) && self.flow.controls.cheat.presses == [1, 2, 3] {
            self.flow.controls.cheat.unlocked = u16::from(FLAG_ON);
        }
        if self.take_key_press(Key::Space) && self.flow.controls.cheat.unlocked == u16::from(FLAG_ON) {
            self.refill_health();
            self.flow.controls.cheat.unlocked = u16::from(FLAG_OFF);
        }
        if self.take_key_press(Key::KeypadStar) && self.flow.controls.cheat.unlocked == u16::from(FLAG_ON) {
            self.flow.controls.cheat.invincible = u16::from(FLAG_ON);
        }
    }

    /// `sub_1A9B5`：把按鍵狀態轉成開火與轉向旗標。玩家 1 開火是右 Shift、轉向是 `/`；玩家 2 是左 Shift 與 Z。
    /// 轉向鍵處理後改寫成 2，所以按住不會連續轉向。
    fn latch_fire_flags(&mut self) {
        // 原版的檢查順序是新按（兩位）、事件（兩位）、轉向（兩位）。六個旗標由不同的鍵與狀態決定，互不相干，
        // 這裡改成一位玩家一組，結果相同。
        for (who, fire_key, turn_key) in [(0, Key::RightShift, Key::Slash), (1, Key::LeftShift, Key::Z)] {
            if self.key_released(fire_key) {
                self.flow.controls.fire.new_press[who] = FLAG_ON;
            }
            if self.key_down(fire_key) {
                self.flow.controls.fire.event[who] = FLAG_ON;
            }
            if self.key_down(turn_key) {
                self.flow.controls.fire.turn[who] = FLAG_ON;
                self.mark_key_released(turn_key);
            }
        }
    }

    /// Ctrl+J、Ctrl+K：搖桿旗標。
    fn toggle_joymouse(&mut self) {
        if !self.key_down(Key::Ctrl) {
            return;
        }
        if self.take_key_press(Key::J) {
            self.flow.controls.joymouse = FLAG_ON;
        }
        if self.take_key_press(Key::K) {
            self.flow.controls.joymouse = FLAG_OFF;
        }
    }

    /// `sub_1A891`：S 鍵切換音效。要累積 5 個 tick 才生效，標題期間（`byte_2739A` 是 `N`）不生效。
    fn toggle_sound(&mut self) {
        if self.flow.clock.since_sound_toggle < SOUND_TOGGLE_MIN_TICKS || !self.sound_key_enabled() || !self.key_down(Key::S) {
            return;
        }
        self.toggle_sound_mute();
        self.release_key(Key::S);
        self.clear_last_key();
        self.flow.clock.since_sound_toggle = 0;
    }

    /// 無敵秘技是否開著（原版 `word_273A5` 的低位元組是 `Y`）。
    pub(crate) fn invincible_cheat(&self) -> bool {
        self.flow.controls.cheat.invincible as u8 == FLAG_ON
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Fixes, Start};

    fn core() -> Core {
        let image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
        Core::new(&image, &[], Start::Demo, Fixes::NONE)
    }

    const DIGIT_1: u8 = 0x02;
    const DIGIT_2: u8 = 0x03;
    const DIGIT_3: u8 = 0x04;
    const KEYPAD_5: u8 = 0x4C;
    const SPACE: u8 = 0x39;
    const KEYPAD_STAR: u8 = 0x37;
    const ALT: u8 = 0x38;

    fn tap(c: &mut Core, scan: u8) {
        c.handle_scancode(scan);
        c.handle_scancode(scan | BREAK_BIT);
    }

    #[test]
    fn a_make_code_sets_the_key_the_last_scan_and_the_ascii_and_a_break_code_marks_release() {
        let mut c = core();
        c.handle_scancode(0x30);
        assert!(c.key_held(0x30));
        assert_eq!(c.flow.controls.last_scan, 0x30);
        c.handle_scancode(0x30 | BREAK_BIT);
        assert!(!c.key_held(0x30), "放開後是 2，不算按住");
        assert_eq!(c.flow.controls.last_scan, 0, "放開時最後的掃描碼歸 0");
        assert_eq!(c.key_raw(0x30), 2);
    }

    #[test]
    fn the_fire_key_latches_new_press_on_release_and_event_while_held() {
        let mut c = core();
        c.handle_scancode(0x36);
        assert_eq!(c.flow.controls.fire.event, [FLAG_ON, 0]);
        assert_eq!(c.flow.controls.fire.new_press, [0, 0], "按下時還不算新按");
        c.handle_scancode(0x36 | BREAK_BIT);
        assert_eq!(c.flow.controls.fire.new_press, [FLAG_ON, 0], "放開後下一次才算新按");
        // 玩家 2 用左 Shift。
        c.handle_scancode(0x2A);
        assert_eq!(c.flow.controls.fire.event[1], FLAG_ON);
    }

    #[test]
    fn the_turn_key_sets_the_flag_once_and_turns_into_released() {
        let mut c = core();
        c.handle_scancode(0x35);
        assert_eq!(c.flow.controls.fire.turn, [FLAG_ON, 0]);
        assert_eq!(c.key_raw(0x35), 2, "處理後改寫成 2，按住不會連續轉向");
        c.handle_scancode(0x2C);
        assert_eq!(c.flow.controls.fire.turn, [FLAG_ON, FLAG_ON]);
    }

    #[test]
    fn the_cheat_needs_1_2_3_then_keypad_5_before_space_refills_health() {
        let mut c = core();
        c.pl.ship[0].hp = 0x03;
        tap(&mut c, SPACE);
        assert_eq!(c.pl.ship[0].hp, 0x03, "沒解鎖");
        for scan in [DIGIT_1, DIGIT_2, DIGIT_2, DIGIT_3, DIGIT_3, DIGIT_3] {
            tap(&mut c, scan);
        }
        assert_eq!(c.flow.controls.cheat.presses, [1, 2, 3]);
        tap(&mut c, KEYPAD_5);
        assert_eq!(c.flow.controls.cheat.unlocked, u16::from(FLAG_ON));
        tap(&mut c, SPACE);
        assert_eq!(c.pl.ship[0].hp, 0xFF);
        assert_eq!(c.flow.controls.cheat.unlocked, u16::from(FLAG_OFF), "補血用掉解鎖");
        assert!(!c.invincible_cheat());
    }

    #[test]
    fn the_cheat_only_counts_the_presses_and_ignores_their_order() {
        // 機器碼（`1A94Ch` 至 `1A96Eh`）只比較三個計數是 1、2、3，按鍵順序不拘。
        let mut c = core();
        for scan in [DIGIT_3, DIGIT_2, DIGIT_3, DIGIT_1, DIGIT_2, DIGIT_3, KEYPAD_5] {
            tap(&mut c, scan);
        }
        assert_eq!(c.flow.controls.cheat.unlocked, u16::from(FLAG_ON));
        // 計數不對（2、2、3）就不解鎖。
        let mut c = core();
        for scan in [DIGIT_2, DIGIT_2, DIGIT_3, DIGIT_3, DIGIT_3, KEYPAD_5] {
            tap(&mut c, scan);
        }
        assert_ne!(c.flow.controls.cheat.unlocked, u16::from(FLAG_ON));
    }

    #[test]
    fn keypad_star_after_unlocking_turns_on_invincibility_and_alt_clears_it() {
        let mut c = core();
        for scan in [DIGIT_1, DIGIT_2, DIGIT_2, DIGIT_3, DIGIT_3, DIGIT_3, KEYPAD_5, KEYPAD_STAR] {
            tap(&mut c, scan);
        }
        assert!(c.invincible_cheat());
        tap(&mut c, ALT);
        assert!(!c.invincible_cheat());
        assert_eq!(c.flow.controls.cheat.presses, [0; 3], "Alt 清掉三個計數");
        assert_eq!(c.flow.controls.cheat.unlocked, u16::from(FLAG_ON), "Alt 不動解鎖旗標");
    }

    #[test]
    fn ctrl_j_and_ctrl_k_set_the_joystick_flag_only_while_ctrl_is_held() {
        let mut c = core();
        tap(&mut c, 0x24);
        assert_ne!(c.flow.controls.joymouse, FLAG_ON, "沒按 Ctrl");
        c.handle_scancode(0x1D);
        c.handle_scancode(0x24);
        assert_eq!(c.flow.controls.joymouse, FLAG_ON);
        c.handle_scancode(0x25);
        assert_eq!(c.flow.controls.joymouse, FLAG_OFF);
    }

    #[test]
    fn the_sound_key_needs_five_ticks_and_an_enabled_stage() {
        let mut c = core();
        let muted = |c: &Core| c.sound_muted();
        c.set_sound_key_enabled(true);
        c.flow.clock.since_sound_toggle = 4;
        c.handle_scancode(0x1F);
        assert!(!muted(&c), "不到 5 個 tick");
        c.flow.clock.since_sound_toggle = 5;
        c.set_sound_key_enabled(false);
        c.handle_scancode(0x1F | BREAK_BIT);
        c.handle_scancode(0x1F);
        assert!(!muted(&c), "標題期間不生效");
        c.set_sound_key_enabled(true);
        c.handle_scancode(0x1F | BREAK_BIT);
        c.handle_scancode(0x1F);
        assert!(muted(&c));
        assert_eq!(c.flow.clock.since_sound_toggle, 0);
        assert_eq!(c.last_key(), 0, "切換時清掉最近的按鍵");
        assert_eq!(c.key_raw(0x1F), 0, "S 鍵狀態清 0");
    }
}
