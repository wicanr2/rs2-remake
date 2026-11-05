//! 第一關背景五層。原本與玩家子彈放在同一個檔案，玩家彈領域重寫時整段原封不動搬出，留給之後的關卡領域；
//! 實體繪製（`sub_164BB`、`sub_164D7`、`sub_164F3`、`sub_16541`）已隨敵人領域重寫，搬到 `entity.rs`。
//! 逐函式移植自機器碼，查法見 `tools/q.sh`。規格：`docs/spec/001-rust-first-stage.md`。
//!
//! 位址慣例：`a(0x1F1A8)` 是 IDA 線性位址名稱 `word_1F1A8`。

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

}
