//! `VgaSink`：把 `rs2-core` 記下的 blit 命令套到平面記憶體，逐像素重現原版 E 模式（byte_1CF74 = `E`）。
//!
//! 每個 `Op` 對應的原版函式與底層都在函式的文件註解寫明位址。CGA、MGA、純 EGA、單色分支不移植。
//! 規格見 `docs/spec/001-rust-first-stage.md`「繪製模型」，變體表見 `docs/re/12-stages-presentation.md` 第 2 節。
//!
//! 位址約定：偏移是 `A000` 段內的 16 位元偏移，運算一律環繞，與 8086 的 `di`、`si` 相同。

use crate::assets::Assets;
use crate::font::DIGITS;
use crate::vga::Vga;
use rs2_core::{BlitCmd, DrawCmd, Op, ShtGroup};
use rs2_formats::{BankItem, GoFile};

/// 色號經屬性暫存器後的 DAC 索引（`sub_1007D` 設定：`00～07,38h～3Fh`）。
pub(crate) const AC: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 56, 57, 58, 59, 60, 61, 62, 63];

/// 顯示頁的列位址表 `dseg:A00Ah`：200 列、每列 40 位元組。第 200 項起接著暫存區列表 `A19Ah`。
/// 規則是 `40 * 列`，由 `rs2-trace` 的 A5 測試對映像的表格逐項核對。
pub fn disp_row(row: u16) -> u16 {
    if row < 200 { row * 40 } else { scratch_row(row - 200) }
}

/// 暫存區列位址表 `dseg:A19Ah`：`1F40h + 80 * 列`。
pub fn scratch_row(row: u16) -> u16 {
    0x1F40u16.wrapping_add(row.wrapping_mul(80))
}

/// `sub_19683` 的結果。
struct Rows {
    bx: u16,
    cx: u16,
    si: u16,
    dx: u16,
    di: u16,
}

/// 跨命令的暫存變數與診斷。
#[derive(Clone)]
struct State {
    /// `word_2687E`：最近一次 `sub_18AA4`（或 `sub_1A2D2`、`sub_1935A`）寫入的每列位元組數。
    /// `sub_19683` 跳過 y < 0 的列時用它推進來源指標，`sub_19BA4` 等不先載入圖庫項的函式讀到的是殘值。
    stride_reg: u16,
    anomalies: u64,
    log: Vec<String>,
    /// 診斷：列位址表查過的最大列號（顯示頁表只計實際可能寫入的列，即 y 上限以內加介面列）。
    max_disp_row: u16,
    max_scratch_row: u16,
}

impl State {
    fn anomaly(&mut self, msg: String) {
        self.anomalies += 1;
        if self.log.len() < 64 {
            self.log.push(msg);
        }
    }
}

/// `sub_19683`：y < 0 的列跳過（高度與來源指標跟著調整），再查顯示頁列表算出列起點與寫入點。
/// 高度不足時 `cx = 1`、`bx = 12Ch`，使後面的列迴圈整個被 y 上限跳過。
fn prep(st: &mut State, y: u16, mut cx: u16, mut si: u16, page: u16, x: u16) -> Rows {
    let mut bx = y;
    while (bx as i16) < 0 {
        bx = bx.wrapping_add(1);
        cx = cx.wrapping_sub(1);
        si = si.wrapping_add(st.stride_reg);
    }
    if bx <= 0xCF {
        st.max_disp_row = st.max_disp_row.max(bx);
    }
    let dx = disp_row(bx).wrapping_add(page);
    let di = dx.wrapping_add(x);
    if (cx as i16) <= 0 {
        cx = 1;
        bx = 0x12C;
    }
    Rows { bx, cx, si, dx, di }
}

/// 圖庫列迴圈的寫入方式。
#[derive(Clone, Copy)]
enum Put {
    /// `loc_193E0`：`mov es:[di], al`。
    Mov,
    /// `loc_1938C`（含 `sub_192EA` 的繪圖部分）：`or es:[di], al`，讀取平面與寫入平面相同。
    Or,
    /// `loc_194DC`：`lodsb; mov ah, es:[di]; stosb`，GC 功能由呼叫端設成 AND。
    MaskAnd,
    /// `loc_19534`：同上但資料先取反，GC 功能由呼叫端設成 OR。
    MaskInv,
}

/// 圖庫項的列迴圈：`sub_18AA4` 取項，`sub_19683` 裁上緣，再逐列逐位元組寫。
/// 每列只寫 `[列起點, 列起點 + 27h]`，列號超過 189 的整列不寫，來源指標照常推進（跳列時不推進）。
fn put_rows(vga: &mut Vga, st: &mut State, it: &BankItem, c: &BlitCmd, put: Put) {
    let stride = it.stride;
    st.stride_reg = stride;
    if stride == 0 {
        st.anomaly(format!("{:?} index {} 每列位元組數為 0", c.op, c.index));
        return;
    }
    let r = prep(st, c.y, it.height, 0, c.page, c.x);
    let (mut bx, mut si, mut dx, mut di) = (r.bx, r.si, r.dx, r.di);
    for _ in 0..r.cx {
        let ax = dx.wrapping_add(0x27);
        if (bx as i16) <= 0xBD {
            for col in 0..stride {
                let d = di.wrapping_add(col);
                if d > ax || d < dx {
                    continue;
                }
                let b = match it.data.get(si as usize + col as usize) {
                    Some(&b) => b,
                    None => {
                        st.anomaly(format!("{:?} index {} 來源超出項資料", c.op, c.index));
                        0
                    }
                };
                match put {
                    Put::Mov => vga.write(d, b),
                    Put::Or => {
                        let v = vga.read(d);
                        vga.write(d, v | b);
                    }
                    Put::MaskAnd => {
                        vga.read(d);
                        vga.write(d, b);
                    }
                    Put::MaskInv => {
                        vga.read(d);
                        vga.write(d, !b);
                    }
                }
            }
            si = si.wrapping_add(stride);
        }
        bx = bx.wrapping_add(1);
        dx = dx.wrapping_add(0x28);
        di = di.wrapping_add(0x28);
    }
}

/// `sub_1A4F1`：圖庫項寫進背景暫存區。不裁切、不看 y 上限，每列 80 位元組，
/// 用 `rep movsw` 複製 `stride / 2` 個字組，所以奇數寬度的最後一個位元組被丟掉，來源指標只前進偶數位元組。
fn scratch_rows(vga: &mut Vga, st: &mut State, it: &BankItem, c: &BlitCmd) {
    let stride = it.stride;
    st.stride_reg = stride;
    st.max_scratch_row = st.max_scratch_row.max(c.y);
    let mut di = scratch_row(c.y).wrapping_add(c.scratch).wrapping_add(c.x);
    let mut si = 0usize;
    let words = (stride >> 1) as usize;
    for _ in 0..it.height {
        for i in 0..words {
            let lo = it.data.get(si + 2 * i).copied();
            let hi = it.data.get(si + 2 * i + 1).copied();
            if lo.is_none() || hi.is_none() {
                st.anomaly(format!("{:?} index {} 來源超出項資料", c.op, c.index));
            }
            let o = di.wrapping_add(2 * i as u16);
            vga.write(o, lo.unwrap_or(0));
            vga.write(o.wrapping_add(1), hi.unwrap_or(0));
        }
        si += 2 * words;
        di = di.wrapping_add(0x50);
    }
}

/// 暫存區來源指標：`(word_26866 的低位元組) * 80 + word_26864 + 1F40h + word_26868 + word_2685C`。
/// 原版用 `mul bl`，只取 `word_26866` 的低位元組。
pub(crate) fn scratch_src(c: &BlitCmd) -> u16 {
    ((c.src_y & 0xFF) * 0x50)
        .wrapping_add(c.src_x)
        .wrapping_add(0x1F40)
        .wrapping_add(c.shift)
        .wrapping_add(c.scratch)
}

#[derive(Clone)]
pub struct VgaSink {
    assets: Assets,
    vga: Vga,
    dac: [[u8; 3]; 256],
    /// 幀計數（`sub_1890D`）預設不畫。
    pub show_counter: bool,
    st: State,
    seq: u64,
    watch_seq: Option<u64>,
}

impl VgaSink {
    /// 平面全 0，DAC 全 0。
    pub fn new(assets: Assets) -> VgaSink {
        VgaSink {
            assets,
            vga: Vga::new(),
            dac: [[0; 3]; 256],
            show_counter: false,
            st: State { stride_reg: 0, anomalies: 0, log: Vec::new(), max_disp_row: 0, max_scratch_row: 0 },
            seq: 0,
            watch_seq: None,
        }
    }

    /// 平面 `p`（0 至 3，VGA 的平面編號），64 KB。
    pub fn plane(&self, p: usize) -> &[u8] {
        self.vga.plane(p)
    }

    /// DAC 暫存器，6 位元。
    pub fn dac(&self, index: u8) -> [u8; 3] {
        self.dac[index as usize]
    }

    /// 色號 0 至 15 經屬性暫存器後的 DAC 值（6 位元），等於收據的 `shown16_dac6`。
    pub fn shown16(&self) -> [[u8; 3]; 16] {
        let mut o = [[0u8; 3]; 16];
        for (i, v) in o.iter_mut().enumerate() {
            *v = self.dac[AC[i]];
        }
        o
    }

    /// 已套用的命令數。
    pub fn applied(&self) -> u64 {
        self.seq
    }

    /// 無法照機器碼重現的次數（圖庫索引或來源超出範圍等）。
    pub fn anomalies(&self) -> u64 {
        self.st.anomalies
    }

    pub fn anomaly_log(&self) -> &[String] {
        &self.st.log
    }

    /// 診斷：目前為止列位址表查過的最大列號（顯示頁表, 暫存區表）。
    pub fn max_table_rows(&self) -> (u16, u16) {
        (self.st.max_disp_row, self.st.max_scratch_row)
    }

    /// 除錯用：監看某平面某位元組，之後 `last_write_seq` 回傳最後寫到它的命令序號（從 0 起算）。
    pub fn watch(&mut self, plane: usize, off: u16) {
        self.vga.set_watch(plane, off);
        self.watch_seq = None;
    }

    pub fn last_write_seq(&self) -> Option<u64> {
        self.watch_seq
    }

    /// 依序套用一個命令。
    pub fn apply(&mut self, cmd: &DrawCmd) {
        match cmd {
            DrawCmd::SetDac { start, rgb } => {
                for i in 0..16usize {
                    let idx = (*start as usize + i) & 0xFF;
                    self.dac[idx] = [rgb[3 * i] & 0x3F, rgb[3 * i + 1] & 0x3F, rgb[3 * i + 2] & 0x3F];
                }
            }
            DrawCmd::Blit(c) => self.blit(c),
        }
        if self.vga.take_watch_hit() {
            self.watch_seq = Some(self.seq);
        }
        self.seq += 1;
    }

    /// 320x200 RGB（每像素 3 位元組，共 192000）。`page` 是頁偏移，0 或 8000h。
    /// 色號經屬性暫存器 `[0..7, 56..63]` 變成 DAC 索引，6 位元轉 8 位元用 `(v * 255 + 31) / 63`。
    pub fn render_page(&self, page: u16, out: &mut [u8]) {
        assert!(out.len() >= 320 * 200 * 3, "輸出緩衝區不足 192000 位元組");
        let to8 = |v: u8| ((v as u32 * 255 + 31) / 63) as u8;
        let pl = [self.vga.plane(0), self.vga.plane(1), self.vga.plane(2), self.vga.plane(3)];
        for y in 0..200usize {
            for bx in 0..40usize {
                let a = page.wrapping_add((y * 40 + bx) as u16) as usize;
                let b = [pl[0][a], pl[1][a], pl[2][a], pl[3][a]];
                for bit in 0..8usize {
                    let sh = 7 - bit;
                    let color = ((b[0] >> sh) & 1) | (((b[1] >> sh) & 1) << 1) | (((b[2] >> sh) & 1) << 2) | (((b[3] >> sh) & 1) << 3);
                    let rgb = self.dac[AC[color as usize]];
                    let o = (y * 320 + bx * 8 + bit) * 3;
                    out[o] = to8(rgb[0]);
                    out[o + 1] = to8(rgb[1]);
                    out[o + 2] = to8(rgb[2]);
                }
            }
        }
    }

    fn blit(&mut self, c: &BlitCmd) {
        let Self { assets, vga, st, show_counter, .. } = self;
        fn sht_go(a: &Assets, g: ShtGroup) -> &GoFile {
            match g {
                ShtGroup::Bh1 => &a.bh1,
                ShtGroup::Sht1 => &a.sht1,
            }
        }
        let sht = |g: ShtGroup| sht_go(assets, g);
        match c.op {
            // MY 4 平面。
            Op::S18C0A => planes4(vga, st, &assets.my, c, Put::Mov),
            Op::S18CE2 | Op::S18EB6 | Op::S18DCC => planes4(vga, st, &assets.my, c, Put::Or),
            // MY 遮罩。
            Op::S18D46 | Op::S18E30 | Op::S18E73 => mask_op(vga, st, assets.my.mask.as_deref(), c, Put::MaskAnd),
            Op::S18D89 => mask_op(vga, st, assets.my.mask.as_deref(), c, Put::MaskInv),
            // SHT 遮罩（`word_1CF5C`，載入 BH 不會蓋掉它）與 4 平面（`word_1CF5E～64`，BH 與 SHT 共用）。
            Op::S18F1A => mask_op(vga, st, assets.sht1.mask.as_deref(), c, Put::MaskAnd),
            Op::S19025 => mask_op(vga, st, assets.sht1.mask.as_deref(), c, Put::MaskInv),
            Op::S18F5D => planes4(vga, st, sht(c.sht_group), c, Put::Or),
            Op::S18FC1 => planes4(vga, st, sht(c.sht_group), c, Put::Mov),
            Op::S19ADC => scratch4(vga, st, sht(c.sht_group), c),
            Op::S19BA4 => copy_rows(vga, st, c),
            Op::S19D08 => copy_rows_shift(vga, st, c),
            Op::S19CF9 => clear_page(vga, c),
            Op::S19DA8 => or_rows(vga, st, c),
            Op::S1A09F => and_rows(vga, st, c),
            Op::S1A287 => fill_rows(vga, st, c),
            Op::S1929F => bar_cell(vga, st, c),
            Op::S1152D => clear_scratch_b(vga),
            Op::S1890D => {
                // 幀計數由工具列開關控制。其他位置的數字是 sub_16C7D 的擊中計數，照原版畫。
                let is_frame_counter = c.text_x == COUNTER_X && c.text_y == COUNTER_Y;
                if !is_frame_counter || *show_counter {
                    counter(vga, c.text_x, c.text_y, c.cx);
                }
            }
        }
    }
}

/// `sub_1882E`：`bl` 為 1、2、4、8 時，讀取平面（GC 索引 4）= 3、2、1、0，寫入平面（序列器索引 2）= 8、4、2、1。
/// `k` 是 `bl` 的位元序號：圖庫第 `k` 個平面區段對應 VGA 平面 `3 - k`。
fn sel_plane(vga: &mut Vga, k: usize) {
    vga.gc(4, (3 - k) as u8);
    vga.seq_map_mask(1 << (3 - k));
}

/// `sub_18878`：map mask = 0Fh，GC 功能 = 取代、旋轉 0。
fn restore(vga: &mut Vga) {
    vga.seq_map_mask(0x0F);
    vga.gc(3, 0);
}

/// `sub_18C0A`、`18CE2`、`18DCC`、`18EB6`、`18F5D`、`18FC1` 的 E 分支：四個平面各跑一次列迴圈。
fn planes4(vga: &mut Vga, st: &mut State, go: &GoFile, c: &BlitCmd, put: Put) {
    for k in 0..4 {
        sel_plane(vga, k);
        match go.planes[k].get(c.index as usize) {
            Some(it) => put_rows(vga, st, it, c, put),
            None => st.anomaly(format!("{:?} 平面 {k} 沒有 index {}", c.op, c.index)),
        }
    }
    restore(vga);
}

/// `sub_18D46`、`18D89`、`18E30`、`18E73`、`18F1A`、`19025`：GC 寫入模式 0，功能 AND（`0803h`）或 OR（`1003h`），
/// map mask 沿用上一次 `sub_18878` 設的 0Fh，所以遮罩同時作用在四個平面。
fn mask_op(vga: &mut Vga, st: &mut State, mask: Option<&[BankItem]>, c: &BlitCmd, put: Put) {
    vga.gc(5, 0);
    vga.gc(3, if matches!(put, Put::MaskAnd) { 0x08 } else { 0x10 });
    match mask.and_then(|m| m.get(c.index as usize)) {
        Some(it) => put_rows(vga, st, it, c, put),
        None => st.anomaly(format!("{:?} 沒有遮罩 index {}", c.op, c.index)),
    }
    restore(vga);
}

/// `sub_19ADC` 的 E 分支：四個平面各寫一次暫存區。
fn scratch4(vga: &mut Vga, st: &mut State, go: &GoFile, c: &BlitCmd) {
    for k in 0..4 {
        sel_plane(vga, k);
        match go.planes[k].get(c.index as usize) {
            Some(it) => scratch_rows(vga, st, it, c),
            None => st.anomaly(format!("{:?} 平面 {k} 沒有 index {}", c.op, c.index)),
        }
    }
    restore(vga);
}

/// `sub_19BA4` 的 E 分支：GC 寫入模式 1，把暫存區的列以閂鎖複製到顯示頁，每列 40 位元組，來源列距 80。
/// 目的不裁切欄，只裁 y：列號超過 189 結束，y < 0 的列跳過。結束時 GC 模式回 0。
fn copy_rows(vga: &mut Vga, st: &mut State, c: &BlitCmd) {
    vga.gc(5, 1);
    vga.gc(3, 0);
    let r = prep(st, c.y, c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut di) = (r.bx, r.si, r.di);
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        for i in 0..0x28u16 {
            vga.read(si.wrapping_add(i));
            vga.write(di.wrapping_add(i), 0);
        }
        di = di.wrapping_add(0x28);
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
    vga.gc(5, 0);
}

/// `sub_19D08` 的 E 分支（`sub_1A2D2`）：同 `sub_19BA4`，另有水平位移 `word_26868`。
/// 位移也加進來源指標；每寫一個位元組計數加 1，計數 >= 50h（含負數）的位元組不寫。
/// `sub_1A2D2` 一開始把 `word_2687E` 設成 50h。
fn copy_rows_shift(vga: &mut Vga, st: &mut State, c: &BlitCmd) {
    vga.gc(5, 1);
    vga.gc(3, 0);
    st.stride_reg = 0x50;
    let r = prep(st, c.y, c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut di) = (r.bx, r.si, r.di);
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        let mut cnt = c.shift;
        for i in 0..0x28u16 {
            vga.read(si.wrapping_add(i));
            if cnt < 0x50 {
                vga.write(di.wrapping_add(i), 0);
            }
            cnt = cnt.wrapping_add(1);
        }
        di = di.wrapping_add(0x28);
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
    vga.gc(5, 0);
}

/// `sub_19CF9`（`sub_1A317`）：`sub_18878` 後，從 `word_26872` 起填 `0ED8h` 個字組的 0，也就是 190 列。
fn clear_page(vga: &mut Vga, c: &BlitCmd) {
    restore(vga);
    let mut di = c.page;
    for _ in 0..0x0ED8u16 {
        vga.write(di, 0);
        vga.write(di.wrapping_add(1), 0);
        di = di.wrapping_add(2);
    }
}

/// `sub_19DA8`（`sub_1A369`）：逐平面把暫存區整列 OR 到顯示頁。每個平面讀、寫同一個平面，
/// 每列 14h 個字組，來源列距 80。
fn or_rows(vga: &mut Vga, st: &mut State, c: &BlitCmd) {
    let src0 = scratch_src(c);
    for k in 0..4 {
        sel_plane(vga, k);
        let r = prep(st, c.y, c.cx, src0, c.page, c.x);
        let (mut bx, mut si, mut di) = (r.bx, r.si, r.di);
        for _ in 0..r.cx {
            if bx > 0xBD {
                break;
            }
            let mut d = di;
            for _ in 0..0x14 {
                let slo = vga.read(si);
                let shi = vga.read(si.wrapping_add(1));
                si = si.wrapping_add(2);
                let dlo = vga.read(d);
                let dhi = vga.read(d.wrapping_add(1));
                vga.write(d, dlo | slo);
                vga.write(d.wrapping_add(1), dhi | shi);
                d = d.wrapping_add(2);
            }
            di = di.wrapping_add(0x28);
            si = si.wrapping_add(0x28);
            bx = bx.wrapping_add(1);
        }
    }
    restore(vga);
}

/// `sub_1A09F`（`sub_1A3C5`）：GC 寫入模式 0、功能 AND。來源位元組用 `lodsb` 從顯示記憶體讀，
/// 讀的是 GC 索引 4 目前選的平面（殘值，通常是上一次 `sub_1882E` 留下的平面 0），
/// 再 AND 到目的的四個平面。每列 40 位元組，來源列距 80。
fn and_rows(vga: &mut Vga, st: &mut State, c: &BlitCmd) {
    vga.gc(5, 0);
    vga.gc(3, 0x08);
    let r = prep(st, c.y, c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut di) = (r.bx, r.si, r.di);
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        for i in 0..0x28u16 {
            let b = vga.read(si.wrapping_add(i));
            vga.read(di.wrapping_add(i));
            vga.write(di.wrapping_add(i), b);
        }
        di = di.wrapping_add(0x28);
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
    restore(vga);
}

/// `sub_1A287`（`sub_1A4C9`）：GC 寫入模式 2 填實色，`dx` 的低 4 位元是色號。
/// 從 `di` 起連續寫 `cx * 40` 位元組（列距就是 40，不另外歸位），列號超過 189 結束。
fn fill_rows(vga: &mut Vga, st: &mut State, c: &BlitCmd) {
    vga.gc(5, 2);
    vga.gc(3, 0);
    let r = prep(st, c.y, c.cx, 0, c.page, c.x);
    let (mut bx, mut di) = (r.bx, r.di);
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        for _ in 0..0x28 {
            vga.write(di, c.dx as u8);
            di = di.wrapping_add(1);
        }
        bx = bx.wrapping_add(1);
    }
    vga.gc(5, 0);
}

/// `sub_1929F`（`sub_1935A`）：血條的一格，寫入模式 2，7 列各 1 位元組，每列同時寫兩頁（`xor di, 8000h`）。
/// 不裁 y 上限。`sub_1935A` 把 `word_2687E` 設成 1。
fn bar_cell(vga: &mut Vga, st: &mut State, c: &BlitCmd) {
    vga.gc(5, 2);
    vga.gc(3, 0);
    st.stride_reg = 1;
    let r = prep(st, c.y, c.cx, 0, c.page, c.x);
    let mut di = r.di;
    for _ in 0..7 {
        vga.write(di, c.dx as u8);
        vga.write(di ^ 0x8000, c.dx as u8);
        di = di.wrapping_add(0x28);
    }
    vga.gc(5, 0);
}

/// `sub_1152D`：從 `A800:1F40` 起填 `20BFh` 個字組的 0，也就是平面偏移 `9F40h` 起 `417Eh` 位元組。
fn clear_scratch_b(vga: &mut Vga) {
    let mut di = 0x8000u16.wrapping_add(0x1F40);
    for _ in 0..0x20BFu16 {
        vga.write(di, 0);
        vga.write(di.wrapping_add(1), 0);
        di = di.wrapping_add(2);
    }
}

/// 幀計數的位置：`sub_1100A` 把 `word_2688A` 設成 12h、`word_2688E` 設成 BFh。
/// `BlitCmd` 沒有帶這兩個變數，所以固定用這組值。`sub_16C7D` 的雷射擊中數字用別的位置，見回報。
pub(crate) const COUNTER_X: u16 = 0x12;
pub(crate) const COUNTER_Y: u16 = 0xBF;

/// `sub_1890D`：5 位數字寫進平面 3，從 `word_2688A` 欄起 5 欄、`word_2688E` 列起 8 列，
/// 每列同時寫兩頁（`es:[di]` 與 `es:[di - 8000h]`）。字型由重製自繪。
fn counter(vga: &mut Vga, col: u16, row: u16, value: u16) {
    vga.seq_map_mask(8);
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
            vga.write(off, bits);
            vga.write(off.wrapping_sub(0x8000), bits);
        }
    }
    restore(vga);
}

/// 把平面偏移換算成人看得懂的位置，比對失敗時回報用。
pub fn locate(off: u16) -> String {
    let o = off as u32;
    if o < 8000 {
        format!("頁 0 列 {} 欄 {}", o / 40, o % 40)
    } else if o < 0x1F40 {
        format!("頁 0 之後 {}", o)
    } else if o < 0x8000 {
        format!("暫存區 A 列 {} 欄 {}", (o - 0x1F40) / 80, (o - 0x1F40) % 80)
    } else if o < 0x8000 + 8000 {
        format!("頁 1 列 {} 欄 {}", (o - 0x8000) / 40, (o - 0x8000) % 40)
    } else if o < 0x9F40 {
        format!("頁 1 之後 {}", o)
    } else {
        format!("暫存區 B 列 {} 欄 {}", (o - 0x9F40) / 80, (o - 0x9F40) % 80)
    }
}
