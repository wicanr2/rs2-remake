//! 第四關：進入、每輪流程、普通段與頭目段的一幀、通道背景。原版是 `sub_13DAF`、`sub_13E74`、`sub_13EAB`、`sub_13EF9`、
//! `sub_13F7A`。
//!
//! 只移植 E 模式（`byte_1CF74` = `E`、`byte_1CF75` = `N`），顯示模式由 `rs2-gfx` 轉換。
//! 緩衝區配置與釋放的簿記見 `mirror.rs` 的 `StageBuffers`。

use crate::draw::{MixFile, Pool, ShtGroup};
use crate::entity::Timetable;
use crate::flow::{ChainShift, STAGE_4};
use crate::game::{Core, Exit, sample};
use crate::flow::{Palette, StageBuffers};
use crate::video::{Page, RowCopy, SCRATCH_A};

/// 計數不超過這個值走普通段，之後走頭目段。
const NORMAL_LAST_FRAME: u16 = 0x3E8;
/// 頭目段開始後的這一幀清暫存區 A 並畫 `SHT4` 第 29 項（16×1 的佔位，沒有可見內容）。
const PLACEHOLDER_FRAME: u16 = 0x3EA;
/// 計數超過這個值，15 個實體槽全空就過關（頭目被打倒）。
const CLEAR_CHECK_FRAME: u16 = 0x426;

/// 通道壁的水平相位循環：0 到 27h。
const PHASE_LAST: u16 = 0x27;
/// 下壁與上壁的基準像素列（加上 `Tunnel::offset`）、複製的列數。
const FLOOR_ROW: u16 = 0x96;
const CEILING_ROW: u16 = 0x28;
const WALL_ROWS: u16 = 0x64;
/// 每幀以色號 0 清滿的列數（實際只寫到列 189）。
const CLEAR_ROWS: u16 = 0xC8;
/// 頭目段的背景：暫存區 A（黑底）複製到畫面的列數。
const BOSS_BACKDROP_ROWS: u16 = 0xC8;
/// 腳本指令一步的上下位移（像素列）。
const WOBBLE_STEP: u16 = 0x0A;

/// 通道背景的狀態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tunnel {
    /// `word_1D91D`：兩壁一起上下偏移的像素列數，由腳本逐幀 ±10。
    pub offset: u16,
    /// `word_1D91F`：下壁的水平相位，每幀加 1（8 像素），到 40 歸零。
    pub floor_phase: u16,
    /// `word_1D921`：上壁的水平相位，初值 12h，與下壁差 18 位元組。
    pub ceiling_phase: u16,
    /// `word_1D9A2`：腳本讀取位置，從 0 起。
    pub script_pos: u16,
}

impl Tunnel {
    /// 進關卡的初值。
    pub(crate) fn restart(&mut self) {
        *self = Tunnel { offset: 0, floor_phase: 0, ceiling_phase: 0x12, script_pos: 0 };
    }

    /// 相位加 1，大於等於 27h 歸零。
    fn advance_phase(phase: u16) -> u16 {
        if phase >= PHASE_LAST { 0 } else { phase + 1 }
    }

    /// 腳本的一個位元組（原版 `sub_14014`，讀取位置先加 1）：1 讓偏移減 10、2 加 10、0Fh 把讀取位置退回
    /// （停在結尾），其餘不動。
    fn apply_script_byte(&mut self, byte: u8) {
        match byte {
            1 => self.offset = self.offset.wrapping_sub(WOBBLE_STEP),
            2 => self.offset = self.offset.wrapping_add(WOBBLE_STEP),
            0x0F => self.script_pos = self.script_pos.wrapping_sub(1),
            _ => {}
        }
    }
}

impl Core {
    /// 第四關完整入口，含載入（原版 `sub_13DAF`）。
    pub(crate) async fn play_stage4(&mut self) -> Exit {
        self.begin_stage_field();
        self.dos_allocate_buffers(StageBuffers::Stage4);
        self.dos_free_buffers(StageBuffers::Stage4);
        self.show_page(Page::One);
        self.draw_on(Page::One);
        self.clear_scratch_b();
        self.clear_both_pages();
        self.compose_stage4_backdrop();
        self.load_palette(Palette::Stage4);
        self.scenes.tunnel.restart();
        self.show_page(Page::One);
        self.reset_stage();
        self.set_stage_frame(1);
        self.flow.stage = STAGE_4;
        self.load_mix(MixFile::B);
        self.start_stage_music();
        let exit = loop {
            self.show_page(Page::Zero);
            self.stage4_frame().await;
            self.show_page(Page::One);
            self.stage4_frame().await;
            let exit = self.poll_hotkeys();
            self.clear_when_arena_is_empty();
            if let Some(exit) = exit {
                break exit;
            }
            if !self.flow.is_cleared() {
                continue;
            }
            if let Some(exit) = self.clear_burst_round(ChainShift::BeforeLimit) {
                break exit;
            }
        };
        self.end_stage(exit)
    }

    /// 通道壁寫進暫存區 A 兩份，再載入 `SHT4`（原版 `sub_13D70`）：`BH4` 第 0 項，x = 0 與 28h。
    ///
    /// 戰場加大時（規格 005）多畫一份（x = 50h）：壁的水平相位最大 39，視窗要從相位取 `cols` 欄。
    fn compose_stage4_backdrop(&mut self) {
        self.load_sht_bank(ShtGroup::Bh4);
        self.clear_scratch_a();
        self.select_scratch(SCRATCH_A);
        self.draw_tile_to_scratch(0, 0, 0);
        self.draw_tile_to_scratch(0x28, 0, 0);
        if self.layout().wide() {
            self.draw_tile_to_scratch(0x50, 0, 0);
        }
        self.load_sht_bank(ShtGroup::Sht4);
    }

    /// 依幀計數選普通段或頭目段（原版 `sub_13E74`）。
    async fn stage4_frame(&mut self) {
        if self.stage_frame() <= NORMAL_LAST_FRAME {
            self.stage4_normal_frame().await;
            return;
        }
        self.stage4_boss_frame().await;
        if self.stage_frame() != PLACEHOLDER_FRAME {
            return;
        }
        self.clear_scratch_a();
        self.select_scratch(SCRATCH_A);
        self.draw_tile_to_scratch(0x14, 0, 0x1D);
    }

    /// 普通段的一幀（原版 `sub_13EAB`），背景是通道。`13ED5h` 的 `cmp word_26872, 0` 與 `jz short $+2` 沒有效果。
    async fn stage4_normal_frame(&mut self) {
        self.select_scratch(SCRATCH_A);
        self.spawn_scheduled(Timetable::Stage4);
        self.update_enemies();
        self.update_player(0);
        self.update_player(1);
        self.draw_stage4_tunnel();
        self.step_shots_and_draw_players();
        self.select_scratch(SCRATCH_A);
        self.close_frame(sample::STAGE4_NORMAL).await;
    }

    /// 頭目段的一幀（原版 `sub_13EF9`），背景是全黑。
    async fn stage4_boss_frame(&mut self) {
        self.select_scratch(SCRATCH_A);
        self.spawn_scheduled(Timetable::Stage4);
        self.update_enemies();
        self.update_player(0);
        self.update_player(1);
        self.draw_stage4_boss_backdrop();
        self.step_shots_and_draw_players();
        self.select_scratch(SCRATCH_A);
        self.close_frame(sample::STAGE4_BOSS).await;
    }

    /// 計數超過 426h 且 15 個實體槽全空，就過關（原版 `sub_13F47`）。
    fn clear_when_arena_is_empty(&mut self) {
        if self.stage_frame() > CLEAR_CHECK_FRAME && self.en.slots.iter().all(|slot| slot.link == 0) {
            self.mark_stage_cleared();
        }
    }

    /// 通道背景（原版 `sub_13F7A`）：先讀腳本移動偏移，以色號 0 清滿，再畫下壁與垂直鏡射的上壁。
    fn draw_stage4_tunnel(&mut self) {
        self.step_tunnel_script();
        self.set_obj(Pool::Background, 0);
        self.set_blit_x(0);
        self.set_blit_y(0);
        self.fill_rows(CLEAR_ROWS, 0);
        let t = self.scenes.tunnel;
        self.set_obj(Pool::Background, 1);
        self.set_blit_source(0, 0);
        self.set_blit_x(0);
        self.set_blit_y(FLOOR_ROW.wrapping_add(t.offset));
        self.set_blit_shift(t.floor_phase);
        self.copy_scratch_rows(RowCopy::Replace, WALL_ROWS);
        self.set_obj(Pool::Background, 2);
        self.set_blit_source(0, 0);
        self.set_blit_x(0);
        self.set_blit_y(CEILING_ROW.wrapping_add(t.offset));
        self.set_blit_shift(t.ceiling_phase);
        self.copy_scratch_rows(RowCopy::ReplaceMirrored, WALL_ROWS);
        self.clear_obj();
        let t = &mut self.scenes.tunnel;
        t.floor_phase = Tunnel::advance_phase(t.floor_phase);
        t.ceiling_phase = Tunnel::advance_phase(t.ceiling_phase);
    }

    /// 腳本讀一個位元組（讀取位置先加 1）並套用。
    fn step_tunnel_script(&mut self) {
        let pos = self.scenes.tunnel.script_pos;
        self.scenes.tunnel.script_pos = pos.wrapping_add(1);
        let byte = self.tables.tunnel_script_byte(pos);
        self.scenes.tunnel.apply_script_byte(byte);
    }

    /// 頭目段的背景（原版 `sub_1403E`）：暫存區 A 第 0 列起 200 列複製到畫面。暫存區 A 在幀 3EAh 之後已清成黑底。
    fn draw_stage4_boss_backdrop(&mut self) {
        self.set_obj(Pool::Background, 3);
        self.set_blit_source(0, 0);
        self.set_blit_x(0);
        self.set_blit_y(0);
        self.set_blit_shift(0);
        self.copy_scratch_rows(RowCopy::Replace, BOSS_BACKDROP_ROWS);
        self.clear_obj();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tunnel_script_moves_both_walls_and_the_end_byte_holds_the_position() {
        let mut t = Tunnel::default();
        t.apply_script_byte(1);
        assert_eq!(t.offset, 0xFFF6, "1 減 10");
        t.apply_script_byte(2);
        t.apply_script_byte(2);
        assert_eq!(t.offset, 10, "2 加 10");
        t.apply_script_byte(0);
        t.apply_script_byte(9);
        assert_eq!(t.offset, 10, "其餘不動");
        t.script_pos = 76;
        t.apply_script_byte(0x0F);
        assert_eq!(t.script_pos, 75, "結尾碼把讀取位置退回，下一次讀還是它");
    }

    #[test]
    fn the_wall_phases_wrap_after_27h() {
        assert_eq!(Tunnel::advance_phase(0x26), 0x27);
        assert_eq!(Tunnel::advance_phase(0x27), 0);
        assert_eq!(Tunnel::advance_phase(0x1234), 0, "超出循環的值也歸零");
    }

    #[test]
    fn restart_puts_the_ceiling_18_bytes_ahead_of_the_floor() {
        let mut t = Tunnel { offset: 5, floor_phase: 6, ceiling_phase: 7, script_pos: 8 };
        t.restart();
        assert_eq!(t, Tunnel { offset: 0, floor_phase: 0, ceiling_phase: 0x12, script_pos: 0 });
    }
}
