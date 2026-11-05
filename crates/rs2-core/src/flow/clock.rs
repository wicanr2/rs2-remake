//! 計時中斷（INT 8，18.2 Hz）與等待。對照 `1A7B5h` 的處理函式與 `sub_11097`。
//!
//! 模擬不花時間，所以「等 n 個 tick」是把 tick 一個一個過完：`await_timer_ticks` 讓中斷處理函式跑到計數夠，
//! 再把控制權交回外面，外面依實際時間決定什麼時候恢復。

use crate::game::{Core, Req};

/// 兩個只有密碼畫面讀的循環計數，週期。
const CYCLE_A_PERIOD: u16 = 0x19;
const CYCLE_B_PERIOD: u16 = 0x1E;

/// 計時中斷累計的計數。五個字組都只有中斷處理函式加，讀的人如各欄位說明。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Clock {
    /// `word_273B7`：開機以來的 tick 數，只有中斷加，沒有人讀。
    pub total: u16,
    /// `word_273B9`：自上次等待結束以來的 tick 數。`await_timer_ticks` 讀它，等待結束時歸零。
    pub since_wait: u16,
    /// `word_2739B`：自上次切換音效以來的 tick 數。音效開關鍵要累積 5 個 tick 才生效。
    pub since_sound_toggle: u16,
    /// `word_1D3A8`：0 到 18h 循環，只有密碼畫面讀。
    pub cycle_a: u16,
    /// `word_1D3AE`：0 到 1Dh 循環，只有密碼畫面讀。
    pub cycle_b: u16,
}

impl Clock {
    /// 一個 tick 的效果（`1A7CEh` 至 `1A7D6h`、`sub_1A819`、`sub_1A82B`）。
    ///
    /// 處理函式在 `byte_273AD` 不是 `Y` 時還有一段自我檢查（`1A7E1h` 至 `1A7FBh`），映像裡它是 `Y`，所以略過。
    /// 搖桿與滑鼠的輪詢（`sub_14D8C`）重製不支援。
    pub(crate) fn tick(&mut self) {
        self.total = self.total.wrapping_add(1);
        self.since_wait = self.since_wait.wrapping_add(1);
        self.since_sound_toggle = self.since_sound_toggle.wrapping_add(1);
        self.cycle_a = Self::advance(self.cycle_a, CYCLE_A_PERIOD);
        self.cycle_b = Self::advance(self.cycle_b, CYCLE_B_PERIOD);
    }

    /// 加 1，到週期歸零（`cmp` 之後 `jb`，所以大於等於週期就歸零，不是取餘數）。
    fn advance(count: u16, period: u16) -> u16 {
        let next = count.wrapping_add(1);
        if next >= period { 0 } else { next }
    }
}

impl Core {
    /// 等 `ticks` 個 tick（原版 `sub_11097`，個數放在 `cx`）。進來時計數是上次等待結束後累積的值，
    /// 已經夠了就不再過 tick。INT 9 落在這段等待裡，協程恢復時才處理。
    pub(crate) async fn await_timer_ticks(&mut self, ticks: u16) {
        let mut passed = 0;
        while self.flow.clock.since_wait < ticks {
            self.flow.clock.tick();
            passed += 1;
        }
        self.flow.clock.since_wait = 0;
        self.suspend(Req::Wait { ticks: passed }).await;
        self.suspend(Req::WaitDone { cx: ticks }).await;
    }

    /// 原版的忙等迴圈每轉一圈呼叫一次：過一個 tick，交回控制權等按鍵。
    pub(crate) async fn poll_tick(&mut self) {
        self.flow.clock.tick();
        self.suspend(Req::Poll { paused: false }).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cycles_wrap_at_their_period_and_the_rest_wrap_at_16_bits() {
        let mut c = Clock { total: 0xFFFF, since_wait: 0xFFFF, since_sound_toggle: 0xFFFF, cycle_a: CYCLE_A_PERIOD - 2, cycle_b: CYCLE_B_PERIOD - 1 };
        c.tick();
        assert_eq!((c.total, c.since_wait, c.since_sound_toggle), (0, 0, 0));
        assert_eq!((c.cycle_a, c.cycle_b), (CYCLE_A_PERIOD - 1, 0), "cycle_b 到 1Dh 的下一個 tick 回 0");
        c.tick();
        assert_eq!(c.cycle_a, 0);
    }

    #[test]
    fn a_cycle_past_its_period_goes_to_zero_instead_of_counting_on() {
        // `cmp ax, period; jb` 的寫法：超過週期的值（例如 poke）下一個 tick 也歸零。
        assert_eq!(Clock::advance(0x50, CYCLE_A_PERIOD), 0);
        assert_eq!(Clock::advance(CYCLE_A_PERIOD - 2, CYCLE_A_PERIOD), CYCLE_A_PERIOD - 1);
    }
}
