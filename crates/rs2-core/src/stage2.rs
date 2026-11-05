//! 第二關：進入、每輪流程、兩段的一幀、調色盤與淡出。原版是 `sub_13205`、`sub_132BE`、`sub_132EB`、`sub_13344`。
//!
//! 只移植 E 模式（`byte_1CF74` = `E`、`byte_1CF75` = `N`）。CGA、MGA、純 EGA 的分支不在這裡：顯示模式由
//! `rs2-gfx` 轉換繪製命令（規格 006）。
//!
//! 防拷檢查有三處原版是無限迴圈或無限遞迴（`130AF`、`131C9`、`138F9`），資料段通過開機檢查後走不到，
//! 這裡改成 `panic!`，避免卡死整個程序。
//!
//! 第一段（幀計數不超過 104h，遞增前比較）背景是五層視差，第二段換成頭目合成圖。

mod boss;
mod parallax;

use crate::draw::{MixFile, ShtGroup};
use crate::entity::Timetable;
use crate::flow::{ChainShift, STAGE_2, STAGE_RUNNING};
use crate::game::{Core, Exit, sample};
use crate::loader::DemoFile;
use crate::flow::{Palette, StageBuffers};
use crate::video::{Page, SCRATCH_A, SCRATCH_B};

/// 第一段的最後一幀（遞增前的計數）。這一幀同時把中頭目合成圖寫進暫存區 B。
const OPENING_LAST_FRAME: u16 = 0x104;
/// 中頭目合成圖與頭目合成圖寫進暫存區 B 的幀（遞增前的計數）。
const MIDBOSS_ART_FRAME: u16 = 0x104;
const BOSS_ART_FRAME: u16 = 0x21C;
/// 第二段換調色盤的幀（遞增後的計數）：中頭目、頭目。
const MIDBOSS_PALETTE_FRAME: u16 = 0x106;
const BOSS_PALETTE_FRAME: u16 = 0x276;
/// 計數大於這個值之後，第二頁的幀尾讓色號 8 到 14 淡出一級。
const FADE_AFTER_FRAME: u16 = 0xA0;

/// 近景條的相位與起點：每幀相位減 1，用完起點左移一位元組。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Strip {
    pub phase: u16,
    pub start: u16,
}

/// 淡出工作表的位元組數：`stage2` 調色盤 DAC 表的前 45 位元組。
pub(crate) const FADE_BYTES: usize = 45;
/// 工作表裡被淡出的區段：色號 8 到 14，21 個位元組，從第 24 個位元組起。
const FADE_FIRST: usize = 24;

/// 第二關背景的狀態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backdrop {
    /// `word_1D7BE`、`word_1D7C0`：火焰面底端與頂端的像素列。
    pub flame_bottom: u16,
    pub flame_top: u16,
    /// `word_1D7C2`：火焰面的水平相位，0 到 28h。
    pub flame_phase: u16,
    /// `word_1D7C4`、`word_1D7C6`：近景條 A（相位 7 到 0，起點 8 到 1）。
    pub strip_a: Strip,
    /// `word_1D7C8`、`word_1D7CA`：近景條 B（相位 3 到 0，起點 10 到 1）。
    pub strip_b: Strip,
    /// `word_1D7CC`、`word_1D7CE`：近景條 C（相位 1 到 0，起點 20 到 1）。
    pub strip_c: Strip,
    /// `word_1D7D0`：柱的起點，20 到 1。
    pub pillar: u16,
    /// 淡出工作表（資料段 `0CA7h` 起 45 位元組）：`stage2` 調色盤的複本，淡出時色號 8 到 14 逐級減到 0。
    pub fade: [u8; FADE_BYTES],
}

impl Default for Backdrop {
    fn default() -> Backdrop {
        Backdrop {
            flame_bottom: 0,
            flame_top: 0,
            flame_phase: 0,
            strip_a: Strip::default(),
            strip_b: Strip::default(),
            strip_c: Strip::default(),
            pillar: 0,
            fade: [0; FADE_BYTES],
        }
    }
}

impl Core {
    /// 第二關完整入口，含載入（原版 `sub_13205`）。
    pub(crate) async fn play_stage2(&mut self) -> Exit {
        if self.password_rejected() {
            panic!("word_1D3B0 非 0：原版 jmp sub_13205 無限遞迴（防拷檢查未通過）");
        }
        self.dos_allocate_buffers(StageBuffers::Stage2);
        self.dos_free_buffers(StageBuffers::Stage2);
        self.show_page(Page::One);
        self.draw_on(Page::One);
        self.clear_scratch_b();
        self.compose_stage2_backdrop();
        self.load_demo(DemoFile::Dm2);
        self.reset_fade();
        self.clear_both_pages();
        self.load_palette(Palette::Stage2);
        self.show_page(Page::One);
        self.flow.stage = STAGE_2;
        self.reset_stage();
        self.flow.clear = STAGE_RUNNING;
        self.fx.reset_chain();
        self.load_mix(MixFile::B);
        self.start_stage_music();
        let exit = loop {
            self.show_page(Page::Zero);
            self.stage2_frame().await;
            self.show_page(Page::One);
            self.stage2_frame().await;
            if let Some(exit) = self.poll_hotkeys() {
                break exit;
            }
            if !self.flow.is_cleared() {
                continue;
            }
            if let Some(exit) = self.clear_burst_round(ChainShift::AfterLimit) {
                break exit;
            }
        };
        self.end_stage(exit)
    }

    /// 背景圖寫進暫存區 A 與 B，再載入 `SHT2`（原版 `sub_12FB6`）。
    ///
    /// 暫存區 A：`BH2` 第 0 至 3 項各兩份、第 4 項橫排 20 塊、第 5 至 12 項（8 個相位）、第 13 至 16 項（4 個相位）、
    /// 第 17、18 項（2 個相位）。暫存區 B：第 19 至 22 項（遮罩層的圖）、第 23 至 26 項（遮罩）、第 27 項（柱）。
    fn compose_stage2_backdrop(&mut self) {
        self.load_sht_bank(ShtGroup::Bh2);
        self.select_scratch(SCRATCH_A);
        for (x, y, index) in [(0, 0, 0), (0x28, 0, 0), (0, 0x22, 1), (0x28, 0x22, 1), (0, 0x44, 2), (0x28, 0x44, 2), (0, 0x5A, 3), (0x28, 0x5A, 3)] {
            self.draw_tile_to_scratch(x, y, index);
        }
        self.draw_tile_run(0, 0x70, 4, 0x14, false);
        // 第 5 至 12 項：原版每畫一張檢查 `byte_1D3A2` 必須是 `Y`，否則在 `130A2h` 無限迴圈。
        if !self.password_accepted() {
            panic!("byte_1D3A2 不是 Y：原版在 130A2 無限迴圈（防拷檢查未通過）");
        }
        self.draw_tile_run(0x28, 0x70, 5, 8, true);
        self.draw_tile_run(0x38, 0x70, 0x0D, 4, true);
        self.draw_tile_run(0x40, 0x70, 0x11, 2, true);
        self.select_scratch(SCRATCH_B);
        self.draw_tile_run(0, 0, 0x13, 4, true);
        self.draw_tile_run(8, 0, 0x17, 4, true);
        self.draw_tile_to_scratch(0x10, 0, 0x1B);
        self.load_sht_bank(ShtGroup::Sht2);
    }

    /// 同一列往右畫 `count` 張，每張佔 2 位元組；`next_index` 為真時圖庫索引逐張加 1（相位圖），否則同一張。
    fn draw_tile_run(&mut self, x: u16, y: u16, index: u16, count: u16, next_index: bool) {
        self.set_blit_params(x, y, index);
        for _ in 0..count {
            self.draw_current_tile_to_scratch();
            self.add_blit_x(2);
            if next_index {
                self.add_blit_index(1);
            }
        }
    }

    /// 依幀計數選第一段或第二段（原版 `sub_132BE`）。比較的是遞增前的計數；
    /// `132C3h` 的 `jnz short $+2` 兩個方向都落到下一道，所以對 1 的比較沒有效果。
    async fn stage2_frame(&mut self) {
        if self.stage_frame() == BOSS_ART_FRAME {
            self.compose_stage2_boss_art();
        }
        if self.stage_frame() == MIDBOSS_ART_FRAME {
            self.compose_stage2_midboss_art();
        }
        if self.stage_frame() <= OPENING_LAST_FRAME {
            self.stage2_opening_frame().await;
        } else {
            self.stage2_boss_frame().await;
        }
    }

    /// 第一段的一幀（原版 `sub_132EB`），背景是五層視差。
    async fn stage2_opening_frame(&mut self) {
        self.select_scratch(SCRATCH_A);
        self.spawn_scheduled(Timetable::Stage2);
        self.update_enemies();
        self.update_player(0);
        self.update_player(1);
        self.draw_stage2_floor();
        self.draw_stage2_strip_a();
        self.draw_stage2_strip_b();
        self.draw_stage2_strip_c();
        self.draw_stage2_pillars();
        self.step_shots_and_draw_players();
        self.draw_stage2_bars();
        self.draw_stage2_flames();
        self.close_frame(sample::STAGE2_OPENING).await;
        self.fade_after_opening_frame();
    }

    /// 第二段的一幀（原版 `sub_13344`），背景換成頭目合成腳本。幀尾依遞增後的計數換調色盤、疊嘴部動畫。
    async fn stage2_boss_frame(&mut self) {
        self.select_scratch(SCRATCH_A);
        self.spawn_scheduled(Timetable::Stage2);
        self.update_enemies();
        self.update_player(0);
        self.update_player(1);
        self.draw_stage2_boss_backdrop();
        self.step_shots_and_draw_players();
        self.draw_stage2_bars();
        self.draw_stage2_flames();
        self.close_frame(sample::STAGE2_BOSS).await;
        match self.stage_frame() {
            MIDBOSS_PALETTE_FRAME => {
                self.reset_fade();
                self.load_palette(Palette::Stage2Mid);
            }
            BOSS_PALETTE_FRAME => self.load_palette(Palette::Stage2Boss),
            frame if frame <= BOSS_PALETTE_FRAME => {}
            _ => self.animate_stage2_boss_mouth(),
        }
    }

    /// 第一段的幀尾（原版 `sub_13421`，比較遞增後的計數）：幀 1 重設淡出工作表並設調色盤；
    /// 計數大於 A0h 之後，第二頁（`word_26872` 非 0）淡出一級，幀 161、163、…、261 共 51 次。
    fn fade_after_opening_frame(&mut self) {
        let frame = self.stage_frame();
        if frame > FADE_AFTER_FRAME {
            if self.draw_page() != 0 {
                self.fade_out_one_step();
            }
        } else if frame == 1 {
            self.reset_fade();
            self.load_palette(Palette::Stage2);
        }
    }

    /// 淡出工作表回到 `stage2` 調色盤的樣子（原版 `sub_132AB`）。
    fn reset_fade(&mut self) {
        let dac = self.tables.palette(Palette::Stage2).dac;
        self.scenes.stage2.fade = std::array::from_fn(|k| dac[k]);
    }

    /// 用工作表設調色盤，再把色號 8 到 14 的 21 個位元組各減 1，到 0 為止（原版 `sub_13894`）。
    fn fade_out_one_step(&mut self) {
        let fade = self.scenes.stage2.fade;
        self.set_dac_overlay(0, Palette::Stage2Fade, &fade);
        self.set_dac_overlay(0x30, Palette::Stage2Fade, &fade);
        for level in &mut self.scenes.stage2.fade[FADE_FIRST..] {
            *level = level.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::ShtGroup;
    use crate::game::{Fixes, Start};

    fn core() -> Core {
        Core::for_test(Start::Demo, Fixes::NONE)
    }

    #[test]
    fn the_fade_table_is_the_first_45_bytes_of_the_stage2_palette() {
        let mut c = core();
        let dac: Vec<u8> = (1..=48).collect();
        c.tables_mut().put_dac(Palette::Stage2, 0, &dac);
        c.reset_fade();
        assert_eq!(c.scenes.stage2.fade[0], 1);
        assert_eq!(c.scenes.stage2.fade[44], 45);
    }

    #[test]
    fn fading_lowers_the_21_bytes_from_the_24th_and_stops_at_zero() {
        let mut c = core();
        c.scenes.stage2.fade = [3; FADE_BYTES];
        c.scenes.stage2.fade[30] = 0;
        c.draws.clear();
        c.fade_out_one_step();
        let fade = c.scenes.stage2.fade;
        assert!(fade[..24].iter().all(|&v| v == 3), "前 24 個位元組不變");
        assert!(fade[24..].iter().enumerate().all(|(k, &v)| v == if k == 6 { 0 } else { 2 }));
        assert_eq!(c.draws.len(), 3, "第一次寫 DAC 附屬性表，第二次只有 DAC");
    }

    #[test]
    fn the_fade_palette_takes_its_last_three_bytes_from_the_table() {
        let mut c = core();
        c.tables_mut().put_dac(Palette::Stage2Fade, 45, &[0x80 + 45, 0x80 + 46, 0x80 + 47]);
        c.scenes.stage2.fade = [7; FADE_BYTES];
        c.draws.clear();
        c.fade_out_one_step();
        match c.draws[0] {
            crate::draw::DrawCmd::SetDac { start: 0, rgb } => {
                assert!(rgb[..45].iter().all(|&v| v == 7));
                assert_eq!(rgb[45..], [0x80 + 45, 0x80 + 46, 0x80 + 47]);
            }
            other => panic!("第一個命令應該是 SetDac：{other:?}"),
        }
    }

    #[test]
    fn the_midboss_art_hangs_in_the_original_until_the_player_has_been_hit() {
        let mut c = core();
        c.flow.integrity_mark = b'N';
        let hung = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.compose_stage2_midboss_art()));
        assert!(hung.is_err());
        c.flow.integrity_mark = b'Y';
        c.compose_stage2_midboss_art();
    }

    #[test]
    fn the_bg2_sheet_is_loaded_while_composing_the_backdrop() {
        let mut c = core();
        c.set_password_accepted(true);
        c.compose_stage2_backdrop();
        assert_eq!(c.banks.sht, ShtGroup::Sht2, "最後載入 SHT2");
    }
}
