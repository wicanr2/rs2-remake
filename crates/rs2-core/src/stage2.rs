//! 第二關。
//! 逐函式移植自機器碼，查法見 `tools/q.sh`。規格：`docs/spec/002-full-game.md`。
//!
//! 位址慣例同其他檔：`a(0x1CFE2)` 是 IDA 線性位址名稱 `word_1CFE2`，直接寫偏移的（`0x0C25`）是機器碼裡的 `ds:` 立即值。
//! 只移植 E 模式（`byte_1CF74` = `E`、`byte_1CF75` = `N`）。CGA、MGA、純 EGA 的分支不移植，各處註解標明。
//! `mov ax, seg dseg` 與 `es` 不建模（暫存器只存資料段內的值）。
//!
//! 防拷檢查有三處原版是無限迴圈或無限遞迴（`130AF`、`131C9`、`138F9`），資料段通過開機檢查後走不到，
//! 這裡改成 `panic!`，避免卡死整個程序。

use crate::draw::Pool;
use crate::dseg::a;
use crate::entity::Timetable;
use crate::game::{Core, Req};

impl Core {
    // ---------------------------------------------------------------
    // 小工具（第二關與第四關共用，前綴 s2_ 避免和其他檔撞名）
    // ---------------------------------------------------------------

    /// `loop` 指令：`cx` 減 1，非 0 才跳回去。
    pub(crate) fn s2_loop(&mut self) -> bool {
        self.r.cx = self.r.cx.wrapping_sub(1);
        self.r.cx != 0
    }

    /// `add word ptr [linear], n`。
    pub(crate) fn s2_add(&mut self, linear: u32, n: u16) {
        let v = self.ds.r16(a(linear)).wrapping_add(n);
        self.ds.w16(a(linear), v);
    }

    /// 寫 `word_26880`、`word_26882`、`word_26888` 後呼叫 `sub_19ADC`（圖寫進暫存區）。
    fn s2_put(&mut self, x: u16, y: u16, i: u16) {
        self.ds.w16(a(0x26880), x);
        self.ds.w16(a(0x26882), y);
        self.ds.w16(a(0x26888), i);
        self.s19adc();
    }

    /// `sub_10210` 的 INT 21h AH=48h 之後：`ax` 是 DOS 回傳的段值，`bx` 不變，`dx` 指向錯誤訊息 `348h`，
    /// 旗標來自最後一個 `cmp ax, 8`。每次呼叫把錯誤訊息前綴 `byte_1CEF8`、`byte_1CF0A` 各加 1。
    pub(crate) fn s2_alloc_one(&mut self, seg: u16) {
        self.r.ax = seg;
        let v = self.ds.r8(a(0x1CEF8)).wrapping_add(1);
        self.ds.w8(a(0x1CEF8), v);
        let v = self.ds.r8(a(0x1CF0A)).wrapping_add(1);
        self.ds.w8(a(0x1CF0A), v);
        self.r.dx = 0x348;
        self.r.cf = seg < 8;
        self.r.zf = seg == 8;
    }

    /// 一串 `mov bx, 段落數 / call sub_10210 / mov word_xxxx, ax`。
    ///
    /// 模型：DOS 不在模擬範圍，配置結果用第一個目的字組現有的值當基底（上一關釋放後最低的空洞起點，
    /// 每關相同），之後每塊接在前一塊後面，中間隔一個 MCB 段落：`seg[k] = seg[k-1] + 段落數[k-1] + 1`。
    /// 第一關配置的傾印 `0x4989, 0x4E8A, ...` 符合這個公式。假設不成立時對拍會在這 10 個字組先出現差異。
    pub(crate) fn s2_alloc_chain(&mut self, blocks: &[(u32, u16)]) {
        let mut seg = self.ds.r16(a(blocks[0].0));
        for &(dst, para) in blocks {
            self.r.bx = para;
            self.s2_alloc_one(seg);
            self.ds.w16(a(dst), self.r.ax);
            seg = seg.wrapping_add(para).wrapping_add(1);
        }
    }

    /// `sub_1026C`：`ax` 是要釋放的段值。INT 21h AH=49h 成功時 `al` 不變、`ah` 變 49h。`dx` 指向 `327h`，旗標來自 `cmp ax, 9`。
    pub(crate) fn s2_free(&mut self) {
        let v = self.ds.r8(a(0x1CEF8)).wrapping_add(1);
        self.ds.w8(a(0x1CEF8), v);
        let v = self.ds.r8(a(0x1CF0A)).wrapping_add(1);
        self.ds.w8(a(0x1CF0A), v);
        self.r.set_ah(0x49);
        self.r.dx = 0x327;
        self.r.cf = self.r.ax < 9;
        self.r.zf = self.r.ax == 9;
    }

    // ---------------------------------------------------------------
    // 進入與主迴圈
    // ---------------------------------------------------------------

    /// `sub_12FB6`：載入 BH2，把背景圖寫進暫存區 A（`word_2685C` = 0）與 B（8000h），再載入 SHT2。
    pub(crate) fn s12fb6(&mut self) {
        self.s1af66();
        self.ds.w16(a(0x2685C), 0);
        for (x, y, i) in [
            (0, 0, 0),
            (0x28, 0, 0),
            (0, 0x22, 1),
            (0x28, 0x22, 1),
            (0, 0x44, 2),
            (0x28, 0x44, 2),
            (0, 0x5A, 3),
            (0x28, 0x5A, 3),
        ] {
            self.s2_put(x, y, i);
        }
        // 1306C：第 4 項橫排 20 塊。
        self.r.cx = 0x14;
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26888), 4);
        self.ds.w16(a(0x26882), 0x70);
        loop {
            self.s19adc();
            self.s2_add(0x26880, 2);
            if !self.s2_loop() {
                break;
            }
        }
        // 1308D：第 5 項起 8 張（8 個相位）。130AF 的 byte_1D3A2 必須是 Y，否則原版在 130A2 無限迴圈。
        self.r.cx = 8;
        self.ds.w16(a(0x26880), 0x28);
        self.ds.w16(a(0x26888), 5);
        self.ds.w16(a(0x26882), 0x70);
        loop {
            self.s19adc();
            self.s2_add(0x26880, 2);
            self.s2_add(0x26888, 1);
            if self.ds.r8(a(0x1D3A2)) != 0x59 {
                panic!("byte_1D3A2 不是 Y：原版在 130A2 無限迴圈（防拷檢查未通過）");
            }
            if !self.s2_loop() {
                break;
            }
        }
        // 130B9：第 13 項起 4 張。
        self.r.cx = 4;
        self.ds.w16(a(0x26880), 0x38);
        self.ds.w16(a(0x26888), 0x0D);
        self.ds.w16(a(0x26882), 0x70);
        loop {
            self.s19adc();
            self.s2_add(0x26880, 2);
            self.s2_add(0x26888, 1);
            if !self.s2_loop() {
                break;
            }
        }
        // 130DE：第 17 項起 2 張。
        self.r.cx = 2;
        self.ds.w16(a(0x26880), 0x40);
        self.ds.w16(a(0x26888), 0x11);
        self.ds.w16(a(0x26882), 0x70);
        loop {
            self.s19adc();
            self.s2_add(0x26880, 2);
            self.s2_add(0x26888, 1);
            if !self.s2_loop() {
                break;
            }
        }
        // 13103：暫存區 B。第 19 到 22 項為遮罩層的圖，第 23 到 26 項為遮罩，第 27 項為柱。
        self.ds.w16(a(0x2685C), 0x8000);
        self.r.cx = 4;
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26888), 0x13);
        self.ds.w16(a(0x26882), 0);
        loop {
            self.s19adc();
            self.s2_add(0x26880, 2);
            self.s2_add(0x26888, 1);
            if !self.s2_loop() {
                break;
            }
        }
        self.r.cx = 4;
        self.ds.w16(a(0x26880), 8);
        self.ds.w16(a(0x26888), 0x17);
        self.ds.w16(a(0x26882), 0);
        loop {
            self.s19adc();
            self.s2_add(0x26880, 2);
            self.s2_add(0x26888, 1);
            if !self.s2_loop() {
                break;
            }
        }
        self.s2_put(0x10, 0, 0x1B);
        self.s1ae66();
    }

    /// `sub_1316C`：中頭目合成圖（`SHT2` 第 32、33 項）寫進暫存區 B 四份。
    /// `131C9h` 的 `byte_1CFAA` 為 `N` 時原版跳回函式開頭，永遠不會離開。
    pub(crate) fn s1316c(&mut self) {
        self.s1152d();
        self.ds.w16(a(0x2685C), 0x8000);
        self.s2_put(0, 0, 0x20);
        self.s2_put(0x28, 7, 0x21);
        self.s2_put(0, 0x61, 0x20);
        self.s2_put(0x28, 0x68, 0x21);
        // 原版跳回開頭，因為 byte_1CFAA 只有 sub_11375 會寫，而它只在玩家被打中時執行。
        if self.ds.r8(a(0x1CFAA)) == 0x4E && !self.fixes.no_hit_hang {
            panic!("byte_1CFAA 是 N：原版在 131CE 跳回 sub_1316C 開頭無限迴圈（整局還沒被打中過）");
        }
    }

    /// `sub_131D1`：頭目合成圖（`SHT2` 第 34、35 項）寫進暫存區 B。
    pub(crate) fn s131d1(&mut self) {
        self.s1152d();
        self.ds.w16(a(0x2685C), 0x8000);
        self.s2_put(0, 0, 0x22);
        self.s2_put(0x22, 0x22, 0x23);
    }

    /// `sub_13205`：第二關完整入口，含載入。回傳時 `dx` 是離開碼。
    /// `nullsub_4`（被改成 `retn` 的載入進度條）不移植。
    pub(crate) async fn s13205(&mut self) {
        self.s138d6().await;
        self.s139cd();
        self.s19a88();
        self.ds.w16(a(0x26872), 0x8000);
        self.r.cx = 7;
        self.s1152d();
        self.s12fb6();
        self.s1beef();
        self.s132ab();
        self.s18894();
        self.s137f8();
        self.s19a88();
        self.ds.w8(a(0x1CFDB), b'B');
        self.s10e33();
        self.ds.w8(a(0x1CFAD), b'N');
        self.fx.reset_chain();
        self.s1aa6c();
        self.s10ad4();
        self.ds.w8(a(0x2739A), b'Y');
        loop {
            self.s19a37();
            self.s132be().await;
            self.s19a88();
            self.s132be().await;
            self.s10d99();
            self.s10d8d();
            if matches!(self.r.dx, 0x45 | 0x51 | 0x43) {
                break;
            }
            if self.ds.r8(a(0x1CFAD)) != 0x46 {
                continue;
            }
            // 13280：過關連爆。和第一關不同，先比較 14h 再減 word_20B7A。
            let n = self.ds.r8(a(0x1CFAE)).wrapping_add(1);
            self.ds.w8(a(0x1CFAE), n);
            self.r.dx = 0x43;
            if n > 0x14 {
                break;
            }
            self.fx.shift_chain_left(3);
            let page = self.ds.r16(a(0x26872));
            self.spawn_stage_clear_burst(page);
        }
        // 13277：離開。
        self.ds.w8(a(0x2739A), b'N');
        self.s10b12();
    }

    /// `sub_132AB`：把調色盤表 `0C25h` 起 45 位元組複製到 `0CA7h`（淡出用的工作表）。
    pub(crate) fn s132ab(&mut self) {
        self.r.si = 0x0C25;
        self.r.di = 0x0CA7;
        self.r.cx = 0x2D;
        for k in 0..0x2Du16 {
            let b = self.ds.r8(0x0C25 + k);
            self.ds.w8(0x0CA7 + k, b);
        }
        self.r.si = 0x0C25 + 0x2D;
        self.r.di = 0x0CA7 + 0x2D;
        self.r.cx = 0;
    }

    /// `sub_132BE`：依幀計數分派。`132C3h` 的 `jnz short $+2` 兩個方向都落到下一道，所以對 1 的比較沒有效果。
    pub(crate) async fn s132be(&mut self) {
        if self.ds.r16(a(0x1CFE2)) == 0x21C {
            self.s131d1();
        }
        if self.ds.r16(a(0x1CFE2)) == 0x104 {
            self.s1316c();
        }
        if self.ds.r16(a(0x1CFE2)) <= 0x104 {
            self.s132eb().await;
        } else {
            self.s13344().await;
        }
    }

    /// `sub_132EB`：第一段（幀計數 <= 104h）的一幀，背景是 5 層視差。
    pub(crate) async fn s132eb(&mut self) {
        self.ds.w16(a(0x2685C), 0);
        self.spawn_scheduled(Timetable::Stage2);
        self.update_enemies();
        self.s14458();
        self.s14f92();
        self.s1355b();
        self.s13580();
        self.s135eb();
        self.s1365f();
        self.s13711();
        self.draw_enemies();
        self.update_enemy_bullets();
        self.update_player_bullets();
        self.s14704();
        self.s15232();
        self.draw_enemy_overlays();
        self.update_effects();
        self.s1100a();
        self.s1375f();
        self.s13443();
        self.clear_grid1();
        self.clear_grid2();
        self.s10ecb().await;
        // sub_1117F 只有 MGA 分支。
        self.r.cx = 2;
        self.s11097().await;
        self.s2_add(0x1CFE2, 1);
        self.suspend(Req::Sample(0x1333C)).await;
        self.s13421();
    }

    /// `sub_13344`：第二段（幀計數 > 104h）的一幀，背景換成頭目合成腳本。
    pub(crate) async fn s13344(&mut self) {
        self.ds.w16(a(0x2685C), 0);
        self.spawn_scheduled(Timetable::Stage2);
        self.update_enemies();
        self.s14458();
        self.s14f92();
        self.s137c6();
        self.draw_enemies();
        self.update_enemy_bullets();
        self.update_player_bullets();
        self.s14704();
        self.s15232();
        self.draw_enemy_overlays();
        self.update_effects();
        // 1336E 的 cmp word_26872, 0 與 jz short $+2 沒有效果。
        self.s1100a();
        self.s1375f();
        self.s13443();
        self.clear_grid1();
        self.clear_grid2();
        self.s10ecb().await;
        // sub_1117F 只有 MGA 分支。
        self.r.cx = 2;
        self.s11097().await;
        self.s2_add(0x1CFE2, 1);
        self.suspend(Req::Sample(0x13390)).await;
        let f = self.ds.r16(a(0x1CFE2));
        if f == 0x106 {
            self.s132ab();
            self.s13827();
            return;
        }
        if f == 0x276 {
            self.s13856();
            return;
        }
        if f <= 0x276 {
            return;
        }
        // 133AD：頭目合成圖上的嘴部動畫，依頁交替。
        self.ds.w16(a(0x2685C), 0x8000);
        if self.ds.r16(a(0x26872)) != 0 {
            self.s2_put(0x11, 0x20, 0x1D);
            self.s2_put(3, 0x29, 0x18);
        } else {
            self.s2_put(0x11, 0x20, 0x1E);
            self.s2_put(3, 0x29, 0x19);
        }
    }

    /// `sub_13421`：幀 1 重設淡出工作表與調色盤，幀 > A0h 的奇數頁（`word_26872` 非 0）淡出一級。
    pub(crate) fn s13421(&mut self) {
        let f = self.ds.r16(a(0x1CFE2));
        if f > 0xA0 {
            if self.ds.r16(a(0x26872)) != 0 {
                self.s13894();
            }
            return;
        }
        if f == 1 {
            self.s132ab();
            self.s137f8();
        }
    }

    // ---------------------------------------------------------------
    // 火焰面、近景條、遮罩層、柱、黑框
    // ---------------------------------------------------------------

    /// `sub_13443`：暫存區選 A 後畫火焰面。
    pub(crate) fn s13443(&mut self) {
        self.ds.w16(a(0x2685C), 0);
        self.s13452();
    }

    /// `sub_13452`：火焰面。上下各兩層，遮罩 AND 加圖 OR，頁不同用不同的來源列。
    /// 幀 1 設 `word_1D7BE` = 9Ch、`word_1D7C0` = 22h；幀 >= 280h 兩者各走一步，幀 >= 2A2h 不再畫。
    pub(crate) fn s13452(&mut self) {
        let f = self.ds.r16(a(0x1CFE2));
        if f == 1 {
            self.ds.w16(a(0x1D7BE), 0x9C);
            self.ds.w16(a(0x1D7C0), 0x22);
        }
        if f >= 0x280 {
            self.s2_add(0x1D7BE, 1);
            self.s2_add(0x1D7C0, 1u16.wrapping_neg());
            if f >= 0x2A2 {
                return;
            }
        }
        // 1347E：奇數頁（word_26872 非 0）遮罩從第 44h 列、圖從第 0 列；偶數頁遮罩從第 5Ah 列、圖從第 22h 列。
        let (mask_row, img_row) = if self.ds.r16(a(0x26872)) != 0 { (0x44, 0) } else { (0x5A, 0x22) };
        self.set_obj(Pool::Background, 7);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), mask_row);
        self.ds.w16(a(0x26880), 0);
        self.r.ax = self.ds.r16(a(0x1D7BE));
        self.ds.w16(a(0x26882), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1D7C2));
        self.ds.w16(a(0x26868), self.r.ax);
        self.r.cx = 0x16;
        self.s1a09f();
        self.r.ax = self.ds.r16(a(0x1D7C0));
        self.ds.w16(a(0x26882), self.r.ax);
        self.r.cx = 0x16;
        self.s1a1e6();
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), img_row);
        self.ds.w16(a(0x26880), 0);
        self.r.ax = self.ds.r16(a(0x1D7BE));
        self.ds.w16(a(0x26882), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1D7C2));
        self.ds.w16(a(0x26868), self.r.ax);
        self.r.cx = 0x22;
        self.s19da8();
        self.r.ax = self.ds.r16(a(0x1D7C0));
        self.ds.w16(a(0x26882), self.r.ax);
        self.r.cx = 0x22;
        self.s19f3b();
        self.clear_obj();
        // 13548：水平相位 word_1D7C2 在 0 到 28h 循環。
        if self.ds.r16(a(0x1D7C2)) >= 0x28 {
            self.ds.w16(a(0x1D7C2), 0);
        } else {
            self.s2_add(0x1D7C2, 1);
        }
    }

    /// `sub_1355B`：背景磚，來源列 70h、140 列，靜態複製到 y = 1Ah。
    pub(crate) fn s1355b(&mut self) {
        self.set_obj(Pool::Background, 0);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0x70);
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26882), 0x1A);
        self.ds.w16(a(0x26868), 0);
        self.r.cx = 0x8C;
        self.s19ba4();
        self.clear_obj();
    }

    /// `sub_13580`：近景條 A（8 相位），5 份間隔 8 位元組，兩個 y。相位 `word_1D7C4` 每幀 7 到 0，
    /// 歸零時起點 `word_1D7C6` 左移 1 位元組（8 到 1 循環）。
    pub(crate) fn s13580(&mut self) {
        self.r.cx = 5;
        self.ds.w16(a(0x26882), 0x66);
        self.r.ax = self.ds.r16(a(0x1D7C6));
        self.ds.w16(a(0x26880), self.r.ax);
        self.set_obj(Pool::Background, 1);
        loop {
            let cx = self.r.cx;
            self.ds.w16(a(0x26882), 0x69);
            self.s135cd();
            self.ds.w16(a(0x26882), 0x4D);
            self.s135cd();
            self.s2_add(0x26880, 8);
            self.r.cx = cx;
            if !self.s2_loop() {
                break;
            }
        }
        self.clear_obj();
        self.s2_add(0x1D7C4, 1u16.wrapping_neg());
        if (self.ds.r16(a(0x1D7C4)) as i16) >= 0 {
            return;
        }
        self.ds.w16(a(0x1D7C4), 7);
        self.s2_add(0x1D7C6, 1u16.wrapping_neg());
        if (self.ds.r16(a(0x1D7C6)) as i16) > 0 {
            return;
        }
        self.ds.w16(a(0x1D7C6), 8);
    }

    /// `sub_135CD`：近景條 A 的一份，水平位移 `2 * word_1D7C4 + 28h`，9 列。
    pub(crate) fn s135cd(&mut self) {
        self.r.ax = self.ds.r16(a(0x1D7C4)).wrapping_shl(1);
        self.r.ax = self.r.ax.wrapping_add(0x28);
        self.ds.w16(a(0x26868), self.r.ax);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0x70);
        self.r.cx = 9;
        self.s19ff9();
    }

    /// `sub_135EB`：近景條 B（4 相位）4 份間隔 10 位元組，每份在兩個 y 畫條，另在 y = 4Dh 畫遮罩層（`sub_136CA`）。
    pub(crate) fn s135eb(&mut self) {
        self.r.cx = 4;
        self.ds.w16(a(0x26882), 0x66);
        self.r.ax = self.ds.r16(a(0x1D7CA));
        self.ds.w16(a(0x26880), self.r.ax);
        loop {
            let cx = self.r.cx;
            self.set_obj(Pool::Background, 2);
            self.ds.w16(a(0x26882), 0x72);
            self.s13641();
            self.ds.w16(a(0x26882), 0x44);
            self.s13641();
            self.clear_obj();
            self.ds.w16(a(0x26882), 0x4D);
            self.s136ca();
            self.s2_add(0x26880, 0x0A);
            self.r.cx = cx;
            if !self.s2_loop() {
                break;
            }
        }
        self.s2_add(0x1D7C8, 1u16.wrapping_neg());
        if (self.ds.r16(a(0x1D7C8)) as i16) >= 0 {
            return;
        }
        self.ds.w16(a(0x1D7C8), 3);
        self.s2_add(0x1D7CA, 1u16.wrapping_neg());
        if (self.ds.r16(a(0x1D7CA)) as i16) > 0 {
            return;
        }
        self.ds.w16(a(0x1D7CA), 0x0A);
    }

    /// `sub_13641`：近景條 B 的一份，水平位移 `2 * word_1D7C8 + 38h`，9 列。
    pub(crate) fn s13641(&mut self) {
        self.r.ax = self.ds.r16(a(0x1D7C8)).wrapping_shl(1);
        self.r.ax = self.r.ax.wrapping_add(0x38);
        self.ds.w16(a(0x26868), self.r.ax);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0x70);
        self.r.cx = 9;
        self.s19ff9();
    }

    /// `sub_1365F`：近景條 C（2 相位）2 份間隔 20 位元組，每份兩個 y。
    pub(crate) fn s1365f(&mut self) {
        self.r.cx = 2;
        self.ds.w16(a(0x26882), 0x66);
        self.r.ax = self.ds.r16(a(0x1D7CE));
        self.ds.w16(a(0x26880), self.r.ax);
        self.set_obj(Pool::Background, 3);
        loop {
            let cx = self.r.cx;
            self.ds.w16(a(0x26882), 0x7C);
            self.s136ac();
            self.ds.w16(a(0x26882), 0x36);
            self.s136ac();
            self.s2_add(0x26880, 0x14);
            self.r.cx = cx;
            if !self.s2_loop() {
                break;
            }
        }
        self.clear_obj();
        self.s2_add(0x1D7CC, 1u16.wrapping_neg());
        if (self.ds.r16(a(0x1D7CC)) as i16) >= 0 {
            return;
        }
        self.ds.w16(a(0x1D7CC), 1);
        self.s2_add(0x1D7CE, 1u16.wrapping_neg());
        if (self.ds.r16(a(0x1D7CE)) as i16) > 0 {
            return;
        }
        self.ds.w16(a(0x1D7CE), 0x14);
    }

    /// `sub_136AC`：近景條 C 的一份，水平位移 `2 * word_1D7CC + 40h`，13 列。
    pub(crate) fn s136ac(&mut self) {
        self.r.ax = self.ds.r16(a(0x1D7CC)).wrapping_shl(1);
        self.r.ax = self.r.ax.wrapping_add(0x40);
        self.ds.w16(a(0x26868), self.r.ax);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0x70);
        self.r.cx = 0x0D;
        self.s19ff9();
    }

    /// `sub_136CA`：遮罩層。暫存區 B 的遮罩 AND 加圖 OR，各 37 列，水平位移 `2 * word_1D7C8`。
    pub(crate) fn s136ca(&mut self) {
        self.set_obj(Pool::Background, 4);
        self.r.ax = self.ds.r16(a(0x1D7C8)).wrapping_shl(1);
        self.ds.w16(a(0x26868), self.r.ax);
        self.ds.w16(a(0x26864), 8);
        self.ds.w16(a(0x26866), 0);
        self.r.cx = 0x25;
        self.ds.w16(a(0x2685C), 0x8000);
        self.s1a144();
        self.r.ax = self.ds.r16(a(0x1D7C8)).wrapping_shl(1);
        self.ds.w16(a(0x26868), self.r.ax);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.r.cx = 0x25;
        self.ds.w16(a(0x2685C), 0x8000);
        self.s19e66();
        self.ds.w16(a(0x2685C), 0);
        self.clear_obj();
    }

    /// `sub_13711`：柱，2 份間隔 20 位元組，y = 36h，每幀左移 1 位元組（`word_1D7D0` 從 14h 到 1）。
    pub(crate) fn s13711(&mut self) {
        self.r.cx = 2;
        self.r.ax = self.ds.r16(a(0x1D7D0));
        self.ds.w16(a(0x26880), self.r.ax);
        self.ds.w16(a(0x26882), 0x36);
        self.ds.w16(a(0x26868), 0);
        self.ds.w16(a(0x26864), 0x10);
        self.ds.w16(a(0x26866), 0);
        self.set_obj(Pool::Background, 5);
        loop {
            let cx = self.r.cx;
            self.r.cx = 0x53;
            self.ds.w16(a(0x2685C), 0x8000);
            self.s19ff9();
            self.ds.w16(a(0x2685C), 0);
            self.s2_add(0x26880, 0x14);
            self.r.cx = cx;
            if !self.s2_loop() {
                break;
            }
        }
        self.clear_obj();
        self.s2_add(0x1D7D0, 1u16.wrapping_neg());
        if (self.ds.r16(a(0x1D7D0)) as i16) > 0 {
            return;
        }
        self.ds.w16(a(0x1D7D0), 0x14);
    }

    /// `sub_1375F`：上下兩條黑框。幀 <= 107h 各 20 列（y = 0、AAh），108h 到 27Fh 為 16 列與 15 列（y = 0、AFh），之後不畫。
    pub(crate) fn s1375f(&mut self) {
        let f = self.ds.r16(a(0x1CFE2));
        if f <= 0x107 {
            self.set_obj(Pool::Background, 6);
            self.ds.w16(a(0x26880), 0);
            self.ds.w16(a(0x26882), 0);
            self.r.cx = 0x14;
            self.r.dx = 0;
            self.s1a287();
            self.ds.w16(a(0x26880), 0);
            self.ds.w16(a(0x26882), 0xAA);
            self.r.cx = 0x14;
            self.r.dx = 0;
            self.s1a287();
            self.clear_obj();
            return;
        }
        if f >= 0x280 {
            return;
        }
        self.set_obj(Pool::Background, 6);
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26882), 0);
        self.r.cx = 0x10;
        self.r.dx = 0;
        self.s1a287();
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26882), 0xAF);
        self.r.cx = 0x0F;
        self.r.dx = 0;
        self.s1a287();
        self.clear_obj();
    }

    // ---------------------------------------------------------------
    // 頭目段背景、調色盤
    // ---------------------------------------------------------------

    /// `sub_137C6`：第二段的背景，轉給 `sub_15912`。`137CAh` 起是 `retn` 之後的死碼，不移植。
    pub(crate) fn s137c6(&mut self) {
        self.s15912();
    }

    /// `sub_137F8`：`stage2` 調色盤（表 `0C25h`）。純 EGA 的 `1002h` 分支（`byte_1CF75` = Y）不移植。
    pub(crate) fn s137f8(&mut self) {
        self.r.bx = 0;
        self.r.dx = 0x0C25;
        self.s13885();
        self.r.bx = 0x30;
        self.r.dx = 0x0C25;
        self.s13885();
    }

    /// `sub_13827`：`stage2_b` 調色盤（表 `0C66h`）。分支同 `sub_137F8`。
    pub(crate) fn s13827(&mut self) {
        self.r.bx = 0;
        self.r.dx = 0x0C66;
        self.s13885();
        self.r.bx = 0x30;
        self.r.dx = 0x0C66;
        self.s13885();
    }

    /// `sub_13856`：`stage2_c` 調色盤（表 `0CE8h`）。分支同 `sub_137F8`。
    pub(crate) fn s13856(&mut self) {
        self.r.bx = 0;
        self.r.dx = 0x0CE8;
        self.s13885();
        self.r.bx = 0x30;
        self.r.dx = 0x0CE8;
        self.s13885();
    }

    /// `sub_13885`：INT 10h AX=1012h，從 `bl` 起設 `cx` = 16 個 DAC 暫存器，值在 `ds:dx`。
    pub(crate) fn s13885(&mut self) {
        self.r.set_ah(0x10);
        self.r.set_al(0x12);
        self.r.cx = 0x10;
        self.set_dac(self.r.bl(), self.r.dx);
    }

    /// `sub_13894`：淡出工作表 `0CA7h` 的調色盤，再把色號 8 到 14 的 21 個位元組（`0CBFh` 起）各減 1，到 0 為止。
    /// 純 EGA 的 `1002h` 分支（表 `0C96h`）不移植。
    pub(crate) fn s13894(&mut self) {
        self.r.bx = 0;
        self.r.dx = 0x0CA7;
        self.s13885();
        self.r.bx = 0x30;
        self.r.dx = 0x0CA7;
        self.s13885();
        self.r.si = 0x0CBF;
        self.r.cx = 0x15;
        loop {
            self.r.set_al(self.ds.r8(self.r.si));
            if self.r.al() != 0 {
                self.r.set_al(self.r.al().wrapping_sub(1));
                self.ds.w8(self.r.si, self.r.al());
            }
            self.r.si = self.r.si.wrapping_add(1);
            if !self.s2_loop() {
                break;
            }
        }
    }

    /// `sub_138D6`：配置第二關的圖庫緩衝區（`word_1CF5C` 起 10 個段值）。E 模式 5 塊 600h、4 塊 10h、1 塊 1。
    /// `138F9h` 的 `word_1D3B0` 非 0 時原版 `jmp sub_13205`，無限遞迴，這裡 `panic!`。
    pub(crate) async fn s138d6(&mut self) {
        self.ds.w8(a(0x1CEF6), 0x41);
        self.ds.w8(a(0x1CF08), 0x41);
        self.ds.w8(a(0x1CEF7), 0x2B);
        self.ds.w8(a(0x1CF09), 0x2B);
        self.ds.w8(a(0x1CEF8), 0x30);
        self.ds.w8(a(0x1CF0A), 0x30);
        if self.ds.r16(a(0x1D3B0)) != 0 {
            panic!("word_1D3B0 非 0：原版 jmp sub_13205 無限遞迴（防拷檢查未通過）");
        }
        self.s2_alloc_chain(&[
            (0x1CF5C, 0x600),
            (0x1CF5E, 0x600),
            (0x1CF60, 0x600),
            (0x1CF62, 0x600),
            (0x1CF64, 0x600),
            (0x1CF66, 0x10),
            (0x1CF68, 0x10),
            (0x1CF6A, 0x10),
            (0x1CF6C, 0x10),
            (0x1CF6E, 1),
        ]);
    }

    /// `sub_139CD`：依相反順序釋放 `sub_138D6` 配置的 10 塊。`13205h` 配置後立刻釋放，段值仍留在 `word_1CF5C` 起的字組。
    pub(crate) fn s139cd(&mut self) {
        self.ds.w8(a(0x1CEF6), 0x42);
        self.ds.w8(a(0x1CF08), 0x42);
        self.ds.w8(a(0x1CEF7), 0x2D);
        self.ds.w8(a(0x1CF09), 0x2D);
        self.ds.w8(a(0x1CEF8), 0x30);
        self.ds.w8(a(0x1CF0A), 0x30);
        for dst in [0x1CF6E, 0x1CF6C, 0x1CF6A, 0x1CF68, 0x1CF66, 0x1CF64, 0x1CF62, 0x1CF60, 0x1CF5E, 0x1CF5C] {
            self.r.ax = self.ds.r16(a(dst));
            self.s2_free();
        }
    }

    /// `sub_15912`：第二段背景。清畫面頁，再依序畫三層頭目合成腳本。
    pub(crate) fn s15912(&mut self) {
        self.s19cf9();
        self.s159d6();
        self.s15a24();
        self.s15a72();
    }

    /// `sub_159D6`：中頭目合成層 1。幀 <= 107h 設起點 (y = C8h, x = 0)、腳本 `127Eh`；108h 到 19Ah 讀腳本一步再畫（`sub_15B23`）。
    pub(crate) fn s159d6(&mut self) {
        let f = self.ds.r16(a(0x1CFE2));
        if f > 0x19A {
            return;
        }
        if f <= 0x107 {
            self.ds.w16(a(0x1DB5F), 0xC8);
            self.ds.w16(a(0x1DB61), 0);
            self.ds.w16(a(0x1DB6D), 0x127E);
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DB61));
        self.ds.w16(a(0x1DB5D), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB5F));
        self.ds.w16(a(0x1DB5B), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB6D));
        self.ds.w16(a(0x1DB6B), self.r.ax);
        self.s15bb0();
        self.r.ax = self.ds.r16(a(0x1DB6B));
        self.ds.w16(a(0x1DB6D), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB5B));
        self.ds.w16(a(0x1DB5F), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB5D));
        self.ds.w16(a(0x1DB61), self.r.ax);
        self.s15b23();
    }

    /// `sub_15A24`：中頭目合成層 2。幀 <= 15Bh 設起點 (y = FFECh, x = 28h)、腳本 `1313h`；15Ch 到 21Ch 讀腳本一步再畫（`sub_15B52`）。
    pub(crate) fn s15a24(&mut self) {
        let f = self.ds.r16(a(0x1CFE2));
        if f > 0x21C {
            return;
        }
        if f <= 0x15B {
            self.ds.w16(a(0x1DB63), 0xFFEC);
            self.ds.w16(a(0x1DB65), 0x28);
            self.ds.w16(a(0x1DB6F), 0x1313);
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DB65));
        self.ds.w16(a(0x1DB5D), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB63));
        self.ds.w16(a(0x1DB5B), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB6F));
        self.ds.w16(a(0x1DB6B), self.r.ax);
        self.s15bb0();
        self.r.ax = self.ds.r16(a(0x1DB6B));
        self.ds.w16(a(0x1DB6F), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB5B));
        self.ds.w16(a(0x1DB63), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB5D));
        self.ds.w16(a(0x1DB65), self.r.ax);
        self.s15b52();
    }

    /// `sub_15A72`：頭目合成層 3。幀 <= 276h 設起點 (y = 1Eh, x = 28h)、腳本 `13D1h`；之後讀腳本一步再畫（`sub_15B81`），
    /// 腳本位元組為 `1Fh` 時設 `byte_1CFAD` = `F`（過關）。
    pub(crate) fn s15a72(&mut self) {
        if self.ds.r16(a(0x1CFE2)) <= 0x276 {
            self.ds.w16(a(0x1DB69), 0x28);
            self.ds.w16(a(0x1DB67), 0x1E);
            self.ds.w16(a(0x1DB71), 0x13D1);
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DB69));
        self.ds.w16(a(0x1DB5D), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB67));
        self.ds.w16(a(0x1DB5B), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB71));
        self.ds.w16(a(0x1DB6B), self.r.ax);
        self.s15bb0();
        self.r.ax = self.ds.r16(a(0x1DB6B));
        self.ds.w16(a(0x1DB71), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB5B));
        self.ds.w16(a(0x1DB67), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB5D));
        self.ds.w16(a(0x1DB69), self.r.ax);
        self.s15b81();
        if self.ds.r8(a(0x1DB5A)) == 0x1F {
            self.ds.w8(a(0x1CFAD), 0x46);
        }
    }

    /// `sub_15B23`：合成層 1，暫存區 B 第 0 列起 95 列，水平位移 `-word_1DB5D`，畫到 y = `word_1DB5B`。
    pub(crate) fn s15b23(&mut self) {
        self.ds.w16(a(0x2685C), 0x8000);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.ds.w16(a(0x26880), 0);
        self.r.ax = 0;
        self.r.ax = self.r.ax.wrapping_sub(self.ds.r16(a(0x1DB5D)));
        self.ds.w16(a(0x26868), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB5B));
        self.ds.w16(a(0x26882), self.r.ax);
        self.r.cx = 0x5F;
        self.set_obj(Pool::Boss, 0);
        self.s19d08();
        self.clear_obj();
    }

    /// `sub_15B52`：合成層 2，暫存區 B 第 61h 列起 95 列。其餘同 `sub_15B23`。
    pub(crate) fn s15b52(&mut self) {
        self.ds.w16(a(0x2685C), 0x8000);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0x61);
        self.ds.w16(a(0x26880), 0);
        self.r.ax = 0;
        self.r.ax = self.r.ax.wrapping_sub(self.ds.r16(a(0x1DB5D)));
        self.ds.w16(a(0x26868), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB5B));
        self.ds.w16(a(0x26882), self.r.ax);
        self.r.cx = 0x5F;
        self.set_obj(Pool::Boss, 1);
        self.s19d08();
        self.clear_obj();
    }

    /// `sub_15B81`：合成層 3，暫存區 B 第 0 列起 161 列。其餘同 `sub_15B23`。
    pub(crate) fn s15b81(&mut self) {
        self.ds.w16(a(0x2685C), 0x8000);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.ds.w16(a(0x26880), 0);
        self.r.ax = 0;
        self.r.ax = self.r.ax.wrapping_sub(self.ds.r16(a(0x1DB5D)));
        self.ds.w16(a(0x26868), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB5B));
        self.ds.w16(a(0x26882), self.r.ax);
        self.r.cx = 0xA1;
        self.set_obj(Pool::Boss, 2);
        self.s19d08();
        self.clear_obj();
    }
}
