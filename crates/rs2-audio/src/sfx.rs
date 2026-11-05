//! 打擊回饋的音效層（規格 005「打擊回饋」）：在原版的音樂與喇叭之上疊一層短促的音效，雜訊包絡加低頻。
//!
//! 這是 remake 加的一層，不是原版的聲音，所以獨立於 `Engine` 的驅動、OPL2 與喇叭。沒有人觸發就一個取樣都不加，
//! 音樂與喇叭的輸出和沒有這一層時逐取樣相同（規格 003 C4 不受影響）。音量低於音樂：峰值在 16 位元範圍的 1/10 左右。
//! 取樣用固定的線性同餘亂數，同樣的觸發順序得到同樣的波形。

use crate::opl2::NATIVE_RATE;

/// 音效種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SfxKind {
    /// 玩家開火：短促的啪聲加一點低頻。
    Fire,
    /// 爆炸：低沉的轟聲，尾巴較長。
    Boom,
}

/// 同時最多幾個聲部。超過就偷最舊的。
pub const VOICES: usize = 6;

/// 各種音效的峰值（16 位元取樣）。原版音樂的峰值在 6000 以上，這裡都低於它。
pub const PEAK_FIRE: f32 = 1700.0;
pub const PEAK_BOOM: f32 = 3000.0;

const FIRE_SECS: f32 = 0.09;
const BOOM_SECS: f32 = 0.60;

#[derive(Clone, Copy, Debug)]
struct Voice {
    kind: SfxKind,
    age: u32,
    phase: f32,
    lp: f32,
    rng: u32,
}

/// 音效混音器。
#[derive(Clone, Debug)]
pub struct Sfx {
    voices: [Option<Voice>; VOICES],
    triggers: u32,
}

impl Default for Sfx {
    fn default() -> Self {
        Self::new()
    }
}

fn dur_samples(k: SfxKind) -> u32 {
    let s = match k {
        SfxKind::Fire => FIRE_SECS,
        SfxKind::Boom => BOOM_SECS,
    };
    (s * NATIVE_RATE as f32) as u32
}

impl Sfx {
    pub fn new() -> Sfx {
        Sfx { voices: [None; VOICES], triggers: 0 }
    }

    /// 開始一個音效。聲部滿了就取代最舊的。
    pub fn trigger(&mut self, kind: SfxKind) {
        self.triggers = self.triggers.wrapping_add(1);
        let v = Voice { kind, age: 0, phase: 0.0, lp: 0.0, rng: 0x9E37_79B9u32.wrapping_mul(self.triggers) | 1 };
        if let Some(slot) = self.voices.iter_mut().find(|s| s.is_none()) {
            *slot = Some(v);
            return;
        }
        let oldest = (0..VOICES).max_by_key(|&i| self.voices[i].map(|v| v.age).unwrap_or(0)).unwrap_or(0);
        self.voices[oldest] = Some(v);
    }

    /// 使用中的聲部數。
    pub fn active(&self) -> usize {
        self.voices.iter().flatten().count()
    }

    /// 產生一個取樣（要加到音樂與喇叭的輸出上）。沒有聲部就是 0。
    #[inline]
    pub fn sample(&mut self) -> i32 {
        if self.triggers == 0 {
            return 0;
        }
        let mut sum = 0.0f32;
        for slot in self.voices.iter_mut() {
            let Some(v) = slot else { continue };
            if v.age >= dur_samples(v.kind) {
                *slot = None;
                continue;
            }
            sum += voice_sample(v);
            v.age += 1;
        }
        sum.round() as i32
    }
}

fn voice_sample(v: &mut Voice) -> f32 {
    let r = NATIVE_RATE as f32;
    let t = v.age as f32 / r;
    // 白雜訊：LCG 取高位元。
    v.rng = v.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    let white = (v.rng >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
    // 起音 2 毫秒，避免爆音。
    let attack = (t / 0.002).min(1.0);
    match v.kind {
        SfxKind::Fire => {
            v.lp += 0.45 * (white - v.lp);
            let f = 170.0 - 90.0 * (t / FIRE_SECS).min(1.0);
            v.phase = (v.phase + f / r).fract();
            let noise = v.lp * (-t / 0.012).exp() * 0.55;
            let low = (v.phase * std::f32::consts::TAU).sin() * (-t / 0.028).exp() * 0.60;
            (noise + low) * attack * PEAK_FIRE / 1.0
        }
        SfxKind::Boom => {
            v.lp += 0.10 * (white - v.lp);
            let f = 75.0 - 40.0 * (t / 0.4).min(1.0);
            v.phase = (v.phase + f / r).fract();
            // 低通過的雜訊振幅小，補一個增益。
            let noise = v.lp * 3.0 * (-t / 0.12).exp() * 0.55;
            let low = (v.phase * std::f32::consts::TAU).sin() * (-t / 0.20).exp() * 0.70;
            (noise + low) * attack * PEAK_BOOM / 1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(kind: SfxKind, secs: f32) -> Vec<i32> {
        let mut s = Sfx::new();
        s.trigger(kind);
        (0..(secs * NATIVE_RATE as f32) as usize).map(|_| s.sample()).collect()
    }

    #[test]
    fn silent_until_triggered_and_adds_nothing() {
        let mut s = Sfx::new();
        assert!((0..2000).all(|_| s.sample() == 0));
        assert_eq!(s.active(), 0);
    }

    #[test]
    fn voices_end_by_themselves_and_free_their_slot() {
        let mut s = Sfx::new();
        s.trigger(SfxKind::Fire);
        s.trigger(SfxKind::Boom);
        assert_eq!(s.active(), 2);
        for _ in 0..(0.1 * NATIVE_RATE as f32) as usize {
            s.sample();
        }
        assert_eq!(s.active(), 1, "開火 0.09 秒就結束，爆炸還在");
        for _ in 0..(0.6 * NATIVE_RATE as f32) as usize {
            s.sample();
        }
        assert_eq!(s.active(), 0);
        assert!((0..100).all(|_| s.sample() == 0), "結束後回到無聲");
    }

    #[test]
    fn peaks_are_audible_but_stay_below_the_music() {
        let fire = render(SfxKind::Fire, 0.2);
        let boom = render(SfxKind::Boom, 0.8);
        let peak = |v: &[i32]| v.iter().map(|x| x.abs()).max().unwrap();
        assert!(peak(&fire) > 500 && peak(&fire) <= (PEAK_FIRE * 1.3) as i32, "{}", peak(&fire));
        assert!(peak(&boom) > 1000 && peak(&boom) <= (PEAK_BOOM * 1.3) as i32, "{}", peak(&boom));
        assert!(peak(&boom) < 6000 && peak(&fire) < peak(&boom), "音效低於音樂（峰值 6000 以上）");
    }

    #[test]
    fn same_trigger_order_gives_the_same_waveform() {
        assert_eq!(render(SfxKind::Boom, 0.3), render(SfxKind::Boom, 0.3));
        let mut a = Sfx::new();
        let mut b = Sfx::new();
        for s in [&mut a, &mut b] {
            s.trigger(SfxKind::Fire);
            s.trigger(SfxKind::Boom);
        }
        assert!((0..5000).all(|_| a.sample() == b.sample()));
    }

    #[test]
    fn boom_is_lower_and_longer_than_fire() {
        let zc = |v: &[i32]| v.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count() as f32 / v.len() as f32;
        let fire = render(SfxKind::Fire, 0.05);
        let boom = render(SfxKind::Boom, 0.3);
        assert!(zc(&boom) < zc(&fire), "爆炸的零交越率低（較低沉）：{} {}", zc(&boom), zc(&fire));
        // 尾巴：爆炸在 0.3 秒時還有聲，開火在 0.1 秒後沒有。
        let tail = |kind: SfxKind, from: f32, to: f32| render(kind, to)[(from * NATIVE_RATE as f32) as usize..].iter().map(|x| x.abs()).max().unwrap_or(0);
        assert!(tail(SfxKind::Boom, 0.25, 0.3) > 100);
        assert_eq!(tail(SfxKind::Fire, 0.095, 0.12), 0);
    }

    #[test]
    fn full_voices_steal_the_oldest() {
        let mut s = Sfx::new();
        s.trigger(SfxKind::Boom);
        for _ in 0..1000 {
            s.sample();
        }
        for _ in 0..VOICES - 1 {
            s.trigger(SfxKind::Fire);
        }
        assert_eq!(s.active(), VOICES);
        s.trigger(SfxKind::Fire);
        assert_eq!(s.active(), VOICES, "不超過聲部數");
        // 被偷掉的是最舊的那個爆炸：0.1 秒後全部結束。
        for _ in 0..(0.1 * NATIVE_RATE as f32) as usize {
            s.sample();
        }
        assert_eq!(s.active(), 0);
    }
}
