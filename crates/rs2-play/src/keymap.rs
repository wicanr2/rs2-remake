//! 遊戲鍵與可重綁的對應。不依賴 SDL 子系統。
//!
//! 重綁只改「實體鍵到遊戲鍵」的對應。送進遊戲的永遠是原版的 set 1 掃描碼：
//!
//! | 動作 | 第 1 人 | 第 2 人 |
//! |---|---|---|
//! | 上、下、左、右 | 48h、50h、4Bh、4Dh | 14h、30h、21h、23h |
//! | 左上、右上、左下、右下 | 47h、49h、4Fh、51h | 13h、15h、2Fh、31h |
//! | 開火、轉向 | 36h、35h | 2Ah、2Ch |
//! | 分離 | 沒有原版掃描碼，只記錄按著與否，之後給「砲座像 Force」用 | 同左 |
//!
//! 沒綁定的實體鍵照自然的掃描碼進遊戲（Esc、Enter、空白鍵、字母、數字、Ctrl 都照原版），
//! 但自然掃描碼落在上表的鍵不送：它的功能被重綁到別的鍵，或被清空了。

use crate::ui::text;
use sdl2::keyboard::Scancode as K;

/// 玩家動作。順序就是設定畫面的欄位順序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Act {
    Up,
    Down,
    Left,
    Right,
    UpLeft,
    UpRight,
    DownLeft,
    DownRight,
    Fire,
    Turn,
    Split,
}

pub const ACTS: [Act; 11] =
    [Act::Up, Act::Down, Act::Left, Act::Right, Act::UpLeft, Act::UpRight, Act::DownLeft, Act::DownRight, Act::Fire, Act::Turn, Act::Split];

/// 玩家動作的 set 1 碼：上、下、左、右、左上、右上、左下、右下、開火、轉向。
const CODES: [[u8; 10]; 2] = [[0x48, 0x50, 0x4B, 0x4D, 0x47, 0x49, 0x4F, 0x51, 0x36, 0x35], [0x14, 0x30, 0x21, 0x23, 0x13, 0x15, 0x2F, 0x31, 0x2A, 0x2C]];

/// 確定：Enter 的碼，選單確認與任意鍵。
pub const CODE_CONFIRM: u8 = 0x1C;
/// 選單：C 的碼，標題畫面開選單。
pub const CODE_MENU: u8 = 0x2E;

/// 分離沒有原版掃描碼。用 80h 以上的虛擬碼讓「同一個鍵最後一個來源放開才算放開」的計數照樣運作，這種碼不會送進遊戲。
pub const fn split_pseudo(player: usize) -> u8 {
    0xF0 + player as u8
}

pub fn is_pseudo(code: u8) -> bool {
    code >= 0x80
}

impl Act {
    pub fn index(self) -> usize {
        ACTS.iter().position(|a| *a == self).unwrap_or(0)
    }

    pub fn label(self) -> &'static str {
        match self {
            Act::Up => text::ACT_UP,
            Act::Down => text::ACT_DOWN,
            Act::Left => text::ACT_LEFT,
            Act::Right => text::ACT_RIGHT,
            Act::UpLeft => text::ACT_UPLEFT,
            Act::UpRight => text::ACT_UPRIGHT,
            Act::DownLeft => text::ACT_DOWNLEFT,
            Act::DownRight => text::ACT_DOWNRIGHT,
            Act::Fire => text::ACT_FIRE,
            Act::Turn => text::ACT_TURN,
            Act::Split => text::ACT_SPLIT,
        }
    }

    /// 設定檔用的名稱。
    pub fn save_name(self) -> &'static str {
        match self {
            Act::Up => "up",
            Act::Down => "down",
            Act::Left => "left",
            Act::Right => "right",
            Act::UpLeft => "upleft",
            Act::UpRight => "upright",
            Act::DownLeft => "downleft",
            Act::DownRight => "downright",
            Act::Fire => "fire",
            Act::Turn => "turn",
            Act::Split => "split",
        }
    }

    pub fn from_save_name(s: &str) -> Option<Act> {
        ACTS.iter().copied().find(|a| a.save_name() == s)
    }

    /// 送進遊戲的 set 1 碼。分離沒有，回傳 `None`。
    pub fn code(self, player: usize) -> Option<u8> {
        if self == Act::Split { None } else { Some(CODES[player.min(1)][self.index()]) }
    }

    /// 送進計數層的碼：玩家動作的 set 1 碼，分離是虛擬碼。
    pub fn counted_code(self, player: usize) -> u8 {
        self.code(player).unwrap_or(split_pseudo(player.min(1)))
    }
}

/// 兩位玩家十種動作的 set 1 碼。這些碼只能由對應的動作送出。
pub fn is_game_code(code: u8) -> bool {
    CODES.iter().any(|row| row.contains(&code))
}

/// 預設鍵：原版鍵位（`docs/re/03-game-logic.md`「輸入」），分離是新增的鍵。
fn default_key(player: usize, act: Act) -> K {
    const P1: [K; 11] = [K::Up, K::Down, K::Left, K::Right, K::Home, K::PageUp, K::End, K::PageDown, K::RShift, K::Slash, K::Period];
    const P2: [K; 11] = [K::T, K::B, K::F, K::H, K::R, K::Y, K::V, K::N, K::LShift, K::Z, K::X];
    if player == 0 { P1[act.index()] } else { P2[act.index()] }
}

/// 同一個遊戲碼的兩個實體鍵：方向鍵與有 Num Lock 的小鍵盤數字、`/` 與小鍵盤 `/`、Enter 與小鍵盤 Enter。
/// 綁定在左邊那顆，右邊那顆跟著生效，和原版的行為一樣。綁定右邊那顆就只有它自己。
fn twin(k: K) -> Option<K> {
    Some(match k {
        K::Up => K::Kp8,
        K::Down => K::Kp2,
        K::Left => K::Kp4,
        K::Right => K::Kp6,
        K::Home => K::Kp7,
        K::PageUp => K::Kp9,
        K::End => K::Kp1,
        K::PageDown => K::Kp3,
        K::Slash => K::KpDivide,
        K::Return => K::KpEnter,
        K::Insert => K::Kp0,
        _ => return None,
    })
}

/// 綁定在 `k` 時會觸發的實體鍵。
pub fn aliases(k: K) -> [Option<K>; 2] {
    [Some(k), twin(k)]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindError {
    /// 不能綁定的鍵（F1 到 F12、Esc、Tab、Ctrl、Alt 等）。
    NotBindable,
    /// 已經綁在別的欄位。
    Conflict(usize, Act),
}

/// 兩位玩家各十一個欄位的實體鍵。`None` 是未綁定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyMap {
    keys: [[Option<K>; 11]; 2],
}

impl Default for KeyMap {
    fn default() -> KeyMap {
        let mut keys = [[None; 11]; 2];
        for (p, row) in keys.iter_mut().enumerate() {
            for a in ACTS {
                row[a.index()] = Some(default_key(p, a));
            }
        }
        KeyMap { keys }
    }
}

impl KeyMap {
    pub fn get(&self, player: usize, act: Act) -> Option<K> {
        self.keys[player.min(1)][act.index()]
    }

    /// 實體鍵 `key` 觸發哪個動作。
    pub fn find(&self, key: K) -> Option<(usize, Act)> {
        for p in 0..2 {
            for a in ACTS {
                if let Some(b) = self.keys[p][a.index()] {
                    if aliases(b).contains(&Some(key)) {
                        return Some((p, a));
                    }
                }
            }
        }
        None
    }

    /// 把 `key` 綁到 `(player, act)` 會和哪個欄位衝突。
    pub fn conflict(&self, player: usize, act: Act, key: K) -> Option<(usize, Act)> {
        let mine = aliases(key);
        for p in 0..2 {
            for a in ACTS {
                if (p, a) == (player, act) {
                    continue;
                }
                if let Some(b) = self.keys[p][a.index()] {
                    if aliases(b).iter().flatten().any(|x| mine.contains(&Some(*x))) {
                        return Some((p, a));
                    }
                }
            }
        }
        None
    }

    /// 綁定。衝突或不能綁的鍵不覆寫。
    pub fn bind(&mut self, player: usize, act: Act, key: K) -> Result<(), BindError> {
        if !text::key_bindable(key) {
            return Err(BindError::NotBindable);
        }
        if let Some((p, a)) = self.conflict(player, act, key) {
            return Err(BindError::Conflict(p, a));
        }
        self.keys[player.min(1)][act.index()] = Some(key);
        Ok(())
    }

    pub fn clear(&mut self, player: usize, act: Act) {
        self.keys[player.min(1)][act.index()] = None;
    }

    pub fn reset(&mut self) {
        *self = KeyMap::default();
    }

    /// 設定檔的行：`key.p1.up=Up`，空值是未綁定。
    pub fn to_lines(&self) -> String {
        let mut s = String::new();
        for p in 0..2 {
            for a in ACTS {
                let name = self.keys[p][a.index()].and_then(text::key_name).unwrap_or("");
                s.push_str(&format!("key.p{}.{}={}\n", p + 1, a.save_name(), name));
            }
        }
        s
    }

    /// 解析設定檔的鍵名 `p1.up`，回傳欄位。
    pub fn parse_slot(name: &str) -> Option<(usize, Act)> {
        let (p, a) = name.split_once('.')?;
        let player = match p {
            "p1" => 0,
            "p2" => 1,
            _ => return None,
        };
        Some((player, Act::from_save_name(a)?))
    }

    /// 由設定檔讀到的欄位組成對應。檔案裡有的欄位照檔案，衝突或不能綁的略過並警告；
    /// 檔案沒提到的欄位用預設鍵，預設鍵已被用掉就留空並警告。
    pub fn from_entries(entries: &[(usize, Act, Option<K>)]) -> (KeyMap, Vec<String>) {
        let mut m = KeyMap { keys: [[None; 11]; 2] };
        let mut seen = [[false; 11]; 2];
        let mut warn = Vec::new();
        for &(p, a, k) in entries {
            seen[p][a.index()] = true;
            if let Some(k) = k {
                match m.bind(p, a, k) {
                    Ok(()) => {}
                    // 不能綁的鍵（別的版本留下的檔）：當作沒寫，欄位用預設鍵。
                    Err(BindError::NotBindable) => {
                        seen[p][a.index()] = false;
                        warn.push(format!("第 {} 人 {} 的鍵 {} 不能綁定，略過", p + 1, a.save_name(), k.name()));
                    }
                    // 和別的欄位重複：略過，這個欄位維持未綁定，玩家可以在設定畫面重綁。
                    Err(BindError::Conflict(p2, a2)) => {
                        warn.push(format!("第 {} 人 {} 的鍵 {} 和第 {} 人 {} 重複，略過", p + 1, a.save_name(), k.name(), p2 + 1, a2.save_name()))
                    }
                }
            }
        }
        for p in 0..2 {
            for a in ACTS {
                if seen[p][a.index()] {
                    continue;
                }
                let d = default_key(p, a);
                if m.conflict(p, a, d).is_none() {
                    m.keys[p][a.index()] = Some(d);
                } else {
                    warn.push(format!("第 {} 人 {} 的預設鍵 {} 已被別的欄位用掉，維持未綁定", p + 1, a.save_name(), d.name()));
                }
            }
        }
        (m, warn)
    }
}

// ---------------------------------------------------------------------------
// 手把
// ---------------------------------------------------------------------------

/// 可指派的手把按鈕。方向固定在十字鍵與左搖桿，Guide 由作業系統攔截，所以都不在這裡。
/// LT、RT 是類比扳機，過了門檻算按下。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Btn {
    A,
    B,
    X,
    Y,
    Back,
    Start,
    L3,
    R3,
    LB,
    RB,
    LT,
    RT,
}

pub const BTNS: [Btn; 12] = [Btn::A, Btn::B, Btn::X, Btn::Y, Btn::Back, Btn::Start, Btn::L3, Btn::R3, Btn::LB, Btn::RB, Btn::LT, Btn::RT];

impl Btn {
    pub fn name(self) -> &'static str {
        match self {
            Btn::A => "A",
            Btn::B => "B",
            Btn::X => "X",
            Btn::Y => "Y",
            Btn::Back => "Back",
            Btn::Start => "Start",
            Btn::L3 => "L3",
            Btn::R3 => "R3",
            Btn::LB => "LB",
            Btn::RB => "RB",
            Btn::LT => "LT",
            Btn::RT => "RT",
        }
    }

    pub fn from_name(s: &str) -> Option<Btn> {
        BTNS.iter().copied().find(|b| b.name() == s)
    }

    pub fn from_sdl(b: sdl2::controller::Button) -> Option<Btn> {
        use sdl2::controller::Button as S;
        Some(match b {
            S::A => Btn::A,
            S::B => Btn::B,
            S::X => Btn::X,
            S::Y => Btn::Y,
            S::Back => Btn::Back,
            S::Start => Btn::Start,
            S::LeftStick => Btn::L3,
            S::RightStick => Btn::R3,
            S::LeftShoulder => Btn::LB,
            S::RightShoulder => Btn::RB,
            _ => return None,
        })
    }
}

/// 手把按鈕可以改的功能。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PadFn {
    Fire,
    Turn,
    Split,
    /// 前端暫停，不送進遊戲。
    Pause,
    /// 選單確認，送 Enter。
    Confirm,
    /// 標題畫面的選單，送 C。
    Menu,
}

pub const PADFNS: [PadFn; 6] = [PadFn::Fire, PadFn::Turn, PadFn::Split, PadFn::Pause, PadFn::Confirm, PadFn::Menu];

impl PadFn {
    pub fn index(self) -> usize {
        PADFNS.iter().position(|f| *f == self).unwrap_or(0)
    }

    pub fn label(self) -> &'static str {
        match self {
            PadFn::Fire => text::PADFN_FIRE,
            PadFn::Turn => text::PADFN_TURN,
            PadFn::Split => text::PADFN_SPLIT,
            PadFn::Pause => text::PADFN_PAUSE,
            PadFn::Confirm => text::PADFN_CONFIRM,
            PadFn::Menu => text::PADFN_MENU,
        }
    }

    pub fn save_name(self) -> &'static str {
        match self {
            PadFn::Fire => "fire",
            PadFn::Turn => "turn",
            PadFn::Split => "split",
            PadFn::Pause => "pause",
            PadFn::Confirm => "confirm",
            PadFn::Menu => "menu",
        }
    }

    pub fn from_save_name(s: &str) -> Option<PadFn> {
        PADFNS.iter().copied().find(|f| f.save_name() == s)
    }
}

/// 每個功能兩個按鈕欄位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadMap {
    slots: [[Option<Btn>; 2]; 6],
}

impl Default for PadMap {
    fn default() -> PadMap {
        // 開火 A、RT，轉向 B、LT（原版接法）；分離 RB、LB；暫停 Start；確定 X、Back；選單 Y。
        PadMap {
            slots: [
                [Some(Btn::A), Some(Btn::RT)],
                [Some(Btn::B), Some(Btn::LT)],
                [Some(Btn::RB), Some(Btn::LB)],
                [Some(Btn::Start), None],
                [Some(Btn::X), Some(Btn::Back)],
                [Some(Btn::Y), None],
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadBindError {
    /// 按鈕已經用在這個功能。
    Taken(PadFn),
}

impl PadMap {
    pub fn get(&self, f: PadFn, slot: usize) -> Option<Btn> {
        self.slots[f.index()][slot.min(1)]
    }

    /// 按鈕 `b` 對應的功能。
    pub fn find(&self, b: Btn) -> Option<(PadFn, usize)> {
        for f in PADFNS {
            for s in 0..2 {
                if self.slots[f.index()][s] == Some(b) {
                    return Some((f, s));
                }
            }
        }
        None
    }

    pub fn bind(&mut self, f: PadFn, slot: usize, b: Btn) -> Result<(), PadBindError> {
        if let Some((of, os)) = self.find(b) {
            if (of, os) != (f, slot.min(1)) {
                return Err(PadBindError::Taken(of));
            }
        }
        self.slots[f.index()][slot.min(1)] = Some(b);
        Ok(())
    }

    pub fn clear(&mut self, f: PadFn, slot: usize) {
        self.slots[f.index()][slot.min(1)] = None;
    }

    pub fn reset(&mut self) {
        *self = PadMap::default();
    }

    /// 設定檔的行：`pad.fire=A,RT`，`-` 是空欄位。
    pub fn to_lines(&self) -> String {
        let mut s = String::new();
        for f in PADFNS {
            let n = |b: Option<Btn>| b.map(|b| b.name()).unwrap_or("-");
            s.push_str(&format!("pad.{}={},{}\n", f.save_name(), n(self.slots[f.index()][0]), n(self.slots[f.index()][1])));
        }
        s
    }

    /// 解析 `A,RT` 或 `A,-`。
    pub fn parse_value(v: &str) -> Result<[Option<Btn>; 2], String> {
        let parts: Vec<&str> = v.split(',').map(|p| p.trim()).collect();
        if parts.len() != 2 {
            return Err(format!("{v:?} 要寫兩個欄位，例如 A,RT"));
        }
        let mut out = [None; 2];
        for (i, p) in parts.iter().enumerate() {
            out[i] = match *p {
                "-" | "" => None,
                n => Some(Btn::from_name(n).ok_or_else(|| format!("不認得的按鈕 {n:?}"))?),
            };
        }
        Ok(out)
    }

    /// 由設定檔的功能欄位組成對應。檔案裡有的功能照檔案，重複使用的按鈕略過並警告；沒提到的功能用預設值，按鈕已被用掉就留空。
    pub fn from_entries(entries: &[(PadFn, [Option<Btn>; 2])]) -> (PadMap, Vec<String>) {
        let mut m = PadMap { slots: [[None; 2]; 6] };
        let mut seen = [false; 6];
        let mut warn = Vec::new();
        for &(f, bs) in entries {
            seen[f.index()] = true;
            for (s, b) in bs.iter().enumerate() {
                if let Some(b) = b {
                    if let Err(PadBindError::Taken(of)) = m.bind(f, s, *b) {
                        warn.push(format!("手把按鈕 {} 在 {} 與 {} 重複，略過", b.name(), f.save_name(), of.save_name()));
                    }
                }
            }
        }
        let d = PadMap::default();
        for f in PADFNS {
            if seen[f.index()] {
                continue;
            }
            for s in 0..2 {
                if let Some(b) = d.get(f, s) {
                    if m.find(b).is_none() {
                        m.slots[f.index()][s] = Some(b);
                    } else {
                        warn.push(format!("手把功能 {} 的預設按鈕 {} 已被用掉，維持空白", f.save_name(), b.name()));
                    }
                }
            }
        }
        (m, warn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_equal_the_original_key_layout() {
        let m = KeyMap::default();
        // 第 1 人：方向鍵與 Home、PgUp、End、PgDn，右 Shift 開火，/ 轉向。
        let p1 = [K::Up, K::Down, K::Left, K::Right, K::Home, K::PageUp, K::End, K::PageDown, K::RShift, K::Slash];
        // 第 2 人：T、B、F、H 與 R、Y、V、N，左 Shift 開火，Z 轉向。
        let p2 = [K::T, K::B, K::F, K::H, K::R, K::Y, K::V, K::N, K::LShift, K::Z];
        for (i, a) in ACTS[..10].iter().enumerate() {
            assert_eq!(m.get(0, *a), Some(p1[i]), "{a:?}");
            assert_eq!(m.get(1, *a), Some(p2[i]), "{a:?}");
        }
        assert!(m.get(0, Act::Split).is_some() && m.get(1, Act::Split).is_some());
    }

    #[test]
    fn default_keys_produce_the_original_set1_codes() {
        let m = KeyMap::default();
        for p in 0..2 {
            for a in &ACTS[..10] {
                let k = m.get(p, *a).unwrap();
                assert_eq!(crate::input::set1(k), a.code(p), "第 {} 人 {a:?}", p + 1);
            }
        }
        // 規格 001 的碼。
        assert_eq!(Act::Fire.code(0), Some(0x36));
        assert_eq!(Act::Turn.code(0), Some(0x35));
        assert_eq!(Act::Fire.code(1), Some(0x2A));
        assert_eq!(Act::Turn.code(1), Some(0x2C));
        assert_eq!(Act::Split.code(0), None);
        assert!(is_game_code(0x48) && is_game_code(0x2C) && !is_game_code(0x01) && !is_game_code(CODE_CONFIRM) && !is_game_code(CODE_MENU));
    }

    #[test]
    fn predictor_reads_the_same_codes() {
        // predict.rs 讀的方向碼與這裡的表一致。
        for p in 0..2 {
            let mut state = [false; 128];
            state[Act::Up.code(p).unwrap() as usize] = true;
            assert!(crate::predict::dir_of(&state, p).up);
            let mut state = [false; 128];
            state[Act::DownRight.code(p).unwrap() as usize] = true;
            let d = crate::predict::dir_of(&state, p);
            assert!(d.down && d.right && !d.up && !d.left);
            let mut state = [false; 128];
            state[Act::UpLeft.code(p).unwrap() as usize] = true;
            let d = crate::predict::dir_of(&state, p);
            assert!(d.up && d.left);
        }
    }

    #[test]
    fn default_bindings_have_no_conflicts() {
        let m = KeyMap::default();
        for p in 0..2 {
            for a in ACTS {
                let k = m.get(p, a).unwrap();
                assert_eq!(m.conflict(p, a, k), None, "{p} {a:?}");
                assert_eq!(m.find(k), Some((p, a)));
            }
        }
    }

    #[test]
    fn keypad_twin_follows_the_cursor_key_only_when_that_one_is_bound() {
        let mut m = KeyMap::default();
        assert_eq!(m.find(K::Kp8), Some((0, Act::Up)), "預設：小鍵盤 8 同方向鍵上");
        assert_eq!(m.find(K::KpDivide), Some((0, Act::Turn)));
        // 綁到小鍵盤那顆：只有它自己。
        m.clear(0, Act::Up);
        m.bind(0, Act::Up, K::Kp8).unwrap();
        assert_eq!(m.find(K::Kp8), Some((0, Act::Up)));
        assert_eq!(m.find(K::Up), None);
        // 小鍵盤 8 已被占用：方向鍵上（它的孿生鍵是小鍵盤 8）也綁不進來。
        assert_eq!(m.bind(1, Act::Up, K::Up), Err(BindError::Conflict(0, Act::Up)));
        assert_eq!(m.conflict(1, Act::Down, K::Kp8), Some((0, Act::Up)));
        assert_eq!(m.bind(1, Act::Down, K::Kp8), Err(BindError::Conflict(0, Act::Up)));
    }

    #[test]
    fn rebind_replaces_only_the_mapping_and_keeps_game_codes() {
        let mut m = KeyMap::default();
        m.clear(0, Act::Fire);
        m.bind(0, Act::Fire, K::Space).unwrap();
        assert_eq!(m.find(K::Space), Some((0, Act::Fire)));
        assert_eq!(m.find(K::RShift), None, "舊鍵不再觸發");
        // 送出的碼永遠是原版的碼。
        assert_eq!(Act::Fire.code(0), Some(0x36));
    }

    #[test]
    fn bind_rejects_conflicts_without_overwriting() {
        let mut m = KeyMap::default();
        let before = m;
        // 第 2 人的上想用第 1 人的右 Shift。
        assert_eq!(m.bind(1, Act::Up, K::RShift), Err(BindError::Conflict(0, Act::Fire)));
        assert_eq!(m, before, "衝突時不覆寫");
        // 同一個欄位綁自己的鍵不算衝突。
        assert_eq!(m.bind(0, Act::Fire, K::RShift), Ok(()));
        // 不能綁的鍵。
        for k in [K::F1, K::F6, K::F10, K::Escape, K::Tab, K::LCtrl, K::RCtrl, K::LAlt, K::Delete, K::CapsLock] {
            assert_eq!(m.bind(1, Act::Up, k), Err(BindError::NotBindable), "{k:?}");
        }
        assert_eq!(m, before);
    }

    #[test]
    fn clear_and_reset() {
        let mut m = KeyMap::default();
        m.clear(1, Act::Turn);
        assert_eq!(m.get(1, Act::Turn), None);
        assert_eq!(m.find(K::Z), None, "清空後那顆鍵不觸發");
        // 清空後那顆鍵可以給別人。
        assert_eq!(m.bind(0, Act::Split, K::Z), Ok(()));
        m.reset();
        assert_eq!(m, KeyMap::default());
    }

    #[test]
    fn text_roundtrip_and_partial_files() {
        let mut m = KeyMap::default();
        m.clear(0, Act::Up);
        m.bind(0, Act::Fire, K::Space).unwrap();
        m.clear(1, Act::Split);
        let mut entries = Vec::new();
        for line in m.to_lines().lines() {
            let (k, v) = line.split_once('=').unwrap();
            let slot = KeyMap::parse_slot(k.strip_prefix("key.").unwrap()).unwrap();
            entries.push((slot.0, slot.1, text::key_from_name(v)));
        }
        let (back, warn) = KeyMap::from_entries(&entries);
        assert_eq!(back, m);
        assert!(warn.is_empty(), "{warn:?}");
        // 空檔：全部預設。
        let (d, w) = KeyMap::from_entries(&[]);
        assert_eq!(d, KeyMap::default());
        assert!(w.is_empty());
    }

    #[test]
    fn file_can_swap_two_players_keys() {
        // 兩人互換方向鍵：逐欄檢查衝突時，先寫的不能被預設值擋住。
        let entries = [(0, Act::Up, Some(K::T)), (1, Act::Up, Some(K::Up))];
        let (m, warn) = KeyMap::from_entries(&entries);
        assert_eq!(m.get(0, Act::Up), Some(K::T));
        assert_eq!(m.get(1, Act::Up), Some(K::Up));
        // 其餘欄位用預設，其中沒有衝突的照用。
        assert_eq!(m.get(0, Act::Down), Some(K::Down));
        assert!(warn.is_empty(), "{warn:?}");
    }

    #[test]
    fn bad_file_entries_are_skipped_with_warnings() {
        let entries = [(0, Act::Up, Some(K::F1)), (0, Act::Down, Some(K::W)), (1, Act::Down, Some(K::W)), (1, Act::Left, None)];
        let (m, warn) = KeyMap::from_entries(&entries);
        assert_eq!(m.get(0, Act::Down), Some(K::W));
        assert_eq!(m.get(1, Act::Down), None, "重複的鍵略過，維持未綁定");
        assert_eq!(m.get(0, Act::Up), Some(K::Up), "不能綁的鍵略過，改回預設");
        assert_eq!(m.get(1, Act::Left), None, "明寫空值就是未綁定");
        assert_eq!(warn.len(), 2, "{warn:?}");
        assert!(KeyMap::parse_slot("p3.up").is_none() && KeyMap::parse_slot("p1.sideways").is_none() && KeyMap::parse_slot("up").is_none());
    }

    #[test]
    fn split_has_no_game_code_but_a_counted_pseudo_code() {
        for p in 0..2 {
            assert!(is_pseudo(Act::Split.counted_code(p)));
            assert!(!is_game_code(Act::Split.counted_code(p)));
            assert_eq!(Act::Fire.counted_code(p), Act::Fire.code(p).unwrap());
        }
        assert_ne!(split_pseudo(0), split_pseudo(1));
    }

    #[test]
    fn pad_defaults_and_binding_rules() {
        let mut m = PadMap::default();
        assert_eq!(m.get(PadFn::Fire, 0), Some(Btn::A));
        assert_eq!(m.get(PadFn::Pause, 0), Some(Btn::Start));
        assert_eq!(m.find(Btn::RT), Some((PadFn::Fire, 1)));
        // 同一個按鈕只能用一次。
        let before = m;
        assert_eq!(m.bind(PadFn::Turn, 0, Btn::A), Err(PadBindError::Taken(PadFn::Fire)));
        assert_eq!(m, before);
        assert_eq!(m.bind(PadFn::Fire, 0, Btn::A), Ok(()));
        m.clear(PadFn::Fire, 0);
        assert_eq!(m.bind(PadFn::Turn, 0, Btn::A), Ok(()));
        assert_eq!(m.find(Btn::A), Some((PadFn::Turn, 0)));
        m.reset();
        assert_eq!(m, PadMap::default());
        // 所有預設按鈕互不重複。
        let mut seen = std::collections::HashSet::new();
        for f in PADFNS {
            for s in 0..2 {
                if let Some(b) = PadMap::default().get(f, s) {
                    assert!(seen.insert(b), "預設按鈕重複 {b:?}");
                }
            }
        }
    }

    #[test]
    fn pad_text_roundtrip_and_bad_values() {
        let mut m = PadMap::default();
        m.clear(PadFn::Split, 1);
        m.clear(PadFn::Pause, 0);
        m.bind(PadFn::Pause, 1, Btn::L3).unwrap();
        let mut entries = Vec::new();
        for line in m.to_lines().lines() {
            let (k, v) = line.split_once('=').unwrap();
            entries.push((PadFn::from_save_name(k.strip_prefix("pad.").unwrap()).unwrap(), PadMap::parse_value(v).unwrap()));
        }
        let (back, warn) = PadMap::from_entries(&entries);
        assert_eq!(back, m);
        assert!(warn.is_empty(), "{warn:?}");
        assert!(PadMap::parse_value("A").is_err() && PadMap::parse_value("A,Z").is_err() && PadMap::parse_value("A,B,X").is_err());
        assert_eq!(PadMap::parse_value("A,-").unwrap(), [Some(Btn::A), None]);
        let (m, warn) = PadMap::from_entries(&[(PadFn::Fire, [Some(Btn::B), None])]);
        assert_eq!(m.get(PadFn::Fire, 0), Some(Btn::B));
        assert_eq!(m.get(PadFn::Turn, 0), None, "B 已被開火用掉，轉向的預設落空");
        assert!(!warn.is_empty());
    }

    #[test]
    fn button_names_roundtrip_and_sdl_mapping() {
        for b in BTNS {
            assert_eq!(Btn::from_name(b.name()), Some(b));
        }
        use sdl2::controller::Button as S;
        assert_eq!(Btn::from_sdl(S::A), Some(Btn::A));
        assert_eq!(Btn::from_sdl(S::LeftShoulder), Some(Btn::LB));
        assert_eq!(Btn::from_sdl(S::DPadUp), None, "方向固定，不可指派");
        assert_eq!(Btn::from_sdl(S::Guide), None);
        assert_eq!(Act::from_save_name("downleft"), Some(Act::DownLeft));
        for a in ACTS {
            assert_eq!(Act::from_save_name(a.save_name()), Some(a));
        }
        for f in PADFNS {
            assert_eq!(PadFn::from_save_name(f.save_name()), Some(f));
        }
    }
}
