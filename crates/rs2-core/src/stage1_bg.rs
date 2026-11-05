//! 第一關背景五層與實體繪製。原本與玩家子彈放在同一個檔案，玩家彈領域重寫時整段原封不動搬出，
//! 留給之後的敵人與關卡領域。逐函式移植自機器碼，查法見 `tools/q.sh`。規格：`docs/spec/001-rust-first-stage.md`。
//!
//! 位址慣例：`a(0x1F1A8)` 是 IDA 線性位址名稱 `word_1F1A8`，直接寫偏移的（`0x2294`、`0x1B32`）
//! 是機器碼裡的 `ds:` 立即值。

use crate::draw::Pool;
use crate::dseg::a;
use crate::game::Core;

impl Core {
    /// `sub_16302`：第一關背景五層（天空、波浪、倒懸岩、山影遮罩、山影圖），再填上方空白。
    pub(crate) fn s16302(&mut self) {
        // 天空（層 0）。
        self.set_obj(Pool::Background, 0);
        self.ds.w16(a(0x2685C), 0);
        self.ds.w16(a(0x26888), 0x0F);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0x6E);
        self.ds.w16(a(0x26880), 0);
        let ax = self.ds.r16(a(0x1E6BA));
        self.r.ax = ax;
        self.ds.w16(a(0x26882), ax);
        self.r.cx = 0x59;
        self.ds.w16(a(0x26868), 0);
        self.s19ba4();
        self.s1641b();
        // 波浪（層 1）。
        self.set_obj(Pool::Background, 1);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.ds.w16(a(0x26880), 0);
        let ax = self.ds.r16(a(0x1E6B8)).wrapping_add(0x95);
        self.ds.w16(a(0x26882), ax);
        self.r.cx = 0x29;
        let ax = self.ds.r16(a(0x1E6BC));
        self.r.ax = ax;
        self.ds.w16(a(0x26868), ax);
        self.s19ba4();
        self.s1643f();
        // 倒懸岩（層 2）。
        self.set_obj(Pool::Background, 2);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0x28);
        self.ds.w16(a(0x26880), 0);
        let ax = self.ds.r16(a(0x1E6B8)).wrapping_add(0x52);
        self.ds.w16(a(0x26882), ax);
        self.r.cx = 0x45;
        let ax = self.ds.r16(a(0x1E6BE));
        self.r.ax = ax;
        self.ds.w16(a(0x26868), ax);
        self.s19ba4();
        // 山影遮罩（層 3），AND。
        self.set_obj(Pool::Background, 3);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0xF3);
        self.ds.w16(a(0x26880), 0);
        let ax = self.ds.r16(a(0x1E6B8)).wrapping_add(0x28);
        self.ds.w16(a(0x26882), ax);
        self.r.cx = 0x2B;
        let ax = self.ds.r16(a(0x1E6BE));
        self.r.ax = ax;
        self.ds.w16(a(0x26868), ax);
        self.s1a09f();
        // 山影圖（層 4），OR。
        self.set_obj(Pool::Background, 4);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0xC8);
        self.ds.w16(a(0x26880), 0);
        let ax = self.ds.r16(a(0x1E6B8)).wrapping_add(0x28);
        self.ds.w16(a(0x26882), ax);
        self.r.cx = 0x2B;
        let ax = self.ds.r16(a(0x1E6BE));
        self.r.ax = ax;
        self.ds.w16(a(0x26868), ax);
        self.s19da8();
        self.clear_obj();
        self.s16453();
        if self.ds.r16(a(0x1CFE2)) <= 0xFA {
            self.ds.w16(a(0x1E6B8), 0);
            self.ds.w16(a(0x1E6BA), 0);
            return;
        }
        // 163EC：計數超過 250 後所有層一起下移。
        let v = self.ds.r16(a(0x1E6B8)).wrapping_add(1);
        self.ds.w16(a(0x1E6B8), v);
        if self.ds.r16(a(0x1CFE2)) <= 0x113 {
            return;
        }
        // 163F9：只有 word_26872 非 0 的頁先加 1，填色列數用加完的值。
        if self.ds.r16(a(0x26872)) != 0 {
            let v = self.ds.r16(a(0x1E6BA)).wrapping_add(1);
            self.ds.w16(a(0x1E6BA), v);
        }
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26882), 0);
        self.r.cx = self.ds.r16(a(0x1E6BA));
        self.r.dx = 0;
        self.s1a287();
    }

    /// `sub_1641B`：計數超過 250 時，在天空下方以色號 1 填 `word_1E6B8` 列。屬於天空層。
    pub(crate) fn s1641b(&mut self) {
        if self.ds.r16(a(0x1CFE2)) <= 0xFA {
            return;
        }
        self.ds.w16(a(0x26880), 0);
        let ax = 0x59u16.wrapping_add(self.ds.r16(a(0x1E6BA)));
        self.r.ax = ax;
        self.ds.w16(a(0x26882), ax);
        self.r.cx = self.ds.r16(a(0x1E6B8));
        self.r.dx = 1;
        self.s1a287();
    }

    /// `sub_1643F`：波浪相位 `word_1E6BC` 加 2，到 40 歸零。
    pub(crate) fn s1643f(&mut self) {
        let v = self.ds.r16(a(0x1E6BC)).wrapping_add(2);
        self.ds.w16(a(0x1E6BC), v);
        if v >= 0x28 {
            self.ds.w16(a(0x1E6BC), 0);
        }
    }

    /// `sub_16453`：倒懸岩與山影相位 `word_1E6BE` 加 1，到 40 歸零。
    pub(crate) fn s16453(&mut self) {
        let v = self.ds.r16(a(0x1E6BE)).wrapping_add(1);
        self.ds.w16(a(0x1E6BE), v);
        if v >= 0x28 {
            self.ds.w16(a(0x1E6BE), 0);
        }
    }

    /// `sub_164BB`：畫 15 個實體的本體（遮罩加平面）。
    pub(crate) fn s164bb(&mut self) {
        self.r.bx = 0;
        self.r.cx = 0x0F;
        loop {
            let cx = self.r.cx;
            self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x2294));
            if self.r.ax != 0 {
                let bx = self.r.bx;
                self.set_obj(Pool::Entity, (bx >> 1) as u8);
                self.s164f3();
                self.clear_obj();
                self.r.bx = bx;
            }
            self.r.bx = self.r.bx.wrapping_add(2);
            self.r.cx = cx.wrapping_sub(1);
            if self.r.cx == 0 {
                break;
            }
        }
    }

    /// `sub_164D7`：畫 15 個實體的第二輪（只有種類 `S` 會畫，頭目段在實體之上）。
    pub(crate) fn s164d7(&mut self) {
        self.r.bx = 0;
        self.r.cx = 0x0F;
        loop {
            let cx = self.r.cx;
            self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x2294));
            if self.r.ax != 0 {
                let bx = self.r.bx;
                self.set_obj(Pool::Entity, (bx >> 1) as u8);
                self.s16541();
                self.clear_obj();
                self.r.bx = bx;
            }
            self.r.bx = self.r.bx.wrapping_add(2);
            self.r.cx = cx.wrapping_sub(1);
            if self.r.cx == 0 {
                break;
            }
        }
    }

    /// `sub_164F3`：畫 `bx` 指向的實體。種類 `N`、`K`、`S` 不在這一輪畫。
    /// 閃白旗標 `word_1E6B6` 為 `Y` 時只畫白色遮罩，否則遮罩後 y 加 1 再畫平面。
    pub(crate) fn s164f3(&mut self) {
        let bx = self.r.bx;
        let ax = self.ds.r16(bx.wrapping_add(0x21EC));
        self.ds.w16(a(0x26888), ax);
        let ax = self.ds.r16(bx.wrapping_add(0x220C));
        self.ds.w16(a(0x26880), ax);
        let ax = self.ds.r16(bx.wrapping_add(0x222C));
        self.ds.w16(a(0x26882), ax);
        let ax = self.ds.r16(bx.wrapping_add(0x226C));
        self.ds.w16(a(0x1E6B6), ax);
        let ax = self.ds.r16(bx.wrapping_add(0x224C));
        self.r.ax = ax;
        self.ds.w16(a(0x1F19A), ax);
        let kind = self.ds.r16(a(0x1F19A));
        if kind == 0x4E || kind == 0x4B || kind == 0x53 {
            return;
        }
        self.s18f1a();
        if self.ds.r16(a(0x1E6B6)) == 0x59 {
            self.s19025();
            return;
        }
        let y = self.ds.r16(a(0x26882)).wrapping_add(1);
        self.ds.w16(a(0x26882), y);
        self.s18f5d();
    }

    /// `sub_16541`：畫 `bx` 指向的實體的第二輪，只有種類 `S` 以 `sub_18FC1` 覆寫。
    pub(crate) fn s16541(&mut self) {
        let bx = self.r.bx;
        let ax = self.ds.r16(bx.wrapping_add(0x21EC));
        self.ds.w16(a(0x26888), ax);
        let ax = self.ds.r16(bx.wrapping_add(0x220C));
        self.ds.w16(a(0x26880), ax);
        let ax = self.ds.r16(bx.wrapping_add(0x222C));
        self.ds.w16(a(0x26882), ax);
        let ax = self.ds.r16(bx.wrapping_add(0x224C));
        self.r.ax = ax;
        self.ds.w16(a(0x1F19A), ax);
        let kind = self.ds.r16(a(0x1F19A));
        if kind == 0x4E || kind == 0x4B {
            return;
        }
        if kind == 0x53 {
            self.s18fc1();
        }
    }
}
