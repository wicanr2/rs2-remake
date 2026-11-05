//! 畫面下緣的介面：血條、幀計數，以及血條重畫時順帶做的完整性檢查。

use crate::game::Core;

/// 血條所在的像素列。
const BAR_ROW: u16 = 0xBE;
/// 玩家 1 的血條從右往左畫，第一格的欄；玩家 2 從左往右畫。
const BAR_FIRST_COL: [u16; 2] = [0x22, 6];
/// 一條血條的格數：血量的低 8 位元各一格。
const BAR_CELLS: u32 = 8;
/// 亮格與暗格的色號。
const CELL_LIT: u16 = 0x0B;
const CELL_DARK: u16 = 0;
/// 完整性檢查通過時，檢查函式把 `ax` 的低位元組改成這個值（`sub_11375` 把自己的第一個位元組換成 `retn`）。
const INTEGRITY_PATCH_BYTE: u8 = 0xC3;
/// 完整性檢查的通過條件：`sub_12F95` 累加的和。
const INTEGRITY_SUM: u16 = 0x227C;
/// 通過時寫進 `byte_1CFAA` 的值（`dl`）。
const INTEGRITY_MARK: u8 = b'Y';

/// 幀計數的位置：欄與列。
const FRAME_COUNTER_COL: u16 = 0x12;
const FRAME_COUNTER_ROW: u16 = 0xBF;

impl Core {
    /// 玩家（0 是玩家 1，1 是玩家 2）的血條，血量的低 8 位元由低到高各畫一格（原版 `sub_11336`、`sub_1130D`）。
    /// 玩家 1 的條在右側，往左畫；玩家 2 的在左側，往右畫。只在受擊時重畫（`Core::take_hit`）。
    ///
    /// 每一格畫之前先過 [`Core::integrity_check`]。檢查第一次執行時改寫 `ax`，而畫格的顏色取自 `al`
    /// 的位元 0，所以整局第一次畫血條的第一格，顏色取自檢查留下的值而不是血量的位元。
    pub(crate) fn draw_health_bar(&mut self, who: usize) {
        let step = if who == 0 { 1u16.wrapping_neg() } else { 1 };
        self.set_blit_x(BAR_FIRST_COL[who]);
        self.set_blit_y(BAR_ROW);
        let bits = self.pl.ship[who].hp & 0x00FF;
        for cell in 0..BAR_CELLS {
            let al = self.integrity_check().unwrap_or((bits >> cell) as u8);
            self.clear_obj();
            self.draw_bar_cell(if al & 1 == 0 { CELL_DARK } else { CELL_LIT });
            self.add_blit_x(step);
        }
    }

    /// 幀計數：5 位數字，用 BIOS 的 8×8 字型（原版 `sub_1100A`）。
    pub(crate) fn draw_frame_counter(&mut self) {
        self.set_text_position(FRAME_COUNTER_COL, FRAME_COUNTER_ROW);
        self.clear_obj();
        self.draw_number(self.stage_frame());
    }

    /// 完整性檢查（原版 `sub_11375`）：累加資料段 `7F4h` 起 400 個字組，和等於 [`INTEGRITY_SUM`] 才通過。
    /// 通過時把自己改成 `retn`，之後不再檢查，並在 `byte_1CFAA` 記下 `Y`（第二關 `sub_1316C` 讀它）。
    /// 不通過（資料段不是原版）時原版跳到 `sub_113B2`，輸入雜湊已驗過，正常流程走不到；這裡照原版不改旗標，
    /// 下次再檢查。
    ///
    /// 回傳檢查留在 `ax` 的低位元組：已經通過過了就是 `None`（函式一進來就返回，不碰 `ax`）；
    /// 剛通過是 [`INTEGRITY_PATCH_BYTE`]；沒通過是累加和的低位元組。
    fn integrity_check(&mut self) -> Option<u8> {
        if self.integrity_checked {
            return None;
        }
        let sum = self.tables.integrity_sum;
        if sum != INTEGRITY_SUM {
            return Some(sum as u8);
        }
        self.integrity_checked = true;
        self.flow.integrity_mark = INTEGRITY_MARK;
        Some(INTEGRITY_PATCH_BYTE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{BlitCmd, DrawCmd, Op};
    use crate::game::{Fixes, Start};

    fn core() -> Core {
        let mut c = Core::for_test(Start::Demo, Fixes::NONE);
        c.draws.clear();
        c
    }

    /// 讓完整性檢查通過：表格的累加和剛好等於 [`INTEGRITY_SUM`]。
    fn pass_the_integrity_check(c: &mut Core) {
        c.tables_mut().integrity_sum = INTEGRITY_SUM;
    }

    fn cells(c: &Core) -> Vec<BlitCmd> {
        c.draws.iter().filter_map(|d| if let DrawCmd::Blit(b) = d { Some(*b) } else { None }).filter(|b| b.op == Op::S1929F).collect()
    }

    #[test]
    fn player_one_fills_from_the_right_and_player_two_from_the_left_one_cell_per_bit() {
        let mut c = core();
        pass_the_integrity_check(&mut c);
        c.integrity_checked = true;
        c.pl.ship[0].hp = 0b0000_0101;
        c.pl.ship[1].hp = 0b1000_0010;
        c.draw_health_bar(0);
        c.draw_health_bar(1);
        let blits = cells(&c);
        assert_eq!(blits.len(), 16);
        let (p1, p2) = blits.split_at(8);
        assert_eq!(p1.iter().map(|b| b.x).collect::<Vec<_>>(), [0x22, 0x21, 0x20, 0x1F, 0x1E, 0x1D, 0x1C, 0x1B], "玩家 1 往左");
        assert_eq!(p2.iter().map(|b| b.x).collect::<Vec<_>>(), [6, 7, 8, 9, 10, 11, 12, 13], "玩家 2 往右");
        assert!(blits.iter().all(|b| b.y == 0xBE));
        let lit = |part: &[BlitCmd]| part.iter().map(|b| b.dx == 0x0B).collect::<Vec<_>>();
        assert_eq!(lit(p1), [true, false, true, false, false, false, false, false], "血量位元 0 在最右邊那一格");
        assert_eq!(lit(p2), [false, true, false, false, false, false, false, true]);
        assert!(blits.iter().all(|b| b.dx == 0 || b.dx == 0x0B));
    }

    #[test]
    fn the_very_first_bar_cell_is_lit_by_the_integrity_patch_whatever_the_hp_bit() {
        let mut c = core();
        pass_the_integrity_check(&mut c);
        c.pl.ship[0].hp = 0b0000_0010;
        c.draw_health_bar(0);
        assert!(c.integrity_checked, "通過後把自己改成 retn");
        assert_eq!(c.flow.integrity_mark, b'Y', "byte_1CFAA 記下 Y");
        let lit: Vec<bool> = cells(&c).iter().map(|b| b.dx == 0x0B).collect();
        assert_eq!(lit, [true, true, false, false, false, false, false, false], "位元 0 是 0，但第一格被檢查留下的 C3h 點亮");
        c.draws.clear();
        c.draw_health_bar(0);
        let lit: Vec<bool> = cells(&c).iter().map(|b| b.dx == 0x0B).collect();
        assert_eq!(lit, [false, true, false, false, false, false, false, false], "第二次起照血量");
    }

    #[test]
    fn a_failed_check_leaves_the_sum_in_al_for_every_cell_and_is_retried_each_time() {
        // 表格不是原版（累加和不對）：每格取檢查留下的累加和低位元組，不管血量。
        let mut c = core();
        c.pl.ship[0].hp = 0xFF;
        let sum = c.tables.integrity_sum;
        assert_ne!(sum, INTEGRITY_SUM);
        c.draw_health_bar(0);
        assert!(!c.integrity_checked);
        assert!(cells(&c).iter().all(|b| (b.dx == 0x0B) == (sum & 1 == 1)));
        assert_eq!(c.flow.integrity_mark, 0, "沒有通過就不記");
    }

    #[test]
    fn the_frame_counter_prints_the_frame_at_its_fixed_position() {
        let mut c = core();
        c.set_stage_frame(1234);
        c.draw_frame_counter();
        let blit = c.draws.iter().find_map(|d| if let DrawCmd::Blit(b) = d { Some(*b) } else { None }).unwrap();
        assert_eq!((blit.op, blit.cx, blit.text_x, blit.text_y), (Op::S1890D, 1234, 0x12, 0xBF));
    }
}
