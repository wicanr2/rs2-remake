//! 標題、開場、示範、選單、關間過場。
//! 逐函式移植自機器碼，查法見 `tools/q.sh`。規格：`docs/spec/002-full-game.md`。
//!
//! 不移植的呼叫（呼叫處都有註解）：
//! - `sub_12B49`、`sub_12BCF`：DOS 記憶體配置與釋放。寫入檔名工作區 `1CEF6h` 至 `1CF0Ah`
//!   與配置段落 `1CF46h` 至 `1CF4Ch`，都在 `A0_EXEMPT`。
//! - `sub_1117F`：`byte_1CF74` 為 `E` 時直接 `retn`，只有 MGA 分支。
//! - CGA、MGA、純 EGA 分支：`byte_1CF74` 固定 `E`、`byte_1CF75` 固定 `N`。

use crate::draw::DrawCmd;
use crate::dseg::a;
use crate::game::Core;

/// 兩頁交替的一輪，重複 `cx` 次。原版的 `push cx` 到 `pop cx` 加 `loop`，
/// 每輪兩次呼叫同一個步進函式（頁 0 與頁 1），步進函式內有 `sub_11097` 等待。
macro_rules! two_page_loop {
    ($s:ident, $n:expr, $step:ident) => {{
        $s.r.cx = $n;
        loop {
            let saved = $s.r.cx;
            $s.s19a88();
            $s.s18812();
            $s.s1282d();
            $s.$step().await;
            $s.s1117f();
            $s.s19a37();
            $s.s18819();
            $s.s1282d();
            $s.$step().await;
            $s.s1117f();
            $s.r.cx = saved;
            if !$s.t_loop() {
                break;
            }
        }
    }};
}

impl Core {
    /// `word_26880` 寫入 x（位元組）。
    fn t_x(&mut self, v: u16) {
        self.ds.w16(a(0x26880), v);
    }

    /// `word_26882` 寫入 y（像素列）。
    fn t_y(&mut self, v: u16) {
        self.ds.w16(a(0x26882), v);
    }

    /// `word_26888` 寫入圖庫索引。
    fn t_i(&mut self, v: u16) {
        self.ds.w16(a(0x26888), v);
    }

    /// `byte_27422`：最近一次按下的鍵（ASCII），0 表示沒有。
    fn t_key(&self) -> u8 {
        self.ds.r8(a(0x27422))
    }

    /// `loop` 指令：`cx` 減 1，回傳是否繼續。
    fn t_loop(&mut self) -> bool {
        self.r.cx = self.r.cx.wrapping_sub(1);
        self.r.cx != 0
    }

    /// 等任意鍵：清 `byte_27422`，輪詢到非 0，再清一次。
    async fn t_wait_key(&mut self) {
        self.ds.w8(a(0x27422), 0);
        while self.t_key() == 0 {
            self.poll_tick().await;
        }
        self.ds.w8(a(0x27422), 0);
    }

    /// BEG1／J 圖庫的一個 blit（`sub_19068`）。
    fn t_beg(&mut self, x: u16, y: u16, i: u16) {
        self.t_x(x);
        self.t_y(y);
        self.t_i(i);
        self.s19068();
    }

    /// ACT 字樣的一個字：遮罩 AND（`sub_18D46`）加平面 OR（`sub_18CE2`）。
    fn t_act(&mut self, x: u16, y: u16, i: u16) {
        self.t_x(x);
        self.t_y(y);
        self.t_i(i);
        self.s18d46();
        self.s18ce2();
    }

    /// 16 色調色盤載入兩次（DAC 起點 0 與 30h），`table` 是 `dseg` 內 DAC 表偏移。
    /// 原版先判斷 `byte_1CF75`，`Y`（純 EGA）走 `1002h`，不移植。
    fn t_pal(&mut self, table: u16) {
        self.r.bx = 0;
        self.r.dx = table;
        self.s12b3a();
        self.r.dx = table;
        self.r.bx = 0x30;
        self.s12b3a();
    }

    /// `sub_11556`：標題迴圈與正式遊戲的主流程。離開時 `dx` 是離開碼。
    /// `11619h` 的自修改跳躍通過密碼檢查後是兩個 NOP，所以 `sub_12438` 之後接 `1161Bh`。
    pub(crate) async fn s11556(&mut self) {
        self.ds.w8(a(0x1CFAC), b'Y');
        'title: loop {
            // loc_1155B
            self.ds.w8(a(0x1CFAB), b'Y');
            self.s1aa38();
            self.s10ad4();
            self.s19a37();
            self.s18812();
            // sub_12B49、sub_12BCF：DOS 記憶體配置，不移植。
            self.s12b49();
            self.s19a37();
            self.s1bc4d();
            'anim: loop {
                // loc_1157D
                self.ds.w8(a(0x27422), 0);
                self.s1152d();
                self.s18894();
                self.ds.w16(a(0x2685C), 0x8000);
                // 開場動畫的七段。任何一段結束後 byte_27422 非 0 就跳到 loc_115F8。
                let pressed = 'seq: {
                    self.s11910().await;
                    self.s1295b();
                    if self.t_key() != 0 {
                        break 'seq true;
                    }
                    self.s1152d();
                    self.s18894();
                    self.s1175b().await;
                    if self.t_key() != 0 {
                        break 'seq true;
                    }
                    self.s1152d();
                    self.s12989();
                    self.s18894();
                    self.s11806().await;
                    if self.t_key() != 0 {
                        break 'seq true;
                    }
                    self.s1152d();
                    self.s1295b();
                    self.s18894();
                    self.s116f3();
                    if self.t_key() != 0 {
                        break 'seq true;
                    }
                    self.s117bf();
                    self.r.cx = 0x0A;
                    loop {
                        let saved = self.r.cx;
                        self.r.cx = 4;
                        self.s11097().await;
                        self.r.cx = saved;
                        if self.t_key() != 0 {
                            break 'seq true;
                        }
                        if !self.t_loop() {
                            break;
                        }
                    }
                    false
                };
                if !pressed {
                    // 115E8：10 x 4 tick 沒有按鍵，進示範模式。
                    self.ds.w8(a(0x1CFDC), b'Y');
                    self.s10b12();
                    self.ds.w8(a(0x2739A), b'N');
                    // loc_11679
                    self.s11685().await;
                    if self.r.dx == 0x51 {
                        return;
                    }
                    continue 'title;
                }
                // loc_115F8
                self.s10b12();
                self.ds.w8(a(0x2739A), b'N');
                if self.t_key() == b'C' {
                    self.s12588().await;
                    continue 'anim;
                }
                // loc_1160E
                self.s18894();
                self.ds.w8(a(0x1CFDC), b'N');
                self.s12438().await;
                // 1161B：word_1D3B0 非 0 表示密碼檢查失敗，返回。
                if self.ds.r16(a(0x1D3B0)) != 0 {
                    return;
                }
                self.s11c6b().await;
                self.s10cd0().await;
                if self.r.dx == 0x51 {
                    return;
                }
                if self.r.dx == 0x45 {
                    continue 'title;
                }
                self.s11e46().await;
                self.ds.w8(a(0x274EC), 0x32);
                self.s13205().await;
                if self.r.dx == 0x51 {
                    return;
                }
                if self.r.dx == 0x45 {
                    continue 'title;
                }
                self.s121dd().await;
                self.ds.w8(a(0x274EC), 0x32);
                self.s13a51().await;
                if self.r.dx == 0x51 {
                    return;
                }
                if self.r.dx == 0x45 {
                    continue 'title;
                }
                self.s12379().await;
                self.ds.w8(a(0x274EC), 0x32);
                self.s13daf().await;
                if self.r.dx == 0x51 {
                    return;
                }
                if self.r.dx == 0x45 {
                    continue 'title;
                }
                self.s141d0().await;
                return;
            }
        }
    }

    /// `sub_11685`：示範模式。`word_1D146` 輪播第一關與第二關，到 3 或 4 重置為 1 返回。
    /// 有按鍵（`byte_27422` 非 0）也返回。`116DFh` 以後的第三關示範不可達。
    pub(crate) async fn s11685(&mut self) {
        loop {
            self.ds.w8(a(0x1CFDD), 1);
            if self.t_key() != 0 {
                return;
            }
            match self.ds.r16(a(0x1D146)) {
                1 => {
                    self.s10e33();
                    self.s11c6b().await;
                    self.s10cd0().await;
                    let v = self.ds.r16(a(0x1D146)).wrapping_add(1);
                    self.ds.w16(a(0x1D146), v);
                }
                2 => {
                    self.s10e33();
                    self.s11e46().await;
                    self.ds.w8(a(0x274EC), 0x32);
                    self.s13205().await;
                    let v = self.ds.r16(a(0x1D146)).wrapping_add(1);
                    self.ds.w16(a(0x1D146), v);
                }
                3 | 4 => {
                    self.ds.w16(a(0x1D146), 1);
                    return;
                }
                _ => {
                    self.ds.w16(a(0x1D146), 1);
                }
            }
        }
    }

    /// `sub_116F3`：BEG1 第 0 項（眼部特寫）從畫面下方往上捲，之後畫 `MY` 第 47 項。
    pub(crate) fn s116f3(&mut self) {
        self.t_x(0);
        self.t_y(0);
        self.t_i(0);
        self.s19b40();
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.t_x(5);
        self.ds.w16(a(0x26868), 0);
        self.t_y(0xF0);
        self.s19a88();
        self.s18819();
        self.r.cx = 0x28;
        loop {
            let saved = self.r.cx;
            self.r.cx = 0xC8;
            self.s19d08();
            let y = self.ds.r16(a(0x26882)).wrapping_sub(5);
            self.t_y(y);
            self.r.cx = saved;
            if self.t_key() != 0 {
                return;
            }
            if !self.t_loop() {
                break;
            }
        }
        self.t_x(0x0F);
        self.t_y(0x14);
        self.t_i(0x2F);
        self.s18c0a();
    }

    /// `sub_1175B`：BEG1 第 1 項（手臂）寫進暫存區 B，水平位移從 -10 每步 -1 共 33 步。
    pub(crate) async fn s1175b(&mut self) {
        self.t_x(1);
        self.t_y(0);
        self.t_i(1);
        self.s19b40();
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.t_x(0);
        self.ds.w16(a(0x26868), 0xFFF6);
        self.t_y(0x28);
        self.s19a88();
        self.s18819();
        self.r.cx = 0xC8;
        self.s19d08();
        self.r.cx = 1;
        self.s11097().await;
        self.r.cx = 0x0F;
        self.s11097().await;
        self.r.cx = 0x21;
        loop {
            let saved = self.r.cx;
            self.r.cx = 0xC8;
            self.s19d08();
            let v = self.ds.r16(a(0x26868)).wrapping_sub(1);
            self.ds.w16(a(0x26868), v);
            self.r.cx = saved;
            if self.t_key() != 0 {
                return;
            }
            if !self.t_loop() {
                break;
            }
        }
    }

    /// `sub_117BF`：製作名單（`MY` 第 48 項）與 SOFTSTAR 標誌（第 15 項）對向移動 20 步。
    pub(crate) fn s117bf(&mut self) {
        self.ds.w16(a(0x1D148), 0x28);
        self.ds.w16(a(0x1D14A), 0xFFF1);
        self.t_y(0x98);
        self.r.cx = 0x14;
        loop {
            let saved = self.r.cx;
            self.r.ax = self.ds.r16(a(0x1D148));
            self.t_x(self.r.ax);
            self.t_i(0x30);
            self.s18c0a();
            self.r.ax = self.ds.r16(a(0x1D14A));
            self.t_x(self.r.ax);
            self.t_i(0x0F);
            self.s18c0a();
            let v = self.ds.r16(a(0x1D14A)).wrapping_add(1);
            self.ds.w16(a(0x1D14A), v);
            let v = self.ds.r16(a(0x1D148)).wrapping_sub(1);
            self.ds.w16(a(0x1D148), v);
            self.r.cx = saved;
            if self.t_key() != 0 {
                return;
            }
            if !self.t_loop() {
                break;
            }
        }
    }

    /// `sub_11806`：BEG1 第 6 到 2 項（太空船正面）在 y=100 依序放大，最後接 `sub_118A2`。
    pub(crate) async fn s11806(&mut self) {
        self.t_y(0x64);
        self.t_x(0x12);
        self.t_i(6);
        self.s19068();
        self.r.cx = 6;
        self.s11097().await;
        if self.t_key() != 0 {
            return;
        }
        self.t_x(0x11);
        self.t_i(5);
        self.s19068();
        self.r.cx = 3;
        self.s11097().await;
        if self.t_key() != 0 {
            return;
        }
        self.t_x(0x10);
        self.t_i(4);
        self.s19068();
        self.r.cx = 2;
        self.s11097().await;
        if self.t_key() != 0 {
            return;
        }
        self.t_x(0x0E);
        self.t_i(3);
        self.s19068();
        self.r.cx = 2;
        self.s11097().await;
        if self.t_key() != 0 {
            return;
        }
        self.t_x(0x0D);
        self.t_i(2);
        self.s19068();
        self.s118a2().await;
        self.r.cx = 1;
        self.s11097().await;
        if self.t_key() != 0 {
            return;
        }
        self.r.cx = 0x0E;
        self.s11097().await;
    }

    /// `sub_118A2`：BEG1 第 14 項（`RS-S` 字樣）往左 38 步，再依序疊上第 15、16、17 項。
    pub(crate) async fn s118a2(&mut self) {
        self.t_x(0x28);
        self.t_y(0x1E);
        self.t_i(0x0E);
        self.r.cx = 0x26;
        loop {
            let saved = self.r.cx;
            self.s19068();
            let v = self.ds.r16(a(0x26880)).wrapping_sub(1);
            self.t_x(v);
            self.r.cx = saved;
            if self.t_key() != 0 {
                return;
            }
            if !self.t_loop() {
                break;
            }
        }
        self.r.cx = 1;
        self.s11097().await;
        self.r.cx = 8;
        self.s11097().await;
        self.t_x(0x1D);
        self.t_y(0x1E);
        self.t_i(0x0F);
        self.s19068();
        self.r.cx = 2;
        self.s11097().await;
        if self.t_key() != 0 {
            return;
        }
        self.t_i(0x10);
        self.s19068();
        self.r.cx = 2;
        self.s11097().await;
        self.t_i(0x11);
        self.s19068();
    }

    /// `sub_11910`：BEG1 第 7 項（地球）從左緣滑入，20 輪 x 2 頁，結尾接 `sub_1199F`。
    pub(crate) async fn s11910(&mut self) {
        self.s129e5();
        self.s1152d();
        self.t_x(8);
        self.t_y(0);
        self.t_i(7);
        self.s19b40();
        self.ds.w16(a(0x26864), 0x28);
        self.ds.w16(a(0x26866), 0);
        self.t_x(0);
        self.ds.w16(a(0x26868), 0);
        self.t_y(0);
        self.s19a88();
        self.s18819();
        self.r.cx = 0x14;
        loop {
            let saved = self.r.cx;
            self.s19a37();
            self.s19cf9();
            self.r.cx = 0xC8;
            self.s19d08();
            let v = self.ds.r16(a(0x26864)).wrapping_sub(1);
            self.ds.w16(a(0x26864), v);
            self.s1117f();
            self.s19a88();
            self.s19cf9();
            self.r.cx = 0xC8;
            self.s19d08();
            let v = self.ds.r16(a(0x26864)).wrapping_sub(1);
            self.ds.w16(a(0x26864), v);
            self.s1117f();
            self.r.cx = saved;
            if self.t_key() != 0 {
                return;
            }
            if !self.t_loop() {
                break;
            }
        }
        self.s1117f();
        self.s19cf9();
        self.r.cx = 0xC8;
        self.s19d08();
        self.s1117f();
        self.s19a37();
        self.r.cx = 1;
        self.s11097().await;
        self.s1199f().await;
    }

    /// `sub_1199F`：太空船接近的 12 步，每步先 `sub_11C26` 重畫地球背景，雙頁交替。
    /// 步 12 之後 `sub_12A13` 切 `beg1_b` 閃光，再切回 `beg1_a`。
    pub(crate) async fn s1199f(&mut self) {
        // 步 0
        self.s11c26();
        self.t_beg(0x1B, 0x35, 8);
        self.s1117f();
        self.r.cx = 5;
        self.s11097().await;
        self.s19a88();
        // 步 1
        self.s11c26();
        self.t_beg(0x1A, 0x3B, 9);
        self.t_beg(0x1E, 0x37, 0x15);
        self.s1117f();
        self.r.cx = 5;
        self.s11097().await;
        self.s19a37();
        // 步 2
        self.s11c26();
        self.t_beg(0x19, 0x40, 0x0A);
        self.t_beg(0x1E, 0x37, 0x16);
        self.s1117f();
        self.r.cx = 5;
        self.s11097().await;
        self.s19a88();
        // 步 3
        self.s11c26();
        self.t_beg(0x18, 0x45, 0x0B);
        self.t_beg(0x1E, 0x35, 0x17);
        self.s1117f();
        self.r.cx = 4;
        self.s11097().await;
        self.s19a37();
        // 步 4
        self.s11c26();
        self.t_beg(0x16, 0x4C, 0x0C);
        self.t_beg(0x1E, 0x34, 0x18);
        self.s1117f();
        self.r.cx = 4;
        self.s11097().await;
        self.s19a88();
        // 步 5
        self.s11c26();
        self.t_beg(0x13, 0x52, 0x0D);
        self.t_beg(0x1F, 0x33, 0x19);
        self.s1117f();
        self.r.cx = 3;
        self.s11097().await;
        self.s19a37();
        // 步 6
        self.s11c26();
        self.t_beg(0x10, 0x5A, 0x12);
        self.t_beg(0x1F, 0x39, 0x1A);
        self.s1117f();
        self.r.cx = 3;
        self.s11097().await;
        self.s19a88();
        // 步 7
        self.s11c26();
        self.t_beg(0x0A, 0x63, 0x13);
        self.t_beg(0x1F, 0x37, 0x1B);
        self.s1117f();
        self.r.cx = 2;
        self.s11097().await;
        self.s19a37();
        // 步 8
        self.s11c26();
        self.t_beg(3, 0x7A, 0x14);
        self.t_beg(0x1F, 0x37, 0x1C);
        self.s1117f();
        self.r.cx = 1;
        self.s11097().await;
        self.s19a88();
        // 步 9：行星
        self.s11c26();
        self.t_beg(0x1D, 0x28, 0x1D);
        self.s1117f();
        self.r.cx = 1;
        self.s11097().await;
        self.s19a37();
        // 步 10
        self.s11c26();
        self.t_beg(0x19, 3, 0x1E);
        self.s1117f();
        self.r.cx = 1;
        self.s11097().await;
        self.s19a88();
        // 步 11
        self.s11c26();
        self.t_beg(0x13, 0, 0x1F);
        self.s1117f();
        self.r.cx = 1;
        self.s11097().await;
        self.s19a37();
        // 結尾：只重畫背景，等 4 tick，灰階閃光 20 tick，切回 beg1_a。
        self.s11c26();
        self.s1117f();
        self.r.cx = 4;
        self.s11097().await;
        self.s12a13();
        self.r.cx = 0x14;
        self.s11097().await;
        self.s19a88();
        self.s129e5();
        self.r.cx = 3;
        self.s11097().await;
        if self.t_key() != 0 {
            return;
        }
        self.r.cx = 0x0F;
        self.s11097().await;
    }

    /// `sub_11C26`：清目前的頁，再把暫存區 B 整頁複製上去（地球背景）。
    pub(crate) fn s11c26(&mut self) {
        self.s19cf9();
        self.t_x(0);
        self.t_y(0);
        self.r.cx = 0xC8;
        self.s19d08();
    }

    /// `sub_11C3C`：第一關的 J 圖。一般模式畫 J1 第 2 項並等任意鍵，示範模式只清暫存區 B 並設 `title_d`。
    pub(crate) async fn s11c3c(&mut self) {
        if self.ds.r8(a(0x1CFDC)) != b'N' {
            self.s1152d();
            self.s1295b();
            return;
        }
        self.s12858();
        self.s1117f();
        self.s19a37();
        self.s18812();
        self.s1282d();
        self.t_wait_key().await;
    }

    /// `sub_11C6B`：第一關過場。J 圖之後 A、T、C 與數字 1 的 ACT 動畫，三段共 10、5、12 輪。
    pub(crate) async fn s11c6b(&mut self) {
        self.s12a41();
        self.s11c3c().await;
        self.ds.w16(a(0x1D148), 0xFFF7);
        self.ds.w16(a(0x1D152), 0xFFCE);
        self.ds.w16(a(0x1D14C), 0x27);
        self.s1282d();
        two_page_loop!(self, 0x0A, s11cfc);
        two_page_loop!(self, 5, s11d58);
        self.ds.w16(a(0x1D14E), 0);
        two_page_loop!(self, 0x0C, s11dac);
    }

    /// `sub_11CFC`：A 向右、T 向左、C 往下落，各一步，等 1 tick。
    pub(crate) async fn s11cfc(&mut self) {
        let v = self.ds.r16(a(0x1D148)).wrapping_add(1);
        self.ds.w16(a(0x1D148), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x32, 0x3D);
        let v = self.ds.r16(a(0x1D14C)).wrapping_sub(1);
        self.ds.w16(a(0x1D14C), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x32, 0x3F);
        let v = self.ds.r16(a(0x1D152)).wrapping_add(0x0A);
        self.ds.w16(a(0x1D152), v);
        self.r.ax = v;
        self.t_act(0x0F, self.r.ax, 0x3E);
        self.r.cx = 1;
        self.s11097().await;
    }

    /// `sub_11D58`：A、T 原位重畫，C 往上彈 10 列，等 1 tick。
    pub(crate) async fn s11d58(&mut self) {
        self.r.ax = self.ds.r16(a(0x1D148));
        self.t_act(self.r.ax, 0x32, 0x3D);
        self.r.ax = self.ds.r16(a(0x1D14C));
        self.t_act(self.r.ax, 0x32, 0x3F);
        let v = self.ds.r16(a(0x1D152)).wrapping_sub(0x0A);
        self.ds.w16(a(0x1D152), v);
        self.r.ax = v;
        self.t_act(0x0F, self.r.ax, 0x3E);
        self.r.cx = 1;
        self.s11097().await;
    }

    /// `sub_11DAC`：數字 1（`MY` 第 64 項）每步右移，A、T、C 原位重畫，等 1 tick。
    pub(crate) async fn s11dac(&mut self) {
        let v = self.ds.r16(a(0x1D14E)).wrapping_add(1);
        self.ds.w16(a(0x1D14E), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x32, 0x40);
        self.r.ax = self.ds.r16(a(0x1D148));
        self.t_act(self.r.ax, 0x32, 0x3D);
        self.r.ax = self.ds.r16(a(0x1D14C));
        self.t_act(self.r.ax, 0x32, 0x3F);
        self.r.ax = self.ds.r16(a(0x1D152));
        self.t_act(0x0F, self.r.ax, 0x3E);
        self.r.cx = 1;
        self.s11097().await;
    }

    /// `sub_11E17`：第二關的 J 圖（J2 第 1 項），其餘同 `sub_11C3C`。
    pub(crate) async fn s11e17(&mut self) {
        if self.ds.r8(a(0x1CFDC)) != b'N' {
            self.s1152d();
            self.s1295b();
            return;
        }
        self.s12883();
        self.s1117f();
        self.s19a37();
        self.s18812();
        self.s1282d();
        self.t_wait_key().await;
    }

    /// `sub_11E46`：第二關過場，ACT 三字分三列進場後依五段換位。
    pub(crate) async fn s11e46(&mut self) {
        self.s12a6f();
        self.s11e17().await;
        self.ds.w16(a(0x1D148), 0xFFFF);
        self.ds.w16(a(0x1D14A), 0xFFFF);
        self.ds.w16(a(0x1D14C), 0xFFFF);
        self.ds.w16(a(0x1D14E), 0x28);
        self.ds.w16(a(0x1D156), 0x32);
        self.s1282d();
        two_page_loop!(self, 0x0E, s11f58);
        two_page_loop!(self, 2, s11fb3);
        self.ds.w16(a(0x1D152), 0x5A);
        two_page_loop!(self, 2, s1200a);
        self.ds.w16(a(0x1D154), 0x82);
        two_page_loop!(self, 4, s12062);
        two_page_loop!(self, 5, s120b6);
        two_page_loop!(self, 5, s12132);
    }

    /// `sub_11F58`：三字各右移 1，分三列（y=32h、5Ah、82h），等 1 tick。
    pub(crate) async fn s11f58(&mut self) {
        let v = self.ds.r16(a(0x1D148)).wrapping_add(1);
        self.ds.w16(a(0x1D148), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x32, 0x3D);
        let v = self.ds.r16(a(0x1D14A)).wrapping_add(1);
        self.ds.w16(a(0x1D14A), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x5A, 0x3E);
        let v = self.ds.r16(a(0x1D14C)).wrapping_add(1);
        self.ds.w16(a(0x1D14C), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x82, 0x3F);
        self.r.cx = 1;
        self.s11097().await;
    }

    /// `sub_11FB3`：A 不動，另兩字各右移 1。
    pub(crate) async fn s11fb3(&mut self) {
        self.r.ax = self.ds.r16(a(0x1D148));
        self.t_act(self.r.ax, 0x32, 0x3D);
        let v = self.ds.r16(a(0x1D14A)).wrapping_add(1);
        self.ds.w16(a(0x1D14A), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x5A, 0x3E);
        let v = self.ds.r16(a(0x1D14C)).wrapping_add(1);
        self.ds.w16(a(0x1D14C), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x82, 0x3F);
        self.r.cx = 1;
        self.s11097().await;
    }

    /// `sub_1200A`：T 的 y 每步 -10（`word_1D152`），C 右移 1。
    pub(crate) async fn s1200a(&mut self) {
        self.r.ax = self.ds.r16(a(0x1D148));
        self.t_act(self.r.ax, 0x32, 0x3D);
        let x = self.ds.r16(a(0x1D14A));
        let v = self.ds.r16(a(0x1D152)).wrapping_sub(0x0A);
        self.ds.w16(a(0x1D152), v);
        self.r.ax = v;
        self.t_act(x, self.r.ax, 0x3E);
        let v = self.ds.r16(a(0x1D14C)).wrapping_add(1);
        self.ds.w16(a(0x1D14C), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x82, 0x3F);
        self.r.cx = 1;
        self.s11097().await;
    }

    /// `sub_12062`：C 的 y 每步 -10（`word_1D154`）。
    pub(crate) async fn s12062(&mut self) {
        self.r.ax = self.ds.r16(a(0x1D148));
        self.t_act(self.r.ax, 0x32, 0x3D);
        let x = self.ds.r16(a(0x1D14A));
        self.r.ax = self.ds.r16(a(0x1D152));
        self.t_act(x, self.r.ax, 0x3E);
        let v = self.ds.r16(a(0x1D154)).wrapping_sub(0x0A);
        self.ds.w16(a(0x1D154), v);
        let x = self.ds.r16(a(0x1D14C));
        self.r.ax = v;
        self.t_act(x, self.r.ax, 0x3F);
        self.r.cx = 1;
        self.s11097().await;
    }

    /// `sub_120B6`：三字各左移 1，數字 2（`MY` 第 65 項，`word_1D14E` 左移、`word_1D156` 下移 10）。
    pub(crate) async fn s120b6(&mut self) {
        let v = self.ds.r16(a(0x1D148)).wrapping_sub(1);
        self.ds.w16(a(0x1D148), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x32, 0x3D);
        let v = self.ds.r16(a(0x1D14A)).wrapping_sub(1);
        self.ds.w16(a(0x1D14A), v);
        self.r.ax = v;
        let y = self.ds.r16(a(0x1D152));
        self.t_act(self.r.ax, y, 0x3E);
        let v = self.ds.r16(a(0x1D14C)).wrapping_sub(1);
        self.ds.w16(a(0x1D14C), v);
        self.r.ax = v;
        let y = self.ds.r16(a(0x1D154));
        self.t_act(self.r.ax, y, 0x3F);
        let y = self.ds.r16(a(0x1D156)).wrapping_add(0x0A);
        self.ds.w16(a(0x1D156), y);
        let v = self.ds.r16(a(0x1D14E)).wrapping_sub(1);
        self.ds.w16(a(0x1D14E), v);
        self.r.ax = y;
        self.t_act(v, self.r.ax, 0x41);
        self.r.cx = 1;
        self.s11097().await;
    }

    /// `sub_12132`：同 `sub_120B6`，但數字 2 的 y 每步 -10。
    pub(crate) async fn s12132(&mut self) {
        let v = self.ds.r16(a(0x1D148)).wrapping_sub(1);
        self.ds.w16(a(0x1D148), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x32, 0x3D);
        let v = self.ds.r16(a(0x1D14A)).wrapping_sub(1);
        self.ds.w16(a(0x1D14A), v);
        self.r.ax = v;
        let y = self.ds.r16(a(0x1D152));
        self.t_act(self.r.ax, y, 0x3E);
        let v = self.ds.r16(a(0x1D14C)).wrapping_sub(1);
        self.ds.w16(a(0x1D14C), v);
        self.r.ax = v;
        let y = self.ds.r16(a(0x1D154));
        self.t_act(self.r.ax, y, 0x3F);
        let y = self.ds.r16(a(0x1D156)).wrapping_sub(0x0A);
        self.ds.w16(a(0x1D156), y);
        let v = self.ds.r16(a(0x1D14E)).wrapping_sub(1);
        self.ds.w16(a(0x1D14E), v);
        self.r.ax = y;
        self.t_act(v, self.r.ax, 0x41);
        self.r.cx = 1;
        self.s11097().await;
    }

    /// `sub_121AE`：第三關的 J 圖（J3 第 0 項，由下往上滑入），其餘同 `sub_11C3C`。
    pub(crate) async fn s121ae(&mut self) {
        if self.ds.r8(a(0x1CFDC)) != b'N' {
            self.s1152d();
            self.s1295b();
            return;
        }
        self.s127cd();
        self.s1117f();
        self.s19a37();
        self.s18812();
        self.s1282d();
        self.t_wait_key().await;
    }

    /// `sub_121DD`：第三關過場，四字上下夾擊進場 13 輪。
    pub(crate) async fn s121dd(&mut self) {
        self.s121ae().await;
        self.ds.w16(a(0x1D150), 0xFF7B);
        self.ds.w16(a(0x1D152), 0xFFC9);
        self.ds.w16(a(0x1D154), 0xCD);
        self.ds.w16(a(0x1D156), 0x11B);
        two_page_loop!(self, 0x0D, s1221e);
    }

    /// `sub_1221E`：A、C 從上方（+8、+5），T 與數字 3 從下方（-5、-8），x 固定 0Ch、10h、14h、18h。
    pub(crate) async fn s1221e(&mut self) {
        let v = self.ds.r16(a(0x1D150)).wrapping_add(8);
        self.ds.w16(a(0x1D150), v);
        self.r.ax = v;
        self.t_act(0x0C, self.r.ax, 0x3D);
        let v = self.ds.r16(a(0x1D152)).wrapping_add(5);
        self.ds.w16(a(0x1D152), v);
        self.r.ax = v;
        self.t_act(0x10, self.r.ax, 0x3E);
        let v = self.ds.r16(a(0x1D154)).wrapping_sub(5);
        self.ds.w16(a(0x1D154), v);
        self.r.ax = v;
        self.t_act(0x14, self.r.ax, 0x3F);
        let v = self.ds.r16(a(0x1D156)).wrapping_sub(8);
        self.ds.w16(a(0x1D156), v);
        self.r.ax = v;
        self.t_act(0x18, self.r.ax, 0x42);
        self.r.cx = 1;
        self.s11097().await;
    }

    /// `sub_1234A`：第四關的 J 圖（J4 第 3 項），其餘同 `sub_11C3C`。
    pub(crate) async fn s1234a(&mut self) {
        if self.ds.r8(a(0x1CFDC)) != b'N' {
            self.s1152d();
            self.s1295b();
            return;
        }
        self.s128ae();
        self.s1117f();
        self.s19a37();
        self.s18812();
        self.s1282d();
        self.t_wait_key().await;
    }

    /// `sub_12379`：第四關過場，四字左右進場 13 輪。
    pub(crate) async fn s12379(&mut self) {
        self.s1234a().await;
        self.ds.w16(a(0x1D148), 0x3C);
        self.ds.w16(a(0x1D14A), 0x28);
        self.ds.w16(a(0x1D14C), 0xFFFB);
        self.ds.w16(a(0x1D14E), 0xFFE7);
        self.s1291a();
        two_page_loop!(self, 0x0D, s123bd);
    }

    /// `sub_123BD`：四字在 y=50h，x 依序 -2、-1、+1、+2。
    pub(crate) async fn s123bd(&mut self) {
        let v = self.ds.r16(a(0x1D148)).wrapping_sub(2);
        self.ds.w16(a(0x1D148), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x50, 0x3D);
        let v = self.ds.r16(a(0x1D14A)).wrapping_sub(1);
        self.ds.w16(a(0x1D14A), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x50, 0x3E);
        let v = self.ds.r16(a(0x1D14C)).wrapping_add(1);
        self.ds.w16(a(0x1D14C), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x50, 0x3F);
        let v = self.ds.r16(a(0x1D14E)).wrapping_add(2);
        self.ds.w16(a(0x1D14E), v);
        self.r.ax = v;
        self.t_act(self.r.ax, 0x50, 0x43);
        self.r.cx = 1;
        self.s11097().await;
    }

    /// `sub_12438`：只有 `call sub_127A5; retn`，其後的舊選單畫面沒有任何入口。
    pub(crate) async fn s12438(&mut self) {
        self.s127a5();
    }

    /// `sub_12588`：選單。方向鍵改難度（`word_1D142`）與角色（`word_1D144`），
    /// `byte_274C4`（Enter 一類）或 `byte_274A7` 離開。
    pub(crate) async fn s12588(&mut self) {
        self.s129b7();
        self.s18894();
        self.s19a37();
        self.ds.w16(a(0x26872), 0);
        'redraw: loop {
            self.s126b8().await;
            loop {
                // loc_1259A
                if self.ds.r8(a(0x274D3)) == 1 || self.ds.r8(a(0x274DB)) == 1 {
                    // loc_125CE：上下鍵切換選中的列。
                    self.ds.w8(a(0x274D3), 0);
                    self.ds.w8(a(0x274DB), 0);
                    match self.ds.r16(a(0x1D140)) {
                        0 => self.ds.w16(a(0x1D140), 1),
                        1 => self.ds.w16(a(0x1D140), 0),
                        _ => {}
                    }
                    continue 'redraw;
                }
                self.s1262c();
                if self.r.dx == 0x59 {
                    continue 'redraw;
                }
                self.s12672();
                if self.r.dx == 0x59 {
                    continue 'redraw;
                }
                if self.ds.r8(a(0x274C4)) == 1 || self.ds.r8(a(0x274A7)) == 1 {
                    self.ds.w8(a(0x27422), 0);
                    return;
                }
                // 原版此處是不等待的忙等迴圈。
                self.poll_tick().await;
            }
        }
    }

    /// `sub_1262C`：選中第 0 列（難度）時，左右鍵改 `word_1D142`（0 至 3 環繞）。`dx` 為 `Y` 表示有變動。
    pub(crate) fn s1262c(&mut self) {
        if self.ds.r16(a(0x1D140)) != 0 {
            self.r.dx = 0x4E;
            return;
        }
        self.r.dx = 0x59;
        if self.ds.r8(a(0x274D8)) == 1 {
            let v = self.ds.r16(a(0x1D142));
            if v < 3 {
                self.ds.w16(a(0x1D142), v.wrapping_add(1));
            } else {
                self.ds.w16(a(0x1D142), 0);
            }
            return;
        }
        if self.ds.r8(a(0x274D6)) == 1 {
            let v = self.ds.r16(a(0x1D142));
            if v > 0 {
                self.ds.w16(a(0x1D142), v.wrapping_sub(1));
            } else {
                self.ds.w16(a(0x1D142), 3);
            }
            return;
        }
        self.r.dx = 0x4E;
    }

    /// `sub_12672`：選中第 1 列（角色）時，左右鍵改 `word_1D144`（0 至 2 環繞）。`dx` 為 `Y` 表示有變動。
    pub(crate) fn s12672(&mut self) {
        if self.ds.r16(a(0x1D140)) != 1 {
            self.r.dx = 0x4E;
            return;
        }
        self.r.dx = 0x59;
        if self.ds.r8(a(0x274D8)) == 1 {
            let v = self.ds.r16(a(0x1D144));
            if v < 2 {
                self.ds.w16(a(0x1D144), v.wrapping_add(1));
            } else {
                self.ds.w16(a(0x1D144), 0);
            }
            return;
        }
        if self.ds.r8(a(0x274D6)) == 1 {
            let v = self.ds.r16(a(0x1D144));
            if v > 0 {
                self.ds.w16(a(0x1D144), v.wrapping_sub(1));
            } else {
                self.ds.w16(a(0x1D144), 2);
            }
            return;
        }
        self.r.dx = 0x4E;
    }

    /// `sub_126B8`：畫選單。選中的列用 `sub_18C6E`（反相），沒選中的用 `sub_18C0A`。最後等 1 加 7 tick。
    pub(crate) async fn s126b8(&mut self) {
        self.t_x(0x14);
        self.t_y(0x1E);
        self.t_i(0x4D);
        if self.ds.r16(a(0x1D140)) != 0 {
            self.s18c0a();
        } else {
            self.s18c6e();
        }
        self.t_x(0x14);
        self.t_y(0x28);
        self.t_i(0x52);
        if self.ds.r16(a(0x1D140)) != 1 {
            self.s18c0a();
        } else {
            self.s18c6e();
        }
        self.t_x(0x1C);
        self.t_y(0x1E);
        self.r.ax = self.ds.r16(a(0x1D142)).wrapping_add(0x4E);
        self.t_i(self.r.ax);
        self.s18c0a();
        self.t_x(0x1C);
        self.t_y(0x28);
        self.r.ax = self.ds.r16(a(0x1D144)).wrapping_add(0x53);
        self.t_i(self.r.ax);
        self.s18c0a();
        self.t_x(3);
        self.t_y(0x1E);
        self.r.ax = self.ds.r16(a(0x1D142)).wrapping_add(0x49);
        self.t_i(self.r.ax);
        self.s18c0a();
        self.s12754();
        self.r.cx = 1;
        self.s11097().await;
        self.r.cx = 7;
        self.s11097().await;
    }

    /// `sub_12754`：難度 0 至 3 對應 `word_1E6B4`、`word_1E6B2`（敵彈密度與血量）。
    pub(crate) fn s12754(&mut self) {
        match self.ds.r16(a(0x1D142)) {
            0 => {
                self.ds.w16(a(0x1E6B4), 0);
                self.ds.w16(a(0x1E6B2), 0);
            }
            1 => {
                self.ds.w16(a(0x1E6B4), 2);
                self.ds.w16(a(0x1E6B2), 0);
            }
            2 => {
                self.ds.w16(a(0x1E6B4), 2);
                self.ds.w16(a(0x1E6B2), 1);
            }
            3 => {
                self.ds.w16(a(0x1E6B4), 3);
                self.ds.w16(a(0x1E6B2), 1);
            }
            _ => {}
        }
    }

    /// `sub_127A5`：角色選項 0、1、2 對應 `byte_1CFDD` 的 01h、10h、11h（玩家 1、玩家 2、雙人）。
    pub(crate) fn s127a5(&mut self) {
        match self.ds.r16(a(0x1D144)) {
            0 => self.ds.w8(a(0x1CFDD), 0x01),
            1 => self.ds.w8(a(0x1CFDD), 0x10),
            2 => self.ds.w8(a(0x1CFDD), 0x11),
            _ => {}
        }
    }

    /// `sub_127CD`：載入 J3，第 0 項寫進暫存區 B，再用 200 次 `sub_19D08` 由下往上滑入。
    pub(crate) fn s127cd(&mut self) {
        // sub_12B49、sub_12BCF：DOS 記憶體配置，不移植。
        self.s12b49();
        self.s1b032();
        self.ds.w16(a(0x2685C), 0x8000);
        self.s1152d();
        self.s12a9d();
        self.t_x(0);
        self.t_y(0);
        self.t_i(0);
        self.s19b40();
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.t_x(0);
        self.ds.w16(a(0x26868), 0);
        self.t_y(0xC8);
        self.s19a88();
        self.s18819();
        self.r.cx = 0xC8;
        loop {
            let saved = self.r.cx;
            self.r.cx = 0xC8;
            self.s19d08();
            let y = self.ds.r16(a(0x26882)).wrapping_sub(1);
            self.t_y(y);
            self.r.cx = saved;
            if !self.t_loop() {
                break;
            }
        }
    }

    /// `sub_1282D`：把暫存區 B 整頁複製到目前的頁，來源與位移都歸零。
    pub(crate) fn s1282d(&mut self) {
        self.ds.w16(a(0x2685C), 0x8000);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.t_x(0);
        self.ds.w16(a(0x26868), 0);
        self.t_y(0);
        self.r.cx = 0xC8;
        self.s19d08();
    }

    /// `sub_12858`：載入 J1，第 2 項寫進暫存區 B，設 `j1` 調色盤。
    pub(crate) fn s12858(&mut self) {
        // sub_12B49、sub_12BCF：DOS 記憶體配置，不移植。
        self.s12b49();
        self.s1b008();
        self.ds.w16(a(0x2685C), 0x8000);
        self.s1152d();
        self.s12a41();
        self.t_x(0);
        self.t_y(0);
        self.t_i(2);
        self.s19b40();
    }

    /// `sub_12883`：載入 J2，第 1 項，`j2` 調色盤。
    pub(crate) fn s12883(&mut self) {
        // sub_12B49、sub_12BCF：DOS 記憶體配置，不移植。
        self.s12b49();
        self.s1b01d();
        self.ds.w16(a(0x2685C), 0x8000);
        self.s1152d();
        self.s12a6f();
        self.t_x(0);
        self.t_y(0);
        self.t_i(1);
        self.s19b40();
    }

    /// `sub_128AE`：載入 J4，第 3 項，`j4` 調色盤。
    pub(crate) fn s128ae(&mut self) {
        // sub_12B49、sub_12BCF：DOS 記憶體配置，不移植。
        self.s12b49();
        self.s1b047();
        self.ds.w16(a(0x2685C), 0x8000);
        self.s1152d();
        self.s12acb();
        self.t_x(0);
        self.t_y(0);
        self.t_i(3);
        self.s19b40();
    }

    /// `sub_128D9`：清顯示頁 0（`A000:0000`，`0FA0h` 個字組，值 0）。E 模式的 `dseg` 沒有副作用。
    /// 目前沒有可達的呼叫（只在 `122BEh` 的死碼裡）。
    pub(crate) fn s128d9(&mut self) {
        self.s18878();
        self.draws.push(DrawCmd::VramFill { off: 0, words: 0x0FA0, value: 0 });
        // rep stosw 之後：ax = 0、cx = 0、di = 1F40h。
        self.r.ax = 0;
        self.r.cx = 0;
        self.r.di = 0x1F40;
    }

    /// `sub_1291A`：清顯示頁 1（`A800:0000`，即頁偏移 8000h，`0FA0h` 個字組，值 0）。
    /// 原版不碰 map mask，進來時已是 0Fh（`sub_19B40` 結尾的 `sub_18878`），所以這裡的 `sub_18878` 不改變結果，
    /// 只是讓 `VramFill` 的平面範圍在命令串裡看得見。
    pub(crate) fn s1291a(&mut self) {
        self.s18878();
        self.draws.push(DrawCmd::VramFill { off: 0x8000, words: 0x0FA0, value: 0 });
        // rep stosw 之後：ax = 0、cx = 0、di = 1F40h。
        self.r.ax = 0;
        self.r.cx = 0;
        self.r.di = 0x1F40;
    }

    /// `sub_1295B`：`title_d` 調色盤。
    pub(crate) fn s1295b(&mut self) {
        self.t_pal(0x5A9);
    }

    /// `sub_12989`：`title_e` 調色盤。
    pub(crate) fn s12989(&mut self) {
        self.t_pal(0x5EA);
    }

    /// `sub_129B7`：`menu` 調色盤。
    pub(crate) fn s129b7(&mut self) {
        self.t_pal(0x62B);
    }

    /// `sub_129E5`：`beg1_a` 調色盤。
    pub(crate) fn s129e5(&mut self) {
        self.t_pal(0x66C);
    }

    /// `sub_12A13`：`beg1_b` 調色盤。
    pub(crate) fn s12a13(&mut self) {
        self.t_pal(0x6AD);
    }

    /// `sub_12A41`：`j1` 調色盤。
    pub(crate) fn s12a41(&mut self) {
        self.t_pal(0x6EE);
    }

    /// `sub_12A6F`：`j2` 調色盤。
    pub(crate) fn s12a6f(&mut self) {
        self.t_pal(0x72F);
    }

    /// `sub_12A9D`：`j3` 調色盤。
    pub(crate) fn s12a9d(&mut self) {
        self.t_pal(0x770);
    }

    /// `sub_12ACB`：`j4` 調色盤。`12AF9h` 起的淡出碼沒有入口。
    pub(crate) fn s12acb(&mut self) {
        self.t_pal(0x7B1);
    }

    /// `sub_12B3A`：INT 10h AX=1012h，從 DAC 暫存器 `bx` 起寫 16 個，表在 `ds:dx`。
    pub(crate) fn s12b3a(&mut self) {
        self.r.set_ah(0x10);
        self.r.set_al(0x12);
        self.r.cx = 0x10;
        self.set_dac(self.r.bx as u8, self.r.dx);
    }

    /// `sub_12B49`：DOS 記憶體配置（檔名工作區與四個緩衝區段落），不移植。
    /// 寫入的 `1CEF6h` 至 `1CF0Ah` 與 `1CF46h` 至 `1CF4Ch` 都在 `A0_EXEMPT`，
    /// 圖庫由 `loader.rs` 的組別切換代替。
    pub(crate) fn s12b49(&mut self) {}
}

/// 冒煙測試：標題流程在無輸入與少量輸入下的等待序列，和機器碼的呼叫順序一致。
/// 需要原版檔，由 `tools/rust.sh test -p rs2-core smoke -- --ignored --nocapture` 跑。
/// 這不取代 `rs2-trace` 的全流程對拍（規格 002 B2）。
#[cfg(test)]
mod smoke {
    use crate::game::{Entry, Fixes, Frame, Game, GameFiles, Input};

    fn load() -> Game {
        let dir = std::path::PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh 跑"));
        let exe = std::fs::read(dir.join("S.EXE")).unwrap();
        let image = rs2_formats::unpack_s_exe(&exe).unwrap();
        let dm1 = std::fs::read(dir.join("DM1.LCK")).unwrap();
        let dm2 = std::fs::read(dir.join("DM2.LCK")).unwrap();
        let mut snap = vec![0u8; 0x10000];
        let src = &image[rs2_formats::DSEG_IMAGE_OFFSET..];
        let n = src.len().min(0x10000);
        snap[..n].copy_from_slice(&src[..n]);
        let files = GameFiles { image, dm1, dm2, ..GameFiles::default() };
        Game::from_snapshot(files, &snap, Entry::Title, Fixes::NONE)
    }

    /// 開場動畫的等待序列：`sub_11910` 結尾 1 tick 加 `sub_1199F` 的 15 次（含時間到才有的最後一次）。
    fn intro_first() -> Vec<u16> {
        let mut v = vec![1];
        v.extend([5, 5, 5, 4, 4, 3, 3, 2, 1, 1, 1, 1, 4, 0x14, 3]);
        v
    }

    /// 跑到等待序列長度達 `n` 再多跑 `extra` 步，`inputs(waits_so_far, step)` 給該步的掃描碼。
    fn drive(g: &mut Game, n: usize, mut inputs: impl FnMut(usize, usize) -> Vec<u8>) -> Vec<u16> {
        let mut waits: Vec<u16> = Vec::new();
        for step in 0..200_000 {
            let sc = inputs(waits.len(), step);
            let mut w2 = Vec::new();
            let f: Frame = g.step_full(&Input { scancodes: &sc }, &mut |_g, _pt| {}, &mut |_g, cx| w2.push(cx));
            waits.extend(w2);
            if waits.len() >= n || f.exit.is_some() {
                break;
            }
        }
        waits
    }

    #[test]
    #[ignore]
    fn no_input_reaches_demo_stage1() {
        let mut g = load();
        let mut want = intro_first();
        want.push(0x0F);
        want.extend([1, 0x0F]);
        want.extend([6, 3, 2, 2, 1, 8, 2, 2, 1, 0x0E]);
        want.extend([4; 10]);
        want.extend([1; 54]);
        want.push(2);
        let got = drive(&mut g, want.len(), |_, _| Vec::new());
        assert_eq!(&got[..want.len()], &want[..], "等待序列不同");
    }

    #[test]
    #[ignore]
    fn key_goes_through_stage1_card() {
        let mut g = load();
        let mut sent = false;
        let mut want = intro_first();
        want.extend([1; 54]);
        want.push(2);
        let got = drive(&mut g, want.len(), |n, step| {
            if step == 0 {
                vec![0x1E]
            } else if n == 16 && !sent {
                sent = true;
                vec![0x1E]
            } else {
                Vec::new()
            }
        });
        assert_eq!(&got[..want.len()], &want[..], "等待序列不同");
    }

    #[test]
    #[ignore]
    fn menu_changes_difficulty_and_returns() {
        let mut g = load();
        let mut want = intro_first();
        want.extend([1, 7]);
        want.extend([1, 7]);
        want.push(1);
        let mut phase = 0;
        let got = drive(&mut g, want.len(), |n, step| {
            if step == 0 {
                return vec![0x2E];
            }
            match (phase, n) {
                (0, 17) => {
                    phase = 1;
                    vec![0x4D]
                }
                (1, 18) => {
                    phase = 2;
                    vec![0xCD]
                }
                (2, 19) => {
                    phase = 3;
                    vec![0x1C]
                }
                _ => Vec::new(),
            }
        });
        assert_eq!(&got[..want.len()], &want[..], "等待序列不同");
        assert_eq!(g.ds.r16(0x0582), 2, "難度應從 1 變 2");
    }
}

/// 對 `wt_title` 收據的暫時比對：標題、DM1 的第一關示範、第二關的過場卡（不含第二關本體）。
/// 比 `cx`、幀計數與資料段。資料段的豁免用 `DS_EXEMPT`（`A7F9h` 收據記在歸零之前）。
/// 全流程的正式對拍在 `rs2-trace` 的 `wait` 測試，這裡只用來提早抓錯。
#[cfg(test)]
mod receipt {
    use crate::game::{Entry, Fixes, Game, GameFiles, Input};

    fn u16le(b: &[u8], at: usize) -> usize {
        u16::from_le_bytes([b[at], b[at + 1]]) as usize
    }

    fn u32le(b: &[u8], at: usize) -> usize {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
    }

    #[test]
    #[ignore]
    fn title_demo1_and_stage2_card() {
        let orig = std::path::PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG"));
        let run = std::path::PathBuf::from(std::env::var("RS2_DOSRUN").expect("缺 RS2_DOSRUN")).join("wt_title");
        let exe = std::fs::read(orig.join("S.EXE")).unwrap();
        let image = rs2_formats::unpack_s_exe(&exe).unwrap();
        let dm1 = std::fs::read(orig.join("DM1.LCK")).unwrap();
        let dm2 = std::fs::read(orig.join("DM2.LCK")).unwrap();
        let snap = std::fs::read(run.join("entry.ds")).unwrap();
        let files = GameFiles { image, dm1, dm2, ..GameFiles::default() };
        let mut g = Game::from_snapshot(files, &snap, Entry::Title, Fixes::NONE);

        // 收據：cx、frame。
        let text = std::fs::read_to_string(run.join("wait-wt_title.jsonl")).unwrap();
        let field = |line: &str, key: &str| -> u64 {
            let k = line.find(&format!("\"{key}\":")).unwrap() + key.len() + 3;
            let rest = &line[k..];
            let end = rest.find([',', '}']).unwrap();
            rest[..end].parse().unwrap()
        };
        let rows: Vec<(u16, u16)> = text.lines().map(|l| (field(l, "cx") as u16, field(l, "frame") as u16)).collect();
        // 第一個超過 100 筆的 cx=2 區段是第一關示範，接著的 cx=1 區段是第二關過場卡。止於過場卡結束。
        let mut i = 0;
        let mut end = None;
        while i < rows.len() {
            if rows[i].0 == 2 {
                let mut j = i;
                while j < rows.len() && rows[j].0 == 2 {
                    j += 1;
                }
                if j - i > 100 {
                    let mut k = j;
                    while k < rows.len() && rows[k].0 == 1 {
                        k += 1;
                    }
                    end = Some(k);
                    break;
                }
                i = j;
            } else {
                i += 1;
            }
        }
        let end = end.expect("收據裡找不到第二關過場卡");

        // 資料段重建。
        let dsd = std::fs::read(run.join("wait-wt_title.dsd")).unwrap();
        let mut cur = vec![0u8; 0x10000];
        let mut at = 0usize;
        let mut ds_by_n: Vec<Vec<u8>> = Vec::new();
        while at < dsd.len() && ds_by_n.len() < end {
            let n = u32le(&dsd, at);
            let cnt = u32le(&dsd, at + 4);
            at += 8;
            for _ in 0..cnt {
                let off = u16le(&dsd, at);
                let len = u16le(&dsd, at + 2);
                at += 4;
                cur[off..off + len].copy_from_slice(&dsd[at..at + len]);
                at += len;
            }
            assert_eq!(n, ds_by_n.len());
            ds_by_n.push(cur.clone());
        }

        let exempt = |o: usize| -> bool { crate::game::ds_exempt(o as u16) };

        let mut n = 0usize;
        let mut bad: Vec<String> = Vec::new();
        let mut diff_total = 0usize;
        // 偏移 -> (第一筆, 筆數, 第一筆的重製值, 收據值)
        let mut by_off: std::collections::BTreeMap<usize, (usize, usize, u8, u8)> = std::collections::BTreeMap::new();
        for step in 0..400_000 {
            let mut hits: Vec<(u16, u16, Vec<u8>)> = Vec::new();
            let f = g.step_full(&Input { scancodes: &[] }, &mut |_g, _pt| {}, &mut |g, cx| {
                hits.push((cx, g.ds.r16(0x0422), g.ds.bytes().to_vec()));
            });
            for (cx, frame, ds) in hits {
                if n >= end {
                    break;
                }
                if (cx, frame) != rows[n] {
                    bad.push(format!("n={n} cx/frame 重製 {cx}/{frame} 收據 {}/{}", rows[n].0, rows[n].1));
                }
                let want = &ds_by_n[n];
                let diffs: Vec<usize> = (0..0x10000).filter(|&o| ds[o] != want[o] && !exempt(o)).collect();
                diff_total += diffs.len();
                for &o in &diffs {
                    let e = by_off.entry(o).or_insert((n, 0usize, ds[o], want[o]));
                    e.1 += 1;
                }
                n += 1;
            }
            if n >= end || f.exit.is_some() {
                let _ = step;
                break;
            }
        }
        for b in &bad {
            println!("{b}");
        }
        for (o, (first, cnt, got, want)) in &by_off {
            println!("偏移 {o:04X} 第一筆 n={first} 共 {cnt} 筆 重製 {got:02X} 收據 {want:02X}");
        }
        println!("比對 {n} 筆（目標 {end}），資料段非豁免差異 {diff_total} 位元組，{} 個偏移，回報 {} 行", by_off.len(), bad.len());
        assert!(n >= end, "沒跑滿");
        assert!(bad.is_empty() && by_off.is_empty(), "有差異");
    }
}
