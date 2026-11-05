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
//! - 幀計數（座標 (12h, BFh) 的 `Number`）不寫進記憶體，`render` 時才疊上，所以 `show_counter` 可隨時切換。

mod hd;
mod hd_assets;
mod idx;
mod interp;
pub mod pal;
mod recolor;

use crate::assets::Assets;
use crate::geom::Geom;
use crate::sink::{COUNTER_X, COUNTER_Y};
use hd::{Canvas, HdMem};
use interp::Delta;
use rs2_game::{BlitCmd, DrawCmd, Op};

pub use hd_assets::{HdAssets, HdSet, Lib as HdLib};
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
    /// `FillRows` 的列數。
    pub ocx: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOpts {
    /// 在前後兩個邏輯幀之間內插帶 `obj` 的命令。
    pub interpolate: bool,
    /// 用 HD 圖與 4 倍輸出。
    pub hd: bool,
    /// 畫幀計數（座標 (12h, BFh) 的 `Number`）。
    pub show_counter: bool,
    /// 玩家機預測：相對最新邏輯位置的偏移（HD 像素，邏輯像素的 1/4），索引是玩家編號。
    /// `Some` 時這個玩家的本體、尾焰與砲座不走前後兩幀的補間，一律畫在最新位置加偏移。`None` 照一般補間。
    pub player: [Option<(i32, i32)>; 2],
}

impl Default for RenderOpts {
    fn default() -> Self {
        RenderOpts { interpolate: true, hd: false, show_counter: false, player: [None, None] }
    }
}

/// 邏輯像素對 HD 像素的倍率，`RenderOpts::player` 的單位。
const PLAYER_OFF_SCALE: i32 = 4;

/// 玩家機預測對這個命令的偏移。`scale` 是引擎像素對邏輯像素的倍率。不是玩家機的命令回傳 `None`。
/// `free_pods[p]` 為真時，玩家 `p` 的砲座已脫離機身，回傳 `None`。
fn player_off(player: &[Option<(i32, i32)>; 2], free_pods: &[bool; 2], c: &BlitCmd, scale: i32) -> Option<Off> {
    let o = c.obj?;
    let p = match o.pool {
        rs2_game::Pool::Player => o.slot as usize,
        // 砲座的位置每幀由玩家機位置算出（`sub_14908` 等），槽 0、1 屬於玩家 1，槽 2、3 屬於玩家 2。
        rs2_game::Pool::Pod => {
            let p = (o.slot / 2) as usize;
            if free_pods.get(p).copied().unwrap_or(false) {
                return None;
            }
            p
        }
        _ => return None,
    };
    if !interp::interpolable(c.op) {
        return None;
    }
    let (x, y) = (*player.get(p)?)?;
    let conv = |v: i32| (v * scale + PLAYER_OFF_SCALE / 2).div_euclid(PLAYER_OFF_SCALE);
    Some(Off { ox: conv(x), oy: conv(y), sox: 0, ocx: 0 })
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
    /// 顯示頁與暫存區的版面。`DrawCmd::Field`（規格 005「戰場大小」）換它，輸出寬是欄數 × 8 像素（HD 再乘 4）。
    geom: Geom,
    /// 這一幀命令套完之後的調色盤，與這一幀開始前的。
    pal: pal::Pal,
    pal_before: pal::Pal,
    /// HD 畫布目前的色彩空間：最近一張有名字的調色盤。換成另一張有名字的調色盤時整塊畫布換色，
    /// 漸變中（DAC 不在清單內，或 `SetDacOne` 改了單一暫存器）畫布不動，輸出時才換。
    hd_space: Option<recolor::Colors>,
    /// 畫 HD 時用的調色盤，對應 `hd_space`。
    hd_pal: pal::Pal,
    /// 上一次輸出換色用的查表。
    out_lut: Option<(recolor::Colors, recolor::Colors, recolor::Lut)>,
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
    /// 這一幀開始前的 HD map mask（重放用）。
    hd_mask_before: u8,
    /// 上一次輸出換下來的緩衝區，下一次重放沿用，不必每次配置 4 MB。
    hd_spare: Vec<u8>,
    cur: FrameData,
    out: Vec<u8>,
    pushed: u64,
    /// BIOS 8x8 字型（2048 位元組）。設了之後幀計數照原版寫進記憶體並永遠畫，不再用輸出時疊上的自繪數字。
    counter_font: Option<Vec<u8>>,
    /// 標題選單選中列在 HD 用圓角高亮條，不取反（規格 005「HD 介面」）。預設關，和原版取反的畫法一致。
    menu_highlight: bool,
    /// 玩家的砲座已脫離機身（規格 005「砲座像 Force」）：這個玩家的 `Pool::Pod` 命令不吃 `RenderOpts::player` 的偏移，
    /// 走一般補間。預設全假，和原版一致。
    free_pods: [bool; 2],
}

#[inline]
fn is_frame_counter(c: &BlitCmd) -> bool {
    c.op == Op::Number && c.text_x == COUNTER_X && c.text_y == COUNTER_Y
}

/// `VramFill` 起點落在暫存區（`1F40h` 起到頁 1 之前、`9F40h` 之後）。
fn fill_in_scratch(off: u16) -> bool {
    (0x1F40..0x8000).contains(&off) || off >= 0x9F40
}

/// 寫進背景暫存區的命令。
fn is_scratch_write(d: &DrawCmd) -> bool {
    match d {
        DrawCmd::Blit(c) => matches!(
            c.op,
            Op::StageToScratch
                | Op::BegToScratch
                | Op::MapToScratch
                | Op::ClearScratchB
                | Op::ClearScratchA
                | Op::ScrollRotateCopy
                | Op::ScrollShiftLeft4
                | Op::ScrollRotateColumn
                | Op::ScrollMergeSeam
                | Op::ScrollShiftBothLeft
        ),
        DrawCmd::VramFill { off, .. } => fill_in_scratch(*off),
        _ => false,
    }
}

/// 從暫存區讀來源的複製命令。
fn is_scratch_copy(d: &DrawCmd) -> bool {
    matches!(
        d,
        DrawCmd::Blit(c) if matches!(
            c.op,
            Op::ScratchReplace
                | Op::ScratchReplaceShifted
                | Op::ScratchOr
                | Op::ScratchAnd
                | Op::ScratchReplaceMirrored
                | Op::ScratchOrPairs
                | Op::ScratchOrMirrored
                | Op::ScratchLatchPairs
                | Op::ScratchAndPairs
                | Op::ScratchAndMirrored
                | Op::ScrollShowA
                | Op::ScrollShowB
        )
    )
}

/// 這一幀畫的頁：清頁命令的頁優先，沒有就取最後一個寫顯示頁的命令。
/// 寫暫存區、畫數字與血條、同時清兩頁的命令不算。`VramFill` 起點在顯示頁內才算。
fn page_of(cmds: &[DrawCmd]) -> Option<u16> {
    let (mut clear, mut any) = (None, None);
    for d in cmds {
        match d {
            DrawCmd::Blit(c) => match c.op {
                Op::ClearDrawPage => clear = Some(c.page),
                Op::StageToScratch
                | Op::BegToScratch
                | Op::MapToScratch
                | Op::ClearScratchA
                | Op::ClearScratchB
                | Op::ClearBothPages
                | Op::Number
                | Op::BarCell
                | Op::ScrollRotateCopy
                | Op::ScrollShiftLeft4
                | Op::ScrollRotateColumn
                | Op::ScrollMergeSeam
                | Op::ScrollShiftBothLeft
                | Op::EndOfFrameCopy => {}
                _ => any = Some(c.page),
            },
            DrawCmd::VramFill { off, .. } if *off < 0x1F40 => any = Some(0),
            DrawCmd::VramFill { off, .. } if (0x8000..0x9F40).contains(off) => any = Some(0x8000),
            _ => {}
        }
    }
    clear.or(any)
}

/// 把暫存區 `which` 換到目前的色彩空間。
fn scratch_to_space(
    m: &mut HdMem,
    which: usize,
    space: Option<recolor::Colors>,
    hd_pal: &pal::Pal,
    before: &mut Option<[Canvas; 2]>,
) {
    let Some(space) = space else { return };
    if let Some(t) = &m.scr_tag[which] {
        if t.colors != space {
            let lut = recolor::Lut::build(&t.colors, &space);
            lut.apply_rgba(&mut m.scratch[which].px);
            if let Some(b) = before {
                lut.apply_rgba(&mut b[which].px);
            }
        }
    }
    m.scr_tag[which] = Some(hd::ScrTag { colors: space, pal: hd_pal.clone() });
}

/// 碰暫存區的命令之前：讀取的命令先把來源換到目前的色彩空間；把圖寫進暫存區的命令回傳要用的調色盤。
fn hd_scratch_prepare(
    m: &mut HdMem,
    h: &HdAssets,
    c: &BlitCmd,
    space: Option<recolor::Colors>,
    hd_pal: &pal::Pal,
    before: &mut Option<[Canvas; 2]>,
) -> Option<pal::Pal> {
    let which = ((c.scratch >> 15) & 1) as usize;
    if let Some(lib) = hd::scratch_image_lib(c) {
        let (n, colors) = h.base_pal(lib, c.index)?;
        // 同一塊暫存區混了不同調色盤的圖時，舊內容轉到新圖的調色盤。新圖保持原樣，通常它才是之後要顯示的背景。
        let same = m.scr_tag[which].as_ref().is_some_and(|t| t.colors == colors);
        if !same {
            if let Some(t) = &m.scr_tag[which] {
                let lut = recolor::Lut::build(&t.colors, &colors);
                lut.apply_rgba(&mut m.scratch[which].px);
            }
            let pal = pal::Pal::from_colors(n, &colors);
            m.scr_tag[which] = Some(hd::ScrTag { colors, pal });
        }
        return m.scr_tag[which].as_ref().map(|t| t.pal.clone());
    }
    match c.op {
        Op::ClearScratchA => m.scr_tag[0] = None,
        Op::ClearScratchB => m.scr_tag[1] = None,
        Op::ScratchReplace | Op::ScratchReplaceShifted | Op::ScratchOr | Op::ScratchAnd | Op::ScratchReplaceMirrored | Op::ScratchOrPairs | Op::ScratchOrMirrored | Op::ScratchLatchPairs | Op::ScratchAndPairs | Op::ScratchAndMirrored => {
            scratch_to_space(m, which, space, hd_pal, before);
        }
        Op::ScrollRotateCopy | Op::ScrollShiftLeft4 | Op::ScrollRotateColumn | Op::ScrollMergeSeam | Op::ScrollShowA | Op::ScrollShowB | Op::ScrollShiftBothLeft => {
            scratch_to_space(m, 0, space, hd_pal, before);
            scratch_to_space(m, 1, space, hd_pal, before);
        }
        _ => {}
    }
    None
}

impl LayerSink {
    pub fn new(assets: Assets, hd: Option<HdAssets>) -> LayerSink {
        let g = Geom::ORIGINAL;
        let hdm = hd.as_ref().map(|_| HdMem::new(g));
        let hd_base_page = hdm.as_ref().map(|m| m.pages[0].clone()).unwrap_or_else(Canvas::empty);
        LayerSink {
            assets,
            hd,
            geom: g,
            pal: pal::Pal::new(),
            pal_before: pal::Pal::new(),
            hd_space: None,
            hd_pal: pal::Pal::new(),
            out_lut: None,
            idx: idx::Mem::new(g),
            idx_base: idx::Mem::new(g),
            idx_tmp: idx::Mem::new(g),
            idx_st: idx::St::default(),
            idx_st_before: idx::St::default(),
            hdm,
            hd_base_page,
            hd_scratch_before: None,
            hd_mask_before: 0x0F,
            hd_spare: Vec::new(),
            cur: FrameData::empty(),
            out: Vec::new(),
            pushed: 0,
            counter_font: None,
            menu_highlight: false,
            free_pods: [false; 2],
        }
    }

    /// 用原版某個時點的顯示記憶體與 DAC 當起點，對應 `VgaSink::load_snapshot`（對拍用）。四個平面各 64 KB
    /// （`planes[p]` 是 VGA 平面 `p`），DAC 是 256 色 x 3 的 6 位元值（`rs2dyn` 的 `dac256_hex`）。
    ///
    /// - 索引色記憶體依平面合成；序列器 map mask 回到 0Fh、讀取平面 0、資料旋轉 0（原版每個繪製函式結束時的狀態）。
    /// - 調色盤名稱依 DAC 0 至 15 的指紋辨識（`pal::identify`），不在已知清單內時為 `None`。
    /// - 補間的前一幀記錄清空，畫頁回到 0；已推入幀數、`word_2687E` 殘值（`set_stride_reg`）與異常計數不變。
    /// - HD 模式：兩個顯示頁用原版圖最近鄰放大帶入，暫存區留空。
    ///
    /// # Panics
    /// 任一平面不是 64 KB。
    pub fn load_snapshot(&mut self, planes: [&[u8]; 4], dac: &[u8; 768]) {
        self.set_field(crate::geom::ORIGINAL_COLS);
        self.idx.load_planes(planes);
        self.idx_base.px.copy_from_slice(&self.idx.px);
        let stride = self.idx_st.stride_reg;
        let anomalies = self.idx_st.anomalies;
        self.idx_st = idx::St { stride_reg: stride, anomalies, ..idx::St::default() };
        self.idx_st_before = self.idx_st;
        let mut pal = pal::Pal::new();
        for i in 0..256 {
            pal.dac[i] = [dac[3 * i] & 0x3F, dac[3 * i + 1] & 0x3F, dac[3 * i + 2] & 0x3F];
        }
        let first16: [u8; 48] = std::array::from_fn(|i| dac[i]);
        pal.name = pal::identify(&first16);
        self.pal = pal.clone();
        self.pal_before = pal;
        self.cur = FrameData::empty();
        self.hd_space = None;
        if self.pal.name.is_some() {
            self.hd_space = Some(self.pal.visible8());
            self.hd_pal = self.pal.clone();
        }
        if let Some(m) = &mut self.hdm {
            *m = HdMem::new(self.geom);
            hd::load_pages(m, &self.idx.px, &self.pal.visible8());
            self.hd_base_page = m.pages[0].clone();
            self.hd_scratch_before = None;
            self.hd_mask_before = 0x0F;
        }
    }

    /// 設 `word_2687E`（最近一次載入的圖庫項每列位元組數），同 `VgaSink::set_stride_reg`。
    pub fn set_stride_reg(&mut self, v: u16) {
        self.idx_st.stride_reg = v;
        self.idx_st_before.stride_reg = v;
    }

    /// `word_2687E` 目前的值（診斷用）。
    pub fn stride_reg(&self) -> u16 {
        self.idx_st.stride_reg
    }

    /// 套用一個邏輯幀的命令。`Game::new` 後 `take_draws()` 的初始化命令也走這裡。
    /// 空的命令串（暫停中的步）不算一幀，直接忽略，畫面維持上一幀。
    pub fn push_frame(&mut self, draws: &[DrawCmd]) {
        if draws.is_empty() {
            return;
        }
        // `DrawCmd::Field`（戰場大小）換版面。每一幀只用一種版面，所以命令串在 Field 之前切開，之前的部分用舊版面套完，
        // 之後的部分第一個命令就是 Field。
        if let Some(i) = draws.iter().skip(1).position(|d| matches!(d, DrawCmd::Field { .. })) {
            self.push_frame(&draws[..i + 1]);
            self.push_frame(&draws[i + 1..]);
            return;
        }
        if let Some(DrawCmd::Field { cols }) = draws.first() {
            self.set_field(*cols);
        }
        let page = page_of(draws).unwrap_or(self.cur.page);
        let deltas = interp::pair(&self.cur.cmds, draws);
        let first_scratch = draws.iter().position(is_scratch_write);
        let scratch_ops = first_scratch.is_some();
        let scratch_borrow_ok = first_scratch.is_none_or(|i| !draws[i..].iter().any(is_scratch_copy));

        // 這一幀開始前的快照。
        self.idx_base.px.copy_from_slice(&self.idx.px);
        self.idx_st_before = self.idx_st;
        self.pal_before = self.pal.clone();
        if let Some(m) = &self.hdm {
            let p = ((page >> 15) & 1) as usize;
            self.hd_base_page = m.pages[p].clone();
            self.hd_scratch_before = scratch_ops.then(|| m.scratch.clone());
            self.hd_mask_before = m.map_mask;
        }

        let mut counter = None;
        for d in draws {
            match d {
                DrawCmd::SetDac { start, rgb } => {
                    self.pal.set(*start, rgb);
                    if *start == 0 {
                        self.sync_hd_space(rgb);
                    }
                }
                DrawCmd::SetDacOne { index, rgb } => self.pal.set_one(*index, rgb),
                // EGA 屬性暫存器：LayerSink 只畫 VGA 的 DAC 色，EGA 由呼叫端依 `EgaAttr` 另外處理。
                DrawCmd::EgaAttr { .. } => {}
                DrawCmd::SetMapMask { mask } => {
                    idx::set_map_mask(&mut self.idx_st, *mask);
                    if let Some(m) = &mut self.hdm {
                        hd::set_map_mask(m, *mask);
                    }
                }
                // 版面已在上面換好（Field 一定是這一幀的第一個命令）。
                DrawCmd::Field { .. } => {}
                DrawCmd::VramFill { off, words, value } => {
                    idx::vram_fill(&mut self.idx, &self.idx_st, *off, *words, *value);
                    if let Some(m) = &mut self.hdm {
                        let p = if self.hd_space.is_some() { &self.hd_pal } else { &self.pal };
                        hd::vram_fill(m, &p.visible8(), *off, *words, *value);
                    }
                }
                DrawCmd::Blit(c) => {
                    if is_frame_counter(c) && self.counter_font.is_none() {
                        counter = Some(c.cx);
                        // 不畫，但命令帶的 `word_2687E` 還是要記下：核心把邏輯對它的寫入附在下一個繪製命令上，
                        // `VgaSink::blit` 在分派前先套用（`sink.rs`），之後 `y < 0` 的列複製（`ScratchReplaceMirrored` 等）靠它推進來源指標。
                        if let Some(v) = c.stride_reg {
                            self.idx_st.stride_reg = v;
                        }
                        continue;
                    }
                    idx::apply(&mut self.idx, &mut self.idx_st, &self.assets, self.counter_font.as_deref(), c, Off::default());
                    if let (Some(m), Some(h)) = (&mut self.hdm, &mut self.hd) {
                        let scratch_pal = hd_scratch_prepare(m, h, c, self.hd_space, &self.hd_pal, &mut self.hd_scratch_before);
                        let p = match &scratch_pal {
                            Some(t) => t,
                            None if self.hd_space.is_some() => &self.hd_pal,
                            None => &self.pal,
                        };
                        if self.menu_highlight && c.op == Op::MyOverwriteInverted {
                            hd::menu_highlight(m, h, &self.assets, p, c);
                        } else {
                            hd::apply(m, h, &self.assets, p, c, Off::default());
                        }
                    }
                }
            }
        }
        let any_delta = deltas.iter().any(|d| d.is_some());
        self.cur = FrameData { cmds: draws.to_vec(), deltas, page, any_delta, scratch_borrow_ok, counter };
        self.pushed += 1;
    }

    /// 換顯示頁與暫存區的版面（`DrawCmd::Field`，規格 005「戰場大小」）：記憶體清成黑，補間的前一幀記錄清空，
    /// 調色盤與診斷計數不動。欄數沒變是空動作。
    fn set_field(&mut self, cols: u16) {
        let g = Geom::new(cols);
        if g == self.geom {
            return;
        }
        self.geom = g;
        self.idx = idx::Mem::new(g);
        self.idx_base = idx::Mem::new(g);
        self.idx_tmp = idx::Mem::new(g);
        self.idx_st = idx::St { stride_reg: self.idx_st.stride_reg, anomalies: self.idx_st.anomalies, ..idx::St::default() };
        self.idx_st_before = self.idx_st;
        if let Some(m) = &mut self.hdm {
            *m = HdMem::new(g);
            self.hd_base_page = m.pages[0].clone();
        }
        self.hd_scratch_before = None;
        self.hd_mask_before = 0x0F;
        self.hd_spare = Vec::new();
        self.out = Vec::new();
        self.cur = FrameData::empty();
    }

    /// 目前的戰場欄數（顯示頁每列的位元組數）：40 是原版，戰場加大時是 48 或 60。
    pub fn field_cols(&self) -> u16 {
        self.geom.cols()
    }

    /// 新的整張調色盤。有名字的話把 HD 畫布整塊換色，原版改 DAC 也是連已畫的像素一起變。
    fn sync_hd_space(&mut self, rgb: &[u8; 48]) {
        if self.hdm.is_none() || self.pal.name.is_none() {
            return;
        }
        let want = pal::colors_of(rgb);
        if let Some(old) = self.hd_space {
            if old != want {
                let lut = recolor::Lut::build(&old, &want);
                // 暫存區等到被複製時才換色（`hd_scratch_prepare`）。
                if let Some(m) = &mut self.hdm {
                    for c in m.pages.iter_mut() {
                        lut.apply_rgba(&mut c.px);
                    }
                }
                lut.apply_rgba(&mut self.hd_base_page.px);
            }
        }
        self.hd_space = Some(want);
        self.hd_pal = pal::Pal::from_table(rgb);
    }

    /// 輸出 RGBA。`alpha` 是前一幀到目前這幀的進度，0.0 到 1.0。
    /// 原版圖 320×200，HD 1280×800。沒有載入 HD 資料夾時，`hd: true` 輸出原版圖的最近鄰 4 倍。
    /// 戰場加大時（規格 005）寬變成欄數 × 8 像素（120% 384、150% 480），HD 再乘 4，高不變。
    pub fn render(&mut self, alpha: f32, opts: RenderOpts) -> (&[u8], usize, usize) {
        let t = if alpha.is_nan() { 1.0 } else { alpha.clamp(0.0, 1.0) };
        // 玩家機有預測偏移時，就算這一幀沒有別的東西動也要重放。
        let predicted = opts.player.iter().flatten().any(|&o| o != (0, 0));
        let replay = (opts.interpolate && self.cur.any_delta) || predicted;
        let sd_w = self.geom.px_width();
        if opts.hd && self.hdm.is_some() {
            self.render_hd(t, replay, opts);
            self.hd_output_recolor();
            return (&self.out, sd_w * hd::S as usize, hd::PH as usize);
        }
        // 原版圖：重放或直接用套完的記憶體。
        if replay {
            self.idx_tmp.px.copy_from_slice(&self.idx_base.px);
            let mut st = self.idx_st_before;
            for (i, d) in self.cur.cmds.iter().enumerate() {
                match d {
                    DrawCmd::Blit(c) => {
                        if is_frame_counter(c) && self.counter_font.is_none() {
                            if let Some(v) = c.stride_reg {
                                st.stride_reg = v;
                            }
                            continue;
                        }
                        let off = player_off(&opts.player, &self.free_pods, c, 1).unwrap_or_else(|| {
                            let dl = self.cur.deltas[i].filter(|_| opts.interpolate);
                            dl.map(|dl| interp::offsets(&dl, t, 1)).unwrap_or_default()
                        });
                        idx::apply(&mut self.idx_tmp, &mut st, &self.assets, self.counter_font.as_deref(), c, off);
                    }
                    DrawCmd::VramFill { off, words, value } => idx::vram_fill(&mut self.idx_tmp, &st, *off, *words, *value),
                    DrawCmd::SetMapMask { mask } => idx::set_map_mask(&mut st, *mask),
                    DrawCmd::SetDac { .. } | DrawCmd::SetDacOne { .. } | DrawCmd::EgaAttr { .. } | DrawCmd::Field { .. } => {}
                }
            }
        } else {
            self.idx_tmp.px.copy_from_slice(&self.idx.px);
        }
        if opts.show_counter {
            if let Some(v) = self.cur.counter {
                idx::counter(&mut self.idx_tmp, COUNTER_X, COUNTER_Y, v, None);
            }
        }
        let colors = self.pal.visible8();
        if opts.hd {
            // 沒有 HD 資料夾：原版圖最近鄰放大。
            let mut small = vec![0u8; sd_w * 200 * 4];
            idx::page_to_rgba(&self.idx_tmp, self.cur.page, &colors, &mut small);
            let hd_w = sd_w * 4;
            self.out.resize(hd_w * 800 * 4, 0);
            for y in 0..800usize {
                for x in 0..hd_w {
                    let s = ((y / 4) * sd_w + x / 4) * 4;
                    let o = (y * hd_w + x) * 4;
                    self.out[o..o + 4].copy_from_slice(&small[s..s + 4]);
                }
            }
            return (&self.out, hd_w, 800);
        }
        self.out.resize(sd_w * 200 * 4, 0);
        idx::page_to_rgba(&self.idx_tmp, self.cur.page, &colors, &mut self.out);
        (&self.out, sd_w, 200)
    }

    /// 漸變中目前的 DAC 和畫布的色彩空間不同，輸出時換色。
    fn hd_output_recolor(&mut self) {
        let Some(space) = self.hd_space else { return };
        let now = self.pal.visible8();
        // 原版循環少數幾個色號做火焰閃爍，只影響用那個色號畫的像素。HD 圖沒有色號，換色會染到相近的顏色，
        // 所以少數色號的變化不換，只有整體淡入淡出才換。
        if recolor::changed(&space, &now) < 6 {
            return;
        }
        if !matches!(&self.out_lut, Some((a, b, _)) if *a == space && *b == now) {
            self.out_lut = Some((space, now, recolor::Lut::build(&space, &now)));
        }
        if let Some((_, _, lut)) = &self.out_lut {
            lut.apply_rgba(&mut self.out);
        }
    }

    fn render_hd(&mut self, t: f32, replay: bool, opts: RenderOpts) {
        let show_counter = opts.show_counter;
        let menu_highlight = self.menu_highlight;
        let free_pods = self.free_pods;
        let Self { hd_space, hd_pal, hdm, hd: hd_assets, assets, pal, pal_before, hd_base_page, hd_scratch_before, hd_mask_before, hd_spare, cur, out, counter_font, .. } = self;
        let (Some(mem), Some(hda)) = (hdm.as_mut(), hd_assets.as_mut()) else { return };
        let pg = ((cur.page >> 15) & 1) as usize;
        // 目的緩衝區沿用上一次換下來的 Vec。
        let mut buf = std::mem::take(hd_spare);
        buf.clear();
        buf.extend_from_slice(if replay { &hd_base_page.px } else { &mem.pages[pg].px });
        let mut page = Canvas::from_px(buf, mem.pages[pg].w);
        if replay {
            let mut tmp = HdMem::from_parts(mem.g, [Canvas::empty(), Canvas::empty()], [Canvas::empty(), Canvas::empty()]);
            tmp.map_mask = *hd_mask_before;
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
            // 畫布已經換到 `hd_pal` 的色彩空間（`sync_hd_space` 連「開始前」的快照一起換），重放不用再跟著 DAC 變。
            let mut p = if hd_space.is_some() { hd_pal.clone() } else { pal_before.clone() };
            for (i, d) in cur.cmds.iter().enumerate() {
                if skip_scratch_ops && is_scratch_write(d) {
                    continue;
                }
                match d {
                    DrawCmd::SetDac { start, rgb } => {
                        if hd_space.is_none() {
                            p.set(*start, rgb);
                        }
                    }
                    DrawCmd::SetDacOne { index, rgb } => {
                        if hd_space.is_none() {
                            p.set_one(*index, rgb);
                        }
                    }
                    DrawCmd::EgaAttr { .. } | DrawCmd::Field { .. } => {}
                    DrawCmd::SetMapMask { mask } => hd::set_map_mask(&mut tmp, *mask),
                    DrawCmd::VramFill { off, words, value } => hd::vram_fill(&mut tmp, &p.visible8(), *off, *words, *value),
                    DrawCmd::Blit(c) => {
                        if is_frame_counter(c) && counter_font.is_none() {
                            continue;
                        }
                        let off = player_off(&opts.player, &free_pods, c, hd::S).unwrap_or_else(|| {
                            let dl = cur.deltas[i].filter(|_| opts.interpolate);
                            dl.map(|dl| interp::offsets(&dl, t, hd::S)).unwrap_or_default()
                        });
                        if menu_highlight && c.op == Op::MyOverwriteInverted {
                            hd::menu_highlight(&mut tmp, hda, assets, &p, c);
                        } else {
                            hd::apply(&mut tmp, hda, assets, &p, c, off);
                        }
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
                let c = if hd_space.is_some() { hd_pal.visible8() } else { pal.visible8() };
                hd::counter(&mut pages, &c, COUNTER_X, COUNTER_Y, v, false);
                let [p0, _] = pages;
                page = p0;
            }
        }
        *hd_spare = std::mem::replace(out, page.px);
    }

    /// 標題選單選中列在 HD 畫面用圓角高亮條代替取反（規格 005「HD 介面」）。預設關。
    /// 只影響 HD 引擎之後套用的 `MyOverwriteInverted`；原版圖引擎與 `VgaSink` 不受影響，補間關閉時逐像素等於原版。
    pub fn set_menu_highlight(&mut self, on: bool) {
        self.menu_highlight = on;
    }

    /// 玩家（0 或 1）的砲座脫離機身時，那位玩家的砲座不套用 `RenderOpts::player` 的預測偏移（它不跟著機身動）。
    /// 機身與尾焰照舊。預設全假。
    pub fn set_free_pods(&mut self, free: [bool; 2]) {
        self.free_pods = free;
    }

    /// 這一幀的幀計數值（座標 (12h, BFh) 的 `Number`）。這一幀沒有畫幀計數、或已設 `set_counter_font` 時是 `None`。
    /// HD HUD 用它自己畫幀計數。
    pub fn counter_value(&self) -> Option<u16> {
        self.cur.counter
    }

    /// `MY` 圖庫第 `index` 項的 HD 圖（RGBA，寬，高），依目前的調色盤選變體。沒有 HD 資料夾或項號不存在時是 `None`。
    /// 資料夾裡缺這張圖時，退回原版圖的放大。HUD 的武器圖示用它。
    pub fn my_image(&mut self, index: u16) -> Option<(Vec<u8>, usize, usize)> {
        let pal = if self.hd_space.is_some() { self.hd_pal.clone() } else { self.pal.clone() };
        let h = self.hd.as_mut()?;
        let img = h.image(HdLib::My, index, &pal, &self.assets)?;
        Some((img.rgba.clone(), img.w, img.h))
    }

    /// 預載第一關會用到的 HD 圖（`MY`、`SHT1`、`BH1` 的圖、遮罩與 `stage1_boss` 變體），避免第一次出現時在 `push_frame` 內解碼。
    /// 沒有 HD 資料夾時回傳 `None`。內容見 `HdAssets::preload`。
    pub fn preload_hd(&mut self, threads: usize) -> Option<(usize, usize)> {
        let Self { hd, assets, .. } = self;
        hd.as_mut().map(|h| h.preload(assets, threads))
    }

    /// 依場景預載 HD 圖（標題、各關、過場、片尾），內容見 `HdSet`。可以在換場景前對下一個場景呼叫，
    /// 已在快取裡的不重做。沒有 HD 資料夾時回傳 `None`。
    pub fn preload_hd_set(&mut self, set: HdSet, threads: usize) -> Option<(usize, usize)> {
        let Self { hd, assets, .. } = self;
        hd.as_mut().map(|h| h.preload_set(assets, set, threads))
    }

    /// 成功載入的 HD 檔（相對路徑，依載入順序），診斷用。
    pub fn loaded_hd(&self) -> &[String] {
        self.hd.as_ref().map(|h| h.loaded()).unwrap_or(&[])
    }

    /// 缺的 HD 圖，給啟動日誌與畫面標記。沒有載入 HD 資料夾時是空的。
    pub fn missing_hd(&self) -> &[String] {
        self.hd.as_ref().map(|h| h.missing()).unwrap_or(&[])
    }

    /// BIOS 8x8 字型（2048 位元組，`F000:FA6Eh` 起）。設了之後幀計數照 `sub_1890D`、`sub_18A61` 寫進記憶體並永遠畫
    /// （索引色引擎；`RenderOpts::show_counter` 不再控制），行為同 `VgaSink::set_counter_font`，對拍時畫面才逐像素相同。
    /// HD 模式的幀計數仍用自繪數字。
    ///
    /// # Panics
    /// 長度不是 2048。
    pub fn set_counter_font(&mut self, font: &[u8]) {
        assert_eq!(font.len(), 2048, "BIOS 字型應為 2048 位元組（256 字元 x 8 列）");
        self.counter_font = Some(font.to_vec());
    }

    /// 診斷：索引色引擎目前的記憶體（不含補間重放）。每個像素一個位元組（色號 0 至 15），
    /// 位元組位址 `a` 的第 `b` 個位元（最高位在左）是 `[a * 8 + b]`，共 `65536 * 8` 個。
    /// 與 `VgaSink` 的四個平面逐位元對得上，差分測試用它比對整個位址空間（含暫存區與沒顯示的頁）。
    pub fn index_memory(&self) -> &[u8] {
        &self.idx.px
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

#[cfg(test)]
mod tests {
    use super::*;
    use rs2_game::{BegGroup, ObjId, Pool, ShtGroup};

    fn cmd(pool: Pool, slot: u8) -> BlitCmd {
        BlitCmd {
            op: Op::MyCut,
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
            sht_group: ShtGroup::Bh1,
            mask_group: ShtGroup::Sht1,
            beg_group: BegGroup::Beg1,
            stride_reg: None,
            obj: Some(ObjId { pool, slot, generation: 0 }),
        }
    }

    #[test]
    fn index_memory_and_page_conversion_follow_the_field_geometry() {
        use super::idx::{Mem, page_to_rgba};
        let mut colors = [[0u8; 3]; 16];
        colors[3] = [1, 2, 3];
        colors[5] = [9, 8, 7];
        for cols in [40u16, 48, 60] {
            let g = Geom::new(cols);
            let mut m = Mem::new(g);
            assert_eq!(m.px.len(), g.mem_bytes() * 8, "{cols} 欄：位址空間 × 8 像素");
            // 第一組的頁 0：左上角與右下角（第 199 列、最後一欄的最後一個像素）。
            let last = (199 * usize::from(cols) + usize::from(cols) - 1) * 8 + 7;
            m.px[0] = 3;
            m.px[last] = 5;
            let mut out = vec![0u8; g.px_width() * 200 * 4];
            page_to_rgba(&m, 0, &colors, &mut out);
            assert_eq!(out.len(), usize::from(cols) * 8 * 200 * 4);
            assert_eq!(&out[..4], &[1, 2, 3, 255]);
            assert_eq!(&out[out.len() - 4..], &[9, 8, 7, 255], "{cols} 欄：最後一個像素在右下角");
            // 頁 1 在第二組：命令的頁值 8000h 換成這個版面的組位移，頁 0 的內容不跟著出現。
            m.px[g.set1() as usize * 8] = 3;
            out.fill(0);
            page_to_rgba(&m, 0x8000, &colors, &mut out);
            assert_eq!(&out[..4], &[1, 2, 3, 255]);
            assert_eq!(&out[out.len() - 4..], &[0, 0, 0, 255], "{cols} 欄：頁 1 的右下角是黑");
        }
    }

    #[test]
    fn a_detached_pod_does_not_take_the_ship_prediction_offset() {
        let player = [Some((8, -4)), Some((12, 0))];
        let attached = [false; 2];
        // 附著的砲座（槽 0、1 屬玩家 1，槽 2、3 屬玩家 2）和機身一樣吃偏移。
        for (pool, slot, want) in [(Pool::Player, 0, 2), (Pool::Pod, 0, 2), (Pool::Pod, 1, 2), (Pool::Pod, 2, 3), (Pool::Pod, 3, 3), (Pool::Player, 1, 3)] {
            let off = player_off(&player, &attached, &cmd(pool, slot), 1).expect("有偏移");
            assert_eq!(off.ox, want, "{pool:?} 槽 {slot}");
        }
        // 玩家 1 的砲座分離：只有玩家 1 的砲座槽（0、1）不吃偏移，機身與玩家 2 的砲座照舊。
        let free = [true, false];
        assert!(player_off(&player, &free, &cmd(Pool::Pod, 0), 1).is_none());
        assert!(player_off(&player, &free, &cmd(Pool::Pod, 1), 1).is_none());
        assert!(player_off(&player, &free, &cmd(Pool::Player, 0), 1).is_some(), "機身照舊");
        assert!(player_off(&player, &free, &cmd(Pool::Pod, 2), 1).is_some(), "玩家 2 的砲座照舊");
        // 玩家 2 的砲座分離。
        let free = [false, true];
        assert!(player_off(&player, &free, &cmd(Pool::Pod, 2), 1).is_none());
        assert!(player_off(&player, &free, &cmd(Pool::Pod, 3), 1).is_none());
        assert!(player_off(&player, &free, &cmd(Pool::Pod, 0), 1).is_some());
        // 沒有預測偏移的玩家本來就回傳 None，其他池也是。
        assert!(player_off(&[None, None], &attached, &cmd(Pool::Pod, 0), 1).is_none());
        assert!(player_off(&player, &attached, &cmd(Pool::Entity, 0), 1).is_none());
    }
}
