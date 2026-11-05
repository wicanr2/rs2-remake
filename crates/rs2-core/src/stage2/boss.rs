//! 第二關的中頭目與頭目：合成圖寫進暫存區 B、三層捲動、嘴部動畫。
//! 原版是 `sub_1316C`、`sub_131D1`、`sub_15912`、`sub_159D6`、`sub_15A24`、`sub_15A72` 與 `sub_133AD` 起的嘴部動畫。
//!
//! 中頭目由 `SHT2` 第 32 項（320×96）與第 33 項（96×86）合成，分兩段在幀 263 到 410 與 348 到 540 出現；
//! 頭目由第 34 項（272×161）與第 35 項（144×118）合成，幀 630 起。各層的腳本、起點與來源見下面三個函式。

use crate::game::Core;
use crate::mirror::{BossScript, SCRATCH_B};

/// 中頭目第一層：幀計數不超過 107h 時每幀重設起點，到 19Ah 為止；來源是暫存區 B 的第 0 列起 95 列。
const MID_A_RESET_UNTIL: u16 = 0x107;
const MID_A_LAST: u16 = 0x19A;
/// 中頭目第二層：不超過 15Bh 重設，到 21Ch 為止；來源是暫存區 B 的第 61h 列起 95 列。
const MID_B_RESET_UNTIL: u16 = 0x15B;
const MID_B_LAST: u16 = 0x21C;
/// 頭目：不超過 276h 重設；來源是暫存區 B 的第 0 列起 161 列。
const BOSS_RESET_UNTIL: u16 = 0x276;
/// 頭目腳本的結束碼：讀到就過關。其他關的腳本以 0Fh 結束（`SCRIPT_END`），這一關的腳本結尾是 1Fh。
const BOSS_SCRIPT_CLEAR_BYTE: u8 = 0x1F;

impl Core {
    /// 中頭目合成圖寫進暫存區 B 四份（原版 `sub_1316C`）：`SHT2` 第 32、33 項各兩份。
    ///
    /// 原版在 `131C9h` 檢查 `byte_1CFAA`：整局還沒被打中過（血條重畫的完整性檢查沒有跑過）時是 `N`，
    /// 原版跳回函式開頭，永遠不會離開（卡死，動態已證實，`docs/re/12` 第 4.3 節）。重製預設照原版 panic，
    /// `Fixes::no_hit_hang` 開啟時照常往下走。
    pub(crate) fn compose_stage2_midboss_art(&mut self) {
        self.clear_scratch_b();
        self.select_scratch(SCRATCH_B);
        for (x, y, index) in [(0, 0, 0x20), (0x28, 7, 0x21), (0, 0x61, 0x20), (0x28, 0x68, 0x21)] {
            self.draw_tile_to_scratch(x, y, index);
        }
        if self.flow.integrity_mark == b'N' && !self.fixes.no_hit_hang {
            panic!("byte_1CFAA 是 N：原版在 131CE 跳回 sub_1316C 開頭無限迴圈（整局還沒被打中過）");
        }
    }

    /// 頭目合成圖寫進暫存區 B（原版 `sub_131D1`）：`SHT2` 第 34、35 項。
    pub(crate) fn compose_stage2_boss_art(&mut self) {
        self.clear_scratch_b();
        self.select_scratch(SCRATCH_B);
        self.draw_tile_to_scratch(0, 0, 0x22);
        self.draw_tile_to_scratch(0x22, 0x22, 0x23);
    }

    /// 第二段（幀計數大於 104h）的背景（原版 `sub_137C6` 到 `sub_15912`）：清頁，依序畫三層合成圖。
    /// 五層視差不再畫。
    pub(crate) fn draw_stage2_boss_backdrop(&mut self) {
        self.clear_draw_page();
        self.draw_stage2_mid_layer_a();
        self.draw_stage2_mid_layer_b();
        self.draw_stage2_boss_layer();
    }

    /// 中頭目第一層（原版 `sub_159D6`）：腳本 `127Eh`，起點 x = 0、y = 200，148 步。
    fn draw_stage2_mid_layer_a(&mut self) {
        let frame = self.stage_frame();
        if frame > MID_A_LAST {
            return;
        }
        if frame <= MID_A_RESET_UNTIL {
            self.scenes.boss.layers[0].restart(0xC8, 0, BossScript::Stage2MidA);
            return;
        }
        self.step_boss_layer(0);
        self.draw_boss_layer(0, SCRATCH_B, 0, 0x5F, 0);
    }

    /// 中頭目第二層（原版 `sub_15A24`）：腳本 `1313h`，起點 x = 40、y = −20，189 步。
    fn draw_stage2_mid_layer_b(&mut self) {
        let frame = self.stage_frame();
        if frame > MID_B_LAST {
            return;
        }
        if frame <= MID_B_RESET_UNTIL {
            self.scenes.boss.layers[1].restart(0xFFEC, 0x28, BossScript::Stage2MidB);
            return;
        }
        self.step_boss_layer(1);
        self.draw_boss_layer(1, SCRATCH_B, 0x61, 0x5F, 1);
    }

    /// 頭目（原版 `sub_15A72`）：腳本 `13D1h`，起點 x = 40、y = 30，704 步。腳本走到 1Fh 就過關。
    fn draw_stage2_boss_layer(&mut self) {
        if self.stage_frame() <= BOSS_RESET_UNTIL {
            self.scenes.boss.layers[2].restart(0x1E, 0x28, BossScript::Stage2Boss);
            return;
        }
        let byte = self.step_boss_layer(2);
        self.draw_boss_layer(2, SCRATCH_B, 0, 0xA1, 2);
        if byte == BOSS_SCRIPT_CLEAR_BYTE {
            self.mark_stage_cleared();
        }
    }

    /// 頭目的嘴部動畫（原版 `133ADh` 至 `13415h`）：幀 630 之後每頁在暫存區 B 疊兩張圖，兩頁交替。
    pub(crate) fn animate_stage2_boss_mouth(&mut self) {
        self.select_scratch(SCRATCH_B);
        let (mouth, jaw) = if self.draw_page() != 0 { (0x1D, 0x18) } else { (0x1E, 0x19) };
        self.draw_tile_to_scratch(0x11, 0x20, mouth);
        self.draw_tile_to_scratch(3, 0x29, jaw);
    }
}
