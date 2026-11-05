//! 手把：SDL GameController 的事件換成遊戲的 set 1 碼，再交給 `InputMapper` 的手把來源計數。
//! 另外管裝置清單與「第 1 人、第 2 人各用哪一支」的指派。不依賴 SDL 子系統，好寫單元測試。
//!
//! 指派要在設定畫面明確做，不自動指派，拔除時取消、重接要再指派，也不存檔（裝置編號每次啟動都會變）。
//! 沒有指派的手把不送任何鍵。
//!
//! | 手把 | 作用 |
//! |---|---|
//! | 十字鍵、左搖桿 | 固定：該玩家的上、下、左、右，兩軸同時推就是斜向 |
//! | 開火、轉向 | 該玩家的開火碼、轉向碼 |
//! | 分離 | 虛擬碼，不進遊戲（之後給「砲座像 Force」） |
//! | 暫停 | 前端暫停，不送 Esc |
//! | 確定、選單 | Enter、C |
//!
//! 按鈕對功能的對應在 `keymap::PadMap`，LT、RT 是類比扳機，過了門檻算按下。

use crate::keymap::{Act, Btn, CODE_CONFIRM, CODE_MENU, PadFn, PadMap, split_pseudo};
use sdl2::controller::{Axis, Button};
use std::collections::HashMap;

/// 搖桿推過這個值才算按下。
pub const STICK_ENGAGE: i16 = 12_000;
/// 搖桿回到這個值以內才算放開。兩個門檻不同，搖桿停在門檻附近時不會連續觸發。
pub const STICK_RELEASE: i16 = 8_000;

/// 手把給主迴圈的事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadEvent {
    /// 遊戲碼按下（set 1 make code，或分離的虛擬碼）。同一支手把上同一個碼只在第一個來源按下時送一次。
    Down(u8),
    /// 遊戲碼放開，最後一個來源放開時送。
    Up(u8),
    /// 按了暫停鈕：切換前端暫停。
    Pause,
}

// ---------------------------------------------------------------------------
// 裝置清單與指派
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dev {
    /// SDL 的 instance id，拔除與按鈕事件用它認裝置。
    pub id: u32,
    pub name: String,
    /// 有標準配置（SDL GameController）才能指派。
    pub supported: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignError {
    /// 這支手把已經不在了。
    Gone,
    /// 沒有標準配置。
    Unsupported,
    /// 已經指派給另一位玩家。
    Taken(usize),
}

/// 偵測到的手把與兩位玩家的指派。
#[derive(Clone, Debug, Default)]
pub struct Devices {
    list: Vec<Dev>,
    assigned: [Option<u32>; 2],
    version: u32,
}

impl Devices {
    pub fn new() -> Devices {
        Devices::default()
    }

    /// 內容有變就加一，畫面與 `PadMapper::sync` 用它判斷要不要更新。
    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn list(&self) -> &[Dev] {
        &self.list
    }

    pub fn add(&mut self, id: u32, name: &str, supported: bool) {
        self.version += 1;
        if let Some(d) = self.list.iter_mut().find(|d| d.id == id) {
            d.name = name.to_string();
            d.supported = supported;
            return;
        }
        self.list.push(Dev { id, name: name.to_string(), supported });
    }

    /// 拔除。回傳那支手把，以及它原本指派給哪位玩家（指派一併取消）。
    pub fn remove(&mut self, id: u32) -> Option<(Dev, Option<usize>)> {
        let i = self.list.iter().position(|d| d.id == id)?;
        let dev = self.list.remove(i);
        let mut was = None;
        for p in 0..2 {
            if self.assigned[p] == Some(id) {
                self.assigned[p] = None;
                was = Some(p);
            }
        }
        self.version += 1;
        Some((dev, was))
    }

    pub fn find(&self, id: u32) -> Option<&Dev> {
        self.list.iter().find(|d| d.id == id)
    }

    /// 這支手把指派給哪位玩家。
    pub fn player_of(&self, id: u32) -> Option<usize> {
        (0..2).find(|p| self.assigned[*p] == Some(id))
    }

    pub fn assigned_id(&self, player: usize) -> Option<u32> {
        self.assigned[player.min(1)]
    }

    pub fn assigned_dev(&self, player: usize) -> Option<&Dev> {
        self.assigned_id(player).and_then(|id| self.find(id))
    }

    /// 指派。同一支不能給兩個人，沒有標準配置的不能指派。玩家原來的手把被取代。
    pub fn assign(&mut self, player: usize, id: u32) -> Result<(), AssignError> {
        let player = player.min(1);
        let dev = self.find(id).ok_or(AssignError::Gone)?;
        if !dev.supported {
            return Err(AssignError::Unsupported);
        }
        if let Some(other) = self.player_of(id) {
            if other != player {
                return Err(AssignError::Taken(other));
            }
        }
        self.assigned[player] = Some(id);
        self.version += 1;
        Ok(())
    }

    pub fn unassign(&mut self, player: usize) {
        self.assigned[player.min(1)] = None;
        self.version += 1;
    }

    /// 設定畫面的按鈕：已經指派給這位玩家就取消，否則指派。回傳指派後是否仍在這位玩家手上。
    pub fn toggle(&mut self, player: usize, id: u32) -> Result<bool, AssignError> {
        if self.player_of(id) == Some(player.min(1)) {
            self.unassign(player);
            return Ok(false);
        }
        self.assign(player, id).map(|_| true)
    }
}

// ---------------------------------------------------------------------------
// 事件轉換
// ---------------------------------------------------------------------------

/// 按鈕按下時決定的輸出。放開時用同一個，就算對應在按著的時候改了也不會卡鍵。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Out {
    Code(u8),
    Pause,
    Nothing,
}

#[derive(Debug)]
struct Pad {
    id: u32,
    player: usize,
    /// 每個碼被幾個來源按著（十字鍵與搖桿可以同時按同一個方向，確定鈕可以有兩顆）。
    held: HashMap<u8, u32>,
    dpad: [bool; 4],
    /// 左搖桿目前的方向：-1、0、1。
    stick: [i8; 2],
    /// LT、RT 目前有沒有按著。
    triggers: [bool; 2],
    pressed: HashMap<Btn, Out>,
}

/// 上、下、左、右。
fn dir_code(player: usize, dir: usize) -> u8 {
    [Act::Up, Act::Down, Act::Left, Act::Right][dir].counted_code(player)
}

impl Pad {
    fn press(&mut self, code: u8, out: &mut Vec<PadEvent>) {
        let n = self.held.entry(code).or_insert(0);
        *n += 1;
        if *n == 1 {
            out.push(PadEvent::Down(code));
        }
    }

    fn release(&mut self, code: u8, out: &mut Vec<PadEvent>) {
        if let Some(n) = self.held.get_mut(&code) {
            *n -= 1;
            if *n == 0 {
                self.held.remove(&code);
                out.push(PadEvent::Up(code));
            }
        }
    }

    fn out_of(&self, f: PadFn) -> Out {
        match f {
            PadFn::Fire => Out::Code(Act::Fire.counted_code(self.player)),
            PadFn::Turn => Out::Code(Act::Turn.counted_code(self.player)),
            PadFn::Split => Out::Code(split_pseudo(self.player)),
            PadFn::Pause => Out::Pause,
            PadFn::Confirm => Out::Code(CODE_CONFIRM),
            PadFn::Menu => Out::Code(CODE_MENU),
        }
    }

    fn btn(&mut self, map: &PadMap, b: Btn, down: bool, out: &mut Vec<PadEvent>) {
        if down {
            if self.pressed.contains_key(&b) {
                return;
            }
            let o = match map.find(b) {
                Some((f, _)) => self.out_of(f),
                None => Out::Nothing,
            };
            self.pressed.insert(b, o);
            match o {
                Out::Code(c) => self.press(c, out),
                Out::Pause => out.push(PadEvent::Pause),
                Out::Nothing => {}
            }
        } else if let Some(o) = self.pressed.remove(&b) {
            if let Out::Code(c) = o {
                self.release(c, out);
            }
        }
    }

    fn all_up(&mut self) -> Vec<PadEvent> {
        let mut codes: Vec<u8> = self.held.keys().copied().collect();
        codes.sort_unstable();
        self.held.clear();
        self.dpad = [false; 4];
        self.stick = [0; 2];
        self.triggers = [false; 2];
        self.pressed.clear();
        codes.into_iter().map(PadEvent::Up).collect()
    }
}

#[derive(Debug, Default)]
pub struct PadMapper {
    map: PadMap,
    /// 依玩家編號：指派給第 1 人、第 2 人的手把。
    pads: [Option<Pad>; 2],
}

impl PadMapper {
    pub fn new() -> PadMapper {
        PadMapper::default()
    }

    /// 換按鈕對應。已經按著的鈕不受影響，放開時用按下時的輸出。
    pub fn set_map(&mut self, m: PadMap) {
        self.map = m;
    }

    /// 讓手把狀態和 `devs` 的指派一致：換了手把或玩家的就放開舊的鍵，新指派的從空白狀態開始。
    pub fn sync(&mut self, devs: &Devices) -> Vec<PadEvent> {
        let mut out = Vec::new();
        for p in 0..2 {
            let want = devs.assigned_id(p);
            if self.pads[p].as_ref().map(|x| x.id) == want {
                continue;
            }
            if let Some(mut old) = self.pads[p].take() {
                out.extend(old.all_up());
            }
            if let Some(id) = want {
                self.pads[p] = Some(Pad { id, player: p, held: HashMap::new(), dpad: [false; 4], stick: [0; 2], triggers: [false; 2], pressed: HashMap::new() });
            }
        }
        out
    }

    /// 手把拔掉：放開它按著的鍵，免得卡鍵。呼叫端另外要從 `Devices` 移除並取消指派。
    pub fn detach(&mut self, id: u32) -> Vec<PadEvent> {
        for p in 0..2 {
            if self.pads[p].as_ref().is_some_and(|x| x.id == id) {
                if let Some(mut old) = self.pads[p].take() {
                    return old.all_up();
                }
            }
        }
        Vec::new()
    }

    /// 全部放開並清掉內部狀態（視窗失焦時背景裡錯過的放開事件不會留下卡住的鍵）。指派不變。
    pub fn release_all(&mut self) -> Vec<PadEvent> {
        let mut out = Vec::new();
        for pad in self.pads.iter_mut().flatten() {
            out.extend(pad.all_up());
        }
        out
    }

    fn pad_mut(&mut self, id: u32) -> Option<&mut Pad> {
        self.pads.iter_mut().flatten().find(|p| p.id == id)
    }

    pub fn button(&mut self, id: u32, b: Button, down: bool) -> Vec<PadEvent> {
        let mut out = Vec::new();
        let map = self.map;
        let Some(pad) = self.pad_mut(id) else { return out };
        let dir = match b {
            Button::DPadUp => Some(0),
            Button::DPadDown => Some(1),
            Button::DPadLeft => Some(2),
            Button::DPadRight => Some(3),
            _ => None,
        };
        if let Some(d) = dir {
            if pad.dpad[d] != down {
                pad.dpad[d] = down;
                let code = dir_code(pad.player, d);
                if down {
                    pad.press(code, &mut out);
                } else {
                    pad.release(code, &mut out);
                }
            }
        } else if let Some(btn) = Btn::from_sdl(b) {
            pad.btn(&map, btn, down, &mut out);
        }
        out
    }

    pub fn axis(&mut self, id: u32, axis: Axis, value: i16) -> Vec<PadEvent> {
        let mut out = Vec::new();
        let map = self.map;
        let Some(pad) = self.pad_mut(id) else { return out };
        match axis {
            Axis::LeftX => step_stick(pad, 0, 2, 3, value, &mut out),
            Axis::LeftY => step_stick(pad, 1, 0, 1, value, &mut out),
            Axis::TriggerRight => step_trigger(pad, &map, 1, Btn::RT, value, &mut out),
            Axis::TriggerLeft => step_trigger(pad, &map, 0, Btn::LT, value, &mut out),
            Axis::RightX | Axis::RightY => {}
        }
        out
    }
}

/// `neg`、`pos` 是方向編號（上 0、下 1、左 2、右 3）。
fn step_stick(pad: &mut Pad, axis: usize, neg: usize, pos: usize, value: i16, out: &mut Vec<PadEvent>) {
    let cur = pad.stick[axis];
    let v = value as i32;
    let want = match cur {
        // 推著的時候要回到放開門檻以內，或直接推到另一邊，才改變。
        -1 if v > -(STICK_RELEASE as i32) => i8::from(v >= STICK_ENGAGE as i32),
        1 if v < STICK_RELEASE as i32 => -i8::from(v <= -(STICK_ENGAGE as i32)),
        0 if v >= STICK_ENGAGE as i32 => 1,
        0 if v <= -(STICK_ENGAGE as i32) => -1,
        c => c,
    };
    if want == cur {
        return;
    }
    let player = pad.player;
    match cur {
        -1 => pad.release(dir_code(player, neg), out),
        1 => pad.release(dir_code(player, pos), out),
        _ => {}
    }
    match want {
        -1 => pad.press(dir_code(player, neg), out),
        1 => pad.press(dir_code(player, pos), out),
        _ => {}
    }
    pad.stick[axis] = want;
}

fn step_trigger(pad: &mut Pad, map: &PadMap, which: usize, b: Btn, value: i16, out: &mut Vec<PadEvent>) {
    let on = pad.triggers[which];
    if !on && value >= STICK_ENGAGE {
        pad.triggers[which] = true;
        pad.btn(map, b, true, out);
    } else if on && value < STICK_RELEASE {
        pad.triggers[which] = false;
        pad.btn(map, b, false, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PadEvent::{Down, Pause, Up};

    // 玩家 1、2 的碼。
    const P1_UP: u8 = 0x48;
    const P1_RIGHT: u8 = 0x4D;
    const P1_FIRE: u8 = 0x36;
    const P1_TURN: u8 = 0x35;
    const P2_UP: u8 = 0x14;
    const P2_DOWN: u8 = 0x30;
    const P2_LEFT: u8 = 0x21;
    const P2_RIGHT: u8 = 0x23;
    const P2_FIRE: u8 = 0x2A;
    const P2_TURN: u8 = 0x2C;

    fn devs() -> Devices {
        let mut d = Devices::new();
        d.add(10, "Pad A", true);
        d.add(20, "Pad B", true);
        d
    }

    fn two() -> (PadMapper, Devices) {
        let mut d = devs();
        d.assign(0, 10).unwrap();
        d.assign(1, 20).unwrap();
        let mut m = PadMapper::new();
        m.set_map(PadMap::default());
        assert!(m.sync(&d).is_empty());
        (m, d)
    }

    #[test]
    fn assignment_rejects_duplicates_and_unsupported() {
        let mut d = devs();
        d.add(30, "Odd", false);
        assert_eq!(d.assign(0, 10), Ok(()));
        assert_eq!(d.assign(1, 10), Err(AssignError::Taken(0)), "同一支不能給兩個人");
        assert_eq!(d.assign(1, 30), Err(AssignError::Unsupported), "沒有標準配置不能指派");
        assert_eq!(d.assign(1, 99), Err(AssignError::Gone));
        assert_eq!(d.assigned_id(1), None, "拒絕時不改指派");
        assert_eq!(d.assign(0, 10), Ok(()), "重複指派給同一人沒事");
        assert_eq!(d.assign(1, 20), Ok(()));
        // 玩家換手把：舊的那支放掉，可以給別人。
        assert_eq!(d.assign(0, 20), Err(AssignError::Taken(1)));
        d.unassign(1);
        assert_eq!(d.assign(0, 20), Ok(()));
        assert_eq!(d.player_of(10), None);
        assert_eq!(d.assigned_dev(0).map(|x| x.name.as_str()), Some("Pad B"));
    }

    #[test]
    fn toggle_assigns_then_unassigns() {
        let mut d = devs();
        assert_eq!(d.toggle(0, 10), Ok(true));
        assert_eq!(d.toggle(0, 10), Ok(false));
        assert_eq!(d.player_of(10), None);
        d.toggle(0, 10).unwrap();
        assert_eq!(d.toggle(1, 10), Err(AssignError::Taken(0)));
    }

    #[test]
    fn unplugging_cancels_the_assignment_and_replugging_needs_a_new_one() {
        let mut d = devs();
        d.assign(1, 20).unwrap();
        let v = d.version();
        let (dev, was) = d.remove(20).unwrap();
        assert_eq!((dev.name.as_str(), was), ("Pad B", Some(1)));
        assert!(d.version() > v);
        assert_eq!(d.assigned_id(1), None);
        assert!(d.remove(20).is_none());
        // 重接：同一個 id 或新 id 都是新的、沒指派的裝置。
        d.add(21, "Pad B", true);
        assert_eq!(d.player_of(21), None);
        assert_eq!(d.assigned_id(1), None);
        assert_eq!(d.remove(10).map(|(_, p)| p), Some(None), "沒指派的手把拔除，沒有玩家受影響");
    }

    #[test]
    fn only_assigned_pads_send_keys() {
        let mut d = devs();
        let mut m = PadMapper::new();
        m.sync(&d);
        assert!(m.button(10, Button::A, true).is_empty(), "沒指派不送鍵");
        d.assign(0, 10).unwrap();
        m.sync(&d);
        assert_eq!(m.button(10, Button::A, true), vec![Down(P1_FIRE)]);
        assert!(m.button(20, Button::A, true).is_empty(), "另一支沒指派");
    }

    #[test]
    fn buttons_map_to_the_codes_of_each_player() {
        let (mut m, _) = two();
        assert_eq!(m.button(10, Button::A, true), vec![Down(P1_FIRE)]);
        assert_eq!(m.button(10, Button::A, false), vec![Up(P1_FIRE)]);
        assert_eq!(m.button(20, Button::A, true), vec![Down(P2_FIRE)]);
        assert_eq!(m.button(10, Button::B, true), vec![Down(P1_TURN)]);
        assert_eq!(m.button(20, Button::B, true), vec![Down(P2_TURN)]);
        assert_eq!(m.button(10, Button::DPadUp, true), vec![Down(P1_UP)]);
        assert_eq!(m.button(20, Button::DPadUp, true), vec![Down(P2_UP)]);
        assert_eq!(m.button(20, Button::DPadDown, true), vec![Down(P2_DOWN)]);
        assert_eq!(m.button(20, Button::DPadLeft, true), vec![Down(P2_LEFT)]);
        assert_eq!(m.button(20, Button::DPadRight, true), vec![Down(P2_RIGHT)]);
        assert_eq!(m.button(10, Button::X, true), vec![Down(CODE_CONFIRM)]);
        assert_eq!(m.button(10, Button::Y, true), vec![Down(CODE_MENU)]);
        assert!(m.button(10, Button::Guide, true).is_empty());
        assert!(m.button(10, Button::Misc1, true).is_empty());
    }

    #[test]
    fn default_shoulders_and_triggers() {
        let (mut m, _) = two();
        // 預設：RT 同開火，LT 同轉向，RB 是分離（虛擬碼，不進遊戲）。
        assert_eq!(m.axis(10, Axis::TriggerRight, 32_000), vec![Down(P1_FIRE)]);
        assert_eq!(m.axis(10, Axis::TriggerLeft, 32_000), vec![Down(P1_TURN)]);
        assert_eq!(m.button(10, Button::RightShoulder, true), vec![Down(split_pseudo(0))]);
        assert_eq!(m.button(20, Button::RightShoulder, true), vec![Down(split_pseudo(1))]);
        assert!(crate::keymap::is_pseudo(split_pseudo(0)));
    }

    #[test]
    fn start_pauses_the_front_end_and_sends_nothing_to_the_game() {
        let (mut m, _) = two();
        assert_eq!(m.button(10, Button::Start, true), vec![Pause]);
        assert!(m.button(10, Button::Start, false).is_empty());
        assert!(m.button(10, Button::Start, true) == vec![Pause], "再按一次再送一次");
    }

    #[test]
    fn two_buttons_on_one_code_release_once() {
        let (mut m, _) = two();
        // 確定：預設 X 與 Back 都是 Enter。
        assert_eq!(m.button(10, Button::X, true), vec![Down(CODE_CONFIRM)]);
        assert!(m.button(10, Button::Back, true).is_empty(), "已經按著");
        assert!(m.button(10, Button::X, false).is_empty(), "還有另一顆按著");
        assert_eq!(m.button(10, Button::Back, false), vec![Up(CODE_CONFIRM)]);
        assert!(m.button(10, Button::Back, false).is_empty(), "多的放開事件不影響");
    }

    #[test]
    fn remapping_changes_new_presses_and_never_sticks_a_held_button() {
        let (mut m, _) = two();
        assert_eq!(m.button(10, Button::A, true), vec![Down(P1_FIRE)]);
        let mut map = PadMap::default();
        map.clear(PadFn::Fire, 0);
        map.bind(PadFn::Turn, 1, Btn::A).unwrap();
        m.set_map(map);
        // 按著的時候改對應：放開用按下時的輸出。
        assert_eq!(m.button(10, Button::A, false), vec![Up(P1_FIRE)]);
        // 新的按下走新對應：A 現在是轉向。
        assert_eq!(m.button(10, Button::A, true), vec![Down(P1_TURN)]);
        assert_eq!(m.button(10, Button::A, false), vec![Up(P1_TURN)]);
        // 沒對應的按鈕什麼都不送。
        m.set_map({
            let mut e = PadMap::default();
            e.clear(PadFn::Confirm, 0);
            e
        });
        assert!(m.button(10, Button::X, true).is_empty());
        assert!(m.button(10, Button::X, false).is_empty());
    }

    #[test]
    fn stick_has_a_dead_zone_and_hysteresis() {
        let (mut m, _) = two();
        assert!(m.axis(10, Axis::LeftX, STICK_ENGAGE - 1).is_empty(), "還沒過門檻");
        assert_eq!(m.axis(10, Axis::LeftX, STICK_ENGAGE), vec![Down(P1_RIGHT)]);
        assert!(m.axis(10, Axis::LeftX, STICK_RELEASE).is_empty(), "兩個門檻之間維持按著");
        assert_eq!(m.axis(10, Axis::LeftX, STICK_RELEASE - 1), vec![Up(P1_RIGHT)]);
        assert_eq!(m.axis(10, Axis::LeftX, -STICK_ENGAGE), vec![Down(0x4B)]);
        assert!(m.axis(10, Axis::LeftX, -STICK_RELEASE).is_empty());
        assert_eq!(m.axis(10, Axis::LeftX, 0), vec![Up(0x4B)]);
    }

    #[test]
    fn stick_can_flip_in_one_event() {
        let (mut m, _) = two();
        assert_eq!(m.axis(10, Axis::LeftY, 30_000), vec![Down(0x50)]);
        assert_eq!(m.axis(10, Axis::LeftY, -30_000), vec![Up(0x50), Down(P1_UP)]);
        assert_eq!(m.axis(10, Axis::LeftY, 30_000), vec![Up(P1_UP), Down(0x50)]);
    }

    #[test]
    fn two_axes_make_a_diagonal() {
        let (mut m, _) = two();
        assert_eq!(m.axis(20, Axis::LeftX, 30_000), vec![Down(P2_RIGHT)]);
        assert_eq!(m.axis(20, Axis::LeftY, -30_000), vec![Down(P2_UP)]);
    }

    #[test]
    fn dpad_and_stick_share_a_direction() {
        let (mut m, _) = two();
        assert_eq!(m.axis(10, Axis::LeftX, 30_000), vec![Down(P1_RIGHT)]);
        assert!(m.button(10, Button::DPadRight, true).is_empty());
        assert!(m.axis(10, Axis::LeftX, 0).is_empty(), "十字鍵還按著");
        assert_eq!(m.button(10, Button::DPadRight, false), vec![Up(P1_RIGHT)]);
    }

    #[test]
    fn triggers_act_like_buttons_and_the_right_stick_is_unused() {
        let (mut m, _) = two();
        assert_eq!(m.axis(10, Axis::TriggerRight, 32_000), vec![Down(P1_FIRE)]);
        assert!(m.axis(10, Axis::TriggerRight, 32_000).is_empty());
        assert_eq!(m.axis(10, Axis::TriggerRight, 0), vec![Up(P1_FIRE)]);
        assert_eq!(m.axis(20, Axis::TriggerLeft, 32_000), vec![Down(P2_TURN)]);
        assert!(m.axis(10, Axis::RightX, 32_000).is_empty());
    }

    #[test]
    fn unplugging_releases_held_keys() {
        let (mut m, _) = two();
        m.button(10, Button::A, true);
        m.axis(10, Axis::LeftX, 30_000);
        let ev = m.detach(10);
        assert_eq!(ev.len(), 2);
        assert!(ev.contains(&Up(P1_FIRE)) && ev.contains(&Up(P1_RIGHT)));
        assert!(m.detach(10).is_empty());
        assert!(m.button(10, Button::A, true).is_empty(), "拔掉之後事件不再有作用");
    }

    #[test]
    fn sync_releases_keys_when_the_assignment_changes() {
        let (mut m, mut d) = two();
        m.button(10, Button::A, true);
        m.button(20, Button::DPadUp, true);
        // 取消第 1 人的指派：它按著的鍵放開。
        d.unassign(0);
        assert_eq!(m.sync(&d), vec![Up(P1_FIRE)]);
        assert!(m.button(10, Button::A, true).is_empty());
        // 第 2 人換成另一支（先給第 1 人用過的 10）：舊的放開，新的從空白開始，而且用第 2 人的碼。
        d.assign(1, 10).unwrap();
        assert_eq!(m.sync(&d), vec![Up(P2_UP)]);
        assert_eq!(m.button(10, Button::A, true), vec![Down(P2_FIRE)]);
        assert!(m.sync(&d).is_empty(), "沒有變化不送事件");
    }

    #[test]
    fn release_all_clears_internal_state_so_missed_releases_do_not_stick() {
        let (mut m, _) = two();
        m.button(10, Button::A, true);
        m.button(10, Button::DPadLeft, true);
        m.axis(10, Axis::TriggerLeft, 30_000);
        let mut ev = m.release_all();
        ev.sort_by_key(|e| match e {
            Up(c) | Down(c) => *c,
            Pause => 0,
        });
        assert_eq!(ev, vec![Up(0x35), Up(0x36), Up(0x4B)]);
        assert!(m.release_all().is_empty());
        // 失焦時錯過的放開事件之後，再按照樣送得出去。
        assert_eq!(m.button(10, Button::A, true), vec![Down(P1_FIRE)]);
        assert_eq!(m.axis(10, Axis::TriggerLeft, 30_000), vec![Down(P1_TURN)]);
    }
}
