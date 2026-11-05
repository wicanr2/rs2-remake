//! 手把：SDL GameController 的事件換成鍵盤事件，再交給 `InputMapper`，遊戲看到的還是掃描碼。
//! 不依賴 SDL 子系統，好寫單元測試。第一支手把是玩家 1，第二支是玩家 2，再多的不用。
//!
//! | 手把 | 玩家 1 | 玩家 2 | 遊戲裡的作用 |
//! |---|---|---|---|
//! | 十字鍵、左搖桿 | 方向鍵 | T、B、F、H | 移動，兩軸同時推就是斜向 |
//! | A、RB、RT | 右 Shift | 左 Shift | 開火，標題畫面任意鍵開始 |
//! | B、LB、LT | `/` | Z | 砲座轉向 |
//! | X | 空白鍵 | 空白鍵 | 選單確認 |
//! | Y | C | C | 標題開選單 |
//! | Start | Esc | Esc | 暫停 |
//! | Back | Enter | Enter | 選單確認 |

use sdl2::controller::{Axis, Button};
use sdl2::keyboard::Scancode as K;
use std::collections::HashMap;

/// 搖桿推過這個值才算按下。
pub const STICK_ENGAGE: i16 = 12_000;
/// 搖桿回到這個值以內才算放開。兩個門檻不同，搖桿停在門檻附近時不會連續觸發。
pub const STICK_RELEASE: i16 = 8_000;

/// 可接的手把數，和遊戲的玩家數相同。
const MAX_PADS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadEvent {
    Down(K),
    Up(K),
}

/// 四個方向對應的鍵：上、下、左、右。
fn dir_keys(player: usize) -> [K; 4] {
    if player == 0 { [K::Up, K::Down, K::Left, K::Right] } else { [K::T, K::B, K::F, K::H] }
}

fn fire_key(player: usize) -> K {
    if player == 0 { K::RShift } else { K::LShift }
}

fn turn_key(player: usize) -> K {
    if player == 0 { K::Slash } else { K::Z }
}

/// 按鈕對應的鍵。
pub fn button_key(player: usize, b: Button) -> Option<K> {
    let [up, down, left, right] = dir_keys(player);
    Some(match b {
        Button::DPadUp => up,
        Button::DPadDown => down,
        Button::DPadLeft => left,
        Button::DPadRight => right,
        Button::A | Button::RightShoulder => fire_key(player),
        Button::B | Button::LeftShoulder => turn_key(player),
        Button::X => K::Space,
        Button::Y => K::C,
        Button::Start => K::Escape,
        Button::Back => K::Return,
        _ => return None,
    })
}

#[derive(Debug)]
struct Pad {
    id: u32,
    player: usize,
    /// 每個鍵被幾個來源按著（十字鍵與搖桿可以同時按同一個方向）。
    held: HashMap<K, u32>,
    /// 左搖桿目前的方向：-1、0、1。
    stick: [i8; 2],
    /// LT、RT 目前有沒有按著。
    triggers: [bool; 2],
}

impl Pad {
    fn press(&mut self, k: K, out: &mut Vec<PadEvent>) {
        let n = self.held.entry(k).or_insert(0);
        *n += 1;
        if *n == 1 {
            out.push(PadEvent::Down(k));
        }
    }

    fn release(&mut self, k: K, out: &mut Vec<PadEvent>) {
        if let Some(n) = self.held.get_mut(&k) {
            *n -= 1;
            if *n == 0 {
                self.held.remove(&k);
                out.push(PadEvent::Up(k));
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct PadMapper {
    pads: Vec<Pad>,
}

impl PadMapper {
    pub fn new() -> PadMapper {
        PadMapper::default()
    }

    /// 手把接上，回傳它的玩家編號。已經接滿回傳 `None`，呼叫端不必開它。
    pub fn attach(&mut self, id: u32) -> Option<usize> {
        if let Some(p) = self.pads.iter().find(|p| p.id == id) {
            return Some(p.player);
        }
        let player = (0..MAX_PADS).find(|n| !self.pads.iter().any(|p| p.player == *n))?;
        self.pads.push(Pad { id, player, held: HashMap::new(), stick: [0; 2], triggers: [false; 2] });
        Some(player)
    }

    /// 手把拔掉：放開它按著的鍵，免得卡鍵。
    pub fn detach(&mut self, id: u32) -> Vec<PadEvent> {
        let Some(i) = self.pads.iter().position(|p| p.id == id) else { return Vec::new() };
        let pad = self.pads.remove(i);
        let mut keys: Vec<K> = pad.held.into_keys().collect();
        keys.sort_by_key(|k| *k as i32);
        keys.into_iter().map(PadEvent::Up).collect()
    }

    pub fn button(&mut self, id: u32, b: Button, down: bool) -> Vec<PadEvent> {
        let mut out = Vec::new();
        let Some(pad) = self.pads.iter_mut().find(|p| p.id == id) else { return out };
        if let Some(k) = button_key(pad.player, b) {
            if down {
                pad.press(k, &mut out);
            } else {
                pad.release(k, &mut out);
            }
        }
        out
    }

    pub fn axis(&mut self, id: u32, axis: Axis, value: i16) -> Vec<PadEvent> {
        let mut out = Vec::new();
        let Some(pad) = self.pads.iter_mut().find(|p| p.id == id) else { return out };
        let [up, down, left, right] = dir_keys(pad.player);
        match axis {
            Axis::LeftX => step_stick(pad, 0, left, right, value, &mut out),
            Axis::LeftY => step_stick(pad, 1, up, down, value, &mut out),
            Axis::TriggerRight => step_trigger(pad, 1, fire_key(pad.player), value, &mut out),
            Axis::TriggerLeft => step_trigger(pad, 0, turn_key(pad.player), value, &mut out),
            Axis::RightX | Axis::RightY => {}
        }
        out
    }
}

fn step_stick(pad: &mut Pad, axis: usize, neg: K, pos: K, value: i16, out: &mut Vec<PadEvent>) {
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
    match cur {
        -1 => pad.release(neg, out),
        1 => pad.release(pos, out),
        _ => {}
    }
    match want {
        -1 => pad.press(neg, out),
        1 => pad.press(pos, out),
        _ => {}
    }
    pad.stick[axis] = want;
}

fn step_trigger(pad: &mut Pad, which: usize, k: K, value: i16, out: &mut Vec<PadEvent>) {
    let on = pad.triggers[which];
    if !on && value >= STICK_ENGAGE {
        pad.triggers[which] = true;
        pad.press(k, out);
    } else if on && value < STICK_RELEASE {
        pad.triggers[which] = false;
        pad.release(k, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PadEvent::{Down, Up};

    fn two() -> PadMapper {
        let mut m = PadMapper::new();
        assert_eq!(m.attach(10), Some(0));
        assert_eq!(m.attach(20), Some(1));
        m
    }

    #[test]
    fn first_pad_is_player_one_and_the_third_is_ignored() {
        let mut m = two();
        assert_eq!(m.attach(30), None, "遊戲只有兩個玩家");
        assert_eq!(m.attach(10), Some(0), "同一支再接一次不換位");
        assert!(m.button(30, Button::A, true).is_empty());
    }

    #[test]
    fn unplugging_frees_the_slot() {
        let mut m = two();
        m.detach(10);
        assert_eq!(m.attach(30), Some(0));
    }

    #[test]
    fn buttons_map_to_the_keys_of_each_player() {
        let mut m = two();
        assert_eq!(m.button(10, Button::A, true), vec![Down(K::RShift)]);
        assert_eq!(m.button(10, Button::A, false), vec![Up(K::RShift)]);
        assert_eq!(m.button(20, Button::A, true), vec![Down(K::LShift)]);
        assert_eq!(m.button(10, Button::B, true), vec![Down(K::Slash)]);
        assert_eq!(m.button(20, Button::B, true), vec![Down(K::Z)]);
        assert_eq!(m.button(10, Button::DPadUp, true), vec![Down(K::Up)]);
        assert_eq!(m.button(20, Button::DPadUp, true), vec![Down(K::T)]);
        assert_eq!(m.button(20, Button::DPadDown, true), vec![Down(K::B)]);
        assert_eq!(m.button(20, Button::DPadLeft, true), vec![Down(K::F)]);
        assert_eq!(m.button(20, Button::DPadRight, true), vec![Down(K::H)]);
        assert_eq!(m.button(10, Button::Start, true), vec![Down(K::Escape)]);
        assert_eq!(m.button(10, Button::Y, true), vec![Down(K::C)]);
        assert_eq!(m.button(10, Button::X, true), vec![Down(K::Space)]);
        assert_eq!(m.button(10, Button::Back, true), vec![Down(K::Return)]);
        assert!(m.button(10, Button::Guide, true).is_empty());
    }

    #[test]
    fn two_buttons_on_one_key_release_once() {
        let mut m = two();
        assert_eq!(m.button(10, Button::A, true), vec![Down(K::RShift)]);
        assert!(m.button(10, Button::RightShoulder, true).is_empty(), "已經按著");
        assert!(m.button(10, Button::A, false).is_empty(), "還有另一顆按著");
        assert_eq!(m.button(10, Button::RightShoulder, false), vec![Up(K::RShift)]);
        assert!(m.button(10, Button::RightShoulder, false).is_empty(), "多的放開事件不影響");
    }

    #[test]
    fn stick_has_a_dead_zone_and_hysteresis() {
        let mut m = two();
        assert!(m.axis(10, Axis::LeftX, STICK_ENGAGE - 1).is_empty(), "還沒過門檻");
        assert_eq!(m.axis(10, Axis::LeftX, STICK_ENGAGE), vec![Down(K::Right)]);
        assert!(m.axis(10, Axis::LeftX, STICK_RELEASE).is_empty(), "兩個門檻之間維持按著");
        assert_eq!(m.axis(10, Axis::LeftX, STICK_RELEASE - 1), vec![Up(K::Right)]);
        assert_eq!(m.axis(10, Axis::LeftX, -STICK_ENGAGE), vec![Down(K::Left)]);
        assert!(m.axis(10, Axis::LeftX, -STICK_RELEASE).is_empty());
        assert_eq!(m.axis(10, Axis::LeftX, 0), vec![Up(K::Left)]);
    }

    #[test]
    fn stick_can_flip_in_one_event() {
        let mut m = two();
        assert_eq!(m.axis(10, Axis::LeftY, 30_000), vec![Down(K::Down)]);
        assert_eq!(m.axis(10, Axis::LeftY, -30_000), vec![Up(K::Down), Down(K::Up)]);
        assert_eq!(m.axis(10, Axis::LeftY, 30_000), vec![Up(K::Up), Down(K::Down)]);
    }

    #[test]
    fn two_axes_make_a_diagonal() {
        let mut m = two();
        assert_eq!(m.axis(20, Axis::LeftX, 30_000), vec![Down(K::H)]);
        assert_eq!(m.axis(20, Axis::LeftY, -30_000), vec![Down(K::T)]);
    }

    #[test]
    fn dpad_and_stick_share_a_direction() {
        let mut m = two();
        assert_eq!(m.axis(10, Axis::LeftX, 30_000), vec![Down(K::Right)]);
        assert!(m.button(10, Button::DPadRight, true).is_empty());
        assert!(m.axis(10, Axis::LeftX, 0).is_empty(), "十字鍵還按著");
        assert_eq!(m.button(10, Button::DPadRight, false), vec![Up(K::Right)]);
    }

    #[test]
    fn triggers_act_like_buttons() {
        let mut m = two();
        assert_eq!(m.axis(10, Axis::TriggerRight, 32_000), vec![Down(K::RShift)]);
        assert!(m.axis(10, Axis::TriggerRight, 32_000).is_empty());
        assert_eq!(m.axis(10, Axis::TriggerRight, 0), vec![Up(K::RShift)]);
        assert_eq!(m.axis(20, Axis::TriggerLeft, 32_000), vec![Down(K::Z)]);
        assert!(m.axis(10, Axis::RightX, 32_000).is_empty(), "右搖桿沒有用途");
    }

    #[test]
    fn unplugging_releases_held_keys() {
        let mut m = two();
        m.button(10, Button::A, true);
        m.axis(10, Axis::LeftX, 30_000);
        let mut ev = m.detach(10);
        ev.sort_by_key(|e| match e {
            Up(k) | Down(k) => *k as i32,
        });
        assert_eq!(ev.len(), 2);
        assert!(ev.contains(&Up(K::RShift)) && ev.contains(&Up(K::Right)));
        assert!(m.detach(10).is_empty());
    }
}
