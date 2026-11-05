//! 第三關：進入、每輪流程、普通段與頭目段的一幀。原版是 `sub_13A51`、`sub_13AFA`、`sub_13B0A`、`sub_13B75`。
//! 地圖捲動見 `stage3/map.rs`。
//!
//! 只移植 E 模式（`byte_1CF74` = `E`、`byte_1CF75` = `N`），顯示模式由 `rs2-gfx` 轉換。
//! 地圖 `SHT.SCN` 原版由 `sub_1BF71` 解壓到遠段，之後只有地圖捲動讀它，沒有任何程式寫入，所以重製在 `Core::map_seg`
//! 放一份解壓結果。

mod map;

pub(crate) use map::MapScroll;

use crate::boss_scroll::SCRIPT_END;
use crate::entity::Timetable;
use crate::flow::{ChainShift, STAGE_3, STAGE_3_BOSS, STAGE_RUNNING};
use crate::game::{Core, Exit, Req, sample};
use crate::mirror::{BossScript, Palette, SCRATCH_A, StageBuffers};
use crate::stage_io::StageMusic;

/// 計數超過這個值走頭目段。
const BOSS_FRAME: u16 = 0x469;
/// 頭目段計數等於這個值時換合成圖並把關卡字母改成小寫；頭目合成圖的捲動從這之後開始。
const BOSS_ART_FRAME: u16 = 0x46C;
/// 計數超過這個值就釘回這個值。
const FRAME_CEILING: u16 = 0x474;
/// 頭目合成圖的捲動腳本：起點 x = 40、y = 30，758 步。
const BOSS_START_Y: u16 = 0x1E;
const BOSS_START_X: u16 = 0x28;
/// 合成圖的列數。
const BOSS_ART_ROWS: u16 = 0xC1;

impl Core {
    /// 第三關完整入口，含載入與主迴圈（原版 `sub_13A51`）。`SHT.SCN` 的解壓在 [`Core::load_map`]。
    pub(crate) async fn play_stage3(&mut self) -> Exit {
        self.dos_allocate_buffers(StageBuffers::Stage3);
        self.dos_free_buffers(StageBuffers::Stage3);
        self.show_second_page();
        self.draw_to_second_page();
        self.load_map_sheet();
        self.load_sprite_sheet_3();
        self.load_map_file();
        self.load_map();
        self.clear_both_pages();
        self.load_palette(Palette::Stage3);
        self.show_second_page();
        self.flow.stage = STAGE_3;
        self.reset_stage();
        self.scenes.map.window = 0;
        self.flow.clear = STAGE_RUNNING;
        self.fx.reset_chain();
        self.load_stage_music(StageMusic::Stage3);
        self.start_stage_music();
        let exit = loop {
            self.show_first_page();
            self.stage3_frame().await;
            self.show_second_page();
            self.stage3_frame().await;
            if let Some(exit) = self.poll_hotkeys() {
                break exit;
            }
            if self.flow.is_cleared()
                && let Some(exit) = self.clear_burst_round(ChainShift::BeforeLimit)
            {
                break exit;
            }
            self.suspend(Req::Checkpoint).await;
        };
        self.end_stage(exit)
    }

    /// 計數超過 469h 走頭目段（原版 `sub_13AFA`）。
    async fn stage3_frame(&mut self) {
        if self.stage_frame() > BOSS_FRAME {
            self.stage3_boss_frame().await;
        } else {
            self.stage3_normal_frame().await;
        }
    }

    /// 普通段的一幀（原版 `sub_13B0A`）：第二頁（`word_26872` 非 0）走 `scroll_map_second_page`，
    /// 第一頁走 `scroll_map_first_page`，所以地圖每兩幀前進 1 欄。取樣點在幀計數加 1 之後。
    async fn stage3_normal_frame(&mut self) {
        self.select_scratch(SCRATCH_A);
        if self.draw_page() != 0 {
            self.scroll_map_second_page();
        } else {
            self.scroll_map_first_page();
        }
        self.draw_initial_map_on_first_frame();
        self.select_scratch(SCRATCH_A);
        self.spawn_scheduled(Timetable::Stage3);
        self.update_enemies();
        self.update_player(0);
        self.update_player(1);
        self.step_shots_and_draw_players();
        self.close_frame(sample::STAGE3_NORMAL).await;
    }

    /// 頭目段的一幀（原版 `sub_13B75`）：背景是頭目合成圖。計數到 46Ch 換合成圖並把關卡字母改成小寫 `c`
    /// （地形碰撞停用，`on_terrain_stage` 只認大寫），超過 474h 釘回 474h。
    async fn stage3_boss_frame(&mut self) {
        self.select_scratch(SCRATCH_A);
        self.spawn_scheduled(Timetable::Stage3);
        self.draw_stage3_boss_backdrop();
        self.update_enemies();
        self.update_player(0);
        self.update_player(1);
        self.step_shots_and_draw_players();
        self.close_frame(sample::STAGE3_BOSS).await;
        let frame = self.stage_frame();
        if frame == BOSS_ART_FRAME {
            self.compose_stage3_boss_art();
            self.flow.stage = STAGE_3_BOSS;
        } else if frame > FRAME_CEILING {
            self.set_stage_frame(FRAME_CEILING);
        }
    }

    /// 幀 0：視窗左緣歸 0，畫初始視窗（原版 `sub_13B64`）。幀 0 的頁是第二頁，同一幀稍早已經跑過一次
    /// `scroll_map_second_page`，所以初始視窗的左緣是地圖第 1 欄。
    fn draw_initial_map_on_first_frame(&mut self) {
        if self.stage_frame() != 0 {
            return;
        }
        self.scenes.map.window = 0;
        self.draw_initial_map();
    }

    /// 頭目前換圖（原版 `sub_13A32`）：清暫存區 A，把 `SHT3` 第 43 項（x = 0、y = 0）寫進去。
    fn compose_stage3_boss_art(&mut self) {
        self.clear_scratch_a();
        self.select_scratch(SCRATCH_A);
        self.draw_tile_to_scratch(0, 0, 0x2B);
    }

    /// 頭目段的背景（原版 `sub_15924`、`sub_15983`）：清頁，畫合成圖。計數不超過 46Ch 時每幀重設起點，
    /// 之後讀腳本逐幀移動，腳本走到結束碼就過關。
    fn draw_stage3_boss_backdrop(&mut self) {
        self.clear_draw_page();
        if self.stage_frame() <= BOSS_ART_FRAME {
            self.scenes.boss.layers[0].restart(BOSS_START_Y, BOSS_START_X, BossScript::Stage3);
            return;
        }
        let byte = self.step_boss_layer(0);
        self.draw_boss_layer(0, SCRATCH_A, 0, BOSS_ART_ROWS, 0);
        if byte == SCRIPT_END {
            self.mark_stage_cleared();
        }
    }
}
