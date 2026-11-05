//! 第四關。
//! 逐函式移植自機器碼，查法見 `tools/q.sh`。規格：`docs/spec/002-full-game.md`。
//!
//! 只移植 E 模式（`byte_1CF74` = `E`、`byte_1CF75` = `N`）。CGA、MGA、純 EGA 的分支不移植，各處註解標明。
//! 配置與釋放緩衝區的模型見 `stage2.rs` 的 `s2_alloc_chain`。

use crate::draw::Pool;
use crate::dseg::a;
use crate::entity::Timetable;
use crate::game::{Core, Req};

impl Core {
    /// `sub_13D70`：載入 BH4，清暫存區 A，把第 0 項（通道壁）寫進暫存區 A 兩份（x = 0、28h），再載入 SHT4。
    pub(crate) fn s13d70(&mut self) {
        self.s1af91();
        self.s11504();
        self.ds.w16(a(0x2685C), 0);
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26882), 0);
        self.ds.w16(a(0x26888), 0);
        self.s19adc();
        self.ds.w16(a(0x26880), 0x28);
        self.ds.w16(a(0x26882), 0);
        self.ds.w16(a(0x26888), 0);
        self.s19adc();
        self.s1aebc();
    }

    /// `sub_13DAF`：第四關完整入口，含載入。回傳時 `dx` 是離開碼。
    /// `nullsub_4` 不移植。
    pub(crate) async fn s13daf(&mut self) {
        self.s140a1();
        self.s1418e();
        self.s19a88();
        self.ds.w16(a(0x26872), 0x8000);
        self.r.cx = 7;
        self.s1152d();
        self.s18894();
        self.s13d70();
        self.s14063();
        self.ds.w16(a(0x1D91F), 0);
        self.ds.w16(a(0x1D921), 0x12);
        self.ds.w16(a(0x1D91D), 0);
        self.ds.w16(a(0x1D9A2), 0);
        self.s19a88();
        self.s10e33();
        self.ds.w16(a(0x1CFE2), 1);
        self.ds.w8(a(0x1CFDB), b'D');
        self.s1aaa0();
        self.s10ad4();
        self.ds.w8(a(0x2739A), b'Y');
        loop {
            self.s19a37();
            self.s13e74().await;
            self.s19a88();
            self.s13e74().await;
            self.s10d99();
            self.s10d8d();
            self.s13f47();
            if matches!(self.r.dx, 0x45 | 0x51 | 0x43) {
                break;
            }
            if self.ds.r8(a(0x1CFAD)) != 0x46 {
                continue;
            }
            // 13E49：過關連爆。先減 word_20B7A 再比較 14h，和第三關相同。
            let n = self.ds.r8(a(0x1CFAE)).wrapping_add(1);
            self.ds.w8(a(0x1CFAE), n);
            self.r.dx = 0x43;
            self.fx.shift_chain_left(3);
            if n > 0x14 {
                break;
            }
            let page = self.ds.r16(a(0x26872));
            self.spawn_stage_clear_burst(page);
        }
        // 13E40：離開。
        self.ds.w8(a(0x2739A), b'N');
        self.s10b12();
    }

    /// `sub_13E74`：依幀計數分派。幀 <= 3E8h 走普通段，之後走頭目段；幀 3EAh 清暫存區 A 並畫 `SHT4` 第 29 項（佔位，無可見內容）。
    pub(crate) async fn s13e74(&mut self) {
        if self.ds.r16(a(0x1CFE2)) <= 0x3E8 {
            self.s13eab().await;
            return;
        }
        self.s13ef9().await;
        if self.ds.r16(a(0x1CFE2)) != 0x3EA {
            return;
        }
        self.s11504();
        self.ds.w16(a(0x2685C), 0);
        self.ds.w16(a(0x26880), 0x14);
        self.ds.w16(a(0x26882), 0);
        self.ds.w16(a(0x26888), 0x1D);
        self.s19adc();
    }

    /// `sub_13EAB`：普通段的一幀。`13ED5h` 的 `cmp word_26872, 0` 與 `jz short $+2` 沒有效果。
    pub(crate) async fn s13eab(&mut self) {
        self.ds.w16(a(0x2685C), 0);
        self.spawn_scheduled(Timetable::Stage4);
        self.update_enemies();
        self.s14458();
        self.s14f92();
        self.s13f7a();
        self.draw_enemies();
        self.update_enemy_bullets();
        self.update_player_bullets();
        self.s14704();
        self.s15232();
        self.draw_enemy_overlays();
        self.update_effects();
        self.s1100a();
        self.s13f6e();
        self.clear_grid1();
        self.clear_grid2();
        self.s10ecb().await;
        // sub_1117F 只有 MGA 分支。
        self.r.cx = 2;
        self.s11097().await;
        self.s2_add(0x1CFE2, 1);
        self.suspend(Req::Sample(0x13EF4)).await;
    }

    /// `sub_13EF9`：頭目段的一幀，背景換成 `sub_1403E`（全黑）。
    pub(crate) async fn s13ef9(&mut self) {
        self.ds.w16(a(0x2685C), 0);
        self.spawn_scheduled(Timetable::Stage4);
        self.update_enemies();
        self.s14458();
        self.s14f92();
        self.s1403e();
        self.draw_enemies();
        self.update_enemy_bullets();
        self.update_player_bullets();
        self.s14704();
        self.s15232();
        self.draw_enemy_overlays();
        self.update_effects();
        self.s1100a();
        self.s13f6e();
        self.clear_grid1();
        self.clear_grid2();
        self.s10ecb().await;
        // sub_1117F 只有 MGA 分支。
        self.r.cx = 2;
        self.s11097().await;
        self.s2_add(0x1CFE2, 1);
        self.suspend(Req::Sample(0x13F42)).await;
    }

    /// `sub_13F47`：幀 > 426h 且 15 個敵人槽全空時設 `byte_1CFAD` = `F`（過關）。
    /// 敵人池指標表經 `entity_link` 讀（`bx` 是原版的位元組偏移，槽號 × 2）。
    pub(crate) fn s13f47(&mut self) {
        if self.ds.r16(a(0x1CFE2)) <= 0x426 {
            return;
        }
        self.r.bx = 0;
        self.r.cx = 0x0F;
        loop {
            self.r.ax = self.entity_link((self.r.bx >> 1) as u8);
            if self.r.ax != 0 {
                return;
            }
            self.r.bx = self.r.bx.wrapping_add(2);
            if !self.s2_loop() {
                break;
            }
        }
        self.ds.w8(a(0x1CFAD), 0x46);
    }

    /// `sub_13F6E`：暫存區選 A。
    pub(crate) fn s13f6e(&mut self) {
        self.ds.w16(a(0x2685C), 0);
    }

    /// `sub_13F7A`：通道背景。先讀腳本移動 `word_1D91D`，以色號 0 清滿 200 列，再畫下壁與垂直鏡射的上壁。
    /// 兩壁的水平相位 `word_1D91F`、`word_1D921` 各自在 0 到 27h 循環。
    pub(crate) fn s13f7a(&mut self) {
        self.s14014();
        self.set_obj(Pool::Background, 0);
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26882), 0);
        self.r.cx = 0xC8;
        self.r.dx = 0;
        self.s1a287();
        // 下壁。
        self.set_obj(Pool::Background, 1);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26882), 0x96);
        self.r.ax = self.ds.r16(a(0x1D91D));
        let y = self.ds.r16(a(0x26882)).wrapping_add(self.r.ax);
        self.ds.w16(a(0x26882), y);
        self.r.ax = self.ds.r16(a(0x1D91F));
        self.ds.w16(a(0x26868), self.r.ax);
        self.r.cx = 0x64;
        self.s19ba4();
        // 上壁。
        self.set_obj(Pool::Background, 2);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26882), 0x28);
        self.r.ax = self.ds.r16(a(0x1D91D));
        let y = self.ds.r16(a(0x26882)).wrapping_add(self.r.ax);
        self.ds.w16(a(0x26882), y);
        self.r.ax = self.ds.r16(a(0x1D921));
        self.ds.w16(a(0x26868), self.r.ax);
        self.r.cx = 0x64;
        self.s19c4f();
        self.clear_obj();
        if self.ds.r16(a(0x1D91F)) >= 0x27 {
            self.ds.w16(a(0x1D91F), 0);
        } else {
            self.s2_add(0x1D91F, 1);
        }
        if self.ds.r16(a(0x1D921)) >= 0x27 {
            self.ds.w16(a(0x1D921), 0);
        } else {
            self.s2_add(0x1D921, 1);
        }
    }

    /// `sub_14014`：讀 `ds:0DE4h` 起的腳本一個位元組（讀取位置 `word_1D9A2` 先加 1）：1 讓 `word_1D91D` 減 0Ah，
    /// 2 加 0Ah，0Fh 把讀取位置退回（停在結尾），其餘不動。
    pub(crate) fn s14014(&mut self) {
        self.r.bx = self.ds.r16(a(0x1D9A2));
        self.s2_add(0x1D9A2, 1);
        self.r.set_al(self.ds.r8(self.r.bx.wrapping_add(0x0DE4)));
        match self.r.al() {
            1 => self.s2_add(0x1D91D, 0x0Au16.wrapping_neg()),
            2 => self.s2_add(0x1D91D, 0x0A),
            0x0F => self.s2_add(0x1D9A2, 1u16.wrapping_neg()),
            _ => {}
        }
    }

    /// `sub_1403E`：頭目段的背景。暫存區 A 第 0 列起 200 列複製到畫面，暫存區 A 在幀 3EAh 之後已清成黑底。
    pub(crate) fn s1403e(&mut self) {
        self.set_obj(Pool::Background, 3);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26882), 0);
        self.ds.w16(a(0x26868), 0);
        self.r.cx = 0xC8;
        self.s19ba4();
        self.clear_obj();
    }

    /// `sub_14063`：`stage4` 調色盤（表 `0D82h`）。純 EGA 的 `1002h` 分支（`byte_1CF75` = Y，表 `0D71h`）不移植。
    pub(crate) fn s14063(&mut self) {
        self.r.bx = 0;
        self.r.dx = 0x0D82;
        self.s14092();
        self.r.bx = 0x30;
        self.r.dx = 0x0D82;
        self.s14092();
    }

    /// `sub_14092`：INT 10h AX=1012h，從 `bl` 起設 `cx` = 16 個 DAC 暫存器，值在 `ds:dx`。
    pub(crate) fn s14092(&mut self) {
        self.r.set_ah(0x10);
        self.r.set_al(0x12);
        self.r.cx = 0x10;
        self.set_dac(self.r.bl(), self.r.dx);
    }

    /// `sub_140A1`：配置第四關的圖庫緩衝區（`word_1CF5C` 起 10 個段值）。E 模式 5 塊 600h、5 塊 1。
    pub(crate) fn s140a1(&mut self) {
        self.ds.w8(a(0x1CEF6), 0x41);
        self.ds.w8(a(0x1CF08), 0x41);
        self.ds.w8(a(0x1CEF7), 0x34);
        self.ds.w8(a(0x1CF09), 0x34);
        self.ds.w8(a(0x1CEF8), 0x30);
        self.ds.w8(a(0x1CF0A), 0x30);
        self.s2_alloc_chain(&[
            (0x1CF5C, 0x600),
            (0x1CF5E, 0x600),
            (0x1CF60, 0x600),
            (0x1CF62, 0x600),
            (0x1CF64, 0x600),
            (0x1CF66, 1),
            (0x1CF68, 1),
            (0x1CF6A, 1),
            (0x1CF6C, 1),
            (0x1CF6E, 1),
        ]);
    }

    /// `sub_1418E`：依相反順序釋放 `sub_140A1` 配置的 10 塊。`13DAFh` 配置後立刻釋放，段值仍留在 `word_1CF5C` 起的字組。
    pub(crate) fn s1418e(&mut self) {
        for dst in [0x1CF6E, 0x1CF6C, 0x1CF6A, 0x1CF68, 0x1CF66, 0x1CF64, 0x1CF62, 0x1CF60, 0x1CF5E, 0x1CF5C] {
            self.r.ax = self.ds.r16(a(dst));
            self.s2_free();
        }
    }
}
