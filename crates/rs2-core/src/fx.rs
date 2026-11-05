//! 特效池。
//! 逐函式移植自機器碼，查法見 `tools/q.sh`。規格：`docs/spec/001-rust-first-stage.md`。
//!
//! 池 30 槽：動畫指標 `dseg:403Ah`，x `3FC2h`，y `3FFEh`，各槽一字組。
//! 動畫表每筆 4 字組 `(圖, dx, dy, 形狀指標)`，圖為 0Fh 結束。

use crate::draw::Pool;
use crate::dseg::a;
use crate::game::Game;

impl Game {
    /// `mov ax, 動畫表 / call sub_17C6C`：`sub_17A3A` 每個分支的共同片段。
    fn fx_start(&mut self, table: u16) {
        self.r.ax = table;
        self.s17c6c();
    }

    /// `sub_17A3A`：依 `word_20B78` 的字母在 (`word_20B7C`, `word_20B7E`) 起一個或多個特效。
    /// `S`、`>`、未知字母為火花 `4094h`，`<` 為 `40B2h`，`B` 大爆炸，`V` 兩團，`E` 尾跡與小爆，`A` 追蹤彈尾跡，`G` 玩家死亡，`e` 為過關連爆。
    /// `17A64h` 的第二個 `E` 比較被 `17A4Fh` 攔截，其後的 10 組連爆（`17AA3h`）不可達，不移植。
    pub(crate) fn s17a3a(&mut self) {
        match self.ds.r16(a(0x20B78)) {
            0x53 => self.fx_start(0x4094),
            0x42 => self.fx_start(0x40D0),
            0x56 => {
                self.fx_start(0x40EE);
                let v = self.ds.r16(a(0x20B7C)).wrapping_add(6);
                self.ds.w16(a(0x20B7C), v);
                self.fx_start(0x40EE);
            }
            0x45 => self.fx_start(0x413C),
            0x41 => self.fx_start(0x4162),
            0x47 => self.fx_start(0x4188),
            0x3C => self.fx_start(0x40B2),
            0x65 => self.l_17b99(),
            // 0x3E 與其他字母：loc_17B8B。
            _ => self.fx_start(0x4094),
        }
    }

    /// `17B99h`（`sub_17A3A` 的 `e` 分支）：以 `word_20B7A` 為基底起 10 個特效，每個是 (x 位移, y, 動畫表)。
    fn l_17b99(&mut self) {
        const PIECES: [(u16, u16, u16); 10] = [
            (0x02, 0x07, 0x40D0),
            (0x0F, 0x05, 0x4188),
            (0x11, 0x0A, 0x40EE),
            (0x1C, 0x0B, 0x40D0),
            (0x21, 0x00, 0x413C),
            (0x03, 0x0C, 0x4162),
            (0x22, 0x10, 0x4188),
            (0x24, 0x10, 0x40EE),
            (0x21, 0x02, 0x413C),
            (0x19, 0x07, 0x40D0),
        ];
        for (dx, y, table) in PIECES {
            self.r.ax = self.ds.r16(a(0x20B7A)).wrapping_add(dx);
            self.ds.w16(a(0x20B7C), self.r.ax);
            self.ds.w16(a(0x20B7E), y);
            self.fx_start(table);
        }
    }

    /// `sub_17C6C`：把 `ax` 指向的動畫表與 (`word_20B7C`, `word_20B7E`) 存進第一個空槽，30 槽全滿就放棄。
    pub(crate) fn s17c6c(&mut self) {
        self.ds.w16(a(0x20B74), self.r.ax);
        self.r.cx = 0x1E;
        self.r.bx = 0;
        loop {
            self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x403A));
            if self.r.ax == 0 {
                break;
            }
            self.r.bx = self.r.bx.wrapping_add(2);
            self.r.cx = self.r.cx.wrapping_sub(1);
            if self.r.cx == 0 {
                return;
            }
        }
        let bx = self.r.bx;
        self.r.ax = self.ds.r16(a(0x20B7C));
        self.ds.w16(bx.wrapping_add(0x3FC2), self.r.ax);
        self.r.ax = self.ds.r16(a(0x20B7E));
        self.ds.w16(bx.wrapping_add(0x3FFE), self.r.ax);
        self.r.ax = self.ds.r16(a(0x20B74));
        self.ds.w16(bx.wrapping_add(0x403A), self.r.ax);
        self.bump_gen(Pool::Effect, (bx >> 1) as u8);
    }

    /// `sub_17C9A`：每幀走 30 個特效槽，動畫指標不為 0 的槽交給 `sub_17CB6`。
    pub(crate) fn s17c9a(&mut self) {
        self.r.cx = 0x1E;
        self.r.bx = 0;
        loop {
            let (cx, bx) = (self.r.cx, self.r.bx);
            self.r.ax = self.ds.r16(bx.wrapping_add(0x403A));
            if self.r.ax != 0 {
                self.set_obj(Pool::Effect, (bx >> 1) as u8);
                self.s17cb6();
                self.clear_obj();
            }
            self.r.bx = bx.wrapping_add(2);
            self.r.cx = cx.wrapping_sub(1);
            if self.r.cx == 0 {
                break;
            }
        }
    }

    /// `sub_17CB6`：第 `bx` 槽走一步。讀動畫指標處 4 字組 `(圖, dx, dy, 形狀)`，指標先加 8；
    /// 圖為 0Fh 就清槽，否則 x、y 累加後（y 乘 10 為像素）畫遮罩與平面，並把形狀寫進 G3。
    pub(crate) fn s17cb6(&mut self) {
        let bx = self.r.bx;
        self.r.ax = self.ds.r16(bx.wrapping_add(0x3FC2));
        self.ds.w16(a(0x20B7C), self.r.ax);
        self.r.ax = self.ds.r16(bx.wrapping_add(0x3FFE));
        self.ds.w16(a(0x20B7E), self.r.ax);
        self.r.ax = self.ds.r16(bx.wrapping_add(0x403A));
        self.ds.w16(a(0x20B80), self.r.ax);
        self.r.si = self.r.ax;
        self.r.ax = self.r.ax.wrapping_add(8);
        self.ds.w16(bx.wrapping_add(0x403A), self.r.ax);
        // lodsw：圖。
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        if self.r.ax == 0x0F {
            self.r.ax = 0;
            self.ds.w16(bx.wrapping_add(0x403A), self.r.ax);
            return;
        }
        self.ds.w16(a(0x26888), self.r.ax);
        // lodsw：dx。
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        self.r.ax = self.r.ax.wrapping_add(self.ds.r16(a(0x20B7C)));
        self.ds.w16(bx.wrapping_add(0x3FC2), self.r.ax);
        self.ds.w16(a(0x20B7C), self.r.ax);
        self.ds.w16(a(0x26880), self.r.ax);
        // lodsw：dy。
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        self.r.ax = self.r.ax.wrapping_add(self.ds.r16(a(0x20B7E)));
        self.ds.w16(bx.wrapping_add(0x3FFE), self.r.ax);
        self.ds.w16(a(0x20B7E), self.r.ax);
        self.r.bx = 0x0A;
        // mul bx：dx:ax = ax * bx，無號。
        let p = self.r.ax as u32 * self.r.bx as u32;
        self.r.ax = p as u16;
        self.r.dx = (p >> 16) as u16;
        self.ds.w16(a(0x26882), self.r.ax);
        // lodsw：形狀指標。
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        self.ds.w16(a(0x20B76), self.r.ax);
        self.s18e30();
        self.s18dcc();
        self.s17d2c();
        // 17D0Dh 起的 push、pop 成對（一個已移除的音效呼叫），只留下 ax = 7，堆疊殘值在映像外。
        self.r.ax = 7;
    }

    /// `sub_17D2C`：把形狀（`word_20B76` 指向的 `寬, 高, 寬 × 高 位元組`）以 `or` 寫進 G3，
    /// 起點是 (`word_20B7C`, `word_20B7E`)。位址落在 `964Ch` 至 `9C3Ch` 之外的格略過。
    pub(crate) fn s17d2c(&mut self) {
        self.r.si = self.ds.r16(a(0x20B76));
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        self.ds.w16(a(0x2687E), self.r.ax);
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x20B7E));
        loop {
            let cx = self.r.cx;
            self.s17d5f();
            loop {
                if self.r.di <= 0x9C3C && self.r.di >= 0x964C {
                    self.r.set_al(self.ds.r8(self.r.si));
                    let v = self.ds.r8(self.r.di) | self.r.al();
                    self.ds.w8(self.r.di, v);
                }
                self.r.di = self.r.di.wrapping_add(2);
                self.r.si = self.r.si.wrapping_add(1);
                self.r.cx = self.r.cx.wrapping_sub(1);
                if self.r.cx == 0 {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = cx;
            self.r.cx = self.r.cx.wrapping_sub(1);
            if self.r.cx == 0 {
                break;
            }
        }
    }

    /// `sub_17D5F`：G3 列起點。列號（無號）大於等於 14h 一律用 14h，
    /// `di` = `964Ch` + 列偏移（`21C2h` 表）+ 2 × `word_20B7C` + 1，`cx` = 形狀寬。`bx` 還原。
    pub(crate) fn s17d5f(&mut self) {
        let saved_bx = self.r.bx;
        if self.r.bx >= 0x14 {
            self.r.bx = 0x14;
        }
        self.r.bx = self.r.bx.wrapping_shl(1);
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x21C2));
        self.r.ax = self.r.ax.wrapping_add(0x964C);
        self.r.di = self.r.ax;
        self.r.ax = self.ds.r16(a(0x20B7C));
        self.r.ax = self.r.ax.wrapping_shl(1);
        self.r.ax = self.r.ax.wrapping_add(1);
        self.r.di = self.r.di.wrapping_add(self.r.ax);
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.bx = saved_bx;
    }
}
