//! 原版圖模式的索引色引擎。
//!
//! 記憶體模型：和 `VgaSink` 相同的 64 KB 位址空間，但每個像素一個位元組（色號 0 至 15），
//! 所以共 `65536 * 8` 個像素，位元組位址 `a` 的第 `b` 個位元（最高位在左）是像素 `a * 8 + b`。
//! 四個 VGA 平面合成一個色號：平面 `p` 是色號的第 `p` 位元。
//!
//! 每個函式對應 `sink.rs` 的同名函式，語意不改，只把「位元組」換成「8 個像素」，
//! 並允許目的、來源與列數帶像素單位的偏移（`Off`），補間才有非 8 倍數的位置。
//! 偏移為 0 時結果與 `VgaSink` 逐像素相同，由 `rs2-trace` 的 `layer.rs` 測試核對。

use super::Off;
use crate::assets::Assets;
use crate::font::DIGITS;
use crate::sink::{disp_row, scratch_row, scratch_src};
use rs2_core::{BlitCmd, Op, ShtGroup};
use rs2_formats::{BankItem, GoFile};

/// 像素總數：64 KB 位址空間，每位元組 8 個像素。
pub const MEM_PX: usize = 0x10000 * 8;
const MASK: usize = MEM_PX - 1;

#[derive(Clone)]
pub struct Mem {
    pub px: Vec<u8>,
}

impl Mem {
    pub fn new() -> Mem {
        Mem { px: vec![0; MEM_PX] }
    }
}

impl Default for Mem {
    fn default() -> Self {
        Mem::new()
    }
}

/// 跨命令的暫存變數。
#[derive(Clone, Copy, Default)]
pub struct St {
    /// `word_2687E`，見 `sink.rs` 的 `State::stride_reg`。
    pub stride_reg: u16,
    pub anomalies: u64,
}

struct Rows {
    bx: u16,
    cx: u16,
    si: u16,
    dx: u16,
    di: u16,
}

/// `sub_19683`，同 `sink.rs` 的 `prep`。
fn prep(st: &St, y: u16, mut cx: u16, mut si: u16, page: u16, x: u16) -> Rows {
    let mut bx = y;
    while (bx as i16) < 0 {
        bx = bx.wrapping_add(1);
        cx = cx.wrapping_sub(1);
        si = si.wrapping_add(st.stride_reg);
    }
    let dx = disp_row(bx).wrapping_add(page);
    let di = dx.wrapping_add(x);
    if (cx as i16) <= 0 {
        cx = 1;
        bx = 0x12C;
    }
    Rows { bx, cx, si, dx, di }
}

#[derive(Clone, Copy)]
enum Put {
    Mov,
    Or,
    MaskAnd,
    MaskInv,
}

#[inline]
fn yoff(y: u16, off: i32) -> u16 {
    (y as i16 as i32 + off) as u16
}

/// 圖庫列迴圈，對應 `sink.rs` 的 `put_rows`。`plane` 是 VGA 平面編號，遮罩忽略它。
fn put_rows(m: &mut Mem, st: &mut St, it: &BankItem, c: &BlitCmd, put: Put, plane: u8, off: Off) {
    let stride = it.stride;
    st.stride_reg = stride;
    if stride == 0 {
        st.anomalies += 1;
        return;
    }
    let r = prep(st, yoff(c.y, off.oy), it.height, 0, c.page, c.x);
    let x_px = c.x as i16 as i32 * 8 + off.ox;
    let (mut bx, mut si, mut dx) = (r.bx, r.si, r.dx);
    for _ in 0..r.cx {
        if (bx as i16) <= 0xBD {
            let base = dx as usize * 8;
            for col in 0..stride as usize {
                let b = match it.data.get(si as usize + col) {
                    Some(&b) => b,
                    None => {
                        st.anomalies += 1;
                        0
                    }
                };
                for bit in 0..8i32 {
                    let cp = x_px + col as i32 * 8 + bit;
                    if !(0..320).contains(&cp) {
                        continue;
                    }
                    let v = (b >> (7 - bit)) & 1;
                    let p = &mut m.px[(base + cp as usize) & MASK];
                    match put {
                        Put::Mov => *p = (*p & !(1 << plane)) | (v << plane),
                        Put::Or => *p |= v << plane,
                        Put::MaskAnd => {
                            if v == 0 {
                                *p = 0;
                            }
                        }
                        Put::MaskInv => {
                            if v == 0 {
                                *p = 15;
                            }
                        }
                    }
                }
            }
            si = si.wrapping_add(stride);
        }
        bx = bx.wrapping_add(1);
        dx = dx.wrapping_add(0x28);
    }
}

fn planes4(m: &mut Mem, st: &mut St, go: &GoFile, c: &BlitCmd, put: Put, off: Off) {
    for k in 0..4 {
        match go.planes[k].get(c.index as usize) {
            Some(it) => put_rows(m, st, it, c, put, 3 - k as u8, off),
            None => st.anomalies += 1,
        }
    }
}

fn mask_op(m: &mut Mem, st: &mut St, mask: Option<&[BankItem]>, c: &BlitCmd, put: Put, off: Off) {
    match mask.and_then(|v| v.get(c.index as usize)) {
        Some(it) => put_rows(m, st, it, c, put, 0, off),
        None => st.anomalies += 1,
    }
}

#[inline]
fn set_plane_byte(m: &mut Mem, a: u16, plane: u8, b: u8) {
    let base = a as usize * 8;
    for bit in 0..8 {
        let v = (b >> (7 - bit)) & 1;
        let p = &mut m.px[base + bit];
        *p = (*p & !(1 << plane)) | (v << plane);
    }
}

/// `sub_1A4F1`，對應 `scratch_rows`：奇數寬度丟掉最後一個位元組，不裁切。
fn scratch_rows(m: &mut Mem, st: &mut St, it: &BankItem, c: &BlitCmd, plane: u8) {
    let stride = it.stride;
    st.stride_reg = stride;
    let mut di = scratch_row(c.y).wrapping_add(c.scratch).wrapping_add(c.x);
    let mut si = 0usize;
    let words = (stride >> 1) as usize;
    for _ in 0..it.height {
        for i in 0..words {
            let lo = it.data.get(si + 2 * i).copied();
            let hi = it.data.get(si + 2 * i + 1).copied();
            if lo.is_none() || hi.is_none() {
                st.anomalies += 1;
            }
            let o = di.wrapping_add(2 * i as u16);
            set_plane_byte(m, o, plane, lo.unwrap_or(0));
            set_plane_byte(m, o.wrapping_add(1), plane, hi.unwrap_or(0));
        }
        si += 2 * words;
        di = di.wrapping_add(0x50);
    }
}

fn scratch4(m: &mut Mem, st: &mut St, go: &GoFile, c: &BlitCmd) {
    for k in 0..4 {
        match go.planes[k].get(c.index as usize) {
            Some(it) => scratch_rows(m, st, it, c, 3 - k as u8),
            None => st.anomalies += 1,
        }
    }
}

/// 一列 320 個像素從 `src0` 複製到 `dst0`（像素位址，模 `MEM_PX`）。
#[inline]
fn copy_run(m: &mut Mem, dst0: i64, src0: i64, valid: impl Fn(i32) -> bool) {
    for k in 0..320i32 {
        if !valid(k) {
            continue;
        }
        let s = (src0 + k as i64) as usize & MASK;
        let d = (dst0 + k as i64) as usize & MASK;
        m.px[d] = m.px[s];
    }
}

/// `sub_19BA4`（`shifted = false`）與 `sub_19D08`（`shifted = true`，`sub_1A2D2`）。
/// 位移版每寫一個位元組計數加 1，計數 >= 50h（含負數）的位元組不寫，換成像素就是
/// `shift * 8 + sox + k` 落在 `[0, 640)` 才寫。
fn copy_rows(m: &mut Mem, st: &mut St, c: &BlitCmd, off: Off, shifted: bool) {
    if shifted {
        st.stride_reg = 0x50;
    }
    let r = prep(st, yoff(c.y, off.oy), c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut di) = (r.bx, r.si, r.di);
    let shift_px = c.shift as i16 as i32 * 8 + off.sox;
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        let dst0 = di as i64 * 8 + off.ox as i64;
        let src0 = si as i64 * 8 + off.sox as i64;
        if shifted {
            copy_run(m, dst0, src0, |k| (0..640).contains(&(shift_px + k)));
        } else {
            copy_run(m, dst0, src0, |_| true);
        }
        di = di.wrapping_add(0x28);
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
}

fn clear_page(m: &mut Mem, c: &BlitCmd) {
    let start = c.page as usize * 8;
    for i in 0..0x0ED8usize * 2 * 8 {
        m.px[(start + i) & MASK] = 0;
    }
}

/// `sub_19DA8`：四個平面各 OR 一次，等於整個色號 OR。
fn or_rows(m: &mut Mem, st: &mut St, c: &BlitCmd, off: Off) {
    let r = prep(st, yoff(c.y, off.oy), c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut di) = (r.bx, r.si, r.di);
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        let dst0 = di as i64 * 8 + off.ox as i64;
        let src0 = si as i64 * 8 + off.sox as i64;
        for k in 0..320i64 {
            let s = (src0 + k) as usize & MASK;
            let d = (dst0 + k) as usize & MASK;
            m.px[d] |= m.px[s];
        }
        di = di.wrapping_add(0x28);
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
}

/// `sub_1A09F`：來源只取平面 0（色號的最低位元），為 0 的像素把目的清成 0。
/// 平面 0 來自 GC 讀取平面的殘值，`sink.rs` 的 `and_rows` 有說明。
fn and_rows(m: &mut Mem, st: &mut St, c: &BlitCmd, off: Off) {
    let r = prep(st, yoff(c.y, off.oy), c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut di) = (r.bx, r.si, r.di);
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        let dst0 = di as i64 * 8 + off.ox as i64;
        let src0 = si as i64 * 8 + off.sox as i64;
        for k in 0..320i64 {
            let s = (src0 + k) as usize & MASK;
            let d = (dst0 + k) as usize & MASK;
            if m.px[s] & 1 == 0 {
                m.px[d] = 0;
            }
        }
        di = di.wrapping_add(0x28);
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
}

/// `sub_1A287`：填實色，從 `di` 起連續 `cx * 40` 位元組。`off.ocx` 是列數偏移（補間用）。
fn fill_rows(m: &mut Mem, st: &mut St, c: &BlitCmd, off: Off) {
    let cx = (c.cx as i16 as i32 + off.ocx) as u16;
    let r = prep(st, yoff(c.y, off.oy), cx, 0, c.page, c.x);
    let (mut bx, mut di) = (r.bx, r.di);
    let color = (c.dx & 0x0F) as u8;
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        let dst0 = di as i64 * 8 + off.ox as i64;
        for k in 0..320i64 {
            m.px[(dst0 + k) as usize & MASK] = color;
        }
        di = di.wrapping_add(0x28);
        bx = bx.wrapping_add(1);
    }
}

/// `sub_1929F`：血條的一格，7 列各 8 個像素，兩頁同寫。
fn bar_cell(m: &mut Mem, st: &mut St, c: &BlitCmd) {
    st.stride_reg = 1;
    let r = prep(st, c.y, c.cx, 0, c.page, c.x);
    let mut di = r.di;
    let color = (c.dx & 0x0F) as u8;
    for _ in 0..7 {
        for bit in 0..8usize {
            m.px[(di as usize * 8 + bit) & MASK] = color;
            m.px[((di ^ 0x8000) as usize * 8 + bit) & MASK] = color;
        }
        di = di.wrapping_add(0x28);
    }
}

/// `sub_1152D`：從 `A800:1F40` 起填 `20BFh` 個字組的 0。
fn clear_scratch_b(m: &mut Mem) {
    let start = 0x8000u16.wrapping_add(0x1F40) as usize * 8;
    for i in 0..0x20BFusize * 2 * 8 {
        m.px[(start + i) & MASK] = 0;
    }
}

/// `sub_1890D`：5 位數字寫進平面 3（色號第 3 位元），兩頁同寫。
pub fn counter(m: &mut Mem, col: u16, row: u16, value: u16) {
    let mut v = value;
    let mut digits = [0u8; 5];
    for (i, d) in [10000u16, 1000, 100, 10, 1].into_iter().enumerate() {
        digits[i] = (v / d) as u8;
        v %= d;
    }
    for (i, &dg) in digits.iter().enumerate() {
        let glyph = DIGITS[(dg % 10) as usize];
        for (r, &bits) in glyph.iter().enumerate() {
            let off = disp_row(row + r as u16).wrapping_add(col + i as u16);
            for o in [off, off.wrapping_sub(0x8000)] {
                set_plane_byte(m, o, 3, bits);
            }
        }
    }
}

/// 套用一個 blit 命令。`S1890D` 的幀計數位置不在這裡處理（由 `LayerSink` 在輸出時疊上）。
pub fn apply(m: &mut Mem, st: &mut St, assets: &Assets, c: &BlitCmd, off: Off) {
    let sht = |g: ShtGroup| -> &GoFile {
        match g {
            ShtGroup::Bh1 => &assets.bh1,
            ShtGroup::Sht1 => &assets.sht1,
        }
    };
    match c.op {
        Op::S18C0A => planes4(m, st, &assets.my, c, Put::Mov, off),
        Op::S18CE2 | Op::S18EB6 | Op::S18DCC => planes4(m, st, &assets.my, c, Put::Or, off),
        Op::S18D46 | Op::S18E30 | Op::S18E73 => mask_op(m, st, assets.my.mask.as_deref(), c, Put::MaskAnd, off),
        Op::S18D89 => mask_op(m, st, assets.my.mask.as_deref(), c, Put::MaskInv, off),
        Op::S18F1A => mask_op(m, st, assets.sht1.mask.as_deref(), c, Put::MaskAnd, off),
        Op::S19025 => mask_op(m, st, assets.sht1.mask.as_deref(), c, Put::MaskInv, off),
        Op::S18F5D => planes4(m, st, sht(c.sht_group), c, Put::Or, off),
        Op::S18FC1 => planes4(m, st, sht(c.sht_group), c, Put::Mov, off),
        Op::S19ADC => scratch4(m, st, sht(c.sht_group), c),
        Op::S19BA4 => copy_rows(m, st, c, off, false),
        Op::S19D08 => copy_rows(m, st, c, off, true),
        Op::S19CF9 => clear_page(m, c),
        Op::S19DA8 => or_rows(m, st, c, off),
        Op::S1A09F => and_rows(m, st, c, off),
        Op::S1A287 => fill_rows(m, st, c, off),
        Op::S1929F => bar_cell(m, st, c),
        Op::S1152D => clear_scratch_b(m),
        Op::S1890D => counter(m, c.text_x, c.text_y, c.cx),
    }
}

/// 顯示頁（位元組偏移 `page`）轉 RGBA。`colors` 是色號 0 至 15 的 8 位元 RGB。
pub fn page_to_rgba(m: &Mem, page: u16, colors: &[[u8; 3]; 16], out: &mut [u8]) {
    let base = page as usize * 8;
    for i in 0..320 * 200 {
        let c = colors[(m.px[(base + i) & MASK] & 15) as usize];
        out[4 * i] = c[0];
        out[4 * i + 1] = c[1];
        out[4 * i + 2] = c[2];
        out[4 * i + 3] = 255;
    }
}
