//! 鍵盤中斷（INT 9，`1A83Dh`）：按鍵狀態表、開火與轉向旗標、開發者秘技、搖桿旗標、音效開關。
//!
//! 按鍵狀態表（`Controls::keys`）：0 沒按過，1 按住，2 剛放開。最近按下的鍵（ASCII）、開火旗標、秘技、
//! 搖桿旗標與最後的掃描碼也在 [`Controls`]。

use crate::game::Core;
use crate::tables::ASCII_ENTRIES;

/// 旗標的兩個值：`Y` 成立，`N` 不成立。
pub(crate) const FLAG_ON: u8 = b'Y';
pub(crate) const FLAG_OFF: u8 = b'N';

/// 掃描碼的最高位元是 1 表示放開。
const BREAK_BIT: u8 = 0x80;
/// 按鍵表的值：1 按住，2 剛放開。
const KEY_HELD: u8 = 1;
const KEY_RELEASED: u8 = 2;
/// 開音效開關之前要累積的 tick 數。
const SOUND_TOGGLE_MIN_TICKS: u16 = 5;

/// 每位玩家三個旗標，INT 9 設成 `Y`，玩家邏輯處理後清成 `N`。位元組寬，值不限於 `Y`、`N`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FireFlags {
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
pub struct Cheat {
    /// `word_2739D`：已解鎖（低位元組 `Y`）。
    pub unlocked: u16,
    /// `word_2739F`、`word_273A1`、`word_273A3`：1、2、3 鍵各按了幾次。
    pub presses: [u16; 3],
    /// `word_273A5`：無敵（低位元組 `Y`）。`Game::set_invincible` 也寫它。
    pub invincible: u16,
}

/// 按鍵狀態表的項數：掃描碼 0 至 5Eh。真實的鍵盤沒有更大的掃描碼；原版的表之後是檔名工作區，
/// 5Fh 以上的掃描碼只會寫壞它（重製忽略）。
pub(crate) const KEY_ENTRIES: usize = 0x5F;

/// 按鍵狀態表，以掃描碼索引，每鍵一個位元組：1 按住，2 剛放開，0 沒按過或已消耗。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyTable(pub [u8; KEY_ENTRIES]);

impl Default for KeyTable {
    fn default() -> KeyTable {
        KeyTable([0; KEY_ENTRIES])
    }
}

/// 輸入領域的狀態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    /// 按鍵狀態表（INT 9 維護，標題與片尾直接讀寫）。
    pub keys: KeyTable,
    /// `byte_27422`：最近一次按下的鍵（ASCII），0 表示沒有。標題的各段動畫每步看一次它是不是 0。
    pub last_key: u8,
    /// `byte_2742C`：最後一個按下（非放開）的鍵的掃描碼，放開時歸 0。沒有人讀。
    pub last_scan: u8,
    pub fire: FireFlags,
    pub cheat: Cheat,
    /// `byte_1DAAC`：搖桿與滑鼠輪詢開關（Ctrl+J 開、Ctrl+K 關）。重製不支援輪詢，旗標照原版維護。
    pub joymouse: u8,
    /// `byte_2739A`：音效開關鍵（INT 9 的 `sub_1A891`）是否生效，`N` 不生效。標題期間 `N`，進關卡改 `Y`，離開改 `N`。
    pub sound_key: u8,
}

/// 標題、片尾與關卡讀的鍵。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Up,
    Down,
    Left,
    Right,
    /// 標題與片尾的選單用，關卡不讀。
    #[allow(dead_code)]
    Enter,
    Space,
    Q,
    R,
    Esc,
    Digit1,
    Digit2,
    Digit3,
    W,
    Ctrl,
    S,
    D,
    J,
    K,
    LeftShift,
    Z,
    C,
    Slash,
    RightShift,
    KeypadStar,
    Alt,
    Keypad5,
}

impl Key {
    /// set 1 掃描碼，也是按鍵表的索引。
    pub(crate) const fn scancode(self) -> u16 {
        match self {
            Key::Up => 0x48,
            Key::Down => 0x50,
            Key::Left => 0x4B,
            Key::Right => 0x4D,
            Key::Enter => 0x1C,
            Key::Space => 0x39,
            Key::Q => 0x10,
            Key::R => 0x13,
            Key::Esc => 0x01,
            Key::Digit1 => 0x02,
            Key::Digit2 => 0x03,
            Key::Digit3 => 0x04,
            Key::W => 0x11,
            Key::Ctrl => 0x1D,
            Key::S => 0x1F,
            Key::D => 0x20,
            Key::J => 0x24,
            Key::K => 0x25,
            Key::LeftShift => 0x2A,
            Key::Z => 0x2C,
            Key::C => 0x2E,
            Key::Slash => 0x35,
            Key::RightShift => 0x36,
            Key::KeypadStar => 0x37,
            Key::Alt => 0x38,
            Key::Keypad5 => 0x4C,
        }
    }
}

/// 玩家的開火旗標，INT 9 的 `sub_1A9B5` 依按鍵事件設成 `Y`，玩家邏輯處理後清成 `N`。
/// 旗標本身在 [`FireFlags`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FireFlag {
    /// 開火鍵放開過，下一次按下算新按（玩家 1 `273A7h`，玩家 2 `273A8h`）。
    NewPress,
    /// 本幀有開火鍵事件，含按住時的自動重複（`273A9h`、`273ABh`）。
    Event,
    /// 轉向鍵被按下（`273AAh`、`273ACh`）。
    Turn,
}

impl Core {
    /// 按鍵表第 `scan` 項（低 7 位元）。
    fn key_state(&self, scan: u8) -> u8 {
        self.flow.controls.keys.0.get(usize::from(scan & !BREAK_BIT)).copied().unwrap_or(0)
    }

    fn set_key_state(&mut self, scan: u8, v: u8) {
        if let Some(slot) = self.flow.controls.keys.0.get_mut(usize::from(scan & !BREAK_BIT)) {
            *slot = v;
        }
    }

    /// 按鍵是不是按著（按鍵表的值為 1）。放開後 INT 9 寫 2，所以放開的鍵不算。
    pub(crate) fn key_down(&self, key: Key) -> bool {
        self.key_state(key.scancode() as u8) == KEY_HELD
    }

    /// 按鍵剛放開（按鍵表的值為 2）。關卡的暫停與開火旗標用：暫停中 Esc 的狀態維持 2 直到下一次按下。
    pub(crate) fn key_released(&self, key: Key) -> bool {
        self.key_state(key.scancode() as u8) == KEY_RELEASED
    }

    /// 清掉按鍵表裡這個鍵的狀態（寫 0）。
    pub(crate) fn release_key(&mut self, key: Key) {
        self.set_key_state(key.scancode() as u8, 0);
    }

    /// 把這個鍵的狀態寫成 2（剛放開）。轉向鍵處理後用，暫停用。
    pub(crate) fn mark_key_released(&mut self, key: Key) {
        self.set_key_state(key.scancode() as u8, KEY_RELEASED);
    }

    /// 按住就消耗掉（寫 0）並回傳 true：一次按鍵只處理一次。
    pub(crate) fn take_key_press(&mut self, key: Key) -> bool {
        let pressed = self.key_down(key);
        if pressed {
            self.release_key(key);
        }
        pressed
    }

    /// INT 9 收到按下碼：按鍵表寫 1。
    pub(crate) fn note_key_down(&mut self, scan: u8) {
        self.set_key_state(scan, KEY_HELD);
    }

    /// INT 9 收到放開碼：按鍵表寫 2。
    pub(crate) fn note_key_up(&mut self, scan: u8) {
        self.set_key_state(scan, KEY_RELEASED);
    }

    /// 無敵秘技等測試要看按鍵表原始值。
    #[cfg(test)]
    pub(crate) fn key_raw(&self, scan: u8) -> u8 {
        self.key_state(scan)
    }

    /// 按鍵是不是按住（按鍵狀態表的值是 1；2 是剛放開）。表由 INT 9 維護。
    pub(crate) fn key_held(&self, scan: u8) -> bool {
        self.key_state(scan) == KEY_HELD
    }

    /// 展示模式還原輸入時直接寫按鍵狀態表。
    pub(crate) fn set_key(&mut self, scan: u8, held: bool) {
        self.set_key_state(scan, u8::from(held));
    }

    /// 掃描碼對應的 ASCII（原版 INT 9 的 `xlat`）。表只有 5Eh 項，掃描碼 5Eh 以上
    /// 讀到的是緊接在表後的按鍵狀態表的前面幾項（原版兩張表在資料段相鄰），所以寫成明確的規則：
    /// 超出 ASCII 表的掃描碼，取按鍵狀態表第 `scan - 5Eh` 項。
    pub(crate) fn ascii_of(&self, scan: u8) -> u8 {
        match self.tables.ascii.get(usize::from(scan)) {
            Some(&ascii) => ascii,
            None => self.key_state((usize::from(scan) - ASCII_ENTRIES) as u8),
        }
    }

    /// 最近一次按下的鍵（ASCII），0 表示沒有。
    pub(crate) fn last_key(&self) -> u8 {
        self.flow.controls.last_key
    }

    pub(crate) fn set_last_key(&mut self, ascii: u8) {
        self.flow.controls.last_key = ascii;
    }

    pub(crate) fn clear_last_key(&mut self) {
        self.flow.controls.last_key = 0;
    }

    pub(crate) fn key_pressed(&self) -> bool {
        self.last_key() != 0
    }

    /// 音效開關鍵是否生效（原版 `byte_2739A` 不是 `N`）。
    pub(crate) fn sound_key_enabled(&self) -> bool {
        self.flow.controls.sound_key != FLAG_OFF
    }

    /// 音效開關鍵是否生效（原版 `byte_2739A`）：標題期間 `N`，進關卡 `Y`。
    pub(crate) fn set_sound_key_enabled(&mut self, on: bool) {
        self.flow.controls.sound_key = if on { FLAG_ON } else { FLAG_OFF };
    }

    /// 玩家的開火旗標（`273A7h` 一族，INT 9 的 `sub_1A9B5` 設，玩家邏輯清）。
    pub(crate) fn fire_flag(&self, player: usize, flag: FireFlag) -> bool {
        let fire = &self.flow.controls.fire;
        let slot = match flag {
            FireFlag::NewPress => fire.new_press[player],
            FireFlag::Event => fire.event[player],
            FireFlag::Turn => fire.turn[player],
        };
        slot == FLAG_ON
    }

    pub(crate) fn set_fire_flag(&mut self, player: usize, flag: FireFlag, on: bool) {
        let fire = &mut self.flow.controls.fire;
        let slot = match flag {
            FireFlag::NewPress => &mut fire.new_press[player],
            FireFlag::Event => &mut fire.event[player],
            FireFlag::Turn => &mut fire.turn[player],
        };
        *slot = if on { FLAG_ON } else { FLAG_OFF };
    }

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
        Core::for_test(Start::Demo, Fixes::NONE)
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
    fn key_accessors_use_the_scancode_table() {
        let mut c = core();
        // 1Ch（Enter）的項：INT 9 寫 1 按下、2 放開。
        c.flow.controls.keys.0[0x1C] = 1;
        assert!(c.key_down(Key::Enter) && !c.key_down(Key::Space));
        c.flow.controls.keys.0[0x1C] = 2;
        assert!(!c.key_down(Key::Enter), "放開的鍵不算按著");
        c.flow.controls.keys.0[0x1C] = 1;
        c.release_key(Key::Enter);
        assert_eq!(c.flow.controls.keys.0[0x1C], 0);
        // 其餘的鍵對應：上 48h、下 50h、左 4Bh、右 4Dh、空白 39h、Q 10h、R 13h。
        for (key, sc) in [(Key::Up, 0x48usize), (Key::Down, 0x50), (Key::Left, 0x4B), (Key::Right, 0x4D), (Key::Space, 0x39), (Key::Q, 0x10), (Key::R, 0x13)] {
            c.flow.controls.keys.0[sc] = 1;
            assert!(c.key_down(key), "{key:?}");
        }
        c.flow.controls.last_key = 0x43;
        assert_eq!((c.last_key(), c.key_pressed()), (0x43, true));
        c.clear_last_key();
        assert!(!c.key_pressed());
    }

    #[test]
    fn key_state_helpers_use_the_same_table() {
        let mut c = core();
        c.flow.controls.keys.0[0x48] = 1;
        c.flow.controls.keys.0[0x50] = 2;
        assert!(c.key_held(0x48) && !c.key_held(0x50) && !c.key_held(0x4B));
        c.set_key(0x4B, true);
        assert_eq!(c.key_raw(0x4B), 1);
        c.set_key(0x4B, false);
        assert_eq!(c.key_raw(0x4B), 0);
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
