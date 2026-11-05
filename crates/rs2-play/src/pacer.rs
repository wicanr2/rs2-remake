//! 依 `Frame.ticks` 排程：每一步都說到下一步要等幾個 18.2065 Hz 的 tick。
//! 累積時間、每次最多補的步數、補間用的 alpha 都在這裡。
//! 時間一律以計數器值傳入，不直接讀時鐘，好寫單元測試。

/// 原版的一個 INT 8 tick。
pub const TICK_SECS: f64 = 1.0 / 18.2065;
/// 一次更新最多補的步數。卡頓之後不連跑。
pub const MAX_STEPS_PER_ADVANCE: u32 = 2;

#[derive(Clone, Debug)]
pub struct Pacer {
    /// 一個 tick 等於多少計數值。
    tick: f64,
    last: u64,
    /// 距離上一步的累積計數。
    acc: u64,
    /// 這一步要等多少計數。第一步不等。
    cur: u64,
    /// 這次更新已經跑了幾步。
    taken: u32,
}

impl Pacer {
    /// `freq` 是計數器頻率（每秒幾個計數），`now` 是目前的計數值。
    pub fn new(freq: u64, now: u64) -> Pacer {
        Pacer { tick: freq as f64 * TICK_SECS, last: now, acc: 0, cur: 0, taken: 0 }
    }

    /// 記下經過的時間，開始新的一次更新。
    pub fn update(&mut self, now: u64) {
        let dt = now.saturating_sub(self.last);
        self.last = self.last.max(now);
        self.acc = self.acc.saturating_add(dt);
        self.taken = 0;
    }

    /// 該跑下一步就回傳 `true`。呼叫端跑完要用 `set_next` 告訴這一步之後要等多久。
    /// 落後超過上限時丟掉積壓，從這裡重新計時。
    pub fn take_step(&mut self) -> bool {
        if self.acc < self.cur {
            return false;
        }
        if self.taken >= MAX_STEPS_PER_ADVANCE {
            self.acc = 0;
            return false;
        }
        self.acc -= self.cur;
        self.taken += 1;
        true
    }

    /// 剛跑完的一步回報的 `Frame.ticks`。
    pub fn set_next(&mut self, ticks: u16) {
        self.cur = (self.tick * ticks as f64).round() as u64;
    }

    /// 上一步到現在的進度，夾在 0 到 1。不用等的步直接算 1。
    pub fn alpha(&self) -> f32 {
        if self.cur == 0 {
            return 1.0;
        }
        (self.acc as f64 / self.cur as f64).clamp(0.0, 1.0) as f32
    }

    /// 距離下一步還要多久（計數值），給主迴圈決定睡多久。
    pub fn until_next(&self) -> u64 {
        self.cur.saturating_sub(self.acc)
    }

    /// 視窗重新取得焦點時重設計時，免得一次補好幾步。
    pub fn reset(&mut self, now: u64) {
        self.last = now;
        self.acc = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: u64 = 1_000_000_000; // 1 GHz，1 計數 = 1 ns

    /// 一次更新跑得出幾步，每步之後要等 `ticks`。
    fn steps(p: &mut Pacer, now: u64, ticks: u16) -> u32 {
        p.update(now);
        let mut n = 0;
        while p.take_step() {
            p.set_next(ticks);
            n += 1;
        }
        n
    }

    #[test]
    fn first_step_runs_immediately() {
        let mut p = Pacer::new(F, 1000);
        assert_eq!(steps(&mut p, 1000, 2), 1);
    }

    #[test]
    fn two_ticks_is_about_109_85_ms() {
        let mut p = Pacer::new(F, 0);
        p.set_next(2);
        assert!((p.cur as f64 / 1e6 - 109.85).abs() < 0.01, "{}", p.cur);
    }

    #[test]
    fn wait_follows_the_ticks_of_the_last_step() {
        let mut p = Pacer::new(F, 0);
        assert_eq!(steps(&mut p, 0, 1), 1);
        let one = p.cur;
        assert_eq!(steps(&mut p, one - 1, 5), 0);
        assert_eq!(steps(&mut p, one, 5), 1);
        let five = p.cur;
        assert!(five.abs_diff(5 * one) <= 2);
        assert_eq!(steps(&mut p, one + five - 1, 1), 0);
        assert_eq!(steps(&mut p, one + five, 1), 1);
    }

    #[test]
    fn zero_ticks_does_not_spin() {
        let mut p = Pacer::new(F, 0);
        assert_eq!(steps(&mut p, 0, 0), MAX_STEPS_PER_ADVANCE, "不用等的步一次最多跑上限");
        assert_eq!(p.alpha(), 1.0);
        assert_eq!(steps(&mut p, 1, 2), 1);
    }

    #[test]
    fn alpha_is_progress_clamped_to_unit_interval() {
        let mut p = Pacer::new(F, 0);
        steps(&mut p, 0, 2);
        let s = p.cur;
        assert_eq!(p.alpha(), 0.0);
        p.update(s / 2);
        assert!((p.alpha() - 0.5).abs() < 1e-3);
        p.update(s + s / 4);
        assert_eq!(p.alpha(), 1.0, "還沒跑下一步就停在 1");
        assert!(p.take_step());
        assert!((p.alpha() - 0.25).abs() < 1e-3, "{}", p.alpha());
    }

    #[test]
    fn at_most_two_catch_up_steps_and_the_backlog_is_dropped() {
        let mut p = Pacer::new(F, 0);
        steps(&mut p, 0, 2);
        let s = p.cur;
        assert_eq!(steps(&mut p, 10 * s, 2), 2, "卡頓 10 步也只補 2 步");
        assert_eq!(p.alpha(), 0.0, "積壓丟掉");
        assert_eq!(steps(&mut p, 10 * s + s / 2, 2), 0);
        assert_eq!(steps(&mut p, 10 * s + s, 2), 1);
    }

    #[test]
    fn reset_drops_elapsed_time() {
        let mut p = Pacer::new(F, 0);
        steps(&mut p, 0, 2);
        let s = p.cur;
        p.update(s - 1);
        p.reset(100 * s);
        assert_eq!(steps(&mut p, 100 * s, 2), 0);
    }

    #[test]
    fn until_next_counts_down() {
        let mut p = Pacer::new(F, 0);
        steps(&mut p, 0, 2);
        let s = p.cur;
        assert_eq!(p.until_next(), s);
        p.update(s / 4);
        assert_eq!(p.until_next(), s - s / 4);
    }
}
