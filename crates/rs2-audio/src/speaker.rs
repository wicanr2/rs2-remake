//! PC 喇叭。遊戲的聲音全來自 `sub_192EA`：畫特效圖時順手改 PIT 通道 2 的除數與喇叭閘門。
//!
//! 原版的時間軸是 CPU 跑那段迴圈的速度，靜態無法決定。事件之間的指令數照機器碼算（`rs2-gfx` 的 `speaker`），
//! 換算成時間用 `CLOCKS_PER_STEP`，對應約 4 MIPS 的機器。這是 remake 的選擇，見規格 003「已知差異」。

use std::collections::VecDeque;

/// 一道指令幾個 PIT 週期：`3/10` 約等於 4 MIPS。
const STEP_NUM: u64 = 3;
const STEP_DEN: u64 = 10;
/// 喇叭輸出的振幅。
const AMPLITUDE: i64 = 5000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpkKind {
    /// `sub_187A6`：PIT 通道 2 模式 3，除數 1 到 65536。
    Divisor(u32),
    /// `sub_187B3`（開）、`sub_187BB`（關）：`port 61h` 的位元 0 與 1。
    Gate(bool),
}

/// 一個事件，`steps` 是距離上一個事件的指令數。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpkEv {
    pub steps: u32,
    pub kind: SpkKind,
}

#[derive(Clone, Debug)]
pub struct Speaker {
    /// 以 PIT 週期計的現在時間。
    clock: u64,
    /// 最後一個排入事件的時間。
    tail: u64,
    queue: VecDeque<(u64, SpkKind)>,
    divisor: u32,
    gate: bool,
    /// 通道 2 的輸出。閘門打開時模式 3 從高電位開始。
    high: bool,
    /// 距離下一次翻轉的週期數。
    left: u32,
    /// 高通濾波的狀態，避免喇叭開關造成直流偏移。
    dc: i64,
}

impl Default for Speaker {
    fn default() -> Self {
        Self::new()
    }
}

impl Speaker {
    pub fn new() -> Speaker {
        Speaker { clock: 0, tail: 0, queue: VecDeque::new(), divisor: 65536, gate: false, high: false, left: 0, dc: 0 }
    }

    /// 排入一段事件。
    pub fn push(&mut self, ev: &[SpkEv]) {
        let mut t = self.tail.max(self.clock);
        for e in ev {
            t += e.steps as u64 * STEP_NUM / STEP_DEN;
            self.queue.push_back((t, e.kind));
        }
        self.tail = t;
    }

    /// 模式 3 半週期。除數寫入後等下一次翻轉才生效，所以這裡只改 `divisor`。
    fn half(&self, high: bool) -> u32 {
        // 奇數除數的高半週多一個週期。
        (self.divisor + if high { 1 } else { 0 }) / 2
    }

    /// 推進 `clocks` 個 PIT 週期，回傳這段時間的平均輸出。
    pub fn sample(&mut self, clocks: u32) -> i32 {
        let end = self.clock + clocks as u64;
        let mut on = 0u64;
        while self.clock < end {
            while let Some(&(t, k)) = self.queue.front() {
                if t > self.clock {
                    break;
                }
                self.queue.pop_front();
                match k {
                    SpkKind::Divisor(d) => self.divisor = d.max(2),
                    SpkKind::Gate(g) => {
                        if g && !self.gate {
                            self.high = true;
                            self.left = self.half(true);
                        }
                        self.gate = g;
                    }
                }
            }
            let next_ev = self.queue.front().map_or(end, |e| e.0.min(end));
            let span = if self.gate { (next_ev - self.clock).min(self.left.max(1) as u64) } else { next_ev - self.clock };
            if self.gate && self.high {
                on += span;
            }
            self.clock += span;
            if self.gate {
                self.left -= span as u32;
                if self.left == 0 {
                    self.high = !self.high;
                    self.left = self.half(self.high).max(1);
                }
            }
        }
        // 一階高通，時間常數約 20 毫秒。
        let x = on as i64 * AMPLITUDE / clocks as i64;
        self.dc += (x - self.dc) / 1000;
        (x - self.dc) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_without_events() {
        let mut s = Speaker::new();
        assert!((0..1000).all(|_| s.sample(24) == 0));
    }

    #[test]
    fn gate_produces_a_square_wave_at_the_divisor() {
        let mut s = Speaker::new();
        // 除數 1193：1000 Hz，取樣率 49716 Hz，週期約 49.7 個取樣。
        s.push(&[SpkEv { steps: 0, kind: SpkKind::Divisor(1193) }, SpkEv { steps: 0, kind: SpkKind::Gate(true) }]);
        let v: Vec<i32> = (0..49716).map(|_| s.sample(24)).collect();
        let ups = v.windows(2).filter(|w| w[0] < 1000 && w[1] >= 1000).count();
        assert!((990..=1010).contains(&ups), "{ups}");
    }

    #[test]
    fn gate_off_returns_to_silence() {
        let mut s = Speaker::new();
        s.push(&[
            SpkEv { steps: 0, kind: SpkKind::Divisor(1193) },
            SpkEv { steps: 0, kind: SpkKind::Gate(true) },
            SpkEv { steps: 1000, kind: SpkKind::Gate(false) },
        ]);
        for _ in 0..5000 {
            s.sample(24);
        }
        assert!((0..100).all(|_| s.sample(24).abs() < 100));
    }

    #[test]
    fn bursts_are_queued_one_after_another() {
        let mut s = Speaker::new();
        let e = [SpkEv { steps: 100, kind: SpkKind::Gate(true) }, SpkEv { steps: 100, kind: SpkKind::Gate(false) }];
        s.push(&e);
        s.push(&e);
        assert_eq!(s.queue.len(), 4);
        assert_eq!(s.queue[3].0, 400 * STEP_NUM / STEP_DEN);
    }
}
