//! 第二關第一段（幀計數不超過 104h）的背景：五層視差、前景火焰面與上下黑框。
//! 原版是 `sub_1355B`、`sub_13580`、`sub_135EB`、`sub_1365F`、`sub_136CA`、`sub_13711`、`sub_13443`、`sub_1375F`。
//!
//! 像素位移相位：原版的繪圖以位元組為單位，要慢於 8 像素每幀就預先做好位移 0 到 7 像素的多個相位，每幀選一個。
//! `BH2` 的第 5 至 12 項是 8 個相位、第 13 至 16 項是 4 個、第 17 與 18 項是 2 個。繪製時以 `2 × 相位 + 基底` 當水平位移。

use super::{Backdrop, Strip};
use crate::draw::Pool;
use crate::game::Core;
use crate::video::{RowCopy, SCRATCH_A, SCRATCH_B};

/// 背景磚的來源列與高度，靜態複製到像素列 1Ah。
const FLOOR_SOURCE_ROW: u16 = 0x70;
const FLOOR_TOP_ROW: u16 = 0x1A;
const FLOOR_ROWS: u16 = 0x8C;
/// 近景條的來源列（A、B、C 三種相同，都取 `BH2` 的相位圖）。
const STRIP_SOURCE_ROW: u16 = 0x70;

/// 近景條 A：5 份，間隔 8 位元組，兩個像素列；相位 8 個（7 到 0），起點在 8 到 1 之間循環。
const STRIP_A: StripShape = StripShape { copies: 5, gap: 8, rows: [0x69, 0x4D], shift_base: 0x28, height: 9, phase_reload: 7, start_reload: 8 };
/// 近景條 B：4 份，間隔 10 位元組，相位 4 個。
const STRIP_B: StripShape = StripShape { copies: 4, gap: 0x0A, rows: [0x72, 0x44], shift_base: 0x38, height: 9, phase_reload: 3, start_reload: 0x0A };
/// 近景條 C：2 份，間隔 20 位元組，相位 2 個。
const STRIP_C: StripShape = StripShape { copies: 2, gap: 0x14, rows: [0x7C, 0x36], shift_base: 0x40, height: 0x0D, phase_reload: 1, start_reload: 0x14 };

/// 柱：2 份，間隔 20 位元組，像素列 36h，來源在暫存區 B。
const PILLAR_COPIES: u16 = 2;
const PILLAR_GAP: u16 = 0x14;
const PILLAR_ROW: u16 = 0x36;
const PILLAR_SOURCE_COL: u16 = 0x10;
const PILLAR_ROWS: u16 = 0x53;
const PILLAR_RELOAD: u16 = 0x14;

/// 遮罩層：隨近景條 B 的相位，畫在像素列 4Dh，來源在暫存區 B。
const MASK_ROW: u16 = 0x4D;
const MASK_LAYER_ROWS: u16 = 0x25;

/// 火焰面：每條 22h 列的圖加 16h 列的遮罩，上下各一份（上面一份垂直鏡射）。
const FLAME_IMAGE_ROWS: u16 = 0x22;
const FLAME_MASK_ROWS: u16 = 0x16;
const FLAME_PHASE_LAST: u16 = 0x28;

/// 黑框：幀計數不超過 107h 時上下各 20 列，108h 到 27Fh 為 16 列與 15 列，之後不畫。
const BARS_FULL_UNTIL: u16 = 0x107;
const BARS_NARROW_UNTIL: u16 = 0x280;

/// 三種近景條的差異。
struct StripShape {
    copies: u16,
    /// 相鄰兩份的間隔，單位是位元組。
    gap: u16,
    /// 每份畫兩次的像素列。
    rows: [u16; 2],
    /// 水平位移 = `2 × 相位 + shift_base`。
    shift_base: u16,
    /// 條的列數。
    height: u16,
    /// 相位用完時重新裝填的值，以及起點用完時重新裝填的值。
    phase_reload: u16,
    start_reload: u16,
}

impl Strip {
    /// 相位減 1；用完（變負）重新裝填並讓起點左移一位元組；起點用完（不大於 0）也重新裝填。
    fn advance(&mut self, shape: &StripShape) {
        self.phase = self.phase.wrapping_sub(1);
        if (self.phase as i16) >= 0 {
            return;
        }
        self.phase = shape.phase_reload;
        self.start = self.start.wrapping_sub(1);
        if (self.start as i16) > 0 {
            return;
        }
        self.start = shape.start_reload;
    }
}

impl Core {
    /// 背景磚：來源列 70h 的 140 列靜態複製到像素列 1Ah（原版 `sub_1355B`）。
    pub(crate) fn draw_stage2_floor(&mut self) {
        self.set_obj(Pool::Background, 0);
        self.set_blit_source(0, FLOOR_SOURCE_ROW);
        self.set_blit_x(0);
        self.set_blit_y(FLOOR_TOP_ROW);
        self.set_blit_shift(0);
        self.copy_scratch_rows(RowCopy::Replace, FLOOR_ROWS);
        self.clear_obj();
    }

    /// 近景條 A（原版 `sub_13580`）：每幀相位減 1，每 8 幀整體左移 1 位元組。
    pub(crate) fn draw_stage2_strip_a(&mut self) {
        let strip = self.scenes.stage2.strip_a;
        self.set_blit_x(strip.start);
        self.set_obj(Pool::Background, 1);
        for _ in 0..STRIP_A.copies {
            for row in STRIP_A.rows {
                self.set_blit_y(row);
                self.draw_strip_piece(&STRIP_A, strip.phase);
            }
            self.add_blit_x(STRIP_A.gap);
        }
        self.clear_obj();
        self.scenes.stage2.strip_a.advance(&STRIP_A);
    }

    /// 近景條 B 與它的遮罩層（原版 `sub_135EB`）：每份先畫兩個像素列的條，再畫一份遮罩層。
    pub(crate) fn draw_stage2_strip_b(&mut self) {
        let strip = self.scenes.stage2.strip_b;
        self.set_blit_x(strip.start);
        for _ in 0..STRIP_B.copies {
            self.set_obj(Pool::Background, 2);
            for row in STRIP_B.rows {
                self.set_blit_y(row);
                self.draw_strip_piece(&STRIP_B, strip.phase);
            }
            self.clear_obj();
            self.set_blit_y(MASK_ROW);
            self.draw_stage2_mask_layer(strip.phase);
            self.add_blit_x(STRIP_B.gap);
        }
        self.scenes.stage2.strip_b.advance(&STRIP_B);
    }

    /// 近景條 C（原版 `sub_1365F`）。
    pub(crate) fn draw_stage2_strip_c(&mut self) {
        let strip = self.scenes.stage2.strip_c;
        self.set_blit_x(strip.start);
        self.set_obj(Pool::Background, 3);
        for _ in 0..STRIP_C.copies {
            for row in STRIP_C.rows {
                self.set_blit_y(row);
                self.draw_strip_piece(&STRIP_C, strip.phase);
            }
            self.add_blit_x(STRIP_C.gap);
        }
        self.clear_obj();
        self.scenes.stage2.strip_c.advance(&STRIP_C);
    }

    /// 一份近景條（原版 `sub_135CD`、`sub_13641`、`sub_136AC`）：目的位置已由呼叫端設好。
    fn draw_strip_piece(&mut self, shape: &StripShape, phase: u16) {
        self.set_blit_shift(phase.wrapping_shl(1).wrapping_add(shape.shift_base));
        self.set_blit_source(0, STRIP_SOURCE_ROW);
        self.copy_scratch_rows(RowCopy::LatchPairs, shape.height);
    }

    /// 遮罩層（原版 `sub_136CA`）：暫存區 B 的遮罩 AND 加圖 OR，各 37 列，水平位移 `2 × 近景條 B 的相位`。
    fn draw_stage2_mask_layer(&mut self, phase: u16) {
        let shift = phase.wrapping_shl(1);
        self.set_obj(Pool::Background, 4);
        self.set_blit_shift(shift);
        self.set_blit_source(8, 0);
        self.select_scratch(SCRATCH_B);
        self.copy_scratch_rows(RowCopy::AndPairs, MASK_LAYER_ROWS);
        self.set_blit_shift(shift);
        self.set_blit_source(0, 0);
        self.select_scratch(SCRATCH_B);
        self.copy_scratch_rows(RowCopy::OrPairs, MASK_LAYER_ROWS);
        self.select_scratch(SCRATCH_A);
        self.clear_obj();
    }

    /// 柱（原版 `sub_13711`）：每幀左移 1 位元組，用完（不大於 0）回到起點。
    pub(crate) fn draw_stage2_pillars(&mut self) {
        let start = self.scenes.stage2.pillar;
        self.set_blit_x(start);
        self.set_blit_y(PILLAR_ROW);
        self.set_blit_shift(0);
        self.set_blit_source(PILLAR_SOURCE_COL, 0);
        self.set_obj(Pool::Background, 5);
        for _ in 0..PILLAR_COPIES {
            self.select_scratch(SCRATCH_B);
            self.copy_scratch_rows(RowCopy::LatchPairs, PILLAR_ROWS);
            self.select_scratch(SCRATCH_A);
            self.add_blit_x(PILLAR_GAP);
        }
        self.clear_obj();
        let pillar = &mut self.scenes.stage2.pillar;
        *pillar = pillar.wrapping_sub(1);
        if (*pillar as i16) <= 0 {
            *pillar = PILLAR_RELOAD;
        }
    }

    /// 上下黑框（原版 `sub_1375F`）。
    pub(crate) fn draw_stage2_bars(&mut self) {
        let frame = self.stage_frame();
        let (top_rows, bottom_row, bottom_rows) = if frame <= BARS_FULL_UNTIL {
            (0x14, 0xAA, 0x14)
        } else if frame < BARS_NARROW_UNTIL {
            (0x10, 0xAF, 0x0F)
        } else {
            return;
        };
        self.set_obj(Pool::Background, 6);
        self.set_blit_x(0);
        self.set_blit_y(0);
        self.fill_rows(top_rows, 0);
        self.set_blit_x(0);
        self.set_blit_y(bottom_row);
        self.fill_rows(bottom_rows, 0);
        self.clear_obj();
    }

    /// 前景火焰面（原版 `sub_13443`、`sub_13452`）：上下各兩層，遮罩 AND 加圖 OR，頁不同用不同的來源列。
    /// 幀 1 設起點（底端 156、頂端 34），幀 640 起底端每幀加 1、頂端每幀減 1，幀 674 起不再畫。
    pub(crate) fn draw_stage2_flames(&mut self) {
        self.select_scratch(SCRATCH_A);
        let frame = self.stage_frame();
        if frame == 1 {
            self.scenes.stage2.flame_bottom = 0x9C;
            self.scenes.stage2.flame_top = 0x22;
        }
        if frame >= 0x280 {
            let s = &mut self.scenes.stage2;
            s.flame_bottom = s.flame_bottom.wrapping_add(1);
            s.flame_top = s.flame_top.wrapping_sub(1);
            if frame >= 0x2A2 {
                return;
            }
        }
        let Backdrop { flame_bottom: bottom, flame_top: top, flame_phase: phase, .. } = self.scenes.stage2;
        // 奇數頁（第二頁，`word_26872` 非 0）遮罩從第 44h 列、圖從第 0 列；偶數頁遮罩從第 5Ah 列、圖從第 22h 列。
        let (mask_row, image_row) = if self.draw_page() != 0 { (0x44, 0) } else { (0x5A, 0x22) };
        self.set_obj(Pool::Background, 7);
        self.set_blit_source(0, mask_row);
        self.set_blit_x(0);
        self.set_blit_y(bottom);
        self.set_blit_shift(phase);
        self.copy_scratch_rows(RowCopy::And, FLAME_MASK_ROWS);
        self.set_blit_y(top);
        self.copy_scratch_rows(RowCopy::AndMirrored, FLAME_MASK_ROWS);
        self.set_blit_source(0, image_row);
        self.set_blit_x(0);
        self.set_blit_y(bottom);
        self.set_blit_shift(phase);
        self.copy_scratch_rows(RowCopy::Or, FLAME_IMAGE_ROWS);
        self.set_blit_y(top);
        self.copy_scratch_rows(RowCopy::OrMirrored, FLAME_IMAGE_ROWS);
        self.clear_obj();
        let s = &mut self.scenes.stage2;
        s.flame_phase = if s.flame_phase >= FLAME_PHASE_LAST { 0 } else { s.flame_phase + 1 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strip_reloads_its_phase_when_it_goes_negative_and_its_start_when_that_runs_out() {
        let mut s = Strip { phase: 1, start: 3 };
        s.advance(&STRIP_A);
        assert_eq!((s.phase, s.start), (0, 3));
        s.advance(&STRIP_A);
        assert_eq!((s.phase, s.start), (STRIP_A.phase_reload, 2), "相位變負：重新裝填，起點減 1");
        s.start = 1;
        s.phase = 0;
        s.advance(&STRIP_A);
        assert_eq!((s.phase, s.start), (STRIP_A.phase_reload, STRIP_A.start_reload), "起點減到 0：重新裝填");
    }

    #[test]
    fn the_strip_phase_test_is_signed_and_the_start_test_is_strictly_positive() {
        // 相位 8001h 減 1 是 8000h，有號比較是負數，所以重新裝填；起點 7FFFh 減 1 是 7FFEh，仍為正，保留。
        let mut s = Strip { phase: 0x8001, start: 0x7FFF };
        s.advance(&STRIP_C);
        assert_eq!((s.phase, s.start), (STRIP_C.phase_reload, 0x7FFE));
        // 起點 0 減 1 是 FFFFh（負數），重新裝填。
        let mut s = Strip { phase: 0, start: 0 };
        s.advance(&STRIP_C);
        s.advance(&STRIP_C);
        assert_eq!(s.start, STRIP_C.start_reload);
    }
}
