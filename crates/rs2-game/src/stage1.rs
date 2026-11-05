//! 第一關：進入、每輪流程、普通段與頭目段的一幀。原版是 `sub_10CD0`、`sub_10EEA`、`sub_10F41`、`sub_10F9F`。
//!
//! 第一關每幀的呼叫順序和第二、三、四關不同：玩家 1 的更新排在敵人更新之後、背景之前，玩家 1 的繪製與玩家 2 的
//! 更新、繪製排在子彈更新之後。順序決定共用池（玩家彈槽、特效槽、敵彈池連結）的槽號，不能合併。

mod background;

pub use background::Backdrop;

use crate::draw::{MixFile, ShtGroup};
use crate::entity::Timetable;
use crate::flow::{ChainShift, FRAME_TICKS, STAGE_1, STAGE_RUNNING};
use crate::game::{Core, Exit, Req, sample};
use crate::loader::DemoFile;
use crate::flow::Palette;
use crate::video::Page;

/// 計數 1：換成第一關的調色盤。
const MAIN_PALETTE_FRAME: u16 = 1;
/// 計數 26Ch：換成頭目調色盤。這一幀仍走普通段。
const BOSS_PALETTE_FRAME: u16 = 0x26C;
/// 計數超過 26Ch 起走頭目段；頭目段結束時計數等於 26Eh，就把頭目合成圖寫進暫存區 B。
const BOSS_ART_FRAME: u16 = 0x26E;
/// 計數 28Ah 起釘在 288h，在 288h 與 289h 之間來回（取樣點只看到 649、650 交替）。
const FRAME_CEILING: u16 = 0x28A;
const FRAME_PINNED: u16 = 0x288;
/// 展示模式整關重播的條件：入口計數等於這個值。不可達（頭目段結束時計數被釘在 288h 以下，見上），保留原版判斷。
const DEMO_RESTART_FRAME: u16 = 0x28A;

/// 橫幅（`MY` 第 26 項）的位置與圖：欄 27、像素列 172。普通段才畫。
const BANNER: (u16, u16, u16) = (0x1B, 0xAC, 0x1A);
/// 普通段每幀設的喇叭音效旗標（`byte_2684F`），選第三組參數。
const SPEAKER_FLAGS: u8 = 4;

impl Core {
    /// 第一關的初始化，不含檔案載入（原版 `10CD0h` 至 `10D11h`）。`Core::new`、`Game::restart` 用。
    pub(crate) fn init_stage1(&mut self) {
        self.draw_on(Page::Zero);
        self.draw_on(Page::One);
        self.banks.sht = ShtGroup::Bh1;
        self.compose_stage1_backdrop();
        self.draw_on(Page::One);
        self.banks.sht = ShtGroup::Sht1;
        self.flow.stage = STAGE_1;
        self.load_palette(Palette::Loader);
        self.reset_stage();
        self.flow.clear = STAGE_RUNNING;
        self.fx.reset_chain();
    }

    /// 第一關完整入口，含載入（原版 `sub_10CD0`）。
    pub(crate) async fn play_stage1(&mut self) -> Exit {
        self.show_page(Page::Zero);
        self.draw_on(Page::Zero);
        self.show_page(Page::One);
        self.draw_on(Page::One);
        self.load_sht_bank(ShtGroup::Bh1);
        self.compose_stage1_backdrop();
        self.draw_on(Page::One);
        self.load_sht_bank(ShtGroup::Sht1);
        self.load_demo(DemoFile::Dm1);
        self.flow.stage = STAGE_1;
        self.clear_both_pages();
        self.load_palette(Palette::Loader);
        self.reset_stage();
        self.flow.clear = STAGE_RUNNING;
        self.fx.reset_chain();
        if self.started_from_title() {
            self.load_mix(MixFile::B);
            self.start_stage_music();
        }
        self.run_stage1_rounds().await
    }

    /// 每一輪畫兩頁（`10D29h` 至 `10D8Bh`），每頁一幀。輪尾檢查暫停、熱鍵、過關連爆。
    pub(crate) async fn run_stage1_rounds(&mut self) -> Exit {
        loop {
            self.show_page(Page::Zero);
            self.count_down_tamper_timer();
            self.stage1_frame().await;
            self.show_page(Page::One);
            self.port.borrow_mut().at_round_end = true;
            self.stage1_frame().await;
            self.port.borrow_mut().at_round_end = false;
            self.pause_if_requested().await;
            if let Some(exit) = self.poll_hotkeys() {
                return self.end_stage(exit);
            }
            if self.flow.is_cleared()
                && let Some(exit) = self.clear_burst_round(ChainShift::BeforeLimit)
            {
                return self.end_stage(exit);
            }
            self.suspend(Req::Checkpoint).await;
        }
    }

    /// 一幀：依幀計數換調色盤，走普通段或頭目段（原版 `sub_10EEA`）。
    async fn stage1_frame(&mut self) {
        if self.demo_playing() && self.stage_frame() == DEMO_RESTART_FRAME {
            self.reset_stage();
        }
        if self.stage_frame() == MAIN_PALETTE_FRAME {
            self.load_palette(Palette::Stage1);
        }
        if self.stage_frame() == BOSS_PALETTE_FRAME {
            self.load_palette(Palette::Stage1Boss);
        }
        if self.stage_frame() <= BOSS_PALETTE_FRAME {
            self.stage1_normal_frame().await;
            return;
        }
        self.stage1_boss_frame().await;
        if self.stage_frame() == BOSS_ART_FRAME {
            self.compose_stage1_boss_art();
        } else if self.stage_frame() == FRAME_CEILING {
            self.set_stage_frame(FRAME_PINNED);
        }
    }

    /// 普通段的一幀（原版 `sub_10F41`）。
    async fn stage1_normal_frame(&mut self) {
        self.spawn_scheduled(Timetable::Stage1);
        self.spawn_scenery();
        self.update_enemies();
        self.update_player(0);
        self.draw_stage1_backdrop();
        self.draw_scenery();
        self.draw_enemies();
        self.update_enemy_bullets();
        self.update_player_bullets();
        self.draw_player(0);
        self.update_player(1);
        self.draw_player(1);
        self.draw_enemy_overlays();
        self.update_effects();
        self.draw_frame_counter();
        self.advance_stage_frame();
        self.end_of_frame_copy();
        self.clear_grid1();
        self.clear_grid2();
        self.draw_banner();
        self.set_speaker_flags(SPEAKER_FLAGS);
        self.apply_speaker_flags();
        self.suspend(Req::Sample(sample::STAGE1_NORMAL)).await;
        self.await_timer_ticks(FRAME_TICKS).await;
    }

    /// 頭目段的一幀（原版 `sub_10F9F`）：背景換成頭目合成圖，不畫橫幅與喇叭音效。
    async fn stage1_boss_frame(&mut self) {
        self.spawn_scheduled(Timetable::Stage1);
        self.spawn_scenery();
        self.update_enemies();
        self.update_player(0);
        self.draw_stage1_boss_backdrop();
        self.draw_enemies();
        self.draw_enemy_overlays();
        self.draw_scenery();
        self.update_enemy_bullets();
        self.update_player_bullets();
        self.draw_player(0);
        self.update_player(1);
        self.draw_player(1);
        self.update_effects();
        self.draw_frame_counter();
        self.advance_stage_frame();
        self.end_of_frame_copy();
        self.clear_grid1();
        self.clear_grid2();
        self.suspend(Req::Sample(sample::STAGE1_BOSS)).await;
        self.await_timer_ticks(FRAME_TICKS).await;
    }

    /// 橫幅：`MY` 第 26 項（原版 `sub_18C0A`）。
    fn draw_banner(&mut self) {
        let (x, y, index) = BANNER;
        self.set_blit_params(x, y, index);
        self.clear_obj();
        self.overwrite_my();
    }
}
