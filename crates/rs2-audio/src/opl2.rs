//! YM3812（OPL2）合成，一個取樣 = 72 個主時脈（3.579545 MHz ÷ 72 = 49716 Hz）。
//!
//! 整數運算，同樣的暫存器序列永遠得到同樣的取樣。行為依公開的晶片分析：
//! 對數正弦與指數表的公式來自 OPLx 解封裝的 ROM 內容（Gambrell、Niemitalo），
//! 包絡、節奏聲部與相位的規則是這個系列晶片已知的行為。Nuked OPL3 只當黑箱參考，
//! 比對工具在 `tools/audio/`，它的程式碼不在這個 repo。

/// 晶片取樣率的分子分母：`3579545 / 72`。
pub const NATIVE_RATE_NUM: u64 = 3_579_545;
pub const NATIVE_RATE_DEN: u64 = 72;
/// 四捨五入的取樣率，給音效裝置。
pub const NATIVE_RATE: u32 = 49716;

const MULT: [u32; 16] = [1, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 20, 24, 24, 30, 30];
/// 音階衰減表，以 `fnum` 高 4 位元查。
const KSL_ROM: [i32; 16] = [0, 32, 40, 45, 48, 51, 53, 55, 56, 58, 59, 60, 61, 62, 63, 64];
/// KSL 值到右移量：0 dB、3、1.5、6 dB/oct。
const KSL_SHIFT: [u32; 4] = [8, 1, 2, 0];
/// 暫存器位移到槽位，`-1` 不是運算子。
const SLOT_OF: [i8; 22] = [0, 1, 2, 3, 4, 5, -1, -1, 6, 7, 8, 9, 10, 11, -1, -1, 12, 13, 14, 15, 16, 17];
/// 每個頻道的調變器槽位，載波是調變器加 3。
const MOD_SLOT: [usize; 9] = [0, 1, 2, 6, 7, 8, 12, 13, 14];
/// 槽位所屬頻道。
const CH_OF: [usize; 18] = [0, 1, 2, 0, 1, 2, 3, 4, 5, 3, 4, 5, 6, 7, 8, 6, 7, 8];
/// 快速段每個 tick 的微調，以 `rate_lo` 與計時器低 2 位元查。
const INC_STEP: [[u8; 4]; 4] = [[0, 0, 0, 0], [1, 0, 0, 0], [1, 0, 1, 0], [1, 1, 1, 0]];

const KEY_CHANNEL: u8 = 1;
const KEY_RHYTHM: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Eg {
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Debug)]
struct Slot {
    am: bool,
    vib: bool,
    egt: bool,
    ksr: bool,
    mult: u8,
    ksl: u8,
    tl: u8,
    ar: u8,
    dr: u8,
    sl: u8,
    rr: u8,
    wave: u8,
    key: u8,
    eg: Eg,
    /// 9 位元衰減，每單位 0.1875 dB。511 是靜音。
    att: u16,
    reset: bool,
    /// 19 位元相位累加器，高 10 位元是輸出相位。
    phase: u32,
    /// 累加器的高 10 位元，節奏聲部的相位由它組合。
    phase_raw: u16,
    phase_out: u16,
    out: i32,
    prev: i32,
    fbmod: i32,
}

impl Slot {
    fn new() -> Slot {
        Slot {
            am: false,
            vib: false,
            egt: false,
            ksr: false,
            mult: 0,
            ksl: 0,
            tl: 0,
            ar: 0,
            dr: 0,
            sl: 0,
            rr: 0,
            wave: 0,
            key: 0,
            eg: Eg::Release,
            att: 0x1FF,
            reset: false,
            phase: 0,
            phase_raw: 0,
            phase_out: 0,
            out: 0,
            prev: 0,
            fbmod: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    fnum: u16,
    block: u8,
    fb: u8,
    cnt: bool,
}

#[derive(Clone)]
pub struct Opl2 {
    logsin: [u16; 256],
    exp: [u16; 256],
    slots: Vec<Slot>,
    ch: [Channel; 9],
    wse: bool,
    nts: bool,
    dam: bool,
    dvb: bool,
    rhythm: bool,
    /// 每個取樣加 1 的計時器，震音與顫音用。
    timer: u32,
    tremolo_pos: u8,
    vib_pos: u8,
    /// 包絡計時器，每兩個取樣走一步。
    eg_timer: u64,
    eg_half: bool,
    eg_add: u32,
    eg_low: usize,
    /// 23 位元雜訊暫存器。
    noise: u32,
}

impl Default for Opl2 {
    fn default() -> Self {
        Self::new()
    }
}

impl Opl2 {
    pub fn new() -> Opl2 {
        let mut logsin = [0u16; 256];
        let mut exp = [0u16; 256];
        for i in 0..256 {
            let x = (i as f64 + 0.5) * std::f64::consts::PI / 512.0;
            logsin[i] = (-x.sin().log2() * 256.0).round() as u16;
            exp[i] = ((2f64.powf(i as f64 / 256.0) - 1.0) * 1024.0).round() as u16;
        }
        Opl2 {
            logsin,
            exp,
            slots: vec![Slot::new(); 18],
            ch: [Channel::default(); 9],
            wse: false,
            nts: false,
            dam: false,
            dvb: false,
            rhythm: false,
            timer: 0,
            tremolo_pos: 0,
            vib_pos: 0,
            eg_timer: 0,
            eg_half: false,
            eg_add: 0,
            eg_low: 0,
            noise: 1,
        }
    }

    pub fn write(&mut self, reg: u8, v: u8) {
        match reg {
            0x01 => self.wse = v & 0x20 != 0,
            0x08 => self.nts = v & 0x40 != 0,
            0x20..=0x35 | 0x40..=0x55 | 0x60..=0x75 | 0x80..=0x95 | 0xE0..=0xF5 => {
                let s = SLOT_OF[(reg & 0x1F) as usize];
                if s < 0 {
                    return;
                }
                let sl = &mut self.slots[s as usize];
                match reg & 0xE0 {
                    0x20 => {
                        sl.am = v & 0x80 != 0;
                        sl.vib = v & 0x40 != 0;
                        sl.egt = v & 0x20 != 0;
                        sl.ksr = v & 0x10 != 0;
                        sl.mult = v & 0x0F;
                    }
                    0x40 => {
                        sl.ksl = v >> 6;
                        sl.tl = v & 0x3F;
                    }
                    0x60 => {
                        sl.ar = v >> 4;
                        sl.dr = v & 0x0F;
                    }
                    0x80 => {
                        sl.sl = if v >> 4 == 0x0F { 0x1F } else { v >> 4 };
                        sl.rr = v & 0x0F;
                    }
                    _ => sl.wave = v & 3,
                }
            }
            0xA0..=0xA8 => {
                let c = &mut self.ch[(reg - 0xA0) as usize];
                c.fnum = (c.fnum & 0x300) | v as u16;
            }
            0xB0..=0xB8 => {
                let i = (reg - 0xB0) as usize;
                let c = &mut self.ch[i];
                c.fnum = (c.fnum & 0xFF) | (((v & 3) as u16) << 8);
                c.block = (v >> 2) & 7;
                self.set_key(MOD_SLOT[i], KEY_CHANNEL, v & 0x20 != 0);
                self.set_key(MOD_SLOT[i] + 3, KEY_CHANNEL, v & 0x20 != 0);
            }
            0xC0..=0xC8 => {
                let c = &mut self.ch[(reg - 0xC0) as usize];
                c.fb = (v >> 1) & 7;
                c.cnt = v & 1 != 0;
            }
            0xBD => {
                self.dam = v & 0x80 != 0;
                self.dvb = v & 0x40 != 0;
                self.rhythm = v & 0x20 != 0;
                let on = |bit: u8| self.rhythm && v & bit != 0;
                let (bd, sd, tom, tc, hh) = (on(0x10), on(0x08), on(0x04), on(0x02), on(0x01));
                self.set_key(12, KEY_RHYTHM, bd);
                self.set_key(15, KEY_RHYTHM, bd);
                self.set_key(13, KEY_RHYTHM, hh);
                self.set_key(14, KEY_RHYTHM, tom);
                self.set_key(16, KEY_RHYTHM, sd);
                self.set_key(17, KEY_RHYTHM, tc);
            }
            _ => {}
        }
    }

    fn set_key(&mut self, s: usize, bit: u8, on: bool) {
        if on {
            self.slots[s].key |= bit;
        } else {
            self.slots[s].key &= !bit;
        }
    }

    /// 震音值，9 位元衰減單位。
    fn tremolo(&self) -> u16 {
        let p = self.tremolo_pos as u16;
        let t = if p >= 105 { 210 - p } else { p };
        t >> if self.dam { 2 } else { 4 }
    }

    fn eg_update(&mut self, s: usize) {
        let c = self.ch[CH_OF[s]];
        let sl = &self.slots[s];
        let reset = sl.key != 0 && sl.eg == Eg::Release;
        let reg_rate = if reset {
            sl.ar
        } else {
            match sl.eg {
                Eg::Attack => sl.ar,
                Eg::Decay => sl.dr,
                Eg::Sustain => {
                    if sl.egt {
                        0
                    } else {
                        sl.rr
                    }
                }
                Eg::Release => sl.rr,
            }
        };
        // 速率
        let ksv = ((c.block as u32) << 1) | ((c.fnum as u32 >> if self.nts { 8 } else { 9 }) & 1);
        let rate = if reg_rate != 0 { reg_rate as u32 * 4 + (ksv >> if sl.ksr { 0 } else { 2 }) } else { 0 };
        let rate_hi = (rate >> 2).min(15);
        let rate_lo = (rate & 3) as usize;
        let mut step = 0u32;
        if reg_rate != 0 {
            if rate_hi < 12 {
                if self.eg_half {
                    match rate_hi + self.eg_add {
                        12 => step = 1,
                        13 => step = (rate_lo as u32 >> 1) & 1,
                        14 => step = rate_lo as u32 & 1,
                        _ => {}
                    }
                }
            } else {
                step = (rate_hi & 3) + INC_STEP[rate_lo][self.eg_low] as u32;
                if step & 4 != 0 {
                    step = 3;
                }
                if step == 0 {
                    step = self.eg_half as u32;
                }
            }
        }
        let old = sl.att as i32;
        let mut att = old;
        let mut inc = 0i32;
        if reset && rate_hi == 15 {
            att = 0;
        }
        let off = old & 0x1F8 == 0x1F8;
        if sl.eg != Eg::Attack && !reset && off {
            att = 0x1FF;
        }
        let mut eg = sl.eg;
        match sl.eg {
            Eg::Attack => {
                if old == 0 {
                    eg = Eg::Decay;
                } else if sl.key != 0 && step > 0 && rate_hi != 15 {
                    inc = (!old) >> (4 - step);
                }
            }
            Eg::Decay => {
                if (old >> 4) == sl.sl as i32 {
                    eg = Eg::Sustain;
                } else if !off && !reset && step > 0 {
                    inc = 1 << (step - 1);
                }
            }
            Eg::Sustain | Eg::Release => {
                if !off && !reset && step > 0 {
                    inc = 1 << (step - 1);
                }
            }
        }
        let key = sl.key;
        let sl = &mut self.slots[s];
        sl.att = ((att + inc) & 0x1FF) as u16;
        sl.reset = reset;
        if reset {
            eg = Eg::Attack;
        }
        if key == 0 {
            eg = Eg::Release;
        }
        sl.eg = eg;
    }

    fn pg_update(&mut self, s: usize) {
        let c = self.ch[CH_OF[s]];
        let vib_pos = self.vib_pos;
        let dvb = self.dvb;
        let rhythm = self.rhythm;
        let noise = self.noise;
        let (p13, p17) = (self.slots[13].phase_raw as u32, self.slots[17].phase_raw as u32);
        let sl = &mut self.slots[s];
        let mut f = c.fnum as i32;
        if sl.vib {
            let mut range = (f >> 7) & 7;
            if vib_pos & 3 == 0 {
                range = 0;
            } else if vib_pos & 1 != 0 {
                range >>= 1;
            }
            range >>= if dvb { 0 } else { 1 };
            if vib_pos & 4 != 0 {
                range = -range;
            }
            f += range;
        }
        let base = ((f as u32) << c.block) >> 1;
        let mut out = sl.phase >> 9;
        sl.phase_raw = (out & 0x3FF) as u16;
        if sl.reset {
            sl.phase = 0;
        }
        sl.phase = sl.phase.wrapping_add((base * MULT[sl.mult as usize]) >> 1);
        if rhythm && (s == 13 || s == 16 || s == 17) {
            // 節奏聲部：鈸與踩鈸的相位由槽 13、17 的位元組合，小鼓由雜訊與槽 13 組合。
            // 槽 13 算到自己時要用這一個取樣的相位，槽 17 也一樣。
            let (p13, p17) = match s {
                13 => (sl.phase_raw as u32, p17),
                17 => (p13, sl.phase_raw as u32),
                _ => (p13, p17),
            };
            let b = |p: u32, n: u32| (p >> n) & 1;
            let x = (b(p13, 2) ^ b(p13, 7)) | (b(p13, 3) ^ b(p17, 5)) | (b(p17, 3) ^ b(p17, 5));
            let n = noise & 1;
            out = match s {
                13 => (x << 9) | if x ^ n != 0 { 0xD0 } else { 0x34 },
                16 => (b(p13, 8) << 9) | ((b(p13, 8) ^ n) << 8),
                _ => (x << 9) | 0x80,
            };
        }
        sl.phase_out = (out & 0x3FF) as u16;
    }

    fn envelope(&self, s: usize) -> u32 {
        let sl = &self.slots[s];
        let c = self.ch[CH_OF[s]];
        let ksl = (KSL_ROM[(c.fnum >> 6) as usize & 0xF] << 2) - ((8 - c.block as i32) << 5);
        let ksl = ksl.max(0) as u32 >> KSL_SHIFT[sl.ksl as usize];
        let trem = if sl.am { self.tremolo() as u32 } else { 0 };
        (sl.att as u32 + ((sl.tl as u32) << 2) + ksl + trem).min(0x1FF)
    }

    fn exp_out(&self, level: u32) -> i32 {
        let level = level.min(0x1FFF);
        let m = self.exp[255 - (level & 0xFF) as usize] as i32 + 1024;
        (m << 1) >> (level >> 8)
    }

    /// 四種波形。回傳 13 位元有號值，負半週是一的補數。
    fn wave_out(&self, wave: u8, phase: u32, env: u32) -> i32 {
        let phase = phase & 0x3FF;
        let quad = |p: u32| -> usize { if p & 0x100 != 0 { ((p & 0xFF) ^ 0xFF) as usize } else { (p & 0xFF) as usize } };
        let (level, neg) = match wave {
            0 => (self.logsin[quad(phase)] as u32, phase & 0x200 != 0),
            1 => {
                if phase & 0x200 != 0 {
                    (0x1000, false)
                } else {
                    (self.logsin[quad(phase)] as u32, false)
                }
            }
            2 => (self.logsin[quad(phase)] as u32, false),
            _ => {
                if phase & 0x100 != 0 {
                    (0x1000, false)
                } else {
                    (self.logsin[(phase & 0xFF) as usize] as u32, false)
                }
            }
        };
        let o = self.exp_out(level + (env << 3));
        if neg { !o } else { o }
    }

    /// 一個運算子：包絡、相位、輸出。`modulation` 是加進相位的值。
    fn slot_generate(&mut self, s: usize, modulation: i32) {
        // 輸出用的是這次更新之前的衰減值。
        let env = self.envelope(s);
        self.eg_update(s);
        self.pg_update(s);
        let wave = if self.wse { self.slots[s].wave } else { 0 };
        let phase = (self.slots[s].phase_out as i32 + modulation) as u32;
        let o = self.wave_out(wave, phase, env);
        let fb = self.ch[CH_OF[s]].fb as u32;
        let sl = &mut self.slots[s];
        sl.prev = sl.out;
        sl.out = o;
        sl.fbmod = if fb != 0 { (sl.prev + sl.out) >> (9 - fb) } else { 0 };
    }

    /// 這個槽位的相位調變來源。
    fn modulation_of(&self, s: usize) -> i32 {
        let ch = CH_OF[s];
        let is_mod = s == MOD_SLOT[ch];
        if self.rhythm && ch >= 7 {
            return 0;
        }
        if is_mod {
            return if ch == 6 || !self.rhythm || ch < 6 { self.slots[s].fbmod } else { 0 };
        }
        if self.ch[ch].cnt {
            0
        } else {
            self.slots[s - 3].out
        }
    }

    /// 產生一個取樣。
    pub fn sample(&mut self) -> i16 {
        for s in 0..18 {
            let m = self.modulation_of(s);
            self.slot_generate(s, m);
        }
        let mut mix = 0i32;
        for c in 0..9 {
            let (a, b) = (self.slots[MOD_SLOT[c]].out, self.slots[MOD_SLOT[c] + 3].out);
            if self.rhythm && c >= 6 {
                // 節奏聲部：大鼓只取載波，其餘兩個運算子各自輸出，都是兩倍。
                mix += if c == 6 { 2 * b } else { 2 * (a + b) };
            } else if self.ch[c].cnt {
                mix += a + b;
            } else {
                mix += b;
            }
        }
        self.advance();
        mix.clamp(-32768, 32767) as i16
    }

    /// 全域計時器。
    fn advance(&mut self) {
        if self.timer & 0x3F == 0x3F {
            self.tremolo_pos = (self.tremolo_pos + 1) % 210;
        }
        if self.timer & 0x3FF == 0x3FF {
            self.vib_pos = (self.vib_pos + 1) & 7;
        }
        self.timer = self.timer.wrapping_add(1);
        // 雜訊：23 位元 LFSR，位元 0 與 14 互斥後進最高位。
        let n = (self.noise ^ (self.noise >> 14)) & 1;
        self.noise = (self.noise >> 1) | (n << 22);
        // 包絡計時器每兩個取樣走一步，eg_add 是計時器最低的 1 在第幾位元加 1。
        if self.eg_half {
            let tz = self.eg_timer.trailing_zeros();
            self.eg_add = if tz > 12 { 0 } else { tz + 1 };
            self.eg_low = (self.eg_timer & 3) as usize;
        }
        if self.eg_half {
            self.eg_timer = self.eg_timer.wrapping_add(1);
        }
        self.eg_half = !self.eg_half;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一個載波直接輸出的音：`fnum`、`block` 決定頻率，TL 0、最快包絡。
    fn tone(o: &mut Opl2, fnum: u16, block: u8) {
        o.write(0x01, 0x20);
        o.write(0x20, 0x01);
        o.write(0x23, 0x01);
        o.write(0x40, 0x3F);
        o.write(0x43, 0x00);
        o.write(0x60, 0xF0);
        o.write(0x63, 0xF0);
        o.write(0x80, 0x00);
        o.write(0x83, 0x00);
        o.write(0xC0, 0x00);
        o.write(0xA0, fnum as u8);
        o.write(0xB0, 0x20 | (block << 2) | (fnum >> 8) as u8);
    }

    #[test]
    fn rom_tables_match_known_endpoints() {
        let o = Opl2::new();
        assert_eq!(o.logsin[0], 0x859);
        assert_eq!(o.logsin[255], 0);
        assert_eq!(o.exp[255], 1018);
        assert_eq!(o.exp[0], 0);
        assert_eq!(o.exp_out(0), 4084);
    }

    #[test]
    fn silent_after_reset() {
        let mut o = Opl2::new();
        assert!((0..1000).all(|_| o.sample() == 0));
    }

    /// 頻率公式：`fnum × 2^block × 49716 / 2^20`。數過零點。
    #[test]
    fn tone_has_the_expected_frequency() {
        let mut o = Opl2::new();
        tone(&mut o, 0x241, 4); // 約 440 Hz
        for _ in 0..200 {
            o.sample();
        }
        let n = 49716usize;
        let s: Vec<i16> = (0..n).map(|_| o.sample()).collect();
        let ups = s.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count();
        let want = 0x241 as f64 * 16.0 * 49716.0 / (1u64 << 20) as f64;
        assert!((ups as f64 - want).abs() <= 1.0, "{ups} {want}");
        let peak = s.iter().map(|&v| (v as i32).abs()).max().unwrap();
        assert!((4000..=4085).contains(&peak), "{peak}");
    }

    #[test]
    fn key_off_decays_to_silence() {
        let mut o = Opl2::new();
        tone(&mut o, 0x241, 4);
        o.write(0x83, 0x0F);
        for _ in 0..2000 {
            o.sample();
        }
        o.write(0xB0, 0x10);
        for _ in 0..200_000 {
            o.sample();
        }
        assert!((0..1000).all(|_| o.sample().abs() <= 1));
    }

    #[test]
    fn same_writes_give_same_samples() {
        let run = || {
            let mut o = Opl2::new();
            tone(&mut o, 0x1A0, 5);
            o.write(0xBD, 0x3F);
            (0..20000).map(|_| o.sample() as i32).sum::<i32>()
        };
        assert_eq!(run(), run());
    }
}
