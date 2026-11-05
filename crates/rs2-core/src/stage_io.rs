//! 關卡進出用的載入與清除呼叫。
//!
//! 載入（`loader.rs`）與繪製（`video.rs`）還沒重寫，函式仍以原版位址命名。關卡程式經這裡的具名函式呼叫，
//! 讀得懂在做什麼；載入與繪製領域重寫時，只需要改這個檔案。檔名對照見 `docs/re/02-file-formats.md`。

use crate::game::Core;

impl Core {
    // ---- 清除 ----

    /// 清除目前的繪製頁（原版 `sub_19CF9`，190 列）。頭目段的背景函式第一件事。
    pub(crate) fn clear_draw_page(&mut self) {
        self.s19cf9();
    }

    /// 清暫存區 A（原版 `sub_11504`）。
    pub(crate) fn clear_scratch_a(&mut self) {
        self.s11504();
    }

    /// 清暫存區 B（原版 `sub_1152D`）。
    pub(crate) fn clear_scratch_b(&mut self) {
        self.s1152d();
    }

    /// 兩個顯示頁都清成黑（原版 `sub_18894`）。
    pub(crate) fn clear_both_pages(&mut self) {
        self.s18894();
    }

    /// 清第三關的地形格 G4（原版 `sub_18599`）。每個關卡進場都清。
    pub(crate) fn clear_terrain_grid(&mut self) {
        self.s18599();
    }

    /// 一幀畫完、翻頁之前的頁複製（原版 `sub_1117F`）。只有 MGA（Hercules）有動作，位置見 `docs/re/14-display-modes.md`。
    pub(crate) fn end_of_frame_copy(&mut self) {
        self.s1117f();
    }

    /// 依喇叭音效旗標（`Core::set_speaker_flags`）設喇叭參數（原版 `sub_18672`）。
    pub(crate) fn apply_speaker_flags(&mut self) {
        self.s18672();
    }

    /// `MY` 圖庫的圖覆寫到顯示頁，位置與圖索引取自繪製參數（原版 `sub_18C0A`）。
    pub(crate) fn draw_my_image_overwrite(&mut self) {
        self.s18c0a();
    }

    // ---- 載入：圖庫 ----

    /// `BH1.GO`，第一關背景。
    pub(crate) fn load_backdrop_sheet_1(&mut self) {
        self.s1af3b();
    }

    /// `BH2.GO`，第二關背景。
    pub(crate) fn load_backdrop_sheet_2(&mut self) {
        self.s1af66();
    }

    /// `BH4.GO`，第四關背景。
    pub(crate) fn load_backdrop_sheet_4(&mut self) {
        self.s1af91();
    }

    /// `SHT1.GO` 到 `SHT4.GO`：各關的實體、子彈與頭目圖庫（遮罩一起換）。
    pub(crate) fn load_sprite_sheet_1(&mut self) {
        self.s1ae3b();
    }

    pub(crate) fn load_sprite_sheet_2(&mut self) {
        self.s1ae66();
    }

    pub(crate) fn load_sprite_sheet_3(&mut self) {
        self.s1ae91();
    }

    pub(crate) fn load_sprite_sheet_4(&mut self) {
        self.s1aebc();
    }

    // ---- 載入：展示錄影與音樂 ----

    /// `DM1.LCK`，展示錄影，3000 位元組讀進展示錄影區。
    pub(crate) fn load_demo_1(&mut self) {
        self.s1bed5();
    }

    /// `DM2.LCK`。第二關入口載入它，蓋掉第一關的錄影。
    pub(crate) fn load_demo_2(&mut self) {
        self.s1beef();
    }

    /// `B.MIX`，關卡音樂。第一至四關的載入函式各一個，都是同一份檔案。
    pub(crate) fn load_stage_music(&mut self, stage: StageMusic) {
        match stage {
            StageMusic::Stage1 => self.s1aa52(),
            StageMusic::Stage2 => self.s1aa6c(),
            StageMusic::Stage3 => self.s1aa86(),
            StageMusic::Stage4 => self.s1aaa0(),
        }
    }

    /// `MAP.GO`，第三關的地圖圖庫（有自己的段變數，`Op::S18ACE` 固定讀它，重製沒有狀態要記）。
    pub(crate) fn load_map_sheet(&mut self) {
        self.s1afbc();
    }

    /// `SHT.SCN` 的檔案載入（解壓在 [`Core::load_map`]，重製沒有狀態要記）。
    pub(crate) fn load_map_file(&mut self) {
        self.s1bf71();
    }
}

/// 關卡音樂的載入函式，各關一個。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StageMusic {
    Stage1,
    Stage2,
    Stage3,
    Stage4,
}
