//! `VgaSink`：把 `rs2-game` 記下的 blit 命令套到平面記憶體，逐像素重現原版 E 模式（byte_1CF74 = `E`）。
//!
//! 每個 `Op` 對應的原版函式與底層都在函式的文件註解寫明位址。CGA、MGA、純 EGA、單色分支不移植。
//! 規格見 `docs/spec/001-rust-first-stage.md`「繪製模型」，變體表見 `docs/re/12-stages-presentation.md` 第 2 節。
//!
//! 位址約定：偏移是 `A000` 段內的 16 位元偏移，運算一律環繞，與 8086 的 `di`、`si` 相同。

use crate::assets::Assets;
use crate::font::DIGITS;
use crate::vga::{PLANE_SIZE, Vga};
use rs2_game::{BlitCmd, DrawCmd, Op, ShtGroup};
use rs2_formats::{BankItem, GoFile, sha256_hex};

/// 色號經屬性暫存器後的 DAC 索引（`sub_1007D` 設定：`00～07,38h～3Fh`）。
pub(crate) const AC: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 56, 57, 58, 59, 60, 61, 62, 63];

/// 17 位元組屬性暫存器：16 個調色盤暫存器接邊框色。
/// VGA 路徑（`byte_1CF75 != 'Y'`）只在開機時由 `sub_1007D` 以 INT 10h AX=1002h 載入 `dseg:3C2h` 的表
/// （`00～07,38h～3Fh`，邊框 `00`），之後的調色盤切換只寫 DAC（`sub_11170`），所以這組值固定。
/// 與 `rs2dyn` 的 `ac[0..16] = VGA.Pal(i)`、`ac[16] = VGA.Overscan()` 同序。
pub const ATTR_REGS: [u8; 17] = [0, 1, 2, 3, 4, 5, 6, 7, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0];

/// 16 個 EGA 屬性暫存器值（6 位元 rgbRGB）換成 8 位元 RGB。
pub fn ega_rgb16(regs: &[u8; 16]) -> [[u8; 3]; 16] {
    let mut o = [[0u8; 3]; 16];
    for (i, c) in regs.iter().enumerate() {
        let r = ((c >> 2) & 1) * 2 + ((c >> 5) & 1);
        let g = ((c >> 1) & 1) * 2 + ((c >> 4) & 1);
        let b = (c & 1) * 2 + ((c >> 3) & 1);
        o[i] = [r * 85, g * 85, b * 85];
    }
    o
}

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

/// 原版的圖庫記憶體傾印（`rs2dyn` 的 `.bk`），只給對拍用。
///
/// 圖號超過圖庫項數時，原版照樣查表：`sub_18AA4` 從段開頭取 `圖號 × 2` 的字組當指標，
/// 指標處的兩個字組當高度與每列位元組數，後面當像素。結果取決於段內的殘留內容，
/// 指標可以跨進別的圖庫段，所以重製的正式路徑不畫，對拍時才用傾印重現。
#[derive(Clone)]
pub struct BankDump {
    group: ShtGroup,
    /// 遮罩段與四個平面段（`word_1CF5C`、`1CF5E`、`1CF60`、`1CF62`、`1CF64`）。
    segs: [u16; 5],
    /// 從 `segs[0]:0` 起的線性記憶體。
    mem: Vec<u8>,
}

impl BankDump {
    /// `raw` 的前 10 位元組是 5 個段值，之後是線性記憶體，要涵蓋到最後一段的 64 KB。
    pub fn parse(group: ShtGroup, raw: &[u8]) -> Result<BankDump, String> {
        let head = raw.get(..10).ok_or("圖庫傾印不足 10 位元組")?;
        let mut segs = [0u16; 5];
        for (i, s) in segs.iter_mut().enumerate() {
            *s = u16::from_le_bytes([head[2 * i], head[2 * i + 1]]);
        }
        let need = ((segs[4] as usize).wrapping_sub(segs[0] as usize) << 4) + 0x10000;
        if segs[4] < segs[0] || raw.len() != 10 + need {
            return Err(format!("圖庫傾印長度 {}，段值 {segs:04X?}", raw.len()));
        }
        Ok(BankDump { group, segs, mem: raw[10..].to_vec() })
    }

    fn rd8(&self, seg: u16, off: u16) -> u8 {
        self.mem[(((seg - self.segs[0]) as usize) << 4) + off as usize]
    }

    /// 字組的兩個位元組各自在段內環繞。
    fn rd16(&self, seg: u16, off: u16) -> u16 {
        u16::from_le_bytes([self.rd8(seg, off), self.rd8(seg, off.wrapping_add(1))])
    }

    /// `sub_18AA4` 在段 `which`（0 是遮罩，1 到 4 是 `word_1CF5E` 到 `1CF64`）讀到的圖庫項。
    /// 來源指標在段內環繞，列迴圈會多讀一列的量，所以資料放 128 KB。
    fn item(&self, which: usize, index: u16) -> BankItem {
        let seg = self.segs[which];
        let p = self.rd16(seg, index.wrapping_mul(2));
        let data = (0..0x20000u32).map(|i| self.rd8(seg, p.wrapping_add(4).wrapping_add(i as u16))).collect();
        BankItem { height: self.rd16(seg, p), stride: self.rd16(seg, p.wrapping_add(2)), data }
    }
}

/// 跨命令的暫存變數與診斷。
#[derive(Clone)]
struct State {
    /// 對拍用：原版的圖庫記憶體。正式路徑是 `None`，越界圖號不畫。
    dump: Option<BankDump>,
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
    fn dump_item(&self, group: ShtGroup, which: usize, index: u16) -> Option<BankItem> {
        self.dump.as_ref().filter(|d| d.group == group).map(|d| d.item(which, index))
    }

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
    /// EGA 屬性暫存器 0 至 15（6 位元 rgbRGB），調色盤函式在 EGA（`byte_1CF75` = `Y`）寫的值。
    /// 初值是 `sub_1007D` 開機時載入的 `00～07,38h～3Fh`，也就是標準的 16 色。VGA 的畫面不用它。
    ega: [u8; 16],
    /// 幀計數（`sub_1890D`）預設不畫。設了 BIOS 字型（`set_counter_font`）之後永遠畫。
    pub show_counter: bool,
    /// BIOS 8x8 字型 2048 位元組（`F000:FA6Eh` 起 256 字元，每字元 8 列），`sub_18A61` 讀的來源。
    counter_font: Option<Vec<u8>>,
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
            ega: ATTR_REGS[..16].try_into().expect("16 個暫存器"),
            show_counter: false,
            counter_font: None,
            st: State { dump: None, stride_reg: 0, anomalies: 0, log: Vec::new(), max_disp_row: 0, max_scratch_row: 0 },
            seq: 0,
            watch_seq: None,
        }
    }

    /// 對拍用：之後對這組圖庫的越界圖號照原版讀傾印來畫，見 `BankDump`。
    pub fn set_bank_dump(&mut self, dump: BankDump) {
        self.st.dump = Some(dump);
    }

    /// 平面 `p`（0 至 3，VGA 的平面編號），64 KB。
    pub fn plane(&self, p: usize) -> &[u8] {
        self.vga.plane(p)
    }

    /// 用原版某個時點的顯示記憶體與 DAC 當起點（對拍用）。四個平面各 64 KB（`planes[p]` 是 VGA 平面 `p`），
    /// DAC 是 256 色 x 3 的 6 位元值（`rs2dyn` 的 `dac256_hex`）。序列器與圖形控制器暫存器回到預設
    /// （map mask 0Fh、寫入模式 0、功能取代），也就是原版每個繪製函式結束時的狀態。
    /// 命令序號、診斷計數與 `word_2687E` 殘值不變；殘值用 `set_stride_reg` 另設。
    ///
    /// # Panics
    /// 任一平面不是 64 KB。
    pub fn load_snapshot(&mut self, planes: [&[u8]; 4], dac: &[u8; 768]) {
        for (p, data) in planes.iter().enumerate() {
            assert_eq!(data.len(), PLANE_SIZE, "平面 {p} 不是 64 KB（{} 位元組）", data.len());
        }
        let mut vga = Vga::new();
        for (p, data) in planes.iter().enumerate() {
            vga.load_plane(p, data);
        }
        self.vga = vga;
        for i in 0..256 {
            self.dac[i] = [dac[3 * i] & 0x3F, dac[3 * i + 1] & 0x3F, dac[3 * i + 2] & 0x3F];
        }
    }

    /// 設 `word_2687E`（最近一次載入的圖庫項每列位元組數）。`sub_19683` 跳過 y < 0 的列時用它推進來源指標，
    /// 從快照起跑而還沒有載入圖庫項的命令讀到的是原版的殘值。
    pub fn set_stride_reg(&mut self, v: u16) {
        self.st.stride_reg = v;
    }

    /// `word_2687E` 目前的值（診斷用）。
    pub fn stride_reg(&self) -> u16 {
        self.st.stride_reg
    }

    /// BIOS 8x8 字型（2048 位元組）。設了之後幀計數照 `sub_1890D`、`sub_18A61` 用這份字型畫，並且永遠畫
    /// （不看 `show_counter`）。沒設就維持內建的自繪數字與 `show_counter` 開關。
    ///
    /// # Panics
    /// 長度不是 2048。
    pub fn set_counter_font(&mut self, font: &[u8]) {
        assert_eq!(font.len(), 2048, "BIOS 字型應為 2048 位元組（256 字元 x 8 列）");
        self.counter_font = Some(font.to_vec());
    }

    /// 4 個平面（0 至 3，各 64 KB，共 256 KB）依序串接的 SHA-256，小寫十六進位。對應 `waittrace` 的 `vram`。
    pub fn vram_sha256(&self) -> String {
        let mut buf = Vec::with_capacity(4 * PLANE_SIZE);
        for p in 0..4 {
            buf.extend_from_slice(self.vga.plane(p));
        }
        sha256_hex(&buf)
    }

    /// 256 色 DAC（768 位元組，每色 R、G、B 6 位元）接 17 位元組屬性暫存器（`ATTR_REGS`）的 SHA-256，
    /// 小寫十六進位。對應 `waittrace` 的 `dac`。
    pub fn dac_sha256(&self) -> String {
        let mut buf = Vec::with_capacity(768 + 17);
        for rgb in &self.dac {
            buf.extend_from_slice(rgb);
        }
        buf.extend_from_slice(&ATTR_REGS);
        sha256_hex(&buf)
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
            DrawCmd::SetDacOne { index, rgb } => {
                self.dac[*index as usize] = [rgb[0] & 0x3F, rgb[1] & 0x3F, rgb[2] & 0x3F];
            }
            DrawCmd::EgaAttr { regs } => self.ega.copy_from_slice(&regs[..16]),
            DrawCmd::VramFill { off, words, value } => vram_fill(&mut self.vga, *off, *words, *value),
            DrawCmd::SetMapMask { mask } => set_map_mask(&mut self.vga, *mask),
            DrawCmd::Blit(c) => self.blit(c),
        }
        if self.vga.take_watch_hit() {
            self.watch_seq = Some(self.seq);
        }
        self.seq += 1;
    }

    /// EGA 屬性暫存器 0 至 15 的目前值（6 位元 rgbRGB）。
    pub fn ega_regs(&self) -> [u8; 16] {
        self.ega
    }

    /// 色號 0 至 15 在 EGA 的 8 位元 RGB。6 位元 rgbRGB 的位元 2、1、0 是紅、綠、藍（2/3 亮度），位元 5、4、3 是紅、綠、藍
    /// （1/3 亮度），每個通道 `2 × 主 + 副` 個 55h（EGA 彩色顯示器的 4 階，`tools/rs2_common.py` 的 `ega6_to_rgb` 同）。
    pub fn ega_colors(&self) -> [[u8; 3]; 16] {
        ega_rgb16(&self.ega)
    }

    /// 同 `render_page`，但用 EGA 屬性暫存器的顏色（顯示模式 EGA）。平面的內容與 VGA 相同，差別只在色號對到的顏色。
    pub fn render_page_ega(&self, page: u16, out: &mut [u8]) {
        assert!(out.len() >= 320 * 200 * 3, "輸出緩衝區不足 192000 位元組");
        let pal = self.ega_colors();
        let pl = [self.vga.plane(0), self.vga.plane(1), self.vga.plane(2), self.vga.plane(3)];
        for y in 0..200usize {
            for bx in 0..40usize {
                let a = page.wrapping_add((y * 40 + bx) as u16) as usize;
                let b = [pl[0][a], pl[1][a], pl[2][a], pl[3][a]];
                for bit in 0..8usize {
                    let sh = 7 - bit;
                    let color = ((b[0] >> sh) & 1) | (((b[1] >> sh) & 1) << 1) | (((b[2] >> sh) & 1) << 2) | (((b[3] >> sh) & 1) << 3);
                    let o = (y * 320 + bx * 8 + bit) * 3;
                    out[o..o + 3].copy_from_slice(&pal[color as usize]);
                }
            }
        }
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
        if let Some(v) = c.stride_reg {
            self.st.stride_reg = v;
        }
        let Self { assets, vga, st, show_counter, counter_font, .. } = self;
        // SHT 4 平面（`word_1CF5E～64`，BH 與 SHT 共用）；遮罩（`word_1CF5C`，載入 BH 不會蓋掉它）用 `mask_group`。
        let sht = assets.sht(c.sht_group);
        let sht_mask = assets.sht_mask(c.mask_group);
        let beg = assets.beg(c.beg_group);
        match c.op {
            // MY 4 平面。
            Op::MyOverwrite => planes4(vga, st, &assets.my, c, Put::Mov),
            Op::MyOverlay | Op::MyOverlayPlayerShot | Op::MyOverlayEffect => planes4(vga, st, &assets.my, c, Put::Or),
            // `loc_19534`：資料取反後寫入，GC 函式沿用（取代）。
            Op::MyOverwriteInverted => planes4(vga, st, &assets.my, c, Put::MaskInv),
            // MY 遮罩。
            Op::MyCut | Op::MyCutEffect | Op::MyCutPlayerShot => mask_op(vga, st, assets.my.mask.as_deref(), c, Put::MaskAnd),
            Op::MyFlash => mask_op(vga, st, assets.my.mask.as_deref(), c, Put::MaskInv),
            Op::StageCut => mask_op(vga, st, sht_mask, c, Put::MaskAnd),
            Op::StageFlash => mask_op(vga, st, sht_mask, c, Put::MaskInv),
            Op::StageOverlay => planes4(vga, st, sht, c, Put::Or),
            Op::StageOverwrite => planes4(vga, st, sht, c, Put::Mov),
            // BEG1／J 4 平面（`word_1CF46～4C`）。
            Op::BegOverwrite => planes4(vga, st, beg, c, Put::Mov),
            Op::BegInverted => planes4(vga, st, beg, c, Put::MaskInv),
            Op::StageToScratch => scratch4(vga, st, sht, c),
            Op::BegToScratch => scratch4(vga, st, beg, c),
            Op::MapToScratch => map_tiles(vga, st, &assets.map, c),
            Op::ScratchReplace => copy_rows(vga, st, c),
            Op::ScratchReplaceShifted => copy_rows_shift(vga, st, c),
            Op::ScratchReplaceMirrored => copy_rows_mirror(vga, st, c),
            Op::ClearDrawPage => clear_page(vga, c),
            Op::ScratchOr => or_rows(vga, st, c, false),
            Op::ScratchOrMirrored => or_rows(vga, st, c, true),
            Op::ScratchOrPairs => or_word_rows(vga, st, c),
            Op::ScratchLatchPairs => latch2_rows(vga, st, c),
            Op::ScratchAnd => and_rows(vga, st, c, false),
            Op::ScratchAndMirrored => and_rows(vga, st, c, true),
            Op::ScratchAndPairs => and2_rows(vga, st, c),
            Op::FillRows => fill_rows(vga, st, c),
            Op::BarCell => bar_cell(vga, st, c),
            Op::ClearScratchA => clear_scratch(vga, 0),
            Op::ClearScratchB => clear_scratch(vga, 0x8000),
            Op::ClearBothPages => clear_both_pages(vga),
            // 第三關地圖捲動（語意見 `workspace/progress/ops-needed.md`，機器碼 `sub_17EBA` 起）。
            Op::ScrollRotateCopy => scroll_rot_copy(vga, c),
            Op::ScrollShiftLeft4 => scroll_shift_merge(vga, c),
            Op::ScrollRotateColumn => scroll_col_rot(vga, c),
            Op::ScrollMergeSeam => scroll_col_merge(vga, c),
            Op::ScrollShowA | Op::ScrollShowB => scroll_view(vga, c),
            Op::ScrollShiftBothLeft => scroll_left(vga),
            // MGA 專用（`sub_1117F`），E 直接 `retn`。
            Op::EndOfFrameCopy => {}
            Op::Number => {
                // 沒設 BIOS 字型時，幀計數由工具列開關控制；設了就永遠畫（對拍用）。
                // 其他位置的數字是 sub_16C7D 的擊中計數，照原版畫。
                let is_frame_counter = c.text_x == COUNTER_X && c.text_y == COUNTER_Y;
                if counter_font.is_some() || !is_frame_counter || *show_counter {
                    counter(vga, c.text_x, c.text_y, c.cx, counter_font.as_deref());
                }
            }
        }
    }
}

/// `DrawCmd::SetMapMask`：`sub_1882E` 的結果（單一平面）同時選讀取平面（GC 索引 4 = 3、2、1、0）與寫入平面；
/// `sub_18878` 的 0Fh 另外把 GC 功能設回取代、旋轉 0，不動讀取平面。
fn set_map_mask(vga: &mut Vga, mask: u8) {
    let m = mask & 0x0F;
    match m {
        8 => vga.gc(4, 3),
        4 => vga.gc(4, 2),
        2 => vga.gc(4, 1),
        1 => vga.gc(4, 0),
        0x0F => vga.gc(3, 0),
        _ => {}
    }
    vga.seq_map_mask(m);
}

/// `DrawCmd::VramFill`：`rep stosw`，從 `A000:off` 起寫 `words` 個字組，低位元組在前。
/// 寫入的平面由目前的 map mask 決定，寫入模式與功能沿用目前值（繪製函式結束時是模式 0、取代）。
fn vram_fill(vga: &mut Vga, off: u16, words: u16, value: u16) {
    let (lo, hi) = (value as u8, (value >> 8) as u8);
    let mut di = off;
    for _ in 0..words {
        vga.write(di, lo);
        vga.write(di.wrapping_add(1), hi);
        di = di.wrapping_add(2);
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
            None => {
                st.anomaly(format!("{:?} 平面 {k} 沒有 index {}", c.op, c.index));
                if let Some(it) = st.dump_item(c.sht_group, k + 1, c.index) {
                    put_rows(vga, st, &it, c, put);
                }
            }
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
        None => {
            st.anomaly(format!("{:?} 沒有遮罩 index {}", c.op, c.index));
            if let Some(it) = st.dump_item(c.mask_group, 0, c.index) {
                put_rows(vga, st, &it, c, put);
            }
        }
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
/// 每列 14h 個字組，來源列距 80。`mirror` 是 `sub_19F3B`（`sub_1A397`）：目的列遞減（`sub_19683` 算出的
/// 起點每列減 28h），其餘相同，第四關通道的上壁。
fn or_rows(vga: &mut Vga, st: &mut State, c: &BlitCmd, mirror: bool) {
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
            di = if mirror { di.wrapping_sub(0x28) } else { di.wrapping_add(0x28) };
            si = si.wrapping_add(0x28);
            bx = bx.wrapping_add(1);
        }
    }
    restore(vga);
}

/// `sub_19E66`（`sub_1A453`）：逐平面，每列只 OR 一個字組（16 像素）。
/// 只檢查字組起點：`di` 大於列起點 + 27h 就整個字組不寫（不檢查下界），所以起點在列內最後一個位元組時，
/// 字組的高位元組寫進下一列的第 0 欄。來源列距 80。
fn or_word_rows(vga: &mut Vga, st: &mut State, c: &BlitCmd) {
    let src0 = scratch_src(c);
    for k in 0..4 {
        sel_plane(vga, k);
        let r = prep(st, c.y, c.cx, src0, c.page, c.x);
        let (mut bx, mut si, mut dx, mut di) = (r.bx, r.si, r.dx, r.di);
        for _ in 0..r.cx {
            if bx > 0xBD {
                break;
            }
            let lim = dx.wrapping_add(0x27);
            let slo = vga.read(si);
            let shi = vga.read(si.wrapping_add(1));
            if di <= lim {
                let dlo = vga.read(di);
                let dhi = vga.read(di.wrapping_add(1));
                vga.write(di, dlo | slo);
                vga.write(di.wrapping_add(1), dhi | shi);
            }
            dx = dx.wrapping_add(0x28);
            di = di.wrapping_add(0x28);
            si = si.wrapping_add(0x50);
            bx = bx.wrapping_add(1);
        }
    }
    restore(vga);
}

/// `sub_19FF9`（`sub_1A41D`）：GC 寫入模式 1，每列以閂鎖複製 2 位元組。
/// 每個位元組先 `lodsb`（載入閂鎖）再比對，`di` 大於列起點 + 27h 就停止這一列（來源已前進）。
/// 來源指標不還原：每列前進「實際 `lodsb` 的次數 + 4Eh」，所以任何一列在第一個位元組就被擋掉時，
/// 之後各列的來源少前進 1。結束時只把寫入模式設回 0，map mask 沿用目前值。
fn latch2_rows(vga: &mut Vga, st: &mut State, c: &BlitCmd) {
    vga.gc(5, 1);
    vga.gc(3, 0);
    let r = prep(st, c.y, c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut dx, mut di) = (r.bx, r.si, r.dx, r.di);
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        let lim = dx.wrapping_add(0x27);
        let mut d = di;
        for _ in 0..2 {
            vga.read(si);
            si = si.wrapping_add(1);
            if d > lim {
                break;
            }
            vga.write(d, 0);
            d = d.wrapping_add(1);
        }
        si = si.wrapping_add(0x4E);
        dx = dx.wrapping_add(0x28);
        di = di.wrapping_add(0x28);
        bx = bx.wrapping_add(1);
    }
    vga.gc(5, 0);
}

/// `sub_19C4F`：`sub_19BA4` 的垂直鏡射版。寫入模式 1，每列 40 位元組，目的列遞減（每列減 28h），
/// 來源列距 80，列號（`bx`）遞減，小於 0 就停（沒有 189 的上限）。`sub_19683` 把列數調成不足時，
/// 照常畫一列（`bx = 12Ch`），這一列沒有被 y 上限擋掉。結束時只把寫入模式設回 0，map mask 沿用目前值。
fn copy_rows_mirror(vga: &mut Vga, st: &mut State, c: &BlitCmd) {
    vga.gc(5, 1);
    vga.gc(3, 0);
    let r = prep(st, c.y, c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut di) = (r.bx, r.si, r.di);
    for _ in 0..r.cx {
        if (bx as i16) < 0 {
            break;
        }
        for i in 0..0x28u16 {
            vga.read(si.wrapping_add(i));
            vga.write(di.wrapping_add(i), 0);
        }
        di = di.wrapping_sub(0x28);
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_sub(1);
    }
    vga.gc(5, 0);
}

/// `sub_1A09F`（`sub_1A3C5`）：GC 寫入模式 0、功能 AND。來源位元組用 `lodsb` 從顯示記憶體讀，
/// 讀的是 GC 索引 4 目前選的平面（殘值，通常是上一次 `sub_1882E` 留下的平面 0），
/// 再 AND 到目的的四個平面。每列 40 位元組，來源列距 80。`mirror` 是 `sub_1A1E6`（`sub_1A3F1`）：
/// 目的列遞減（每列減 28h），其餘相同。
fn and_rows(vga: &mut Vga, st: &mut State, c: &BlitCmd, mirror: bool) {
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
        di = if mirror { di.wrapping_sub(0x28) } else { di.wrapping_add(0x28) };
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
    restore(vga);
}

/// `sub_1A144`（`sub_1A48F`）：AND 遮罩，每列只取 2 位元組。同 `sub_1A09F` 的讀寫方式，
/// 每個位元組先 `lodsb`（從讀取平面）再比對 `di` 與列起點 + 27h，大於就停止這一列。來源每列前進 80。
fn and2_rows(vga: &mut Vga, st: &mut State, c: &BlitCmd) {
    vga.gc(5, 0);
    vga.gc(3, 0x08);
    let r = prep(st, c.y, c.cx, scratch_src(c), c.page, c.x);
    let (mut bx, mut si, mut dx, mut di) = (r.bx, r.si, r.dx, r.di);
    for _ in 0..r.cx {
        if bx > 0xBD {
            break;
        }
        let lim = dx.wrapping_add(0x27);
        let mut d = di;
        for i in 0..2u16 {
            let b = vga.read(si.wrapping_add(i));
            if d > lim {
                break;
            }
            vga.read(d);
            vga.write(d, b);
            d = d.wrapping_add(1);
        }
        dx = dx.wrapping_add(0x28);
        di = di.wrapping_add(0x28);
        si = si.wrapping_add(0x50);
        bx = bx.wrapping_add(1);
    }
    restore(vga);
}

/// `sub_18ACE`（`sub_196E5`）：MAP 圖塊 8x10 寫進暫存區，四個平面各一次。每次先 `sub_18AA4` 載入項
/// （只用來設 `word_2687E`，高度不看），再固定 10 列、每列 `movsb` 一個位元組（來源連續，不看每列位元組數），
/// 目的每列加 50h。目的 = `A19A` 表的暫存區列位址 + `word_2685C` + x，不裁切。
fn map_tiles(vga: &mut Vga, st: &mut State, go: &GoFile, c: &BlitCmd) {
    for k in 0..4 {
        sel_plane(vga, k);
        match go.planes[k].get(c.index as usize) {
            Some(it) => {
                st.stride_reg = it.stride;
                st.max_scratch_row = st.max_scratch_row.max(c.y);
                let mut di = scratch_row(c.y).wrapping_add(c.scratch).wrapping_add(c.x);
                for row in 0..10usize {
                    let b = match it.data.get(row) {
                        Some(&b) => b,
                        None => {
                            st.anomaly(format!("{:?} index {} 圖塊資料不足 10 位元組", c.op, c.index));
                            0
                        }
                    };
                    vga.write(di, b);
                    di = di.wrapping_add(0x50);
                }
            }
            None => st.anomaly(format!("{:?} 平面 {k} 沒有 index {}", c.op, c.index)),
        }
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

/// 第三關捲動的來源起點：`sub_1A6A8(bx=0)` = `1F40h + word_2685C + word_26880`。
fn scroll_src(c: &BlitCmd) -> u16 {
    0x1F40u16.wrapping_add(c.scratch).wrapping_add(c.x)
}

/// `sub_17EBA`：GC 寫入模式 0、資料旋轉 4（`0403h`）、位元遮罩 FFh。`rep movsb` 3E80h 位元組（200 列）
/// 從來源（暫存區 A）到來源 + 8000h（暫存區 B），讀取平面與寫入平面是前一個 `sub_1882E` 選的單一平面。
/// 每個位元組旋轉 4 位元（高低半位元組對調）。結束時資料旋轉維持 4，由緊接的 `ScrollShiftLeft4` 改回 0。
fn scroll_rot_copy(vga: &mut Vga, c: &BlitCmd) {
    vga.gc(5, 0);
    vga.gc(3, 0x04);
    vga.gc(8, 0xFF);
    let src = scroll_src(c);
    let dst = src.wrapping_add(0x8000);
    for i in 0..0x3E80u16 {
        let b = vga.read(src.wrapping_add(i));
        vga.write(dst.wrapping_add(i), b);
    }
}

/// `sub_17EEA`：資料旋轉 0、位元遮罩 0Fh。目的 = 來源 + 8000h，來源讀位元組 `di + 1`（讀取平面），
/// 先讀 `[di]` 載入閂鎖再寫 `di`：低半位元組取自來源、高半位元組取自閂鎖，重複 3E80h 次。
/// 結果是暫存區 B 相對 A 左移 4 像素。最後一輪讀到 B 的 3E80h 偏移（複製範圍外的殘值），照讀。
/// 結束時位元遮罩設回 FFh。
fn scroll_shift_merge(vga: &mut Vga, c: &BlitCmd) {
    vga.gc(5, 0);
    vga.gc(3, 0);
    vga.gc(8, 0x0F);
    let mut di = scroll_src(c).wrapping_add(0x8000);
    let mut si = di.wrapping_add(1);
    for _ in 0..0x3E80u16 {
        let al = vga.read(si);
        si = si.wrapping_add(1);
        vga.read(di);
        vga.write(di, al);
        di = di.wrapping_add(1);
    }
    vga.gc(8, 0xFF);
}

/// `sub_17F26`：同 `sub_17EBA` 的設定，但只複製一欄：200 列各 `movsb` 一個位元組（旋轉 4），列距 80。
/// 結束時資料旋轉維持 4，由緊接的 `ScrollMergeSeam` 改回 0。
fn scroll_col_rot(vga: &mut Vga, c: &BlitCmd) {
    vga.gc(5, 0);
    vga.gc(3, 0x04);
    vga.gc(8, 0xFF);
    let mut si = scroll_src(c);
    let mut di = si.wrapping_add(0x8000);
    for _ in 0..0xC8 {
        let b = vga.read(si);
        vga.write(di, b);
        si = si.wrapping_add(0x50);
        di = di.wrapping_add(0x50);
    }
}

/// `sub_17F5D`：`sub_17EEA` 的一欄版本。目的 = 來源 + 8000h - 1（左邊一欄），來源 = 目的 + 1，200 列，列距 80。
/// 結束時位元遮罩設回 FFh。
fn scroll_col_merge(vga: &mut Vga, c: &BlitCmd) {
    vga.gc(5, 0);
    vga.gc(3, 0);
    vga.gc(8, 0x0F);
    let mut di = scroll_src(c).wrapping_add(0x8000).wrapping_sub(1);
    let mut si = di.wrapping_add(1);
    for _ in 0..0xC8 {
        let al = vga.read(si);
        vga.read(di);
        vga.write(di, al);
        si = si.wrapping_add(0x50);
        di = di.wrapping_add(0x50);
    }
    vga.gc(8, 0xFF);
}

/// `sub_17FA0`（暫存區 A，`word_2685C` = 0）與 `sub_18031`（暫存區 B，8000h）：寫入模式 1，190 列各以閂鎖複製 40 位元組
/// 到顯示頁（`word_26872` 起連續，列距 28h），來源從暫存區第 0 列的第 `word_25BC4` 欄起，列距 80。
/// 寫入的平面由目前的 map mask 決定（呼叫端是 0Fh）。不裁切。
fn scroll_view(vga: &mut Vga, c: &BlitCmd) {
    vga.gc(5, 1);
    vga.gc(3, 0);
    let mut si = scroll_src(c);
    let mut di = c.page;
    for _ in 0..0xBE {
        for i in 0..0x28u16 {
            vga.read(si.wrapping_add(i));
            vga.write(di.wrapping_add(i), 0);
        }
        si = si.wrapping_add(0x50);
        di = di.wrapping_add(0x28);
    }
    vga.gc(5, 0);
}

/// `sub_180C6` 進位時的搬移：暫存區 A（`1F40h`）與 B（`9F40h`）各整體左移 28h 位元組。寫入模式 1，
/// 順向逐位元組複製 3B60h 位元組，來源領先目的 28h。x、scratch 在函式內固定設 0，不看命令的欄位。
fn scroll_left(vga: &mut Vga) {
    vga.gc(5, 1);
    vga.gc(3, 0);
    for base in [0x1F40u16, 0x1F40u16.wrapping_add(0x8000)] {
        for i in 0..0x3B60u16 {
            vga.read(base.wrapping_add(0x28).wrapping_add(i));
            vga.write(base.wrapping_add(i), 0);
        }
    }
    vga.gc(5, 0);
}

/// `sub_1152D`（`base` = 8000h，`A800:1F40`）與 `sub_11504`（`base` = 0，`A000:1F40`）：
/// 從 `base + 1F40h` 起填 `20BFh` 個字組的 0，也就是 `base + 1F40h` 起 `417Eh` 位元組。
/// 沒有 `sub_18878`，寫入的平面由目前的 map mask 決定。
fn clear_scratch(vga: &mut Vga, base: u16) {
    vram_fill(vga, base.wrapping_add(0x1F40), 0x20BF, 0);
}

/// `sub_18894` 的 E 分支（`sub_18878` 加 `sub_188B1`）：map mask 設 0Fh，兩頁各填 `0FA0h` 個字組的 0
/// （頁 0 的 `0000h` 起與頁 1 的 `8000h` 起各 8000 位元組，也就是 200 列，不含暫存區）。
fn clear_both_pages(vga: &mut Vga) {
    restore(vga);
    vram_fill(vga, 0, 0x0FA0, 0);
    vram_fill(vga, 0x8000, 0x0FA0, 0);
}

/// 幀計數的位置：`sub_1100A` 把 `word_2688A` 設成 12h、`word_2688E` 設成 BFh。
/// `BlitCmd` 沒有帶這兩個變數，所以固定用這組值。`sub_16C7D` 的雷射擊中數字用別的位置，見回報。
pub(crate) const COUNTER_X: u16 = 0x12;
pub(crate) const COUNTER_Y: u16 = 0xBF;

/// `sub_1890D`：5 位數字寫進平面 3，從 `word_2688A` 欄起 5 欄、`word_2688E` 列起 8 列，
/// 每列同時寫兩頁（`es:[di]` 與 `es:[di - 8000h]`）。
/// 字型：原版讀 BIOS 字型 `F000:FA6Eh + 字元 * 8`（`sub_18A61`，字元是數字的 ASCII 碼）。
/// `font` 是 2048 位元組的 BIOS 字型時照原版畫，否則用重製自繪的數字字型。
fn counter(vga: &mut Vga, col: u16, row: u16, value: u16, font: Option<&[u8]>) {
    vga.seq_map_mask(8);
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
