//! 第一關的背景。普通段是五層視差（天空、波浪、倒懸岩、山影遮罩、山影圖），頭目段是一張大合成圖。
//! 原版是 `sub_16302` 一族、`sub_10B65`（背景圖寫進暫存區 A）、`sub_10C3D`（頭目圖寫進暫存區 B）、`sub_15906`。

use crate::boss_scroll::SCRIPT_END;
use crate::draw::Pool;
use crate::game::Core;
use crate::mirror::{BossScript, SCRATCH_A, SCRATCH_B};

/// 計數超過這個值，背景整體開始下移，並在天空下方補色。
const DROP_START_FRAME: u16 = 0xFA;
/// 計數超過這個值，天空層另以半速下移，上方補黑。
const SKY_DROP_START_FRAME: u16 = 0x113;
/// 水平相位的循環長度（欄）。
const PHASE_PERIOD: u16 = 0x28;
/// 頭目合成圖的捲動腳本，從幀 26Fh 開始走（之前每幀重設起點）。
const BOSS_SCRIPT_START_FRAME: u16 = 0x26E;

/// 五層背景的狀態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Backdrop {
    /// `word_1E6B8`：所有層一起往下移的像素列數，計數超過 250 後每幀加 1。
    pub drop: u16,
    /// `word_1E6BA`：天空層另外的下移，計數超過 275 後每兩幀加 1（只有第二頁加）。
    pub sky_drop: u16,
    /// `word_1E6BC`：波浪的水平相位，每幀加 2（16 像素），到 40 歸零。
    pub wave_phase: u16,
    /// `word_1E6BE`：倒懸岩與山影的水平相位，每幀加 1（8 像素），到 40 歸零。
    pub rock_phase: u16,
}

impl Backdrop {
    /// 相位加 `step`，大於等於循環長度歸零。
    fn advance_phase(phase: u16, step: u16) -> u16 {
        let next = phase.wrapping_add(step);
        if next >= PHASE_PERIOD { 0 } else { next }
    }
}

impl Core {
    /// 五層背景（原版 `sub_16302`），畫到目前頁。
    ///
    /// | 層 | 圖庫項 | 來源列 | 顯示列 | 畫法 |
    /// |---|---|---|---|---|
    /// | 天空 | `BH1` 第 1 項，橫排 | 6Eh | 天空下移 | 覆寫 |
    /// | 波浪 | 第 0 項 | 0 | 下移 + 95h | 覆寫，相位 `wave_phase` |
    /// | 倒懸岩 | 第 2 項 | 28h | 下移 + 52h | 覆寫，相位 `rock_phase` |
    /// | 山影遮罩 | 第 4 項 | F3h | 下移 + 28h | AND |
    /// | 山影圖 | 第 3 項 | C8h | 下移 + 28h | OR |
    pub(crate) fn draw_stage1_backdrop(&mut self) {
        let frame = self.stage_frame();
        let b = self.scenes.stage1;
        // 天空。
        self.set_obj(Pool::Background, 0);
        self.select_scratch(SCRATCH_A);
        self.set_blit_index(0x0F);
        self.set_blit_source(0, 0x6E);
        self.set_blit_x(0);
        self.set_blit_y(b.sky_drop);
        self.set_blit_shift(0);
        self.copy_rows_to_page(0x59);
        self.fill_below_sky(frame);
        // 波浪。
        self.set_obj(Pool::Background, 1);
        self.set_blit_source(0, 0);
        self.set_blit_x(0);
        self.set_blit_y(b.drop.wrapping_add(0x95));
        self.set_blit_shift(b.wave_phase);
        self.copy_rows_to_page(0x29);
        self.scenes.stage1.wave_phase = Backdrop::advance_phase(b.wave_phase, 2);
        // 倒懸岩。
        self.set_obj(Pool::Background, 2);
        self.set_blit_source(0, 0x28);
        self.set_blit_x(0);
        self.set_blit_y(b.drop.wrapping_add(0x52));
        self.set_blit_shift(b.rock_phase);
        self.copy_rows_to_page(0x45);
        // 山影遮罩，AND 清出山形。
        self.set_obj(Pool::Background, 3);
        self.set_blit_source(0, 0xF3);
        self.set_blit_x(0);
        self.set_blit_y(b.drop.wrapping_add(0x28));
        self.set_blit_shift(b.rock_phase);
        self.and_rows_to_page(0x2B);
        // 山影圖，OR 補上山形。
        self.set_obj(Pool::Background, 4);
        self.set_blit_source(0, 0xC8);
        self.set_blit_x(0);
        self.set_blit_y(b.drop.wrapping_add(0x28));
        self.set_blit_shift(b.rock_phase);
        self.or_rows_to_page(0x2B);
        self.clear_obj();
        self.scenes.stage1.rock_phase = Backdrop::advance_phase(b.rock_phase, 1);
        self.drop_the_backdrop(frame);
    }

    /// 計數超過 250 後，在天空下方以色號 1 填 `drop` 列（原版 `sub_1641B`）。
    fn fill_below_sky(&mut self, frame: u16) {
        if frame <= DROP_START_FRAME {
            return;
        }
        let b = self.scenes.stage1;
        self.set_blit_x(0);
        self.set_blit_y(0x59u16.wrapping_add(b.sky_drop));
        self.fill_rows(b.drop, 1);
    }

    /// 背景下移：計數不超過 250 時兩個下移量歸零；之後所有層下移 1 列；超過 275 後天空另加 1 列
    /// （只有第二頁加），並用色號 0 填上方 `sky_drop` 列。填色的列數用加完的值，天空繪製用加之前的值。
    fn drop_the_backdrop(&mut self, frame: u16) {
        if frame <= DROP_START_FRAME {
            self.scenes.stage1.drop = 0;
            self.scenes.stage1.sky_drop = 0;
            return;
        }
        self.scenes.stage1.drop = self.scenes.stage1.drop.wrapping_add(1);
        if frame <= SKY_DROP_START_FRAME {
            return;
        }
        if self.draw_page() != 0 {
            self.scenes.stage1.sky_drop = self.scenes.stage1.sky_drop.wrapping_add(1);
        }
        self.set_blit_x(0);
        self.set_blit_y(0);
        self.fill_rows(self.scenes.stage1.sky_drop, 0);
    }

    /// 背景圖寫進暫存區 A（原版 `sub_10B65`）：`BH1` 的第 0 項（波浪）兩份、第 2 項（倒懸岩）兩份、
    /// 第 1 項（天空）十塊、第 3 項（山影圖）兩份、第 4 項（山影遮罩）兩份。
    pub(crate) fn compose_stage1_backdrop(&mut self) {
        self.select_scratch(SCRATCH_A);
        self.draw_to_first_page();
        for (x, y, index) in [(0, 0, 0), (0x28, 0, 0), (0, 0x28, 2), (0x28, 0x28, 2)] {
            self.draw_tile_to_scratch(x, y, index);
        }
        self.set_blit_x(0);
        for _ in 0..10 {
            self.set_blit_y(0x6E);
            self.set_blit_index(1);
            self.draw_current_tile_to_scratch();
            self.add_blit_x(4);
        }
        for (x, y, index) in [(0, 0xC8, 3), (0x28, 0xC8, 3), (0, 0xF3, 4), (0x28, 0xF3, 4)] {
            self.draw_tile_to_scratch(x, y, index);
        }
    }

    /// 頭目合成圖寫進暫存區 B（原版 `sub_10C3D`）：清暫存區 B，`SHT1` 第 33、34、35 項。
    pub(crate) fn compose_stage1_boss_art(&mut self) {
        self.clear_obj();
        self.clear_scratch_b();
        self.select_scratch(SCRATCH_B);
        for (x, y, index) in [(0, 0, 0x21), (0x12, 0x22, 0x22), (0x26, 0x0D, 0x23)] {
            self.draw_tile_to_scratch(x, y, index);
        }
    }

    /// 頭目段的背景（原版 `sub_15906`、`sub_15930`）：清頁，畫頭目合成圖。計數不超過 26Eh 時每幀重設起點，
    /// 之後讀腳本逐幀移動，腳本走到結束碼就過關。
    pub(crate) fn draw_stage1_boss_backdrop(&mut self) {
        self.clear_draw_page();
        if self.stage_frame() <= BOSS_SCRIPT_START_FRAME {
            self.scenes.boss.layers[0].restart(0xC8, 0xFFEC, BossScript::Stage1);
            return;
        }
        let byte = self.step_boss_layer(0);
        self.draw_boss_layer(0, SCRATCH_B, 0, 0xC1, 0);
        if byte == SCRIPT_END {
            self.mark_stage_cleared();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_wrap_to_zero_at_the_period() {
        assert_eq!(Backdrop::advance_phase(0x26, 2), 0);
        assert_eq!(Backdrop::advance_phase(0x24, 2), 0x26);
        assert_eq!(Backdrop::advance_phase(0x27, 1), 0);
        assert_eq!(Backdrop::advance_phase(0x50, 1), 0, "超出循環的值（例如 poke）下一幀也歸零");
    }
}
