//! 繪製介面。
//!
//! 原版的繪製常式（`sub_18C0A` 一族）從資料段讀位置、來源、頁等參數。邏輯只改寫要變的幾個，沒寫的沿用上一次的值，
//! 所以這些參數是狀態（[`DrawParams`]），掛在 `Core::draw_params`，經 `Mirror` 對應回資料段。每個繪製動作把當下的
//! 參數連同動作種類記成一個繪製命令（[`BlitCmd`]），真正的 blit 在 `rs2-gfx`。
//!
//! 動作種類（[`Op`]）沿用原版位址命名，`rs2-gfx` 與 `rs2-trace` 依它分派，不能改。本檔每個動作有自己的名字：
//!
//! | `Op` | 名字 |
//! |---|---|
//! | `S18D46`、`S18E30`、`S18E73` | [`Core::cut_my`]（遮罩 AND），用 [`MyCopy`] 選副本 |
//! | `S18CE2`、`S18DCC`、`S18EB6` | [`Core::overlay_my`]（四平面 OR），用 [`MyCopy`] 選副本 |
//! | `S18D89` | [`Core::flash_my`] |
//! | `S18C0A` | [`Core::overwrite_my`] |
//! | `S18C6E` | [`Core::overwrite_my_inverted`] |
//! | `S18F1A`、`S19025`、`S18F5D`、`S18FC1` | [`Core::cut_stage`]、[`Core::flash_stage`]、[`Core::overlay_stage`]、[`Core::overwrite_stage`] |
//! | `S19068` | [`Core::overwrite_beg`] |
//! | `S190CC` | 沒有。唯一呼叫端 `sub_12438` 的 `1243Ch` 起是死碼（`docs/re/12` 第 661 行），前一個領域留下的 `s190cc` 已刪 |
//! | `S19ADC`、`S19B40`、`S18ACE` | [`Core::draw_current_tile_to_scratch`]、[`Core::draw_current_beg_to_scratch`]、[`Core::draw_map_tile_to_scratch`] |
//! | `S19BA4`、`S19C4F`、`S19D08`、`S19DA8`、`S19F3B`、`S1A09F`、`S1A1E6`、`S19E66`、`S19FF9`、`S1A144` | [`Core::copy_scratch_rows`]，用 [`RowCopy`] 選種類 |
//! | `S1A287`、`S1929F`、`S1890D` | [`Core::fill_rows`]、[`Core::draw_bar_cell`]、[`Core::draw_number`] |
//! | `S19CF9`、`S11504`、`S1152D`、`S18894` | [`Core::clear_draw_page`]、[`Core::clear_scratch_a`]、[`Core::clear_scratch_b`]、[`Core::clear_both_pages`] |
//! | `S1117F` | [`Core::end_of_frame_copy`] |
//! | `S17EBA`、`S17EEA`、`S17F26`、`S17F5D`、`S17FA0`、`S18031`、`S180C6` | [`Core::scroll_op`]，用 [`ScrollOp`] 選種類 |
//!
//! 繪製命令的 `cx`、`dx` 兩個欄位原版是暫存器，不屬於繪製參數。接收端（`rs2-gfx` 的 `sink.rs`、`mono.rs`、
//! `mono_hd.rs`、`layer.rs`、`layer/idx.rs`、`layer/hd.rs`、`layer/interp.rs`）只在下列動作讀它們，
//! 這些動作的呼叫在命令送出前一刻帶進來；其餘動作不讀，命令帶 0。
//!
//! | 動作 | `cx` | `dx` |
//! |---|---|---|
//! | 暫存區列複製（`copy_scratch_rows` 的十種） | 列數 | 不讀 |
//! | `S1A287`（`fill_rows`） | 列數 | 色號 |
//! | `S1929F`（`draw_bar_cell`） | 不讀 | 色號 |
//! | `S1890D`（`draw_number`） | 數值 | 不讀 |
//!
//! `S1929F` 的接收端（`sink.rs`、`layer/idx.rs`）把 `cx` 傳進列起點的函式，但回傳的寫入位址在用到 `cx` 之前就算好，
//! `cx` 沒有影響結果。

use crate::draw::{BlitCmd, DrawCmd, Op};
use crate::game::Core;

/// 暫存區 A 的選擇值（`word_2685C`）。
pub(crate) const SCRATCH_A: u16 = 0;
/// 暫存區 B 的選擇值。
pub(crate) const SCRATCH_B: u16 = 0x8000;

/// 繪製參數：下一個繪製命令帶走的狀態。欄位照原版字組的寬度，`Mirror` 的 `load` 與 `store` 互為反函數。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DrawParams {
    /// `word_26880`：目的欄，單位是位元組（8 像素）。
    pub x: u16,
    /// `word_26882`：目的像素列。
    pub y: u16,
    /// `word_26888`：圖庫索引。
    pub index: u16,
    /// `word_26872`：繪製頁的偏移，0 或 8000h。
    pub page: u16,
    /// `word_2685C`：暫存區選擇，[`SCRATCH_A`] 或 [`SCRATCH_B`]。
    pub scratch: u16,
    /// `word_26864`、`word_26866`：暫存區複製的來源欄與來源列。
    pub source_col: u16,
    pub source_row: u16,
    /// `word_26868`：暫存區複製的水平位移。
    pub shift: u16,
    /// `word_2688A`、`word_2688E`：數字的欄與列。
    pub text_col: u16,
    pub text_row: u16,
    /// `word_2687E`：形狀的寬，邏輯（敵人足跡、敵彈、特效、玩家彈、玩家接觸）寫進去。
    /// 繪製常式同一個字當每列位元組數用，載入圖庫項時由接收端改寫，核心不會更新。
    /// `Some` 是上一個繪製命令之後邏輯寫過的值，`None` 是沒寫過。下一個繪製命令把它記成 `stride_reg`，
    /// 之後回到 `None`。資料段上的對應是 FFFFh（見 `mirror.rs`）。
    pub shape_width: Option<u16>,
}

/// 雙緩衝的兩頁。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    /// 顯示記憶體偏移 0。
    Zero,
    /// 顯示記憶體偏移 8000h。
    One,
}

impl Page {
    /// 頁在顯示記憶體裡的偏移。
    pub(crate) const fn offset(self) -> u16 {
        match self {
            Page::Zero => 0,
            Page::One => 0x8000,
        }
    }

    /// 另一頁。
    pub(crate) const fn other(self) -> Page {
        match self {
            Page::Zero => Page::One,
            Page::One => Page::Zero,
        }
    }
}

/// `MY.GO` 的遮罩與四平面，原版各有三份相同的常式，各領域用自己那份。接收端當同一種畫法，
/// 但命令的 `Op` 不同，對拍比的是 `Op`，所以呼叫端要選對副本。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MyCopy {
    /// 玩家、砲座、發射閃光、尾焰、撿取光柱、`MY` 圖庫的敵彈、過場字樣。遮罩 `18D46h`，平面 `18CE2h`。
    General,
    /// 特效。遮罩 `18E30h`，平面 `18DCCh`。平面這份在 E 模式另外驅動 PC 喇叭（`docs/re/13` 第 4.3 節）。
    Effect,
    /// 玩家彈。遮罩 `18E73h`，平面 `18EB6h`。
    PlayerShot,
}

/// 暫存區複製到繪製頁的種類，`rows` 是列數（見 [`Core::copy_scratch_rows`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowCopy {
    /// 整列覆寫（`sub_19BA4`）。
    Replace,
    /// 整列覆寫，帶水平位移（`sub_19D08`）。
    ReplaceShifted,
    /// 整列覆寫，列號遞減，垂直鏡射（`sub_19C4F`）。
    ReplaceMirrored,
    /// 整列 OR（`sub_19DA8`）。
    Or,
    /// 整列 OR，垂直鏡射（`sub_19F3B`）。
    OrMirrored,
    /// 整列 AND（`sub_1A09F`）。
    And,
    /// 整列 AND，垂直鏡射（`sub_1A1E6`）。
    AndMirrored,
    /// 閂鎖複製，每列 2 位元組（`sub_19FF9`）。
    LatchPairs,
    /// AND，每列 2 位元組（`sub_1A144`）。
    AndPairs,
    /// 逐平面 OR，每列 2 位元組（`sub_19E66`）。
    OrPairs,
}

/// 第三關地圖捲動的顯示記憶體命令。這些命令只讀目的欄（`word_26880`）、頁與暫存區選擇，不讀列數與色號。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScrollOp {
    /// 暫存區 A 的前 3E80h 位元組轉 4 位元複製到 B（`sub_17EBA`）。
    RotateCopy,
    /// 暫存區 B 就地左移 4 像素（`sub_17EEA`）。
    ShiftLeft4,
    /// 暫存區 A 的一欄（200 列）轉 4 位元複製到 B（`sub_17F26`）。
    RotateColumn,
    /// 暫存區 B 的接縫欄取下一欄的低半（`sub_17F5D`）。
    MergeSeam,
    /// 暫存區 A 的 40 欄 × 190 列複製到繪製頁（`sub_17FA0`），暫存區選擇要是 [`SCRATCH_A`]。
    ShowA,
    /// 暫存區 B 的同一份顯示（`sub_18031`），暫存區選擇要是 [`SCRATCH_B`]。
    ShowB,
    /// 暫存區 A、B 各整體左移 28h 位元組（`sub_180C6`）。
    ShiftBothLeft,
}

impl Core {
    // ---- 繪製參數 ----

    pub(crate) fn set_blit_x(&mut self, x: u16) {
        self.draw_params.x = x;
    }

    pub(crate) fn set_blit_y(&mut self, y: u16) {
        self.draw_params.y = y;
    }

    pub(crate) fn set_blit_index(&mut self, index: u16) {
        self.draw_params.index = index;
    }

    /// 暫存區複製的來源欄與來源列。
    pub(crate) fn set_blit_source(&mut self, col: u16, row: u16) {
        self.draw_params.source_col = col;
        self.draw_params.source_row = row;
    }

    pub(crate) fn set_blit_shift(&mut self, shift: u16) {
        self.draw_params.shift = shift;
    }

    /// 目的欄加 `n`（原版 `add word_26880, n`，迴圈裡逐份往右畫）。
    pub(crate) fn add_blit_x(&mut self, n: u16) {
        self.draw_params.x = self.draw_params.x.wrapping_add(n);
    }

    pub(crate) fn add_blit_y(&mut self, n: u16) {
        self.draw_params.y = self.draw_params.y.wrapping_add(n);
    }

    pub(crate) fn add_blit_index(&mut self, n: u16) {
        self.draw_params.index = self.draw_params.index.wrapping_add(n);
    }

    /// 欄 `x`、像素列 `y`、圖庫索引 `index`。下一個繪製命令帶走它們。
    pub(crate) fn set_blit_params(&mut self, x: u16, y: u16, index: u16) {
        self.draw_params.index = index;
        self.draw_params.x = x;
        self.draw_params.y = y;
    }

    /// 欄與像素列，不動圖庫索引。暫存區整頁複製用它當目的位置。
    pub(crate) fn set_blit_position(&mut self, x: u16, y: u16) {
        self.draw_params.x = x;
        self.draw_params.y = y;
    }

    /// 暫存區複製的來源欄、來源列、水平位移。
    pub(crate) fn set_blit_source_shifted(&mut self, source_col: u16, source_row: u16, shift: u16) {
        self.draw_params.source_col = source_col;
        self.draw_params.source_row = source_row;
        self.draw_params.shift = shift;
    }

    /// 數字（[`Core::draw_number`]）的位置：欄與列。
    pub(crate) fn set_text_position(&mut self, col: u16, row: u16) {
        self.draw_params.text_col = col;
        self.draw_params.text_row = row;
    }

    /// 邏輯寫 `word_2687E`（形狀的寬）。下一個繪製命令把它記成 `stride_reg`。
    /// FFFFh 是資料段上「沒寫過」的記號，形狀寬不會是這個值，寫了等於沒寫。
    pub(crate) fn set_shape_width(&mut self, width: u16) {
        self.draw_params.shape_width = DrawParams::shape_width_from_word(width);
    }

    /// 繪製的暫存區選擇（原版 `word_2685C`），之後的繪製命令都帶這個值，直到有人再設。
    pub(crate) fn select_scratch(&mut self, select: u16) {
        self.draw_params.scratch = select;
    }

    /// 目前繪製頁的偏移：奇偶幀交替，0 或 8000h。
    pub(crate) fn draw_page(&self) -> u16 {
        self.draw_params.page
    }

    // ---- 頁 ----

    /// 顯示 `page`，改畫另一頁（原版 `sub_19A37`、`sub_19A88`）。顯示頁 0 時一併同步音效開關（`sub_187C3`）。
    pub(crate) fn show_page(&mut self, page: Page) {
        if page == Page::Zero {
            self.sync_sound_switch();
        }
        self.display_start = Some(page.offset());
        self.draw_params.page = page.other().offset();
    }

    /// 之後的繪製畫在 `page`，不改顯示的頁（原版 `sub_18812`、`sub_18819`）。
    pub(crate) fn draw_on(&mut self, page: Page) {
        self.draw_params.page = page.offset();
    }

    // ---- 圖庫項 ----

    /// `MY.GO` 的遮罩 AND，挖出形狀。
    pub(crate) fn cut_my(&mut self, copy: MyCopy) {
        self.emit(match copy {
            MyCopy::General => Op::S18D46,
            MyCopy::Effect => Op::S18E30,
            MyCopy::PlayerShot => Op::S18E73,
        });
    }

    /// `MY.GO` 的四平面 OR。
    pub(crate) fn overlay_my(&mut self, copy: MyCopy) {
        self.emit(match copy {
            MyCopy::General => Op::S18CE2,
            MyCopy::Effect => Op::S18DCC,
            MyCopy::PlayerShot => Op::S18EB6,
        });
    }

    /// `MY.GO` 的遮罩閃白（`sub_18D89`），玩家受擊後的無敵幀。
    pub(crate) fn flash_my(&mut self) {
        self.emit(Op::S18D89);
    }

    /// `MY.GO` 的四平面覆寫（`sub_18C0A`）。
    pub(crate) fn overwrite_my(&mut self) {
        self.emit(Op::S18C0A);
    }

    /// `MY.GO` 的四平面反相覆寫（`sub_18C6E`），選單用來標出選中的列。
    pub(crate) fn overwrite_my_inverted(&mut self) {
        self.emit(Op::S18C6E);
    }

    /// 目前載入的關卡圖庫（`SHT`、`BH`）的遮罩 AND（`sub_18F1A`）。遮罩取自 [`Banks::mask`](crate::loader::Banks)。
    pub(crate) fn cut_stage(&mut self) {
        self.emit(Op::S18F1A);
    }

    /// 關卡圖庫的遮罩閃白（`sub_19025`）。
    pub(crate) fn flash_stage(&mut self) {
        self.emit(Op::S19025);
    }

    /// 關卡圖庫的四平面 OR（`sub_18F5D`）。
    pub(crate) fn overlay_stage(&mut self) {
        self.emit(Op::S18F5D);
    }

    /// 關卡圖庫的四平面覆寫（`sub_18FC1`）。
    pub(crate) fn overwrite_stage(&mut self) {
        self.emit(Op::S18FC1);
    }

    /// `BEG1`、`J` 圖庫的四平面覆寫（`sub_19068`）。
    pub(crate) fn overwrite_beg(&mut self) {
        self.emit(Op::S19068);
    }

    // ---- 寫進暫存區 ----

    /// 關卡圖庫 `index` 的圖寫進暫存區，位置 (`x`, `y`)（原版寫 `word_26880`、`26882`、`26888` 後呼叫 `sub_19ADC`）。
    pub(crate) fn draw_tile_to_scratch(&mut self, x: u16, y: u16, index: u16) {
        self.set_blit_params(x, y, index);
        self.draw_current_tile_to_scratch();
    }

    /// 同上，位置與索引沿用現有的繪製參數（迴圈裡逐份往右畫時用）。
    pub(crate) fn draw_current_tile_to_scratch(&mut self) {
        self.emit(Op::S19ADC);
    }

    /// `BEG1`、`J` 圖庫的圖寫進暫存區，位置與索引取自繪製參數（`sub_19B40`）。
    pub(crate) fn draw_current_beg_to_scratch(&mut self) {
        self.emit(Op::S19B40);
    }

    /// 第三關的地圖圖塊寫進暫存區（`sub_18ACE`，固定讀 `MAP` 圖庫）。
    pub(crate) fn draw_map_tile_to_scratch(&mut self) {
        self.emit(Op::S18ACE);
    }

    // ---- 暫存區複製到繪製頁 ----

    /// 暫存區的 `rows` 列複製到繪製頁，種類見 [`RowCopy`]。位置與來源取自繪製參數。
    /// `rows` 進命令的 `cx`。
    pub(crate) fn copy_scratch_rows(&mut self, how: RowCopy, rows: u16) {
        let op = match how {
            RowCopy::Replace => Op::S19BA4,
            RowCopy::ReplaceShifted => Op::S19D08,
            RowCopy::ReplaceMirrored => Op::S19C4F,
            RowCopy::Or => Op::S19DA8,
            RowCopy::OrMirrored => Op::S19F3B,
            RowCopy::And => Op::S1A09F,
            RowCopy::AndMirrored => Op::S1A1E6,
            RowCopy::LatchPairs => Op::S19FF9,
            RowCopy::AndPairs => Op::S1A144,
            RowCopy::OrPairs => Op::S19E66,
        };
        self.emit_with(op, rows, 0);
    }

    // ---- 填色與數字 ----

    /// 填實色（`sub_1A287`）：從目的列起 `rows` 列，`color` 是色號。
    pub(crate) fn fill_rows(&mut self, rows: u16, color: u16) {
        self.emit_with(Op::S1A287, rows, color);
    }

    /// 血條的一格（`sub_1929F`），`color` 是色號。
    pub(crate) fn draw_bar_cell(&mut self, color: u16) {
        self.emit_with(Op::S1929F, 0, color);
    }

    /// 5 位數字（`sub_1890D`），位置見 [`Core::set_text_position`]。`value` 進命令的 `cx`。
    /// 原版先把數字轉成 5 個 ASCII 位元組寫進 `268A1h` 起的緩衝，再由繪製常式讀；緩衝是函式內先寫後讀的工作變數，
    /// 在 `DS_EXEMPT`（`9CE1..9CE5`），這裡不寫，由接收端依 `value` 轉換。
    pub(crate) fn draw_number(&mut self, value: u16) {
        self.emit_with(Op::S1890D, value, 0);
    }

    // ---- 清除 ----

    /// 清除目前的繪製頁（原版 `sub_19CF9`，190 列）。
    pub(crate) fn clear_draw_page(&mut self) {
        self.emit(Op::S19CF9);
    }

    /// 清暫存區 A（原版 `sub_11504`）。
    pub(crate) fn clear_scratch_a(&mut self) {
        self.emit(Op::S11504);
    }

    /// 清暫存區 B（原版 `sub_1152D`）。
    pub(crate) fn clear_scratch_b(&mut self) {
        self.emit(Op::S1152D);
    }

    /// 兩個顯示頁都清成黑（原版 `sub_18894`）。
    pub(crate) fn clear_both_pages(&mut self) {
        self.emit(Op::S18894);
    }

    /// 一幀畫完、翻頁之前的頁複製（原版 `sub_1117F`）。只有 MGA（Hercules）有動作，位置見 `docs/re/14-display-modes.md`。
    /// 命令只記頁偏移，不動 `shape_width`，所以 E 模式的其他命令與資料段完全不受影響。
    pub(crate) fn end_of_frame_copy(&mut self) {
        let mut cmd = self.blit_cmd(Op::S1117F, 0, 0);
        cmd.stride_reg = None;
        cmd.obj = None;
        self.draws.push(DrawCmd::Blit(cmd));
    }

    // ---- 顯示記憶體的平面 ----

    /// 只留一個平面可寫（原版 `sub_1882E`）。`plane` 是 1、2、4、8，對應平面 3、2、1、0。
    pub(crate) fn select_plane(&mut self, plane: u8) {
        let mask = if plane & 1 != 0 {
            8
        } else if plane & 2 != 0 {
            4
        } else if plane & 4 != 0 {
            2
        } else {
            1
        };
        self.draws.push(DrawCmd::SetMapMask { mask });
    }

    /// 全部平面可寫（原版 `sub_18878`，map mask 回到 0Fh，GC 回到覆寫）。
    pub(crate) fn select_all_planes(&mut self) {
        self.draws.push(DrawCmd::SetMapMask { mask: 0x0F });
    }

    /// 第三關地圖捲動的顯示記憶體命令，種類見 [`ScrollOp`]。
    pub(crate) fn scroll_op(&mut self, which: ScrollOp) {
        self.emit(match which {
            ScrollOp::RotateCopy => Op::S17EBA,
            ScrollOp::ShiftLeft4 => Op::S17EEA,
            ScrollOp::RotateColumn => Op::S17F26,
            ScrollOp::MergeSeam => Op::S17F5D,
            ScrollOp::ShowA => Op::S17FA0,
            ScrollOp::ShowB => Op::S18031,
            ScrollOp::ShiftBothLeft => Op::S180C6,
        });
    }

    // ---- 命令 ----

    /// 送出不讀 `cx`、`dx` 的繪製命令。
    fn emit(&mut self, op: Op) {
        self.emit_with(op, 0, 0);
    }

    /// 送出繪製命令，`cx`、`dx` 是接收端要讀的列數、色號或數值（見檔頭的表）。
    /// 繪製對 `word_2687E` 的寫入在接收端，核心不會更新，所以送出後回到「沒寫過」，才分得出之後邏輯有沒有再寫。
    fn emit_with(&mut self, op: Op, cx: u16, dx: u16) {
        let cmd = self.blit_cmd(op, cx, dx);
        self.draw_params.shape_width = None;
        self.draws.push(DrawCmd::Blit(cmd));
    }

    fn blit_cmd(&self, op: Op, cx: u16, dx: u16) -> BlitCmd {
        let p = &self.draw_params;
        BlitCmd {
            op,
            x: p.x,
            y: p.y,
            index: p.index,
            page: p.page,
            scratch: p.scratch,
            src_x: p.source_col,
            src_y: p.source_row,
            shift: p.shift,
            cx,
            dx,
            text_x: p.text_col,
            text_y: p.text_row,
            sht_group: self.banks.sht,
            mask_group: self.banks.mask,
            beg_group: self.banks.beg,
            stride_reg: p.shape_width,
            obj: self.cur_obj,
        }
    }

    /// 測試用：設繪製頁。
    #[cfg(test)]
    pub(crate) fn set_draw_page(&mut self, page: u16) {
        self.draw_params.page = page;
    }

    /// 測試用：邏輯寫過、還沒被繪製命令帶走的形狀寬；沒有則 `None`。
    #[cfg(test)]
    pub(crate) fn pending_shape_width(&self) -> Option<u16> {
        self.draw_params.shape_width
    }

    /// 測試用：回到「上一個繪製命令之後邏輯沒有寫過」的狀態。
    #[cfg(test)]
    pub(crate) fn forget_shape_width(&mut self) {
        self.draw_params.shape_width = None;
    }

    /// 測試用：目前的暫存區選擇。
    #[cfg(test)]
    pub(crate) fn scratch_select(&self) -> u16 {
        self.draw_params.scratch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Fixes, Start};

    fn core() -> Core {
        let image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
        let mut c = Core::new(&image, &[], Start::Demo, Fixes::NONE);
        c.draws.clear();
        c
    }

    fn last_blit(c: &Core) -> BlitCmd {
        let Some(DrawCmd::Blit(b)) = c.draws.last().copied() else { panic!("沒有繪製命令") };
        b
    }

    #[test]
    fn parameters_stay_until_someone_writes_them() {
        let mut c = core();
        c.set_blit_params(7, 0x30, 9);
        c.set_blit_source_shifted(2, 3, 4);
        c.select_scratch(SCRATCH_B);
        c.overlay_stage();
        c.set_blit_y(0x40);
        c.add_blit_x(2);
        c.add_blit_index(1);
        c.draw_current_tile_to_scratch();
        let b = last_blit(&c);
        assert_eq!((b.x, b.y, b.index), (9, 0x40, 10), "沒寫的參數沿用，寫了的換新值，加法環繞");
        assert_eq!((b.src_x, b.src_y, b.shift, b.scratch), (2, 3, 4, SCRATCH_B));
    }

    #[test]
    fn the_shape_width_goes_into_the_next_command_only() {
        let mut c = core();
        c.forget_shape_width();
        c.set_shape_width(7);
        c.overlay_stage();
        assert_eq!(last_blit(&c).stride_reg, Some(7));
        assert_eq!(c.pending_shape_width(), None, "命令帶走之後回到沒寫過");
        c.overlay_stage();
        assert_eq!(last_blit(&c).stride_reg, None);
        c.set_shape_width(0xFFFF);
        assert_eq!(c.pending_shape_width(), None, "FFFFh 是沒寫過的記號");
    }

    #[test]
    fn the_hercules_copy_does_not_take_the_shape_width() {
        let mut c = core();
        c.set_shape_width(5);
        c.end_of_frame_copy();
        let b = last_blit(&c);
        assert_eq!((b.op, b.stride_reg, b.obj), (Op::S1117F, None, None));
        assert_eq!(c.pending_shape_width(), Some(5), "S1117F 在 E 模式是 retn，不動 word_2687E");
    }

    #[test]
    fn only_the_documented_commands_carry_rows_color_or_value() {
        let mut c = core();
        for how in [
            RowCopy::Replace,
            RowCopy::ReplaceShifted,
            RowCopy::ReplaceMirrored,
            RowCopy::Or,
            RowCopy::OrMirrored,
            RowCopy::And,
            RowCopy::AndMirrored,
            RowCopy::LatchPairs,
            RowCopy::AndPairs,
            RowCopy::OrPairs,
        ] {
            c.copy_scratch_rows(how, 0x59);
            let b = last_blit(&c);
            assert_eq!((b.cx, b.dx), (0x59, 0), "{how:?} 的 cx 是列數");
        }
        c.fill_rows(0x2B, 1);
        assert_eq!((last_blit(&c).cx, last_blit(&c).dx), (0x2B, 1));
        c.draw_bar_cell(0x0B);
        assert_eq!((last_blit(&c).cx, last_blit(&c).dx), (0, 0x0B));
        c.draw_number(12345);
        assert_eq!((last_blit(&c).cx, last_blit(&c).dx), (12345, 0));
        // 其餘動作不讀 cx、dx，前面留下的列數、色號不會跟進來。
        c.draw_current_tile_to_scratch();
        c.cut_my(MyCopy::General);
        c.overlay_my(MyCopy::Effect);
        c.clear_draw_page();
        c.scroll_op(ScrollOp::ShowA);
        for cmd in &c.draws[c.draws.len() - 4..] {
            let DrawCmd::Blit(b) = cmd else { panic!() };
            assert_eq!((b.cx, b.dx), (0, 0), "{:?}", b.op);
        }
    }

    #[test]
    fn each_copy_of_the_my_routines_emits_its_own_op() {
        let mut c = core();
        let mut ops = Vec::new();
        for copy in [MyCopy::General, MyCopy::Effect, MyCopy::PlayerShot] {
            c.cut_my(copy);
            ops.push(last_blit(&c).op);
            c.overlay_my(copy);
            ops.push(last_blit(&c).op);
        }
        assert_eq!(ops, [Op::S18D46, Op::S18CE2, Op::S18E30, Op::S18DCC, Op::S18E73, Op::S18EB6]);
    }

    #[test]
    fn showing_page_zero_draws_on_page_one_and_the_other_way_round() {
        let mut c = core();
        c.show_page(Page::Zero);
        assert_eq!((c.display_start, c.draw_page()), (Some(0), 0x8000));
        c.show_page(Page::One);
        assert_eq!((c.display_start, c.draw_page()), (Some(0x8000), 0));
        c.draw_on(Page::One);
        assert_eq!((c.display_start, c.draw_page()), (Some(0x8000), 0x8000), "draw_on 不動顯示頁");
    }

    #[test]
    fn plane_selection_maps_the_bit_to_the_vga_plane() {
        let mut c = core();
        for (plane, mask) in [(1u8, 8u8), (2, 4), (4, 2), (8, 1)] {
            c.select_plane(plane);
            assert_eq!(c.draws.last(), Some(&DrawCmd::SetMapMask { mask }));
        }
        c.select_all_planes();
        assert_eq!(c.draws.last(), Some(&DrawCmd::SetMapMask { mask: 0x0F }));
    }
}
