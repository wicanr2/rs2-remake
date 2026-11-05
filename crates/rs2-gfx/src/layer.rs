//! `LayerSink`：吃和 `VgaSink` 相同的繪製命令，輸出可補間、可換 HD 圖的 RGBA 畫面。
//! 規格見 `docs/spec/001-rust-first-stage.md`「繪製模型」「前端」。
//!
//! 結構：
//! - 原版圖模式（`hd: false`）用索引色引擎（`idx.rs`），64 KB 位址空間每像素一個位元組，語意和 `VgaSink` 相同，
//!   補間關閉時逐像素相同。輸出 320×200。
//! - HD 模式（`hd: true`）用 RGBA 引擎（`hd.rs`），座標乘 4，輸出 1280×800。
//! - 補間：`push_frame` 把這一幀的命令與前一幀配對，算出每個命令的位移（`interp.rs`）。
//!   `render(alpha)` 取「這一幀開始前」的記憶體當底，複製一份，把這一幀的命令重放上去，
//!   帶位移的命令依 `alpha` 換成內插後的偏移。提交給下一幀的記憶體永遠由不內插的命令產生，
//!   所以補間不會污染畫面狀態，同一幀重複 `render` 也不會累積。
//! - 幀計數（座標 (12h, BFh) 的 `S1890D`）不寫進記憶體，`render` 時才疊上，所以 `show_counter` 可隨時切換。

mod hd;
mod hd_assets;
mod idx;
mod interp;
pub mod pal;

use crate::assets::Assets;
use crate::sink::{COUNTER_X, COUNTER_Y};
use hd::{Canvas, HdMem};
use interp::Delta;
use rs2_core::{BlitCmd, DrawCmd, Op};

pub use hd_assets::HdAssets;
pub use interp::{INTERP_CROSS_OP, INTERP_MAX_PX};

/// 引擎的偏移，單位是該引擎的像素（原版圖是邏輯像素，HD 是 HD 像素）。全 0 就是不內插。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Off {
    /// 目的欄。
    pub ox: i32,
    /// 目的列。
    pub oy: i32,
    /// 暫存區來源欄。
    pub sox: i32,
    /// `S1A287` 的列數。
    pub ocx: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOpts {
    /// 在前後兩個邏輯幀之間內插帶 `obj` 的命令。
    pub interpolate: bool,
    /// 用 HD 圖與 4 倍輸出。
    pub hd: bool,
    /// 畫幀計數（座標 (12h, BFh) 的 `S1890D`）。
    pub show_counter: bool,
}

impl Default for RenderOpts {
    fn default() -> Self {
        RenderOpts { interpolate: true, hd: false, show_counter: false }
    }
}

struct FrameData {
    cmds: Vec<DrawCmd>,
    deltas: Vec<Option<Delta>>,
    /// 這一幀畫的頁，也是翻頁後顯示的頁（0 或 8000h）。
    page: u16,
    any_delta: bool,
    /// 這一幀有暫存區寫入，而且寫入之後沒有再從暫存區複製：重放時可以直接借用「開始前」的暫存區並略過寫入，
    /// 不必複製 25 MB。否則（寫入之後還有複製）重放必須複製一份、照順序重放寫入。
    scratch_borrow_ok: bool,
    counter: Option<u16>,
}

impl FrameData {
    fn empty() -> FrameData {
        FrameData { cmds: Vec::new(), deltas: Vec::new(), page: 0, any_delta: false, scratch_borrow_ok: true, counter: None }
    }
}

pub struct LayerSink {
    assets: Assets,
    hd: Option<HdAssets>,
    /// 這一幀命令套完之後的調色盤，與這一幀開始前的。
    pal: pal::Pal,
    pal_before: pal::Pal,
    /// 索引色記憶體：套完這一幀（不內插）、這一幀開始前、重放用的暫存。
    idx: idx::Mem,
    idx_base: idx::Mem,
    idx_tmp: idx::Mem,
    idx_st: idx::St,
    idx_st_before: idx::St,
    /// HD 記憶體：套完這一幀。底圖只留這一幀畫的那一頁，暫存區只在這一幀有寫入時才另存。
    hdm: Option<HdMem>,
    hd_base_page: Canvas,
    hd_scratch_before: Option<[Canvas; 2]>,
    /// 上一次輸出換下來的緩衝區，下一次重放沿用，不必每次配置 4 MB。
    hd_spare: Vec<u8>,
    cur: FrameData,
    out: Vec<u8>,
    pushed: u64,
}

#[inline]
fn is_frame_counter(c: &BlitCmd) -> bool {
    c.op == Op::S1890D && c.text_x == COUNTER_X && c.text_y == COUNTER_Y
}

/// 這一幀畫的頁：清頁命令的頁優先，沒有就取最後一個寫顯示頁的命令。
fn page_of(cmds: &[DrawCmd]) -> Option<u16> {
    let (mut clear, mut any) = (None, None);
    for d in cmds {
        if let DrawCmd::Blit(c) = d {
            match c.op {
                Op::S19CF9 => clear = Some(c.page),
                Op::S19ADC | Op::S1152D | Op::S1890D | Op::S1929F => {}
                _ => any = Some(c.page),
            }
        }
    }
    clear.or(any)
}

impl LayerSink {
    pub fn new(assets: Assets, hd: Option<HdAssets>) -> LayerSink {
        let hdm = hd.as_ref().map(|_| HdMem::new());
        let hd_base_page = hdm.as_ref().map(|m| m.pages[0].clone()).unwrap_or_else(Canvas::empty);
        LayerSink {
            assets,
            hd,
            pal: pal::Pal::new(),
            pal_before: pal::Pal::new(),
            idx: idx::Mem::new(),
            idx_base: idx::Mem::new(),
            idx_tmp: idx::Mem::new(),
            idx_st: idx::St::default(),
            idx_st_before: idx::St::default(),
            hdm,
            hd_base_page,
            hd_scratch_before: None,
            hd_spare: Vec::new(),
            cur: FrameData::empty(),
            out: Vec::new(),
            pushed: 0,
        }
    }

    /// 套用一個邏輯幀的命令。`Game::new` 後 `take_draws()` 的初始化命令也走這裡。
    /// 空的命令串（暫停中的步）不算一幀，直接忽略，畫面維持上一幀。
    pub fn push_frame(&mut self, draws: &[DrawCmd]) {
        if draws.is_empty() {
            return;
        }
        let page = page_of(draws).unwrap_or(self.cur.page);
        let deltas = interp::pair(&self.cur.cmds, draws);
        let is_scratch_op = |d: &DrawCmd| matches!(d, DrawCmd::Blit(c) if matches!(c.op, Op::S19ADC | Op::S1152D));
        let is_copy_op = |d: &DrawCmd| matches!(d, DrawCmd::Blit(c) if matches!(c.op, Op::S19BA4 | Op::S19D08 | Op::S19DA8 | Op::S1A09F));
        let first_scratch = draws.iter().position(is_scratch_op);
        let scratch_ops = first_scratch.is_some();
        let scratch_borrow_ok = first_scratch.is_none_or(|i| !draws[i..].iter().any(is_copy_op));

        // 這一幀開始前的快照。
        self.idx_base.px.copy_from_slice(&self.idx.px);
        self.idx_st_before = self.idx_st;
        self.pal_before = self.pal.clone();
        if let Some(m) = &self.hdm {
            let p = ((page >> 15) & 1) as usize;
            self.hd_base_page = m.pages[p].clone();
            self.hd_scratch_before = scratch_ops.then(|| m.scratch.clone());
        }

        let mut counter = None;
        for d in draws {
            match d {
                DrawCmd::SetDac { start, rgb } => self.pal.set(*start, rgb),
                DrawCmd::Blit(c) => {
                    if is_frame_counter(c) {
                        counter = Some(c.cx);
                        continue;
                    }
                    idx::apply(&mut self.idx, &mut self.idx_st, &self.assets, c, Off::default());
                    if let (Some(m), Some(h)) = (&mut self.hdm, &mut self.hd) {
                        hd::apply(m, h, &self.assets, &self.pal, c, Off::default());
                    }
                }
            }
        }
        let any_delta = deltas.iter().any(|d| d.is_some());
        self.cur = FrameData { cmds: draws.to_vec(), deltas, page, any_delta, scratch_borrow_ok, counter };
        self.pushed += 1;
    }

    /// 輸出 RGBA。`alpha` 是前一幀到目前這幀的進度，0.0 到 1.0。
    /// 原版圖 320×200，HD 1280×800。沒有載入 HD 資料夾時，`hd: true` 輸出原版圖的最近鄰 4 倍。
    pub fn render(&mut self, alpha: f32, opts: RenderOpts) -> (&[u8], usize, usize) {
        let t = if alpha.is_nan() { 1.0 } else { alpha.clamp(0.0, 1.0) };
        let replay = opts.interpolate && self.cur.any_delta;
        if opts.hd && self.hdm.is_some() {
            self.render_hd(t, replay, opts.show_counter);
            return (&self.out, hd::PW as usize, hd::PH as usize);
        }
        // 原版圖：重放或直接用套完的記憶體。
        if replay {
            self.idx_tmp.px.copy_from_slice(&self.idx_base.px);
            let mut st = self.idx_st_before;
            for (i, d) in self.cur.cmds.iter().enumerate() {
                if let DrawCmd::Blit(c) = d {
                    if is_frame_counter(c) {
                        continue;
                    }
                    let off = self.cur.deltas[i].map(|dl| interp::offsets(&dl, t, 1)).unwrap_or_default();
                    idx::apply(&mut self.idx_tmp, &mut st, &self.assets, c, off);
                }
            }
        } else {
            self.idx_tmp.px.copy_from_slice(&self.idx.px);
        }
        if opts.show_counter {
            if let Some(v) = self.cur.counter {
                idx::counter(&mut self.idx_tmp, COUNTER_X, COUNTER_Y, v);
            }
        }
        let colors = self.pal.visible8();
        if opts.hd {
            // 沒有 HD 資料夾：原版圖最近鄰放大。
            let mut small = vec![0u8; 320 * 200 * 4];
            idx::page_to_rgba(&self.idx_tmp, self.cur.page, &colors, &mut small);
            self.out.resize(1280 * 800 * 4, 0);
            for y in 0..800usize {
                for x in 0..1280usize {
                    let s = ((y / 4) * 320 + x / 4) * 4;
                    let o = (y * 1280 + x) * 4;
                    self.out[o..o + 4].copy_from_slice(&small[s..s + 4]);
                }
            }
            return (&self.out, 1280, 800);
        }
        self.out.resize(320 * 200 * 4, 0);
        idx::page_to_rgba(&self.idx_tmp, self.cur.page, &colors, &mut self.out);
        (&self.out, 320, 200)
    }

    fn render_hd(&mut self, t: f32, replay: bool, show_counter: bool) {
        let Self { hdm, hd: hd_assets, assets, pal, pal_before, hd_base_page, hd_scratch_before, hd_spare, cur, out, .. } = self;
        let (Some(mem), Some(hda)) = (hdm.as_mut(), hd_assets.as_mut()) else { return };
        let pg = ((cur.page >> 15) & 1) as usize;
        // 目的緩衝區沿用上一次換下來的 Vec。
        let mut buf = std::mem::take(hd_spare);
        buf.clear();
        buf.extend_from_slice(if replay { &hd_base_page.px } else { &mem.pages[pg].px });
        let mut page = Canvas { px: buf };
        if replay {
            let mut tmp = HdMem { pages: [Canvas::empty(), Canvas::empty()], scratch: [Canvas::empty(), Canvas::empty()] };
            tmp.pages[pg] = page;
            // 暫存區的來源：沒有寫入的幀借用目前的；有寫入而且之後沒有複製的幀借用「開始前」的並略過寫入；
            // 其餘的複製一份「開始前」的，照順序重放寫入。
            enum Src {
                Current,
                Before,
                Cloned,
            }
            let src = match hd_scratch_before {
                None => Src::Current,
                Some(_) if cur.scratch_borrow_ok => Src::Before,
                Some(_) => Src::Cloned,
            };
            match (&src, hd_scratch_before.as_mut()) {
                (Src::Current, _) => std::mem::swap(&mut tmp.scratch, &mut mem.scratch),
                (Src::Before, Some(b)) => std::mem::swap(&mut tmp.scratch, b),
                (_, Some(b)) => tmp.scratch = b.clone(),
                _ => {}
            }
            let skip_scratch_ops = matches!(src, Src::Before);
            let mut p = pal_before.clone();
            for (i, d) in cur.cmds.iter().enumerate() {
                match d {
                    DrawCmd::SetDac { start, rgb } => p.set(*start, rgb),
                    DrawCmd::Blit(c) => {
                        if is_frame_counter(c) || (skip_scratch_ops && matches!(c.op, Op::S19ADC | Op::S1152D)) {
                            continue;
                        }
                        let off = cur.deltas[i].map(|dl| interp::offsets(&dl, t, hd::S)).unwrap_or_default();
                        hd::apply(&mut tmp, hda, assets, &p, c, off);
                    }
                }
            }
            match (&src, hd_scratch_before.as_mut()) {
                (Src::Current, _) => std::mem::swap(&mut tmp.scratch, &mut mem.scratch),
                (Src::Before, Some(b)) => std::mem::swap(&mut tmp.scratch, b),
                _ => {}
            }
            page = std::mem::replace(&mut tmp.pages[pg], Canvas::empty());
        }
        if show_counter {
            if let Some(v) = cur.counter {
                let mut pages = [page, Canvas::empty()];
                hd::counter(&mut pages, &pal.visible8(), COUNTER_X, COUNTER_Y, v, false);
                let [p0, _] = pages;
                page = p0;
            }
        }
        *hd_spare = std::mem::replace(out, page.px);
    }

    /// 預載第一關會用到的 HD 圖（`MY`、`SHT1`、`BH1` 的圖、遮罩與 `stage1_boss` 變體），避免第一次出現時在 `push_frame` 內解碼。
    /// 沒有 HD 資料夾時回傳 `None`。內容見 `HdAssets::preload`。
    pub fn preload_hd(&mut self, threads: usize) -> Option<(usize, usize)> {
        let Self { hd, assets, .. } = self;
        hd.as_mut().map(|h| h.preload(assets, threads))
    }

    /// 成功載入的 HD 檔（相對路徑，依載入順序），診斷用。
    pub fn loaded_hd(&self) -> &[String] {
        self.hd.as_ref().map(|h| h.loaded()).unwrap_or(&[])
    }

    /// 缺的 HD 圖，給啟動日誌與畫面標記。沒有載入 HD 資料夾時是空的。
    pub fn missing_hd(&self) -> &[String] {
        self.hd.as_ref().map(|h| h.missing()).unwrap_or(&[])
    }

    /// 這一幀畫的頁，也是翻頁後顯示的頁（0 或 8000h）。
    pub fn draw_page(&self) -> u16 {
        self.cur.page & 0x8000
    }

    /// 目前調色盤的名稱（`stage1`、`stage1_boss` 等），DAC 表不在已知清單內時為 `None`。
    pub fn palette_name(&self) -> Option<&'static str> {
        self.pal.name_str()
    }

    /// 已推入的幀數（含初始化命令）。
    pub fn frames_pushed(&self) -> u64 {
        self.pushed
    }

    /// 索引色引擎無法照機器碼重現的次數（圖庫索引或來源超出範圍等）。
    pub fn anomalies(&self) -> u64 {
        self.idx_st.anomalies
    }

    /// 目前這一幀有幾個命令帶位移（測試與診斷用）。
    pub fn interpolated_cmds(&self) -> usize {
        self.cur.deltas.iter().filter(|d| d.is_some()).count()
    }
}
