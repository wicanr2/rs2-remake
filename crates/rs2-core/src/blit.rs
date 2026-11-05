//! 關卡畫面用的具名繪製呼叫。
//!
//! `video.rs` 的繪製函式（`s19ba4` 一族）從資料段的繪製參數與暫存器讀參數：欄與列、來源、水平位移、圖庫索引、
//! 列數（`cx`）、色號（`dx`）。關卡的程式不碰暫存器與繪製參數的位移，經這裡的 setter 與呼叫寫入。
//!
//! 每個 setter 只寫原版對應的那一個字組，沒寫的參數沿用上一次的值（原版的行為），所以繪製命令（`BlitCmd`）
//! 帶的欄位和逐函式移植時完全相同。列數與色號改成函式參數，呼叫前一刻才寫進 `cx`、`dx`，與原版的
//! `mov cx, N` 緊接 `call` 相同。繪製領域搬走時，這個檔案整個換成真正的繪製介面。

use crate::draw::Op;
use crate::dseg::a;
use crate::game::Core;

/// `word_26880`：目的欄，單位是位元組（8 像素）。
const BLIT_X: u16 = a(0x26880);
/// `word_26882`：目的像素列。
const BLIT_Y: u16 = a(0x26882);
/// `word_26888`：圖庫索引。
const BLIT_INDEX: u16 = a(0x26888);
/// `word_26864`、`word_26866`：來源欄與來源列（暫存區內）。
const BLIT_SOURCE_COL: u16 = a(0x26864);
const BLIT_SOURCE_ROW: u16 = a(0x26866);
/// `word_26868`：水平位移。
const BLIT_SHIFT: u16 = a(0x26868);
/// `word_2688A`、`word_2688E`：數字的欄與列。
const TEXT_COL: u16 = a(0x2688A);
const TEXT_ROW: u16 = a(0x2688E);

impl Core {
    // ---- 繪製參數 ----

    pub(crate) fn set_blit_x(&mut self, x: u16) {
        self.ds.w16(BLIT_X, x);
    }

    pub(crate) fn set_blit_y(&mut self, y: u16) {
        self.ds.w16(BLIT_Y, y);
    }

    pub(crate) fn set_blit_index(&mut self, index: u16) {
        self.ds.w16(BLIT_INDEX, index);
    }

    /// 來源欄與來源列。
    pub(crate) fn set_blit_source(&mut self, col: u16, row: u16) {
        self.ds.w16(BLIT_SOURCE_COL, col);
        self.ds.w16(BLIT_SOURCE_ROW, row);
    }

    pub(crate) fn set_blit_shift(&mut self, shift: u16) {
        self.ds.w16(BLIT_SHIFT, shift);
    }

    /// 目的欄加 `n`（原版 `add word_26880, n`，迴圈裡逐份往右畫）。
    pub(crate) fn add_blit_x(&mut self, n: u16) {
        let x = self.ds.r16(BLIT_X).wrapping_add(n);
        self.ds.w16(BLIT_X, x);
    }

    pub(crate) fn add_blit_y(&mut self, n: u16) {
        let y = self.ds.r16(BLIT_Y).wrapping_add(n);
        self.ds.w16(BLIT_Y, y);
    }

    pub(crate) fn add_blit_index(&mut self, n: u16) {
        let index = self.ds.r16(BLIT_INDEX).wrapping_add(n);
        self.ds.w16(BLIT_INDEX, index);
    }

    /// 數字（`draw_number`）的位置：欄與列。
    pub(crate) fn set_text_position(&mut self, col: u16, row: u16) {
        self.ds.w16(TEXT_COL, col);
        self.ds.w16(TEXT_ROW, row);
    }

    // ---- 頁 ----

    /// 顯示第一頁（顯示起點 0），改畫第二頁（原版 `sub_19A37`）。
    pub(crate) fn show_first_page(&mut self) {
        self.s19a37();
    }

    /// 顯示第二頁（顯示起點 8000h），改畫第一頁（原版 `sub_19A88`）。
    pub(crate) fn show_second_page(&mut self) {
        self.s19a88();
    }

    /// 之後畫進第一頁，不動顯示起點（原版 `sub_18812`）。
    pub(crate) fn draw_to_first_page(&mut self) {
        self.s18812();
    }

    /// 之後畫進第二頁，不動顯示起點（原版 `sub_18819`）。
    pub(crate) fn draw_to_second_page(&mut self) {
        self.s18819();
    }

    // ---- 寫進暫存區 ----

    /// 圖庫 `index` 的圖寫進暫存區，位置 (`x`, `y`)（原版寫 `word_26880`、`26882`、`26888` 後呼叫 `sub_19ADC`）。
    pub(crate) fn draw_tile_to_scratch(&mut self, x: u16, y: u16, index: u16) {
        self.set_blit_params(x, y, index);
        self.s19adc();
    }

    /// 同上，位置與索引沿用現有的繪製參數（迴圈裡逐份往右畫時用）。
    pub(crate) fn draw_current_tile_to_scratch(&mut self) {
        self.s19adc();
    }

    /// 第三關的地圖圖塊寫進暫存區 A（原版 `sub_18ACE`，固定讀 `MAP` 圖庫）。
    pub(crate) fn draw_map_tile_to_scratch(&mut self) {
        self.s18ace();
    }

    // ---- 暫存區複製到顯示頁，`rows` 是列數（原版 `cx`）----

    /// 暫存區列複製到顯示頁，覆寫（`sub_19BA4`）。
    pub(crate) fn copy_rows_to_page(&mut self, rows: u16) {
        self.r.cx = rows;
        self.s19ba4();
    }

    /// 暫存區列複製到顯示頁，帶水平位移（`sub_19D08`）。
    pub(crate) fn copy_rows_to_page_shifted(&mut self, rows: u16) {
        self.r.cx = rows;
        self.s19d08();
    }

    /// 暫存區列複製到顯示頁，垂直鏡射（`sub_19C4F`）。
    pub(crate) fn copy_rows_to_page_mirrored(&mut self, rows: u16) {
        self.r.cx = rows;
        self.s19c4f();
    }

    /// 暫存區整列 OR 到顯示頁（`sub_19DA8`）。
    pub(crate) fn or_rows_to_page(&mut self, rows: u16) {
        self.r.cx = rows;
        self.s19da8();
    }

    /// 暫存區整列 OR 到顯示頁，垂直鏡射（`sub_19F3B`）。
    pub(crate) fn or_rows_to_page_mirrored(&mut self, rows: u16) {
        self.r.cx = rows;
        self.s19f3b();
    }

    /// 暫存區整列 AND 到顯示頁（`sub_1A09F`）。
    pub(crate) fn and_rows_to_page(&mut self, rows: u16) {
        self.r.cx = rows;
        self.s1a09f();
    }

    /// 暫存區整列 AND 到顯示頁，垂直鏡射（`sub_1A1E6`）。
    pub(crate) fn and_rows_to_page_mirrored(&mut self, rows: u16) {
        self.r.cx = rows;
        self.s1a1e6();
    }

    /// 閂鎖複製，每列 2 位元組（`sub_19FF9`）。
    pub(crate) fn latch_copy_pairs(&mut self, rows: u16) {
        self.r.cx = rows;
        self.s19ff9();
    }

    /// 暫存區 AND，每列 2 位元組（`sub_1A144`）。
    pub(crate) fn and_pairs_to_page(&mut self, rows: u16) {
        self.r.cx = rows;
        self.s1a144();
    }

    /// 暫存區逐平面 OR，每列 2 位元組（`sub_19E66`）。
    pub(crate) fn or_pairs_to_page(&mut self, rows: u16) {
        self.r.cx = rows;
        self.s19e66();
    }

    /// 填實色（`sub_1A287`）：從目的列起 `rows` 列，`color` 是色號。
    pub(crate) fn fill_rows(&mut self, rows: u16, color: u16) {
        self.r.cx = rows;
        self.r.dx = color;
        self.s1a287();
    }

    /// 血條的一格（`sub_1929F`），`color` 是色號。
    pub(crate) fn draw_bar_cell(&mut self, color: u16) {
        self.r.dx = color;
        self.s1929f();
    }

    /// 5 位數字（`sub_1890D`），位置見 [`Core::set_text_position`]。
    pub(crate) fn draw_number(&mut self, value: u16) {
        self.r.cx = value;
        self.s1890d();
    }

    // ---- 第三關捲動的顯示記憶體命令 ----

    /// 選圖庫第 k 個平面（`sub_1882E`），`plane` 是 1、2、4、8。
    pub(crate) fn select_plane(&mut self, plane: u16) {
        self.r.bx = plane;
        self.s1882e();
    }

    /// 全部平面（`sub_18878`，map mask 回到 0Fh）。
    pub(crate) fn select_all_planes(&mut self) {
        self.s18878();
    }

    /// 第三關捲動的顯示記憶體命令（`S17EBA` 一族）。這些命令只讀 `word_26880`、頁與暫存區選擇，不讀列數與色號。
    pub(crate) fn emit_scroll_op(&mut self, op: Op) {
        self.emit_op(op);
    }

    // ---- 音樂 ----

    pub(crate) fn start_music(&mut self) {
        self.s10ad4();
    }

    pub(crate) fn stop_music(&mut self) {
        self.s10b12();
    }
}
