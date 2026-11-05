//! 背景繪製、實體繪製與玩家子彈。
//! 逐函式移植自機器碼，查法見 `tools/q.sh`。規格：`docs/spec/001-rust-first-stage.md`。
//!
//! 位址慣例：`a(0x1F1A8)` 是 IDA 線性位址名稱 `word_1F1A8`，直接寫偏移的（`0x2294`、`0x1B32`）
//! 是機器碼裡的 `ds:` 立即值。
//!
//! 玩家子彈池：20 槽，指標表 `ds:2607h`，記錄 16 字組（32 位元組），基底 `ds:262Fh`。
//! `sub_16779` 把記錄複製到工作區 `ds:28B1h`（`word_1F471` 起），`sub_1678E` 複製回去，
//! `word_1F46F`（`ds:28AFh`）存記錄位址，也是 `sub_16578` 讀取樣板位址的變數。

use crate::draw::Pool;
use crate::dseg::a;
use crate::fx::EffectKind;
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

    /// `sub_16578`：在玩家彈池找空槽，複製 `word_1F46F` 指的樣板，
    /// 寫入欄 `word_1F1C1`、列 `word_1F1C3`、`列 × 10`、型別 `word_1F1BC`。池滿則放棄。
    /// 結束時 `bx` = 10、`dx` = 乘法高位、`ax` = 型別，這是原版殘值。
    pub(crate) fn s16578(&mut self) {
        self.r.bx = 0;
        self.r.dx = 0x262F;
        self.r.cx = 0x14;
        loop {
            self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x2607));
            if self.r.ax == 0 {
                break;
            }
            self.r.bx = self.r.bx.wrapping_add(2);
            self.r.dx = self.r.dx.wrapping_add(0x20);
            self.r.cx = self.r.cx.wrapping_sub(1);
            if self.r.cx == 0 {
                return;
            }
        }
        // 16593
        self.r.ax = self.r.dx;
        self.ds.w16(self.r.bx.wrapping_add(0x2607), self.r.ax);
        self.bump_gen(Pool::PlayerBullet, (self.r.bx >> 1) as u8);
        self.pb_spawned = self.pb_spawned.wrapping_add(1);
        let tmpl = self.ds.r16(a(0x1F46F));
        let rec = self.r.dx;
        self.ds.copy16(rec, tmpl, 0x10);
        self.r.cx = 0;
        self.r.si = tmpl;
        self.r.di = rec;
        let ax = self.ds.r16(a(0x1F1C1));
        self.r.ax = ax;
        self.ds.w16(rec.wrapping_add(0x12), ax);
        self.ds.w16(rec.wrapping_add(4), ax);
        let ax = self.ds.r16(a(0x1F1C3));
        self.ds.w16(rec.wrapping_add(0x14), ax);
        self.r.bx = 0x0A;
        let prod = ax as u32 * 0x0A;
        self.r.ax = prod as u16;
        self.r.dx = (prod >> 16) as u16;
        self.ds.w16(rec.wrapping_add(6), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F1BC));
        self.ds.w16(rec.wrapping_add(0x0E), self.r.ax);
    }

    /// `sub_165CF`：朝右的武器派發。`word_1F1BE` 不是 `3Eh` 時改走朝左的 `sub_1669F`。
    /// 型別字母在 `word_1F1BC`：`K` 大彈、`S` 兩彈、`B` 兩顆追蹤彈、`L` 雷射、`F` 掃射六顆、其他普通彈。
    /// `165FCh` 至 `16619h` 的雙彈碼在 `retn` 之後無跳入，不移植。
    pub(crate) fn s165cf(&mut self) {
        self.r.ax = self.ds.r16(a(0x1F1BC));
        if self.ds.r16(a(0x1F1BE)) != 0x3E {
            self.s1669f();
            return;
        }
        match self.r.ax {
            0x4B => {
                // K
                self.r.ax = self.ds.r16(a(0x1F1C3));
                self.ds.w16(a(0x1F46F), 0x2935);
                self.s16578();
            }
            0x53 => {
                // S
                let v = self.ds.r16(a(0x1F1C3)).wrapping_add(1);
                self.ds.w16(a(0x1F1C3), v);
                self.ds.w16(a(0x1F46F), 0x2955);
                self.s16578();
                self.ds.w16(a(0x1F46F), 0x2975);
                self.s16578();
            }
            0x42 => {
                // B：word_1F1A8 > 5 時不射。
                let v = self.ds.r16(a(0x1F1C3)).wrapping_add(1);
                self.ds.w16(a(0x1F1C3), v);
                if self.ds.r16(a(0x1F1A8)) > 5 {
                    return;
                }
                self.pb_spawned = 0;
                self.ds.w16(a(0x1F46F), 0x2995);
                self.s16578();
                self.ds.w16(a(0x1F46F), 0x29B5);
                self.s16578();
                // 不看 sub_16578 是否配到槽，一律加 2（seeker_spawn_leak 的原版行為）。
                let n = if self.fixes.seeker_spawn_leak { self.pb_spawned } else { 2 };
                let v = self.ds.r16(a(0x1F1A8)).wrapping_add(n);
                self.ds.w16(a(0x1F1A8), v);
            }
            0x4C => {
                // L：欄 +3，列 +1。
                self.ds.w16(a(0x1F46F), 0x29D5);
                let v = self.ds.r16(a(0x1F1C3)).wrapping_add(1);
                self.ds.w16(a(0x1F1C3), v);
                let v = self.ds.r16(a(0x1F1C1)).wrapping_add(3);
                self.ds.w16(a(0x1F1C1), v);
                self.s16578();
            }
            0x46 => {
                // F：列 1、4、7、10、13、16 各一顆。
                self.r.cx = 6;
                self.ds.w16(a(0x1F1C3), 1);
                loop {
                    let cx = self.r.cx;
                    self.ds.w16(a(0x1F46F), 0x2B35);
                    self.s16578();
                    let v = self.ds.r16(a(0x1F1C3)).wrapping_add(3);
                    self.ds.w16(a(0x1F1C3), v);
                    self.r.cx = cx.wrapping_sub(1);
                    if self.r.cx == 0 {
                        break;
                    }
                }
            }
            _ => {
                // 普通彈：列 +1。
                let v = self.ds.r16(a(0x1F1C3)).wrapping_add(1);
                self.ds.w16(a(0x1F1C3), v);
                self.ds.w16(a(0x1F46F), 0x28D5);
                self.s16578();
            }
        }
    }

    /// `sub_1669F`：朝左的武器派發，型別字母由 `sub_165CF` 留在 `ax`。
    pub(crate) fn s1669f(&mut self) {
        match self.r.ax {
            0x4B => {
                // K
                self.r.ax = self.ds.r16(a(0x1F1C3));
                self.ds.w16(a(0x1F46F), 0x2A75);
                self.s16578();
            }
            0x4C => {
                // L：列 +1，欄 -2。
                self.ds.w16(a(0x1F46F), 0x29F5);
                let v = self.ds.r16(a(0x1F1C3)).wrapping_add(1);
                self.ds.w16(a(0x1F1C3), v);
                let v = self.ds.r16(a(0x1F1C1)).wrapping_sub(2);
                self.ds.w16(a(0x1F1C1), v);
                self.s16578();
            }
            0x53 => {
                // S：朝左不修正列。
                self.ds.w16(a(0x1F46F), 0x2A95);
                self.s16578();
                self.ds.w16(a(0x1F46F), 0x2AB5);
                self.s16578();
            }
            0x42 => {
                // B
                let v = self.ds.r16(a(0x1F1C3)).wrapping_add(1);
                self.ds.w16(a(0x1F1C3), v);
                if self.ds.r16(a(0x1F1A8)) > 5 {
                    return;
                }
                self.pb_spawned = 0;
                self.ds.w16(a(0x1F46F), 0x2AD5);
                self.s16578();
                self.ds.w16(a(0x1F46F), 0x2AF5);
                self.s16578();
                let n = if self.fixes.seeker_spawn_leak { self.pb_spawned } else { 2 };
                let v = self.ds.r16(a(0x1F1A8)).wrapping_add(n);
                self.ds.w16(a(0x1F1A8), v);
            }
            0x46 => {
                // F：朝左仍是同一份向右的樣板。
                self.r.cx = 6;
                self.ds.w16(a(0x1F1C3), 1);
                loop {
                    let cx = self.r.cx;
                    self.ds.w16(a(0x1F46F), 0x2B35);
                    self.s16578();
                    let v = self.ds.r16(a(0x1F1C3)).wrapping_add(3);
                    self.ds.w16(a(0x1F1C3), v);
                    self.r.cx = cx.wrapping_sub(1);
                    if self.r.cx == 0 {
                        break;
                    }
                }
            }
            _ => {
                // 普通彈：列 +1。
                let v = self.ds.r16(a(0x1F1C3)).wrapping_add(1);
                self.ds.w16(a(0x1F1C3), v);
                self.ds.w16(a(0x1F46F), 0x2A15);
                self.s16578();
            }
        }
    }

    /// `sub_1673E`：更新 20 顆玩家子彈。每槽複製進工作區、移動繪製碰撞、複製回去；
    /// `word_1F1C5` 為 `Y` 時釋放槽，型別 `A` 的追蹤彈同時讓 `word_1F1A8` 減 1。
    pub(crate) fn s1673e(&mut self) {
        self.r.bx = 0;
        self.r.cx = 0x14;
        loop {
            let cx = self.r.cx;
            self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x2607));
            if self.r.ax != 0 {
                let bx = self.r.bx;
                self.set_obj(Pool::PlayerBullet, (bx >> 1) as u8);
                self.s16779();
                self.s167a3();
                self.s1678e();
                self.r.bx = bx;
                if self.ds.r16(a(0x1F1C5)) == 0x59 {
                    self.r.ax = 0;
                    self.ds.w16(bx.wrapping_add(0x2607), 0);
                    if self.ds.r16(a(0x1F471)) == 0x41 {
                        let v = self.ds.r16(a(0x1F1A8)).wrapping_sub(1);
                        self.ds.w16(a(0x1F1A8), v);
                    }
                }
            }
            self.r.bx = self.r.bx.wrapping_add(2);
            self.r.cx = cx.wrapping_sub(1);
            if self.r.cx == 0 {
                break;
            }
        }
        self.clear_obj();
    }

    /// `sub_16779`：`ax` 指的記錄複製到工作區 `28B1h`，並把位址存進 `word_1F46F`。
    pub(crate) fn s16779(&mut self) {
        let ax = self.r.ax;
        self.ds.w16(a(0x1F46F), ax);
        self.ds.copy16(0x28B1, ax, 0x10);
        self.r.si = ax.wrapping_add(0x20);
        self.r.di = 0x28B1u16.wrapping_add(0x20);
        self.r.cx = 0;
    }

    /// `sub_1678E`：工作區複製回 `word_1F46F` 指的記錄。
    pub(crate) fn s1678e(&mut self) {
        let ax = self.ds.r16(a(0x1F46F));
        self.r.ax = ax;
        self.ds.copy16(ax, 0x28B1, 0x10);
        self.r.di = ax.wrapping_add(0x20);
        self.r.si = 0x28B1u16.wrapping_add(0x20);
        self.r.cx = 0;
    }

    /// `sub_167A3`：一顆子彈的一幀。雷射（`L`、`l`）走光束路徑，其餘取腳本位元組後
    /// 追蹤、移動、繪製。腳本位元組低 4 位元放 `al`，高 4 位元放 `ah`。
    pub(crate) fn s167a3(&mut self) {
        let t = self.ds.r16(a(0x1F471));
        if t == 0x4C {
            self.s16a4e();
            let al = self.ds.r8(a(0x1F1C0));
            self.r.set_al(al);
            if al != 0 {
                self.s16946();
            }
            return;
        }
        if t == 0x6C {
            self.s16aca();
            let al = self.ds.r8(a(0x1F1C0));
            self.r.set_al(al);
            if al != 0 {
                self.s16946();
            }
            return;
        }
        let ax = self.ds.r16(a(0x1F48B));
        self.ds.w16(a(0x1F48B), ax.wrapping_add(1));
        self.r.si = ax;
        let b = self.ds.r8(ax);
        self.r.ax = (b as u16) << 8 | (b & 0x0F) as u16;
        self.r.set_ah(b & 0xF0);
        self.s167e9();
        self.s16817();
        self.s1691a();
    }

    /// `sub_167E9`：腳本指令 `0Ah` 時播特效 `A`，用 `sub_16BA8` 找最近的格決定方向，
    /// 找不到（`ax` = FFh）就沿用 `word_1F47D`。方向放進 `al`、存回 `word_1F47D`。
    pub(crate) fn s167e9(&mut self) {
        if self.r.al() != 0x0A {
            return;
        }
        let (x, y) = (self.ds.r16(a(0x1F483)), self.ds.r16(a(0x1F485)));
        let o = self.cur_obj;
        self.spawn_effect(EffectKind::SeekerTrail, x, y);
        self.cur_obj = o;
        self.s16ba8();
        if self.r.ax == 0xFF {
            self.r.ax = self.ds.r16(a(0x1F47D));
            return;
        }
        self.r.set_al(self.r.dl());
        self.r.set_ah(0);
        self.ds.w16(a(0x1F47D), self.r.ax);
    }

    /// `sub_16817`：處理往回跳後依方向碼移動。`word_1F1C5` 先設 `N`，出界時被移動函式設 `Y`。
    /// 1、3、5、7 為上右下左，2、4、6、8 為上右、下右、下左、左上。
    pub(crate) fn s16817(&mut self) {
        self.s16873();
        self.ds.w16(a(0x1F1C5), 0x4E);
        match self.r.al() {
            1 => self.s1688e(),
            3 => self.s168f7(),
            5 => self.s168b1(),
            7 => self.s168d4(),
            2 => {
                self.s1688e();
                self.s168f7();
            }
            4 => {
                self.s168b1();
                self.s168f7();
            }
            6 => {
                self.s168b1();
                self.s168d4();
            }
            8 => {
                self.s168d4();
                self.s1688e();
            }
            _ => {}
        }
    }

    /// `sub_16873`：`ah` 為 `F0h` 時往回跳 `al` 個位元組，不遞增指標，以該處位元組當本幀指令。
    pub(crate) fn s16873(&mut self) {
        if self.r.ah() != 0xF0 {
            return;
        }
        self.r.set_ah(0);
        let p = self.ds.r16(a(0x1F48B)).wrapping_sub(self.r.ax);
        self.ds.w16(a(0x1F48B), p);
        self.r.ax = p;
        self.r.si = p;
        let b = self.ds.r8(p);
        self.r.ax = (b as u16) << 8 | (b & 0x0F) as u16;
        self.r.set_ah(b & 0xF0);
    }

    /// `sub_1688E`：往上。y_px 減 dy_px、列減 dy 步，列 >= 12h（無符號）設 `word_1F1C5` = `Y`。
    pub(crate) fn s1688e(&mut self) {
        let ax = self.ds.r16(a(0x1F477)).wrapping_sub(self.ds.r16(a(0x1F47B)));
        self.ds.w16(a(0x1F477), ax);
        let ax = self.ds.r16(a(0x1F485)).wrapping_sub(self.ds.r16(a(0x1F489)));
        self.r.ax = ax;
        self.ds.w16(a(0x1F485), ax);
        if self.ds.r16(a(0x1F485)) >= 0x12 {
            self.ds.w16(a(0x1F1C5), 0x59);
        }
    }

    /// `sub_168B1`：往下。
    pub(crate) fn s168b1(&mut self) {
        let ax = self.ds.r16(a(0x1F477)).wrapping_add(self.ds.r16(a(0x1F47B)));
        self.ds.w16(a(0x1F477), ax);
        let ax = self.ds.r16(a(0x1F485)).wrapping_add(self.ds.r16(a(0x1F489)));
        self.r.ax = ax;
        self.ds.w16(a(0x1F485), ax);
        if self.ds.r16(a(0x1F485)) >= 0x12 {
            self.ds.w16(a(0x1F1C5), 0x59);
        }
    }

    /// `sub_168D4`：往左。x_px 減 dx_px、欄減 dx 步，欄 >= 27h（無符號）設 `Y`。
    pub(crate) fn s168d4(&mut self) {
        let ax = self.ds.r16(a(0x1F475)).wrapping_sub(self.ds.r16(a(0x1F479)));
        self.ds.w16(a(0x1F475), ax);
        let ax = self.ds.r16(a(0x1F483)).wrapping_sub(self.ds.r16(a(0x1F487)));
        self.r.ax = ax;
        self.ds.w16(a(0x1F483), ax);
        if self.ds.r16(a(0x1F483)) >= 0x27 {
            self.ds.w16(a(0x1F1C5), 0x59);
        }
    }

    /// `sub_168F7`：往右。
    pub(crate) fn s168f7(&mut self) {
        let ax = self.ds.r16(a(0x1F475)).wrapping_add(self.ds.r16(a(0x1F479)));
        self.ds.w16(a(0x1F475), ax);
        let ax = self.ds.r16(a(0x1F483)).wrapping_add(self.ds.r16(a(0x1F487)));
        self.r.ax = ax;
        self.ds.w16(a(0x1F483), ax);
        if self.ds.r16(a(0x1F483)) >= 0x27 {
            self.ds.w16(a(0x1F1C5), 0x59);
        }
    }

    /// `sub_1691A`：畫子彈（遮罩加平面），標 G3 髒格，對 G1 做碰撞，第三關再對 G4 做碰撞，
    /// 有命中時結算傷害。
    pub(crate) fn s1691a(&mut self) {
        let ax = self.ds.r16(a(0x1F475));
        self.ds.w16(a(0x26880), ax);
        let ax = self.ds.r16(a(0x1F477));
        self.ds.w16(a(0x26882), ax);
        let ax = self.ds.r16(a(0x1F473));
        self.r.ax = ax;
        self.ds.w16(a(0x26888), ax);
        self.s18e73();
        self.s18eb6();
        self.s1698c();
        self.s16b49();
        self.s169bf();
        if self.ds.r8(a(0x1F1C0)) != 0 {
            self.s16946();
        }
    }

    /// `sub_16946`：命中結算。`byte_1F1C0` 是命中格值（FFh 為地形或無目標，不寫傷害），
    /// 槽號 = 格值 - 1，實體記錄 `+20h` 的字組：高位元組設 1，低位元組加 1（型別 `S`、`A` 加 3）。
    /// 裝甲格 FEh 取到槽 FDh，讀 `ds:248Eh`，寫到 `dseg:0020h`，這是原版行為。
    /// 最後在子彈目前位置播命中特效。
    pub(crate) fn s16946(&mut self) {
        let mut al = self.ds.r8(a(0x1F1C0));
        self.r.set_al(al);
        if al != 0xFF {
            al = al.wrapping_sub(1);
            self.r.set_al(al);
            self.r.set_ah(0);
            self.r.bx = (al as u16) << 1;
            let p = self.ds.r16(self.r.bx.wrapping_add(0x2294));
            // armor_cell_index：裝甲格 FEh 與已釋放的槽（指標 0）不寫傷害，原版會寫到 dseg:0020h。
            if !(self.fixes.armor_cell_index && (al == 0xFD || p == 0)) {
                let ax = p.wrapping_add(0x20);
                self.r.ax = ax;
                self.r.si = ax;
                self.r.ax = self.ds.r16(ax);
                self.r.set_ah(1);
                let t = self.ds.r16(a(0x1F471));
                let mut al = self.r.al();
                if t == 0x53 || t == 0x41 {
                    al = al.wrapping_add(2);
                }
                al = al.wrapping_add(1);
                self.r.set_al(al);
                self.ds.w16(self.r.si, self.r.ax);
            }
        }
        // 16976
        let (x, y) = (self.ds.r16(a(0x1F483)), self.ds.r16(a(0x1F485)));
        let letter = self.ds.r16(a(0x1F48F));
        let o = self.cur_obj;
        self.spawn_effect(EffectKind::from_letter(letter), x, y);
        self.cur_obj = o;
    }

    /// `sub_1698C`：把子彈形狀（`word_1F48D` 指的寬、高、格值）用 OR 寫進 G3 髒格 `964Ch` 起，
    /// 範圍 `964Ch` 至 `9C3Ch`。
    pub(crate) fn s1698c(&mut self) {
        let mut si = self.ds.r16(a(0x1F48D));
        let w = self.ds.r16(si);
        si = si.wrapping_add(2);
        self.ds.w16(a(0x2687E), w);
        let mut cx = self.ds.r16(si);
        si = si.wrapping_add(2);
        let mut bx = self.ds.r16(a(0x1F485));
        let mut di;
        loop {
            let outer = cx;
            self.r.bx = bx;
            self.s16a0a();
            di = self.r.di;
            let mut icx = self.r.cx;
            loop {
                if di <= 0x9C3C && di >= 0x964C {
                    let al = self.ds.r8(si);
                    self.r.set_al(al);
                    let v = self.ds.r8(di) | al;
                    self.ds.w8(di, v);
                }
                di = di.wrapping_add(2);
                si = si.wrapping_add(1);
                icx = icx.wrapping_sub(1);
                if icx == 0 {
                    break;
                }
            }
            bx = bx.wrapping_add(1);
            cx = outer.wrapping_sub(1);
            if cx == 0 {
                break;
            }
        }
        self.r.si = si;
        self.r.di = di;
        self.r.bx = bx;
        self.r.cx = 0;
    }

    /// `sub_169BF`：第三關（`byte_1CFDB` = `C`）才有。子彈形狀掃 G4 地形格 `900Ch` 起，
    /// 字組非 0 就設 `word_1F1C5` = `Y`、`byte_1F1C0` = FFh。列從子彈列減 1 起。
    pub(crate) fn s169bf(&mut self) {
        if self.ds.r8(a(0x1CFDB)) != 0x43 {
            return;
        }
        let mut si = self.ds.r16(a(0x1F48D));
        let w = self.ds.r16(si);
        si = si.wrapping_add(2);
        self.ds.w16(a(0x2687E), w);
        let mut cx = self.ds.r16(si);
        si = si.wrapping_add(2);
        let mut bx = self.ds.r16(a(0x1F485)).wrapping_sub(1);
        let mut di;
        loop {
            let outer = cx;
            self.r.bx = bx;
            self.s16a2c();
            di = self.r.di;
            let mut icx = self.r.cx;
            loop {
                if di <= 0x95FC && di >= 0x900C {
                    let ax = self.ds.r16(di);
                    self.r.ax = ax;
                    if ax != 0 {
                        self.ds.w16(a(0x1F1C5), 0x59);
                        self.ds.w8(a(0x1F1C0), 0xFF);
                    }
                }
                di = di.wrapping_add(2);
                si = si.wrapping_add(1);
                icx = icx.wrapping_sub(1);
                if icx == 0 {
                    break;
                }
            }
            bx = bx.wrapping_add(1);
            cx = outer.wrapping_sub(1);
            if cx == 0 {
                break;
            }
        }
        self.r.si = si;
        self.r.di = di;
        self.r.bx = bx;
        self.r.cx = 0;
    }

    /// `sub_16A0A`：G3 髒格的起點。`bx` = 列（>= 14h 改取 14h），`di` = `964Ch` + 列基底 + 欄 × 2 + 1，
    /// `cx` = 形狀寬。`bx` 保留。
    pub(crate) fn s16a0a(&mut self) {
        let mut bx = self.r.bx;
        if bx >= 0x14 {
            bx = 0x14;
        }
        let base = self.ds.r16((bx << 1).wrapping_add(0x21C2)).wrapping_add(0x964C);
        let c = (self.ds.r16(a(0x1F483)) << 1).wrapping_add(1);
        self.r.ax = c;
        self.r.di = base.wrapping_add(c);
        self.r.cx = self.ds.r16(a(0x2687E));
    }

    /// `sub_16A2C`：G4 地形格的起點，同 `sub_16A0A` 但基底是 `900Ch`。
    pub(crate) fn s16a2c(&mut self) {
        let mut bx = self.r.bx;
        if bx >= 0x14 {
            bx = 0x14;
        }
        let base = self.ds.r16((bx << 1).wrapping_add(0x21C2)).wrapping_add(0x900C);
        let c = (self.ds.r16(a(0x1F483)) << 1).wrapping_add(1);
        self.r.ax = c;
        self.r.di = base.wrapping_add(c);
        self.r.cx = self.ds.r16(a(0x2687E));
    }

    /// `sub_16A4E`：朝右的雷射。從子彈欄往右逐格掃，G4 同列上一列非 0 就停，G1 格值非 0、FFh 就記錄
    /// 為命中並停，掃到欄 39 也停。`bx` 是光束格數，交給 `sub_16A99` 畫。子彈當幀移除。
    pub(crate) fn s16a4e(&mut self) {
        self.ds.w8(a(0x1F1C0), 0xFF);
        self.r.bx = self.ds.r16(a(0x1F485));
        self.s16cc6();
        let mut di = self.r.di;
        let dx = self.r.dx;
        self.r.bx = self.r.bx.wrapping_sub(1);
        self.s16a2c();
        let mut si = self.r.di;
        let mut bx = 1u16;
        loop {
            di = di.wrapping_add(1);
            si = si.wrapping_add(2);
            bx = bx.wrapping_add(1);
            if di >= dx {
                break;
            }
            let ax = self.ds.r16(si);
            self.r.ax = ax;
            if ax != 0 {
                break;
            }
            let al = self.ds.r8(di);
            self.r.set_al(al);
            if al == 0 || al == 0xFF {
                continue;
            }
            self.ds.w8(a(0x1F1C0), al);
            self.r.bx = bx;
            self.r.di = di;
            self.r.si = si;
            self.s16c7d();
            break;
        }
        // 16A8F
        self.ds.w16(a(0x1F1C5), 0x59);
        self.r.bx = bx;
        self.r.si = si;
        self.r.di = di;
        self.s16a99();
    }

    /// `sub_16A99`：往右畫 `bx` 格光束，圖 4（頁 0）或 19h（頁 8000h），每格欄加 1，
    /// 同時推進 `word_1F483`（`sub_16946` 的特效位置在光束末端）。
    pub(crate) fn s16a99(&mut self) {
        let mut cx = self.r.bx;
        self.r.cx = cx;
        let ax = self.ds.r16(a(0x1F477));
        self.ds.w16(a(0x26882), ax);
        let ax = self.ds.r16(a(0x1F475));
        self.ds.w16(a(0x26880), ax);
        self.r.ax = 4;
        self.ds.w16(a(0x26888), 4);
        if self.ds.r16(a(0x26872)) != 0 {
            self.r.ax = 0x19;
            self.ds.w16(a(0x26888), 0x19);
        }
        loop {
            self.r.cx = cx;
            self.s18c0a();
            let v = self.ds.r16(a(0x26880)).wrapping_add(1);
            self.ds.w16(a(0x26880), v);
            let v = self.ds.r16(a(0x1F483)).wrapping_add(1);
            self.ds.w16(a(0x1F483), v);
            cx = cx.wrapping_sub(1);
            if cx == 0 {
                break;
            }
        }
        self.r.cx = 0;
    }

    /// `sub_16ACA`：朝左的雷射，掃描方向相反，掃到列基底（`dx` - 27h）也停。
    pub(crate) fn s16aca(&mut self) {
        self.ds.w8(a(0x1F1C0), 0xFF);
        self.r.bx = self.ds.r16(a(0x1F485));
        self.s16cc6();
        let dx = self.r.dx.wrapping_sub(0x27);
        self.r.dx = dx;
        let mut di = self.r.di;
        self.r.bx = self.r.bx.wrapping_sub(1);
        self.s16a2c();
        let mut si = self.r.di;
        let mut bx = 1u16;
        loop {
            di = di.wrapping_sub(1);
            si = si.wrapping_sub(2);
            bx = bx.wrapping_add(1);
            if di <= dx {
                break;
            }
            let ax = self.ds.r16(si);
            self.r.ax = ax;
            if ax != 0 {
                break;
            }
            let al = self.ds.r8(di);
            self.r.set_al(al);
            if al == 0 || al == 0xFF {
                continue;
            }
            self.ds.w8(a(0x1F1C0), al);
            self.r.bx = bx;
            self.r.di = di;
            self.r.si = si;
            self.s16c7d();
            break;
        }
        // 16B0E
        self.ds.w16(a(0x1F1C5), 0x59);
        self.r.bx = bx;
        self.r.si = si;
        self.r.di = di;
        self.s16b18();
    }

    /// `sub_16B18`：往左畫 `bx` 格光束，每格欄減 1。
    pub(crate) fn s16b18(&mut self) {
        let mut cx = self.r.bx;
        self.r.cx = cx;
        let ax = self.ds.r16(a(0x1F477));
        self.ds.w16(a(0x26882), ax);
        let ax = self.ds.r16(a(0x1F475));
        self.ds.w16(a(0x26880), ax);
        self.r.ax = 4;
        self.ds.w16(a(0x26888), 4);
        if self.ds.r16(a(0x26872)) != 0 {
            self.r.ax = 0x19;
            self.ds.w16(a(0x26888), 0x19);
        }
        loop {
            self.r.cx = cx;
            self.s18c0a();
            let v = self.ds.r16(a(0x26880)).wrapping_sub(1);
            self.ds.w16(a(0x26880), v);
            let v = self.ds.r16(a(0x1F483)).wrapping_sub(1);
            self.ds.w16(a(0x1F483), v);
            cx = cx.wrapping_sub(1);
            if cx == 0 {
                break;
            }
        }
        self.r.cx = 0;
    }

    /// `sub_16B49`：子彈形狀逐格對 G1 做碰撞。可測範圍 `1B32h` < `di` < `1E2Ah` 且在列內，
    /// 格值 0、FFh 略過，FEh（裝甲）不記錄目標，其他記錄到 `byte_1F1C0`（後掃到的覆蓋前面）。
    /// 格值非 0、非 FFh 的格（含 FEh）都會讓子彈消失（`word_1F1C5` = `Y`），型別 `S` 除外，
    /// 並呼叫 `sub_16C7D` 計數。
    pub(crate) fn s16b49(&mut self) {
        self.ds.w8(a(0x1F1C0), 0);
        let mut si = self.ds.r16(a(0x1F48D));
        let w = self.ds.r16(si);
        si = si.wrapping_add(2);
        self.ds.w16(a(0x2687E), w);
        let mut cx = self.ds.r16(si);
        si = si.wrapping_add(2);
        let mut bx = self.ds.r16(a(0x1F485));
        let mut di;
        let mut dx;
        loop {
            let outer = cx;
            self.r.bx = bx;
            self.s16cc6();
            let ax = self.r.ax;
            dx = self.r.dx;
            di = self.r.di;
            let mut icx = self.r.cx;
            loop {
                if !(di >= 0x1E2A || di <= 0x1B32 || di >= dx || di <= ax) {
                    let al = self.ds.r8(di);
                    self.r.set_al(al);
                    if al != 0 && al != 0xFF {
                        if al != 0xFE {
                            self.ds.w8(a(0x1F1C0), al);
                        }
                        if self.ds.r16(a(0x1F471)) != 0x53 {
                            self.ds.w16(a(0x1F1C5), 0x59);
                        }
                        self.r.bx = bx;
                        self.r.cx = icx;
                        self.r.di = di;
                        self.s16c7d();
                    }
                }
                di = di.wrapping_add(1);
                icx = icx.wrapping_sub(1);
                if icx == 0 {
                    break;
                }
            }
            bx = bx.wrapping_add(1);
            cx = outer.wrapping_sub(1);
            if cx == 0 {
                break;
            }
        }
        self.r.si = si;
        self.r.di = di;
        self.r.dx = dx;
        self.r.bx = bx;
        self.r.cx = 0;
    }

    /// `sub_16BA8`：追蹤彈找方向。沿 G1 的上、右、下、左四向，從子彈所在格起數到第一個
    /// 非 0 非 FFh 的格距（找不到為 FFh），結果在 `word_1F1AA`（上）、`1F1AE`（右）、`1F1AC`（下）、
    /// `1F1B0`（左）。回傳 `ax` = 最小格距，`dx` = 方向 1、3、5、7。平手時後者優先：左 > 下 > 右 > 上。
    pub(crate) fn s16ba8(&mut self) {
        self.r.bx = self.ds.r16(a(0x1F485));
        self.s16cc6();
        self.ds.w16(a(0x1F1B2), self.r.di);
        self.ds.w16(a(0x1F1B6), self.r.ax);
        self.ds.w16(a(0x1F1B4), self.r.dx);
        self.s16bf5();
        self.s16c18();
        self.s16c3b();
        self.s16c5c();
        self.r.dx = 1;
        let mut ax = self.ds.r16(a(0x1F1AA));
        if ax >= self.ds.r16(a(0x1F1AE)) {
            ax = self.ds.r16(a(0x1F1AE));
            self.r.dx = 3;
        }
        if ax >= self.ds.r16(a(0x1F1AC)) {
            ax = self.ds.r16(a(0x1F1AC));
            self.r.dx = 5;
        }
        if ax >= self.ds.r16(a(0x1F1B0)) {
            self.r.dx = 7;
        }
        self.r.ax = ax;
    }

    /// `sub_16BF5`：往上數格距，每格減 28h，到 `1B32h` 以下仍無則 FFh。結果存 `word_1F1AA`。
    pub(crate) fn s16bf5(&mut self) {
        let mut bx = 0u16;
        let mut si = self.ds.r16(a(0x1F1B2));
        loop {
            let al = self.ds.r8(si);
            self.r.set_al(al);
            if al != 0xFF && al != 0 {
                break;
            }
            si = si.wrapping_sub(0x28);
            bx = bx.wrapping_add(1);
            if si > 0x1B32 {
                continue;
            }
            bx = 0xFF;
            break;
        }
        self.r.bx = bx;
        self.r.si = si;
        self.ds.w16(a(0x1F1AA), bx);
    }

    /// `sub_16C18`：往下數格距，每格加 28h，到 `1E2Ah` 以上仍無則 FFh。結果存 `word_1F1AC`。
    pub(crate) fn s16c18(&mut self) {
        let mut bx = 0u16;
        let mut si = self.ds.r16(a(0x1F1B2));
        loop {
            let al = self.ds.r8(si);
            self.r.set_al(al);
            if al != 0xFF && al != 0 {
                break;
            }
            si = si.wrapping_add(0x28);
            bx = bx.wrapping_add(1);
            if si < 0x1E2A {
                continue;
            }
            bx = 0xFF;
            break;
        }
        self.r.bx = bx;
        self.r.si = si;
        self.ds.w16(a(0x1F1AC), bx);
    }

    /// `sub_16C3B`：往右數格距，到列尾（`word_1F1B4`）仍無則 FFh。結果存 `word_1F1AE`。
    pub(crate) fn s16c3b(&mut self) {
        let mut bx = 0u16;
        let mut si = self.ds.r16(a(0x1F1B2));
        loop {
            let al = self.ds.r8(si);
            self.r.set_al(al);
            if al != 0xFF && al != 0 {
                break;
            }
            si = si.wrapping_add(1);
            bx = bx.wrapping_add(1);
            if si < self.ds.r16(a(0x1F1B4)) {
                continue;
            }
            bx = 0xFF;
            break;
        }
        self.r.bx = bx;
        self.r.si = si;
        self.ds.w16(a(0x1F1AE), bx);
    }

    /// `sub_16C5C`：往左數格距，到列首（`word_1F1B6`）仍無則 FFh。結果存 `word_1F1B0`。
    pub(crate) fn s16c5c(&mut self) {
        let mut bx = 0u16;
        let mut si = self.ds.r16(a(0x1F1B2));
        loop {
            let al = self.ds.r8(si);
            self.r.set_al(al);
            if al != 0xFF && al != 0 {
                break;
            }
            si = si.wrapping_sub(1);
            bx = bx.wrapping_add(1);
            if si > self.ds.r16(a(0x1F1B6)) {
                continue;
            }
            bx = 0xFF;
            break;
        }
        self.r.bx = bx;
        self.r.si = si;
        self.ds.w16(a(0x1F1B0), bx);
    }

    /// `sub_16C7D`：擊中計數。子彈型別 `word_1F47F` 為 `1`（31h）時 `word_1F1B8` 加 1、`2`（32h）時
    /// `word_1F1BA` 加 1，並在介面畫出數字。保留 `bx`、`cx`、`di`。數字屬於介面，不帶子彈的物件編號。
    pub(crate) fn s16c7d(&mut self) {
        let (bx, cx, di) = (self.r.bx, self.r.cx, self.r.di);
        let owner = self.ds.r16(a(0x1F47F));
        if owner == 0x31 {
            let v = self.ds.r16(a(0x1F1B8)).wrapping_add(1);
            self.ds.w16(a(0x1F1B8), v);
            self.ds.w16(a(0x2688A), 0x23);
            self.ds.w16(a(0x2688E), 0xBE);
            self.r.cx = self.ds.r16(a(0x1F1B8));
            let o = self.cur_obj;
            self.clear_obj();
            self.s1890d();
            self.cur_obj = o;
        } else if owner == 0x32 {
            let v = self.ds.r16(a(0x1F1BA)).wrapping_add(1);
            self.ds.w16(a(0x1F1BA), v);
            self.ds.w16(a(0x2688A), 0);
            self.ds.w16(a(0x2688E), 0xBE);
            self.r.cx = self.ds.r16(a(0x1F1BA));
            let o = self.cur_obj;
            self.clear_obj();
            self.s1890d();
            self.cur_obj = o;
        }
        self.r.bx = bx;
        self.r.cx = cx;
        self.r.di = di;
    }

    /// `sub_16CC6`：G1 的起點。`bx` = 列（>= 17Ch 改取 1F4h，等於讀 `1EF2h` 的 0），
    /// 回傳 `ax` = 列基底（`1B32h` + 列偏移），`di` = 基底 + 欄 `word_1F483`，`dx` = 基底 + 27h，
    /// `cx` = 形狀寬 `word_2687E`。`bx` 保留。
    pub(crate) fn s16cc6(&mut self) {
        let mut bx = self.r.bx;
        if bx >= 0x17C {
            bx = 0x1F4;
        }
        let ax = self.ds.r16((bx << 1).wrapping_add(0x1B0A)).wrapping_add(0x1B32);
        self.r.ax = ax;
        self.r.di = ax.wrapping_add(self.ds.r16(a(0x1F483)));
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.dx = ax.wrapping_add(0x27);
    }
}
