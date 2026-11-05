//! 原版音訊收據（`rs2dyn` 的 `audiotrace`）的解析與比對，給測試用。
//!
//! 收據一行一筆：`O 步數 tick 週期 暫存器 值`（十六進位）、`S 步數 tick 0 位準 閘門`、`P 步數 tick 0 除數 模式`。

use crate::driver::{Driver, Mix};
use crate::engine::Track;

/// 一筆 OPL 寫入。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OplW {
    pub step: u64,
    pub tick: u64,
    pub reg: u8,
    pub val: u8,
}

/// 喇叭與 PIT 通道 2 的事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpkLine {
    Divisor { step: u64, tick: u64, div: u32 },
    Gate { step: u64, tick: u64, on: bool },
}

impl SpkLine {
    pub fn step(&self) -> u64 {
        match *self {
            SpkLine::Divisor { step, .. } | SpkLine::Gate { step, .. } => step,
        }
    }

    /// IRQ 0 的累計次數。中斷處理常式的指令也算在步數裡，兩筆之間有中斷時步數差不等於繪圖迴圈的指令數。
    pub fn tick(&self) -> u64 {
        match *self {
            SpkLine::Divisor { tick, .. } | SpkLine::Gate { tick, .. } => tick,
        }
    }
}

/// 解析收據，回傳 OPL 寫入與喇叭事件。
pub fn parse_tsv(text: &str) -> Result<(Vec<OplW>, Vec<SpkLine>), String> {
    let mut o = Vec::new();
    let mut s = Vec::new();
    for l in text.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 6 {
            return Err(format!("收據格式不符：{l}"));
        }
        let n = |x: &str| x.parse::<u64>().map_err(|e| format!("{x}：{e}"));
        let hex = |x: &str| u8::from_str_radix(x, 16).map_err(|e| format!("{x}：{e}"));
        match f[0] {
            "O" => o.push(OplW { step: n(f[1])?, tick: n(f[2])?, reg: hex(f[4])?, val: hex(f[5])? }),
            "P" => s.push(SpkLine::Divisor { step: n(f[1])?, tick: n(f[2])?, div: n(f[4])? as u32 }),
            "S" => s.push(SpkLine::Gate { step: n(f[1])?, tick: n(f[2])?, on: f[5] == "1" }),
            k => return Err(format!("不認得的種類 {k}")),
        }
    }
    Ok((o, s))
}

/// 由寫入本身分出的驅動呼叫。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seg {
    /// `AX=0`。
    Cold,
    /// `AX=2`。
    Play(Track),
    /// `AX=3`。
    Stop,
}

const COLD: [(u8, u8); 6] = [(4, 0x60), (4, 0x80), (2, 0xFF), (4, 0x21), (4, 0x60), (4, 0x80)];

fn is_cold(real: &[OplW], at: usize) -> bool {
    real.len() >= at + COLD.len() && COLD.iter().enumerate().all(|(i, c)| (real[at + i].reg, real[at + i].val) == *c)
}

/// 這一筆起是 `AX=3` 的 note off：節奏模式下連續 5 個 `BDh`，再來 `A5h`、`B5h`。
/// 已經停止時節奏模式是關的，11 個聲部都走 `sub_10E58`，從聲部 10（`AAh`、`BAh`）開始。
fn is_stop(real: &[OplW], at: usize) -> bool {
    if real.get(at).is_some_and(|w| w.reg == 0xAA) && real.get(at + 1).is_some_and(|w| w.reg == 0xBA) {
        return true;
    }
    let mut k = at;
    while real.get(k).is_some_and(|w| w.reg == 0xBD) {
        k += 1;
    }
    k - at == 5 && real.get(k).is_some_and(|w| w.reg == 0xA5) && real.get(k + 1).is_some_and(|w| w.reg == 0xB5)
}

fn check(real: &[OplW], at: &mut usize, what: &str, got: &[(u8, u8)]) -> Result<(), String> {
    for (i, &(r, v)) in got.iter().enumerate() {
        let Some(w) = real.get(*at) else {
            return Err(format!("{what}：實錄在第 {at} 筆結束，模型還有 {} 筆", got.len() - i));
        };
        if (w.reg, w.val) != (r, v) {
            return Err(format!("{what}：第 {at} 筆，tick {}，實錄 {:02X}:{:02X}，模型 {r:02X}:{v:02X}", w.tick, w.reg, w.val));
        }
        *at += 1;
    }
    Ok(())
}

/// 用模型重播整份實錄，每筆寫入的暫存器與值都要相同，每次播放內 tick 差固定。
/// 回傳每個驅動呼叫的種類與它第一筆寫入的位置。曲目由比較決定，兩首都試，取相同前綴較長的。
pub fn replay_driver(real: &[OplW], a: &Mix, b: &Mix) -> Result<Vec<(usize, Seg)>, String> {
    let mut d = Driver::new();
    let mut at = 0usize;
    let mut segs = Vec::new();
    while at < real.len() {
        if is_stop(real, at) {
            segs.push((at, Seg::Stop));
            let mut got = Vec::new();
            d.stop(&mut |r, v| got.push((r, v)));
            check(real, &mut at, "AX=3", &got)?;
            continue;
        }
        if !is_cold(real, at) {
            return Err(format!("第 {at} 筆不是 AX=0，也不是 AX=3：{:02X}:{:02X}", real[at].reg, real[at].val));
        }
        segs.push((at, Seg::Cold));
        let mut got = Vec::new();
        d.cold_init(&mut |r, v| got.push((r, v)));
        check(real, &mut at, "AX=0", &got)?;
        if at >= real.len() || is_cold(real, at) || is_stop(real, at) {
            continue;
        }
        // AX=2。開頭的寫入兩首相同，要到第一個音符才分得出來。
        let start_tick = real[at].tick;
        let mut best: Option<(usize, Track)> = None;
        for (mix, tr) in [(a, Track::A), (b, Track::B)] {
            let mut t = d.clone();
            let mut log: Vec<(u8, u8)> = Vec::new();
            t.play(mix, &mut |r, v| log.push((r, v)));
            t.set_loop(true);
            let (mut k, mut same) = (0usize, 0usize);
            'sim: for _ in 0..40000 {
                while k < log.len() {
                    match real.get(at + k) {
                        Some(w) if (w.reg, w.val) == log[k] => {
                            same += 1;
                            k += 1;
                        }
                        _ => break 'sim,
                    }
                }
                t.tick(mix, &mut |r, v| log.push((r, v)));
            }
            if best.is_none_or(|(s, _)| same > s) {
                best = Some((same, tr));
            }
        }
        let (_, tr) = best.expect("兩首都試過");
        let mix = if tr == Track::A { a } else { b };
        segs.push((at, Seg::Play(tr)));
        let mut got = Vec::new();
        d.play(mix, &mut |r, v| got.push((r, v)));
        check(real, &mut at, "AX=2", &got)?;
        d.set_loop(true);
        // 逐 tick 推進，直到實錄進入 AX=3 或結束。
        let mut tick = 0u64;
        let mut offset: Option<i64> = None;
        while at < real.len() && !is_stop(real, at) {
            tick += 1;
            let mut got = Vec::new();
            d.tick(mix, &mut |r, v| got.push((r, v)));
            if !got.is_empty() {
                let off = real[at].tick as i64 - start_tick as i64 - tick as i64;
                match offset {
                    None => offset = Some(off),
                    Some(o) if o != off => {
                        return Err(format!("播放：第 {at} 筆實錄 tick 偏移 {off}，開頭是 {o}（模型 tick {tick}）"));
                    }
                    _ => {}
                }
                check(real, &mut at, "播放", &got)?;
            }
            if tick > 4_000_000 {
                return Err(format!("播放：第 {at} 筆跑太久"));
            }
        }
    }
    Ok(segs)
}
