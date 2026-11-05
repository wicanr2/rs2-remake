//! 固定步長的計時：累積器、每次最多補的步數、補間用的 alpha。
//! 時間一律以計數器 tick 傳入，不直接讀時鐘，好寫單元測試。

/// 一個模擬步 = 原版主迴圈的兩次 INT 8 = 2 / 18.2065 秒（約 109.85 毫秒）。
pub const STEP_SECS: f64 = 2.0 / 18.2065;
/// 一次 `advance` 最多補的步數。卡頓之後不連跑。
pub const MAX_STEPS_PER_ADVANCE: u32 = 2;

#[derive(Clone, Debug)]
pub struct Pacer {
    step_ticks: u64,
    last: u64,
    /// 距離上一步的累積 tick。
    acc: u64,
}

impl Pacer {
    /// `freq` 是計數器頻率（每秒 tick 數），`now` 是目前的計數值。
    pub fn new(freq: u64, now: u64) -> Pacer {
        let step_ticks = ((freq as f64) * STEP_SECS).round().max(1.0) as u64;
        Pacer { step_ticks, last: now, acc: 0 }
    }

    #[cfg(test)]
    pub fn step_ticks(&self) -> u64 {
        self.step_ticks
    }

    /// 回傳這次要跑幾步（0 到 `MAX_STEPS_PER_ADVANCE`）。積壓超過上限的部分丟掉。
    pub fn advance(&mut self, now: u64) -> u32 {
        let dt = now.saturating_sub(self.last);
        self.last = self.last.max(now);
        self.acc = self.acc.saturating_add(dt);
        let mut n = 0;
        while self.acc >= self.step_ticks && n < MAX_STEPS_PER_ADVANCE {
            self.acc -= self.step_ticks;
            n += 1;
        }
        if self.acc >= self.step_ticks {
            // 落後超過上限：丟掉積壓，從這一步重新計時。
            self.acc = 0;
        }
        n
    }

    /// 上一步到現在的進度，夾在 0 到 1。
    pub fn alpha(&self) -> f32 {
        (self.acc as f64 / self.step_ticks as f64).clamp(0.0, 1.0) as f32
    }

    /// 距離下一步還要多久（tick），給主迴圈決定睡多久。
    pub fn until_next(&self) -> u64 {
        self.step_ticks.saturating_sub(self.acc)
    }

    /// 暫停結束或視窗重新取得焦點時重設計時，免得一次補好幾步。
    pub fn reset(&mut self, now: u64) {
        self.last = now;
        self.acc = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: u64 = 1_000_000_000; // 1 GHz，1 tick = 1 ns

    #[test]
    fn step_is_about_109_85_ms() {
        let p = Pacer::new(F, 0);
        let ms = p.step_ticks() as f64 / 1e6;
        assert!((ms - 109.85).abs() < 0.01, "{ms}");
        assert!((STEP_SECS - 0.109_850_5).abs() < 1e-6);
    }

    #[test]
    fn no_step_until_a_full_interval() {
        let mut p = Pacer::new(F, 1000);
        let s = p.step_ticks();
        assert_eq!(p.advance(1000 + s - 1), 0);
        assert_eq!(p.advance(1000 + s), 1);
        assert_eq!(p.advance(1000 + s + 5), 0);
    }

    #[test]
    fn alpha_is_progress_clamped_to_unit_interval() {
        let mut p = Pacer::new(F, 0);
        let s = p.step_ticks();
        assert_eq!(p.alpha(), 0.0);
        p.advance(s / 2);
        assert!((p.alpha() - 0.5).abs() < 1e-3);
        p.advance(s + s / 4);
        assert!((p.alpha() - 0.25).abs() < 1e-3, "{}", p.alpha());
        for t in (0..=40u64).map(|i| i * s / 7) {
            p.advance(t.max(s * 2));
            let a = p.alpha();
            assert!((0.0..=1.0).contains(&a));
        }
    }

    #[test]
    fn at_most_two_catch_up_steps_and_the_backlog_is_dropped() {
        let mut p = Pacer::new(F, 0);
        let s = p.step_ticks();
        assert_eq!(p.advance(10 * s), 2, "卡頓 10 步也只補 2 步");
        assert_eq!(p.alpha(), 0.0, "積壓丟掉");
        assert_eq!(p.advance(10 * s + s / 2), 0);
        assert_eq!(p.advance(10 * s + s), 1);
    }

    #[test]
    fn exact_two_steps_keep_the_remainder() {
        let mut p = Pacer::new(F, 0);
        let s = p.step_ticks();
        assert_eq!(p.advance(2 * s + s / 4), 2);
        assert!((p.alpha() - 0.25).abs() < 1e-3);
    }

    #[test]
    fn long_run_average_rate_matches_the_step_length() {
        let mut p = Pacer::new(F, 0);
        let mut steps = 0u64;
        let mut now = 0u64;
        // 16.6 ms 的顯示間隔（60 Hz）跑 60 秒。
        while now < 60 * F {
            now += 16_666_667;
            steps += p.advance(now) as u64;
        }
        let want = 60.0 / STEP_SECS; // 約 546.2
        assert!((steps as f64 - want).abs() <= 1.0, "{steps} vs {want}");
    }

    #[test]
    fn clock_going_backwards_does_not_underflow() {
        let mut p = Pacer::new(F, 1_000_000);
        assert_eq!(p.advance(10), 0);
        assert!(p.alpha() >= 0.0);
    }

    #[test]
    fn reset_clears_progress() {
        let mut p = Pacer::new(F, 0);
        let s = p.step_ticks();
        p.advance(s / 2);
        p.reset(5 * s);
        assert_eq!(p.alpha(), 0.0);
        assert_eq!(p.advance(5 * s + s / 2), 0);
    }
}
