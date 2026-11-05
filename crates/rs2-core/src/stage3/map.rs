//! 第三關的地圖捲動。原版是 `sub_17DF9`、`sub_17E73`、`sub_182CD`、`sub_18337`、`sub_180C6`、`sub_17FA0`、`sub_18031`、
//! `sub_1839F`、`sub_1840E`。規格見 `docs/re/12-stages-presentation.md` 第 5.3 節。
//!
//! 地圖 `SHT.SCN` 解壓後是 600 欄 × 20 列的圖塊索引（u16），列寬 600，索引公式 `(600 × 列 + 欄) × 2`。畫面視窗 42 欄
//! （其中 40 欄可見）× 19 列，每個圖塊 8×10 像素，畫在暫存區 A；暫存區 B 是 A 左移 4 像素的複本。
//! 每兩個頁面捲動 1 欄（8 像素）：第二頁推進視窗、把新的一欄畫進 A 並顯示 A，第一頁把新欄同步到 B 並顯示 B。
//!
//! 同時有兩份格網跟著捲動：碰撞用的 G4（每格是該處圖塊的索引，非 0 就擋玩家彈與玩家）和髒格 G3。它們仍在資料段，
//! 由 `mirror.rs` 的格網一節擁有，這裡經 `scroll_terrain_grids` 等具名函式操作。

use crate::draw::Pool;
use crate::game::Core;
use crate::grid::GRID_COLS;
use crate::video::{SCRATCH_A, SCRATCH_B, ScrollOp};

/// 地圖每列的欄數。戰場加大（規格 005）時地圖欄數與畫面欄數（`GRID_COLS`）、新欄生成位置要一起改。
const MAP_COLUMNS: u16 = 0x258;
/// 每個捲動步前進幾欄（視窗與地圖左緣各加這麼多），每兩個頁面一步。
const SCROLL_STEP_COLUMNS: u16 = 1;
/// 視窗的欄數（含畫面右緣外多畫的兩欄）與可見欄數；視窗左緣在 0 到 `GRID_COLS - 1` 之間循環。
const WINDOW_COLUMNS: u16 = 0x2A;
/// 視窗的列數（地圖有 20 列，第 20 列在可達碼中沒有用到）。
const WINDOW_ROWS: u16 = 0x13;
/// 一個圖塊的像素列數。
const TILE_ROWS: u16 = 0x0A;
/// 新的一欄畫在視窗左緣之右這麼多欄（可見寬度）。
const NEW_COLUMN_OFFSET: u16 = GRID_COLS;
/// 地圖往下一列的索引增量，單位是位元組（`2 × MAP_COLUMNS`）。
const MAP_ROW_BYTES: u16 = 2 * MAP_COLUMNS;
/// 初始填入時，視窗畫完一列後地圖索引要再跳過的位元組：一列 `MAP_ROW_BYTES` 減去已走過的 `2 × WINDOW_COLUMNS`。
const MAP_ROW_SKIP: u16 = MAP_ROW_BYTES - 2 * WINDOW_COLUMNS;
/// 捲動步取新欄時，地圖索引逐列增加的位元組：一列 `MAP_ROW_BYTES`。
const NEW_COLUMN_STRIDE: u16 = MAP_ROW_BYTES;

/// 地圖視窗的位置。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapScroll {
    /// `word_25BC4`：視窗在 80 位元組寬的暫存區內的欄偏移，0 到 39，每個捲動步加 1，到 40 歸零並把暫存區整個左移。
    pub window: u16,
    /// `word_25BC6`：地圖左緣欄，每個捲動步加 1。
    pub left_col: u16,
    /// `word_25BC8`：地圖第一列。可達碼中恆為 0（垂直捲動的段落沒有入口）。
    pub top_row: u16,
}

impl Core {
    /// 解壓地圖：`SHT.SCN`（E）或 `SHT.SCR`（單色）的 RLE 串流解到 `map_seg`（原版 `sub_1BF71` 的解壓部分）。
    /// 選哪一份由前端的顯示模式決定，見 `Game::set_terrain_scr`。
    pub(crate) fn load_map(&mut self) {
        let files = self.files.clone();
        let src = if self.port.borrow().terrain_scr { &files.scr } else { &files.scn };
        self.map_seg = if src.len() >= 2 { rs2_formats::unrle(src, 0) } else { Vec::new() };
    }

    /// 地圖的一個 u16（`es:[bx]`，`es` 是地圖遠段）。超出解壓結果的部分原版讀到遠記憶體殘值，這裡讀 0；
    /// 可達的最大位移約 5900h，遠小於解壓後的 24005 位元組，所以實際不會落到這裡。
    fn map_word(&self, byte_offset: u16) -> u16 {
        let o = usize::from(byte_offset);
        let lo = self.map_seg.get(o).copied().unwrap_or(0);
        let hi = self.map_seg.get(o + 1).copied().unwrap_or(0);
        u16::from_le_bytes([lo, hi])
    }

    /// 視窗左緣在地圖裡的圖塊序號：`600 × 地圖第一列 + 地圖左緣欄`（16 位元）。
    fn map_origin(&self) -> u16 {
        let row_start = u32::from(MAP_COLUMNS) * u32::from(self.scenes.map.top_row);
        (row_start as u16).wrapping_add(self.scenes.map.left_col)
    }

    /// 幀 0 的初始視窗（原版 `sub_17DF9`）：把地圖左側 42 欄 × 19 列圖塊從視窗左緣畫進暫存區 A，
    /// 圖塊索引寫進 G4，再建立暫存區 B。
    ///
    /// G4 的初始填入每列連續寫 42 個字組，列距 54h 位元組；之後各步寫入與碰撞讀取的列距是 50h
    /// （`TERRAIN_ROW_BYTES`）。40 個捲動步之後 19 列 × 40 欄全由捲動步的資料取代，在那之前碰撞讀到的格與畫面上的圖塊不對齊。
    pub(crate) fn draw_initial_map(&mut self) {
        let mut cursor = self.map_origin().wrapping_shl(1);
        self.select_scratch(SCRATCH_A);
        let window = self.scenes.map.window;
        self.set_blit_x(window);
        self.set_blit_y(0);
        let mut cell = 0;
        for _ in 0..WINDOW_ROWS {
            self.set_blit_x(window);
            for _ in 0..WINDOW_COLUMNS {
                let tile = self.map_word(cursor);
                self.fill_terrain_cell(cell, tile);
                self.set_blit_index(tile);
                self.draw_map_tile_to_scratch();
                cursor = cursor.wrapping_add(2);
                self.add_blit_x(1);
                cell += 1;
            }
            cursor = cursor.wrapping_add(MAP_ROW_SKIP);
            self.add_blit_y(TILE_ROWS);
        }
        self.show_scratch_a();
        self.build_shifted_copy();
    }

    /// 暫存區 B 建成 A 左移 4 像素的複本（原版 `sub_17E73`）：四個平面各做一次整份複製與就地左移。
    ///
    /// 複製把 A 的前 3E80h 位元組（200 列 × 80）寫到 B，資料旋轉 4 位元，等於每個位元組的高低半互換；
    /// 左移取下一個位元組的低半與自己的高半。每列最右的位元組取到的是下一列第 0 個位元組的高半，
    /// 最後一輪讀到複製範圍之外的內容。
    fn build_shifted_copy(&mut self) {
        self.set_blit_x(0);
        self.select_scratch(SCRATCH_A);
        for plane in [1, 2, 4, 8] {
            self.select_plane(plane);
            self.scroll_op(ScrollOp::RotateCopy);
            self.scroll_op(ScrollOp::ShiftLeft4);
        }
        self.select_all_planes();
    }

    /// 暫存區 A 從視窗左緣起的 40 欄 × 190 列，以寫入模式 1 複製到目前的顯示頁（原版 `sub_17FA0`）。
    fn show_scratch_a(&mut self) {
        self.show_scratch_window(SCRATCH_A, ScrollOp::ShowA);
    }

    /// 暫存區 B（A 左移 4 像素）的同一份顯示（原版 `sub_18031`）。結束時暫存區選擇留在 B，呼叫端再選回 A。
    fn show_scratch_b(&mut self) {
        self.show_scratch_window(SCRATCH_B, ScrollOp::ShowB);
    }

    fn show_scratch_window(&mut self, scratch: u16, op: ScrollOp) {
        let window = self.scenes.map.window;
        self.set_blit_x(window);
        self.select_scratch(scratch);
        self.set_obj(Pool::Background, 0);
        self.scroll_op(op);
        self.clear_obj();
        self.set_blit_x(0);
    }

    /// 視窗左緣加 1（原版 `sub_180C6`）。到 40 歸零，並把暫存區 A、B 各整體左移 40 位元組（寫入模式 1）。
    fn advance_window(&mut self) {
        self.scenes.map.window = self.scenes.map.window.wrapping_add(SCROLL_STEP_COLUMNS);
        if self.scenes.map.window < GRID_COLS {
            return;
        }
        self.scenes.map.window = 0;
        self.set_blit_x(0);
        self.select_scratch(SCRATCH_A);
        self.scroll_op(ScrollOp::ShiftBothLeft);
    }

    /// 第二頁（`word_26872` 非 0）的捲動步（原版 `sub_182CD`）：推進視窗，地圖左緣加 1，G4 與 G3 左移一格；
    /// 在視窗右緣外一欄取 19 個圖塊，經 G4 的寫入游標存進 G4，畫進暫存區 A；顯示 A。
    pub(crate) fn scroll_map_second_page(&mut self) {
        self.advance_window();
        self.scenes.map.left_col = self.scenes.map.left_col.wrapping_add(SCROLL_STEP_COLUMNS);
        self.scroll_terrain_grids();
        let mut cursor = self.map_origin().wrapping_add(NEW_COLUMN_OFFSET).wrapping_shl(1);
        self.set_blit_x(self.scenes.map.window.wrapping_add(NEW_COLUMN_OFFSET));
        self.set_blit_y(0);
        self.start_terrain_column();
        for _ in 0..WINDOW_ROWS {
            let tile = self.map_word(cursor);
            self.store_terrain_tile(tile);
            self.set_blit_index(tile);
            self.draw_map_tile_to_scratch();
            cursor = cursor.wrapping_add(NEW_COLUMN_STRIDE);
            self.add_blit_y(TILE_ROWS);
        }
        self.show_scratch_a();
    }

    /// 第一頁（`word_26872` 為 0）的捲動步（原版 `sub_18337`）：新欄 x 是視窗左緣加 40，四個平面各把 A 的這一欄
    /// 轉 4 位元複製到 B，再把 B 的前一欄取自這一欄的低半；顯示 B。
    pub(crate) fn scroll_map_first_page(&mut self) {
        self.set_blit_x(self.scenes.map.window.wrapping_add(NEW_COLUMN_OFFSET));
        self.select_scratch(SCRATCH_A);
        for plane in [1, 2, 4, 8] {
            self.select_plane(plane);
            self.scroll_op(ScrollOp::RotateColumn);
            self.scroll_op(ScrollOp::MergeSeam);
        }
        self.select_all_planes();
        self.show_scratch_b();
        self.select_scratch(SCRATCH_A);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::Op;
    use crate::game::{Fixes, Start};
    use crate::grid::{GRID3_START, GRID4_START, TERRAIN_ROW_BYTES};

    fn core_with_map() -> Core {
        let mut c = Core::for_test(Start::Demo, Fixes::NONE);
        // 圖塊索引 = 1 + 欄 + 7 × 列（用小數字，不碰地圖的真實內容）。
        let mut map = vec![0u8; 20 * usize::from(MAP_COLUMNS) * 2];
        for row in 0..20usize {
            for col in 0..usize::from(MAP_COLUMNS) {
                let v = (1 + col + 7 * row) as u16;
                let o = (row * usize::from(MAP_COLUMNS) + col) * 2;
                map[o..o + 2].copy_from_slice(&v.to_le_bytes());
            }
        }
        c.map_seg = map;
        c.draws.clear();
        c
    }

    fn g4(c: &Core, row: u16, col: u16) -> u16 {
        c.grids.peek16(GRID4_START + 2 + row * TERRAIN_ROW_BYTES + 2 * col)
    }

    #[test]
    fn the_map_origin_is_row_times_600_plus_the_left_column() {
        let mut c = core_with_map();
        c.scenes.map.top_row = 3;
        c.scenes.map.left_col = 5;
        assert_eq!(c.map_origin(), 3 * 600 + 5);
        c.scenes.map.top_row = 200;
        assert_eq!(c.map_origin(), (200u32 * 600 % 65536) as u16 + 5, "乘積取 16 位元");
    }

    #[test]
    fn the_initial_fill_writes_42_words_per_row_with_a_row_stride_of_54h_bytes() {
        let mut c = core_with_map();
        c.scenes.map.left_col = 1;
        c.draw_initial_map();
        let at = |row: u16, col: u16| c.grids.peek16(GRID4_START + 2 + 0x54 * row + 2 * col);
        assert_eq!(at(0, 0), 2, "視窗左緣是地圖第 1 欄");
        assert_eq!(at(0, 41), 43);
        assert_eq!(at(1, 0), 2 + 7, "下一列接在 42 個字組之後");
        assert_eq!(at(18, 41), 2 + 41 + 7 * 18);
        // 碰撞讀取的列距是 50h：列 1 欄 0 讀到的是初始填入的列 0 欄 40。
        assert_eq!(g4(&c, 1, 0), at(0, 40));
    }

    #[test]
    fn a_scroll_step_shifts_the_grids_and_writes_the_new_column_at_the_right_edge() {
        let mut c = core_with_map();
        c.scenes.map.left_col = 0;
        c.draw_initial_map();
        let before: Vec<u16> = (0..40).map(|col| g4(&c, 3, col)).collect();
        c.scroll_map_second_page();
        assert_eq!(c.scenes.map.left_col, 1);
        assert_eq!(c.scenes.map.window, 1);
        for col in 0..39 {
            assert_eq!(g4(&c, 3, col), before[col as usize + 1], "格 {col} 取自右鄰");
        }
        // 新欄寫在 (列 r + 1, 欄 0) 的位置，線性位址上緊接 (列 r, 欄 39)，下一步左移才進到欄 38。
        // 地圖欄 = 左緣 + 40 = 41，列 r 的索引是 1 + 41 + 7r。
        for row in 0..19u16 {
            assert_eq!(c.grids.peek16(GRID4_START + TERRAIN_ROW_BYTES * (row + 1)), 42 + 7 * row, "新欄的列 {row}");
        }
    }

    #[test]
    fn the_grid_shift_pulls_g3s_first_word_into_g4_and_the_write_cursor_into_g3() {
        let mut c = core_with_map();
        let g3_last_word = GRID3_START + 0x63E;
        c.grids.poke16(GRID3_START, 0x1234);
        c.set_terrain_cursor(0x0640);
        c.grids.poke16(g3_last_word, 0xBEEF);
        c.scroll_terrain_grids();
        assert_eq!(c.grids.peek16(GRID4_START + 0x63E), 0x1234, "G4 的最後一個字組取自 G3 的第一個，在 G3 左移之前讀");
        assert_eq!(c.grids.peek16(g3_last_word), 0x0640, "G3 的最後一個字組取自 word_2684C");
        assert_eq!(c.terrain_cursor(), 0x0600, "word_2684C 的低位元組被清 0");
    }

    #[test]
    fn the_window_wraps_at_40_columns_and_shifts_both_scratch_areas() {
        let mut c = core_with_map();
        c.scenes.map.window = 39;
        c.advance_window();
        assert_eq!(c.scenes.map.window, 0);
        let ops: Vec<Op> = c.draws.iter().filter_map(|d| if let crate::draw::DrawCmd::Blit(b) = d { Some(b.op) } else { None }).collect();
        assert_eq!(ops, [Op::S180C6]);
        c.draws.clear();
        c.advance_window();
        assert_eq!(c.scenes.map.window, 1);
        assert!(c.draws.is_empty(), "不到 40 不移動暫存區");
    }

    #[test]
    fn the_first_page_step_walks_the_four_planes_then_shows_b() {
        let mut c = core_with_map();
        c.scenes.map.window = 5;
        c.scroll_map_first_page();
        let blits: Vec<_> = c.draws.iter().filter_map(|d| if let crate::draw::DrawCmd::Blit(b) = d { Some(*b) } else { None }).collect();
        let ops: Vec<Op> = blits.iter().map(|b| b.op).collect();
        assert_eq!(
            ops,
            [Op::S17F26, Op::S17F5D, Op::S17F26, Op::S17F5D, Op::S17F26, Op::S17F5D, Op::S17F26, Op::S17F5D, Op::S18031]
        );
        assert!(blits[..8].iter().all(|b| b.x == 5 + 40), "新欄是視窗左緣加 40");
        assert_eq!((blits[8].x, blits[8].scratch), (5, SCRATCH_B), "顯示 B 從視窗左緣起");
        assert_eq!(c.scratch_select(), SCRATCH_A);
    }

    /// 煙霧測試：以第一關初始化後的資料段從 `Entry::Stage3` 跑到頭目段，確認沒有溢位、越界或進到死碼，
    /// 並逐步檢查 G4 快取等於地圖。不是對拍，對拍由 `rs2-trace` 的 wait 測試負責。需要原版目錄（`RS2_ORIG`）。
    #[test]
    #[ignore]
    fn stage3_smoke() {
        use crate::game::{Entry, Game, GameFiles, Input};
        let dir = std::path::PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh test 跑"));
        let exe = std::fs::read(dir.join("S.EXE")).unwrap();
        let image = rs2_formats::unpack_s_exe(&exe).unwrap();
        let scn = std::fs::read(dir.join("SHT.SCN")).unwrap();
        assert_eq!(rs2_formats::unrle(&scn, 0).len(), 24005);
        let start = Game::new(&image, &[], Start::NewGame { difficulty: None }, Fixes::NONE);
        let state = start.state().clone();
        let map = rs2_formats::unrle(&scn, 0);
        let files = GameFiles { image, scn, ..GameFiles::default() };
        let mut g = Game::from_state(files, state, Entry::Stage3, Fixes::NONE);
        let mut samples = Vec::new();
        let mut cache_checked = 0usize;
        for _ in 0..2600 {
            // 玩家不死（沒有輸入，機體會被地形擠壓），才跑得到頭目段。
            g.state_mut().pl.ship[0].hp = 0xFF;
            let f = g.step(&Input::default(), &mut |g: &Game, pt: u32| {
                samples.push(pt);
                let frame = g.state().flow.frame;
                if pt == crate::game::sample::STAGE3_NORMAL && (100..1100).contains(&frame) {
                    // 捲動 40 步之後，G4 快取的 (列 r, 欄 c) 等於地圖 (r, 左緣欄 + c)。
                    // 例外是欄 0 的第 1 列起：新欄寫在 (r+1, 0)，線性位址上緊接 (r, 39)，下一步左移才進到 (r, 39)，
                    // 所以那 19 格是最新的一欄（地圖 (r-1, 左緣欄 + 40)）。
                    let l = usize::from(g.state().scenes.map.left_col);
                    for r in 0..19usize {
                        for c in 0..40usize {
                            let got = g.state().grids.peek16(GRID4_START + (r * usize::from(TERRAIN_ROW_BYTES) + c * 2) as u16);
                            let (mr, mc) = if c == 0 && r >= 1 { (r - 1, l + 40) } else { (r, l + c) };
                            let o = (mr * usize::from(MAP_COLUMNS) + mc) * 2;
                            let want = u16::from_le_bytes([map[o], map[o + 1]]);
                            assert_eq!(got, want, "frame {frame} L {l} r {r} c {c}");
                        }
                    }
                    cache_checked += 1;
                }
            });
            if f.exit.is_some() {
                let st = g.state();
                println!("離開 {:?}，計數 {}，hp {} {}，存活 {}", f.exit, st.flow.frame, st.pl.ship[0].hp, st.pl.ship[1].hp, st.pl.alive);
                break;
            }
        }
        let normal = samples.iter().filter(|&&p| p == crate::game::sample::STAGE3_NORMAL).count();
        let boss = samples.iter().filter(|&&p| p == crate::game::sample::STAGE3_BOSS).count();
        println!("取樣 普通段 {normal} 頭目段 {boss}，最後地圖左緣 {}", g.state().scenes.map.left_col);
        println!("G4 快取檢查 {cache_checked} 個取樣");
        assert!(normal > 1000 && boss > 100 && cache_checked > 900);
    }
}
