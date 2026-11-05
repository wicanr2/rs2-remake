//! 玩家機的顯示層預測：按鍵後下一次繪製玩家機就動，模擬完全不受影響。
//!
//! 原版一秒只跑約 18 步，按鍵要等到下一步才生效，補間又比邏輯晚一步，所以從按鍵到畫面最多差兩步（約 110 毫秒）。
//! 這裡讓玩家機領先邏輯：`顯示位置 = 邏輯位置 + lead + err`。
//!
//! - `lead`：依目前按鍵累積的預測位移，速度是「一步的位移除以一步的時間」，所以下一步到來時剛好等於實際位移。
//! - `err`：邏輯步到來時，舊的顯示位置和新的邏輯位置的差。依時間常數衰減，所以猜錯（放開鍵、撞牆）也不會跳動。
//!
//! 預測只算偏移，不寫回 `Game`，所以每步的資料段雜湊不變。規格見 `docs/spec/004-rewrite-and-delivery.md`「體感」。

/// 水平一步的位移（邏輯像素）：`sub_146DA`、`sub_146F2` 每步 2 位元組（靜態已證實）。
pub const STEP_X: f32 = 16.0;
/// 垂直一步的位移（邏輯像素）：`sub_146AC`、`sub_146C3` 每步 0Ah 列（靜態已證實）。
pub const STEP_Y: f32 = 10.0;
/// `err` 衰減的時間常數。
pub const ERR_TAU_SECS: f32 = 0.040;
/// 一步的位移超過這個就是換關或重生，不補償。
const TELEPORT_PX: i32 = 48;
/// HD 像素對邏輯像素的倍率。偏移用 HD 像素當單位，才不會在原版圖的整數像素上損失精度。
const HD_SCALE: f32 = 4.0;

/// 下一步會生效的方向，已照原版的優先順序化簡：上先於下，左先於右。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dir {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

/// 玩家 `player`（0 或 1）的方向。`state` 是下一步時每個 set 1 碼是否按著。
///
/// 掃描碼來自 `sub_14501`（玩家 1）與 `sub_1502F`（玩家 2）讀的按鍵狀態表，表的位址是 `2748Bh + 掃描碼`：
/// 玩家 1 的斜向是 Home、PgUp、End、PgDn，玩家 2 的斜向是 R、Y、V、N。
pub fn dir_of(state: &[bool; 128], player: usize) -> Dir {
    let [ul, u, ur, dl, d, dr, l, r]: [u8; 8] = if player == 0 {
        [0x47, 0x48, 0x49, 0x4F, 0x50, 0x51, 0x4B, 0x4D]
    } else {
        [0x13, 0x14, 0x15, 0x2F, 0x30, 0x31, 0x21, 0x23]
    };
    let k = |c: u8| state[c as usize];
    let up = k(ul) || k(u) || k(ur);
    let down = !up && (k(dl) || k(d) || k(dr));
    let left = k(dl) || k(l) || k(ul);
    let right = !left && (k(dr) || k(r) || k(ur));
    Dir { up, down, left, right }
}

/// 玩家機還能往哪裡走（邏輯那邊的界限），和 `rs2_core::PlayerStatus` 同內容，這裡不依賴它好寫測試。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reach {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

impl Reach {
    pub const ALL: Reach = Reach { up: true, down: true, left: true, right: true };
}

#[derive(Clone, Copy, Debug, Default)]
struct Track {
    lead: (f32, f32),
    err: (f32, f32),
    /// 上一步的邏輯位置（邏輯像素）。`None` 表示這一步之前不能控制。
    last: Option<(i32, i32)>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Predictor {
    tracks: [Track; 2],
}

impl Predictor {
    pub fn new() -> Predictor {
        Predictor::default()
    }

    /// 邏輯步跑完。`pos` 是玩家機新的邏輯位置；不能控制（示範、死亡、過場）時傳 `None`，清掉預測。
    pub fn on_step(&mut self, player: usize, pos: Option<(i32, i32)>) {
        let t = &mut self.tracks[player];
        let Some(pos) = pos else {
            *t = Track::default();
            return;
        };
        let last = t.last.replace(pos);
        let Some(last) = last else { return };
        let (mx, my) = (pos.0 - last.0, pos.1 - last.1);
        if mx.abs().max(my.abs()) > TELEPORT_PX {
            t.lead = (0.0, 0.0);
            t.err = (0.0, 0.0);
            return;
        }
        // 舊的顯示位置 = 舊邏輯 + lead + err，新的顯示位置 = 新邏輯 + err'。兩者相等，所以畫面不跳。
        t.err = (t.err.0 + t.lead.0 - mx as f32, t.err.1 + t.lead.1 - my as f32);
        t.lead = (0.0, 0.0);
    }

    /// 經過 `dt` 秒。`step_secs` 是這一步到下一步的時間，0 表示不用等，這時不累積 `lead`。
    pub fn advance(&mut self, player: usize, dir: Dir, reach: Reach, dt: f32, step_secs: f32) {
        let t = &mut self.tracks[player];
        if t.last.is_none() {
            return;
        }
        if step_secs > 0.0 {
            let f = dt / step_secs;
            let vx = if dir.left && reach.left {
                -1.0
            } else if dir.right && reach.right {
                1.0
            } else {
                0.0
            };
            let vy = if dir.up && reach.up {
                -1.0
            } else if dir.down && reach.down {
                1.0
            } else {
                0.0
            };
            t.lead.0 = (t.lead.0 + vx * STEP_X * f).clamp(-STEP_X, STEP_X);
            t.lead.1 = (t.lead.1 + vy * STEP_Y * f).clamp(-STEP_Y, STEP_Y);
        }
        let k = (-dt / ERR_TAU_SECS).exp();
        t.err = (t.err.0 * k, t.err.1 * k);
    }

    /// 相對最新邏輯位置的顯示偏移，單位是 HD 像素（邏輯像素的 1/4）。`None` 表示這個玩家不預測，照一般補間畫。
    pub fn offset_hd(&self, player: usize) -> Option<(i32, i32)> {
        let t = &self.tracks[player];
        t.last?;
        let f = |lead: f32, err: f32| ((lead + err) * HD_SCALE).round() as i32;
        Some((f(t.lead.0, t.err.0), f(t.lead.1, t.err.1)))
    }

    /// 顯示偏移，單位是邏輯像素（測試用）。
    #[cfg(test)]
    fn offset_px(&self, player: usize) -> (f32, f32) {
        let t = &self.tracks[player];
        (t.lead.0 + t.err.0, t.lead.1 + t.err.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: f32 = 1.0 / 18.2065;
    const FRAME: f32 = 1.0 / 60.0;
    const RIGHT: Dir = Dir { up: false, down: false, left: false, right: true };

    fn started() -> Predictor {
        let mut p = Predictor::new();
        p.on_step(0, Some((100, 80)));
        p
    }

    #[test]
    fn key_press_moves_the_ship_on_the_next_draw() {
        let mut p = started();
        assert_eq!(p.offset_hd(0), Some((0, 0)));
        p.advance(0, RIGHT, Reach::ALL, FRAME, STEP);
        let (x, y) = p.offset_px(0);
        assert!(x > 0.0, "一次繪製之後就要動：{x}");
        assert!((x - 16.0 * FRAME / STEP).abs() < 1e-3);
        assert_eq!(y, 0.0);
    }

    #[test]
    fn held_key_tracks_the_logic_without_a_jump() {
        let mut p = started();
        let mut logic = 100;
        let mut last_shown = logic as f32;
        // 照 60 Hz 繪製，每 3 次繪製跑一步（比 18.2 Hz 稍快一點，偏移不會剛好抵銷）。
        for frame in 1..=120 {
            p.advance(0, RIGHT, Reach::ALL, FRAME, STEP);
            if frame % 3 == 0 {
                logic += 16;
                p.on_step(0, Some((logic, 80)));
            }
            let shown = logic as f32 + p.offset_px(0).0;
            let d = shown - last_shown;
            assert!(d > -1.0 && d < 16.0, "第 {frame} 次繪製跳了 {d}");
            last_shown = shown;
        }
        // 全程一步的距離以內。
        assert!(p.offset_px(0).0.abs() <= 16.0);
    }

    #[test]
    fn exact_timing_leaves_no_error() {
        let mut p = started();
        for _ in 0..10 {
            p.advance(0, RIGHT, Reach::ALL, STEP, STEP);
            assert!((p.offset_px(0).0 - 16.0).abs() < 1e-3);
            let t = p.tracks[0].last.unwrap();
            p.on_step(0, Some((t.0 + 16, t.1)));
            assert!(p.offset_px(0).0.abs() < 1e-3, "時間剛好時，下一步到來的誤差是 0");
        }
    }

    #[test]
    fn release_before_the_step_slides_back_smoothly() {
        let mut p = started();
        p.advance(0, RIGHT, Reach::ALL, STEP / 2.0, STEP);
        let lead = p.offset_px(0).0;
        assert!((lead - 8.0).abs() < 1e-3);
        // 放開：lead 停住，不往回跳。
        p.advance(0, Dir::default(), Reach::ALL, FRAME, STEP);
        assert_eq!(p.offset_px(0).0, lead);
        // 下一步沒有移動：lead 轉成 err，畫面仍在同一點，之後衰減回邏輯位置。
        p.on_step(0, Some((100, 80)));
        assert_eq!(p.offset_px(0).0, lead);
        let mut prev = lead;
        for _ in 0..30 {
            p.advance(0, Dir::default(), Reach::ALL, FRAME, STEP);
            let now = p.offset_px(0).0;
            assert!(now < prev && now >= 0.0);
            prev = now;
        }
        assert!(prev < 0.01, "半秒之後要回到邏輯位置：{prev}");
    }

    #[test]
    fn walls_stop_the_prediction() {
        let mut p = started();
        let blocked = Reach { right: false, ..Reach::ALL };
        p.advance(0, RIGHT, blocked, STEP, STEP);
        assert_eq!(p.offset_hd(0), Some((0, 0)), "撞牆的方向不預測");
    }

    #[test]
    fn lead_never_passes_one_step() {
        let mut p = started();
        // 卡頓：一次過了五步的時間。
        p.advance(0, RIGHT, Reach::ALL, STEP * 5.0, STEP);
        assert_eq!(p.offset_px(0).0, STEP_X);
        p.advance(0, Dir { up: true, ..Dir::default() }, Reach::ALL, STEP * 5.0, STEP);
        assert_eq!(p.offset_px(0).1, -STEP_Y);
    }

    #[test]
    fn teleport_and_loss_of_control_clear_everything() {
        let mut p = started();
        p.advance(0, RIGHT, Reach::ALL, STEP, STEP);
        p.on_step(0, Some((300, 80)));
        assert_eq!(p.offset_hd(0), Some((0, 0)), "瞬移不補償");
        p.advance(0, RIGHT, Reach::ALL, STEP, STEP);
        p.on_step(0, None);
        assert_eq!(p.offset_hd(0), None, "不能控制時回到一般補間");
        p.advance(0, RIGHT, Reach::ALL, STEP, STEP);
        assert_eq!(p.offset_hd(0), None);
        // 重新取得控制：從新位置開始，舊的預測不留。
        p.on_step(0, Some((40, 40)));
        assert_eq!(p.offset_hd(0), Some((0, 0)));
    }

    #[test]
    fn zero_step_time_accumulates_nothing() {
        let mut p = started();
        p.advance(0, RIGHT, Reach::ALL, FRAME, 0.0);
        assert_eq!(p.offset_hd(0), Some((0, 0)));
    }

    #[test]
    fn players_are_independent() {
        let mut p = started();
        p.on_step(1, Some((10, 10)));
        p.advance(1, RIGHT, Reach::ALL, STEP, STEP);
        assert_eq!(p.offset_hd(0), Some((0, 0)));
        assert_eq!(p.offset_hd(1), Some((64, 0)));
    }

    fn keys(codes: &[u8]) -> [bool; 128] {
        let mut s = [false; 128];
        for &c in codes {
            s[c as usize] = true;
        }
        s
    }

    #[test]
    fn directions_follow_the_original_priority() {
        let d = |c: &[u8], p| dir_of(&keys(c), p);
        // 玩家 1：單鍵與斜向。
        assert_eq!(d(&[0x48], 0), Dir { up: true, ..Dir::default() });
        assert_eq!(d(&[0x50], 0), Dir { down: true, ..Dir::default() });
        assert_eq!(d(&[0x4B], 0), Dir { left: true, ..Dir::default() });
        assert_eq!(d(&[0x4D], 0), RIGHT);
        assert_eq!(d(&[0x47], 0), Dir { up: true, left: true, ..Dir::default() });
        assert_eq!(d(&[0x49], 0), Dir { up: true, right: true, ..Dir::default() });
        assert_eq!(d(&[0x4F], 0), Dir { down: true, left: true, ..Dir::default() });
        assert_eq!(d(&[0x51], 0), Dir { down: true, right: true, ..Dir::default() });
        // 上先於下，左先於右（`sub_14501` 的判斷順序）。
        assert_eq!(d(&[0x48, 0x50], 0), Dir { up: true, ..Dir::default() });
        assert_eq!(d(&[0x4B, 0x4D], 0), Dir { left: true, ..Dir::default() });
        // 玩家 2 的鍵不影響玩家 1。
        assert_eq!(d(&[0x14], 0), Dir::default());
        // 玩家 2：T、B、F、H，斜向 R、Y、V、N。
        assert_eq!(d(&[0x14], 1), Dir { up: true, ..Dir::default() });
        assert_eq!(d(&[0x30], 1), Dir { down: true, ..Dir::default() });
        assert_eq!(d(&[0x21], 1), Dir { left: true, ..Dir::default() });
        assert_eq!(d(&[0x23], 1), RIGHT);
        assert_eq!(d(&[0x13], 1), Dir { up: true, left: true, ..Dir::default() });
        assert_eq!(d(&[0x15], 1), Dir { up: true, right: true, ..Dir::default() });
        assert_eq!(d(&[0x2F], 1), Dir { down: true, left: true, ..Dir::default() });
        assert_eq!(d(&[0x31], 1), Dir { down: true, right: true, ..Dir::default() });
    }
}
