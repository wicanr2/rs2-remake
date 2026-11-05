//! 實體更新、移動腳本、生成時間表與景物。
//! 逐函式移植自機器碼，查法見 `tools/q.sh`。規格：`docs/spec/001-rust-first-stage.md`。
//!
//! 位址慣例：`a(0x1F188)` 是 IDA 線性位址名稱 `word_1F188`，裸的 `0x2294` 是 `dseg` 偏移。
//! 實體記錄的工作區在 `1F176h`（w0）起 24 字組，欄位對照見 `docs/re/11-entities-combat.md` 第 2.1 節。

use crate::draw::Pool;
use crate::dseg::a;
use crate::ebullet::turn_toward;
use crate::fx::EffectKind;
use crate::game::Core;

impl Core {
    /// `rep movsw`（方向旗標清除）：照 `cx`、`si`、`di` 搬字組，並更新三者。
    fn rep_movsw(&mut self) {
        let n = self.r.cx;
        self.ds.copy16(self.r.di, self.r.si, n);
        let step = n.wrapping_mul(2);
        self.r.si = self.r.si.wrapping_add(step);
        self.r.di = self.r.di.wrapping_add(step);
        self.r.cx = 0;
    }

    /// `sub_15C3C`：依幀計數 `word_1CFE2` 掃景物群組表 `dseg:19DCh`，幀相符的列把記錄指標交給 `sub_15C5F`。
    pub(crate) fn s15c3c(&mut self) {
        let mut si = 0x19DCu16;
        loop {
            // lodsw 兩次：bx = 幀，dx = 記錄指標，ax 停在第二個字組。
            let frame = self.ds.r16(si);
            let rec = self.ds.r16(si.wrapping_add(2));
            si = si.wrapping_add(4);
            self.r.bx = frame;
            self.r.dx = rec;
            self.r.ax = rec;
            self.r.si = si;
            if frame == 0 {
                return;
            }
            if frame == self.ds.r16(a(0x1CFE2)) {
                self.s15c5f();
                self.r.si = si;
            }
        }
    }

    /// `sub_15C5F`：把 `dx` 指向的 4 字組 `(狀態, x, y, dx)` 存進第一個狀態為 0 的景物槽，滿了就放棄。
    pub(crate) fn s15c5f(&mut self) {
        self.r.cx = 0x0A;
        self.r.bx = 0;
        loop {
            self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x19C8));
            if self.r.ax == 0 {
                break;
            }
            self.r.bx = self.r.bx.wrapping_add(2);
            self.r.cx = self.r.cx.wrapping_sub(1);
            if self.r.cx == 0 {
                return;
            }
        }
        self.r.si = self.r.dx;
        let si = self.r.si;
        let bx = self.r.bx;
        self.r.ax = self.ds.r16(si);
        self.ds.w16(bx.wrapping_add(0x19C8), self.r.ax);
        self.r.ax = self.ds.r16(si.wrapping_add(2));
        self.ds.w16(bx.wrapping_add(0x19A0), self.r.ax);
        self.r.ax = self.ds.r16(si.wrapping_add(4));
        self.ds.w16(bx.wrapping_add(0x19B4), self.r.ax);
        self.r.ax = self.ds.r16(si.wrapping_add(6));
        self.ds.w16(bx.wrapping_add(0x198C), self.r.ax);
        self.bump_gen(Pool::Scenery, (bx >> 1) as u8);
    }

    /// `sub_15C92`：景物繪製，狀態 10h、0Fh、0Eh 依序。
    pub(crate) fn s15c92(&mut self) {
        self.s15c9c();
        self.s15cb8();
        self.s15cd4();
    }

    /// 景物 10 槽的共用迴圈：狀態等於 `state` 的槽交給 `sub_15CF0`。
    /// `sub_15C9C`、`sub_15CB8`、`sub_15CD4` 的機器碼只差比較的立即值。
    fn scenery_layer(&mut self, state: u16) {
        self.r.cx = 0x0A;
        self.r.bx = 0;
        loop {
            let (cx, bx) = (self.r.cx, self.r.bx);
            self.r.ax = self.ds.r16(bx.wrapping_add(0x19C8));
            if self.r.ax == state {
                self.set_obj(Pool::Scenery, (bx >> 1) as u8);
                self.s15cf0();
                self.clear_obj();
            }
            self.r.bx = bx.wrapping_add(2);
            self.r.cx = cx.wrapping_sub(1);
            if self.r.cx == 0 {
                break;
            }
        }
    }

    /// `sub_15C9C`：畫狀態 10h 的景物（最遠，最先畫）。
    pub(crate) fn s15c9c(&mut self) {
        self.scenery_layer(0x10);
    }

    /// `sub_15CB8`：畫狀態 0Fh 的景物。
    pub(crate) fn s15cb8(&mut self) {
        self.scenery_layer(0x0F);
    }

    /// `sub_15CD4`：畫狀態 0Eh 的景物（最近，最後畫）。
    pub(crate) fn s15cd4(&mut self) {
        self.scenery_layer(0x0E);
    }

    /// `sub_15CF0`：畫第 `bx` 槽景物（`ax` 為圖庫索引）於舊 x，之後 x 減 dx；新 x 小於等於 -7 就清除狀態並把 x 寫成 0。
    pub(crate) fn s15cf0(&mut self) {
        self.ds.w16(a(0x26888), self.r.ax);
        let bx = self.r.bx;
        self.r.ax = self.ds.r16(bx.wrapping_add(0x19A0));
        self.ds.w16(a(0x26880), self.r.ax);
        self.r.ax = self.r.ax.wrapping_sub(self.ds.r16(bx.wrapping_add(0x198C)));
        // cmp ax, 0FFF9h / jg：有號比較，新 x > -7 才保留。
        if (self.r.ax as i16) <= -7 {
            self.r.ax = 0;
            self.ds.w16(bx.wrapping_add(0x19C8), self.r.ax);
        }
        self.ds.w16(bx.wrapping_add(0x19A0), self.r.ax);
        self.r.ax = self.ds.r16(bx.wrapping_add(0x19B4));
        self.ds.w16(a(0x26882), self.r.ax);
        self.s18f1a();
        self.s18f5d();
    }

    /// `sub_15D1C`：掃時間表（`si` 起，每列 `(幀, 樣板指標)`，幀為 0 結束），幀等於 `word_1CFE2` 的列生成一個實體。
    pub(crate) fn s15d1c(&mut self) {
        loop {
            self.r.ax = self.ds.r16(self.r.si);
            if self.r.ax == 0 {
                return;
            }
            if self.r.ax == self.ds.r16(a(0x1CFE2)) {
                self.r.si = self.r.si.wrapping_add(2);
                self.r.ax = self.ds.r16(self.r.si);
                self.ds.w16(a(0x1F174), self.r.ax);
                let si = self.r.si;
                self.s15d41();
                self.r.si = si.wrapping_add(2);
            } else {
                self.r.si = self.r.si.wrapping_add(4);
            }
        }
    }

    /// `sub_15D41`：把 `word_1F174` 指向的樣板複製進第一個空實體槽（記錄 `22B4h` 起，每槽 30h），
    /// 再設 w2 = 樣板 w9、w3 = 樣板 w10 乘 10。沒有空槽就放棄。
    pub(crate) fn s15d41(&mut self) {
        self.r.bx = 0;
        self.r.dx = 0x22B4;
        self.r.cx = 0x0F;
        loop {
            self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x2294));
            if self.r.ax == 0 {
                break;
            }
            self.r.bx = self.r.bx.wrapping_add(2);
            self.r.dx = self.r.dx.wrapping_add(0x30);
            self.r.cx = self.r.cx.wrapping_sub(1);
            if self.r.cx == 0 {
                return;
            }
        }
        self.r.ax = self.r.dx;
        self.ds.w16(self.r.bx.wrapping_add(0x2294), self.r.ax);
        let slot = (self.r.bx >> 1) as u8;
        self.r.si = self.ds.r16(a(0x1F174));
        self.r.di = self.r.dx;
        self.r.cx = 0x18;
        self.rep_movsw();
        self.r.si = self.ds.r16(a(0x1F174));
        self.r.di = self.r.dx;
        self.r.ax = self.ds.r16(self.r.si.wrapping_add(0x12));
        self.ds.w16(self.r.di.wrapping_add(4), self.r.ax);
        self.r.ax = self.ds.r16(self.r.si.wrapping_add(0x14));
        self.r.bx = 0x0A;
        // mul bx：dx:ax = ax * bx，無號。
        let p = self.r.ax as u32 * self.r.bx as u32;
        self.r.ax = p as u16;
        self.r.dx = (p >> 16) as u16;
        self.ds.w16(self.r.di.wrapping_add(6), self.r.ax);
        self.bump_gen(Pool::Entity, slot);
    }

    /// `sub_15D8C`：更新 15 個實體槽。每個存活的槽：複製進工作區、跑一幀、複製回、存渲染值、處理死亡。
    /// `byte_1E6C9` 是目前槽號，`sub_16233` 把它加進 G1 的格值。
    pub(crate) fn s15d8c(&mut self) {
        self.r.bx = 0;
        self.r.cx = 0x0F;
        self.ds.w8(a(0x1E6C9), 0);
        loop {
            let cx = self.r.cx;
            self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x2294));
            if self.r.ax != 0 {
                let bx = self.r.bx;
                self.s15e1a();
                self.s15e68();
                self.s15e2f();
                self.r.bx = bx;
                self.s15e44();
                self.s15dbe();
            }
            self.r.bx = self.r.bx.wrapping_add(2);
            self.r.cx = cx;
            let n = self.ds.r8(a(0x1E6C9)).wrapping_add(1);
            self.ds.w8(a(0x1E6C9), n);
            self.r.cx = self.r.cx.wrapping_sub(1);
            if self.r.cx == 0 {
                break;
            }
        }
    }

    /// `sub_15DBE`：本幀標記死亡（`word_1EDAA` = `Y`）的實體：核心設過關旗標、清槽指標、
    /// kind `K` 把損壞貼片畫進暫存區 B、carrier `P` 掉出 w7 字母的道具。`bx` 是槽的位元組偏移。
    pub(crate) fn s15dbe(&mut self) {
        if self.ds.r16(a(0x1EDAA)) != 0x59 {
            return;
        }
        if self.ds.r16(a(0x1F184)) == 0x45 {
            self.ds.w8(a(0x1CFAD), 0x46);
        }
        self.r.ax = 0;
        self.ds.w16(self.r.bx.wrapping_add(0x2294), self.r.ax);
        if self.ds.r16(a(0x1F19A)) == 0x4B {
            self.l_15dfe();
            return;
        }
        if self.ds.r16(a(0x1F19C)) != 0x50 {
            return;
        }
        let bx = self.r.bx;
        let (x, y) = (self.ds.r16(a(0x1F188)), self.ds.r16(a(0x1F18A)).wrapping_add(2));
        self.fire_enemy_bullets(self.ds.r16(a(0x1F184)), x, y);
        self.r.bx = bx;
    }

    /// `15DFEh`（`sub_15DBE` 的中段）：以 w1 指向的三字組 `(圖, x, y)` 把圖無遮罩畫進暫存區 B（`word_2685C` = 8000h）。
    /// 這是累積在背景上的損壞貼片，不屬於任何補間物件。
    fn l_15dfe(&mut self) {
        let bx = self.r.bx;
        self.r.si = self.ds.r16(a(0x1F178));
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        self.ds.w16(a(0x26888), self.r.ax);
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        self.ds.w16(a(0x26880), self.r.ax);
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        self.ds.w16(a(0x26882), self.r.ax);
        self.ds.w16(a(0x2685C), 0x8000);
        self.clear_obj();
        self.s19adc();
        self.r.bx = bx;
    }

    /// `sub_15E1A`：把 `ax` 指向的實體記錄（24 字組）複製進工作區 `25B6h`，`word_1F174` 記下來源。
    pub(crate) fn s15e1a(&mut self) {
        self.ds.w16(a(0x1F174), self.r.ax);
        self.r.si = self.r.ax;
        self.r.di = 0x25B6;
        self.r.cx = 0x18;
        self.rep_movsw();
    }

    /// `sub_15E2F`：把工作區複製回 `word_1F174` 指向的實體記錄。
    pub(crate) fn s15e2f(&mut self) {
        self.r.ax = self.ds.r16(a(0x1F174));
        self.r.di = self.r.ax;
        self.r.si = 0x25B6;
        self.r.cx = 0x18;
        self.rep_movsw();
    }

    /// `sub_15E44`：把本幀的圖框、x、y 像素、kind、閃爍旗標存進第 `bx` 槽的渲染陣列。
    pub(crate) fn s15e44(&mut self) {
        let bx = self.r.bx;
        self.r.ax = self.ds.r16(a(0x1EE4C));
        self.ds.w16(bx.wrapping_add(0x21EC), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1EE4E));
        self.ds.w16(bx.wrapping_add(0x220C), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1EE50));
        self.ds.w16(bx.wrapping_add(0x222C), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1EE52));
        self.ds.w16(bx.wrapping_add(0x224C), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1E6B6));
        self.ds.w16(bx.wrapping_add(0x226C), self.r.ax);
    }

    /// `sub_15E68`：工作區內一個實體的一幀：命中結算、取腳本碼或追蹤朝向、移動、圖框、渲染值與 G3、G1、隨機開火。
    /// 本幀死亡（`word_1EDAA` = `Y`）時結算後直接結束。
    pub(crate) fn s15e68(&mut self) {
        self.s15f8d();
        if self.ds.r16(a(0x1EDAA)) == 0x59 {
            return;
        }
        if self.ds.r16(a(0x1F186)) == 0x41 {
            self.l_15e9a();
            return;
        }
        let p = self.ds.r16(a(0x1F190));
        self.r.ax = p;
        self.ds.w16(a(0x1F190), p.wrapping_add(1));
        self.r.si = p;
        self.r.set_al(self.ds.r8(p));
        self.ds.w8(a(0x1E6C8), self.r.al());
        self.s15ecd();
        self.s15fd9();
        self.s15f31();
        self.s15f4c();
        self.s16145();
        self.s1617a();
    }

    /// `15E9Ah`（`sub_15E68` 的追蹤路徑，mode `A`）：以 (w9, w10) 求目標朝向，轉一步後當移動碼。
    fn l_15e9a(&mut self) {
        let target = self.aim_heading(self.ds.r16(a(0x1F188)), self.ds.r16(a(0x1F18A)));
        self.r.dx = turn_toward(target, self.ds.r16(a(0x1F190)));
        self.ds.w16(a(0x1F190), self.r.dx);
        self.ds.w8(a(0x1E6C8), self.r.dl());
        self.r.ax = self.r.dx;
        self.s15ecd();
        self.s15fd9();
        self.s15f31();
        self.s15f4c();
        self.s16145();
        self.s1617a();
    }

    /// `sub_15ECD`：腳本控制碼。`al` 為 F0h 跳轉（後接位址），E0h 變身（後接樣板位址），其他不處理。`si` 指向剛讀的控制碼。
    pub(crate) fn s15ecd(&mut self) {
        let al = self.r.al();
        if al == 0xF0 {
            // loc_15ED6：w13 = 位址 + 1，讀該處位元組。
            self.r.si = self.r.si.wrapping_add(1);
            self.r.ax = self.ds.r16(self.r.si);
            self.ds.w16(a(0x1F190), self.r.ax);
            let v = self.ds.r16(a(0x1F190)).wrapping_add(1);
            self.ds.w16(a(0x1F190), v);
            self.r.si = self.r.ax;
            self.r.set_al(self.ds.r8(self.r.si));
            self.ds.w8(a(0x1E6C8), self.r.al());
            return;
        }
        if al != 0xE0 {
            return;
        }
        // loc_15EE8：載入新樣板，保留目前的 w9、w10、w17。
        let w9 = self.ds.r16(a(0x1F188));
        let w10 = self.ds.r16(a(0x1F18A));
        let w17 = self.ds.r16(a(0x1F198));
        self.r.si = self.r.si.wrapping_add(1);
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.ax;
        self.r.di = 0x25B6;
        self.r.cx = 0x18;
        self.rep_movsw();
        self.r.ax = w17;
        self.ds.w16(a(0x1F198), self.r.ax);
        self.r.ax = w10;
        self.ds.w16(a(0x1F18A), self.r.ax);
        self.r.ax = w9;
        self.ds.w16(a(0x1F188), self.r.ax);
        self.ds.w16(a(0x1F17A), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F18A));
        self.r.bx = 0x0A;
        let p = self.r.ax as u32 * self.r.bx as u32;
        self.r.ax = p as u16;
        self.r.dx = (p >> 16) as u16;
        self.ds.w16(a(0x1F17C), self.r.ax);
        let q = self.ds.r16(a(0x1F190));
        self.r.ax = q;
        self.ds.w16(a(0x1F190), q.wrapping_add(1));
        self.r.si = q;
        self.r.set_al(self.ds.r8(q));
        self.ds.w8(a(0x1E6C8), self.r.al());
    }

    /// `sub_15F31`：mode `Y` 的實體隨景物左移。
    pub(crate) fn s15f31(&mut self) {
        if self.ds.r16(a(0x1F186)) == 0x59 {
            self.s15f3d();
        }
    }

    /// `sub_15F3D`：w2、w9 各減 1。
    pub(crate) fn s15f3d(&mut self) {
        self.r.ax = self.ds.r16(a(0x1F17A)).wrapping_sub(1);
        self.ds.w16(a(0x1F17A), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F188)).wrapping_sub(1);
        self.ds.w16(a(0x1F188), self.r.ax);
    }

    /// `sub_15F4C`：依移動碼選圖框（碼 0 至 8 取 frame_tbl[碼] + anim，其餘取 frame_tbl[0]），再推進動畫計數。
    /// 碼大於 8 時 `bx` 不乘 2，上限表就以位元組偏移 `bx` 讀字組（位置不對齊，照原版）。
    pub(crate) fn s15f4c(&mut self) {
        self.r.si = self.ds.r16(a(0x1F178));
        self.r.bx = 0;
        self.r.set_bl(self.ds.r8(a(0x1E6C8)));
        if self.r.bx <= 8 {
            self.r.bx = self.r.bx.wrapping_shl(1);
            self.r.ax = self.ds.r16(self.r.bx.wrapping_add(self.r.si));
            self.r.ax = self.r.ax.wrapping_add(self.ds.r16(a(0x1F1A0)));
            self.ds.w16(a(0x1E6C6), self.r.ax);
        } else {
            self.r.ax = self.ds.r16(self.r.si);
            self.ds.w16(a(0x1E6C6), self.r.ax);
        }
        if self.ds.r16(a(0x1F19E)) == 0 {
            return;
        }
        self.r.si = self.ds.r16(a(0x1F19E));
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(self.r.si));
        // cmp word_1E6C6, ax / jnb：圖框 < 上限才加 1。
        if self.ds.r16(a(0x1E6C6)) < self.r.ax {
            let v = self.ds.r16(a(0x1F1A0)).wrapping_add(1);
            self.ds.w16(a(0x1F1A0), v);
            return;
        }
        self.ds.w16(a(0x1F1A0), 0);
    }

    /// `sub_15F8D`：命中結算。w16 高位元組為 1 時取低位元組，累計傷害大於 hp 加難度修正就死亡並播爆炸，否則本幀閃爍。
    pub(crate) fn s15f8d(&mut self) {
        self.ds.w16(a(0x1E6B6), 0x4E);
        self.ds.w16(a(0x1EDAA), 0x4E);
        self.r.ax = self.ds.r16(a(0x1F196));
        if self.r.ah() != 1 {
            return;
        }
        self.r.set_ah(0);
        self.ds.w16(a(0x1F196), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F196));
        self.r.bx = self.ds.r16(a(0x1F176)).wrapping_add(self.ds.r16(a(0x1E6B2)));
        if self.r.ax <= self.r.bx {
            self.ds.w16(a(0x1E6B6), 0x59);
            return;
        }
        self.ds.w16(a(0x1EDAA), 0x59);
        let (x, y) = (self.ds.r16(a(0x1F188)), self.ds.r16(a(0x1F18A)));
        let letter = self.ds.r16(a(0x1F198));
        self.spawn_effect(EffectKind::from_letter(letter), x, y);
    }

    /// `sub_15FD9`：依移動碼 `al` 移動。1 至 8 為八向，9 追玩家 1 列，0Ah 右跳，0Bh 發射 `B`，0Fh 死亡，0Dh 先由 `sub_1606A` 換成上或下。
    pub(crate) fn s15fd9(&mut self) {
        self.ds.w16(a(0x1EDAA), 0x4E);
        self.s1606a();
        match self.r.al() {
            1 => self.s160b4(),
            3 => self.s1611d(),
            5 => self.s160c9(),
            7 => self.s16108(),
            2 => {
                self.s160b4();
                self.s1611d();
            }
            4 => {
                self.s160c9();
                self.s1611d();
            }
            6 => {
                self.s160c9();
                self.s16108();
            }
            8 => {
                self.s16108();
                self.s160b4();
            }
            9 => self.s1609c(),
            0x0A => self.s16132(),
            0x0B => {
                let (x, y) = (self.ds.r16(a(0x1F188)), self.ds.r16(a(0x1F18A)).wrapping_add(2));
                self.fire_enemy_bullets(u16::from(b'B'), x, y);
            }
            0x0F => self.ds.w16(a(0x1EDAA), 0x59),
            _ => {}
        }
    }

    /// `sub_1606A`：移動碼 0Dh 時求 (x+3, y+4) 到最近玩家的朝向，朝向 4 至 6 取 5（下），其餘取 1（上）。
    /// y+4 小於等於 0 取下，大於 14h 取上，這兩種不呼叫瞄準。
    pub(crate) fn s1606a(&mut self) {
        if self.r.al() != 0x0D {
            return;
        }
        let x = self.ds.r16(a(0x1F188)).wrapping_add(3);
        self.r.ax = self.ds.r16(a(0x1F18A)).wrapping_add(4);
        if (self.r.ax as i16) <= 0 {
            self.r.set_al(5);
            return;
        }
        if (self.r.ax as i16) > 0x14 {
            self.r.set_al(1);
            return;
        }
        self.r.ax = self.aim_heading(x, self.r.ax).code();
        let al = self.r.al();
        if al <= 3 || al >= 7 {
            self.r.set_al(1);
            return;
        }
        self.r.set_al(5);
    }

    /// `sub_1609C`：移動碼 9。w10 大於 18h 或小於玩家 1 列（`word_1DAC9`）就向下，否則向上。
    pub(crate) fn s1609c(&mut self) {
        if self.ds.r16(a(0x1F18A)) > 0x18 {
            self.s160f3();
            return;
        }
        self.r.ax = self.ds.r16(a(0x1F18A));
        if self.r.ax < self.ds.r16(a(0x1DAC9)) {
            self.s160f3();
            return;
        }
        self.s160de();
    }

    /// `sub_160B4`：向上，w3 減 w5、w10 減 w12。
    pub(crate) fn s160b4(&mut self) {
        self.r.ax = self.ds.r16(a(0x1F17C)).wrapping_sub(self.ds.r16(a(0x1F180)));
        self.ds.w16(a(0x1F17C), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F18A)).wrapping_sub(self.ds.r16(a(0x1F18E)));
        self.ds.w16(a(0x1F18A), self.r.ax);
    }

    /// `sub_160C9`：向下，w3 加 w5、w10 加 w12。
    pub(crate) fn s160c9(&mut self) {
        self.r.ax = self.ds.r16(a(0x1F17C)).wrapping_add(self.ds.r16(a(0x1F180)));
        self.ds.w16(a(0x1F17C), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F18A)).wrapping_add(self.ds.r16(a(0x1F18E)));
        self.ds.w16(a(0x1F18A), self.r.ax);
    }

    /// `sub_160DE`：向上，與 `sub_160B4` 相同的運算，給碼 9 用。
    pub(crate) fn s160de(&mut self) {
        self.r.ax = self.ds.r16(a(0x1F17C)).wrapping_sub(self.ds.r16(a(0x1F180)));
        self.ds.w16(a(0x1F17C), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F18A)).wrapping_sub(self.ds.r16(a(0x1F18E)));
        self.ds.w16(a(0x1F18A), self.r.ax);
    }

    /// `sub_160F3`：向下，與 `sub_160C9` 相同的運算，給碼 9 用。
    pub(crate) fn s160f3(&mut self) {
        self.r.ax = self.ds.r16(a(0x1F17C)).wrapping_add(self.ds.r16(a(0x1F180)));
        self.ds.w16(a(0x1F17C), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F18A)).wrapping_add(self.ds.r16(a(0x1F18E)));
        self.ds.w16(a(0x1F18A), self.r.ax);
    }

    /// `sub_16108`：向左，w2 減 w4、w9 減 w11。
    pub(crate) fn s16108(&mut self) {
        self.r.ax = self.ds.r16(a(0x1F17A)).wrapping_sub(self.ds.r16(a(0x1F17E)));
        self.ds.w16(a(0x1F17A), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F188)).wrapping_sub(self.ds.r16(a(0x1F18C)));
        self.ds.w16(a(0x1F188), self.r.ax);
    }

    /// `sub_1611D`：向右，w2 加 w4、w9 加 w11。
    pub(crate) fn s1611d(&mut self) {
        self.r.ax = self.ds.r16(a(0x1F17A)).wrapping_add(self.ds.r16(a(0x1F17E)));
        self.ds.w16(a(0x1F17A), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F188)).wrapping_add(self.ds.r16(a(0x1F18C)));
        self.ds.w16(a(0x1F188), self.r.ax);
    }

    /// `sub_16132`：右跳，w2、w9 各加 6。
    pub(crate) fn s16132(&mut self) {
        self.r.ax = self.ds.r16(a(0x1F17A)).wrapping_add(6);
        self.ds.w16(a(0x1F17A), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F188)).wrapping_add(6);
        self.ds.w16(a(0x1F188), self.r.ax);
    }

    /// `sub_16145`：存本幀渲染值（圖框、x、y 像素、kind），再寫 G3（`sub_161B0`）與 G1（`sub_16233`）。
    pub(crate) fn s16145(&mut self) {
        self.s16167();
        self.r.ax = self.ds.r16(a(0x26888));
        self.ds.w16(a(0x1EE4C), self.r.ax);
        self.r.ax = self.ds.r16(a(0x26880));
        self.ds.w16(a(0x1EE4E), self.r.ax);
        self.r.ax = self.ds.r16(a(0x26882));
        self.ds.w16(a(0x1EE50), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F19A));
        self.ds.w16(a(0x1EE52), self.r.ax);
        self.s161b0();
        self.s16233();
    }

    /// `sub_16167`：繪製參數 x = w2、y = w3、圖 = 本幀圖框 `word_1E6C6`。
    pub(crate) fn s16167(&mut self) {
        self.r.ax = self.ds.r16(a(0x1F17A));
        self.ds.w16(a(0x26880), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1F17C));
        self.ds.w16(a(0x26882), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1E6C6));
        self.ds.w16(a(0x26888), self.r.ax);
    }

    /// `sub_1617A`：隨機開火。每個實體的亂數索引 w22 先推進，w6 為 63h 永不開火，
    /// 否則 亂數值 + 難度修正 `word_1E6B4` 大於 w6 才以 w7 字母呼叫 `sub_16D3B`，起點 (w9, w10)。
    pub(crate) fn s1617a(&mut self) {
        self.r.ax = self.ds.r16(a(0x1F1A2));
        self.ds.w16(a(0x1CFEA), self.r.ax);
        self.s110a4();
        self.r.bx = self.ds.r16(a(0x1CFEA));
        self.ds.w16(a(0x1F1A2), self.r.bx);
        if self.ds.r16(a(0x1F182)) == 0x63 {
            return;
        }
        self.r.ax = self.r.ax.wrapping_add(self.ds.r16(a(0x1E6B4)));
        if self.r.ax <= self.ds.r16(a(0x1F182)) {
            return;
        }
        let (x, y) = (self.ds.r16(a(0x1F188)), self.ds.r16(a(0x1F18A)));
        self.fire_enemy_bullets(self.ds.r16(a(0x1F184)), x, y);
    }

    /// `sub_161B0`：把碰撞形狀以 `or` 寫進 G3 髒格（`964Ch` 起，每列 50h 位元組，奇數位元組）。
    /// 形狀在螢幕上方時先跳過整列（`sub_161FF`）。`bx` 保持原列號，所以列上限 `13h` 的檢查對負列放寬，照原版。
    pub(crate) fn s161b0(&mut self) {
        self.r.si = self.ds.r16(a(0x1F192));
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        self.ds.w16(a(0x2687E), self.r.ax);
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1F18A));
        self.s161ff();
        if (self.r.cx as i16) <= 0 {
            return;
        }
        loop {
            if (self.r.bx as i16) >= 0x13 {
                return;
            }
            let (cx, dx, di) = (self.r.cx, self.r.dx, self.r.di);
            self.s16225();
            self.r.ax = self.r.dx.wrapping_add(0x4E);
            loop {
                let ax = self.r.ax;
                if !(self.r.di > self.r.ax) && !(self.r.di < self.r.dx) {
                    self.r.set_al(self.ds.r8(self.r.si));
                    let v = self.ds.r8(self.r.di) | self.r.al();
                    self.ds.w8(self.r.di, v);
                }
                self.r.ax = ax;
                self.r.di = self.r.di.wrapping_add(2);
                self.r.si = self.r.si.wrapping_add(1);
                self.r.cx = self.r.cx.wrapping_sub(1);
                if self.r.cx == 0 {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.di = di.wrapping_add(0x50);
            self.r.dx = dx.wrapping_add(0x50);
            self.r.cx = cx;
            self.r.cx = self.r.cx.wrapping_sub(1);
            if self.r.cx == 0 {
                break;
            }
        }
    }

    /// `sub_161FF`：G3 的起點。列號為負時跳過形狀的列（`bx` 加 1、`cx` 減 1、`si` 前進一列寬），
    /// 之後 `di` = `964Ch` + 列偏移 + 2 × w9 + 1，`dx` = 該列的奇數位元組起點。`bx` 還原成呼叫時的值。
    pub(crate) fn s161ff(&mut self) {
        let saved_bx = self.r.bx;
        while (self.r.bx as i16) < 0 {
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = self.r.cx.wrapping_sub(1);
            self.r.si = self.r.si.wrapping_add(self.ds.r16(a(0x2687E)));
        }
        self.r.bx = self.r.bx.wrapping_shl(1);
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x21C2));
        self.r.ax = self.r.ax.wrapping_add(0x964C);
        self.r.di = self.r.ax;
        self.r.dx = self.r.ax;
        self.r.dx = self.r.dx.wrapping_add(1);
        self.r.ax = self.ds.r16(a(0x1F188));
        self.r.ax = self.r.ax.wrapping_shl(1);
        self.r.ax = self.r.ax.wrapping_add(1);
        self.r.di = self.r.di.wrapping_add(self.r.ax);
        self.r.bx = saved_bx;
    }

    /// `sub_16225`：`cx` = 形狀寬 `word_2687E`。
    pub(crate) fn s16225(&mut self) {
        self.r.cx = self.ds.r16(a(0x2687E));
    }

    /// `sub_16233`：把碰撞形狀寫進 G1 實體格（`1B32h` 起，每列 28h 位元組）。
    /// 格值 0 略過，FFh 與 FEh 照寫，其餘加上槽號 `byte_1E6C9`，覆蓋而不是 or。
    pub(crate) fn s16233(&mut self) {
        self.r.si = self.ds.r16(a(0x1F192));
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        self.ds.w16(a(0x2687E), self.r.ax);
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1F18A));
        self.s16290();
        if (self.r.cx as i16) <= 0 {
            return;
        }
        loop {
            if (self.r.bx as i16) >= 0x13 {
                return;
            }
            let (cx, dx, di) = (self.r.cx, self.r.dx, self.r.di);
            self.s16225();
            self.r.ax = self.r.dx.wrapping_add(0x27);
            loop {
                let ax = self.r.ax;
                if !(self.r.di > self.r.ax) && !(self.r.di < self.r.dx) {
                    self.r.set_al(self.ds.r8(self.r.si));
                    let al = self.r.al();
                    if al != 0 {
                        if al != 0xFF && al != 0xFE {
                            self.r.set_al(al.wrapping_add(self.ds.r8(a(0x1E6C9))));
                        }
                        self.ds.w8(self.r.di, self.r.al());
                    }
                }
                self.r.ax = ax;
                self.r.di = self.r.di.wrapping_add(1);
                self.r.si = self.r.si.wrapping_add(1);
                self.r.cx = self.r.cx.wrapping_sub(1);
                if self.r.cx == 0 {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.di = di.wrapping_add(0x28);
            self.r.dx = dx.wrapping_add(0x28);
            self.r.cx = cx;
            self.r.cx = self.r.cx.wrapping_sub(1);
            if self.r.cx == 0 {
                break;
            }
        }
    }

    /// `sub_16290`：G1 的起點。列號為負時跳過形狀的列，之後 `dx` = `1B32h` + 列偏移（`1B0Ah` 表），`di` = `dx` + w9。`bx` 還原。
    pub(crate) fn s16290(&mut self) {
        let saved_bx = self.r.bx;
        while (self.r.bx as i16) < 0 {
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = self.r.cx.wrapping_sub(1);
            self.r.si = self.r.si.wrapping_add(self.ds.r16(a(0x2687E)));
        }
        self.r.bx = self.r.bx.wrapping_shl(1);
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x1B0A));
        self.r.ax = self.r.ax.wrapping_add(0x1B32);
        self.r.di = self.r.ax;
        self.r.dx = self.r.ax;
        self.r.di = self.r.di.wrapping_add(self.ds.r16(a(0x1F188)));
        self.r.bx = saved_bx;
    }

    /// `sub_162B1`：清空 G1（`1B32h` 起 190h 個字組）。
    pub(crate) fn s162b1(&mut self) {
        self.r.di = 0x1B32;
        self.r.cx = 0x190;
        self.r.ax = 0;
        self.ds.fill16(0x1B32, 0x190, 0);
        self.r.di = 0x1B32u16.wrapping_add(0x190 * 2);
        self.r.cx = 0;
    }

    /// `sub_17D82`：第一關的實體時間表 `dseg:41BEh`。
    pub(crate) fn s17d82(&mut self) {
        self.r.si = 0x41BE;
        self.s15d1c();
    }
}
