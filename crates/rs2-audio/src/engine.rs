//! 把驅動、OPL2 與喇叭接成一條取樣流。一個取樣是 24 個 PIT 週期（OPL2 的 72 主時脈，時脈比 3:1），
//! 所以驅動 tick 落在哪個取樣是整數運算，不會累積誤差。

use crate::driver::{Driver, Mix};
use crate::opl2::Opl2;
use crate::speaker::{Speaker, SpkEv};

/// 一個晶片取樣的 PIT 週期數。
pub const PIT_PER_SAMPLE: u32 = 24;

/// 兩首曲子。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    A,
    B,
}

pub struct Engine {
    a: Mix,
    b: Mix,
    cur: Track,
    drv: Driver,
    opl: Opl2,
    acc: u32,
    spk: Speaker,
}

impl Engine {
    pub fn new(a_mix: &[u8], b_mix: &[u8]) -> Result<Engine, String> {
        Ok(Engine {
            a: Mix::parse(a_mix)?,
            b: Mix::parse(b_mix)?,
            cur: Track::A,
            drv: Driver::new(),
            opl: Opl2::new(),
            acc: 0,
            spk: Speaker::new(),
        })
    }

    /// `10AD4h`：`AX=0`、`AX=2`、`AX=4`。
    pub fn play(&mut self, track: Track, log: &mut dyn FnMut(u8, u8)) {
        self.cur = track;
        let opl = &mut self.opl;
        let mut w = |r: u8, v: u8| {
            opl.write(r, v);
            log(r, v);
        };
        self.drv.cold_init(&mut w);
        let mix = match track {
            Track::A => &self.a,
            Track::B => &self.b,
        };
        self.drv.play(mix, &mut w);
        self.drv.set_loop(true);
        self.acc = 0;
    }

    /// `10B12h`：`AX=5`、`AX=3`、`AX=0`。
    pub fn stop(&mut self, log: &mut dyn FnMut(u8, u8)) {
        self.drv.set_loop(false);
        let opl = &mut self.opl;
        let mut w = |r: u8, v: u8| {
            opl.write(r, v);
            log(r, v);
        };
        self.drv.stop(&mut w);
        self.drv.cold_init(&mut w);
        self.acc = 0;
    }

    /// 產生一個取樣。`log` 收這個取樣之前的暫存器寫入。
    pub fn sample(&mut self, log: &mut dyn FnMut(u8, u8)) -> i16 {
        if self.drv.playing() {
            self.acc += PIT_PER_SAMPLE;
            loop {
                let div = match self.drv.divisor() {
                    0 => 65536,
                    d => d,
                };
                if self.acc < div {
                    break;
                }
                self.acc -= div;
                let mix = match self.cur {
                    Track::A => &self.a,
                    Track::B => &self.b,
                };
                let opl = &mut self.opl;
                self.drv.tick(mix, &mut |r, v| {
                    opl.write(r, v);
                    log(r, v);
                });
                if !self.drv.playing() {
                    break;
                }
            }
        }
        let v = self.opl.sample() as i32 + self.spk.sample(PIT_PER_SAMPLE);
        v.clamp(-32768, 32767) as i16
    }

    /// 一段繪圖附帶音（`sub_192EA`），排在目前喇叭佇列之後。
    pub fn speaker(&mut self, ev: &[SpkEv]) {
        self.spk.push(ev);
    }

    pub fn track(&self) -> Track {
        self.cur
    }
}
