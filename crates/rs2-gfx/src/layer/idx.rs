//! 原版圖模式的索引色引擎。
//!
//! 記憶體模型：和 `VgaSink` 相同的 64 KB 位址空間，但每個像素一個位元組（色號 0 至 15），
//! 所以共 `65536 * 8` 個像素，位元組位址 `a` 的第 `b` 個位元（最高位在左）是像素 `a * 8 + b`。
//! 四個 VGA 平面合成一個色號：平面 `p` 是色號的第 `p` 位元。
//!
//! 每個函式對應 `sink.rs` 的同名函式，語意不改，只把「位元組」換成「8 個像素」，
//! 並允許目的、來源與列數帶像素單位的偏移（`Off`），補間才有非 8 倍數的位置。
//! 偏移為 0 時結果與 `VgaSink` 逐像素相同，由 `rs2-trace` 的 `layer.rs` 測試與 `tests/layer_units.rs` 的差分測試核對。
//!
//! 序列器 map mask 與 GC 讀取平面記在 `St`：`VgaSink` 的函式依「目前」的 map mask 決定寫入哪些平面
//! （閂鎖複製、填色、AND 遮罩、清暫存區），所以這裡也要把它當狀態，`DrawCmd::SetMapMask` 才有效果。

use super::Off;
use crate::assets::Assets;
use crate::font::DIGITS;
use crate::sink::{disp_row, scratch_row, scratch_src};
use rs2_game::{BlitCmd, Op};
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

impl Mem {
    /// 四個 VGA 平面（各 64 KB）合成索引色：平面 `p` 是色號的第 `p` 位元，位元組位址 `a` 的第 `b` 個位元（最高位在左）
    /// 是像素 `a * 8 + b`。`VgaSink::load_snapshot` 的對應。
    pub fn load_planes(&mut self, planes: [&[u8]; 4]) {
        for (p, data) in planes.iter().enumerate() {
            assert_eq!(data.len(), 0x10000, "平面 {p} 不是 64 KB（{} 位元組）", data.len());
        }
        for a in 0..0x10000usize {
            let b = [planes[0][a], planes[1][a], planes[2][a], planes[3][a]];
            for bit in 0..8usize {
                let sh = 7 - bit;
                self.px[a * 8 + bit] = ((b[0] >> sh) & 1) | (((b[1] >> sh) & 1) << 1) | (((b[2] >> sh) & 1) << 2) | (((b[3] >> sh) & 1) << 3);
            }
        }
    }
}

impl Default for Mem {
    fn default() -> Self {
        Mem::new()
    }
}

/// 跨命令的暫存變數。
#[derive(Clone, Copy)]
pub struct St {
    /// `word_2687E`，見 `sink.rs` 的 `State::stride_reg`。
    pub stride_reg: u16,
    pub anomalies: u64,
    /// 序列器索引 2（寫入的平面），預設 0Fh。
    pub map_mask: u8,
    /// GC 索引 4（讀取平面），預設 0。
    pub read_map: u8,
    /// GC 索引 3 的資料旋轉。只有第三關捲動的 `ScrollRotateCopy`、`ScrollRotateColumn` 會設成 4，緊接的 `ScrollShiftLeft4`、`ScrollMergeSeam` 設回 0。
    /// 索引色引擎只在這四個命令內套用旋轉；旋轉還在時執行別的命令，原版的寫入資料會被旋轉，這裡不模擬，
    /// 只把 `anomalies` 加 1（原版的呼叫序列不會發生）。
    pub rotate: u8,
}

impl Default for St {
    fn default() -> Self {
        St { stride_reg: 0, anomalies: 0, map_mask: 0x0F, read_map: 0, rotate: 0 }
    }
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
    /// `loc_19534`，GC 函式為取代：資料取反後寫進一個平面。
    MovInv,
    MaskAnd,
    MaskInv,
}

#[inline]
fn yoff(y: u16, off: i32) -> u16 {
    (y as i16 as i32 + off) as u16
}

/// `sel_plane`：`sub_1882E` 選圖庫第 `k` 個平面區段，對應 VGA 平面 `3 - k`。
#[inline]
fn sel_plane(st: &mut St, k: usize) {
    st.read_map = (3 - k) as u8;
    st.map_mask = 1 << (3 - k);
}

/// `sub_18878`：map mask 回到 0Fh。
#[inline]
fn restore(st: &mut St) {
    st.map_mask = 0x0F;
}

/// 平面 `pm` 的位元換成 `v` 的對應位元。
#[inline]
fn set_bits(p: &mut u8, v: u8, pm: u8) {
    *p = (*p & !pm) | (v & pm);
}

/// 圖庫列迴圈，對應 `sink.rs` 的 `put_rows`。`plane` 是 VGA 平面編號（遮罩忽略它），`pm` 是遮罩操作的 map mask。
fn put_rows(m: &mut Mem, st: &mut St, it: &BankItem, c: &BlitCmd, put: Put, plane: u8, pm: u8, off: Off) {
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
                        Put::MovInv => *p = (*p & !(1 << plane)) | ((v ^ 1) << plane),
                        Put::MaskAnd => {
                            if v == 0 {
                                *p &= !pm;
                            }
                        }
                        Put::MaskInv => {
                            if v == 0 {
                                *p |= pm;
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
        sel_plane(st, k);
        match go.planes[k].get(c.index as usize) {
            Some(it) => put_rows(m, st, it, c, put, 3 - k as u8, 1 << (3 - k), off),
            None => st.anomalies += 1,
        }
    }
    restore(st);
}

fn mask_op(m: &mut Mem, st: &mut St, mask: Option<&[BankItem]>, c: &BlitCmd, put: Put, off: Off) {
    let pm = st.map_mask;
    match mask.and_then(|v| v.get(c.index as usize)) {
        Some(it) => put_rows(m, st, it, c, put, 0, pm, off),
        None => st.anomalies += 1,
    }
    restore(st);
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

/// 一個位元組（8 個像素）寫進 `pm` 內的平面：每個平面的值都是 `b`。
#[inline]
fn put_byte(m: &mut Mem, a: u16, b: u8, pm: u8) {
    let base = a as usize * 8;
    for bit in 0..8 {
        let v = if (b >> (7 - bit)) & 1 != 0 { pm } else { 0 };
        set_bits(&mut m.px[base + bit], v, pm);
    }
}

/// `rep stosw`：從 `off` 起寫 `words` 個字組，低位元組在前，寫進 `pm` 內的平面。
fn fill_words(m: &mut Mem, off: u16, words: u16, value: u16, pm: u8) {
    if value == 0 && pm == 0x0F {
        let start = off as usize * 8;
        for i in 0..words as usize * 2 * 8 {
            m.px[(start + i) & MASK] = 0;
        }
        return;
    }
    let (lo, hi) = (value as u8, (value >> 8) as u8);
    let mut di = off;
    for _ in 0..words {
        put_byte(m, di, lo, pm);
        put_byte(m, di.wrapping_add(1), hi, pm);
        di = di.wrapping_add(2);
    }
}

/// `DrawCmd::VramFill`，同 `sink.rs` 的 `vram_fill`。
pub fn vram_fill(m: &mut Mem, st: &St, off: u16, words: u16, value: u16) {
    fill_words(m, off, words, value, st.map_mask);
}

/// `DrawCmd::SetMapMask`，同 `sink.rs` 的 `set_map_mask`。
pub fn set_map_mask(st: &mut St, mask: u8) {
    let m = mask & 0x0F;
    match m {
        8 => st.read_map = 3,
        4 => st.read_map = 2,
        2 => st.read_map = 1,
        1 => st.read_map = 0,
        _ => {}
    }
    st.map_mask = m;
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
        sel_plane(st, k);
        match go.planes[k].get(c.index as usize) {
            Some(it) => scratch_rows(m, st, it, c, 3 - k as u8),
            None => st.anomalies += 1,
        }
    }
    restore(st);
}

/// `sub_18ACE`（`sub_196E5`），對應 `map_tiles`：10 列、每列 1 位元組，目的每列加 50h。
fn map_tiles(m: &mut Mem, st: &mut St, go: &GoFile, c: &BlitCmd) {
    for k in 0..4 {
        sel_plane(st, k);
        match go.planes[k].get(c.index as usize) {
            Some(it) => {
                st.stride_reg = it.stride;
                let mut di = scratch_row(c.y).wrapping_add(c.scratch).wrapping_add(c.x);
                for row in 0..10usize {
                    let b = match it.data.get(row) {
                        Some(&b) => b,
                        None => {
                            st.anomalies += 1;
                            0
                        }
                    };
                    set_plane_byte(m, di, 3 - k as u8, b);
                    di = di.wrapping_add(0x50);
                }
            }
            None => st.anomalies += 1,
        }
    }
    restore(st);
}

/// 一列 320 個像素從 `src0` 複製到 `dst0`（像素位址，模 `MEM_PX`），只改 `pm` 內的平面（閂鎖複製）。
#[inline]
fn copy_run(m: &mut Mem, dst0: i64, src0: i64, valid: impl Fn(i32) -> bool, pm: u8) {
    for k in 0..320i32 {
        if !valid(k) {
            continue;
        }
        let s = (src0 + k as i64) as usize & MASK;
        let d = (dst0 + k as i64) as usize & MASK;
        let sv = m.px[s];
        set_bits(&mut m.px[d], sv, pm);
    }
}

/// `sub_19BA4`（`shifted = false`）與 `sub_19D08`（`shifted = true`，`sub_1A2D2`）。
/// 位移版每寫一個位元組計數加 1，計數 >= 50h（含負數）的位元組不寫，換成像素就是
/// `shift * 8 + sox + k` 落在 `[0, 640)` 才寫。
fn copy_rows(m: &mut Mem, st: &mut St, c: &BlitCmd, off: Off, shifted: bool) {
    if shifted {
        st.stride_reg = 0x50;
    }
    let pm = st.map_mask;
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
            copy_run(m, dst0, src0, |k| (0..640).contains(&(shift_px + k)), pm);
        } else {
            copy_run(m, dst0, src0, |_| true, pm);
        }
        di = di.wrapping_add(0x28);
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
}

/// `sub_19C4F`：垂直鏡射的 `sub_19BA4`。目的列遞減，列號（有號）小於 0 才停，沒有 189 上限。
fn copy_rows_mirror(m: &mut Mem, st: &mut St, c: &BlitCmd) {
    let pm = st.map_mask;
    let r = prep(st, c.y, c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut di) = (r.bx, r.si, r.di);
    for _ in 0..r.cx {
        if (bx as i16) < 0 {
            break;
        }
        copy_run(m, di as i64 * 8, si as i64 * 8, |_| true, pm);
        di = di.wrapping_sub(0x28);
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_sub(1);
    }
}

/// `sub_19FF9`（`sub_1A41D`）：每列閂鎖複製 2 位元組，來源指標每列前進「實際讀的次數 + 4Eh」。
fn latch2_rows(m: &mut Mem, st: &mut St, c: &BlitCmd) {
    let pm = st.map_mask;
    let r = prep(st, c.y, c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut dx, mut di) = (r.bx, r.si, r.dx, r.di);
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        let lim = dx.wrapping_add(0x27);
        let mut d = di;
        for _ in 0..2 {
            let s = si;
            si = si.wrapping_add(1);
            if d > lim {
                break;
            }
            for k in 0..8usize {
                let sv = m.px[(s as usize * 8 + k) & MASK];
                set_bits(&mut m.px[(d as usize * 8 + k) & MASK], sv, pm);
            }
            d = d.wrapping_add(1);
        }
        si = si.wrapping_add(0x4E);
        dx = dx.wrapping_add(0x28);
        di = di.wrapping_add(0x28);
        bx = bx.wrapping_add(1);
    }
}

fn clear_page(m: &mut Mem, st: &mut St, c: &BlitCmd) {
    restore(st);
    fill_words(m, c.page, 0x0ED8, 0, 0x0F);
}

/// `sub_19DA8`：四個平面各 OR 一次，等於整個色號 OR。`mirror` 是 `sub_19F3B`，目的列遞減。
fn or_rows(m: &mut Mem, st: &mut St, c: &BlitCmd, off: Off, mirror: bool) {
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
        di = if mirror { di.wrapping_sub(0x28) } else { di.wrapping_add(0x28) };
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
    // 逐平面選取，最後一次是平面 0。
    sel_plane(st, 3);
    restore(st);
}

/// `sub_19E66`（`sub_1A453`）：每列只 OR 一個字組（16 像素）。字組起點大於列起點 + 27h 就不寫，沒有下界。
fn or_word_rows(m: &mut Mem, st: &mut St, c: &BlitCmd) {
    let r = prep(st, c.y, c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut dx, mut di) = (r.bx, r.si, r.dx, r.di);
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        if di <= dx.wrapping_add(0x27) {
            let mut s = [0u8; 16];
            for (k, v) in s.iter_mut().enumerate() {
                *v = m.px[(si as usize * 8 + k) & MASK];
            }
            for (k, v) in s.iter().enumerate() {
                m.px[(di as usize * 8 + k) & MASK] |= *v;
            }
        }
        dx = dx.wrapping_add(0x28);
        di = di.wrapping_add(0x28);
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
    sel_plane(st, 3);
    restore(st);
}

/// `sub_1A09F`：來源只取讀取平面（預設平面 0，色號的最低位元），為 0 的像素把目的清成 0。
/// 讀取平面來自 GC 索引 4 的殘值，`sink.rs` 的 `and_rows` 有說明。`mirror` 是 `sub_1A1E6`，目的列遞減。
fn and_rows(m: &mut Mem, st: &mut St, c: &BlitCmd, off: Off, mirror: bool) {
    let (pm, rd) = (st.map_mask, st.read_map);
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
            if (m.px[s] >> rd) & 1 == 0 {
                m.px[d] &= !pm;
            }
        }
        di = if mirror { di.wrapping_sub(0x28) } else { di.wrapping_add(0x28) };
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
    restore(st);
}

/// `sub_1A144`（`sub_1A48F`）：每列只取 2 位元組的 AND 遮罩。
fn and2_rows(m: &mut Mem, st: &mut St, c: &BlitCmd) {
    let (pm, rd) = (st.map_mask, st.read_map);
    let r = prep(st, c.y, c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut dx, mut di) = (r.bx, r.si, r.dx, r.di);
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        let lim = dx.wrapping_add(0x27);
        let mut d = di;
        for i in 0..2u16 {
            if d > lim {
                break;
            }
            let s = si.wrapping_add(i);
            for k in 0..8usize {
                if (m.px[(s as usize * 8 + k) & MASK] >> rd) & 1 == 0 {
                    m.px[(d as usize * 8 + k) & MASK] &= !pm;
                }
            }
            d = d.wrapping_add(1);
        }
        dx = dx.wrapping_add(0x28);
        di = di.wrapping_add(0x28);
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
    restore(st);
}

/// `sub_1A287`：填實色，從 `di` 起連續 `cx * 40` 位元組。`off.ocx` 是列數偏移（補間用）。
fn fill_rows(m: &mut Mem, st: &mut St, c: &BlitCmd, off: Off) {
    let cx = (c.cx as i16 as i32 + off.ocx) as u16;
    let pm = st.map_mask;
    let r = prep(st, yoff(c.y, off.oy), cx, 0, c.page, c.x);
    let (mut bx, mut di) = (r.bx, r.di);
    let color = (c.dx & 0x0F) as u8;
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        let dst0 = di as i64 * 8 + off.ox as i64;
        for k in 0..320i64 {
            set_bits(&mut m.px[(dst0 + k) as usize & MASK], color, pm);
        }
        di = di.wrapping_add(0x28);
        bx = bx.wrapping_add(1);
    }
}

/// `sub_1929F`：血條的一格，7 列各 8 個像素，兩頁同寫。
fn bar_cell(m: &mut Mem, st: &mut St, c: &BlitCmd) {
    st.stride_reg = 1;
    let pm = st.map_mask;
    let r = prep(st, c.y, c.cx, 0, c.page, c.x);
    let mut di = r.di;
    let color = (c.dx & 0x0F) as u8;
    for _ in 0..7 {
        for bit in 0..8usize {
            set_bits(&mut m.px[(di as usize * 8 + bit) & MASK], color, pm);
            set_bits(&mut m.px[((di ^ 0x8000) as usize * 8 + bit) & MASK], color, pm);
        }
        di = di.wrapping_add(0x28);
    }
}

/// `sub_11504`、`sub_1152D`：從 `base + 1F40h` 起填 `20BFh` 個字組的 0，寫進目前 map mask 的平面。
fn clear_scratch(m: &mut Mem, st: &St, base: u16) {
    fill_words(m, base.wrapping_add(0x1F40), 0x20BF, 0, st.map_mask);
}

/// `sub_18894`：map mask 0Fh，兩頁各清 `0FA0h` 個字組。
fn clear_both_pages(m: &mut Mem, st: &mut St) {
    restore(st);
    fill_words(m, 0, 0x0FA0, 0, 0x0F);
    fill_words(m, 0x8000, 0x0FA0, 0, 0x0F);
}

/// 位元組 `a` 在平面 `p` 的 8 個位元（最高位在左）。
#[inline]
fn plane_byte(m: &Mem, a: u16, p: u8) -> u8 {
    let base = a as usize * 8;
    let mut b = 0u8;
    for bit in 0..8 {
        b = (b << 1) | ((m.px[base + bit] >> p) & 1);
    }
    b
}

/// 位元組 `a` 的像素 4 至 7（位元 3 至 0）換成 `al` 的低半位元組，只改 `pm` 內的平面（位元遮罩 0Fh 的寫入）。
#[inline]
fn put_low_nibble(m: &mut Mem, a: u16, al: u8, pm: u8) {
    let base = a as usize * 8;
    for bit in 4..8 {
        let v = if (al >> (7 - bit)) & 1 != 0 { pm } else { 0 };
        set_bits(&mut m.px[base + bit], v, pm);
    }
}

/// 第三關捲動的來源起點，同 `sink.rs` 的 `scroll_src`。
fn scroll_src(c: &BlitCmd) -> u16 {
    0x1F40u16.wrapping_add(c.scratch).wrapping_add(c.x)
}

/// `sub_17EBA`：A 的 3E80h 位元組各旋轉 4 位元（高低半位元組對調）寫進 B，讀取平面讀、`pm` 內的平面寫。
fn scroll_rot_copy(m: &mut Mem, st: &mut St, c: &BlitCmd) {
    let (rd, pm) = (st.read_map, st.map_mask);
    let src = scroll_src(c);
    let dst = src.wrapping_add(0x8000);
    for i in 0..0x3E80u16 {
        let b = plane_byte(m, src.wrapping_add(i), rd).rotate_right(4);
        put_byte(m, dst.wrapping_add(i), b, pm);
    }
    st.rotate = 4;
}

/// `sub_17EEA`：B 的每個位元組，像素 4 至 7 換成下一個位元組（讀取平面）的低半位元組，3E80h 次。
fn scroll_shift_merge(m: &mut Mem, st: &mut St, c: &BlitCmd) {
    let (rd, pm) = (st.read_map, st.map_mask);
    let mut di = scroll_src(c).wrapping_add(0x8000);
    let mut si = di.wrapping_add(1);
    for _ in 0..0x3E80u16 {
        let al = plane_byte(m, si, rd);
        put_low_nibble(m, di, al, pm);
        si = si.wrapping_add(1);
        di = di.wrapping_add(1);
    }
    st.rotate = 0;
}

/// `sub_17F26`：`scroll_rot_copy` 的一欄版本，200 列，列距 80。
fn scroll_col_rot(m: &mut Mem, st: &mut St, c: &BlitCmd) {
    let (rd, pm) = (st.read_map, st.map_mask);
    let mut si = scroll_src(c);
    let mut di = si.wrapping_add(0x8000);
    for _ in 0..0xC8 {
        let b = plane_byte(m, si, rd).rotate_right(4);
        put_byte(m, di, b, pm);
        si = si.wrapping_add(0x50);
        di = di.wrapping_add(0x50);
    }
    st.rotate = 4;
}

/// `sub_17F5D`：`scroll_shift_merge` 的一欄版本，目的是 B 的左邊一欄，200 列，列距 80。
fn scroll_col_merge(m: &mut Mem, st: &mut St, c: &BlitCmd) {
    let (rd, pm) = (st.read_map, st.map_mask);
    let mut di = scroll_src(c).wrapping_add(0x8000).wrapping_sub(1);
    let mut si = di.wrapping_add(1);
    for _ in 0..0xC8 {
        let al = plane_byte(m, si, rd);
        put_low_nibble(m, di, al, pm);
        si = si.wrapping_add(0x50);
        di = di.wrapping_add(0x50);
    }
    st.rotate = 0;
}

/// `sub_17FA0`、`sub_18031`：暫存區 190 列各 40 位元組閂鎖複製到顯示頁（`c.page` 起連續）。
fn scroll_view(m: &mut Mem, st: &St, c: &BlitCmd) {
    let pm = st.map_mask;
    let mut si = scroll_src(c);
    let mut di = c.page;
    for _ in 0..0xBE {
        copy_run(m, di as i64 * 8, si as i64 * 8, |_| true, pm);
        si = si.wrapping_add(0x50);
        di = di.wrapping_add(0x28);
    }
}

/// `sub_180C6` 進位時的搬移：A、B 各整體左移 28h 位元組（順向逐位元組閂鎖複製 3B60h 位元組）。
fn scroll_left(m: &mut Mem, st: &St) {
    let pm = st.map_mask;
    for base in [0x1F40u16, 0x1F40u16.wrapping_add(0x8000)] {
        for i in 0..0x3B60u16 {
            let (s, d) = (base.wrapping_add(0x28).wrapping_add(i) as usize * 8, base.wrapping_add(i) as usize * 8);
            for k in 0..8usize {
                let sv = m.px[s + k];
                set_bits(&mut m.px[d + k], sv, pm);
            }
        }
    }
}

/// `sub_1890D`：5 位數字寫進平面 3（色號第 3 位元），兩頁同寫。`font` 是 2048 位元組的 BIOS 字型時
/// 照 `sub_18A61` 讀字元的 8 列（同 `sink.rs` 的 `counter`），否則用自繪數字。
pub fn counter(m: &mut Mem, col: u16, row: u16, value: u16, font: Option<&[u8]>) {
    let mut v = value;
    let mut digits = [0u8; 5];
    for (i, d) in [10000u16, 1000, 100, 10, 1].into_iter().enumerate() {
        digits[i] = (v / d) as u8;
        v %= d;
    }
    for (i, &dg) in digits.iter().enumerate() {
        let glyph: [u8; 8] = match font {
            Some(f) => {
                let at = (0x30 + dg as usize) * 8;
                std::array::from_fn(|r| f[at + r])
            }
            None => DIGITS[(dg % 10) as usize],
        };
        for (r, &bits) in glyph.iter().enumerate() {
            let off = disp_row(row + r as u16).wrapping_add(col + i as u16);
            for o in [off, off.wrapping_sub(0x8000)] {
                set_plane_byte(m, o, 3, bits);
            }
        }
    }
}

/// 套用一個 blit 命令。`Number` 的幀計數位置不在這裡處理（由 `LayerSink` 在輸出時疊上）。
pub fn apply(m: &mut Mem, st: &mut St, assets: &Assets, font: Option<&[u8]>, c: &BlitCmd, off: Off) {
    if let Some(v) = c.stride_reg {
        st.stride_reg = v;
    }
    let sht = assets.sht(c.sht_group);
    let sht_mask = assets.sht_mask(c.mask_group);
    let beg = assets.beg(c.beg_group);
    if st.rotate != 0 && !matches!(c.op, Op::ScrollRotateCopy | Op::ScrollShiftLeft4 | Op::ScrollRotateColumn | Op::ScrollMergeSeam) {
        st.anomalies += 1;
    }
    match c.op {
        Op::MyOverwrite => planes4(m, st, &assets.my, c, Put::Mov, off),
        Op::MyOverlay | Op::MyOverlayPlayerShot | Op::MyOverlayEffect => planes4(m, st, &assets.my, c, Put::Or, off),
        Op::MyOverwriteInverted => planes4(m, st, &assets.my, c, Put::MovInv, off),
        Op::MyCut | Op::MyCutEffect | Op::MyCutPlayerShot => mask_op(m, st, assets.my.mask.as_deref(), c, Put::MaskAnd, off),
        Op::MyFlash => mask_op(m, st, assets.my.mask.as_deref(), c, Put::MaskInv, off),
        Op::StageCut => mask_op(m, st, sht_mask, c, Put::MaskAnd, off),
        Op::StageFlash => mask_op(m, st, sht_mask, c, Put::MaskInv, off),
        Op::StageOverlay => planes4(m, st, sht, c, Put::Or, off),
        Op::StageOverwrite => planes4(m, st, sht, c, Put::Mov, off),
        Op::BegOverwrite => planes4(m, st, beg, c, Put::Mov, off),
        Op::BegInverted => planes4(m, st, beg, c, Put::MovInv, off),
        Op::StageToScratch => scratch4(m, st, sht, c),
        Op::BegToScratch => scratch4(m, st, beg, c),
        Op::MapToScratch => map_tiles(m, st, &assets.map, c),
        Op::ScratchReplace => copy_rows(m, st, c, off, false),
        Op::ScratchReplaceShifted => copy_rows(m, st, c, off, true),
        Op::ScratchReplaceMirrored => copy_rows_mirror(m, st, c),
        Op::ClearDrawPage => clear_page(m, st, c),
        Op::ScratchOr => or_rows(m, st, c, off, false),
        Op::ScratchOrMirrored => or_rows(m, st, c, off, true),
        Op::ScratchOrPairs => or_word_rows(m, st, c),
        Op::ScratchLatchPairs => latch2_rows(m, st, c),
        Op::ScratchAnd => and_rows(m, st, c, off, false),
        Op::ScratchAndMirrored => and_rows(m, st, c, off, true),
        Op::ScratchAndPairs => and2_rows(m, st, c),
        Op::FillRows => fill_rows(m, st, c, off),
        Op::BarCell => bar_cell(m, st, c),
        Op::ClearScratchA => clear_scratch(m, st, 0),
        Op::ClearScratchB => clear_scratch(m, st, 0x8000),
        Op::ClearBothPages => clear_both_pages(m, st),
        Op::ScrollRotateCopy => scroll_rot_copy(m, st, c),
        Op::ScrollShiftLeft4 => scroll_shift_merge(m, st, c),
        Op::ScrollRotateColumn => scroll_col_rot(m, st, c),
        Op::ScrollMergeSeam => scroll_col_merge(m, st, c),
        Op::ScrollShowA | Op::ScrollShowB => scroll_view(m, st, c),
        Op::ScrollShiftBothLeft => scroll_left(m, st),
        // MGA 專用（`sub_1117F`），E 直接 `retn`。
        Op::EndOfFrameCopy => {}
        Op::Number => {
            counter(m, c.text_x, c.text_y, c.cx, font);
            restore(st);
        }
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
