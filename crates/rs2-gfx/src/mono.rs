//! `MonoSink`：單色顯示模式 C（CGA，BIOS 模式 6）與 H（MGA，Hercules）的接收端。
//!
//! 吃的是 `rs2-core` 為 E 模式（VGA 與 EGA）記下的同一批繪製命令，逐位元組重現原版 C、H 的顯示記憶體與背景暫存區。
//! 規格見 `docs/spec/006-display-modes.md`，判斷點盤點見 `docs/re/14-display-modes.md`。
//!
//! # 原版的單色路徑
//!
//! `byte_1CF74`（M）不是 `E` 時，每個 blit 包裝（`sub_18C0A` 等）把 x 乘 2（位元組，16 像素）、
//! 換成單色圖庫的段，呼叫字組版的底層：
//!
//! | 運算 | 底層 | 命令 |
//! |---|---|---|
//! | 覆寫 | `sub_19892` | `S18C0A`、`S18FC1`、`S19068` |
//! | 取反覆寫 | `sub_19840` | `S18C6E` |
//! | OR | `sub_1979F` | `S18CE2`、`S18DCC`、`S18EB6`、`S18F5D` |
//! | 取反 OR | `sub_197EE` | `S18D89`、`S19025`、`S190CC` |
//! | AND（遮罩） | `sub_1974B` | `S18D46`、`S18E30`、`S18E73`、`S18F1A` |
//!
//! 背景暫存區 A、B 在主記憶體（段值 `word_1CF70`、`word_1CF72`，各 0D00h 段落），列距 160 位元組
//! （列位址表 `dseg:A4BAh`，`n × 0A0h`）。`sub_1A70A` 依 `word_2685C` 選 A（0）或 B（8000h）。
//!
//! # 顯示記憶體
//!
//! | | C | H |
//! |---|---|---|
//! | 段 | B800h | B000h |
//! | 列位址（`dseg:9E7Ah`、`9CEAh`） | 偶數列 `n/2 × 50h`，奇數列加 2000h | 偶數列 `n/2 × 50h`，奇數列加 4000h |
//! | 第二頁 | B800:4000 | B000:8000 |
//! | 範圍 | 32 KB | 64 KB |
//!
//! 核心記的頁偏移是 E 的 `8000h`／0，C 的第二頁是 `4000h`，這裡換算。
//! 超出範圍的寫入（C 的 8000h 以上是 EGA／VGA 的 ROM 區）丟掉。
//!
//! # 偏移與環繞
//!
//! 偏移是段內的 16 位元值，運算一律環繞，與 8086 的 `di`、`si` 相同。

use crate::font::DIGITS;
use crate::sink::{COUNTER_X, COUNTER_Y};
use rs2_core::{BegGroup, BlitCmd, DrawCmd, Op, ShtGroup};
use rs2_formats::{check_hash, sha256_hex, unrle};
use std::path::Path;
use std::sync::Arc;

/// 單色圖庫的預期 SHA-256，來源是 `source-manifest.json`。
pub const MONO_EXPECTED: [(&str, &str); 21] = [
    ("MY.BDB", "c2886e6a8ab577f0292cf004601ed5ae56a906767092af40ed04dc87210c1102"),
    ("MYB.BDT", "97694cf45afc304e4109b3494523ef04071aa23ccc49b11c83ae8eb89fcd1afa"),
    ("SHT1.BDB", "990bcb4ec96fe5a4bade14122f4b2b198400f1d073d6d754ff19079e5311fc7f"),
    ("SHT1.BDH", "56d4c67de04e454f0b84d117f2f321278cd278ca1d96a4f445fcb464dda6f03e"),
    ("SHT2.BDB", "e5d88af552b6804fdc3b274bd137d39b36786cf331a8b319a322ba6020288e67"),
    ("SHT2.BDH", "a67745236dede25261e07bd4afa2011e6e713fc37d1c6c5049a58c1d3e05c5a5"),
    ("SHT3.BDB", "df4a9fc550e63577280258af8e85f89a3b14fda0bb8e828cc486b13e5eeedc84"),
    ("SHT3.BDH", "44e405468f8d27121ad7aeded2d00af09fbe4d0ec7a6e20f934d1f68657f7c5c"),
    ("SHT4.BDB", "93be3ee4b3a7af1a7714cc5368a483ed1705de5688fba74a757065a6949b84a3"),
    ("SHT4.BDH", "51d52369678ec46de497c4bd79c3f8eb39137f0c6d93f7649ba72d2e5ec34a40"),
    ("SHT5.BDB", "1bb97797284307c04e606754a89ae0953538af8ac8364ecf4e2c20a26ef72083"),
    ("SHT5.BDH", "7b1b076665b536a2e45e3332176f26690223e29930dcdb910337dea5bdbc7890"),
    ("BH1.BDB", "ae0bff90b6508437129f87a426d326fb6167754939755971157ff759ddb503d0"),
    ("BH2.BDB", "b6fdebe22bad963cb8575c0f082ced72c7ddc09915622159ab99af902748bb3e"),
    ("BH4.BDB", "f3f06b639c6112f943f0ca0b96230dc2b23b96583974dd7cf66f9b317766d79c"),
    ("MAP.BDB", "472be4406153c7e354f00364887a562f5684705c5d9dbcb2655262b7933393d8"),
    ("J1.GOB", "6f091dfe397cf97b441d4972e584306fca26eb552c986fe61f97476211f3d26f"),
    ("J2.GOB", "4c854ce091a03ad31d63bec13a13fc1f9bff3c9f8deb51e5a1c7632ac6eed605"),
    ("J3.GOB", "409cdcae97e5988eebc612f8fa247ec679cc9b61265d7ae903c2d7ab9803aa2f"),
    ("J4.GOB", "4bae7ead698e534fa21a8b13dc2c7b80b1efed545c1977f52c3041afecd489b7"),
    ("BEG1.BDB", "077be33d74d182aa7bace9137b6ce4e97bd8c963637e699748730679492f6a93"),
];

/// 背景暫存區 A、B 的大小：`sub_10162` 各配置 0D00h 段落。
pub const SCRATCH_SIZE: usize = 0xD000;

/// 單色畫面的種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MonoKind {
    /// CGA，BIOS 模式 6，640×200。
    Cga,
    /// MGA（Hercules），本作寫的 CRTC 是 R1=28h、R6=64h、R9=02h，640×300。
    Mga,
}

impl MonoKind {
    /// 顯示記憶體在段內的有效範圍（位元組）。
    pub fn mem_size(self) -> usize {
        match self {
            MonoKind::Cga => 0x8000,
            MonoKind::Mga => 0x10000,
        }
    }

    /// 畫面像素尺寸。H 是 CRTC 的 R6 × (R9 + 1) 條掃描線。
    pub fn size(self) -> (usize, usize) {
        match self {
            MonoKind::Cga => (640, 200),
            MonoKind::Mga => (640, 300),
        }
    }
}

/// 一個圖庫：解壓後的位元組，與原版放在段落裡的內容相同。
/// 開頭是每項的位移表，每項是 `[高 u16][每列位元組數 u16][像素]`，尾端是 `FF FF 00 00`。
/// 越界的圖號與來源指標照原版讀同一段內的其他位元組，所以保留原始位元組而不是拆成項。
#[derive(Clone, Debug, Default)]
pub struct MonoBank {
    pub(crate) raw: Vec<u8>,
}

/// `sub_18AA4` 的結果。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Item {
    pub(crate) height: u16,
    pub(crate) stride: u16,
    /// 像素資料在圖庫內的位移。
    pub(crate) data: usize,
}

impl MonoBank {
    /// 解 `[u16 長度][RLE]`（單獨成檔的圖庫）。
    pub fn from_file(data: &[u8]) -> Result<MonoBank, String> {
        if data.len() < 2 {
            return Err("圖庫檔太短".into());
        }
        let raw = unrle(data, 0);
        if raw.len() < 4 {
            return Err("圖庫解開後太短".into());
        }
        let b = MonoBank { raw };
        if b.rd16(0).is_none_or(|n| n % 2 != 0 || n as usize > b.raw.len()) {
            return Err("圖庫的位移表長度不合".into());
        }
        Ok(b)
    }

    /// 項數（位移表第一個字組除以 2）。
    pub fn len(&self) -> usize {
        self.rd16(0).map_or(0, |n| n as usize / 2)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 第 `index` 項的 `(高, 每列位元組數)`，圖號超出圖庫回 `None`。診斷與 HD 圖尺寸比對用。
    pub fn dims(&self, index: u16) -> Option<(u16, u16)> {
        self.item(index).map(|it| (it.height, it.stride))
    }

    pub(crate) fn rd8(&self, off: usize) -> Option<u8> {
        self.raw.get(off).copied()
    }

    fn rd16(&self, off: usize) -> Option<u16> {
        Some(u16::from_le_bytes([*self.raw.get(off)?, *self.raw.get(off + 1)?]))
    }

    /// `sub_18AA4`：`bx = 圖號 × 2`（16 位元環繞），取指標、高、每列位元組數。
    pub(crate) fn item(&self, index: u16) -> Option<Item> {
        let p = self.rd16(index.wrapping_mul(2) as usize)? as usize;
        Some(Item { height: self.rd16(p)?, stride: self.rd16(p + 2)?, data: p + 4 })
    }
}

/// 單色版的全部圖庫。
#[derive(Clone, Debug, Default)]
pub struct MonoAssets {
    /// `MY.BDB`（`word_1CF52`）。
    pub my: MonoBank,
    /// `MYB.BDT`（`word_1CF5A`）。
    pub my_mask: MonoBank,
    /// `SHT1.BDB`～`SHT5.BDB`（`word_1CF5E`）。
    pub sht: [MonoBank; 5],
    /// `SHT1.BDH`～`SHT5.BDH`（`word_1CF5C`）。
    pub sht_mask: [MonoBank; 5],
    /// `BH1.BDB`、`BH2.BDB`、`BH4.BDB`，與 `sht` 共用 `word_1CF5E`。
    pub bh: [MonoBank; 3],
    /// `MAP.BDB`（`word_1CF66`），第三關的 16×10 圖塊。
    pub map: MonoBank,
    /// `J1.GOB`～`J4.GOB`（`word_1CF46`）。
    pub j: [MonoBank; 4],
    /// `BEG1.BDB`（`word_1CF46`）。
    pub beg1: MonoBank,
    /// 載入時雜湊沒有預期值的檔名。
    pub unverified: Vec<String>,
}

impl MonoAssets {
    /// 讀 21 個單色圖庫，先驗 SHA-256 再解碼，雜湊不符就拒絕。
    pub fn load(dir: &Path) -> Result<MonoAssets, String> {
        let mut unverified = Vec::new();
        let mut bank = |name: &str| -> Result<MonoBank, String> {
            let data = std::fs::read(dir.join(name)).map_err(|e| format!("讀不到 {name}：{e}"))?;
            match MONO_EXPECTED.iter().find(|(n, _)| *n == name) {
                Some((_, want)) => check_hash(name, &data, want)?,
                None => unverified.push(name.to_string()),
            }
            MonoBank::from_file(&data).map_err(|e| format!("{name} 解碼失敗：{e}"))
        };
        let my = bank("MY.BDB")?;
        let my_mask = bank("MYB.BDT")?;
        let mut sht: [MonoBank; 5] = Default::default();
        let mut sht_mask: [MonoBank; 5] = Default::default();
        for n in 0..5 {
            sht[n] = bank(&format!("SHT{}.BDB", n + 1))?;
            sht_mask[n] = bank(&format!("SHT{}.BDH", n + 1))?;
        }
        let bh = [bank("BH1.BDB")?, bank("BH2.BDB")?, bank("BH4.BDB")?];
        let map = bank("MAP.BDB")?;
        let j = [bank("J1.GOB")?, bank("J2.GOB")?, bank("J3.GOB")?, bank("J4.GOB")?];
        let beg1 = bank("BEG1.BDB")?;
        Ok(MonoAssets { my, my_mask, sht, sht_mask, bh, map, j, beg1, unverified })
    }

    /// `word_1CF5E` 目前指向的圖庫。
    pub(crate) fn sht(&self, g: ShtGroup) -> &MonoBank {
        match g {
            ShtGroup::Bh1 => &self.bh[0],
            ShtGroup::Bh2 => &self.bh[1],
            ShtGroup::Bh4 => &self.bh[2],
            ShtGroup::Sht1 => &self.sht[0],
            ShtGroup::Sht2 => &self.sht[1],
            ShtGroup::Sht3 => &self.sht[2],
            ShtGroup::Sht4 => &self.sht[3],
            ShtGroup::Sht5 => &self.sht[4],
        }
    }

    /// `word_1CF5C` 目前指向的遮罩庫。BH 沒有遮罩，載入 BH 不會蓋掉它，所以 `mask_group` 一定是 SHT。
    pub(crate) fn sht_mask(&self, g: ShtGroup) -> Option<&MonoBank> {
        match g {
            ShtGroup::Sht1 => Some(&self.sht_mask[0]),
            ShtGroup::Sht2 => Some(&self.sht_mask[1]),
            ShtGroup::Sht3 => Some(&self.sht_mask[2]),
            ShtGroup::Sht4 => Some(&self.sht_mask[3]),
            ShtGroup::Sht5 => Some(&self.sht_mask[4]),
            ShtGroup::Bh1 | ShtGroup::Bh2 | ShtGroup::Bh4 => None,
        }
    }

    /// `word_1CF46` 目前指向的圖庫。
    pub(crate) fn beg(&self, g: BegGroup) -> &MonoBank {
        match g {
            BegGroup::Beg1 => &self.beg1,
            BegGroup::J1 => &self.j[0],
            BegGroup::J2 => &self.j[1],
            BegGroup::J3 => &self.j[2],
            BegGroup::J4 => &self.j[3],
        }
    }
}

/// 字組寫入的方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Put {
    /// `mov es:[di], ax`（`sub_19892`）。
    Set,
    /// `xor ax, 0FFFFh; mov es:[di], ax`（`sub_19840`）。
    SetInv,
    /// `or es:[di], ax`（`sub_1979F`）。
    Or,
    /// `xor ax, 0FFFFh; or es:[di], ax`（`sub_197EE`）。
    OrInv,
    /// `and es:[di], ax`（`sub_1974B`）。
    And,
}

/// 背景暫存區的選擇。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scratch {
    A,
    B,
}

/// `sub_19979` 的結果。
struct Row {
    /// 寫入點（`di`）：列起點加 `word_26880`。
    di: u16,
}

#[derive(Clone)]
pub struct MonoSink {
    kind: MonoKind,
    assets: Arc<MonoAssets>,
    /// 顯示記憶體，從段內位移 0 起，C 用前 8000h 位元組。
    vmem: Vec<u8>,
    sa: Vec<u8>,
    sb: Vec<u8>,
    /// 幀計數預設不畫。設了 BIOS 字型（`set_counter_font`）之後永遠畫。
    pub show_counter: bool,
    /// BIOS 8x8 字型 2048 位元組，`sub_18A61` 讀的來源。
    counter_font: Option<Vec<u8>>,
    /// 第三關進場的 `S17FA0` 之前的顯示頁內容，見 `stage3_view`。
    undo: Option<(usize, Vec<u8>)>,
    anomalies: u64,
    log: Vec<String>,
    seq: u64,
}

impl MonoSink {
    /// 顯示記憶體與兩塊暫存區都是 0。
    pub fn new(kind: MonoKind, assets: Arc<MonoAssets>) -> MonoSink {
        MonoSink {
            kind,
            assets,
            vmem: vec![0; 0x10000],
            sa: vec![0; SCRATCH_SIZE],
            sb: vec![0; SCRATCH_SIZE],
            show_counter: false,
            counter_font: None,
            undo: None,
            anomalies: 0,
            log: Vec::new(),
            seq: 0,
        }
    }

    pub fn kind(&self) -> MonoKind {
        self.kind
    }

    /// 用原版某個時點的顯示記憶體與暫存區當起點（對拍用）。`vmem` 是 `B0000..BFFFF` 的 64 KB（`rs2dyn dumpmono` 的 `.mono`），
    /// C 只取 `B8000..BFFFF` 的 32 KB；`sa`、`sb` 是兩塊暫存區（`.msa`、`.msb`，各 `0D000h` 位元組）。
    ///
    /// # Panics
    /// 長度不符。
    pub fn load_snapshot(&mut self, vmem: &[u8], sa: &[u8], sb: &[u8]) {
        assert_eq!(vmem.len(), 0x10000, "顯示記憶體傾印應為 64 KB（{} 位元組）", vmem.len());
        assert_eq!(sa.len(), SCRATCH_SIZE, "暫存區 A 傾印長度 {}", sa.len());
        assert_eq!(sb.len(), SCRATCH_SIZE, "暫存區 B 傾印長度 {}", sb.len());
        self.vmem = match self.kind {
            MonoKind::Cga => {
                let mut v = vec![0; 0x10000];
                v[..0x8000].copy_from_slice(&vmem[0x8000..]);
                v
            }
            MonoKind::Mga => vmem.to_vec(),
        };
        self.sa = sa.to_vec();
        self.sb = sb.to_vec();
        self.undo = None;
    }

    /// BIOS 8x8 字型（2048 位元組）。設了之後幀計數照原版用這份字型畫，並且永遠畫。
    ///
    /// # Panics
    /// 長度不是 2048。
    pub fn set_counter_font(&mut self, font: &[u8]) {
        assert_eq!(font.len(), 2048, "BIOS 字型應為 2048 位元組（256 字元 x 8 列）");
        self.counter_font = Some(font.to_vec());
    }

    /// 顯示記憶體整塊的 SHA-256，對應 `waittrace` 的 `mono`（C 是 `B8000..BFFFF`，H 是 `B0000..BFFFF`）。
    pub fn vram_sha256(&self) -> String {
        sha256_hex(&self.vmem[..self.kind.mem_size()])
    }

    /// 背景暫存區 A 的 SHA-256，對應 `msa`。
    pub fn scratch_a_sha256(&self) -> String {
        sha256_hex(&self.sa)
    }

    /// 背景暫存區 B 的 SHA-256，對應 `msb`。
    pub fn scratch_b_sha256(&self) -> String {
        sha256_hex(&self.sb)
    }

    /// 顯示記憶體（C 是 32 KB，H 是 64 KB）。
    pub fn vram(&self) -> &[u8] {
        &self.vmem[..self.kind.mem_size()]
    }

    pub fn scratch_a(&self) -> &[u8] {
        &self.sa
    }

    pub fn scratch_b(&self) -> &[u8] {
        &self.sb
    }

    /// 已套用的命令數。
    pub fn applied(&self) -> u64 {
        self.seq
    }

    /// 無法照機器碼重現的次數（圖號超出圖庫、來源超出圖庫位元組等）。
    pub fn anomalies(&self) -> u64 {
        self.anomalies
    }

    pub fn anomaly_log(&self) -> &[String] {
        &self.log
    }

    fn anomaly(&mut self, msg: String) {
        self.anomalies += 1;
        if self.log.len() < 64 {
            self.log.push(msg);
        }
    }

    // ---- 位址與記憶體 ----

    /// 核心的頁偏移（E 的 `8000h`／0）換成這個模式的：C 的第二頁在 `4000h`。
    fn page(&self, core_page: u16) -> u16 {
        match self.kind {
            MonoKind::Cga => {
                if core_page == 0 {
                    0
                } else {
                    0x4000
                }
            }
            MonoKind::Mga => core_page,
        }
    }

    /// 列位址表（C 的 `dseg:9E7Ah`、H 的 `9CEAh`）：偶數列 `n/2 × 50h`，奇數列加 2000h（C）或 4000h（H）。
    /// 表只有 200 項，超過的索引原版讀到 E 的暫存區列表，這裡照同一條公式算：那些列不會被寫入（y 上限 190）。
    fn row(&self, bx: u16) -> u16 {
        let odd = match self.kind {
            MonoKind::Cga => 0x2000,
            MonoKind::Mga => 0x4000,
        };
        (bx >> 1).wrapping_mul(0x50).wrapping_add(if bx & 1 != 0 { odd } else { 0 })
    }

    /// 暫存區列位址表（`dseg:A4BAh`）：`n × 0A0h`。
    fn scratch_row(n: u16) -> u16 {
        n.wrapping_mul(0xA0)
    }

    fn vget(&self, off: u16) -> u8 {
        if (off as usize) < self.kind.mem_size() { self.vmem[off as usize] } else { 0 }
    }

    fn vput(&mut self, off: u16, v: u8) {
        if (off as usize) < self.kind.mem_size() {
            self.vmem[off as usize] = v;
        }
    }

    fn vget16(&self, off: u16) -> u16 {
        u16::from_le_bytes([self.vget(off), self.vget(off.wrapping_add(1))])
    }

    fn vput16(&mut self, off: u16, v: u16) {
        self.vput(off, v as u8);
        self.vput(off.wrapping_add(1), (v >> 8) as u8);
    }

    fn scratch(&self, s: Scratch) -> &[u8] {
        match s {
            Scratch::A => &self.sa,
            Scratch::B => &self.sb,
        }
    }

    fn sget(&self, s: Scratch, off: u16) -> u8 {
        self.scratch(s).get(off as usize).copied().unwrap_or(0)
    }

    fn sput(&mut self, s: Scratch, off: u16, v: u8) {
        let buf = match s {
            Scratch::A => &mut self.sa,
            Scratch::B => &mut self.sb,
        };
        if let Some(b) = buf.get_mut(off as usize) {
            *b = v;
        }
    }

    fn sget16(&self, s: Scratch, off: u16) -> u16 {
        u16::from_le_bytes([self.sget(s, off), self.sget(s, off.wrapping_add(1))])
    }

    /// `sub_1A70A`：`word_2685C` 為 8000h 選 B，否則 A。
    fn pick(c: &BlitCmd) -> Scratch {
        if c.scratch == 0x8000 { Scratch::B } else { Scratch::A }
    }

    /// `sub_19979`（與 `sub_1995E`、`sub_19683` 同型）：bx < 0 的列跳過（bx 加 1、cx 減 1、si 加 `stride`），
    /// 再查列位址表算出列起點與寫入點；cx ≤ 0（有號）時 cx = 1、bx = 12Ch，使後面的列迴圈被 y 上限跳過。
    fn p19979(&self, bx: &mut u16, cx: &mut u16, si: &mut u16, stride: u16, page: u16, x: u16) -> Row {
        while (*bx as i16) < 0 {
            *bx = bx.wrapping_add(1);
            *cx = cx.wrapping_sub(1);
            *si = si.wrapping_add(stride);
        }
        let start = self.row(*bx).wrapping_add(page);
        if (*cx as i16) <= 0 {
            *cx = 1;
            *bx = 0x12C;
        }
        Row { di: start.wrapping_add(x) }
    }

    // ---- 圖庫 blit ----

    /// 字組運算。
    fn put_word(&mut self, di: u16, src: u16, put: Put) {
        let v = match put {
            Put::Set => src,
            Put::SetInv => !src,
            Put::Or => self.vget16(di) | src,
            Put::OrInv => self.vget16(di) | !src,
            Put::And => self.vget16(di) & src,
        };
        self.vput16(di, v);
    }

    /// `sub_1974B`、`sub_1979F`、`sub_197EE`、`sub_19840`、`sub_19892`：`sub_18AA4` 取項，`sub_1995E` 裁上緣，
    /// 逐列逐字組寫。字組的位址（`di`）超出列起點到列起點 + 4Fh 的範圍，或列號超過 190（`0BEh`，有號比較），
    /// 整個字組不寫，來源照樣前進 2 位元組。每列字組數是 `每列位元組數 >> 1`，為 0 時 `loop` 會轉 65536 次。
    fn blit(&mut self, bank: &MonoBank, c: &BlitCmd, put: Put) {
        let Some(it) = bank.item(c.index) else {
            self.anomaly(format!("{:?} 圖號 {} 超出圖庫", c.op, c.index));
            return;
        };
        let page = self.page(c.page);
        let x = c.x.wrapping_mul(2);
        let mut bx = c.y;
        let mut cx = it.height;
        let mut si = 0u16;
        // sub_1995E：先裁一次上緣。
        while (bx as i16) < 0 {
            bx = bx.wrapping_add(1);
            cx = cx.wrapping_sub(1);
            si = si.wrapping_add(it.stride);
        }
        if (cx as i16) <= 0 {
            cx = 1;
            bx = 0x12C;
        }
        let mut src = it.data.wrapping_add(si as usize);
        let words = it.stride >> 1;
        let n_words = if words == 0 { 0x10000u32 } else { words as u32 };
        let n_rows = if cx == 0 { 0x10000u32 } else { cx as u32 };
        let mut oob_reported = false;
        for _ in 0..n_rows {
            let start = self.row(bx).wrapping_add(page);
            let lim = start.wrapping_add(0x4F);
            let mut di = start.wrapping_add(x);
            let row_ok = (bx as i16) <= 0xBE;
            for _ in 0..n_words {
                let in_row = (di as i16) <= (lim as i16) && (di as i16) >= (start as i16);
                if in_row && row_ok {
                    let w = match (bank.rd8(src), bank.rd8(src + 1)) {
                        (Some(a), Some(b)) => u16::from_le_bytes([a, b]),
                        _ => {
                            if !oob_reported {
                                oob_reported = true;
                                self.anomaly(format!("{:?} 圖號 {} 來源超出圖庫", c.op, c.index));
                            }
                            0
                        }
                    };
                    self.put_word(di, w, put);
                }
                src += 2;
                di = di.wrapping_add(2);
            }
            bx = bx.wrapping_add(1);
        }
    }

    /// `sub_1A6CE`（`S19ADC`、`S19B40`）：圖庫項寫進背景暫存區。不裁切，每列 `每列位元組數 >> 1` 個字組，
    /// 目的是 `dseg:A4BAh` 的列位址加 x（位元組，已乘 2），來源每列前進 `2 × 字組數`。
    fn to_scratch(&mut self, bank: &MonoBank, c: &BlitCmd) {
        let Some(it) = bank.item(c.index) else {
            self.anomaly(format!("{:?} 圖號 {} 超出圖庫", c.op, c.index));
            return;
        };
        let s = Self::pick(c);
        let x = c.x.wrapping_mul(2);
        let words = (it.stride >> 1) as usize;
        let n_rows = if it.height == 0 { 0x10000u32 } else { it.height as u32 };
        let mut src = it.data;
        let mut bx = c.y;
        let mut oob_reported = false;
        for _ in 0..n_rows {
            let mut di = Self::scratch_row(bx).wrapping_add(x);
            for _ in 0..words {
                for _ in 0..2 {
                    let b = match bank.rd8(src) {
                        Some(b) => b,
                        None => {
                            if !oob_reported {
                                oob_reported = true;
                                self.anomaly(format!("{:?} 圖號 {} 來源超出圖庫", c.op, c.index));
                            }
                            0
                        }
                    };
                    self.sput(s, di, b);
                    di = di.wrapping_add(1);
                    src += 1;
                }
            }
            bx = bx.wrapping_add(1);
        }
    }

    /// `sub_18ACE` 的單色分支（`sub_19706`）：MAP 圖塊 16×10 寫進暫存區。固定 10 列、每列 `movsw` 一個字組
    /// （來源連續，不看每列位元組數），目的每列加 0A0h。x 乘 2。
    fn map_tile(&mut self, c: &BlitCmd) {
        let assets = Arc::clone(&self.assets);
        let Some(it) = assets.map.item(c.index) else {
            self.anomaly(format!("S18ACE 圖號 {} 超出 MAP.BDB", c.index));
            return;
        };
        let s = Self::pick(c);
        let mut di = Self::scratch_row(c.y).wrapping_add(c.x.wrapping_mul(2));
        let mut src = it.data;
        for _ in 0..10 {
            for k in 0..2u16 {
                let b = assets.map.rd8(src).unwrap_or_else(|| {
                    self.anomaly(format!("S18ACE 圖號 {} 圖塊資料不足 20 位元組", c.index));
                    0
                });
                self.sput(s, di.wrapping_add(k), b);
                src += 1;
            }
            di = di.wrapping_add(0xA0);
        }
    }

    // ---- 暫存區搬到顯示頁 ----

    /// 暫存區來源指標：`(word_26866 的低位元組) × 0A0h + word_26864 × 2 + word_26868 × 2`。
    /// 原版用 `mul bl`，只取 `word_26866` 的低位元組。`src_x_doubled` 為假時 `word_26864` 不乘 2（`sub_1A1E6` 的單色分支就是這樣）。
    fn scratch_src(c: &BlitCmd, src_x_doubled: bool) -> u16 {
        let sx = if src_x_doubled { c.src_x.wrapping_mul(2) } else { c.src_x };
        (c.src_y & 0xFF).wrapping_mul(0xA0).wrapping_add(sx).wrapping_add(c.shift.wrapping_mul(2))
    }

    /// 暫存區整列（80 位元組）運算到顯示頁：`S19BA4`（覆寫）、`S19C4F`（覆寫，列號遞減）、`S19DA8`（OR）、
    /// `S19F3B`（OR，遞減）、`S1A09F`（AND）、`S1A1E6`（AND，遞減）。迴圈條件 `cmp bx, 0BDh; ja` 是無號比較，
    /// 所以負的列號直接結束；cx ≤ 0 時只畫一列。每列來源前進 0A0h。
    fn rows80(&mut self, c: &BlitCmd, put: Put, down: bool, x: u16, src_x_doubled: bool) {
        let s = Self::pick(c);
        let page = self.page(c.page);
        let mut si = Self::scratch_src(c, src_x_doubled);
        let mut bx = c.y;
        let n_rows = if (c.cx as i16) <= 0 { 1u32 } else { c.cx as u32 };
        for _ in 0..n_rows {
            if bx > 0xBD {
                break;
            }
            let start = self.row(bx).wrapping_add(page);
            let mut di = start.wrapping_add(x);
            for k in 0..0x28u16 {
                let w = self.sget16(s, si.wrapping_add(2 * k));
                self.put_word(di, w, put);
                di = di.wrapping_add(2);
            }
            si = si.wrapping_add(0xA0);
            bx = if down { bx.wrapping_sub(1) } else { bx.wrapping_add(1) };
        }
    }

    /// `S19D08`（`loc_19D46`）：同 `S19BA4`，另有水平位移。位移也加進來源指標；每個字組先取來源再看計數，
    /// 計數（`word_26868` 起，每字組加 1）≥ 50h 的字組不寫。`sub_19979` 在判斷列號之前呼叫，
    /// 所以負的 y 照 `word_2687E = 0A0h` 跳過上緣。
    fn rows80_shift(&mut self, c: &BlitCmd) {
        let s = Self::pick(c);
        let page = self.page(c.page);
        let mut si = Self::scratch_src(c, true);
        let mut bx = c.y;
        let mut cx = c.cx;
        for _ in 0..0x10000u32 {
            // cx 被 `sub_19979` 改過的值（跳過的上緣列、cx ≤ 0 時的 1）留到 `loop`，不還原。
            let row = self.p19979(&mut bx, &mut cx, &mut si, 0xA0, page, c.x);
            if bx > 0xBD {
                break;
            }
            let mut di = row.di;
            let mut cnt = c.shift;
            for k in 0..0x28u16 {
                let w = self.sget16(s, si.wrapping_add(2 * k));
                if cnt < 0x50 {
                    self.vput16(di, w);
                }
                di = di.wrapping_add(2);
                cnt = cnt.wrapping_add(1);
            }
            bx = bx.wrapping_add(1);
            si = si.wrapping_add(0xA0);
            cx = cx.wrapping_sub(1);
            if cx == 0 {
                break;
            }
        }
    }

    /// `S19E66`（OR）、`S19FF9`（覆寫）、`S1A144`（AND）：每列只處理 2 個字組（x 已乘 2）。
    /// 每個字組先取來源再比對 `di` 與列起點 + 4Fh（無號），大於就結束這一列；之後來源前進 9Ch，
    /// 所以第一個字組就被擋掉時來源只前進 9Eh。
    fn rows2(&mut self, c: &BlitCmd, put: Put) {
        let s = Self::pick(c);
        let page = self.page(c.page);
        let mut si = Self::scratch_src(c, true);
        let x = c.x.wrapping_mul(2);
        let mut bx = c.y;
        let n_rows = if (c.cx as i16) <= 0 { 1u32 } else { c.cx as u32 };
        for _ in 0..n_rows {
            if bx > 0xBD {
                break;
            }
            let start = self.row(bx).wrapping_add(page);
            let lim = start.wrapping_add(0x4F);
            let mut di = start.wrapping_add(x);
            for _ in 0..2 {
                let w = self.sget16(s, si);
                si = si.wrapping_add(2);
                if di > lim {
                    break;
                }
                self.put_word(di, w, put);
                di = di.wrapping_add(2);
            }
            si = si.wrapping_add(0x9C);
            bx = bx.wrapping_add(1);
        }
    }

    /// `S1A287`（`sub_1A51F`）：整列填字組 `dx`（`dx == 1` 時 0AAAAh），x 乘 2。每個迴圈次數取一列；
    /// 列號超過 0BEh（無號）就不寫，也不前進。cx ≤ 0 時什麼都不畫。
    fn fill_rows(&mut self, c: &BlitCmd) {
        let page = self.page(c.page);
        let color = if c.dx == 1 { 0xAAAA } else { c.dx };
        let x = c.x.wrapping_mul(2);
        let mut bx = c.y;
        let mut left = c.cx;
        for _ in 0..0x10000u32 {
            // `sub_19979` 改的 cx 在 `pop cx` 還原，bx 的改動（跳過上緣、12Ch）留下。
            let mut cx = left;
            let mut si = 0u16;
            let row = self.p19979(&mut bx, &mut cx, &mut si, 0, page, x);
            if bx <= 0xBE {
                let mut di = row.di;
                for _ in 0..0x28 {
                    self.vput16(di, color);
                    di = di.wrapping_add(2);
                }
                bx = bx.wrapping_add(1);
            }
            left = left.wrapping_sub(1);
            if left == 0 {
                break;
            }
        }
    }

    /// `S1929F`（`sub_198E1`、`sub_19925`）：血條的一格。`dx == 0Bh` 時 0AAAAh。x 乘 2，7 列，
    /// 每列在目前頁與另一頁（C 是 `xor 4000h`，H 是 `xor 8000h`）各寫一個字組 `dx`。
    fn bar_cell(&mut self, c: &BlitCmd) {
        let page = self.page(c.page);
        let color = if c.dx == 0x0B { 0xAAAA } else { c.dx };
        let flip = match self.kind {
            MonoKind::Cga => 0x4000,
            MonoKind::Mga => 0x8000,
        };
        let mut bx = c.y;
        let mut cx = 7u16;
        let mut si = 0u16;
        for _ in 0..7 {
            let row = self.p19979(&mut bx, &mut cx, &mut si, 1, page, c.x.wrapping_mul(2));
            self.vput16(row.di, color);
            self.vput16(row.di ^ flip, color);
            bx = bx.wrapping_add(1);
            cx = 7;
        }
    }

    // ---- 清除 ----

    fn clear_vmem(&mut self, from: u16, words: u16) {
        let mut di = from;
        for _ in 0..words {
            self.vput16(di, 0);
            di = di.wrapping_add(2);
        }
    }

    fn clear_scratch(&mut self, s: Scratch) {
        for i in 0..0x3FFFu16 {
            self.sput(s, 2 * i, 0);
            self.sput(s, 2 * i + 1, 0);
        }
    }

    /// `sub_1A332`：清目前頁。C 是 B800:[page] 的 2000h 個字組，H 是 B000:[page] 的 3FFFh 個字組。
    fn clear_page(&mut self, c: &BlitCmd) {
        let page = self.page(c.page);
        match self.kind {
            MonoKind::Cga => self.clear_vmem(page, 0x2000),
            MonoKind::Mga => self.clear_vmem(page, 0x3FFF),
        }
    }

    /// `S18894`（`sub_188DE`、`sub_188CD`）：兩頁一起清。C 是 3FFFh 個字組，H 是 7FFFh 個字組，都從 0 起。
    fn clear_both(&mut self) {
        match self.kind {
            MonoKind::Cga => self.clear_vmem(0, 0x3FFF),
            MonoKind::Mga => self.clear_vmem(0, 0x7FFF),
        }
    }

    /// `sub_1117F`（只有 H）：目前頁的 0000h 至 1FFFh 複製到 2000h 至 3FFFh，也就是偶數列的那一組複製到中間那條掃描線。
    fn hgc_copy(&mut self, c: &BlitCmd) {
        if self.kind != MonoKind::Mga {
            return;
        }
        let page = self.page(c.page);
        for i in 0..0x2000u16 {
            let v = self.vget(page.wrapping_add(i));
            self.vput(page.wrapping_add(0x2000).wrapping_add(i), v);
        }
    }

    // ---- 幀計數 ----

    /// `sub_1890D`：5 位數字，每列寫進列位址（不加頁偏移）加欄的位置，同時寫 `di` 與 `di - 8000h`。
    /// C 的 `di - 8000h` 落在 8000h 以上，寫不進去，所以只有第一頁有幀計數。
    fn counter(&mut self, col: u16, row: u16, value: u16) {
        let mut v = value;
        let mut digits = [0u8; 5];
        for (i, d) in [10000u16, 1000, 100, 10, 1].into_iter().enumerate() {
            digits[i] = (v / d) as u8;
            v %= d;
        }
        for (i, &dg) in digits.iter().enumerate() {
            let glyph: [u8; 8] = match &self.counter_font {
                Some(f) => {
                    let at = (0x30 + dg as usize) * 8;
                    std::array::from_fn(|r| f[at + r])
                }
                None => DIGITS[(dg % 10) as usize],
            };
            for (r, &bits) in glyph.iter().enumerate() {
                let off = self.row(row + r as u16).wrapping_add(col + i as u16);
                self.vput(off, bits);
                self.vput(off.wrapping_sub(0x8000), bits);
            }
        }
    }

    // ---- 第三關捲動 ----

    /// `sub_17FF5`（`S17FA0`，`odd` 為假）與 `sub_18086`（`S18031`，`odd` 為真，多 1 個位元組）：暫存區 A 從 `word_25BC4 × 2`
    /// （`S18031` 再加 1）位元組起、190 列、每列 80 位元組，複製到目前頁（列號 0 起，x = 0），來源每列前進 0A0h。
    /// 原版單色不維護暫存區 B 與 E 的逐平面搬移，那些命令（`S17EBA`、`S17EEA`、`S17F26`、`S17F5D`）在這裡不動作。
    fn stage3_view(&mut self, c: &BlitCmd, odd: bool) {
        let page = self.page(c.page);
        let col = c.x.wrapping_shl(1).wrapping_add(odd as u16);
        if !odd {
            // 進場的 `sub_17DF9` 在 E 也呼叫 `S17FA0`，單色的 `sub_185AA` 沒有這一步：進場後緊接 `S17EBA` 時要退回。
            // 190 列覆蓋的範圍：偶數列組 0000h 起，奇數列組加 2000h（C）或 4000h（H），各 1F40h 位元組。
            let base = page as usize;
            let len = match self.kind {
                MonoKind::Cga => 0x4000,
                MonoKind::Mga => 0x6000,
            };
            let end = (base + len).min(self.kind.mem_size());
            if base < end {
                self.undo = Some((base, self.vmem[base..end].to_vec()));
            }
        }
        let mut si = Self::scratch_row(0).wrapping_add(col);
        let mut bx = 0u16;
        for _ in 0..0xBE {
            let start = self.row(bx).wrapping_add(page);
            let mut di = start;
            for k in 0..0x28u16 {
                let w = self.sget16(Scratch::A, si.wrapping_add(2 * k));
                self.vput16(di, w);
                di = di.wrapping_add(2);
            }
            si = si.wrapping_add(0xA0);
            bx = bx.wrapping_add(1);
        }
    }

    /// `sub_1812E`：視窗位移歸 0 時，暫存區 A 往左搬 50h 位元組（3B60h 個字組）。核心在 `word_25BC4` 超過 27h 時才記 `S180C6`。
    fn stage3_wrap(&mut self) {
        for i in 0..0x76C0u16 {
            let v = self.sget(Scratch::A, i.wrapping_add(0x50));
            self.sput(Scratch::A, i, v);
        }
    }

    // ---- 命令分派 ----

    /// 依序套用一個命令。
    pub fn apply(&mut self, cmd: &DrawCmd) {
        match cmd {
            DrawCmd::Blit(c) => self.blit_op(c),
            DrawCmd::VramFill { off, words, value } => self.clear_one_page(*off, *words, *value),
            // 調色盤與序列器的 map mask 是 E 專用的硬體操作，單色沒有。
            DrawCmd::SetDac { .. } | DrawCmd::SetDacOne { .. } | DrawCmd::EgaAttr { .. } | DrawCmd::SetMapMask { .. } => {}
        }
        self.seq += 1;
    }

    /// 核心把 `sub_1291A`（頁 1，`off` = 8000h）與 `sub_128D9`（頁 0，`off` = 0）記成 E 的 `VramFill`（0FA0h 個字組的 0）。
    /// 單色的分支清一整頁：C 是 B800:[頁] 的 2000h 個字組（`12932h` 的 `BC00h` 段即 B800:4000），
    /// H 是 B000:[頁] 的 3FFFh 個字組（`12932h` 的 B800:0 即 B000:8000）。
    fn clear_one_page(&mut self, off: u16, words: u16, value: u16) {
        if value != 0 || words != 0x0FA0 || (off != 0 && off != 0x8000) {
            self.anomaly(format!("VramFill off={off:04X} words={words:04X} value={value:04X} 不是 sub_1291A、sub_128D9 的清頁"));
            return;
        }
        let page = self.page(off);
        match self.kind {
            MonoKind::Cga => self.clear_vmem(page, 0x2000),
            MonoKind::Mga => self.clear_vmem(page, 0x3FFF),
        }
    }

    fn blit_op(&mut self, c: &BlitCmd) {
        let assets = Arc::clone(&self.assets);
        // 第三關進場：`S17FA0` 之後緊接 `S17EBA`，代表剛才那次顯示頁複製單色沒有。
        let pending = self.undo.take();
        if c.op == Op::S17EBA {
            if let Some((base, data)) = pending {
                self.vmem[base..base + data.len()].copy_from_slice(&data);
            }
        }
        let x2 = |c: &BlitCmd| c.x.wrapping_mul(2);
        match c.op {
            Op::S18C0A => self.blit(&assets.my, c, Put::Set),
            Op::S18C6E => self.blit(&assets.my, c, Put::SetInv),
            Op::S18CE2 | Op::S18DCC | Op::S18EB6 => self.blit(&assets.my, c, Put::Or),
            Op::S18D46 | Op::S18E30 | Op::S18E73 => self.blit(&assets.my_mask, c, Put::And),
            Op::S18D89 => self.blit(&assets.my_mask, c, Put::OrInv),
            Op::S18F1A => match assets.sht_mask(c.mask_group) {
                Some(m) => self.blit(m, c, Put::And),
                None => self.anomaly(format!("S18F1A 沒有遮罩庫（{:?}）", c.mask_group)),
            },
            Op::S19025 => match assets.sht_mask(c.mask_group) {
                Some(m) => self.blit(m, c, Put::OrInv),
                None => self.anomaly(format!("S19025 沒有遮罩庫（{:?}）", c.mask_group)),
            },
            Op::S18F5D => self.blit(assets.sht(c.sht_group), c, Put::Or),
            Op::S18FC1 => self.blit(assets.sht(c.sht_group), c, Put::Set),
            Op::S19068 => self.blit(assets.beg(c.beg_group), c, Put::Set),
            Op::S190CC => self.blit(assets.beg(c.beg_group), c, Put::OrInv),
            Op::S19ADC => self.to_scratch(assets.sht(c.sht_group), c),
            Op::S19B40 => self.to_scratch(assets.beg(c.beg_group), c),
            Op::S18ACE => self.map_tile(c),
            Op::S19BA4 => self.rows80(c, Put::Set, false, c.x, true),
            Op::S19C4F => self.rows80(c, Put::Set, true, c.x, true),
            Op::S19D08 => self.rows80_shift(c),
            Op::S19DA8 => self.rows80(c, Put::Or, false, c.x, true),
            Op::S19F3B => self.rows80(c, Put::Or, true, c.x, true),
            Op::S1A09F => self.rows80(c, Put::And, false, x2(c), true),
            Op::S1A1E6 => self.rows80(c, Put::And, true, x2(c), false),
            Op::S19E66 => self.rows2(c, Put::Or),
            Op::S19FF9 => self.rows2(c, Put::Set),
            Op::S1A144 => self.rows2(c, Put::And),
            Op::S1A287 => self.fill_rows(c),
            Op::S1929F => self.bar_cell(c),
            Op::S19CF9 => self.clear_page(c),
            Op::S18894 => self.clear_both(),
            Op::S11504 => self.clear_scratch(Scratch::A),
            Op::S1152D => self.clear_scratch(Scratch::B),
            Op::S1117F => self.hgc_copy(c),
            Op::S1890D => {
                let is_frame_counter = c.text_x == COUNTER_X && c.text_y == COUNTER_Y;
                if self.counter_font.is_some() || !is_frame_counter || self.show_counter {
                    self.counter(c.text_x, c.text_y, c.cx);
                }
            }
            Op::S17FA0 => self.stage3_view(c, false),
            Op::S18031 => self.stage3_view(c, true),
            Op::S180C6 => self.stage3_wrap(),
            // E 專用：暫存區 B 的逐平面維護，單色沒有。
            Op::S17EBA | Op::S17EEA | Op::S17F26 | Op::S17F5D => {}
        }
    }

    // ---- 呈現 ----

    /// 畫面的像素尺寸（寬、高）。
    pub fn size(&self) -> (usize, usize) {
        self.kind.size()
    }

    /// 輸出顯示中那一頁的 RGBA，白底黑字反過來：位元 1 是亮點（白），0 是暗（黑）。
    /// `display_page` 是核心的頁偏移（0 或 8000h）。C 是 640×200；H 是 640×300，
    /// 依 CRTC（R9 = 2，每字元列 3 條掃描線）從 0000h、2000h、4000h 三組各取一條。
    pub fn render_rgba(&self, display_page: u16, out: &mut Vec<u8>) -> (usize, usize) {
        let (w, h) = self.kind.size();
        out.clear();
        out.resize(w * h * 4, 0);
        let page = self.page(display_page) as usize;
        for y in 0..h {
            let base = match self.kind {
                MonoKind::Cga => page + (y / 2) * 80 + (y % 2) * 0x2000,
                MonoKind::Mga => page + (y % 3) * 0x2000 + (y / 3) * 80,
            };
            for bx in 0..80usize {
                let byte = self.vmem.get(base + bx).copied().unwrap_or(0);
                for bit in 0..8usize {
                    let on = byte & (0x80 >> bit) != 0;
                    let v = if on { 255 } else { 0 };
                    let o = (y * w + bx * 8 + bit) * 4;
                    out[o] = v;
                    out[o + 1] = v;
                    out[o + 2] = v;
                    out[o + 3] = 255;
                }
            }
        }
        (w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rs2_core::{BegGroup, ShtGroup};

    fn cmd(op: Op) -> BlitCmd {
        BlitCmd {
            op,
            x: 0,
            y: 0,
            index: 0,
            page: 0,
            scratch: 0,
            src_x: 0,
            src_y: 0,
            shift: 0,
            cx: 0,
            dx: 0,
            text_x: 0,
            text_y: 0,
            sht_group: ShtGroup::Sht1,
            mask_group: ShtGroup::Sht1,
            beg_group: BegGroup::Beg1,
            stride_reg: None,
            obj: None,
        }
    }

    /// 一個圖庫：項 0 是 `height` 列 × `stride` 位元組，內容 `data`。
    fn bank(height: u16, stride: u16, data: &[u8]) -> MonoBank {
        // 位移表 2 項（圖與哨兵），第一個字組是 4。
        let sentinel = 8 + data.len() as u16;
        let mut raw = vec![4, 0];
        raw.extend_from_slice(&sentinel.to_le_bytes());
        raw.extend_from_slice(&height.to_le_bytes());
        raw.extend_from_slice(&stride.to_le_bytes());
        raw.extend_from_slice(data);
        raw.extend_from_slice(&[0xFF, 0xFF, 0, 0]);
        MonoBank { raw }
    }

    fn sink(kind: MonoKind, my: MonoBank) -> MonoSink {
        MonoSink::new(kind, Arc::new(MonoAssets { my, ..MonoAssets::default() }))
    }

    #[test]
    fn row_tables_follow_the_original() {
        let c = sink(MonoKind::Cga, MonoBank::default());
        let h = sink(MonoKind::Mga, MonoBank::default());
        assert_eq!([c.row(0), c.row(1), c.row(2), c.row(3)], [0, 0x2000, 0x50, 0x2050]);
        assert_eq!([h.row(0), h.row(1), h.row(2), h.row(3)], [0, 0x4000, 0x50, 0x4050]);
    }

    #[test]
    fn page_offset_is_converted_for_cga_only() {
        let c = sink(MonoKind::Cga, MonoBank::default());
        let h = sink(MonoKind::Mga, MonoBank::default());
        assert_eq!((c.page(0), c.page(0x8000)), (0, 0x4000));
        assert_eq!((h.page(0), h.page(0x8000)), (0, 0x8000));
    }

    #[test]
    fn overwrite_doubles_x_and_writes_words() {
        // 2 列 × 2 位元組：AA 55 / 0F F0。x = 3 → 位元組 6。
        let mut s = sink(MonoKind::Mga, bank(2, 2, &[0xAA, 0x55, 0x0F, 0xF0]));
        let mut c = cmd(Op::S18C0A);
        c.x = 3;
        c.y = 1;
        s.apply(&DrawCmd::Blit(c));
        // 列 1 在 4000h 組，列 2 在 0050h。
        assert_eq!(&s.vmem[0x4006..0x4008], &[0xAA, 0x55]);
        assert_eq!(&s.vmem[0x0056..0x0058], &[0x0F, 0xF0]);
        assert_eq!(s.anomalies(), 0);
    }

    #[test]
    fn or_and_inverse_operations() {
        let mut s = sink(MonoKind::Cga, bank(1, 2, &[0xF0, 0x0F]));
        s.vmem[0] = 0x0F;
        s.vmem[1] = 0xF0;
        s.apply(&DrawCmd::Blit(cmd(Op::S18CE2)));
        assert_eq!(&s.vmem[0..2], &[0xFF, 0xFF], "OR");
        s.apply(&DrawCmd::Blit(cmd(Op::S18C6E)));
        assert_eq!(&s.vmem[0..2], &[0x0F, 0xF0], "取反覆寫");
        s.vmem[0] = 0x00;
        s.vmem[1] = 0x00;
        s.apply(&DrawCmd::Blit(cmd(Op::S18D89)));
        // 遮罩圖庫是 my_mask，沒有載入：記異常，不寫。
        assert_eq!(&s.vmem[0..2], &[0, 0]);
        assert_eq!(s.anomalies(), 1);
    }

    #[test]
    fn rows_past_190_and_words_outside_the_row_are_skipped() {
        let mut s = sink(MonoKind::Cga, bank(1, 2, &[0xFF, 0xFF]));
        let mut c = cmd(Op::S18C0A);
        c.y = 191;
        s.apply(&DrawCmd::Blit(c));
        assert!(s.vmem.iter().all(|&b| b == 0), "列 191 不畫");
        c.y = 190;
        s.apply(&DrawCmd::Blit(c));
        assert_eq!(s.vmem[95 * 0x50], 0xFF, "列 190 要畫（偶數列，在 95 × 50h）");
        // x = 40 → 位元組 80，超出列尾（列起點 + 4Fh）：整個字組不寫。
        let mut s = sink(MonoKind::Cga, bank(1, 2, &[0xFF, 0xFF]));
        let mut c = cmd(Op::S18C0A);
        c.x = 40;
        s.apply(&DrawCmd::Blit(c));
        assert!(s.vmem.iter().all(|&b| b == 0));
    }

    #[test]
    fn cga_ignores_writes_past_32k_and_hercules_copies_the_even_rows() {
        let mut s = sink(MonoKind::Cga, MonoBank::default());
        s.vput(0x8000, 1);
        assert_eq!(s.vget(0x8000), 0);
        let mut h = sink(MonoKind::Mga, MonoBank::default());
        h.vmem[0x0000] = 0x11;
        h.vmem[0x1FFF] = 0x22;
        let mut c = cmd(Op::S1117F);
        c.page = 0x8000;
        h.vmem[0x8000] = 0x33;
        h.apply(&DrawCmd::Blit(c));
        assert_eq!(h.vmem[0xA000], 0x33, "第二頁的 0000h 組複製到 2000h 組");
        assert_eq!(h.vmem[0x2000], 0, "頁 0 不動");
        let mut c2 = cmd(Op::S1117F);
        c2.page = 0;
        h.apply(&DrawCmd::Blit(c2));
        assert_eq!((h.vmem[0x2000], h.vmem[0x3FFF]), (0x11, 0x22));
        let mut cga = sink(MonoKind::Cga, MonoBank::default());
        cga.vmem[0] = 5;
        cga.apply(&DrawCmd::Blit(c2));
        assert_eq!(cga.vmem[0x2000], 0, "C 沒有這個動作");
    }

    #[test]
    fn clears_follow_the_original_extents() {
        let mut c = sink(MonoKind::Cga, MonoBank::default());
        c.vmem.iter_mut().for_each(|b| *b = 1);
        c.apply(&DrawCmd::Blit(cmd(Op::S18894)));
        assert_eq!((c.vmem[0], c.vmem[0x7FFD], c.vmem[0x7FFE]), (0, 0, 1), "3FFFh 個字組");
        let mut h = sink(MonoKind::Mga, MonoBank::default());
        h.vmem.iter_mut().for_each(|b| *b = 1);
        let mut p = cmd(Op::S19CF9);
        p.page = 0x8000;
        h.apply(&DrawCmd::Blit(p));
        assert_eq!((h.vmem[0x7FFF], h.vmem[0x8000], h.vmem[0xFFFD], h.vmem[0xFFFE]), (1, 0, 0, 1));
    }

    #[test]
    fn scratch_copy_and_wrap() {
        let mut s = sink(MonoKind::Mga, MonoBank::default());
        for i in 0..0x80u16 {
            s.sput(Scratch::A, i, i as u8 + 1);
        }
        let mut c = cmd(Op::S19BA4);
        c.cx = 1;
        c.src_x = 1; // 來源 2 位元組
        s.apply(&DrawCmd::Blit(c));
        assert_eq!(s.vmem[0], 3);
        assert_eq!(s.vmem[0x4F], 0x52);
        // 搬移：暫存區 A 往左 50h 位元組。
        s.sput(Scratch::A, 0x50, 0x99);
        s.apply(&DrawCmd::Blit(cmd(Op::S180C6)));
        assert_eq!(s.sget(Scratch::A, 0), 0x99);
    }

    #[test]
    fn stage3_entry_display_copy_is_undone() {
        let mut s = sink(MonoKind::Cga, MonoBank::default());
        s.sput(Scratch::A, 0, 0xAB);
        s.apply(&DrawCmd::Blit(cmd(Op::S17FA0)));
        assert_eq!(s.vmem[0], 0xAB);
        // 進場：S17FA0 後面接著 S17EBA，單色沒有這一步顯示。
        s.apply(&DrawCmd::Blit(cmd(Op::S17EBA)));
        assert_eq!(s.vmem[0], 0);
        // 捲動：S17FA0 後面不接 S17EBA，保留。
        s.apply(&DrawCmd::Blit(cmd(Op::S17FA0)));
        s.apply(&DrawCmd::Blit(cmd(Op::S17F26)));
        assert_eq!(s.vmem[0], 0xAB);
    }

    #[test]
    fn render_geometry() {
        let mut c = sink(MonoKind::Cga, MonoBank::default());
        c.vmem[0x2000] = 0x80; // 第 1 列最左一點
        let mut buf = Vec::new();
        let (w, h) = c.render_rgba(0, &mut buf);
        assert_eq!((w, h), (640, 200));
        assert_eq!(buf[640 * 4], 255);
        let mut m = sink(MonoKind::Mga, MonoBank::default());
        m.vmem[0x2000] = 0x80; // 字元列 0 的第 2 條掃描線（偶數列的複製）
        m.vmem[0x4000] = 0x80; // 第 3 條（奇數列）
        let (w, h) = m.render_rgba(0, &mut buf);
        assert_eq!((w, h), (640, 300));
        assert_eq!((buf[640 * 4], buf[640 * 4 * 2], buf[0]), (255, 255, 0));
    }
}
