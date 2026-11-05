//! 輸入：SDL 掃描碼轉 IBM PC set 1 掃描碼、自動重複、前端熱鍵，以及鍵盤、手把、觸控三個來源的所有權。
//!
//! 規格 001「輸入模型」：`Input.scancodes` 是上一步到這一步之間送達的 set 1 碼，含放開碼（加 80h）與自動重複。
//! 方向鍵與 Home 等一律送不帶 `E0h` 前綴的碼，原版 INT 9 只看單一位元組。
//! 時間一律由呼叫端傳入（微秒），這裡不讀時鐘，好寫單元測試。
//!
//! 所有權（規格 005「工具列與設定畫面」）：
//!
//! - 三個來源各自計數，同一個遊戲碼要等最後一個來源放開才送放開碼。
//! - `block()`（設定畫面開著、前端暫停）放開全部來源，之後不送碼；`unblock()` 之後每個來源要等自己的按鍵都放開才重新送碼，
//!   所以設定畫面裡按的鍵、開關設定畫面用的鍵不會進遊戲。
//! - `release_all()`（視窗失焦）放開全部來源並忘記按著的鍵，不要求先放開。
//! - 重綁只改實體鍵到遊戲鍵的對應（`keymap::KeyMap`），送進遊戲的永遠是原版的 set 1 碼。

use crate::keymap::{KeyMap, is_game_code, is_pseudo, split_pseudo};
use sdl2::keyboard::Scancode as K;
use std::collections::{HashMap, HashSet};

/// SDL 掃描碼轉 set 1 按下碼（make code）。沒有對應的鍵回傳 `None`。
///
/// 兩個實體鍵共用一個碼的有：左右 Ctrl（1Dh）、左右 Alt（38h）、小鍵盤 Enter 與主鍵 Enter（1Ch）、
/// 小鍵盤 `/` 與主鍵 `/`（35h）、方向鍵與有 Num Lock 的小鍵盤數字（47h 至 53h）。
pub fn set1(key: K) -> Option<u8> {
    Some(match key {
        K::Escape => 0x01,
        K::Num1 => 0x02,
        K::Num2 => 0x03,
        K::Num3 => 0x04,
        K::Num4 => 0x05,
        K::Num5 => 0x06,
        K::Num6 => 0x07,
        K::Num7 => 0x08,
        K::Num8 => 0x09,
        K::Num9 => 0x0A,
        K::Num0 => 0x0B,
        K::Minus => 0x0C,
        K::Equals => 0x0D,
        K::Backspace => 0x0E,
        K::Tab => 0x0F,
        K::Q => 0x10,
        K::W => 0x11,
        K::E => 0x12,
        K::R => 0x13,
        K::T => 0x14,
        K::Y => 0x15,
        K::U => 0x16,
        K::I => 0x17,
        K::O => 0x18,
        K::P => 0x19,
        K::LeftBracket => 0x1A,
        K::RightBracket => 0x1B,
        K::Return | K::KpEnter => 0x1C,
        K::LCtrl | K::RCtrl => 0x1D,
        K::A => 0x1E,
        K::S => 0x1F,
        K::D => 0x20,
        K::F => 0x21,
        K::G => 0x22,
        K::H => 0x23,
        K::J => 0x24,
        K::K => 0x25,
        K::L => 0x26,
        K::Semicolon => 0x27,
        K::Apostrophe => 0x28,
        K::Grave => 0x29,
        K::LShift => 0x2A,
        K::Backslash | K::NonUsHash => 0x2B,
        K::Z => 0x2C,
        K::X => 0x2D,
        K::C => 0x2E,
        K::V => 0x2F,
        K::B => 0x30,
        K::N => 0x31,
        K::M => 0x32,
        K::Comma => 0x33,
        K::Period => 0x34,
        K::Slash | K::KpDivide => 0x35,
        K::RShift => 0x36,
        K::KpMultiply => 0x37,
        K::LAlt | K::RAlt => 0x38,
        K::Space => 0x39,
        K::CapsLock => 0x3A,
        K::F1 => 0x3B,
        K::F2 => 0x3C,
        K::F3 => 0x3D,
        K::F4 => 0x3E,
        K::F5 => 0x3F,
        K::F6 => 0x40,
        K::F7 => 0x41,
        K::F8 => 0x42,
        K::F9 => 0x43,
        K::F10 => 0x44,
        K::NumLockClear => 0x45,
        K::ScrollLock => 0x46,
        K::Home | K::Kp7 => 0x47,
        K::Up | K::Kp8 => 0x48,
        K::PageUp | K::Kp9 => 0x49,
        K::KpMinus => 0x4A,
        K::Left | K::Kp4 => 0x4B,
        K::Kp5 => 0x4C,
        K::Right | K::Kp6 => 0x4D,
        K::KpPlus => 0x4E,
        K::End | K::Kp1 => 0x4F,
        K::Down | K::Kp2 => 0x50,
        K::PageDown | K::Kp3 => 0x51,
        K::Insert | K::Kp0 => 0x52,
        K::Delete | K::KpPeriod => 0x53,
        K::NonUsBackslash => 0x56,
        K::F11 => 0x57,
        K::F12 => 0x58,
        _ => return None,
    })
}

/// 放開碼。
pub fn break_code(make: u8) -> u8 {
    make | 0x80
}

/// 前端熱鍵，不送進遊戲。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    /// F1：畫面補間。
    Tween,
    /// F2：幀計數。
    Counter,
    /// F3：原版圖與 HD。
    Gfx,
    /// F4：倍率循環。
    Scale,
    /// F5：玩家機預測與短按保留，關掉就是原版時序。
    Feel,
    /// F8：設定畫面換頁。
    Page,
    /// F10：開關設定畫面。
    Settings,
    /// F11：全螢幕。
    Fullscreen,
}

pub fn hotkey_of(key: K) -> Option<Hotkey> {
    Some(match key {
        K::F1 => Hotkey::Tween,
        K::F2 => Hotkey::Counter,
        K::F3 => Hotkey::Gfx,
        K::F4 => Hotkey::Scale,
        K::F5 => Hotkey::Feel,
        K::F8 => Hotkey::Page,
        K::F10 => Hotkey::Settings,
        K::F11 => Hotkey::Fullscreen,
        _ => return None,
    })
}

/// 自動重複的延遲：IBM AT 鍵盤控制器的預設值 500 毫秒（規格 001 列為 remake-owned）。
pub const REPEAT_DELAY_US: u64 = 500_000;
/// 自動重複的週期：IBM AT 鍵盤控制器預設每秒 10.9 次，1 秒 / 10.9 = 91743 微秒。
pub const REPEAT_PERIOD_US: u64 = 91_743;
/// 一次 `poll` 最多補的重複碼數。視窗被拖住很久之後不要一口氣灌進一大串碼。
pub const MAX_REPEATS_PER_POLL: usize = 4;

/// 自動重複。只重複最後按下且還按著的鍵；放開那個鍵就停止，不會改回去重複先前還按著的鍵。
#[derive(Clone, Debug, Default)]
pub struct Typematic {
    last: Option<u8>,
    next_due_us: u64,
}

impl Typematic {
    #[cfg(test)]
    pub fn new() -> Typematic {
        Typematic::default()
    }

    /// `code` 是剛按下的鍵的 make code，成為重複對象，第一次重複在 `REPEAT_DELAY_US` 之後。
    pub fn press(&mut self, code: u8, now_us: u64) {
        self.last = Some(code);
        self.next_due_us = now_us.saturating_add(REPEAT_DELAY_US);
    }

    /// 放開 `code`。只有它就是目前的重複對象才停止。
    pub fn release(&mut self, code: u8) {
        if self.last == Some(code) {
            self.last = None;
        }
    }

    pub fn clear(&mut self) {
        self.last = None;
    }

    /// 目前的重複對象。
    #[cfg(test)]
    pub fn current(&self) -> Option<u8> {
        self.last
    }

    /// 把到 `now_us` 為止該產生的重複碼（make code）接在 `out` 後面。
    pub fn poll(&mut self, now_us: u64, out: &mut Vec<u8>) {
        let Some(code) = self.last else { return };
        let mut n = 0;
        while self.next_due_us <= now_us {
            if n == MAX_REPEATS_PER_POLL {
                // 落後太多：丟掉積壓，從現在重新計時。
                self.next_due_us = now_us.saturating_add(REPEAT_PERIOD_US);
                break;
            }
            out.push(code);
            n += 1;
            self.next_due_us += REPEAT_PERIOD_US;
        }
    }
}

/// `InputMapper::key_down` 的結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyResult {
    /// 已排入遊戲的掃描碼佇列。
    Game,
    /// 前端熱鍵，呼叫端處理。
    Hotkey(Hotkey),
    /// 沒有對應、作業系統的重複事件，或已經按著。
    Ignored,
}

/// 按鍵的來源。同一個遊戲碼要等所有來源都放開才送放開碼。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Keyboard = 0,
    Pad = 1,
    Touch = 2,
}

/// 手把與觸控來源在 `phys_codes` 裡的索引。
fn pt_index(src: Source) -> usize {
    src as usize - 1
}

/// 鍵盤、手把、觸控到 set 1 掃描碼佇列。
///
/// 原版的鍵盤中斷隨時都會改按鍵狀態，遊戲一步才看一次，兩步之間按下又放開的鍵看不到。
/// `hold_short` 開著時，這種鍵的放開碼留到下一步才送，所以至少生效一步。
#[derive(Debug)]
pub struct InputMapper {
    keymap: KeyMap,
    /// 已送進遊戲的實體鍵 -> 送出的碼。
    held: HashMap<K, u8>,
    /// 按著的實體鍵，不論有沒有送進遊戲，判斷「都放開了」用。
    phys: HashSet<K>,
    /// 按下時被當成熱鍵吃掉的鍵，放開時也吃掉。
    swallowed: HashSet<K>,
    /// 手把（0）與觸控（1）按著的碼 -> 實體按住的數量，不論有沒有送進遊戲。
    phys_codes: [HashMap<u8, u32>; 2],
    /// 各來源已送進遊戲的數量，索引 `[碼][來源]`。
    owners: Box<[[u32; 3]; 256]>,
    /// 各來源現在送不送碼。被 `block()` 停掉後，要等該來源全部放開才重新開始。
    armed: [bool; 3],
    blocked: bool,
    queue: Vec<u8>,
    typematic: Typematic,
    hold_short: bool,
    /// 留到下一步才送的放開碼。
    carry: Vec<u8>,
    /// 遊戲目前看到的按鍵狀態，索引是 make code。
    game_down: [bool; 128],
}

impl Default for InputMapper {
    fn default() -> InputMapper {
        InputMapper {
            keymap: KeyMap::default(),
            held: HashMap::new(),
            phys: HashSet::new(),
            swallowed: HashSet::new(),
            phys_codes: [HashMap::new(), HashMap::new()],
            owners: Box::new([[0; 3]; 256]),
            armed: [true; 3],
            blocked: false,
            queue: Vec::new(),
            typematic: Typematic::default(),
            hold_short: true,
            carry: Vec::new(),
            game_down: [false; 128],
        }
    }
}

/// 依序套用掃描碼，更新按鍵狀態。
fn apply_codes(state: &mut [bool; 128], codes: &[u8]) {
    for &c in codes {
        state[(c & 0x7F) as usize] = c & 0x80 == 0;
    }
}

impl InputMapper {
    pub fn new() -> InputMapper {
        InputMapper::default()
    }

    /// 開關短按保留。關掉時碼照到達順序送，等於原版時序。已經留著的放開碼仍然在下一步送出。
    pub fn set_hold_short(&mut self, on: bool) {
        self.hold_short = on;
    }

    /// 下一步要送的碼，和留到再下一步的放開碼。
    fn split(&self) -> (Vec<u8>, Vec<u8>) {
        let mut out = self.carry.clone();
        if !self.hold_short {
            out.extend_from_slice(&self.queue);
            return (out, Vec::new());
        }
        let mut before = self.game_down;
        apply_codes(&mut before, &self.carry);
        let mut carry = Vec::new();
        for (i, &c) in self.queue.iter().enumerate() {
            let make = c & 0x7F;
            let is_break = c & 0x80 != 0;
            // 這個鍵在這段時間才按下，而且這是它最後一筆碼：放開碼留到下一步。
            let short = is_break && !before[make as usize] && !self.queue[i + 1..].iter().any(|&d| d & 0x7F == make);
            if short {
                carry.push(c);
            } else {
                out.push(c);
            }
        }
        (out, carry)
    }

    /// 下一步時每個 make code 是否按著，顯示層預測用。
    pub fn next_state(&self) -> [bool; 128] {
        let mut s = self.game_down;
        apply_codes(&mut s, &self.split().0);
        s
    }

    /// 換鍵盤對應。已經按著的鍵放開時用按下時的碼，不會因為改對應而卡鍵。
    pub fn set_keymap(&mut self, km: KeyMap) {
        self.keymap = km;
    }

    fn total(&self, code: u8) -> u32 {
        self.owners[code as usize].iter().sum()
    }

    /// 某個來源把 `code` 送進遊戲。第一個來源按下時才排入按下碼並重新計時自動重複。
    fn deliver(&mut self, src: Source, code: u8, now_us: u64) {
        let first = self.total(code) == 0;
        self.owners[code as usize][src as usize] += 1;
        if first && !is_pseudo(code) {
            self.queue.push(code);
            self.typematic.press(code, now_us);
        }
    }

    /// 某個來源放開 `code`。最後一個來源放開才排入放開碼。
    fn undeliver(&mut self, src: Source, code: u8) {
        let n = &mut self.owners[code as usize][src as usize];
        if *n == 0 {
            return;
        }
        *n -= 1;
        if self.total(code) == 0 && !is_pseudo(code) {
            self.queue.push(break_code(code));
            self.typematic.release(code);
        }
    }

    /// 全部來源已送進遊戲的碼一律放開（放開碼依碼的順序排入），不動「實體上還按著」的記錄。
    fn drop_delivered(&mut self) {
        for code in 0..=255u8 {
            for src in [Source::Keyboard, Source::Pad, Source::Touch] {
                while self.owners[code as usize][src as usize] > 0 {
                    self.undeliver(src, code);
                }
            }
        }
        self.held.clear();
        self.typematic.clear();
    }

    fn neutral(&self, src: Source) -> bool {
        match src {
            Source::Keyboard => self.phys.is_empty(),
            s => self.phys_codes[pt_index(s)].is_empty(),
        }
    }

    /// 沒被擋住、而且這個來源的按鍵都放開了，就恢復送碼。
    fn try_arm(&mut self, src: Source) {
        if !self.blocked && self.neutral(src) {
            self.armed[src as usize] = true;
        }
    }

    /// 現在不送進遊戲（設定畫面開著、前端暫停）。已送出的碼全部放開，之後的按鍵只記錄、不送。
    pub fn block(&mut self) {
        self.blocked = true;
        self.drop_delivered();
        self.armed = [false; 3];
    }

    /// 恢復。每個來源要等自己的按鍵都放開才重新送碼，所以在擋住期間按下、還按著的鍵不會進遊戲。
    pub fn unblock(&mut self) {
        self.blocked = false;
        for src in [Source::Keyboard, Source::Pad, Source::Touch] {
            self.try_arm(src);
        }
    }

    pub fn is_blocked(&self) -> bool {
        self.blocked
    }

    /// 這個來源現在有沒有在送碼（沒被擋住，而且已經等到按鍵都放開）。
    pub fn is_armed(&self, src: Source) -> bool {
        self.armed[src as usize]
    }

    /// 分離鍵現在有沒有按著（任何來源）。沒有原版掃描碼，之後給「砲座像 Force」用。
    pub fn split_held(&self, player: usize) -> bool {
        self.total(split_pseudo(player.min(1))) > 0
    }

    /// `os_repeat` 是 SDL 的 `repeat` 旗標。作業系統的重複一律忽略，重複碼由 `Typematic` 自己產生。
    ///
    /// 熱鍵不管擋不擋住都回報。其餘的鍵先查綁定，沒綁定的照自然的掃描碼進遊戲，
    /// 但自然掃描碼是玩家動作碼的鍵（功能已經重綁到別的鍵，或被清空）不送。
    pub fn key_down(&mut self, key: K, os_repeat: bool, now_us: u64) -> KeyResult {
        if os_repeat {
            return KeyResult::Ignored;
        }
        if let Some(h) = hotkey_of(key) {
            self.phys.insert(key);
            self.swallowed.insert(key);
            return KeyResult::Hotkey(h);
        }
        if !self.phys.insert(key) {
            return KeyResult::Ignored;
        }
        if !self.armed[Source::Keyboard as usize] {
            return KeyResult::Ignored;
        }
        let code = match self.keymap.find(key) {
            Some((p, act)) => act.counted_code(p),
            None => match set1(key) {
                Some(c) if !is_game_code(c) => c,
                _ => return KeyResult::Ignored,
            },
        };
        self.held.insert(key, code);
        self.deliver(Source::Keyboard, code, now_us);
        KeyResult::Game
    }

    pub fn key_up(&mut self, key: K) {
        self.phys.remove(&key);
        if !self.swallowed.remove(&key) {
            if let Some(code) = self.held.remove(&key) {
                self.undeliver(Source::Keyboard, code);
            }
        }
        self.try_arm(Source::Keyboard);
    }

    /// 手把或觸控按下一個遊戲碼（`PadEvent::Down`）。
    pub fn code_down(&mut self, src: Source, code: u8, now_us: u64) {
        if src == Source::Keyboard {
            return;
        }
        *self.phys_codes[pt_index(src)].entry(code).or_insert(0) += 1;
        if self.armed[src as usize] {
            self.deliver(src, code, now_us);
        }
    }

    /// 手把或觸控放開一個遊戲碼（`PadEvent::Up`）。
    pub fn code_up(&mut self, src: Source, code: u8) {
        if src == Source::Keyboard {
            return;
        }
        let idx = pt_index(src);
        let Some(n) = self.phys_codes[idx].get_mut(&code) else { return };
        *n -= 1;
        let left = *n;
        if left == 0 {
            self.phys_codes[idx].remove(&code);
        }
        // 已送進遊戲的數量超過還按著的數量，就是放開了一個送出去的。
        if self.owners[code as usize][src as usize] > left {
            self.undeliver(src, code);
        }
        self.try_arm(src);
    }

    /// 視窗失去焦點：全部來源放開並忘記按著的鍵，免得卡鍵。回到前景不要求先放開，因為失焦期間放開的事件收不到。
    pub fn release_all(&mut self) {
        self.drop_delivered();
        self.phys.clear();
        self.swallowed.clear();
        self.phys_codes = [HashMap::new(), HashMap::new()];
        self.armed = [!self.blocked; 3];
    }

    /// 產生到 `now_us` 為止的自動重複碼。擋住時不重複。
    pub fn poll(&mut self, now_us: u64) {
        if !self.blocked {
            self.typematic.poll(now_us, &mut self.queue);
        }
    }

    /// 取出這一步要送的掃描碼，順序就是送達順序，留到下一步的放開碼排在下一次的最前面。
    pub fn take(&mut self) -> Vec<u8> {
        let (out, carry) = self.split();
        apply_codes(&mut self.game_down, &out);
        self.queue.clear();
        self.carry = carry;
        out
    }

    /// 丟掉佇列裡的碼，不動按鍵狀態（回放或暫停時不讓鍵盤輸入累積）。
    pub fn discard_queue(&mut self) {
        self.queue.clear();
        self.carry.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_fixed_codes() {
        // 規格 001「輸入模型」列出的碼，方向鍵不帶 E0h 前綴。
        assert_eq!(set1(K::Up), Some(0x48));
        assert_eq!(set1(K::Down), Some(0x50));
        assert_eq!(set1(K::Left), Some(0x4B));
        assert_eq!(set1(K::Right), Some(0x4D));
        assert_eq!(set1(K::Home), Some(0x47));
        assert_eq!(set1(K::PageUp), Some(0x49));
        assert_eq!(set1(K::End), Some(0x4F));
        assert_eq!(set1(K::PageDown), Some(0x51));
        assert_eq!(set1(K::RShift), Some(0x36));
        assert_eq!(set1(K::Slash), Some(0x35));
        assert_eq!(set1(K::Escape), Some(0x01));
        assert_eq!(set1(K::Return), Some(0x1C));
        assert_eq!(set1(K::Space), Some(0x39));
        assert_eq!(set1(K::Tab), Some(0x0F));
        assert_eq!(set1(K::LCtrl), Some(0x1D));
        assert_eq!(set1(K::RCtrl), Some(0x1D));
        assert_eq!(set1(K::LShift), Some(0x2A));
        assert_eq!(set1(K::LAlt), Some(0x38));
        assert_eq!(set1(K::RAlt), Some(0x38));
        // 遊戲熱鍵用到的 Ctrl+Q、Ctrl+R、Ctrl+C。
        assert_eq!((set1(K::Q), set1(K::R), set1(K::C)), (Some(0x10), Some(0x13), Some(0x2E)));
    }

    #[test]
    fn letters_digits_symbols_follow_keyboard_rows() {
        let top = [K::Q, K::W, K::E, K::R, K::T, K::Y, K::U, K::I, K::O, K::P];
        for (i, k) in top.iter().enumerate() {
            assert_eq!(set1(*k), Some(0x10 + i as u8));
        }
        let home = [K::A, K::S, K::D, K::F, K::G, K::H, K::J, K::K, K::L];
        for (i, k) in home.iter().enumerate() {
            assert_eq!(set1(*k), Some(0x1E + i as u8));
        }
        let bottom = [K::Z, K::X, K::C, K::V, K::B, K::N, K::M];
        for (i, k) in bottom.iter().enumerate() {
            assert_eq!(set1(*k), Some(0x2C + i as u8));
        }
        let digits = [K::Num1, K::Num2, K::Num3, K::Num4, K::Num5, K::Num6, K::Num7, K::Num8, K::Num9, K::Num0];
        for (i, k) in digits.iter().enumerate() {
            assert_eq!(set1(*k), Some(0x02 + i as u8));
        }
        assert_eq!(set1(K::Minus), Some(0x0C));
        assert_eq!(set1(K::Equals), Some(0x0D));
        assert_eq!(set1(K::LeftBracket), Some(0x1A));
        assert_eq!(set1(K::RightBracket), Some(0x1B));
        assert_eq!(set1(K::Semicolon), Some(0x27));
        assert_eq!(set1(K::Apostrophe), Some(0x28));
        assert_eq!(set1(K::Grave), Some(0x29));
        assert_eq!(set1(K::Backslash), Some(0x2B));
        assert_eq!(set1(K::Comma), Some(0x33));
        assert_eq!(set1(K::Period), Some(0x34));
    }

    #[test]
    fn keypad_follows_set1() {
        let pad = [
            (K::Kp7, 0x47),
            (K::Kp8, 0x48),
            (K::Kp9, 0x49),
            (K::KpMinus, 0x4A),
            (K::Kp4, 0x4B),
            (K::Kp5, 0x4C),
            (K::Kp6, 0x4D),
            (K::KpPlus, 0x4E),
            (K::Kp1, 0x4F),
            (K::Kp2, 0x50),
            (K::Kp3, 0x51),
            (K::Kp0, 0x52),
            (K::KpPeriod, 0x53),
            (K::KpMultiply, 0x37),
            (K::KpEnter, 0x1C),
        ];
        for (k, c) in pad {
            assert_eq!(set1(k), Some(c), "{k:?}");
        }
    }

    #[test]
    fn no_make_code_has_the_break_bit_and_break_is_80h_more() {
        let all = [
            K::A, K::Escape, K::Up, K::Down, K::Left, K::Right, K::Home, K::End, K::PageUp, K::PageDown, K::F12, K::NonUsBackslash,
            K::Kp0, K::RShift, K::Delete, K::Insert, K::Space,
        ];
        for k in all {
            let c = set1(k).unwrap();
            assert!(c != 0 && c < 0x80, "{k:?} -> {c:#x}");
            assert_eq!(break_code(c), c + 0x80);
        }
        assert_eq!(break_code(0x48), 0xC8);
    }

    #[test]
    fn unmapped_keys_are_none() {
        for k in [K::LGui, K::RGui, K::PrintScreen, K::Pause, K::Application, K::VolumeUp, K::Menu] {
            assert_eq!(set1(k), None, "{k:?}");
        }
    }

    #[test]
    fn hotkeys_are_the_front_end_keys() {
        assert_eq!(hotkey_of(K::F1), Some(Hotkey::Tween));
        assert_eq!(hotkey_of(K::F2), Some(Hotkey::Counter));
        assert_eq!(hotkey_of(K::F3), Some(Hotkey::Gfx));
        assert_eq!(hotkey_of(K::F4), Some(Hotkey::Scale));
        assert_eq!(hotkey_of(K::F5), Some(Hotkey::Feel));
        assert_eq!(hotkey_of(K::F8), Some(Hotkey::Page));
        assert_eq!(hotkey_of(K::F10), Some(Hotkey::Settings), "F10 開設定畫面，隱藏工具列改在顯示頁");
        assert_eq!(hotkey_of(K::F11), Some(Hotkey::Fullscreen));
        assert_eq!(hotkey_of(K::F6), None);
        assert_eq!(hotkey_of(K::Escape), None, "鍵盤的 Esc 照原版送進遊戲");
    }

    fn drain(t: &mut Typematic, now: u64) -> Vec<u8> {
        let mut v = Vec::new();
        t.poll(now, &mut v);
        v
    }

    #[test]
    fn typematic_waits_for_the_delay() {
        let mut t = Typematic::new();
        t.press(0x4D, 1_000_000);
        assert!(drain(&mut t, 1_000_000).is_empty());
        assert!(drain(&mut t, 1_000_000 + REPEAT_DELAY_US - 1).is_empty());
        assert_eq!(drain(&mut t, 1_000_000 + REPEAT_DELAY_US), vec![0x4D]);
    }

    #[test]
    fn typematic_rate_is_10_9_per_second() {
        let mut t = Typematic::new();
        t.press(0x39, 0);
        let mut total = 0usize;
        // 延遲 500 ms 之後再過 10 秒，polling 間隔 7 ms（不會觸發每次上限）。
        let mut now = 0;
        while now <= REPEAT_DELAY_US + 10_000_000 {
            total += drain(&mut t, now).len();
            now += 7_000;
        }
        // 第一次在 500 ms，之後每 91743 us 一次：10 秒內 floor(10e6 / 91743) + 1 = 110 次。
        assert_eq!(total, 10_000_000 / REPEAT_PERIOD_US as usize + 1);
        assert!((108..=111).contains(&total), "{total}");
        assert_eq!(REPEAT_PERIOD_US, (1_000_000.0f64 / 10.9).floor() as u64);
    }

    #[test]
    fn typematic_repeats_only_the_last_pressed_key() {
        let mut t = Typematic::new();
        t.press(0x4B, 0); // 左
        t.press(0x48, 100_000); // 上，按得比較晚
        assert_eq!(t.current(), Some(0x48));
        let out = drain(&mut t, 100_000 + REPEAT_DELAY_US + 3 * REPEAT_PERIOD_US);
        assert_eq!(out, vec![0x48; 4]);
    }

    #[test]
    fn typematic_stops_on_release_and_does_not_fall_back() {
        let mut t = Typematic::new();
        t.press(0x4B, 0);
        t.press(0x48, 10);
        t.release(0x4B); // 放開先前的鍵：不影響目前的重複
        assert_eq!(t.current(), Some(0x48));
        t.release(0x48); // 放開最後的鍵：停止，且不回頭重複左
        assert_eq!(t.current(), None);
        assert!(drain(&mut t, 5_000_000).is_empty());
    }

    #[test]
    fn typematic_caps_catch_up_after_a_stall() {
        let mut t = Typematic::new();
        t.press(0x39, 0);
        let out = drain(&mut t, 60_000_000);
        assert_eq!(out.len(), MAX_REPEATS_PER_POLL);
        // 積壓被丟掉，下一個週期才有新的。
        assert!(drain(&mut t, 60_000_000 + REPEAT_PERIOD_US - 1).is_empty());
        assert_eq!(drain(&mut t, 60_000_000 + REPEAT_PERIOD_US), vec![0x39]);
    }

    #[test]
    fn mapper_orders_make_repeat_and_break() {
        let mut m = InputMapper::new();
        assert_eq!(m.key_down(K::Right, false, 0), KeyResult::Game);
        m.poll(REPEAT_DELAY_US);
        m.poll(REPEAT_DELAY_US + REPEAT_PERIOD_US);
        m.key_up(K::Right);
        assert_eq!(m.take(), vec![0x4D, 0x4D, 0x4D]);
        assert_eq!(m.take(), vec![0xCD], "這段時間才按下的鍵，放開碼留到下一步");
        assert!(m.take().is_empty());
    }

    #[test]
    fn short_press_lasts_until_the_next_step() {
        let mut m = InputMapper::new();
        m.key_down(K::Right, false, 0);
        m.key_up(K::Right);
        assert_eq!(m.next_state()[0x4D], true, "下一步會看到按著");
        assert_eq!(m.take(), vec![0x4D]);
        assert_eq!(m.next_state()[0x4D], false, "再下一步會看到放開");
        assert_eq!(m.take(), vec![0xCD]);
        assert!(m.take().is_empty());
    }

    #[test]
    fn key_held_over_a_step_releases_right_away() {
        let mut m = InputMapper::new();
        m.key_down(K::Right, false, 0);
        assert_eq!(m.take(), vec![0x4D]);
        m.key_up(K::Right);
        assert_eq!(m.next_state()[0x4D], false);
        assert_eq!(m.take(), vec![0xCD], "已經生效過一步的鍵不延後");
    }

    #[test]
    fn repeat_on_an_old_key_does_not_delay_the_release() {
        let mut m = InputMapper::new();
        m.key_down(K::Right, false, 0);
        assert_eq!(m.take(), vec![0x4D]);
        m.poll(REPEAT_DELAY_US);
        m.key_up(K::Right);
        assert_eq!(m.take(), vec![0x4D, 0xCD], "自動重複不算新按下");
    }

    #[test]
    fn double_tap_within_a_step_keeps_both_edges() {
        let mut m = InputMapper::new();
        m.key_down(K::Slash, false, 0);
        m.key_up(K::Slash);
        m.key_down(K::Slash, false, 1);
        m.key_up(K::Slash);
        // 原版的 INT 9 看邊緣：`/` 的轉向要兩次都收到。只有最後的放開碼延後。
        assert_eq!(m.take(), vec![0x35, 0xB5, 0x35]);
        assert_eq!(m.take(), vec![0xB5]);
    }

    #[test]
    fn press_again_right_after_a_carried_release() {
        let mut m = InputMapper::new();
        m.key_down(K::Right, false, 0);
        m.key_up(K::Right);
        assert_eq!(m.take(), vec![0x4D]);
        m.key_down(K::Right, false, 1);
        m.key_up(K::Right);
        // 留著的放開碼先送，新的短按再保留一步。
        assert_eq!(m.next_state()[0x4D], true);
        assert_eq!(m.take(), vec![0xCD, 0x4D]);
        assert_eq!(m.take(), vec![0xCD]);
    }

    #[test]
    fn original_timing_passes_codes_through() {
        let mut m = InputMapper::new();
        m.set_hold_short(false);
        m.key_down(K::Right, false, 0);
        m.key_up(K::Right);
        assert_eq!(m.next_state()[0x4D], false);
        assert_eq!(m.take(), vec![0x4D, 0xCD]);
        assert!(m.take().is_empty());
    }

    #[test]
    fn switching_to_original_timing_still_sends_the_carried_release() {
        let mut m = InputMapper::new();
        m.key_down(K::Right, false, 0);
        m.key_up(K::Right);
        assert_eq!(m.take(), vec![0x4D]);
        m.set_hold_short(false);
        assert_eq!(m.take(), vec![0xCD], "不能卡鍵");
    }

    #[test]
    fn discard_drops_the_carry_too() {
        let mut m = InputMapper::new();
        m.key_down(K::Right, false, 0);
        m.key_up(K::Right);
        m.take();
        m.discard_queue();
        assert!(m.take().is_empty());
    }

    #[test]
    fn mapper_ignores_os_repeat_events() {
        let mut m = InputMapper::new();
        assert_eq!(m.key_down(K::Space, false, 0), KeyResult::Game);
        for i in 1..20u64 {
            assert_eq!(m.key_down(K::Space, true, i * 30_000), KeyResult::Ignored);
        }
        // 作業系統的重複事件不產生碼，也不推進自己的計時。
        assert_eq!(m.take(), vec![0x39]);
        m.poll(REPEAT_DELAY_US - 1);
        assert!(m.take().is_empty());
        m.poll(REPEAT_DELAY_US);
        assert_eq!(m.take(), vec![0x39]);
    }

    #[test]
    fn mapper_swallows_hotkeys_and_their_release() {
        let mut m = InputMapper::new();
        assert_eq!(m.key_down(K::F1, false, 0), KeyResult::Hotkey(Hotkey::Tween));
        assert_eq!(m.key_down(K::F1, true, 10), KeyResult::Ignored);
        m.key_up(K::F1);
        assert!(m.take().is_empty());
        assert_eq!(m.key_down(K::F10, false, 0), KeyResult::Hotkey(Hotkey::Settings));
        // 放開 F1 之後再按會再觸發一次。
        assert_eq!(m.key_down(K::F1, false, 20), KeyResult::Hotkey(Hotkey::Tween));
    }

    #[test]
    fn mapper_unmapped_and_stray_release_do_nothing() {
        let mut m = InputMapper::new();
        assert_eq!(m.key_down(K::LGui, false, 0), KeyResult::Ignored);
        m.key_up(K::LGui);
        m.key_up(K::A); // 沒按過
        assert!(m.take().is_empty());
    }

    #[test]
    fn mapper_release_all_sends_breaks_and_stops_repeat() {
        let mut m = InputMapper::new();
        m.key_down(K::Left, false, 0);
        m.key_down(K::LCtrl, false, 1);
        m.release_all();
        assert_eq!(m.take(), vec![0x4B, 0x1D]);
        assert_eq!(m.take(), vec![0x9D, 0xCB]);
        m.poll(10_000_000);
        assert!(m.take().is_empty());
        m.key_up(K::Left); // 已經放開過
        assert!(m.take().is_empty());
    }

    #[test]
    fn mapper_two_keys_release_order() {
        let mut m = InputMapper::new();
        m.key_down(K::Left, false, 0);
        m.key_down(K::Up, false, 50_000);
        m.key_up(K::Left);
        m.poll(50_000 + REPEAT_DELAY_US);
        m.key_up(K::Up);
        m.poll(50_000 + REPEAT_DELAY_US + 10 * REPEAT_PERIOD_US);
        // 放開 Up 之後不會回頭重複 Left（Left 已放開，也不在重複）。兩個鍵都是這段時間才按下，放開碼留到下一步。
        assert_eq!(m.take(), vec![0x4B, 0x48, 0x48]);
        assert_eq!(m.take(), vec![0xCB, 0xC8]);
    }

    // -----------------------------------------------------------------------
    // 重綁：只改實體鍵到遊戲鍵的對應，送進遊戲的永遠是原版掃描碼。
    // -----------------------------------------------------------------------

    use crate::keymap::{Act, CODE_CONFIRM};

    fn drain_all(m: &mut InputMapper) -> Vec<u8> {
        // 把短按保留的碼也取乾淨。
        let mut v = m.take();
        v.extend(m.take());
        v
    }

    #[test]
    fn default_keymap_sends_the_original_codes() {
        let mut m = InputMapper::new();
        for (k, c) in [(K::RShift, 0x36u8), (K::Slash, 0x35), (K::T, 0x14), (K::LShift, 0x2A), (K::Z, 0x2C), (K::Home, 0x47), (K::N, 0x31)] {
            assert_eq!(m.key_down(k, false, 0), KeyResult::Game);
            assert_eq!(m.take(), vec![c], "{k:?}");
            m.key_up(k);
            m.take();
            m.take();
        }
        // 小鍵盤 8 預設同方向鍵上（原版的行為）。
        assert_eq!(m.key_down(K::Kp8, false, 0), KeyResult::Game);
        assert_eq!(m.take(), vec![0x48]);
    }

    #[test]
    fn rebinding_sends_the_same_original_code_from_the_new_key() {
        let mut km = KeyMap::default();
        km.clear(0, Act::Fire);
        km.bind(0, Act::Fire, K::Space).unwrap();
        let mut m = InputMapper::new();
        m.set_keymap(km);
        assert_eq!(m.key_down(K::Space, false, 0), KeyResult::Game);
        assert_eq!(m.take(), vec![0x36], "送進遊戲的還是右 Shift 的原版碼");
        m.key_up(K::Space);
        assert_eq!(drain_all(&mut m), vec![0xB6]);
        // 舊鍵不再送：它的功能已經在別的鍵上。
        assert_eq!(m.key_down(K::RShift, false, 1), KeyResult::Ignored);
        assert!(drain_all(&mut m).is_empty());
        m.key_up(K::RShift);
        assert!(drain_all(&mut m).is_empty(), "沒送過的鍵放開也不送");
    }

    #[test]
    fn cleared_binding_blocks_the_natural_key_and_unrelated_keys_still_pass() {
        let mut km = KeyMap::default();
        km.clear(1, Act::Up);
        let mut m = InputMapper::new();
        m.set_keymap(km);
        assert_eq!(m.key_down(K::T, false, 0), KeyResult::Ignored, "清空後 T 不再是第 2 人的上");
        assert!(m.take().is_empty());
        // Esc、Enter、空白鍵、C、S、Ctrl 照原版送。
        for (k, c) in [(K::Escape, 0x01u8), (K::Return, 0x1C), (K::Space, 0x39), (K::C, 0x2E), (K::S, 0x1F), (K::LCtrl, 0x1D)] {
            assert_eq!(m.key_down(k, false, 0), KeyResult::Game);
            assert_eq!(m.take(), vec![c], "{k:?}");
        }
    }

    #[test]
    fn rebinding_while_a_key_is_held_does_not_stick_it() {
        let mut m = InputMapper::new();
        m.key_down(K::RShift, false, 0);
        assert_eq!(m.take(), vec![0x36]);
        let mut km = KeyMap::default();
        km.clear(0, Act::Fire);
        m.set_keymap(km);
        m.key_up(K::RShift);
        assert_eq!(m.take(), vec![0xB6], "放開用按下時的碼");
    }

    #[test]
    fn split_key_is_counted_but_never_sent_to_the_game() {
        let mut m = InputMapper::new();
        assert!(!m.split_held(0));
        assert_eq!(m.key_down(K::Period, false, 0), KeyResult::Game);
        assert!(m.split_held(0) && !m.split_held(1));
        assert!(drain_all(&mut m).is_empty(), "分離沒有原版掃描碼");
        m.poll(10_000_000);
        assert!(drain_all(&mut m).is_empty(), "也不會自動重複");
        m.key_up(K::Period);
        assert!(!m.split_held(0));
        assert!(drain_all(&mut m).is_empty());
    }

    // -----------------------------------------------------------------------
    // 三個來源的所有權
    // -----------------------------------------------------------------------

    #[test]
    fn last_source_to_release_sends_the_break_code() {
        // 鍵盤先放開，手把還按著：不送放開碼。
        let mut m = InputMapper::new();
        m.key_down(K::Right, false, 0);
        m.code_down(Source::Pad, 0x4D, 1);
        assert_eq!(m.take(), vec![0x4D], "第二個來源按同一個碼不再送按下碼");
        m.key_up(K::Right);
        assert!(m.take().is_empty(), "手把還按著");
        m.code_up(Source::Pad, 0x4D);
        assert_eq!(m.take(), vec![0xCD], "最後一個來源放開才送放開碼");
        // 反過來：手把先放開，鍵盤還按著。
        let mut m = InputMapper::new();
        m.code_down(Source::Pad, 0x4D, 0);
        m.key_down(K::Right, false, 1);
        m.take();
        m.code_up(Source::Pad, 0x4D);
        assert!(m.take().is_empty());
        m.key_up(K::Right);
        assert_eq!(m.take(), vec![0xCD]);
    }

    #[test]
    fn three_sources_on_one_code() {
        let mut m = InputMapper::new();
        m.key_down(K::RShift, false, 0);
        m.code_down(Source::Pad, 0x36, 1);
        m.code_down(Source::Touch, 0x36, 2);
        assert_eq!(m.take(), vec![0x36]);
        m.code_up(Source::Touch, 0x36);
        m.key_up(K::RShift);
        assert!(m.take().is_empty());
        m.code_up(Source::Pad, 0x36);
        assert_eq!(m.take(), vec![0xB6]);
        // 兩支手把按同一個碼（兩個確定鈕）也是計數。
        let mut m = InputMapper::new();
        m.code_down(Source::Pad, CODE_CONFIRM, 0);
        m.code_down(Source::Pad, CODE_CONFIRM, 1);
        m.take();
        m.code_up(Source::Pad, CODE_CONFIRM);
        assert!(m.take().is_empty());
        m.code_up(Source::Pad, CODE_CONFIRM);
        assert_eq!(m.take(), vec![0x9C]);
        // 多餘的放開事件沒有作用。
        m.code_up(Source::Pad, CODE_CONFIRM);
        m.code_up(Source::Touch, 0x01);
        assert!(m.take().is_empty());
    }

    #[test]
    fn short_press_is_kept_for_pad_and_touch_too() {
        let mut m = InputMapper::new();
        m.code_down(Source::Touch, 0x36, 0);
        m.code_up(Source::Touch, 0x36);
        assert_eq!(m.take(), vec![0x36], "兩步之間的短按至少生效一步");
        assert_eq!(m.take(), vec![0xB6]);
    }

    #[test]
    fn blocking_releases_every_source_and_stops_sending() {
        let mut m = InputMapper::new();
        m.key_down(K::Left, false, 0);
        m.code_down(Source::Pad, 0x36, 1);
        m.code_down(Source::Touch, 0x4D, 2);
        assert_eq!(m.take(), vec![0x4B, 0x36, 0x4D]);
        m.block();
        assert!(m.is_blocked() && !m.is_armed(Source::Keyboard));
        // 放開碼依碼的順序，全部送出去，不留卡住的鍵。
        assert_eq!(m.take(), vec![0xB6, 0xCB, 0xCD]);
        // 擋住期間的按鍵不送，自動重複也停。
        assert_eq!(m.key_down(K::Up, false, 3), KeyResult::Ignored);
        m.code_down(Source::Pad, 0x36, 4);
        m.poll(100_000_000);
        assert!(drain_all(&mut m).is_empty());
        // 擋住期間熱鍵照樣回報（設定畫面裡要能用 F10、F11）。
        assert_eq!(m.key_down(K::F11, false, 5), KeyResult::Hotkey(Hotkey::Fullscreen));
    }

    #[test]
    fn after_unblocking_keys_still_held_are_not_sent_until_all_are_released() {
        let mut m = InputMapper::new();
        m.block();
        // 設定畫面裡按的鍵：Enter 開始重綁，Right 移動焦點。
        m.key_down(K::Return, false, 0);
        m.key_down(K::Right, false, 1);
        m.unblock();
        assert!(!m.is_armed(Source::Keyboard));
        // 還按著的鍵之後放開，不送。
        m.key_up(K::Return);
        assert_eq!(m.key_down(K::Up, false, 2), KeyResult::Ignored, "還有一顆沒放開，新按的鍵也不送");
        m.key_up(K::Right);
        assert!(!m.is_armed(Source::Keyboard), "Up 還按著");
        m.key_up(K::Up);
        assert!(m.is_armed(Source::Keyboard));
        assert!(drain_all(&mut m).is_empty(), "設定畫面的按鍵沒有任何一個進遊戲");
        // 全部放開之後恢復。
        assert_eq!(m.key_down(K::Right, false, 3), KeyResult::Game);
        assert_eq!(m.take(), vec![0x4D]);
    }

    #[test]
    fn closing_with_a_key_that_is_still_down_needs_that_key_released() {
        // 用 Esc 關設定畫面：按下時已經解除擋住，Esc 還按著。
        let mut m = InputMapper::new();
        m.block();
        m.key_down(K::Escape, false, 0);
        m.unblock();
        assert_eq!(m.key_down(K::Left, false, 1), KeyResult::Ignored);
        m.key_up(K::Left);
        m.key_up(K::Escape);
        assert!(m.is_armed(Source::Keyboard));
        assert!(drain_all(&mut m).is_empty(), "Esc 沒有被當成原版的暫停送進遊戲");
    }

    #[test]
    fn unblocking_with_nothing_held_resumes_at_once() {
        let mut m = InputMapper::new();
        m.block();
        m.unblock();
        assert!(m.is_armed(Source::Keyboard) && m.is_armed(Source::Pad) && m.is_armed(Source::Touch));
        assert_eq!(m.key_down(K::Right, false, 0), KeyResult::Game);
    }

    #[test]
    fn pad_and_touch_sources_wait_for_their_own_release_too() {
        let mut m = InputMapper::new();
        m.block();
        m.code_down(Source::Pad, 0x36, 0);
        m.code_down(Source::Touch, 0x4D, 0);
        m.unblock();
        assert!(!m.is_armed(Source::Pad) && !m.is_armed(Source::Touch));
        assert!(m.is_armed(Source::Keyboard), "鍵盤沒按著，可以立刻恢復");
        m.code_down(Source::Pad, 0x35, 1);
        assert!(drain_all(&mut m).is_empty());
        m.code_up(Source::Pad, 0x36);
        m.code_up(Source::Pad, 0x35);
        assert!(m.is_armed(Source::Pad));
        m.code_up(Source::Touch, 0x4D);
        assert!(m.is_armed(Source::Touch));
        assert!(drain_all(&mut m).is_empty());
        m.code_down(Source::Pad, 0x36, 2);
        assert_eq!(m.take(), vec![0x36]);
    }

    #[test]
    fn focus_loss_releases_every_source_and_forgets_held_keys() {
        let mut m = InputMapper::new();
        m.key_down(K::Left, false, 0);
        m.code_down(Source::Pad, 0x36, 1);
        m.take();
        m.release_all();
        assert_eq!(m.take(), vec![0xB6, 0xCB]);
        // 失焦期間放開的事件收不到，所以回來不要求先放開：之後按的鍵直接送。
        assert!(m.is_armed(Source::Keyboard));
        m.key_up(K::Left);
        assert_eq!(m.key_down(K::Left, false, 2), KeyResult::Game);
        assert_eq!(m.take(), vec![0x4B]);
        // 手把的舊事件（失焦前按下、之後才收到的放開）不會多送放開碼。
        m.code_up(Source::Pad, 0x36);
        assert!(drain_all(&mut m).is_empty());
    }

    #[test]
    fn focus_loss_while_blocked_stays_blocked() {
        let mut m = InputMapper::new();
        m.block();
        m.release_all();
        assert!(m.is_blocked());
        assert_eq!(m.key_down(K::Right, false, 0), KeyResult::Ignored);
        m.unblock();
        m.key_up(K::Right);
        assert!(m.is_armed(Source::Keyboard));
    }

    #[test]
    fn unplugging_a_pad_releases_only_that_source_codes() {
        // PadMapper::detach 送 Up 事件，這裡確認計數正確：鍵盤同碼還按著時不送放開碼。
        let mut m = InputMapper::new();
        m.key_down(K::Right, false, 0);
        m.code_down(Source::Pad, 0x4D, 1);
        m.take();
        m.code_up(Source::Pad, 0x4D);
        assert!(m.take().is_empty());
        m.key_up(K::Right);
        assert_eq!(m.take(), vec![0xCD]);
    }
}
