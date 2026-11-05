//! 頭目段背景的捲動與八向腳本（原版 `sub_15906` 一族）。
//!
//! 這一組原本和玩家擠在 `player.rs`，但不屬於玩家領域：第一關的頭目段背景（`sub_15906`）、第二關與第三關的合成層
//! （`stage2.rs`、`stage3.rs`）共用腳本直譯器 `sub_15BB0`、`sub_15BCB` 與它的工作變數
//! （`word_1DB5B`、`1DB5D`、`1DB6B`、`byte_1DB5A`）。搬到這裡只為了讓玩家領域乾淨，內容沒有重寫，
//! 仍是逐函式照機器碼移植（暫存器、資料段位址），留給關卡領域跟 `stage2.rs`、`stage3.rs` 的呼叫端一起重寫。

use crate::draw::Pool;
use crate::dseg::a;
use crate::game::Core;

impl Core {
    /// `add word ptr [linear], v`（`sub` 以 `wrapping_neg` 傳入）。
    fn boss_addw(&mut self, linear: u32, v: u16) {
        let n = self.ds.r16(a(linear)).wrapping_add(v);
        self.ds.w16(a(linear), n);
    }

    /// `sub_15906`：第一關頭目段的背景：清頁後畫頭目捲動背景。
    /// 同位置的 `15912`（第二關）與 `15924`（第三關）是各自的入口，不在本函式內。
    pub(crate) fn s15906(&mut self) {
        self.s19cf9();
        self.s15930();
    }

    /// `sub_15930`：第一關頭目背景的捲動：幀計數不超過 26Eh 時重設，之後讀腳本位元組（`word_1DB6D`）逐幀移動，
    /// 腳本結束位元組 0Fh 時設 `byte_1CFAD` = `F`（過關連爆）。
    pub(crate) fn s15930(&mut self) {
        if self.ds.r16(a(0x1CFE2)) <= 0x26E {
            self.ds.w16(a(0x1DB5F), 0xC8);
            self.ds.w16(a(0x1DB61), 0xFFEC);
            self.ds.w16(a(0x1DB6D), 0x0FB3);
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
        self.s15ac5();
        if self.ds.r8(a(0x1DB5A)) == 0x0F {
            self.ds.w8(a(0x1CFAD), 0x46);
        }
    }

    /// `sub_15AC5`：把暫存區 B（`word_2685C` = 8000h）的整幅背景以水平位移 `-word_1DB5D` 畫到目前頁，共 C1h 列。
    pub(crate) fn s15ac5(&mut self) {
        self.ds.w16(a(0x2685C), 0x8000);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.ds.w16(a(0x26880), 0);
        self.r.ax = 0;
        self.r.ax = self.r.ax.wrapping_sub(self.ds.r16(a(0x1DB5D)));
        self.ds.w16(a(0x26868), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB5B));
        self.ds.w16(a(0x26882), self.r.ax);
        self.r.cx = 0xC1;
        self.set_obj(Pool::Boss, 0);
        self.s19d08();
        self.clear_obj();
    }

    /// `sub_15BB0`：讀腳本位元組（`word_1DB6B` 指向），存 `byte_1DB5A`。不是結束位元組 0Fh 就前進並執行該指令，
    /// 是 0Fh 則以指令 0 執行（不前進）。
    pub(crate) fn s15bb0(&mut self) {
        self.r.si = self.ds.r16(a(0x1DB6B));
        self.r.set_al(self.ds.r8(self.r.si));
        self.ds.w8(a(0x1DB5A), self.r.al());
        if self.r.al() == 0x0F {
            self.r.set_al(0);
            self.s15bcb();
            return;
        }
        self.boss_addw(0x1DB6B, 1);
        self.s15bcb();
    }

    /// `sub_15BCB`：八向指令 1 至 8（上、右上、右、右下、下、左下、左、左上）對 `word_1DB5B`（y，每步 0Ah）與
    /// `word_1DB5D`（x，每步 1）的增減。`byte_1CFAD` 為 `F` 時指令一律視為 0。
    pub(crate) fn s15bcb(&mut self) {
        self.s15c1b();
        match self.r.al() {
            1 => self.s15c25(),
            2 => {
                self.s15c25();
                self.s15c36();
            }
            3 => self.s15c36(),
            4 => {
                self.s15c2b();
                self.s15c36();
            }
            5 => self.s15c2b(),
            6 => {
                self.s15c2b();
                self.s15c31();
            }
            7 => self.s15c31(),
            8 => {
                self.s15c25();
                self.s15c31();
            }
            _ => {}
        }
    }

    /// `sub_15C1B`：`byte_1CFAD` 為 `F` 時 `al` 歸 0。
    pub(crate) fn s15c1b(&mut self) {
        if self.ds.r8(a(0x1CFAD)) == 0x46 {
            self.r.set_al(0);
        }
    }

    /// `sub_15C25`：`word_1DB5B` 減 0Ah。
    pub(crate) fn s15c25(&mut self) {
        self.boss_addw(0x1DB5B, 0x0Au16.wrapping_neg());
    }

    /// `sub_15C2B`：`word_1DB5B` 加 0Ah。
    pub(crate) fn s15c2b(&mut self) {
        self.boss_addw(0x1DB5B, 0x0A);
    }

    /// `sub_15C31`：`word_1DB5D` 減 1。
    pub(crate) fn s15c31(&mut self) {
        self.boss_addw(0x1DB5D, 1u16.wrapping_neg());
    }

    /// `sub_15C36`：`word_1DB5D` 加 1。
    pub(crate) fn s15c36(&mut self) {
        self.boss_addw(0x1DB5D, 1);
    }
}
