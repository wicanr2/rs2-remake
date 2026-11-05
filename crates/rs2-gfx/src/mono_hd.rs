//! `MonoHdSink`：單色顯示模式 C（CGA）與 H（MGA）的 HD 接收端。
//!
//! 吃 [`MonoSink`](crate::MonoSink) 同一批繪製命令（`rs2_game::DrawCmd`），輸出 1280×800 RGBA，是邏輯畫面 320×200 的 4 倍。
//! CGA 與 MGA 的 HD 畫面相同：MGA 的 640×300（垂直 1.5 倍）只是 SD 呈現，HD 不重現，`EndOfFrameCopy` 在 HD 不動作。
//!
//! # 設計
//!
//! 單色圖庫的最小單位是 1 個位元組（8 個單色像素 = 4 個邏輯像素）。HD 把每個位元組換成一個 16×4 的 HD 像素區塊：
//! 寬 16 = 8 個單色像素 × 2，高 4 = 1 條掃描線 × 4。區塊用一個 `u64` 存 1 位元像素，第 `y` 列第 `x` 欄在位元 `y × 16 + x`，
//! 所以原版的位元運算（覆寫、OR、AND、取反）在區塊上是同一個運算。
//!
//! 狀態（兩個顯示頁、暫存區 A、B）與位址運算照 `MonoSink` 逐函式移植，只把 `u8` 換成區塊：
//! 位元組位址、列位址表、頁偏移、y 上限 190、字組超出列尾不畫、暫存區列距 160 位元組、清除範圍、
//! 血條寫兩頁、第三關捲動與進場復原都沿用。因此行為與 `MonoSink` 一致，包括原版的環繞與超出範圍的行為。
//! 記憶體的位址配置是 SD 的（CGA 的奇數列加 2000h，MGA 加 4000h，頁在 4000h 或 8000h），畫面輸出時才換成 HD 的列。
//!
//! 圖庫來源：`<hd_dir>/mono/<檔名>/<4 位索引>.png`（`docs/hd/codex-handoff.md`），讀 R 通道，>= 128 是亮點，
//! 忽略 alpha（單色圖的透明靠遮罩庫，HD 檔的透明處 RGB 為 0）。預期尺寸是 `每列位元組數 × 16` × `高度 × 4`。
//! 圖片用到才載入並快取。缺檔、解碼失敗、尺寸不符的圖記進 [`MonoHdSink::missing`]，改用 SD 圖庫項的最近鄰放大
//! （每個單色像素 2×4），畫面不缺角。SD 的來源指標超出該項像素範圍時（原版會讀到相鄰項的位元組），
//! 該位元組也用最近鄰放大，與 SD 的讀法一致。
//!
//! 因此 HD 圖全換成最近鄰放大時，輸出縮回 320×200 與 `MonoSink` 逐像素相同；這由
//! `crates/rs2-trace/tests/mono_hd.rs` 對 14 份收據驗證。
//!
//! # 公開介面
//!
//! | 項目 | 說明 |
//! |---|---|
//! | `MonoHdSink::new(assets: Arc<MonoAssets>, hd_dir: &Path)` | 預設 CGA。`hd_dir` 是 HD 素材根目錄（`LayerSink` 用的同一個，底下有 `mono/`） |
//! | `MonoHdSink::with_kind(kind, assets, hd_dir)`、`kind()` | 指定 CGA 或 MGA。只影響記憶體位址配置、幀計數寫哪一頁，輸出畫面相同 |
//! | `apply(&DrawCmd)` | 套用命令 |
//! | `render_rgba(display_page: u16, out: &mut Vec<u8>) -> (usize, usize)` | 1280×800 RGBA，白色亮點、黑色背景，`display_page` 是核心的頁偏移（0 或 8000h）。`size()`、`MONO_HD_W`、`MONO_HD_H` 給尺寸 |
//! | `missing() -> &[String]`、`hd_loaded()` | 缺的或尺寸不符的 HD 圖（每項一次）、已載入的 HD 圖張數 |
//! | `show_counter`、`set_counter_font(font)` | 幀計數，8×8 字型放大 2 倍寬 4 倍高，語意同 `MonoSink` |
//! | `load_snapshot(vmem, sa, sb)` | 以原版某時點的 SD 記憶體當起點（最近鄰放大），語意同 `MonoSink` |
//! | `preload_all()` | 載入全部 HD 圖，回傳成功數，缺的進 `missing()`（啟動檢查用） |
//! | `applied()`、`anomalies()`、`anomaly_log()` | 同 `MonoSink` |
//! | `vram_blocks()`、`scratch_a_blocks()`、`scratch_b_blocks()`、`MonoHdSink::nn_block(byte)` | 對拍用的內部狀態與位元組的最近鄰放大 |
//!
//! 與 `MonoSink` 的差別只有三處：`EndOfFrameCopy` 不動作；輸出是 1280×800 而非 640×200／640×300；圖來源是 HD 圖。
//! 第三關橫向捲動的解析度與 SD 相同（位元組，16 個 HD 像素一步）。

use crate::font::DIGITS;
use crate::mono::{Item, MonoAssets, MonoBank, MonoKind, Put, SCRATCH_SIZE};
use crate::png;
use crate::sink::{COUNTER_X, COUNTER_Y};
use rs2_game::{BegGroup, BlitCmd, DrawCmd, Op, ShtGroup};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 輸出畫面寬度（HD 像素）。
pub const MONO_HD_W: usize = 1280;
/// 輸出畫面高度（HD 像素）。
pub const MONO_HD_H: usize = 800;
/// 一個位元組對應的區塊寬（HD 像素）。
const BLK_W: usize = 16;
/// 一條掃描線對應的區塊高（HD 像素）。
const BLK_H: usize = 4;

type Blocks = Arc<Vec<u64>>;

const fn expand_table() -> [u16; 256] {
    let mut t = [0u16; 256];
    let mut b = 0usize;
    while b < 256 {
        let mut x = 0usize;
        while x < 16 {
            if (b >> (7 - x / 2)) & 1 != 0 {
                t[b] |= 1 << x;
            }
            x += 1;
        }
        b += 1;
    }
    t
}

/// 一個位元組展成一列 16 個 HD 像素：位元 7 是最左邊的單色像素，每個單色像素 2 個 HD 像素寬。
static EXPAND: [u16; 256] = expand_table();

/// 一個位元組的最近鄰放大：每個單色像素 2 寬 4 高。
fn nn_block(byte: u8) -> u64 {
    let e = EXPAND[byte as usize] as u64;
    e | (e << 16) | (e << 32) | (e << 48)
}

/// 圖庫項的來源：SD 圖庫（取最近鄰放大的位元組）加上選用的 HD 區塊。
struct Src<'a> {
    bank: &'a MonoBank,
    /// SD 像素資料在圖庫內的位移。
    data: usize,
    /// 逐位元組對應的 HD 區塊，索引是 `列 × 每列位元組數 + 欄`，與 SD 像素資料的線性位移相同。
    hd: Option<Blocks>,
}

impl Src<'_> {
    /// SD 圖庫內位移 `off` 那個位元組對應的區塊。HD 圖範圍內取 HD，範圍外取 SD 位元組的最近鄰放大，圖庫外是 `None`。
    fn blk(&self, off: usize) -> Option<u64> {
        if let Some(h) = &self.hd {
            if let Some(b) = off.checked_sub(self.data).and_then(|p| h.get(p)) {
                return Some(*b);
            }
        }
        self.bank.rd8(off).map(nn_block)
    }
}

fn sht_name(g: ShtGroup) -> &'static str {
    match g {
        ShtGroup::Bh1 => "BH1.BDB",
        ShtGroup::Bh2 => "BH2.BDB",
        ShtGroup::Bh4 => "BH4.BDB",
        ShtGroup::Sht1 => "SHT1.BDB",
        ShtGroup::Sht2 => "SHT2.BDB",
        ShtGroup::Sht3 => "SHT3.BDB",
        ShtGroup::Sht4 => "SHT4.BDB",
        ShtGroup::Sht5 => "SHT5.BDB",
    }
}

fn sht_mask_name(g: ShtGroup) -> Option<&'static str> {
    match g {
        ShtGroup::Sht1 => Some("SHT1.BDH"),
        ShtGroup::Sht2 => Some("SHT2.BDH"),
        ShtGroup::Sht3 => Some("SHT3.BDH"),
        ShtGroup::Sht4 => Some("SHT4.BDH"),
        ShtGroup::Sht5 => Some("SHT5.BDH"),
        ShtGroup::Bh1 | ShtGroup::Bh2 | ShtGroup::Bh4 => None,
    }
}

fn beg_name(g: BegGroup) -> &'static str {
    match g {
        BegGroup::Beg1 => "BEG1.BDB",
        BegGroup::J1 => "J1.GOB",
        BegGroup::J2 => "J2.GOB",
        BegGroup::J3 => "J3.GOB",
        BegGroup::J4 => "J4.GOB",
    }
}

/// 21 個單色圖庫與檔名（同 `MONO_EXPECTED`），檔名也是 HD 資料夾 `mono/` 底下的目錄名。
fn bank_list(a: &MonoAssets) -> Vec<(&'static str, &MonoBank)> {
    vec![
        ("MY.BDB", &a.my),
        ("MYB.BDT", &a.my_mask),
        ("SHT1.BDB", &a.sht[0]),
        ("SHT1.BDH", &a.sht_mask[0]),
        ("SHT2.BDB", &a.sht[1]),
        ("SHT2.BDH", &a.sht_mask[1]),
        ("SHT3.BDB", &a.sht[2]),
        ("SHT3.BDH", &a.sht_mask[2]),
        ("SHT4.BDB", &a.sht[3]),
        ("SHT4.BDH", &a.sht_mask[3]),
        ("SHT5.BDB", &a.sht[4]),
        ("SHT5.BDH", &a.sht_mask[4]),
        ("BH1.BDB", &a.bh[0]),
        ("BH2.BDB", &a.bh[1]),
        ("BH4.BDB", &a.bh[2]),
        ("MAP.BDB", &a.map),
        ("J1.GOB", &a.j[0]),
        ("J2.GOB", &a.j[1]),
        ("J3.GOB", &a.j[2]),
        ("J4.GOB", &a.j[3]),
        ("BEG1.BDB", &a.beg1),
    ]
}

/// HD 圖的 (欄 `bx`, 列 `by`) 那個 16×4 區塊。R 通道 >= 128 是亮點。
fn block_from_rgba(img: &png::Image, bx: usize, by: usize) -> u64 {
    let mut v = 0u64;
    for r in 0..BLK_H {
        let row = (by * BLK_H + r) * img.w + bx * BLK_W;
        for x in 0..BLK_W {
            if img.rgba[(row + x) * 4] >= 128 {
                v |= 1u64 << (r * 16 + x);
            }
        }
    }
    v
}

/// 讀 `<dir>/<名>/<索引>.png` 成區塊。失敗時回傳要記進 `missing` 的說明。
fn load_blocks(dir: &Path, name: &str, index: u16, it: &Item) -> Result<Vec<u64>, String> {
    let rel = format!("{name}/{index:04}.png");
    let (cols, rows) = (it.stride as usize, it.height as usize);
    let (ew, eh) = (cols * BLK_W, rows * BLK_H);
    let data = match std::fs::read(dir.join(&rel)) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(rel),
        Err(e) => return Err(format!("{rel}（讀取失敗：{e}）")),
    };
    let img = png::decode(&data).map_err(|e| format!("{rel}（解碼失敗：{e}）"))?;
    if img.w != ew || img.h != eh {
        return Err(format!("{rel}（尺寸 {}x{}，預期 {ew}x{eh}）", img.w, img.h));
    }
    let mut out = Vec::with_capacity(cols * rows);
    for by in 0..rows {
        for bx in 0..cols {
            out.push(block_from_rgba(&img, bx, by));
        }
    }
    Ok(out)
}

/// 背景暫存區的選擇。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scratch {
    A,
    B,
}

#[derive(Clone)]
pub struct MonoHdSink {
    kind: MonoKind,
    assets: Arc<MonoAssets>,
    /// `<hd_dir>/mono`。
    dir: PathBuf,
    /// `dir` 是目錄才讀 HD 圖。
    hd_enabled: bool,
    /// 顯示記憶體，位址同 SD（段內位移），每個位元組一個區塊；CGA 只用前 8000h 個。
    vmem: Vec<u64>,
    sa: Vec<u64>,
    sb: Vec<u64>,
    /// 幀計數預設不畫。設了 BIOS 字型（`set_counter_font`）之後永遠畫。
    pub show_counter: bool,
    counter_font: Option<Vec<u8>>,
    /// 第三關進場的 `ScrollShowA` 之前的顯示頁內容，見 `stage3_view`。
    undo: Option<(usize, Vec<u64>)>,
    /// `(庫檔名, 索引)` 到 HD 區塊。`None` 是沒有可用的 HD 圖。
    cache: HashMap<(&'static str, u16), Option<Blocks>>,
    missing: Vec<String>,
    loaded: usize,
    anomalies: u64,
    log: Vec<String>,
    seq: u64,
    /// 戰場加大的關卡（`DrawCmd::Field`，欄數不是 40），單色不支援，同 `MonoSink`。
    wide: bool,
}

impl MonoHdSink {
    /// CGA 的 HD 接收端。`hd_dir` 是 HD 素材根目錄，圖在 `<hd_dir>/mono/<檔名>/<索引>.png`。
    /// 資料夾不存在時不失敗，`missing()` 記一筆，全部用 SD 圖庫放大。
    pub fn new(assets: Arc<MonoAssets>, hd_dir: &Path) -> MonoHdSink {
        MonoHdSink::with_kind(MonoKind::Cga, assets, hd_dir)
    }

    /// 同 [`MonoHdSink::new`]，另指定 CGA 或 MGA。
    pub fn with_kind(kind: MonoKind, assets: Arc<MonoAssets>, hd_dir: &Path) -> MonoHdSink {
        let dir = hd_dir.join("mono");
        let hd_enabled = dir.is_dir();
        let mut missing = Vec::new();
        if !hd_enabled {
            missing.push(format!("{}（HD 單色圖資料夾不存在，全部用原版圖放大）", dir.display()));
        }
        MonoHdSink {
            kind,
            assets,
            dir,
            hd_enabled,
            vmem: vec![0; 0x10000],
            sa: vec![0; SCRATCH_SIZE],
            sb: vec![0; SCRATCH_SIZE],
            show_counter: false,
            counter_font: None,
            undo: None,
            cache: HashMap::new(),
            missing,
            loaded: 0,
            anomalies: 0,
            log: Vec::new(),
            seq: 0,
            wide: false,
        }
    }

    pub fn kind(&self) -> MonoKind {
        self.kind
    }

    /// 一個 SD 位元組（8 個單色像素）的最近鄰放大區塊：每個單色像素 2 寬 4 高，位元 `y × 16 + x`。
    pub fn nn_block(byte: u8) -> u64 {
        nn_block(byte)
    }

    /// 缺的 HD 圖（含解碼失敗與尺寸不符），每項一次，依發現順序。這些圖改用 SD 圖庫項的最近鄰放大。
    pub fn missing(&self) -> &[String] {
        &self.missing
    }

    /// 已成功載入的 HD 圖張數。
    pub fn hd_loaded(&self) -> usize {
        self.loaded
    }

    /// 已套用的命令數。
    pub fn applied(&self) -> u64 {
        self.seq
    }

    /// 無法照機器碼重現的次數，條件同 `MonoSink::anomalies`。
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

    fn note_missing(&mut self, s: String) {
        if !self.missing.contains(&s) {
            self.missing.push(s);
        }
    }

    /// 用原版某個時點的顯示記憶體與暫存區當起點。格式同 `MonoSink::load_snapshot`，每個位元組以最近鄰放大。
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
                for (d, s) in v[..0x8000].iter_mut().zip(&vmem[0x8000..]) {
                    *d = nn_block(*s);
                }
                v
            }
            MonoKind::Mga => vmem.iter().map(|&b| nn_block(b)).collect(),
        };
        self.sa = sa.iter().map(|&b| nn_block(b)).collect();
        self.sb = sb.iter().map(|&b| nn_block(b)).collect();
        self.undo = None;
    }

    /// BIOS 8x8 字型（2048 位元組）。設了之後幀計數照原版用這份字型畫（放大 2 倍寬 4 倍高），並且永遠畫。
    ///
    /// # Panics
    /// 長度不是 2048。
    pub fn set_counter_font(&mut self, font: &[u8]) {
        assert_eq!(font.len(), 2048, "BIOS 字型應為 2048 位元組（256 字元 x 8 列）");
        self.counter_font = Some(font.to_vec());
    }

    /// 顯示記憶體的區塊（CGA 前 8000h 個，MGA 10000h 個），索引同 SD 顯示記憶體的位元組位移。
    pub fn vram_blocks(&self) -> &[u64] {
        &self.vmem[..self.kind.mem_size()]
    }

    pub fn scratch_a_blocks(&self) -> &[u64] {
        &self.sa
    }

    pub fn scratch_b_blocks(&self) -> &[u64] {
        &self.sb
    }

    /// 載入全部圖庫的每一張 HD 圖，回傳成功的張數。缺的與尺寸不符的進 `missing()`。
    pub fn preload_all(&mut self) -> usize {
        let assets = Arc::clone(&self.assets);
        let before = self.loaded;
        for (name, bank) in bank_list(&assets) {
            for i in 0..bank.len().min(0x8000) {
                let index = i as u16;
                if let Some(it) = bank.item(index) {
                    self.hd_item(name, bank, index, &it);
                }
            }
        }
        self.loaded - before
    }

    // ---- 圖庫來源 ----

    /// 取 `(name, index)` 的 HD 區塊，第一次用到才讀。沒有 HD 圖的情況：`index` 超出圖庫（SD 讀到其他位元組）、
    /// 寬為 0、高為 0 或有號為負。每個圖庫位移表的最後一項是結尾的哨兵（`FF FF 00 00`，高 65535），不是圖。
    fn hd_item(&mut self, name: &'static str, bank: &MonoBank, index: u16, it: &Item) -> Option<Blocks> {
        let key = (name, index);
        if let Some(v) = self.cache.get(&key) {
            return v.clone();
        }
        let v = if !self.hd_enabled || (it.height as i16) <= 0 || it.stride == 0 || index as usize >= bank.len() {
            None
        } else {
            match load_blocks(&self.dir, name, index, it) {
                Ok(b) => {
                    self.loaded += 1;
                    Some(Arc::new(b))
                }
                Err(note) => {
                    self.note_missing(note);
                    None
                }
            }
        };
        self.cache.insert(key, v.clone());
        v
    }

    /// 取圖庫項與來源。圖號超出圖庫回 `None`（SD 的 `sub_18AA4` 取不到項）。
    fn src<'a>(&mut self, name: &'static str, bank: &'a MonoBank, index: u16) -> Option<(Item, Src<'a>)> {
        let it = bank.item(index)?;
        let hd = self.hd_item(name, bank, index, &it);
        Some((it, Src { bank, data: it.data, hd }))
    }

    // ---- 位址與記憶體（同 MonoSink）----

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

    /// 列位址表：偶數列 `n/2 × 50h`，奇數列加 2000h（C）或 4000h（H）。
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

    fn vget(&self, off: u16) -> u64 {
        if (off as usize) < self.kind.mem_size() { self.vmem[off as usize] } else { 0 }
    }

    fn vput(&mut self, off: u16, v: u64) {
        if (off as usize) < self.kind.mem_size() {
            self.vmem[off as usize] = v;
        }
    }

    /// 字組寫入：低位元組在 `off`，高位元組在 `off + 1`（16 位元環繞）。
    fn vput16(&mut self, off: u16, lo: u64, hi: u64) {
        self.vput(off, lo);
        self.vput(off.wrapping_add(1), hi);
    }

    fn scratch(&self, s: Scratch) -> &[u64] {
        match s {
            Scratch::A => &self.sa,
            Scratch::B => &self.sb,
        }
    }

    fn sget(&self, s: Scratch, off: u16) -> u64 {
        self.scratch(s).get(off as usize).copied().unwrap_or(0)
    }

    fn sput(&mut self, s: Scratch, off: u16, v: u64) {
        let buf = match s {
            Scratch::A => &mut self.sa,
            Scratch::B => &mut self.sb,
        };
        if let Some(b) = buf.get_mut(off as usize) {
            *b = v;
        }
    }

    /// `sub_1A70A`：`word_2685C` 為 8000h 選 B，否則 A。
    fn pick(c: &BlitCmd) -> Scratch {
        if c.scratch == 0x8000 { Scratch::B } else { Scratch::A }
    }

    /// `sub_19979`（同 `MonoSink::p19979`）。回傳寫入點（`di`）。
    fn p19979(&self, bx: &mut u16, cx: &mut u16, si: &mut u16, stride: u16, page: u16, x: u16) -> u16 {
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
        start.wrapping_add(x)
    }

    // ---- 圖庫 blit ----

    /// 位元組運算：區塊的位元運算與原版對位元組的運算相同。
    fn put_byte(&mut self, di: u16, src: u64, put: Put) {
        let v = match put {
            Put::Set => src,
            Put::SetInv => !src,
            Put::Or => self.vget(di) | src,
            Put::OrInv => self.vget(di) | !src,
            Put::And => self.vget(di) & src,
        };
        self.vput(di, v);
    }

    /// 字組運算：兩個位元組各自運算（原版的 `or`、`and`、`xor` 都是逐位元）。
    fn put_word(&mut self, di: u16, lo: u64, hi: u64, put: Put) {
        self.put_byte(di, lo, put);
        self.put_byte(di.wrapping_add(1), hi, put);
    }

    /// 同 `MonoSink::blit`：`sub_18AA4` 取項，`sub_1995E` 裁上緣，逐列逐字組寫。
    fn blit(&mut self, name: &'static str, bank: &MonoBank, c: &BlitCmd, put: Put) {
        let Some((it, img)) = self.src(name, bank, c.index) else {
            self.anomaly(format!("{:?} 圖號 {} 超出圖庫", c.op, c.index));
            return;
        };
        let page = self.page(c.page);
        let x = c.x.wrapping_mul(2);
        let mut bx = c.y;
        let mut cx = it.height;
        let mut si = 0u16;
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
                    let (lo, hi) = match (img.blk(src), img.blk(src + 1)) {
                        (Some(a), Some(b)) => (a, b),
                        _ => {
                            if !oob_reported {
                                oob_reported = true;
                                self.anomaly(format!("{:?} 圖號 {} 來源超出圖庫", c.op, c.index));
                            }
                            (0, 0)
                        }
                    };
                    self.put_word(di, lo, hi, put);
                }
                src += 2;
                di = di.wrapping_add(2);
            }
            bx = bx.wrapping_add(1);
        }
    }

    /// 同 `MonoSink::to_scratch`（`StageToScratch`、`BegToScratch`）：圖庫項寫進背景暫存區，不裁切。
    fn to_scratch(&mut self, name: &'static str, bank: &MonoBank, c: &BlitCmd) {
        let Some((it, img)) = self.src(name, bank, c.index) else {
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
                    let b = match img.blk(src) {
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

    /// 同 `MonoSink::map_tile`（`MapToScratch`）：MAP 圖塊寫進暫存區，固定 10 列、每列連續 2 個位元組。
    fn map_tile(&mut self, c: &BlitCmd) {
        let assets = Arc::clone(&self.assets);
        let Some((it, img)) = self.src("MAP.BDB", &assets.map, c.index) else {
            self.anomaly(format!("MapToScratch 圖號 {} 超出 MAP.BDB", c.index));
            return;
        };
        let s = Self::pick(c);
        let mut di = Self::scratch_row(c.y).wrapping_add(c.x.wrapping_mul(2));
        let mut src = it.data;
        for _ in 0..10 {
            for k in 0..2u16 {
                let b = img.blk(src).unwrap_or_else(|| {
                    self.anomaly(format!("MapToScratch 圖號 {} 圖塊資料不足 20 位元組", c.index));
                    0
                });
                self.sput(s, di.wrapping_add(k), b);
                src += 1;
            }
            di = di.wrapping_add(0xA0);
        }
    }

    // ---- 暫存區搬到顯示頁 ----

    /// 暫存區來源指標，同 `MonoSink::scratch_src`。
    fn scratch_src(c: &BlitCmd, src_x_doubled: bool) -> u16 {
        let sx = if src_x_doubled { c.src_x.wrapping_mul(2) } else { c.src_x };
        (c.src_y & 0xFF).wrapping_mul(0xA0).wrapping_add(sx).wrapping_add(c.shift.wrapping_mul(2))
    }

    /// 暫存區的一個字組（`off`、`off + 1`）運算到顯示頁的 `di`。
    fn put_from_scratch(&mut self, s: Scratch, off: u16, di: u16, put: Put) {
        let lo = self.sget(s, off);
        let hi = self.sget(s, off.wrapping_add(1));
        self.put_word(di, lo, hi, put);
    }

    /// 同 `MonoSink::rows80`：暫存區整列（80 位元組）運算到顯示頁。
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
                self.put_from_scratch(s, si.wrapping_add(2 * k), di, put);
                di = di.wrapping_add(2);
            }
            si = si.wrapping_add(0xA0);
            bx = if down { bx.wrapping_sub(1) } else { bx.wrapping_add(1) };
        }
    }

    /// 同 `MonoSink::rows80_shift`（`ScratchReplaceShifted`）。
    fn rows80_shift(&mut self, c: &BlitCmd) {
        let s = Self::pick(c);
        let page = self.page(c.page);
        let mut si = Self::scratch_src(c, true);
        let mut bx = c.y;
        let mut cx = c.cx;
        for _ in 0..0x10000u32 {
            let di0 = self.p19979(&mut bx, &mut cx, &mut si, 0xA0, page, c.x);
            if bx > 0xBD {
                break;
            }
            let mut di = di0;
            let mut cnt = c.shift;
            for k in 0..0x28u16 {
                if cnt < 0x50 {
                    self.put_from_scratch(s, si.wrapping_add(2 * k), di, Put::Set);
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

    /// 同 `MonoSink::rows2`（`ScratchOrPairs`、`ScratchLatchPairs`、`ScratchAndPairs`）：每列只處理 2 個字組。
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
                let at = si;
                si = si.wrapping_add(2);
                if di > lim {
                    break;
                }
                self.put_from_scratch(s, at, di, put);
                di = di.wrapping_add(2);
            }
            si = si.wrapping_add(0x9C);
            bx = bx.wrapping_add(1);
        }
    }

    /// 同 `MonoSink::fill_rows`（`FillRows`）：整列填字組 `dx`（`dx == 1` 時 0AAAAh），每個位元組以最近鄰放大。
    fn fill_rows(&mut self, c: &BlitCmd) {
        let page = self.page(c.page);
        let color = if c.dx == 1 { 0xAAAA } else { c.dx };
        let (lo, hi) = (nn_block(color as u8), nn_block((color >> 8) as u8));
        let x = c.x.wrapping_mul(2);
        let mut bx = c.y;
        let mut left = c.cx;
        for _ in 0..0x10000u32 {
            let mut cx = left;
            let mut si = 0u16;
            let di0 = self.p19979(&mut bx, &mut cx, &mut si, 0, page, x);
            if bx <= 0xBE {
                let mut di = di0;
                for _ in 0..0x28 {
                    self.vput16(di, lo, hi);
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

    /// 同 `MonoSink::bar_cell`（`BarCell`）：血條的一格，7 列，目前頁與另一頁各寫一個字組。
    fn bar_cell(&mut self, c: &BlitCmd) {
        let page = self.page(c.page);
        let color = if c.dx == 0x0B { 0xAAAA } else { c.dx };
        let (lo, hi) = (nn_block(color as u8), nn_block((color >> 8) as u8));
        let flip = match self.kind {
            MonoKind::Cga => 0x4000,
            MonoKind::Mga => 0x8000,
        };
        let mut bx = c.y;
        let mut cx = 7u16;
        let mut si = 0u16;
        for _ in 0..7 {
            let di = self.p19979(&mut bx, &mut cx, &mut si, 1, page, c.x.wrapping_mul(2));
            self.vput16(di, lo, hi);
            self.vput16(di ^ flip, lo, hi);
            bx = bx.wrapping_add(1);
            cx = 7;
        }
    }

    // ---- 清除 ----

    fn clear_vmem(&mut self, from: u16, words: u16) {
        let mut di = from;
        for _ in 0..words {
            self.vput16(di, 0, 0);
            di = di.wrapping_add(2);
        }
    }

    fn clear_scratch(&mut self, s: Scratch) {
        for i in 0..0x3FFFu16 {
            self.sput(s, 2 * i, 0);
            self.sput(s, 2 * i + 1, 0);
        }
    }

    /// 同 `MonoSink::clear_page`（`sub_1A332`）：清目前頁。
    fn clear_page(&mut self, c: &BlitCmd) {
        let page = self.page(c.page);
        match self.kind {
            MonoKind::Cga => self.clear_vmem(page, 0x2000),
            MonoKind::Mga => self.clear_vmem(page, 0x3FFF),
        }
    }

    /// 同 `MonoSink::clear_both`（`ClearBothPages`）：兩頁一起清。
    fn clear_both(&mut self) {
        match self.kind {
            MonoKind::Cga => self.clear_vmem(0, 0x3FFF),
            MonoKind::Mga => self.clear_vmem(0, 0x7FFF),
        }
    }

    /// 同 `MonoSink::clear_one_page`：核心把 `sub_1291A`、`sub_128D9` 記成 E 的 `VramFill`，單色的分支清一整頁。
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

    // ---- 幀計數 ----

    /// 同 `MonoSink::counter`（`sub_1890D`）：5 位數字，8×8 字型的每個位元組放大 2 倍寬 4 倍高，同時寫 `di` 與 `di - 8000h`。
    /// CGA 的 `di - 8000h` 落在範圍外，所以只有第一頁有幀計數；MGA 兩頁都有。
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
                let blk = nn_block(bits);
                self.vput(off, blk);
                self.vput(off.wrapping_sub(0x8000), blk);
            }
        }
    }

    // ---- 第三關捲動 ----

    /// 同 `MonoSink::stage3_view`（`ScrollShowA`、`ScrollShowB`）：暫存區 A 從 `word_25BC4 × 2`（`ScrollShowB` 再加 1）位元組起、
    /// 190 列、每列 80 位元組，複製到目前頁。進場時 `ScrollShowA` 後面接 `ScrollRotateCopy`，要把這次複製退回。
    fn stage3_view(&mut self, c: &BlitCmd, odd: bool) {
        let page = self.page(c.page);
        let col = c.x.wrapping_shl(1).wrapping_add(odd as u16);
        if !odd {
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
                self.put_from_scratch(Scratch::A, si.wrapping_add(2 * k), di, Put::Set);
                di = di.wrapping_add(2);
            }
            si = si.wrapping_add(0xA0);
            bx = bx.wrapping_add(1);
        }
    }

    /// 同 `MonoSink::stage3_wrap`（`sub_1812E`）：暫存區 A 往左搬 50h 位元組。
    fn stage3_wrap(&mut self) {
        for i in 0..0x76C0u16 {
            let v = self.sget(Scratch::A, i.wrapping_add(0x50));
            self.sput(Scratch::A, i, v);
        }
    }

    // ---- 命令分派 ----

    /// 依序套用一個命令。命令與 `MonoSink::apply` 相同；`EndOfFrameCopy`（MGA 的掃描線複製）在 HD 不動作。
    pub fn apply(&mut self, cmd: &DrawCmd) {
        if let DrawCmd::Field { cols } = cmd {
            let wide = *cols != 40;
            if self.wide && !wide {
                self.vmem.fill(0);
                self.sa.fill(0);
                self.sb.fill(0);
                self.undo = None;
            }
            self.wide = wide;
            self.seq += 1;
            return;
        }
        if self.wide {
            self.seq += 1;
            return;
        }
        match cmd {
            DrawCmd::Blit(c) => self.blit_op(c),
            DrawCmd::VramFill { off, words, value } => self.clear_one_page(*off, *words, *value),
            DrawCmd::SetDac { .. } | DrawCmd::SetDacOne { .. } | DrawCmd::EgaAttr { .. } | DrawCmd::SetMapMask { .. } | DrawCmd::Field { .. } => {}
        }
        self.seq += 1;
    }

    fn blit_op(&mut self, c: &BlitCmd) {
        let assets = Arc::clone(&self.assets);
        let pending = self.undo.take();
        if c.op == Op::ScrollRotateCopy {
            if let Some((base, data)) = pending {
                self.vmem[base..base + data.len()].copy_from_slice(&data);
            }
        }
        let x2 = |c: &BlitCmd| c.x.wrapping_mul(2);
        match c.op {
            Op::MyOverwrite => self.blit("MY.BDB", &assets.my, c, Put::Set),
            Op::MyOverwriteInverted => self.blit("MY.BDB", &assets.my, c, Put::SetInv),
            Op::MyOverlay | Op::MyOverlayEffect | Op::MyOverlayPlayerShot => self.blit("MY.BDB", &assets.my, c, Put::Or),
            Op::MyCut | Op::MyCutEffect | Op::MyCutPlayerShot => self.blit("MYB.BDT", &assets.my_mask, c, Put::And),
            Op::MyFlash => self.blit("MYB.BDT", &assets.my_mask, c, Put::OrInv),
            Op::StageCut => match (assets.sht_mask(c.mask_group), sht_mask_name(c.mask_group)) {
                (Some(m), Some(n)) => self.blit(n, m, c, Put::And),
                _ => self.anomaly(format!("StageCut 沒有遮罩庫（{:?}）", c.mask_group)),
            },
            Op::StageFlash => match (assets.sht_mask(c.mask_group), sht_mask_name(c.mask_group)) {
                (Some(m), Some(n)) => self.blit(n, m, c, Put::OrInv),
                _ => self.anomaly(format!("StageFlash 沒有遮罩庫（{:?}）", c.mask_group)),
            },
            Op::StageOverlay => self.blit(sht_name(c.sht_group), assets.sht(c.sht_group), c, Put::Or),
            Op::StageOverwrite => self.blit(sht_name(c.sht_group), assets.sht(c.sht_group), c, Put::Set),
            Op::BegOverwrite => self.blit(beg_name(c.beg_group), assets.beg(c.beg_group), c, Put::Set),
            Op::BegInverted => self.blit(beg_name(c.beg_group), assets.beg(c.beg_group), c, Put::OrInv),
            Op::StageToScratch => self.to_scratch(sht_name(c.sht_group), assets.sht(c.sht_group), c),
            Op::BegToScratch => self.to_scratch(beg_name(c.beg_group), assets.beg(c.beg_group), c),
            Op::MapToScratch => self.map_tile(c),
            Op::ScratchReplace => self.rows80(c, Put::Set, false, c.x, true),
            Op::ScratchReplaceMirrored => self.rows80(c, Put::Set, true, c.x, true),
            Op::ScratchReplaceShifted => self.rows80_shift(c),
            Op::ScratchOr => self.rows80(c, Put::Or, false, c.x, true),
            Op::ScratchOrMirrored => self.rows80(c, Put::Or, true, c.x, true),
            Op::ScratchAnd => self.rows80(c, Put::And, false, x2(c), true),
            Op::ScratchAndMirrored => self.rows80(c, Put::And, true, x2(c), false),
            Op::ScratchOrPairs => self.rows2(c, Put::Or),
            Op::ScratchLatchPairs => self.rows2(c, Put::Set),
            Op::ScratchAndPairs => self.rows2(c, Put::And),
            Op::FillRows => self.fill_rows(c),
            Op::BarCell => self.bar_cell(c),
            Op::ClearDrawPage => self.clear_page(c),
            Op::ClearBothPages => self.clear_both(),
            Op::ClearScratchA => self.clear_scratch(Scratch::A),
            Op::ClearScratchB => self.clear_scratch(Scratch::B),
            // MGA 的 3 條掃描線複製只是 SD 呈現的需要，HD 不重現。
            Op::EndOfFrameCopy => {}
            Op::Number => {
                let is_frame_counter = c.text_x == COUNTER_X && c.text_y == COUNTER_Y;
                if self.counter_font.is_some() || !is_frame_counter || self.show_counter {
                    self.counter(c.text_x, c.text_y, c.cx);
                }
            }
            Op::ScrollShowA => self.stage3_view(c, false),
            Op::ScrollShowB => self.stage3_view(c, true),
            Op::ScrollShiftBothLeft => self.stage3_wrap(),
            // E 專用：暫存區 B 的逐平面維護，單色沒有。
            Op::ScrollRotateCopy | Op::ScrollShiftLeft4 | Op::ScrollRotateColumn | Op::ScrollMergeSeam => {}
        }
    }

    // ---- 呈現 ----

    /// 輸出畫面的像素尺寸，固定 1280×800。
    pub fn size(&self) -> (usize, usize) {
        (MONO_HD_W, MONO_HD_H)
    }

    /// 輸出顯示中那一頁的 RGBA：區塊位元 1 是亮點（白），0 是暗（黑），alpha 255。
    /// `display_page` 是核心的頁偏移（0 或 8000h）。邏輯第 `n` 列（0 至 199）取列位址表的那一列，
    /// 輸出為 HD 的第 `4n` 至 `4n + 3` 列，MGA 的 2000h 組（中間的掃描線複本）不用。
    pub fn render_rgba(&self, display_page: u16, out: &mut Vec<u8>) -> (usize, usize) {
        out.clear();
        out.resize(MONO_HD_W * MONO_HD_H * 4, 0);
        for px in out.chunks_exact_mut(4) {
            px[3] = 255;
        }
        let page = self.page(display_page) as usize;
        for n in 0..MONO_HD_H / BLK_H {
            let base = page + self.row(n as u16) as usize;
            for j in 0..MONO_HD_W / BLK_W {
                let blk = self.vmem.get(base + j).copied().unwrap_or(0);
                if blk == 0 {
                    continue;
                }
                for r in 0..BLK_H {
                    let bits = (blk >> (16 * r)) as u16;
                    if bits == 0 {
                        continue;
                    }
                    let o = ((n * BLK_H + r) * MONO_HD_W + j * BLK_W) * 4;
                    for x in 0..BLK_W {
                        if bits >> x & 1 != 0 {
                            out[o + x * 4] = 255;
                            out[o + x * 4 + 1] = 255;
                            out[o + x * 4 + 2] = 255;
                        }
                    }
                }
            }
        }
        (MONO_HD_W, MONO_HD_H)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mono::MonoSink;

    // ---- 合成圖庫與 HD 圖 ----

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

    /// 多項圖庫：位移表 `n + 1` 項（最後一項指向結尾的哨兵），每項 `[高][每列位元組數][像素]`。
    fn bank_n(items: &[(u16, u16, Vec<u8>)]) -> MonoBank {
        let table = 2 * (items.len() + 1);
        let mut ptrs = Vec::new();
        let mut off = table;
        for (_, _, d) in items {
            ptrs.push(off);
            off += 4 + d.len();
        }
        ptrs.push(off);
        let mut raw = Vec::new();
        for p in ptrs {
            raw.extend_from_slice(&(p as u16).to_le_bytes());
        }
        for (h, s, d) in items {
            raw.extend_from_slice(&h.to_le_bytes());
            raw.extend_from_slice(&s.to_le_bytes());
            raw.extend_from_slice(d);
        }
        raw.extend_from_slice(&[0xFF, 0xFF, 0, 0]);
        MonoBank { raw }
    }

    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> TmpDir {
            let p = std::env::temp_dir().join(format!("rs2-mono-hd-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_hd(root: &Path, name: &str, index: u16, w: usize, h: usize, on: impl Fn(usize, usize) -> bool) {
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let v = if on(x, y) { 255 } else { 0 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let dir = root.join("mono").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{index:04}.png")), png::encode_rgba(w, h, &rgba)).unwrap();
    }

    /// SD 項的最近鄰放大圖，寫成 HD 的 PNG。
    fn write_nn(root: &Path, name: &str, index: u16, h: u16, s: u16, data: &[u8]) {
        let (w, hh) = (s as usize * 16, h as usize * 4);
        write_hd(root, name, index, w, hh, |x, y| {
            let byte = data[(y / 4) * s as usize + x / 16];
            byte >> (7 - (x % 16) / 2) & 1 != 0
        });
    }

    fn pat_f(x: usize, y: usize) -> bool {
        (x * 3 + y * 5) % 7 < 3
    }

    fn pat_g(x: usize, y: usize) -> bool {
        (x + y * 2) % 5 < 2
    }

    fn pat_m(x: usize, y: usize) -> bool {
        (x / 3 + y) % 2 == 0
    }

    fn px(buf: &[u8], x: usize, y: usize) -> bool {
        buf[(y * MONO_HD_W + x) * 4] == 255
    }

    fn render(s: &MonoHdSink, page: u16) -> Vec<u8> {
        let mut buf = Vec::new();
        assert_eq!(s.render_rgba(page, &mut buf), (1280, 800));
        buf
    }

    /// 2 項的 MY 與 1 項的遮罩，HD 圖是三種不同的圖樣。
    fn pattern_sink(kind: MonoKind, tag: &str) -> (MonoHdSink, TmpDir) {
        let t = TmpDir::new(tag);
        write_hd(&t.0, "MY.BDB", 0, 32, 4, pat_f);
        write_hd(&t.0, "MY.BDB", 1, 32, 4, pat_g);
        write_hd(&t.0, "MYB.BDT", 0, 32, 4, pat_m);
        let assets = MonoAssets {
            my: bank_n(&[(1, 2, vec![0xAA, 0x55]), (1, 2, vec![0xF0, 0x0F])]),
            my_mask: bank_n(&[(1, 2, vec![0xFF, 0x00])]),
            ..MonoAssets::default()
        };
        (MonoHdSink::with_kind(kind, Arc::new(assets), &t.0), t)
    }

    /// 在 (c.x = 3, c.y = 1) 畫的 HD 像素 `(X, Y)` 對應的畫面座標。
    fn at(x: usize, y: usize) -> (usize, usize) {
        (3 * 2 * 16 + x, 4 + y)
    }

    fn at_cmd(op: Op, index: u16) -> DrawCmd {
        let mut c = cmd(op);
        c.x = 3;
        c.y = 1;
        c.index = index;
        DrawCmd::Blit(c)
    }

    // ---- 區塊 ----

    #[test]
    fn nn_block_doubles_width_and_quadruples_height() {
        let b = nn_block(0x80);
        let want: u64 = [0usize, 1].iter().flat_map(|&x| (0..4).map(move |y| 1u64 << (y * 16 + x))).sum();
        assert_eq!(b, want);
        let b = nn_block(0x01);
        let want: u64 = [14usize, 15].iter().flat_map(|&x| (0..4).map(move |y| 1u64 << (y * 16 + x))).sum();
        assert_eq!(b, want);
        assert_eq!(nn_block(0), 0);
        assert_eq!(nn_block(0xFF), u64::MAX);
        assert_eq!(MonoHdSink::nn_block(0xAA), nn_block(0xAA));
    }

    // ---- 運算 ----

    #[test]
    fn every_put_operation_uses_the_hd_image() {
        for kind in [MonoKind::Cga, MonoKind::Mga] {
            let cases: [(Op, u16, fn(usize, usize) -> bool); 5] = [
                // (運算, 圖號, 預期：底圖是 f 時的結果)
                (Op::MyOverlay, 1, |x, y| pat_f(x, y) | pat_g(x, y)),
                (Op::MyOverwriteInverted, 1, |x, y| !pat_g(x, y)),
                (Op::MyFlash, 0, |x, y| pat_f(x, y) | !pat_m(x, y)),
                (Op::MyCut, 0, |x, y| pat_f(x, y) & pat_m(x, y)),
                (Op::MyOverwrite, 1, |x, y| pat_g(x, y)),
            ];
            for (op, index, want) in cases {
                let (mut s, _t) = pattern_sink(kind, &format!("ops-{op:?}-{kind:?}"));
                s.apply(&at_cmd(Op::MyOverwrite, 0));
                let buf = render(&s, 0);
                for y in 0..4 {
                    for x in 0..32 {
                        let (px_x, px_y) = at(x, y);
                        assert_eq!(px(&buf, px_x, px_y), pat_f(x, y), "{kind:?} 底圖 ({x},{y})");
                    }
                }
                s.apply(&at_cmd(op, index));
                let buf = render(&s, 0);
                for y in 0..4 {
                    for x in 0..32 {
                        let (px_x, px_y) = at(x, y);
                        assert_eq!(px(&buf, px_x, px_y), want(x, y), "{kind:?} {op:?} ({x},{y})");
                    }
                }
                assert_eq!(s.anomalies(), 0);
                assert!(s.missing().is_empty(), "{:?}", s.missing());
                assert_eq!(s.hd_loaded(), 2);
            }
        }
    }

    #[test]
    fn pages_are_separate_and_rows_use_the_logical_row_table() {
        for kind in [MonoKind::Cga, MonoKind::Mga] {
            let (mut s, _t) = pattern_sink(kind, &format!("pages-{kind:?}"));
            let mut c = cmd(Op::MyOverwrite);
            c.x = 3;
            c.y = 1;
            c.page = 0x8000;
            s.apply(&DrawCmd::Blit(c));
            let p0 = render(&s, 0);
            let p1 = render(&s, 0x8000);
            assert!(p0.chunks_exact(4).all(|p| p[0] == 0), "{kind:?} 頁 0 要是空的");
            let (x0, y0) = at(0, 0);
            let lit = (0..32).any(|x| (0..4).any(|y| px(&p1, x0 + x, y0 + y)));
            assert!(lit, "{kind:?} 頁 1 要有圖");
            // 邏輯列 1 是 HD 的 4 至 7 列，別處是空的。
            for y in 0..800 {
                if !(4..8).contains(&y) {
                    assert!((0..1280).all(|x| !px(&p1, x, y)), "{kind:?} 第 {y} 列不該有東西");
                }
            }
        }
    }

    #[test]
    fn rows_past_190_and_words_outside_the_row_are_skipped() {
        let (mut s, _t) = pattern_sink(MonoKind::Cga, "clip");
        let mut c = cmd(Op::MyOverwrite);
        c.y = 191;
        s.apply(&DrawCmd::Blit(c));
        assert!(s.vram_blocks().iter().all(|&b| b == 0), "列 191 不畫");
        c.y = 190;
        s.apply(&DrawCmd::Blit(c));
        assert_ne!(s.vram_blocks()[95 * 0x50], 0, "列 190 要畫（偶數列，在 95 × 50h）");
        // x = 40 → 位元組 80，超出列尾：整個字組不寫。
        let (mut s, _t) = pattern_sink(MonoKind::Cga, "clip2");
        let mut c = cmd(Op::MyOverwrite);
        c.x = 40;
        s.apply(&DrawCmd::Blit(c));
        assert!(s.vram_blocks().iter().all(|&b| b == 0));
        // y < 0：上緣被裁掉，1 列的圖整個不畫。
        let mut c = cmd(Op::MyOverwrite);
        c.y = 0xFFFF;
        s.apply(&DrawCmd::Blit(c));
        assert!(s.vram_blocks().iter().all(|&b| b == 0));
    }

    #[test]
    fn index_out_of_the_bank_is_an_anomaly_like_the_original() {
        let (mut s, _t) = pattern_sink(MonoKind::Cga, "oob");
        let mut c = cmd(Op::MyOverwrite);
        c.index = 0x7000;
        s.apply(&DrawCmd::Blit(c));
        assert_eq!(s.anomalies(), 1);
        assert!(s.vram_blocks().iter().all(|&b| b == 0));
    }

    // ---- 缺圖 ----

    #[test]
    fn missing_folder_falls_back_to_nearest_neighbour_and_is_reported() {
        let assets = MonoAssets { my: bank_n(&[(1, 2, vec![0xAA, 0x55])]), ..MonoAssets::default() };
        let mut s = MonoHdSink::new(Arc::new(assets), Path::new("/nonexistent-rs2-hd"));
        assert_eq!(s.missing().len(), 1, "資料夾不存在記一筆");
        s.apply(&DrawCmd::Blit(cmd(Op::MyOverwrite)));
        assert_eq!(&s.vram_blocks()[0..2], &[nn_block(0xAA), nn_block(0x55)]);
        assert_eq!(s.hd_loaded(), 0);
        assert_eq!(s.anomalies(), 0);
    }

    #[test]
    fn missing_file_and_wrong_size_fall_back_and_are_reported_once() {
        let t = TmpDir::new("missing");
        write_hd(&t.0, "MY.BDB", 1, 30, 4, pat_f); // 預期 32x4
        let assets = MonoAssets { my: bank_n(&[(1, 2, vec![0xAA, 0x55]), (1, 2, vec![0xF0, 0x0F])]), ..MonoAssets::default() };
        let mut s = MonoHdSink::new(Arc::new(assets), &t.0);
        assert!(s.missing().is_empty());
        for _ in 0..3 {
            s.apply(&at_cmd(Op::MyOverwrite, 0));
            s.apply(&at_cmd(Op::MyOverwrite, 1));
        }
        assert_eq!(s.missing().len(), 2, "{:?}", s.missing());
        assert_eq!(s.missing()[0], "MY.BDB/0000.png");
        assert!(s.missing()[1].contains("尺寸 30x4，預期 32x4"), "{:?}", s.missing());
        assert_eq!(&s.vram_blocks()[0x2006..0x2008], &[nn_block(0xF0), nn_block(0x0F)]);
        assert_eq!(s.hd_loaded(), 0);
    }

    // ---- 暫存區 ----

    #[test]
    fn scratch_copy_uses_hd_blocks_and_picks_a_or_b() {
        let t = TmpDir::new("scratch");
        write_hd(&t.0, "SHT1.BDB", 0, 32, 4, pat_f);
        let mut sht: [MonoBank; 5] = Default::default();
        sht[0] = bank_n(&[(1, 2, vec![0x3C, 0xC3])]);
        let assets = MonoAssets { sht, ..MonoAssets::default() };
        let mut s = MonoHdSink::with_kind(MonoKind::Mga, Arc::new(assets), &t.0);
        // 圖寫進暫存區 B 的第 3 列、位元組欄 4。
        let mut c = cmd(Op::StageToScratch);
        c.x = 2;
        c.y = 3;
        c.scratch = 0x8000;
        s.apply(&DrawCmd::Blit(c));
        assert!(s.scratch_a_blocks().iter().all(|&b| b == 0));
        let off = 3 * 0xA0 + 4;
        assert_eq!(s.scratch_b_blocks()[off], block_from_pattern(0));
        assert_eq!(s.scratch_b_blocks()[off + 1], block_from_pattern(1));
        // 從 A 搬：沒東西。
        let mut copy = cmd(Op::ScratchReplace);
        copy.cx = 1;
        copy.src_y = 3;
        s.apply(&DrawCmd::Blit(copy));
        assert!(s.vram_blocks().iter().all(|&b| b == 0));
        // 從 B 搬到頁 0 的第 0 列。
        copy.scratch = 0x8000;
        s.apply(&DrawCmd::Blit(copy));
        let buf = render(&s, 0);
        for y in 0..4 {
            for x in 0..32 {
                assert_eq!(px(&buf, 4 * 16 + x, y), pat_f(x, y), "({x},{y})");
            }
        }
        // 清暫存區 B。
        s.apply(&DrawCmd::Blit(cmd(Op::ClearScratchB)));
        assert_eq!(s.scratch_b_blocks()[off], 0, "清暫存區 B 的前 7FFEh 位元組，列 3 在範圍內");
    }

    /// 圖樣 f 的第 `col` 個位元組欄（16 寬）對應的區塊。
    fn block_from_pattern(col: usize) -> u64 {
        let mut v = 0u64;
        for y in 0..4 {
            for x in 0..16 {
                if pat_f(col * 16 + x, y) {
                    v |= 1 << (y * 16 + x);
                }
            }
        }
        v
    }

    #[test]
    fn clears_follow_the_original_extents() {
        let mut c = MonoHdSink::new(Arc::new(MonoAssets::default()), Path::new("/nonexistent-rs2-hd"));
        c.vmem.iter_mut().for_each(|b| *b = 1);
        c.apply(&DrawCmd::Blit(cmd(Op::ClearBothPages)));
        assert_eq!((c.vmem[0], c.vmem[0x7FFD], c.vmem[0x7FFE]), (0, 0, 1), "CGA 3FFFh 個字組");
        let mut h = MonoHdSink::with_kind(MonoKind::Mga, Arc::new(MonoAssets::default()), Path::new("/nonexistent-rs2-hd"));
        h.vmem.iter_mut().for_each(|b| *b = 1);
        let mut p = cmd(Op::ClearDrawPage);
        p.page = 0x8000;
        h.apply(&DrawCmd::Blit(p));
        assert_eq!((h.vmem[0x7FFF], h.vmem[0x8000], h.vmem[0xFFFD], h.vmem[0xFFFE]), (1, 0, 0, 1));
        // VramFill 的清頁（核心的 sub_1291A、sub_128D9）。
        h.vmem.iter_mut().for_each(|b| *b = 1);
        h.apply(&DrawCmd::VramFill { off: 0, words: 0x0FA0, value: 0 });
        assert_eq!((h.vmem[0], h.vmem[0x7FFD], h.vmem[0x7FFE], h.vmem[0x8000]), (0, 0, 1, 1));
        h.apply(&DrawCmd::VramFill { off: 0, words: 5, value: 0 });
        assert_eq!(h.anomalies(), 1);
    }

    #[test]
    fn stage3_entry_display_copy_is_undone_and_wrap_shifts_scratch() {
        let mut s = MonoHdSink::new(Arc::new(MonoAssets::default()), Path::new("/nonexistent-rs2-hd"));
        s.sa[0] = 0xABCD;
        s.apply(&DrawCmd::Blit(cmd(Op::ScrollShowA)));
        assert_eq!(s.vmem[0], 0xABCD);
        s.apply(&DrawCmd::Blit(cmd(Op::ScrollRotateCopy)));
        assert_eq!(s.vmem[0], 0, "進場：ScrollShowA 後面接 ScrollRotateCopy，單色沒有這一步顯示");
        s.apply(&DrawCmd::Blit(cmd(Op::ScrollShowA)));
        s.apply(&DrawCmd::Blit(cmd(Op::ScrollRotateColumn)));
        assert_eq!(s.vmem[0], 0xABCD, "捲動：ScrollShowA 後面不接 ScrollRotateCopy，保留");
        s.sa[0x50] = 0x99;
        s.apply(&DrawCmd::Blit(cmd(Op::ScrollShiftBothLeft)));
        assert_eq!(s.sa[0], 0x99);
    }

    // ---- 幀計數、填色、血條、MGA ----

    #[test]
    fn counter_glyph_is_scaled_2x4_and_cga_draws_page_0_only() {
        let mut font = vec![0u8; 2048];
        // '1' 的第 0 列 0x81，第 7 列 0xFF。
        font[0x31 * 8] = 0x81;
        font[0x31 * 8 + 7] = 0xFF;
        for kind in [MonoKind::Cga, MonoKind::Mga] {
            let mut s = MonoHdSink::with_kind(kind, Arc::new(MonoAssets::default()), Path::new("/nonexistent-rs2-hd"));
            s.set_counter_font(&font);
            let mut c = cmd(Op::Number);
            c.text_x = COUNTER_X;
            c.text_y = COUNTER_Y;
            c.cx = 1; // 00001
            s.apply(&DrawCmd::Blit(c));
            // 第 5 位數字在欄 COUNTER_X + 4，列 COUNTER_Y + r。
            let off = s.row(COUNTER_Y).wrapping_add(COUNTER_X + 4) as usize;
            assert_eq!(s.vmem[off], nn_block(0x81), "{kind:?}");
            let off7 = s.row(COUNTER_Y + 7).wrapping_add(COUNTER_X + 4) as usize;
            assert_eq!(s.vmem[off7], u64::MAX, "{kind:?}");
            let page1 = off.wrapping_sub(0x8000) & 0xFFFF;
            match kind {
                MonoKind::Cga => assert_eq!(s.vmem[page1], 0, "CGA 只有第一頁有幀計數"),
                MonoKind::Mga => assert_eq!(s.vmem[page1], nn_block(0x81), "MGA 兩頁都有"),
            }
            // 畫面：最亮的一列是 4 個 HD 列 x 16 欄全亮。
            let buf = render(&s, 0);
            let row = (COUNTER_Y as usize + 7) * 4;
            let col = (COUNTER_X as usize + 4) * 16;
            assert!((0..4).all(|r| (0..16).all(|x| px(&buf, col + x, row + r))));
        }
    }

    #[test]
    fn counter_is_off_by_default_without_a_font() {
        let mut s = MonoHdSink::new(Arc::new(MonoAssets::default()), Path::new("/nonexistent-rs2-hd"));
        let mut c = cmd(Op::Number);
        c.text_x = COUNTER_X;
        c.text_y = COUNTER_Y;
        c.cx = 12345;
        s.apply(&DrawCmd::Blit(c));
        assert!(s.vram_blocks().iter().all(|&b| b == 0));
        s.show_counter = true;
        s.apply(&DrawCmd::Blit(c));
        assert!(s.vram_blocks().iter().any(|&b| b != 0));
    }

    #[test]
    fn fill_and_bar_write_the_nn_pattern_and_the_other_page() {
        for kind in [MonoKind::Cga, MonoKind::Mga] {
            let mut s = MonoHdSink::with_kind(kind, Arc::new(MonoAssets::default()), Path::new("/nonexistent-rs2-hd"));
            let mut f = cmd(Op::FillRows);
            f.cx = 2;
            f.dx = 1; // 0AAAAh
            s.apply(&DrawCmd::Blit(f));
            assert_eq!(s.vmem[0], nn_block(0xAA));
            assert_eq!(s.vmem[79], nn_block(0xAA));
            assert_eq!(s.vmem[s.row(1) as usize], nn_block(0xAA));
            let mut b = cmd(Op::BarCell);
            b.x = 1;
            b.y = 10;
            b.dx = 0x0B;
            s.apply(&DrawCmd::Blit(b));
            let other = match kind {
                MonoKind::Cga => 0x4000,
                MonoKind::Mga => 0x8000,
            };
            let at = s.row(10) as usize + 2;
            assert_eq!(s.vmem[at], nn_block(0xAA));
            assert_eq!(s.vmem[at ^ other], nn_block(0xAA), "{kind:?} 血條另一頁也寫");
        }
    }

    #[test]
    fn hercules_scanline_copy_does_nothing_in_hd() {
        let mut h = MonoHdSink::with_kind(MonoKind::Mga, Arc::new(MonoAssets::default()), Path::new("/nonexistent-rs2-hd"));
        h.vmem[0] = 0x1234;
        h.apply(&DrawCmd::Blit(cmd(Op::EndOfFrameCopy)));
        assert_eq!(h.vmem[0x2000], 0);
        assert_eq!(h.applied(), 1);
    }

    #[test]
    fn mga_and_cga_render_the_same_picture() {
        let mut shots = Vec::new();
        for kind in [MonoKind::Cga, MonoKind::Mga] {
            let (mut s, _t) = pattern_sink(kind, &format!("same-{kind:?}"));
            for i in 0..10u16 {
                let mut c = cmd(Op::MyOverwrite);
                c.x = i * 3;
                c.y = i * 17;
                c.index = i % 2;
                s.apply(&DrawCmd::Blit(c));
            }
            shots.push(render(&s, 0));
        }
        assert!(shots[0] == shots[1]);
        assert!(shots[0].chunks_exact(4).any(|p| p[0] == 255));
    }

    #[test]
    fn snapshot_is_scaled_nearest_neighbour() {
        let mut vm = vec![0u8; 0x10000];
        vm[0x8000] = 0x0F; // CGA 頁 0 的第一個位元組
        vm[0xC000] = 0xF0; // CGA 頁 1（4000h）
        let mut s = MonoHdSink::new(Arc::new(MonoAssets::default()), Path::new("/nonexistent-rs2-hd"));
        s.load_snapshot(&vm, &vec![7u8; SCRATCH_SIZE], &vec![0u8; SCRATCH_SIZE]);
        assert_eq!((s.vmem[0], s.vmem[0x4000]), (nn_block(0x0F), nn_block(0xF0)));
        assert_eq!(s.sa[10], nn_block(7));
    }

    // ---- 對 MonoSink 的差分測試 ----

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    const ALL_OPS: [Op; 43] = [
        Op::MyOverwrite,
        Op::MyOverlay,
        Op::MyCut,
        Op::MyFlash,
        Op::MyOverlayEffect,
        Op::MyCutEffect,
        Op::MyCutPlayerShot,
        Op::MyOverlayPlayerShot,
        Op::StageCut,
        Op::StageOverlay,
        Op::StageOverwrite,
        Op::StageFlash,
        Op::StageToScratch,
        Op::ScratchReplace,
        Op::ClearDrawPage,
        Op::ScratchReplaceShifted,
        Op::ScratchOr,
        Op::ScratchAnd,
        Op::FillRows,
        Op::BarCell,
        Op::Number,
        Op::ClearScratchB,
        Op::ClearScratchA,
        Op::ClearBothPages,
        Op::MyOverwriteInverted,
        Op::BegOverwrite,
        Op::BegInverted,
        Op::BegToScratch,
        Op::ScratchReplaceMirrored,
        Op::ScratchOrPairs,
        Op::ScratchOrMirrored,
        Op::ScratchLatchPairs,
        Op::ScratchAndPairs,
        Op::ScratchAndMirrored,
        Op::MapToScratch,
        Op::ScrollRotateCopy,
        Op::ScrollShiftLeft4,
        Op::ScrollRotateColumn,
        Op::ScrollMergeSeam,
        Op::ScrollShowA,
        Op::ScrollShowB,
        Op::ScrollShiftBothLeft,
        Op::EndOfFrameCopy,
    ];

    const SHT_GROUPS: [ShtGroup; 8] =
        [ShtGroup::Bh1, ShtGroup::Bh2, ShtGroup::Bh4, ShtGroup::Sht1, ShtGroup::Sht2, ShtGroup::Sht3, ShtGroup::Sht4, ShtGroup::Sht5];
    const BEG_GROUPS: [BegGroup; 5] = [BegGroup::Beg1, BegGroup::J1, BegGroup::J2, BegGroup::J3, BegGroup::J4];

    type Items = Vec<(u16, u16, Vec<u8>)>;

    /// 21 個圖庫都填隨機內容的合成圖庫，與每個圖庫的項目（寫 PNG 用）。
    fn synth_assets(rng: &mut Rng) -> (MonoAssets, Vec<(&'static str, Items)>) {
        let mut all: Vec<(&'static str, Items)> = Vec::new();
        let mut mk = |rng: &mut Rng, name: &'static str| -> MonoBank {
            let n = 3 + rng.below(3) as usize;
            let items: Items = (0..n)
                .map(|_| {
                    let (h, s) = if name == "MAP.BDB" { (10u16, 2u16) } else { (1 + rng.below(9) as u16, 1 + rng.below(5) as u16) };
                    // 一部分位元組全 0 或全 1，讓 AND、OR 的差異看得出來。
                    let data = (0..h * s)
                        .map(|_| match rng.below(6) {
                            0 => 0x00,
                            1 => 0xFF,
                            _ => rng.next() as u8,
                        })
                        .collect();
                    (h, s, data)
                })
                .collect();
            let b = bank_n(&items);
            all.push((name, items));
            b
        };
        let my = mk(rng, "MY.BDB");
        let my_mask = mk(rng, "MYB.BDT");
        let mut sht: [MonoBank; 5] = Default::default();
        let mut sht_mask: [MonoBank; 5] = Default::default();
        let names = [("SHT1.BDB", "SHT1.BDH"), ("SHT2.BDB", "SHT2.BDH"), ("SHT3.BDB", "SHT3.BDH"), ("SHT4.BDB", "SHT4.BDH"), ("SHT5.BDB", "SHT5.BDH")];
        for (i, (a, b)) in names.into_iter().enumerate() {
            sht[i] = mk(rng, a);
            sht_mask[i] = mk(rng, b);
        }
        let bh = [mk(rng, "BH1.BDB"), mk(rng, "BH2.BDB"), mk(rng, "BH4.BDB")];
        let map = mk(rng, "MAP.BDB");
        let j = [mk(rng, "J1.GOB"), mk(rng, "J2.GOB"), mk(rng, "J3.GOB"), mk(rng, "J4.GOB")];
        let beg1 = mk(rng, "BEG1.BDB");
        (MonoAssets { my, my_mask, sht, sht_mask, bh, map, j, beg1, unverified: Vec::new() }, all)
    }

    fn rand_cmd(rng: &mut Rng) -> BlitCmd {
        let mut c = cmd(ALL_OPS[rng.below(ALL_OPS.len() as u64) as usize]);
        c.x = if rng.below(10) == 0 { rng.below(100) as u16 } else { rng.below(40) as u16 };
        c.y = match rng.below(8) {
            0 => 0u16.wrapping_sub(rng.below(12) as u16),
            1 => 185 + rng.below(20) as u16,
            _ => rng.below(190) as u16,
        };
        c.index = rng.below(3) as u16;
        c.page = if rng.below(2) == 0 { 0 } else { 0x8000 };
        c.scratch = if rng.below(2) == 0 { 0 } else { 0x8000 };
        c.src_x = rng.below(60) as u16;
        c.src_y = rng.below(40) as u16;
        c.shift = rng.below(90) as u16;
        c.cx = match rng.below(10) {
            0 => 0,
            1 => 0u16.wrapping_sub(rng.below(5) as u16),
            _ => rng.below(25) as u16,
        };
        c.dx = [1, 0x0B, 0, 0xFFFF, rng.next() as u16][rng.below(5) as usize];
        c.text_x = rng.below(70) as u16;
        c.text_y = rng.below(190) as u16;
        c.sht_group = SHT_GROUPS[rng.below(8) as usize];
        c.mask_group = if rng.below(8) == 0 { SHT_GROUPS[rng.below(3) as usize] } else { SHT_GROUPS[3 + rng.below(5) as usize] };
        c.beg_group = BEG_GROUPS[rng.below(5) as usize];
        c
    }

    fn assert_same(sd: &MonoSink, hd: &MonoHdSink, ctx: &str) {
        let pairs: [(&str, &[u8], &[u64]); 3] = [
            ("顯示記憶體", sd.vram(), hd.vram_blocks()),
            ("暫存區 A", sd.scratch_a(), hd.scratch_a_blocks()),
            ("暫存區 B", sd.scratch_b(), hd.scratch_b_blocks()),
        ];
        for (what, a, b) in pairs {
            assert_eq!(a.len(), b.len(), "{ctx} {what} 長度");
            if let Some(i) = (0..a.len()).find(|&i| nn_block(a[i]) != b[i]) {
                panic!("{ctx}：{what} 位移 {i:04X} SD {:02X} 的放大不等於 HD 區塊 {:016X}", a[i], b[i]);
            }
        }
        assert_eq!(sd.anomalies(), hd.anomalies(), "{ctx} 異常次數");
    }

    /// 同一批隨機命令餵 `MonoSink` 與 `MonoHdSink`，每個位元組的最近鄰放大必須等於 HD 區塊。
    /// `png` 為真時 HD 圖是 SD 項的最近鄰放大 PNG（走 PNG 載入與索引路徑），否則走缺圖退回。
    fn differential(kind: MonoKind, seed: u64, via_png: bool, ops: usize) {
        let mut rng = Rng(seed);
        let (assets, all) = synth_assets(&mut rng);
        let assets = Arc::new(assets);
        let t = TmpDir::new(&format!("fuzz-{kind:?}-{seed}-{via_png}"));
        if via_png {
            for (name, items) in &all {
                for (i, (h, s, d)) in items.iter().enumerate() {
                    write_nn(&t.0, name, i as u16, *h, *s, d);
                }
            }
        }
        let dir = if via_png { t.0.clone() } else { PathBuf::from("/nonexistent-rs2-hd") };
        let mut sd = MonoSink::new(kind, Arc::clone(&assets));
        let mut hd = MonoHdSink::with_kind(kind, Arc::clone(&assets), &dir);
        let font: Vec<u8> = (0..2048).map(|_| rng.next() as u8).collect();
        sd.set_counter_font(&font);
        hd.set_counter_font(&font);
        // 起點：隨機的顯示記憶體與暫存區（HD 取最近鄰放大）。
        let rnd = |rng: &mut Rng, n: usize| -> Vec<u8> { (0..n).map(|_| if rng.below(3) == 0 { rng.next() as u8 } else { 0 }).collect() };
        let (v, a, b) = (rnd(&mut rng, 0x10000), rnd(&mut rng, SCRATCH_SIZE), rnd(&mut rng, SCRATCH_SIZE));
        sd.load_snapshot(&v, &a, &b);
        hd.load_snapshot(&v, &a, &b);
        assert_same(&sd, &hd, "起點");
        for n in 0..ops {
            let c = rand_cmd(&mut rng);
            // EndOfFrameCopy 是 MGA 的 SD 呈現專用（複製中間那條掃描線），HD 不動作。只餵 HD，SD 的狀態不變，
            // 這樣其餘狀態仍要逐位元組相同，同時驗證 HD 的 EndOfFrameCopy 確實什麼都沒改。
            if c.op != Op::EndOfFrameCopy {
                sd.apply(&DrawCmd::Blit(c));
            }
            hd.apply(&DrawCmd::Blit(c));
            if n % 16 == 0 || c.op == Op::ScrollRotateCopy {
                assert_same(&sd, &hd, &format!("{kind:?} seed {seed} png {via_png} 第 {n} 個命令 {c:?}"));
            }
        }
        assert_same(&sd, &hd, "結尾");
        if via_png {
            assert_eq!(hd.missing(), &[] as &[String], "PNG 全數存在");
            assert!(hd.hd_loaded() > 0);
        } else {
            assert_eq!(hd.hd_loaded(), 0);
        }
    }

    #[test]
    fn differential_against_mono_sink_nearest_neighbour_fallback() {
        for kind in [MonoKind::Cga, MonoKind::Mga] {
            for seed in 1..=4 {
                differential(kind, 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(seed), false, 3000);
            }
        }
    }

    #[test]
    fn differential_against_mono_sink_through_png_files() {
        for kind in [MonoKind::Cga, MonoKind::Mga] {
            for seed in 5..=7 {
                differential(kind, 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(seed), true, 2500);
            }
        }
    }

    // ---- 真實 HD 圖的尺寸稽核（需要原版檔與 HD 資料夾）----

    /// SD 圖庫項的像素（0 或 1），列優先，寬 `每列位元組數 × 8`。
    fn sd_bits(bank: &MonoBank, it: &Item) -> Vec<f32> {
        let (s, h) = (it.stride as usize, it.height as usize);
        let mut v = Vec::with_capacity(s * 8 * h);
        for r in 0..h {
            for c in 0..s * 8 {
                let b = bank.rd8(it.data + r * s + c / 8).unwrap_or(0);
                v.push(((b >> (7 - c % 8)) & 1) as f32);
            }
        }
        v
    }

    /// HD 圖縮回 SD 解析度：每個單色像素對應 HD 的 2x4，取亮點比例。
    fn hd_means(img: &png::Image, it: &Item) -> Vec<f32> {
        let (w, h) = (it.stride as usize * 8, it.height as usize);
        let mut v = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let mut n = 0u32;
                for yy in 0..4 {
                    for xx in 0..2 {
                        n += (img.rgba[((y * 4 + yy) * img.w + x * 2 + xx) * 4] >= 128) as u32;
                    }
                }
                v.push(n as f32 / 8.0);
            }
        }
        v
    }

    /// Pearson 相關係數。任一邊沒有變化（全同值）時沒有定義。
    fn pearson(a: &[f32], b: &[f32]) -> Option<f64> {
        if a.len() != b.len() || a.is_empty() {
            return None;
        }
        let n = a.len() as f64;
        let (ma, mb) = (a.iter().map(|&x| x as f64).sum::<f64>() / n, b.iter().map(|&x| x as f64).sum::<f64>() / n);
        let (mut saa, mut sbb, mut sab) = (0f64, 0f64, 0f64);
        for (&x, &y) in a.iter().zip(b) {
            let (dx, dy) = (x as f64 - ma, y as f64 - mb);
            saa += dx * dx;
            sbb += dy * dy;
            sab += dx * dy;
        }
        if saa < 1e-9 || sbb < 1e-9 {
            return None;
        }
        Some(sab / (saa * sbb).sqrt())
    }

    /// 每個 SD 圖庫項對照 `<RS2_HD>/mono/<檔名>/<索引>.png`：存在、可解碼、尺寸是 `每列位元組數 × 16` × `高度 × 4`，
    /// 並統計像素值是否只有 0 與 255、R=G=B。列出不符的項。
    /// `tools/rust.sh test -p rs2-gfx real_hd -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_hd_mono_images_match_the_sd_item_dimensions() {
        let orig = PathBuf::from(std::env::var("RS2_ORIG").expect("缺環境變數 RS2_ORIG，用 tools/rust.sh 跑"));
        let hd = PathBuf::from(std::env::var("RS2_HD").expect("缺環境變數 RS2_HD，用 tools/rust.sh 跑"));
        let assets = Arc::new(MonoAssets::load(&orig).expect("載入單色圖庫"));
        let dir = hd.join("mono");
        assert!(dir.is_dir(), "沒有 {}", dir.display());
        let (mut t_items, mut t_ok, mut t_missing, mut t_bad, mut t_skip, mut t_nonbin, mut t_alpha) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
        let mut problems: Vec<String> = Vec::new();
        let mut corrs: Vec<(f64, String)> = Vec::new();
        let mut wrong: Vec<f64> = Vec::new();
        for (name, bank) in bank_list(&assets) {
            let (mut items, mut ok, mut missing, mut bad, mut skip, mut nonbin, mut alpha) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
            let mut bank_corrs: Vec<f64> = Vec::new();
            let mut prev: Option<(u16, u16, Vec<f32>)> = None;
            for i in 0..bank.len() {
                let index = i as u16;
                let Some(it) = bank.item(index) else {
                    skip += 1;
                    continue;
                };
                items += 1;
                // 位移表的最後一項是結尾哨兵（高 65535），不是圖。
                if (it.height as i16) <= 0 || it.stride == 0 {
                    skip += 1;
                    if i + 1 != bank.len() {
                        problems.push(format!("{name}:{i} 高 {} 寬 {}，不是最後一項卻沒有圖", it.height, it.stride));
                    }
                    continue;
                }
                let (ew, eh) = (it.stride as usize * 16, it.height as usize * 4);
                let path = dir.join(name).join(format!("{index:04}.png"));
                let Ok(data) = std::fs::read(&path) else {
                    missing += 1;
                    problems.push(format!("{name}:{i} 缺檔，SD {}x{}（HD 預期 {ew}x{eh}）", it.stride as usize * 8, it.height));
                    continue;
                };
                match png::decode(&data) {
                    Err(e) => {
                        bad += 1;
                        problems.push(format!("{name}:{i} 解碼失敗：{e}"));
                    }
                    Ok(img) if img.w != ew || img.h != eh => {
                        bad += 1;
                        problems.push(format!("{name}:{i} 尺寸 {}x{}，預期 {ew}x{eh}（SD 每列 {} 位元組 × {} 列）", img.w, img.h, it.stride, it.height));
                    }
                    Ok(img) => {
                        ok += 1;
                        let nb = img.rgba.chunks_exact(4).filter(|p| !((p[0] == 0 || p[0] == 255) && p[1] == p[0] && p[2] == p[0])).count();
                        let na = img.rgba.chunks_exact(4).filter(|p| p[3] != 255).count();
                        nonbin += nb;
                        alpha += na;
                        if nb > 0 {
                            problems.push(format!("{name}:{i} 有 {nb} 個非 0／255 或 R≠G≠B 的像素"));
                        }
                        // HD 圖縮回 SD 解析度（每個單色像素 2x4）與 SD 原圖的相關係數；對照組是同庫上一張尺寸相同的 SD 圖。
                        let sd = sd_bits(bank, &it);
                        let hdm = hd_means(&img, &it);
                        if let Some(r) = pearson(&sd, &hdm) {
                            corrs.push((r, format!("{name}:{i}")));
                            bank_corrs.push(r);
                        }
                        if let Some((ps, ph, psd)) = &prev {
                            if (*ps, *ph) == (it.stride, it.height) {
                                if let Some(r) = pearson(psd, &hdm) {
                                    wrong.push(r);
                                }
                            }
                        }
                        prev = Some((it.stride, it.height, sd));
                    }
                }
            }
            bank_corrs.sort_by(|a, b| a.total_cmp(b));
            let med = bank_corrs.get(bank_corrs.len() / 2).copied().unwrap_or(f64::NAN);
            println!(
                "{name:10} 相關係數（HD 縮回 SD 解析度對 SD 原圖）：有效 {:4} 最小 {:.3} 中位數 {med:.3}",
                bank_corrs.len(),
                bank_corrs.first().copied().unwrap_or(f64::NAN)
            );
            println!("{name:10} 項 {items:3} 符合 {ok:3} 缺檔 {missing:3} 尺寸或解碼不符 {bad:3} 略過 {skip:2} 非二值像素 {nonbin} alpha<255 像素 {alpha}");
            t_items += items;
            t_ok += ok;
            t_missing += missing;
            t_bad += bad;
            t_skip += skip;
            t_nonbin += nonbin;
            t_alpha += alpha;
        }
        println!("合計：項 {t_items} 符合 {t_ok} 缺檔 {t_missing} 尺寸或解碼不符 {t_bad} 略過 {t_skip} 非二值像素 {t_nonbin} alpha<255 像素 {t_alpha}");
        for p in problems.iter().take(80) {
            println!("  {p}");
        }
        if problems.len() > 80 {
            println!("  ……還有 {} 筆", problems.len() - 80);
        }
        corrs.sort_by(|a, b| a.0.total_cmp(&b.0));
        wrong.sort_by(|a, b| a.total_cmp(b));
        let q = |v: &[f64], p: f64| v.get(((v.len() as f64 - 1.0) * p) as usize).copied().unwrap_or(f64::NAN);
        let right: Vec<f64> = corrs.iter().map(|c| c.0).collect();
        println!(
            "相關係數（有變化的圖）：配對正確 {} 張，最小 {:.3}、1 百分位 {:.3}、10 百分位 {:.3}、中位數 {:.3}",
            right.len(),
            q(&right, 0.0),
            q(&right, 0.01),
            q(&right, 0.1),
            q(&right, 0.5)
        );
        println!(
            "相關係數（對照：HD 圖配上一張尺寸相同的 SD 圖）：{} 組，中位數 {:.3}、90 百分位 {:.3}、最大 {:.3}",
            wrong.len(),
            q(&wrong, 0.5),
            q(&wrong, 0.9),
            q(&wrong, 1.0)
        );
        println!("相關係數最低的 12 張：{:?}", corrs.iter().take(12).map(|(r, n)| format!("{n} {r:.2}")).collect::<Vec<_>>());
        // MAP.BDB 第 0 項：SD 每列 2 位元組、10 列，HD 32x40。
        let it = assets.map.item(0).expect("MAP.BDB 第 0 項");
        assert_eq!((it.stride, it.height), (2, 10), "MAP.BDB 第 0 項的 SD 尺寸");
        // sink 的載入結果與稽核一致。
        let mut s = MonoHdSink::new(Arc::clone(&assets), &hd);
        let loaded = s.preload_all();
        println!("preload_all：載入 {loaded}，missing {}", s.missing().len());
        assert_eq!(loaded, t_ok, "sink 載入張數與稽核的符合張數相同");
        assert_eq!(s.missing().len(), t_missing + t_bad);
        // 21 個圖庫各有一個結尾哨兵；其餘每一項都要有尺寸相符的 HD 圖，像素只有 0 與 255。
        assert_eq!(t_skip, 21, "每個圖庫恰有一個哨兵");
        assert_eq!((t_missing, t_bad, t_nonbin), (0, 0, 0), "{problems:?}");
        assert_eq!(t_ok, t_items - t_skip);
        assert!(s.missing().is_empty(), "{:?}", s.missing());
    }
}
