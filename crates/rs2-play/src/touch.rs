//! 觸控層：手指事件換成遊戲碼，交給 `InputMapper` 的觸控來源計數。設計見 `docs/android/touch-design.md`，尺寸的單位是 dp。
//! 不依賴 SDL 子系統，好寫單元測試。
//!
//! 觸控是第 1 人。方向盤送第 1 人的方向碼；右手兩顆鈕在遊玩中是開火與轉向，其餘時候是確定（Enter）與選單（C）；
//! 暫停鈕是前端暫停，不送進遊戲；☰ 展開工具列。

use crate::keymap::{Act, CODE_CONFIRM, CODE_MENU};
use crate::pad::PadEvent;
use crate::ui::{font, text};
use std::collections::HashMap;

/// 方向盤中心的死區（dp）。
const DEAD_DP: f32 = 14.0;
/// 控制鈕的填色與外框。
const FILL: [u8; 4] = [232, 236, 248, 70];
const FILL_MAIN: [u8; 4] = [255, 180, 84, 90];
const EDGE: [u8; 4] = [232, 236, 248, 170];
const EDGE_MAIN: [u8; 4] = [255, 180, 84, 200];
const LABEL: [u8; 4] = [255, 255, 255, 220];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Dpad,
    /// 遊玩中是開火，其餘是確定。
    Primary,
    /// 遊玩中是砲座轉向，其餘是選單。
    Secondary,
    /// 前端暫停。
    Pause,
    /// 展開工具列。
    Menu,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Control {
    pub role: Role,
    /// 中心，視窗像素。
    pub x: f32,
    pub y: f32,
    /// 外觀半徑。
    pub r: f32,
    /// 判定半徑，比外觀大。
    pub hit: f32,
}

/// 依視窗大小排出控制鈕。`dp` 是每 dp 幾個像素。視窗太矮時縮小，全部放得下。
pub fn layout(win_w: u32, win_h: u32, dp: f32) -> Vec<Control> {
    let (w, h) = (win_w as f32, win_h as f32);
    // 設計稿的高度是 432 dp，低於 360 dp 就放不下了，再按比例縮。
    let d = dp.min(h / 360.0).max(0.1);
    let c = |role, x: f32, y: f32, r: f32, hit: f32| Control { role, x, y, r: r * d, hit: hit * d };
    vec![
        c(Role::Dpad, 68.0 * d, h - 78.0 * d, 64.0, 92.0),
        c(Role::Primary, w - 62.0 * d, h - 62.0 * d, 42.0, 52.0),
        c(Role::Secondary, w - 62.0 * d, h - 146.0 * d, 30.0, 38.0),
        c(Role::Pause, w - 38.0 * d, 38.0 * d, 24.0, 32.0),
        c(Role::Menu, 38.0 * d, 38.0 * d, 24.0, 32.0),
    ]
}

/// 距離中心 `(dx, dy)` 的方向，最多兩個。八個扇區，每個 45 度。`dead` 以內沒有方向。
pub fn dpad_acts(dx: f32, dy: f32, dead: f32) -> Vec<Act> {
    if dx * dx + dy * dy < dead * dead {
        return Vec::new();
    }
    // 0 度朝右，順時針（螢幕的 y 朝下）。
    let deg = dy.atan2(dx).to_degrees().rem_euclid(360.0);
    let sector = (((deg + 22.5) / 45.0) as i32).rem_euclid(8);
    match sector {
        0 => vec![Act::Right],
        1 => vec![Act::Right, Act::Down],
        2 => vec![Act::Down],
        3 => vec![Act::Left, Act::Down],
        4 => vec![Act::Left],
        5 => vec![Act::Left, Act::Up],
        6 => vec![Act::Up],
        _ => vec![Act::Right, Act::Up],
    }
}

/// 第 1 人的方向碼。
fn dir_code(a: Act) -> u8 {
    a.counted_code(0)
}

/// 右手兩顆鈕送的碼。暫停與 ☰ 沒有。
fn code_of(role: Role, playable: bool) -> Option<u8> {
    Some(match (role, playable) {
        (Role::Primary, true) => Act::Fire.counted_code(0),
        (Role::Primary, false) => CODE_CONFIRM,
        (Role::Secondary, true) => Act::Turn.counted_code(0),
        (Role::Secondary, false) => CODE_MENU,
        _ => return None,
    })
}

#[derive(Debug)]
enum Finger {
    /// 綁在方向盤上，記著目前按的方向。
    Dpad(Vec<Act>),
    /// 按著一顆鈕。放開時要放開按下時的那個碼，就算遊戲狀態已經換了。
    Key(u8),
    /// 按在工具列或沒有控制的地方。
    Idle,
}

/// 手指按下的結果。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Touch {
    pub keys: Vec<PadEvent>,
    /// 按到 ☰。
    pub toggle_toolbar: bool,
    /// 按到暫停鈕。
    pub toggle_pause: bool,
}

/// 手指追蹤。同一個方向可能有兩根手指按著，所以依按住的來源計數，和手把相同的規則。
#[derive(Debug)]
pub struct TouchMapper {
    controls: Vec<Control>,
    dead: f32,
    fingers: HashMap<i64, Finger>,
    held: HashMap<u8, u32>,
}

impl TouchMapper {
    pub fn new(win_w: u32, win_h: u32, dp: f32) -> TouchMapper {
        let d = dp.min(win_h as f32 / 360.0).max(0.1);
        TouchMapper { controls: layout(win_w, win_h, dp), dead: DEAD_DP * d, fingers: HashMap::new(), held: HashMap::new() }
    }

    pub fn controls(&self) -> &[Control] {
        &self.controls
    }

    /// 視窗大小變了（旋轉、分割畫面）。手指都放開。
    pub fn resize(&mut self, win_w: u32, win_h: u32, dp: f32) -> Vec<PadEvent> {
        let ev = self.release_all();
        *self = TouchMapper::new(win_w, win_h, dp);
        ev
    }

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

    /// `(x, y)` 是視窗像素。`in_toolbar` 表示這一點落在工具列上，交給工具列處理。
    pub fn down(&mut self, id: i64, x: f32, y: f32, playable: bool, in_toolbar: bool) -> Touch {
        let mut t = Touch::default();
        if in_toolbar {
            self.fingers.insert(id, Finger::Idle);
            return t;
        }
        // 判定範圍可能重疊，取最近的。
        let hit = self
            .controls
            .iter()
            .filter(|c| (x - c.x).powi(2) + (y - c.y).powi(2) <= c.hit * c.hit)
            .min_by(|a, b| {
                let da = (x - a.x).powi(2) + (y - a.y).powi(2);
                let db = (x - b.x).powi(2) + (y - b.y).powi(2);
                da.total_cmp(&db)
            })
            .copied();
        let Some(c) = hit else {
            self.fingers.insert(id, Finger::Idle);
            return t;
        };
        match c.role {
            Role::Dpad => {
                let acts = dpad_acts(x - c.x, y - c.y, self.dead);
                for &a in &acts {
                    self.press(dir_code(a), &mut t.keys);
                }
                self.fingers.insert(id, Finger::Dpad(acts));
            }
            Role::Menu => {
                t.toggle_toolbar = true;
                self.fingers.insert(id, Finger::Idle);
            }
            Role::Pause => {
                t.toggle_pause = true;
                self.fingers.insert(id, Finger::Idle);
            }
            role => match code_of(role, playable) {
                Some(code) => {
                    self.press(code, &mut t.keys);
                    self.fingers.insert(id, Finger::Key(code));
                }
                None => {
                    self.fingers.insert(id, Finger::Idle);
                }
            },
        }
        t
    }

    pub fn moved(&mut self, id: i64, x: f32, y: f32) -> Vec<PadEvent> {
        let mut out = Vec::new();
        let Some(Finger::Dpad(old)) = self.fingers.get(&id) else { return out };
        let old = old.clone();
        let Some(c) = self.controls.iter().find(|c| c.role == Role::Dpad).copied() else { return out };
        let new = dpad_acts(x - c.x, y - c.y, self.dead);
        if new == old {
            return out;
        }
        for &a in old.iter().filter(|a| !new.contains(a)) {
            self.release(dir_code(a), &mut out);
        }
        for &a in new.iter().filter(|a| !old.contains(a)) {
            self.press(dir_code(a), &mut out);
        }
        self.fingers.insert(id, Finger::Dpad(new));
        out
    }

    pub fn up(&mut self, id: i64) -> Vec<PadEvent> {
        let mut out = Vec::new();
        match self.fingers.remove(&id) {
            Some(Finger::Dpad(acts)) => {
                for a in acts {
                    self.release(dir_code(a), &mut out);
                }
            }
            Some(Finger::Key(code)) => self.release(code, &mut out),
            _ => {}
        }
        out
    }

    /// 切到背景或視窗換大小：全部放開，免得卡鍵。
    pub fn release_all(&mut self) -> Vec<PadEvent> {
        let ids: Vec<i64> = self.fingers.keys().copied().collect();
        let mut out = Vec::new();
        for id in ids {
            out.extend(self.up(id));
        }
        out
    }
}

/// 控制鈕的文字。
fn label(role: Role, playable: bool) -> &'static str {
    match (role, playable) {
        (Role::Dpad, _) => "",
        (Role::Primary, true) => text::TOUCH_FIRE,
        (Role::Primary, false) => text::TOUCH_OK,
        (Role::Secondary, true) => text::TOUCH_TURN,
        (Role::Secondary, false) => text::TOUCH_MENU,
        // 這兩顆畫圖示。
        (Role::Pause, _) | (Role::Menu, _) => "",
    }
}

/// 把顏色 `c` 蓋到 `buf` 的像素上。貼圖用非預乘的 alpha（SDL 的 `BlendMode::Blend`），所以顏色不乘 alpha。
fn blend(buf: &mut [u8], o: usize, c: [u8; 4]) {
    let sa = c[3] as u32;
    let da = buf[o + 3] as u32;
    let out_a = sa * 255 + da * (255 - sa);
    if out_a == 0 {
        return;
    }
    for i in 0..3 {
        buf[o + i] = ((c[i] as u32 * sa * 255 + buf[o + i] as u32 * da * (255 - sa)) / out_a) as u8;
    }
    buf[o + 3] = (out_a / 255) as u8;
}

/// 實心長方形，給圖示用。
fn bar(buf: &mut [u8], w: usize, h: usize, x0: f32, y0: f32, x1: f32, y1: f32, c: [u8; 4]) {
    for y in (y0.max(0.0) as usize)..(y1.min(h as f32) as usize) {
        for x in (x0.max(0.0) as usize)..(x1.min(w as f32) as usize) {
            blend(buf, (y * w + x) * 4, c);
        }
    }
}

/// 控制鈕畫成 `w × h` 的 RGBA，背景透明。只有版面或遊戲狀態改變時才重畫。
pub fn render_overlay(controls: &[Control], playable: bool, w: usize, h: usize) -> Vec<u8> {
    let mut buf = vec![0u8; w * h * 4];
    for c in controls {
        let main = c.role == Role::Primary;
        let (fill, edge) = if main { (FILL_MAIN, EDGE_MAIN) } else { (FILL, EDGE) };
        let r = c.r;
        let (x0, x1) = (((c.x - r - 1.0).floor().max(0.0)) as usize, ((c.x + r + 1.0).ceil() as usize).min(w));
        let (y0, y1) = (((c.y - r - 1.0).floor().max(0.0)) as usize, ((c.y + r + 1.0).ceil() as usize).min(h));
        let thick = (r * 0.05).max(1.5);
        for y in y0..y1 {
            for x in x0..x1 {
                let d = ((x as f32 + 0.5 - c.x).powi(2) + (y as f32 + 0.5 - c.y).powi(2)).sqrt();
                if d > r {
                    continue;
                }
                let o = (y * w + x) * 4;
                blend(&mut buf, o, if d > r - thick { edge } else { fill });
            }
        }
        if c.role == Role::Dpad {
            // 方向盤畫四個箭頭。
            for (dx, dy) in [(0.0f32, -1.0f32), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
                let tip = (c.x + dx * r * 0.82, c.y + dy * r * 0.82);
                let size = r * 0.2;
                let n = (size * 2.0) as i32;
                for i in 0..n {
                    let t = i as f32 / n as f32;
                    // 三角形：從箭頭尖往中心變寬。
                    let cx = tip.0 - dx * size * t;
                    let cy = tip.1 - dy * size * t;
                    let half = (size * t) as i32;
                    for s in -half..=half {
                        let px = (cx + dy.abs() * s as f32) as i32;
                        let py = (cy + dx.abs() * s as f32) as i32;
                        if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                            let o = (py as usize * w + px as usize) * 4;
                            blend(&mut buf, o, LABEL);
                        }
                    }
                }
            }
            continue;
        }
        let s = r * 0.42;
        match c.role {
            // 暫停：兩條直線。
            Role::Pause => {
                bar(&mut buf, w, h, c.x - s * 0.8, c.y - s, c.x - s * 0.25, c.y + s, LABEL);
                bar(&mut buf, w, h, c.x + s * 0.25, c.y - s, c.x + s * 0.8, c.y + s, LABEL);
            }
            // 工具列：三條橫線。
            Role::Menu => {
                for k in [-0.8f32, -0.14, 0.52] {
                    bar(&mut buf, w, h, c.x - s, c.y + s * k, c.x + s, c.y + s * (k + 0.28), LABEL);
                }
            }
            _ => {}
        }
        let t = label(c.role, playable);
        if !t.is_empty() {
            // 字高約半徑的 55%，兩個字的寬度約半徑的 1.1 倍，放得進圓裡。
            let fh = ((r * 0.55) as i32).max(12);
            let tw = font::builtin().text_width(t, fh);
            let full = crate::ui::Rect { x: 0, y: 0, w: w as i32, h: h as i32 };
            font::builtin().draw(&mut buf, w as i32, h as i32, full, (c.x - tw as f32 / 2.0) as i32, (c.y - fh as f32 / 2.0) as i32, t, fh, LABEL);
        }
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use PadEvent::{Down, Up};

    const W: u32 = 2400;
    const H: u32 = 1080;
    const DP: f32 = 2.5;

    const UP: u8 = 0x48;
    const DOWN: u8 = 0x50;
    const LEFT: u8 = 0x4B;
    const RIGHT: u8 = 0x4D;
    const FIRE: u8 = 0x36;
    const TURN: u8 = 0x35;

    fn map() -> TouchMapper {
        TouchMapper::new(W, H, DP)
    }

    fn at(m: &TouchMapper, role: Role) -> (f32, f32) {
        let c = m.controls().iter().find(|c| c.role == role).unwrap();
        (c.x, c.y)
    }

    #[test]
    fn layout_fits_the_window_and_meets_the_minimum_size() {
        for (w, h, dp) in [(2400u32, 1080u32, 2.5f32), (1920, 1080, 2.0), (1280, 800, 2.0), (800, 480, 1.5), (640, 360, 1.0)] {
            for c in layout(w, h, dp) {
                assert!(c.x - c.r >= 0.0 && c.x + c.r <= w as f32, "{c:?} 超出 {w}x{h}");
                assert!(c.y - c.r >= 0.0 && c.y + c.r <= h as f32, "{c:?} 超出 {w}x{h}");
                assert!(c.hit >= c.r);
            }
        }
        // 手機上最小的按鈕直徑 48 dp。
        let min = layout(W, H, DP).iter().map(|c| c.r).fold(f32::MAX, f32::min);
        assert!(min * 2.0 >= 48.0 * DP);
    }

    #[test]
    fn right_hand_buttons_stay_out_of_the_game_on_a_wide_phone() {
        // 2400×1080：遊戲 1728 寬置中，右邊留白從 x = 2064 起。
        for c in layout(W, H, DP) {
            if matches!(c.role, Role::Primary | Role::Secondary) {
                assert!(c.x - c.r >= 2064.0, "{:?} 蓋到遊戲：{}", c.role, c.x - c.r);
            }
        }
    }

    #[test]
    fn overlay_colour_is_not_darkened_by_its_own_alpha() {
        let mut px = [0u8; 4];
        blend(&mut px, 0, [200, 100, 50, 70]);
        assert_eq!(px, [200, 100, 50, 70], "透明底上疊一次，顏色要維持原值");
        blend(&mut px, 0, [200, 100, 50, 255]);
        assert_eq!(px, [200, 100, 50, 255]);
        let mut px = [0u8, 0, 0, 255];
        blend(&mut px, 0, [255, 255, 255, 128]);
        assert!(px[0] > 120 && px[0] < 135 && px[3] == 255, "{px:?}");
    }

    #[test]
    fn controls_do_not_overlap_on_a_phone() {
        let l = layout(W, H, DP);
        for (i, a) in l.iter().enumerate() {
            for b in &l[i + 1..] {
                let d = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
                assert!(d >= a.r + b.r, "{:?} 和 {:?} 重疊", a.role, b.role);
            }
        }
    }

    #[test]
    fn dpad_has_eight_sectors_and_a_dead_zone() {
        let k = |deg: f32| {
            let a = deg.to_radians();
            dpad_acts(a.cos() * 100.0, a.sin() * 100.0, 10.0)
        };
        assert_eq!(k(0.0), vec![Act::Right]);
        assert_eq!(k(45.0), vec![Act::Right, Act::Down]);
        assert_eq!(k(90.0), vec![Act::Down]);
        assert_eq!(k(135.0), vec![Act::Left, Act::Down]);
        assert_eq!(k(180.0), vec![Act::Left]);
        assert_eq!(k(225.0), vec![Act::Left, Act::Up]);
        assert_eq!(k(270.0), vec![Act::Up]);
        assert_eq!(k(315.0), vec![Act::Right, Act::Up]);
        assert_eq!(k(22.0), vec![Act::Right]);
        assert_eq!(k(23.0), vec![Act::Right, Act::Down]);
        assert_eq!(k(359.0), vec![Act::Right]);
        assert!(dpad_acts(5.0, 5.0, 10.0).is_empty());
    }

    #[test]
    fn dpad_slides_between_directions() {
        let mut m = map();
        let (x, y) = at(&m, Role::Dpad);
        assert_eq!(m.down(1, x + 100.0, y, true, false).keys, vec![Down(RIGHT)]);
        assert!(m.moved(1, x + 110.0, y).is_empty(), "同一扇區不重送");
        // 滑到右上：多按一個上。
        assert_eq!(m.moved(1, x + 100.0, y - 100.0), vec![Down(UP)]);
        // 滑到左上：先放開右再按左。
        assert_eq!(m.moved(1, x - 100.0, y - 100.0), vec![Up(RIGHT), Down(LEFT)]);
        // 滑回中心：全放開。
        let ev = m.moved(1, x, y);
        assert_eq!(ev.len(), 2);
        assert!(ev.contains(&Up(LEFT)) && ev.contains(&Up(UP)));
        // 手指滑出判定範圍仍然有效。
        assert_eq!(m.moved(1, x + 1000.0, y), vec![Down(RIGHT)]);
        assert_eq!(m.up(1), vec![Up(RIGHT)]);
        assert!(m.up(1).is_empty());
    }

    #[test]
    fn buttons_follow_the_game_state() {
        let mut m = map();
        let (x, y) = at(&m, Role::Primary);
        assert_eq!(m.down(1, x, y, true, false).keys, vec![Down(FIRE)]);
        assert_eq!(m.up(1), vec![Up(FIRE)]);
        assert_eq!(m.down(1, x, y, false, false).keys, vec![Down(CODE_CONFIRM)]);
        assert_eq!(m.up(1), vec![Up(CODE_CONFIRM)]);
        let (x, y) = at(&m, Role::Secondary);
        assert_eq!(m.down(2, x, y, true, false).keys, vec![Down(TURN)]);
        assert_eq!(m.up(2), vec![Up(TURN)]);
        assert_eq!(m.down(2, x, y, false, false).keys, vec![Down(CODE_MENU)]);
        assert_eq!(m.up(2), vec![Up(CODE_MENU)]);
    }

    #[test]
    fn pause_button_is_a_front_end_pause_and_sends_no_key() {
        let mut m = map();
        let (x, y) = at(&m, Role::Pause);
        let t = m.down(3, x, y, true, false);
        assert!(t.toggle_pause && t.keys.is_empty() && !t.toggle_toolbar, "不送 Esc 給遊戲");
        assert!(m.up(3).is_empty());
    }

    #[test]
    fn release_uses_the_code_from_the_press() {
        let mut m = map();
        let (x, y) = at(&m, Role::Primary);
        assert_eq!(m.down(1, x, y, false, false).keys, vec![Down(CODE_CONFIRM)]);
        // 按著的時候進入遊玩，放開的還是當初按的碼。
        assert_eq!(m.up(1), vec![Up(CODE_CONFIRM)]);
    }

    #[test]
    fn two_fingers_work_at_once() {
        let mut m = map();
        let (dx, dy) = at(&m, Role::Dpad);
        let (fx, fy) = at(&m, Role::Primary);
        assert_eq!(m.down(1, dx + 100.0, dy, true, false).keys, vec![Down(RIGHT)]);
        assert_eq!(m.down(2, fx, fy, true, false).keys, vec![Down(FIRE)]);
        assert_eq!(m.moved(1, dx, dy + 100.0), vec![Up(RIGHT), Down(DOWN)]);
        assert_eq!(m.up(2), vec![Up(FIRE)]);
        assert_eq!(m.up(1), vec![Up(DOWN)]);
    }

    #[test]
    fn menu_button_and_toolbar_area_send_no_keys() {
        let mut m = map();
        let (x, y) = at(&m, Role::Menu);
        let t = m.down(1, x, y, true, false);
        assert!(t.toggle_toolbar && t.keys.is_empty());
        assert!(m.up(1).is_empty());
        let (x, y) = at(&m, Role::Pause);
        let t = m.down(2, x, y, true, true);
        assert!(!t.toggle_toolbar && !t.toggle_pause && t.keys.is_empty(), "工具列蓋著的地方交給工具列");
        // 沒有控制的地方。
        let t = m.down(3, W as f32 / 2.0, H as f32 / 2.0, true, false);
        assert!(!t.toggle_toolbar && !t.toggle_pause && t.keys.is_empty());
        assert!(m.moved(3, 0.0, 0.0).is_empty());
    }

    #[test]
    fn releasing_everything_clears_the_keys() {
        let mut m = map();
        let (dx, dy) = at(&m, Role::Dpad);
        m.down(1, dx + 100.0, dy, true, false);
        let (fx, fy) = at(&m, Role::Primary);
        m.down(2, fx, fy, true, false);
        let ev = m.release_all();
        assert_eq!(ev.len(), 2);
        assert!(ev.contains(&Up(RIGHT)) && ev.contains(&Up(FIRE)));
        assert!(m.release_all().is_empty());
        // 放開之後同樣的手指編號可以再用。
        assert_eq!(m.down(1, dx + 100.0, dy, true, false).keys, vec![Down(RIGHT)]);
    }

    #[test]
    fn overlay_is_transparent_outside_the_controls() {
        let (w, h) = (960usize, 432usize);
        let l = layout(w as u32, h as u32, 1.0);
        let a = render_overlay(&l, true, w, h);
        assert_eq!(a.len(), w * h * 4);
        // 畫面中央全透明，控制鈕的位置不透明。
        assert_eq!(a[(h / 2 * w + w / 2) * 4 + 3], 0);
        let c = l.iter().find(|c| c.role == Role::Primary).unwrap();
        let o = (c.y as usize * w + (c.x as usize - 30)) * 4;
        assert!(a[o + 3] > 0);
        // 遊戲狀態改變，文字跟著變。
        assert_ne!(a, render_overlay(&l, false, w, h));
    }

    #[test]
    fn overlay_labels_are_chinese_and_stay_inside_their_circles() {
        let f = font::builtin();
        for (role, playable) in [(Role::Primary, true), (Role::Primary, false), (Role::Secondary, true), (Role::Secondary, false)] {
            let t = label(role, playable);
            assert!(!t.is_empty() && f.missing(t).is_empty(), "{t}");
        }
        for (w, h, dp) in [(2400u32, 1080u32, 2.5f32), (1280, 720, 2.0), (960, 432, 1.0)] {
            for c in layout(w, h, dp) {
                for playable in [false, true] {
                    let t = label(c.role, playable);
                    if t.is_empty() {
                        continue;
                    }
                    let fh = ((c.r * 0.55) as i32).max(12);
                    let half_diag = (((f.text_width(t, fh) as f32) / 2.0).powi(2) + (fh as f32 / 2.0).powi(2)).sqrt();
                    assert!(half_diag <= c.r, "{t} 放不進半徑 {} 的圓（{w}x{h}）：{half_diag}", c.r);
                }
            }
        }
    }
}
