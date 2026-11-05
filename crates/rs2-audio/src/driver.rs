//! `ASMDRV.EXE` 的 AdLib 驅動（解包映像，IDA 基底 `10000h`）逐函式移植。
//! 註解的位址是驅動映像的線性位址，`dseg:X` = 線性 `10000h + X`。
//!
//! 只寫 OPL2 暫存器，不合成聲音，輸出和原版的 `388h`、`389h` 寫入逐筆比對。
//! 驅動的 INT 8 由呼叫端換成 `Driver::tick`，一個 tick 是 `Driver::divisor` 個 PIT 週期。

/// OPL2 暫存器寫入的接收端。
pub type Sink<'a> = &'a mut dyn FnMut(u8, u8);

/// 運算子到 OPL 槽位偏移，`dseg:1345h`。
const OP_REG_OFF: [u8; 18] = [0, 1, 2, 3, 4, 5, 8, 9, 10, 11, 12, 13, 16, 17, 18, 19, 20, 21];
/// 載波旗標，`dseg:1357h`。
const OP_IS_CARRIER: [bool; 18] =
    [false, false, false, true, true, true, false, false, false, true, true, true, false, false, false, true, true, true];
/// 運算子所屬的 OPL 頻道，`dseg:1369h`。
const OP_CHANNEL: [u8; 18] = [0, 1, 2, 0, 1, 2, 3, 4, 5, 3, 4, 5, 6, 7, 8, 6, 7, 8];
/// 聲部到運算子，`dseg:1329h`，一般模式每聲部兩個。
const VOICE_OPS: [u8; 18] = [0, 3, 1, 4, 2, 5, 6, 9, 7, 10, 8, 11, 12, 15, 13, 16, 14, 17];
/// 節奏模式的聲部 7 到 10 用哪個運算子，`dseg:133Bh` 起。聲部 6 是 12 與 15。
const PERC_OP: [u8; 4] = [0x10, 0x0E, 0x11, 0x0D];
/// BD、SD、TOM、TC、HH 在暫存器 `BDh` 的位元，`dseg:12B4h`。
const RHYTHM_BIT: [u8; 5] = [0x10, 0x08, 0x04, 0x02, 0x01];
/// 預設運算子參數（13 個參數加波形），`dseg:12B9h`。
const DEF_MOD: [u8; 14] = [1, 1, 3, 15, 5, 0, 1, 3, 15, 0, 0, 0, 1, 0];
/// `dseg:12C7h`。
const DEF_CAR: [u8; 14] = [0, 1, 1, 15, 7, 0, 2, 4, 0, 0, 0, 1, 0, 0];
/// 節奏模式的運算子預設，`dseg:12D5h` 至 `131Bh`。
const DEF_PERC: [(u8, [u8; 14]); 6] = [
    (0x0C, [0, 0, 0, 10, 4, 0, 8, 12, 11, 0, 0, 0, 1, 0]),
    (0x0F, [0, 0, 0, 13, 4, 0, 6, 15, 0, 0, 0, 0, 1, 0]),
    (0x10, [0, 12, 0, 15, 11, 0, 8, 5, 0, 0, 0, 0, 0, 0]),
    (0x0E, [0, 4, 0, 15, 11, 0, 7, 5, 0, 0, 0, 0, 0, 0]),
    (0x11, [0, 1, 0, 15, 11, 0, 5, 5, 0, 0, 0, 0, 0, 0]),
    (0x0D, [0, 1, 0, 15, 11, 0, 7, 5, 0, 0, 0, 0, 0, 0]),
];

/// `sub_1091D`、`sub_108A9`、`sub_1080A`：25 列（細調 1/25 半音）× 12 個 fnum，全部是 32 位元整數運算。
fn freq_rows() -> [[u16; 12]; 25] {
    let mut rows = [[0u16; 12]; 25];
    for (r, row) in rows.iter_mut().enumerate() {
        let t = 100u32 * 100 + 6 * (4 * r as u32);
        let q = t.wrapping_mul(0xCB78) / (100 * 100 * 25);
        let mut f = (q << 14).wrapping_mul(9) / 0x1B503;
        for (k, w) in row.iter_mut().enumerate() {
            if k > 0 {
                f = f.wrapping_mul(0x6A) / 100;
            }
            *w = (f as u16).wrapping_add(4) >> 3;
        }
    }
    rows
}

/// `A.MIX`、`B.MIX` 的內容。格式見 `docs/re/13` 第 2 節。
#[derive(Clone, Debug)]
pub struct Mix {
    /// 每個音色 28 個 u16。
    pub inst: Vec<[u16; 28]>,
    pub tick_beat: u8,
    pub sound_mode: u8,
    pub bend_range: u8,
    pub tempo: u16,
    pub stream: Vec<u8>,
}

impl Mix {
    pub fn parse(buf: &[u8]) -> Result<Mix, String> {
        let u16at = |o: usize| -> Result<u16, String> {
            buf.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(|| format!("MIX 在 {o} 不足"))
        };
        let n = u16at(0)? as usize;
        let tab = u16at(2)? as usize;
        if tab != n * 56 {
            return Err(format!("音色表長度 {tab} 不等於 {n} × 56"));
        }
        let mut inst = Vec::with_capacity(n);
        for i in 0..n {
            let mut w = [0u16; 28];
            for (k, v) in w.iter_mut().enumerate() {
                *v = u16at(4 + i * 56 + k * 2)?;
            }
            inst.push(w);
        }
        let p = 4 + tab;
        let head = buf.get(p..p + 8).ok_or("MIX 曲目表頭不足")?;
        let stream_len = u16::from_le_bytes([head[2], head[3]]) as usize;
        let stream = buf[p + 8..].to_vec();
        if stream.len() != stream_len {
            return Err(format!("事件串流 {} 位元組，表頭寫 {stream_len}", stream.len()));
        }
        Ok(Mix {
            inst,
            tick_beat: head[0],
            sound_mode: head[4],
            bend_range: head[5],
            tempo: u16::from_le_bytes([head[6], head[7]]),
            stream,
        })
    }
}

/// `sub_11145`、`sub_1102E`：PIT 除數。`0` 代表 65536。
pub fn tick_divisor(tick_beat: u32, tempo: u32) -> u32 {
    let si = tick_beat.wrapping_mul(tempo) / 60;
    if si == 0 {
        return 0;
    }
    0x1234DC / si.max(0x13)
}

/// 驅動狀態。合成層的欄位跨曲目保留，所以新曲開頭的滑音事件會用到上一首最後的音符。
#[derive(Clone)]
pub struct Driver {
    freq: [[u16; 12]; 25],
    /// `dseg:190Fh`，每運算子 14 位元組。
    prm: [[u8; 14]; 18],
    /// `dseg:1A2Fh`。
    opvol: [u8; 18],
    /// `byte_11A0B`。
    rhythm: u8,
    /// `byte_116B4`。
    rbits: u8,
    /// `word_11A17`。
    wsel: u8,
    /// `word_116B5`、`word_11A41`。
    bend_mul: i32,
    /// `dseg:16A9h`。
    vnote: [i32; 12],
    /// `dseg:1A0Ch`。
    vkey: [bool; 12],
    /// `dseg:1A19h`，列索引。原版存指標。
    vrow: [usize; 12],
    /// `dseg:1A43h`。
    voff: [i32; 12],
    // ---- 事件層 ----
    ptr: usize,
    status: u8,
    chan: u8,
    /// `dseg:1298h`。
    volcache: [u8; 11],
    /// `byte_112A3`。
    playing: bool,
    /// `byte_112A4`。
    looping: bool,
    /// `word_1100C`。
    countdown: u32,
    /// 目前的 PIT 除數。
    divisor: u32,
}

impl Default for Driver {
    fn default() -> Self {
        Self::new()
    }
}

impl Driver {
    pub fn new() -> Driver {
        Driver {
            freq: freq_rows(),
            prm: [[0; 14]; 18],
            opvol: [0x7F; 18],
            rhythm: 0,
            rbits: 0,
            wsel: 0,
            bend_mul: 25,
            vnote: [0; 12],
            vkey: [false; 12],
            vrow: [0; 12],
            voff: [0; 12],
            ptr: 0,
            status: 0,
            chan: 0,
            volcache: [0; 11],
            playing: false,
            looping: false,
            countdown: 0,
            divisor: 0,
        }
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// PIT 除數，一個驅動 tick 的長度。`0` 代表 65536，也就是原版 BIOS 時鐘的 18.2 Hz。
    pub fn divisor(&self) -> u32 {
        self.divisor
    }

    // ---- INT 60h ----

    /// `AX=0`（`10075h`）：`_SOUNDCOLDINIT`，偵測音效卡後重置。
    pub fn cold_init(&mut self, w: Sink) {
        // _BOARDINSTALLED（10F19h）
        for (r, v) in [(0x04, 0x60), (0x04, 0x80), (0x02, 0xFF), (0x04, 0x21), (0x04, 0x60), (0x04, 0x80)] {
            w(r, v);
        }
        // _SOUNDWARMINIT（10427h）
        self.opvol = [0x7F; 18];
        self.vrow = [0; 12];
        self.voff = [0; 12];
        self.set_rhythm_mode(0, w);
        self.reg_bd(w);
        self.reg_08(w);
        for v in 0..9 {
            w(0xA0 + v, 0);
            w(0xB0 + v, 0);
        }
        self.bend_mul = 25;
        self.set_wsel(w);
    }

    /// `AX=2`（`1008Ah`）：開始播放。
    pub fn play(&mut self, mix: &Mix, w: Sink) {
        self.volcache = [0; 11];
        self.set_rhythm_mode(mix.sound_mode, w);
        self.bend_mul = (mix.bend_range as i32).clamp(1, 12) * 25;
        self.divisor = tick_divisor(mix.tick_beat as u32, mix.tempo as u32);
        let first = mix.stream[0];
        self.ptr = 1;
        self.status = 0;
        self.chan = 0;
        self.countdown = if first != 0 { first as u32 } else { 1 };
        self.playing = true;
    }

    /// `AX=4`、`AX=5`（`100E8h`、`100EDh`）：循環開關。
    pub fn set_loop(&mut self, on: bool) {
        self.looping = on;
    }

    /// `AX=3`（`100DCh`）：停止，11 個聲部從最後一個起 note off，計時回 18.2 Hz。
    pub fn stop(&mut self, w: Sink) {
        self.looping = false;
        self.playing = false;
        for v in (0..11).rev() {
            self.note_off(v, w);
        }
        self.divisor = 0;
    }

    /// 一個驅動 tick（`110B8h` 之後的 `word_1100C` 倒數）。
    pub fn tick(&mut self, mix: &Mix, w: Sink) {
        if !self.playing {
            return;
        }
        self.countdown = self.countdown.saturating_sub(1);
        if self.countdown != 0 {
            return;
        }
        match self.step(mix, w) {
            Some(d) => self.countdown = d,
            None => {
                self.playing = false;
                self.divisor = 0;
            }
        }
    }

    // ---- 暫存器層 ----

    /// `sub_10DDA`。AM 與 VIB 深度（`byte_116A4`、`byte_116A8`）這個遊戲都是 0。
    fn reg_bd(&self, w: Sink) {
        w(0xBD, if self.rhythm != 0 { 0x20 } else { 0 } | self.rbits);
    }

    /// `sub_10C4F`。NTS（`byte_116A5`）這個遊戲是 0。
    fn reg_08(&self, w: Sink) {
        w(0x08, 0);
    }

    /// `sub_10BE5`。
    fn reg_tl(&self, op: usize, w: Sink) {
        let p = &self.prm[op];
        let lv = 0x3F - (p[8] as u32 & 0x3F);
        let s = self.opvol[op] as u32 * lv;
        let s = s + (s + 0x7F);
        let x = 0x3F - (s / 0xFE);
        w(0x40 + OP_REG_OFF[op], (x as u8) | (p[0] << 6));
    }

    /// `sub_10C69`。
    fn reg_fb(&self, op: usize, w: Sink) {
        if OP_IS_CARRIER[op] {
            return;
        }
        let p = &self.prm[op];
        w(0xC0 + OP_CHANNEL[op], (p[2] << 1) | if p[12] != 0 { 0 } else { 1 });
    }

    /// `sub_10CBB`。
    fn reg_ad(&self, op: usize, w: Sink) {
        let p = &self.prm[op];
        w(0x60 + OP_REG_OFF[op], (p[3] << 4) | (p[6] & 0x0F));
    }

    /// `sub_10D01`。
    fn reg_sr(&self, op: usize, w: Sink) {
        let p = &self.prm[op];
        w(0x80 + OP_REG_OFF[op], (p[4] << 4) | (p[7] & 0x0F));
    }

    /// `sub_10D47`。
    fn reg_mult(&self, op: usize, w: Sink) {
        let p = &self.prm[op];
        let f = |b: u8, m: u8| if b != 0 { m } else { 0 };
        w(0x20 + OP_REG_OFF[op], f(p[9], 0x80) + f(p[10], 0x40) + f(p[5], 0x20) + f(p[11], 0x10) + (p[1] & 0x0F));
    }

    /// `sub_10E1D`。
    fn reg_wave(&self, op: usize, w: Sink) {
        let v = if self.wsel != 0 { self.prm[op][13] & 3 } else { 0 };
        w(0xE0 + OP_REG_OFF[op], v);
    }

    /// `sub_10BB1`。
    fn op_refresh(&self, op: usize, w: Sink) {
        self.reg_bd(w);
        self.reg_08(w);
        self.reg_tl(op, w);
        self.reg_fb(op, w);
        self.reg_ad(op, w);
        self.reg_sr(op, w);
        self.reg_mult(op, w);
        self.reg_wave(op, w);
    }

    /// `sub_10ABF`。音色欄取低 8 位元。
    fn set_op(&mut self, op: usize, params: &[u16], wave: u16, w: Sink) {
        for (d, s) in self.prm[op].iter_mut().zip(params.iter().take(13)) {
            *d = *s as u8;
        }
        self.prm[op][13] = wave as u8 & 3;
        self.op_refresh(op, w);
    }

    /// `sub_10759`。
    fn init_ops(&mut self, w: Sink) {
        for op in 0..18 {
            let rec = if OP_IS_CARRIER[op] { DEF_CAR } else { DEF_MOD };
            self.set_op(op, &rec.map(u16::from), 0, w);
        }
        if self.rhythm != 0 {
            for (op, rec) in DEF_PERC {
                self.set_op(op as usize, &rec.map(u16::from), 0, w);
            }
        }
    }

    /// `sub_10E58`。
    fn voice_pitch(&mut self, v: usize, note: i32, key: bool, w: Sink) {
        self.vkey[v] = key;
        self.vnote[v] = note;
        let n = (note + self.voff[v]).clamp(0, 0x5F) as usize;
        let fnum = self.freq[self.vrow[v]][n % 12];
        w(0xA0 + v as u8, fnum as u8);
        w(0xB0 + v as u8, if key { 0x20 } else { 0 } + (n / 12) as u8 * 4 + ((fnum >> 8) & 3) as u8);
    }

    /// `sub_10469`。
    fn set_rhythm_mode(&mut self, mode: u8, w: Sink) {
        if mode != 0 {
            for v in 6..9 {
                w(0xA0 + v, 0);
                w(0xB0 + v, 0);
            }
            self.voice_pitch(8, 0x18, false, w);
            self.voice_pitch(7, 0x1F, false, w);
        }
        self.rhythm = mode;
        self.rbits = 0;
        self.init_ops(w);
        self.reg_bd(w);
    }

    /// `_SET3812(1)`（`sub_104C4`）。
    fn set_wsel(&mut self, w: Sink) {
        self.wsel = 0x20;
        for off in OP_REG_OFF {
            w(0xE0 + off, 0);
        }
        w(0x01, self.wsel);
    }

    fn is_perc(&self, v: usize) -> bool {
        self.rhythm != 0 && v >= 6
    }

    /// `sub_10550`。
    fn set_instrument(&mut self, v: usize, inst: &[u16; 28], w: Sink) {
        let (a, b) = (&inst[0..13], &inst[13..26]);
        let (w0, w1) = (inst[26], inst[27]);
        if !self.is_perc(v) {
            self.set_op(VOICE_OPS[2 * v] as usize, a, w0, w);
            self.set_op(VOICE_OPS[2 * v + 1] as usize, b, w1, w);
        } else if v == 6 {
            self.set_op(0x0C, a, w0, w);
            self.set_op(0x0F, b, w1, w);
        } else {
            self.set_op(PERC_OP[v - 7] as usize, a, w0, w);
        }
    }

    fn carrier_op(&self, v: usize) -> usize {
        if !self.is_perc(v) {
            VOICE_OPS[2 * v + 1] as usize
        } else if v == 6 {
            0x0F
        } else {
            PERC_OP[v - 7] as usize
        }
    }

    /// `10603h`。
    fn set_volume(&mut self, v: usize, vol: u8, w: Sink) {
        let op = self.carrier_op(v);
        self.opvol[op] = vol.min(0x7F);
        self.reg_tl(op, w);
    }

    /// `106A1h`。
    fn note_on(&mut self, v: usize, midi: u8, w: Sink) {
        let n = (midi as i32 - 12).max(0);
        if !self.is_perc(v) {
            self.voice_pitch(v, n, true, w);
            return;
        }
        if v == 6 {
            self.voice_pitch(6, n, false, w);
        } else if v == 8 {
            self.voice_pitch(8, n, false, w);
            self.voice_pitch(7, n + 7, false, w);
        }
        self.rbits |= RHYTHM_BIT[v - 6];
        self.reg_bd(w);
    }

    /// `sub_1071E`。
    fn note_off(&mut self, v: usize, w: Sink) {
        if !self.is_perc(v) {
            let n = self.vnote[v];
            self.voice_pitch(v, n, false, w);
        } else {
            self.rbits &= !RHYTHM_BIT[v - 6];
            self.reg_bd(w);
        }
    }

    /// `10660h`、`sub_109B4`。
    fn pitch_bend(&mut self, v: usize, value: u16, w: Sink) {
        if self.rhythm != 0 && v > 6 {
            return;
        }
        let value = value.min(0x3FFF) as i32;
        let prod = (value - 0x2000) * self.bend_mul;
        // Borland 的 LDIV 往零截斷。
        let di = prod / 0x2000;
        if di < 0 {
            let m = -di;
            self.voff[v] = -((0x18 + m) / 25);
            let rem = m % 25;
            self.vrow[v] = if rem != 0 { (25 - rem) as usize } else { 0 };
        } else {
            self.voff[v] = di / 25;
            self.vrow[v] = (di % 25) as usize;
        }
        let (n, k) = (self.vnote[v], self.vkey[v]);
        self.voice_pitch(v, n, k, w);
    }

    // ---- 事件層 ----

    /// `103DDh`：`Ok` 是下一個事件的延遲，`Err(())` 表示同刻繼續處理。
    fn delta_check(&mut self, st: &[u8], d: u8) -> Option<u32> {
        if d == 0 {
            return None;
        }
        if d == 0xF8 {
            let nxt = st[self.ptr];
            if nxt != 0xF8 {
                self.ptr += 1;
                return Some(0xF0 + nxt as u32);
            }
        }
        Some(d as u32)
    }

    /// `103CFh`。
    fn read_delta(&mut self, st: &[u8]) -> Option<u32> {
        let d = st[self.ptr];
        self.ptr += 1;
        self.delta_check(st, d)
    }

    /// `sub_10189`：處理到下一個非零 delta，回傳延遲。曲終且不循環時回傳 `None`。
    fn step(&mut self, mix: &Mix, w: Sink) -> Option<u32> {
        let st = &mix.stream[..];
        loop {
            let p = self.ptr;
            let b = st[p];
            if b == 0xF8 {
                // 101A3h
                self.ptr += 1;
                if let Some(d) = self.delta_check(st, 0xF8) {
                    return Some(d);
                }
                continue;
            }
            if b == 0xFC {
                // 101AFh
                if self.looping {
                    self.ptr = 1;
                    continue;
                }
                return None;
            }
            if b == 0xF0 {
                // 10229h
                let d;
                if st[p + 1] == 0x7F && st[p + 2] == 0 {
                    let base = mix.tempo as u32;
                    let new = ((st[p + 3] as u32 * base) & 0xFFFF).wrapping_add(((st[p + 4] as u32 * base) >> 7) & 0xFFFF) & 0xFFFF;
                    self.divisor = tick_divisor(mix.tick_beat as u32, new);
                    d = st[p + 6];
                    self.ptr = p + 7;
                } else {
                    let mut q = p + 1;
                    while st[q] != 0xF7 {
                        q += 1;
                    }
                    d = st[q + 1];
                    self.ptr = q + 2;
                }
                if let Some(d) = self.delta_check(st, d) {
                    return Some(d);
                }
                continue;
            }
            if b >= 0x80 {
                // 102A1h
                self.status = b & 0xF0;
                self.chan = b & 0x0F;
                self.ptr += 1;
            }
            let p = self.ptr;
            let ch = self.chan as usize;
            match self.status {
                0x90 => {
                    let vel = st[p + 1];
                    if vel == 0 {
                        self.note_off(ch, w);
                    } else {
                        if ch < 11 && vel != self.volcache[ch] {
                            self.volcache[ch] = vel;
                            self.set_volume(ch, vel, w);
                        }
                        self.note_on(ch, st[p], w);
                    }
                    self.ptr = p + 2;
                }
                0xC0 => {
                    let inst = &mix.inst[st[p] as usize];
                    self.set_instrument(ch, inst, w);
                    self.ptr = p + 1;
                }
                0xE0 => {
                    self.pitch_bend(ch, st[p] as u16 + ((st[p + 1] as u16) << 7), w);
                    self.ptr = p + 2;
                }
                0xB0 => self.ptr = p + 2,
                0xD0 => self.ptr = p + 1,
                // 兩首曲子都沒有 80h：`1031Fh` 處理後不讀 delta，是驅動缺陷，這裡不移植那條路。
                0x80 => unreachable!("A.MIX、B.MIX 沒有 80h 事件"),
                _ => {
                    // 103B8h：A0h 等沒有實作的事件，往後掃到下一個 >= 80h 的位元組。
                    let mut q = p;
                    while st[q] < 0x80 {
                        q += 1;
                    }
                    self.ptr = if st[q] != 0xF8 { q - 1 } else { q };
                }
            }
            if let Some(d) = self.read_delta(st) {
                return Some(d);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn divisors_match_the_documented_rates() {
        assert_eq!(tick_divisor(240, 100), 2982);
        assert_eq!(tick_divisor(240, 120), 2485);
        assert_eq!(tick_divisor(1, 0), 0);
    }

    #[test]
    fn freq_row_zero_matches_the_documented_values() {
        assert_eq!(freq_rows()[0], [343, 364, 385, 408, 433, 459, 486, 515, 546, 579, 614, 650]);
    }

    #[test]
    fn cold_init_ends_with_wave_select_enable() {
        let mut d = Driver::new();
        let mut log = Vec::new();
        d.cold_init(&mut |r, v| log.push((r, v)));
        assert_eq!(&log[..6], &[(4, 0x60), (4, 0x80), (2, 0xFF), (4, 0x21), (4, 0x60), (4, 0x80)]);
        assert_eq!(log.last(), Some(&(1, 0x20)));
    }
}
