//! 敵方子彈：派發、瞄準、更新與 G2。
//! 逐函式移植自機器碼，查法見 `tools/q.sh`。規格：`docs/spec/001-rust-first-stage.md`。
//!
//! 敵彈池：20 槽，指標表 `ds:32B3h`，記錄 15 字組（30 位元組），基底 `ds:32DBh`。
//! `sub_1739B` 把記錄複製到工作區 `ds:3535h`（`word_200F5` 起，w0 在 `200F5h`），`sub_173B0` 複製回去，
//! `word_200F3`（`ds:3533h`）存記錄位址，也是 `sub_16CEA` 讀取樣板位址的變數。
//! 生成參數：欄 `word_1FE6D`、列 `word_1FE6F`。`word_1FE71` 為 `Y` 表示本槽要釋放。
//! 目標座標：欄 `word_1F7D3`、列 `word_1F7D5`。

use crate::draw::Pool;
use crate::dseg::a;
use crate::fx::EffectKind;
use crate::game::Core;

impl Core {
    /// 設 `word_200F3` 為樣板位址再呼叫 `sub_16CEA`（機器碼裡 `mov word_200F3, imm` / `call sub_16CEA` 的固定組合）。
    fn eb_spawn(&mut self, tmpl: u16) {
        self.ds.w16(a(0x200F3), tmpl);
        self.s16cea();
    }

    /// 八向樣板表的選擇，`ax` 為朝向 1 至 8，其他值取第一項。
    /// 表的間隔是 1Eh（`sub_1701E`、`170B0`、`17117`、`1717E`、`171E5` 的 40 個立即值已逐項核對）。
    fn eb_pick8(&mut self, base: u16) {
        let ax = self.r.ax;
        let t = if (1..=8).contains(&ax) { base.wrapping_add(0x1E * (ax - 1)) } else { base };
        self.ds.w16(a(0x200F3), t);
    }

    /// `sub_16D3B`：以字母（`ax`）派發敵彈，起點是 `word_1FE6D`、`word_1FE6F`。
    /// 瞄準類（`F`、`l`、`B`、`b`、`k`、`s`、`E`）先呼叫 `sub_16F9F` 取朝向再選樣板。
    pub(crate) fn s16d3b(&mut self) {
        match self.r.ax {
            0x46 => {
                // F：八向瞄準彈。
                self.s16f9f();
                self.s1701e();
                self.s16cea();
            }
            0x6C => {
                // l：L 道具球。
                self.s16f9f();
                self.s170b0();
                self.s16cea();
            }
            0x41 => {
                // A：沒有樣板使用。
                self.eb_spawn(0x3AF7);
            }
            0x42 => {
                // B：列 +1。
                let v = self.ds.r16(a(0x1FE6F)).wrapping_add(1);
                self.ds.w16(a(0x1FE6F), v);
                self.s16f9f();
                self.s17085();
                self.s16cea();
            }
            0x4C => {
                // L：欄 +8，五顆。
                let v = self.ds.r16(a(0x1FE6D)).wrapping_add(8);
                self.ds.w16(a(0x1FE6D), v);
                for t in [0x3B6F, 0x3B8D, 0x3BAB, 0x3BC9, 0x3BE7] {
                    self.eb_spawn(t);
                }
            }
            0x56 => {
                // V：欄 -12，一幀。
                self.ds.w16(a(0x200F3), 0x3C05);
                let v = self.ds.r16(a(0x1FE6D)).wrapping_sub(0x0C);
                self.ds.w16(a(0x1FE6D), v);
                self.s16cea();
            }
            0x62 => {
                // b：B 道具球。
                self.s16f9f();
                self.s17117();
                self.s16cea();
            }
            0x6B => {
                // k：K 道具球。
                self.s16f9f();
                self.s1717e();
                self.s16cea();
            }
            0x73 => {
                // s：S 道具球。
                self.s16f9f();
                self.s171e5();
                self.s16cea();
            }
            0x31 => {
                // 1：欄 +4，列 -2。
                self.ds.w16(a(0x200F3), 0x3C23);
                let v = self.ds.r16(a(0x1FE6D)).wrapping_add(4);
                self.ds.w16(a(0x1FE6D), v);
                let v = self.ds.r16(a(0x1FE6F)).wrapping_sub(2);
                self.ds.w16(a(0x1FE6F), v);
                self.s16cea();
            }
            0x32 => {
                // 2：欄 +3，列 -2。
                self.ds.w16(a(0x200F3), 0x3C23);
                let v = self.ds.r16(a(0x1FE6D)).wrapping_add(3);
                self.ds.w16(a(0x1FE6D), v);
                let v = self.ds.r16(a(0x1FE6F)).wrapping_sub(2);
                self.ds.w16(a(0x1FE6F), v);
                self.s16cea();
            }
            0x58 => {
                // X：N、W、WNW。
                for t in [0x3C41, 0x3CF5, 0x3D13] {
                    self.eb_spawn(t);
                }
            }
            0x43 => {
                // C：ENE、E、ESE。
                for t in [0x3C5F, 0x3C7D, 0x3C9B] {
                    self.eb_spawn(t);
                }
            }
            0x3C => {
                // <：八向放射。
                self.l_16ebb();
            }
            0x51 => {
                // Q：邊界反彈球，欄 +4，第二顆再 +4。
                let v = self.ds.r16(a(0x1FE6D)).wrapping_add(4);
                self.ds.w16(a(0x1FE6D), v);
                self.eb_spawn(0x3D8B);
                let v = self.ds.r16(a(0x1FE6D)).wrapping_add(4);
                self.ds.w16(a(0x1FE6D), v);
                self.eb_spawn(0x3DA9);
            }
            0x3F => {
                // ?：列先 +4 再接 < 的 +2。
                let v = self.ds.r16(a(0x1FE6F)).wrapping_add(4);
                self.ds.w16(a(0x1FE6F), v);
                self.l_16ebb();
            }
            0x55 => {
                // U：WNW + ENE。
                self.eb_spawn(0x3D13);
                self.eb_spawn(0x3C5F);
            }
            0x44 => {
                // D：ESE + WSW。
                self.eb_spawn(0x3C9B);
                self.eb_spawn(0x3CD7);
            }
            0x45 => {
                // E：三顆慢彈，接六發瞄準彈。
                self.eb_spawn(0x3D6D);
                self.eb_spawn(0x3D4F);
                self.eb_spawn(0x3D31);
                self.eb_aimed();
                let v = self.ds.r16(a(0x1FE6D)).wrapping_add(4);
                self.ds.w16(a(0x1FE6D), v);
                let v = self.ds.r16(a(0x1FE6F)).wrapping_sub(8);
                self.ds.w16(a(0x1FE6F), v);
                self.eb_aimed();
                for _ in 0..4 {
                    let v = self.ds.r16(a(0x1FE6F)).wrapping_add(3);
                    self.ds.w16(a(0x1FE6F), v);
                    self.eb_aimed();
                }
            }
            _ => {
                // 其他字母：一顆向上的 F 彈。
                self.eb_spawn(0x3557);
            }
        }
    }

    /// `loc_16EBB`（`sub_16D3B` 中段，`<` 與 `?` 共用）：列 +2，八向放射八顆。
    fn l_16ebb(&mut self) {
        let v = self.ds.r16(a(0x1FE6F)).wrapping_add(2);
        self.ds.w16(a(0x1FE6F), v);
        for t in [0x3C41, 0x3C5F, 0x3C7D, 0x3C9B, 0x3CB9, 0x3CD7, 0x3CF5, 0x3D13] {
            self.eb_spawn(t);
        }
    }

    /// `sub_16D3B` 的 `E` 分支反覆出現的三連呼叫：取朝向、選 F 樣板、生成。
    fn eb_aimed(&mut self) {
        self.s16f9f();
        self.s1701e();
        self.s16cea();
    }

    /// `sub_16F9F`：瞄準。先 `sub_172DE` 選目標，回傳朝向 `ax`（1 至 8），所有比較為無符號。
    /// 目標在上方（發射點 `y-3` 在 0 至 20 且大於目標列）：`x-10 > tx` 得 8，`x+10 < tx` 得 2，否則 1。
    /// 目標在下方（`y+3 < ty`）：同樣得 6、4、5。其餘（同列）：`x > tx` 得 7，否則 3。
    pub(crate) fn s16f9f(&mut self) {
        self.s172de();
        let x = self.ds.r16(a(0x1FE6D));
        let y = self.ds.r16(a(0x1FE6F));
        let tx = self.ds.r16(a(0x1F7D3));
        let ty = self.ds.r16(a(0x1F7D5));
        let up = y.wrapping_sub(3);
        self.r.ax = up;
        if up <= 0x14 && up > ty {
            // 16FB6
            let ax = x.wrapping_sub(0x0A);
            self.r.ax = ax;
            if ax > tx {
                self.r.ax = 8;
                return;
            }
            let ax = x.wrapping_add(0x0A);
            self.r.ax = ax;
            if ax < tx {
                self.r.ax = 2;
                return;
            }
            self.r.ax = 1;
            return;
        }
        // 16FDA
        let ax = y.wrapping_add(3);
        self.r.ax = ax;
        if ax < ty {
            // 16FE9
            let ax = x.wrapping_sub(0x0A);
            self.r.ax = ax;
            if ax > tx {
                self.r.ax = 6;
                return;
            }
            let ax = x.wrapping_add(0x0A);
            self.r.ax = ax;
            if ax < tx {
                self.r.ax = 4;
                return;
            }
            self.r.ax = 5;
            return;
        }
        // 1700D
        self.r.ax = x;
        self.r.ax = if x > tx { 7 } else { 3 };
    }

    /// `sub_1701E`：F 樣板（`3557h` 起，間隔 1Eh）依朝向 `ax` 選取。
    pub(crate) fn s1701e(&mut self) {
        self.eb_pick8(0x3557);
    }

    /// `sub_17085`：B 樣板。朝向 8、7、6 分別選 `3B15h`、`3B33h`、`3B51h`，其他取 `3B33h`。
    pub(crate) fn s17085(&mut self) {
        let t = match self.r.ax {
            8 => 0x3B15,
            7 => 0x3B33,
            6 => 0x3B51,
            _ => 0x3B33,
        };
        self.ds.w16(a(0x200F3), t);
    }

    /// `sub_170B0`：`l` 道具球樣板（`3737h` 起）。
    pub(crate) fn s170b0(&mut self) {
        self.eb_pick8(0x3737);
    }

    /// `sub_17117`：`b` 道具球樣板（`3827h` 起）。
    pub(crate) fn s17117(&mut self) {
        self.eb_pick8(0x3827);
    }

    /// `sub_1717E`：`k` 道具球樣板（`3917h` 起）。
    pub(crate) fn s1717e(&mut self) {
        self.eb_pick8(0x3917);
    }

    /// `sub_171E5`：`s` 道具球樣板（`3A07h` 起）。
    /// `1724Ch` 至 `172DDh` 的碼在 `retn` 之後無跳入，不移植。
    pub(crate) fn s171e5(&mut self) {
        self.eb_pick8(0x3A07);
    }

    /// `sub_172DE`：選瞄準目標，寫 `word_1F7D3`（欄）與 `word_1F7D5`（列）。
    /// `byte_1CFDD` 位元 0 為玩家 1 存活、位元 4 為玩家 2 存活。只有一位存活取該位，兩位都在時取欄距較小者，
    /// 平手規則見 `docs/re/11-entities-combat.md` 第 7.3 節，這裡照機器碼的比較順序（全部無符號）。
    pub(crate) fn s172de(&mut self) {
        let alive = self.ds.r8(a(0x1CFDD));
        // true 取玩家 1（`1DACB`、`1DAC9`），false 取玩家 2（`1DB39`、`1DB37`）。
        let p1 = if alive & 1 == 0 {
            false
        } else if alive & 0x10 == 0 {
            true
        } else {
            let x = self.ds.r16(a(0x1FE6D));
            let c1 = self.ds.r16(a(0x1DACB));
            let c2 = self.ds.r16(a(0x1DB39));
            self.r.ax = x;
            if x > c1 {
                // 172F8
                if x > c2 {
                    // 17301：兩位都在左邊，取欄較大者（較近）；相等取玩家 2。
                    self.r.ax = c1;
                    c1 > c2
                } else {
                    // 1734B
                    let bx = x.wrapping_sub(c1);
                    let ax = c2.wrapping_sub(x);
                    self.r.bx = bx;
                    self.r.ax = ax;
                    !(bx > ax)
                }
            } else {
                // 17324
                if x > c2 {
                    // 17335：在兩位之間。
                    let bx = x.wrapping_sub(c2);
                    let ax = c1.wrapping_sub(x);
                    self.r.bx = bx;
                    self.r.ax = ax;
                    bx > ax
                } else {
                    // 兩位都在右邊，取欄較小者；相等取玩家 1。
                    self.r.ax = c1;
                    !(c1 > c2)
                }
            }
        };
        if p1 {
            // 17317
            let ax = self.ds.r16(a(0x1DACB));
            self.ds.w16(a(0x1F7D3), ax);
            let ax = self.ds.r16(a(0x1DAC9));
            self.r.ax = ax;
            self.ds.w16(a(0x1F7D5), ax);
        } else {
            // 1730A
            let ax = self.ds.r16(a(0x1DB39));
            self.ds.w16(a(0x1F7D3), ax);
            let ax = self.ds.r16(a(0x1DB37));
            self.r.ax = ax;
            self.ds.w16(a(0x1F7D5), ax);
        }
    }

    /// `sub_17361`：更新 20 顆敵彈。每槽複製進工作區、取腳本移動繪製、複製回去，
    /// `word_1FE71` 為 `Y` 時釋放槽。`byte_1F7DB` 是槽號，G2 寫入時加進格值。
    pub(crate) fn s17361(&mut self) {
        self.r.bx = 0;
        self.r.cx = 0x14;
        self.ds.w8(a(0x1F7DB), 0);
        loop {
            let cx = self.r.cx;
            self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x32B3));
            if self.r.ax != 0 {
                let bx = self.r.bx;
                self.set_obj(Pool::EnemyBullet, (bx >> 1) as u8);
                self.s1739b();
                self.s173c5();
                self.s173b0();
                self.r.bx = bx;
                if self.ds.r16(a(0x1FE71)) == 0x59 {
                    self.r.ax = 0;
                    self.ds.w16(bx.wrapping_add(0x32B3), 0);
                }
            }
            self.r.bx = self.r.bx.wrapping_add(2);
            let n = self.ds.r8(a(0x1F7DB)).wrapping_add(1);
            self.ds.w8(a(0x1F7DB), n);
            self.r.cx = cx.wrapping_sub(1);
            if self.r.cx == 0 {
                break;
            }
        }
        self.clear_obj();
    }

    /// `sub_1739B`：`ax` 指的記錄複製到工作區 `3535h`，位址存進 `word_200F3`。
    pub(crate) fn s1739b(&mut self) {
        let ax = self.r.ax;
        self.ds.w16(a(0x200F3), ax);
        self.ds.copy16(0x3535, ax, 0x0F);
        self.r.si = ax.wrapping_add(0x1E);
        self.r.di = 0x3535u16.wrapping_add(0x1E);
        self.r.cx = 0;
    }

    /// `sub_173B0`：工作區複製回 `word_200F3` 指的記錄。
    pub(crate) fn s173b0(&mut self) {
        let ax = self.ds.r16(a(0x200F3));
        self.r.ax = ax;
        self.ds.copy16(ax, 0x3535, 0x0F);
        self.r.di = ax.wrapping_add(0x1E);
        self.r.si = 0x3535u16.wrapping_add(0x1E);
        self.r.cx = 0;
    }

    /// `sub_173C5`：取腳本位元組（指標 `word_2010F` 加 1），低 4 位元放 `al`、高 4 位元放 `ah`，
    /// 移動、繪製並寫 G2 與 G3、播尾跡。
    pub(crate) fn s173c5(&mut self) {
        let ax = self.ds.r16(a(0x2010F));
        self.ds.w16(a(0x2010F), ax.wrapping_add(1));
        self.r.si = ax;
        let b = self.ds.r8(ax);
        self.r.ax = (b as u16) << 8 | (b & 0x0F) as u16;
        self.r.set_ah(b & 0xF0);
        self.s17631();
        self.s17768();
        self.s173e1();
    }

    /// `sub_173E1`：`w7` 為 `Y` 時在 (w9, w10) 播特效 `E`（尾跡）。
    pub(crate) fn s173e1(&mut self) {
        if self.ds.r16(a(0x20103)) != 0x59 {
            return;
        }
        let (x, y) = (self.ds.r16(a(0x20107)), self.ds.r16(a(0x20109)));
        let o = self.cur_obj;
        self.spawn_effect(EffectKind::Trail, x, y);
        self.cur_obj = o;
    }

    /// `sub_173FE`：依目前朝向 `ax`（1 至 8）選轉向表再呼叫 `sub_175EA`，其他值 `dx` = 7。
    /// 呼叫端是實體腳本 `sub_15E68`（追蹤路徑），期望的朝向在 `dx`。
    pub(crate) fn s173fe(&mut self) {
        match self.r.ax {
            1 => self.s1744a(),
            2 => self.s1747e(),
            3 => self.s174b2(),
            4 => self.s174e6(),
            5 => self.s1751a(),
            6 => self.s1754e(),
            7 => self.s17582(),
            8 => self.s175b6(),
            _ => self.r.dx = 7,
        }
    }

    /// 寫 `word_1F7C2` 起 8 個字組（轉向表），再呼叫 `sub_175EA`。
    fn eb_turn(&mut self, t: [u16; 8]) {
        for (i, v) in t.into_iter().enumerate() {
            self.ds.w16(a(0x1F7C2) + 2 * i as u16, v);
        }
        self.s175ea();
    }

    /// `sub_1744A`：目前朝向 1 的轉向表。
    pub(crate) fn s1744a(&mut self) {
        self.eb_turn([2, 3, 4, 5, 6, 7, 8, 1]);
    }

    /// `sub_1747E`：朝向 2。
    pub(crate) fn s1747e(&mut self) {
        self.eb_turn([3, 4, 5, 6, 7, 1, 8, 1]);
    }

    /// `sub_174B2`：朝向 3。
    pub(crate) fn s174b2(&mut self) {
        self.eb_turn([4, 5, 6, 7, 1, 2, 8, 1]);
    }

    /// `sub_174E6`：朝向 4。
    pub(crate) fn s174e6(&mut self) {
        self.eb_turn([5, 6, 7, 8, 1, 2, 3, 4]);
    }

    /// `sub_1751A`：朝向 5。
    pub(crate) fn s1751a(&mut self) {
        self.eb_turn([6, 7, 8, 2, 3, 4, 1, 8]);
    }

    /// `sub_1754E`：朝向 6。
    pub(crate) fn s1754e(&mut self) {
        self.eb_turn([7, 8, 2, 3, 4, 5, 1, 8]);
    }

    /// `sub_17582`：朝向 7。
    pub(crate) fn s17582(&mut self) {
        self.eb_turn([8, 3, 2, 4, 5, 6, 1, 8]);
    }

    /// `sub_175B6`：朝向 8。
    pub(crate) fn s175b6(&mut self) {
        self.eb_turn([2, 3, 4, 5, 6, 7, 1, 8]);
    }

    /// `sub_175EA`：用轉向表把目前朝向 `ax` 轉向期望朝向 `dx`，結果在 `dx`。
    /// 相同則不動。`dx` 等於表 0 至 3 項的值減 1，等於表 4、5 項的值加 1，等於表 6 項取表 7 項，
    /// 都不是取 1。
    pub(crate) fn s175ea(&mut self) {
        if self.r.dx == self.r.ax {
            self.r.dx = self.r.ax;
            return;
        }
        let mut ax = self.r.dx;
        let t = |g: &Core, i: u32| g.ds.r16(a(0x1F7C2 + 2 * i));
        if ax == t(self, 0) || ax == t(self, 1) || ax == t(self, 2) || ax == t(self, 3) {
            ax = ax.wrapping_sub(1);
        } else if ax == t(self, 4) || ax == t(self, 5) {
            ax = ax.wrapping_add(1);
        } else if ax == t(self, 6) {
            ax = self.ds.r16(a(0x1F7D0));
        } else {
            ax = 1;
        }
        self.r.ax = ax;
        self.r.dx = ax;
    }

    /// `sub_17631`：依移動碼（`al`）移動。1、3、5、7 為上右下左，2、4、6、8 為上右、下右、下左、左上，
    /// 9 在頁 0 時不動（`ax` = 0、`w1FE71` = `N`）、頁 8000h 時改依 w0 的朝向移動，0 結束（`w1FE71` = `Y`）。
    /// 移動碼存進 `byte_1F7D2`，9 的重新派發不更新它。
    pub(crate) fn s17631(&mut self) {
        self.s176b5();
        self.ds.w8(a(0x1F7D2), self.r.al());
        self.ds.w16(a(0x1FE71), 0x4E);
        loop {
            match self.r.al() {
                1 => self.s176d0(),
                3 => self.s17742(),
                5 => self.s176f6(),
                7 => self.s1771c(),
                2 => {
                    self.s176d0();
                    self.s17742();
                }
                4 => {
                    self.s176f6();
                    self.s17742();
                }
                6 => {
                    self.s176f6();
                    self.s1771c();
                }
                8 => {
                    self.s1771c();
                    self.s176d0();
                }
                9 => {
                    if self.ds.r16(a(0x26872)) != 0 {
                        self.r.ax = self.ds.r16(a(0x200F5));
                        continue;
                    }
                    self.r.ax = 0;
                    self.ds.w16(a(0x1FE71), 0x4E);
                }
                0 => self.ds.w16(a(0x1FE71), 0x59),
                _ => {}
            }
            return;
        }
    }

    /// `sub_176B5`：`ah` 為 `F0h` 時往回跳 `al` 個位元組，不遞增指標，以該處位元組當本幀指令。
    pub(crate) fn s176b5(&mut self) {
        if self.r.ah() != 0xF0 {
            return;
        }
        self.r.set_ah(0);
        let p = self.ds.r16(a(0x2010F)).wrapping_sub(self.r.ax);
        self.ds.w16(a(0x2010F), p);
        self.r.ax = p;
        self.r.si = p;
        let b = self.ds.r8(p);
        self.r.ax = (b as u16) << 8 | (b & 0x0F) as u16;
        self.r.set_ah(b & 0xF0);
    }

    /// `sub_176D0`：往上。y_px 減 dy_px、列減 dy 步，列 >= 13h（無符號）設 `w1FE71` = `Y` 並呼叫 `sub_17955`。
    pub(crate) fn s176d0(&mut self) {
        let ax = self.ds.r16(a(0x200FB)).wrapping_sub(self.ds.r16(a(0x200FF)));
        self.ds.w16(a(0x200FB), ax);
        let ax = self.ds.r16(a(0x20109)).wrapping_sub(self.ds.r16(a(0x2010D)));
        self.r.ax = ax;
        self.ds.w16(a(0x20109), ax);
        if self.ds.r16(a(0x20109)) >= 0x13 {
            self.ds.w16(a(0x1FE71), 0x59);
            self.s17955();
        }
    }

    /// `sub_176F6`：往下，列 >= 12h 設 `Y` 並呼叫 `sub_1798E`。
    pub(crate) fn s176f6(&mut self) {
        let ax = self.ds.r16(a(0x200FB)).wrapping_add(self.ds.r16(a(0x200FF)));
        self.ds.w16(a(0x200FB), ax);
        let ax = self.ds.r16(a(0x20109)).wrapping_add(self.ds.r16(a(0x2010D)));
        self.r.ax = ax;
        self.ds.w16(a(0x20109), ax);
        if self.ds.r16(a(0x20109)) >= 0x12 {
            self.ds.w16(a(0x1FE71), 0x59);
            self.s1798e();
        }
    }

    /// `sub_1771C`：往左。x_px 減 dx_px、欄減 dx 步，欄 >= 27h 設 `Y` 並呼叫 `sub_17A00`。
    pub(crate) fn s1771c(&mut self) {
        let ax = self.ds.r16(a(0x200F9)).wrapping_sub(self.ds.r16(a(0x200FD)));
        self.ds.w16(a(0x200F9), ax);
        let ax = self.ds.r16(a(0x20107)).wrapping_sub(self.ds.r16(a(0x2010B)));
        self.r.ax = ax;
        self.ds.w16(a(0x20107), ax);
        if self.ds.r16(a(0x20107)) >= 0x27 {
            self.ds.w16(a(0x1FE71), 0x59);
            self.s17a00();
        }
    }

    /// `sub_17742`：往右，欄 >= 27h 設 `Y` 並呼叫 `sub_179C7`。
    pub(crate) fn s17742(&mut self) {
        let ax = self.ds.r16(a(0x200F9)).wrapping_add(self.ds.r16(a(0x200FD)));
        self.ds.w16(a(0x200F9), ax);
        let ax = self.ds.r16(a(0x20107)).wrapping_add(self.ds.r16(a(0x2010B)));
        self.r.ax = ax;
        self.ds.w16(a(0x20107), ax);
        if self.ds.r16(a(0x20107)) >= 0x27 {
            self.ds.w16(a(0x1FE71), 0x59);
            self.s179c7();
        }
    }

    /// `sub_17768`：畫敵彈。`w8` 為 `M`（4Dh）用 MY 圖庫（遮罩、y 加 1、平面），否則用舞台圖庫。
    /// 再標 G3 髒格、寫 G2。
    pub(crate) fn s17768(&mut self) {
        let ax = self.ds.r16(a(0x200F9));
        self.ds.w16(a(0x26880), ax);
        let ax = self.ds.r16(a(0x200FB));
        self.r.ax = ax;
        self.ds.w16(a(0x26882), ax);
        self.s1779b();
        if self.ds.r16(a(0x20105)) == 0x4D {
            self.s18d46();
            let y = self.ds.r16(a(0x26882)).wrapping_add(1);
            self.ds.w16(a(0x26882), y);
            self.s18ce2();
        } else {
            self.s18f1a();
            self.s18f5d();
        }
        self.s177d2();
        self.s17855();
    }

    /// `sub_1779B`：圖索引。`w1` 指的圖框表（9 字組）以移動碼 `byte_1F7D2`（0 至 8）索引，
    /// 大於 8 時取第 0 項。
    pub(crate) fn s1779b(&mut self) {
        let si = self.ds.r16(a(0x200F7));
        self.r.si = si;
        let bx = self.ds.r8(a(0x1F7D2)) as u16;
        self.r.bx = bx;
        let ax = if bx > 8 {
            self.ds.r16(si)
        } else {
            self.r.bx = bx << 1;
            self.ds.r16(si.wrapping_add(bx << 1))
        };
        self.r.ax = ax;
        self.ds.w16(a(0x26888), ax);
    }

    /// `sub_177D2`：把形狀（`w14` 指的寬、高、格值）用 OR 寫進 G3 髒格。列為負的子彈
    /// `sub_17821` 會調整 `si`、`cx`，但 `bx` 還原成負值，下面 `bx >= 12h`（無符號）立刻離開，所以不寫。
    /// `177B8h` 至 `177D1h` 的碼在 `retn` 之後無跳入，不移植。
    pub(crate) fn s177d2(&mut self) {
        let mut si = self.ds.r16(a(0x20111));
        let w = self.ds.r16(si);
        si = si.wrapping_add(2);
        self.ds.w16(a(0x2687E), w);
        let h = self.ds.r16(si);
        si = si.wrapping_add(2);
        self.r.si = si;
        self.r.cx = h;
        self.r.bx = self.ds.r16(a(0x20109));
        self.s17821();
        if (self.r.cx as i16) <= 0 {
            return;
        }
        loop {
            if self.r.bx >= 0x12 {
                return;
            }
            let (cx, dx, di) = (self.r.cx, self.r.dx, self.r.di);
            self.s17847();
            let lim = dx.wrapping_add(0x4E);
            self.r.ax = lim;
            loop {
                if self.r.di <= lim && self.r.di >= self.r.dx {
                    let al = self.ds.r8(self.r.si);
                    let v = self.ds.r8(self.r.di) | al;
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
            self.r.di = di.wrapping_add(0x50);
            self.r.dx = dx.wrapping_add(0x50);
            self.r.cx = cx.wrapping_sub(1);
            if self.r.cx == 0 {
                break;
            }
        }
    }

    /// `sub_17821`：G3 髒格的起點。輸入 `si`、`bx`（列）、`cx`（高）。列為負（有號）時 `si` 跳過一行、
    /// `bx` 加 1、`cx` 減 1，直到列 >= 0。回傳 `di` = `964Ch` + 列基底 + 欄 × 2 + 1，`dx` = 列基底 + 1，
    /// `bx` 還原成輸入值。
    pub(crate) fn s17821(&mut self) {
        let bx0 = self.r.bx;
        let mut bx = bx0;
        while (bx as i16) < 0 {
            self.r.si = self.r.si.wrapping_add(self.ds.r16(a(0x2687E)));
            bx = bx.wrapping_add(1);
            self.r.cx = self.r.cx.wrapping_sub(1);
        }
        let base = self.ds.r16((bx << 1).wrapping_add(0x21C2)).wrapping_add(0x964C);
        self.r.dx = base.wrapping_add(1);
        let c = (self.ds.r16(a(0x20107)) << 1).wrapping_add(1);
        self.r.ax = c;
        self.r.di = base.wrapping_add(c);
        self.r.bx = bx0;
    }

    /// `sub_17847`：`cx` = 形狀寬 `word_2687E`。
    pub(crate) fn s17847(&mut self) {
        self.r.cx = self.ds.r16(a(0x2687E));
    }

    /// `sub_17855`：把形狀格值加槽號（`byte_1F7DB`）寫進 G2 `2C1Ch` 起。可寫範圍欄 1 至 38、
    /// `2C1Ch` <= `di` <= `2F3Ch`，格值 0 不寫。列基底見 `sub_17898`，列 >= 14h（含負列）別名到 G1。
    /// 8 位元相加，不處理進位。
    pub(crate) fn s17855(&mut self) {
        let mut si = self.ds.r16(a(0x20111));
        let w = self.ds.r16(si);
        si = si.wrapping_add(2);
        self.ds.w16(a(0x2687E), w);
        let mut cx = self.ds.r16(si);
        si = si.wrapping_add(2);
        let mut bx = self.ds.r16(a(0x20109));
        let mut di;
        let mut dx;
        loop {
            let outer = cx;
            self.r.bx = bx;
            self.s17898();
            let ax = self.r.ax;
            dx = self.r.dx;
            di = self.r.di;
            let mut icx = self.r.cx;
            loop {
                if di <= 0x2F3C && di >= 0x2C1C && di < dx && di > ax {
                    let mut al = self.ds.r8(si);
                    self.r.set_al(al);
                    if al != 0 {
                        if self.fixes.bullet_slot_carry {
                            // 格值只留形狀，槽號另存，高四位的道具判定不會被進位破壞。
                            let slot = self.ds.r8(a(0x1F7DB));
                            self.g2_slots[di.wrapping_sub(0x2C1C) as usize] = slot.wrapping_add(1);
                        } else {
                            al = al.wrapping_add(self.ds.r8(a(0x1F7DB)));
                        }
                        self.r.set_al(al);
                        self.ds.w8(di, al);
                    }
                }
                di = di.wrapping_add(1);
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
        self.r.dx = dx;
        self.r.bx = bx;
        self.r.cx = 0;
    }

    /// `sub_17898`：G2 的起點。`bx` = 列（>= 14h 改取 14h，該索引的位址 `1B32h` 是 G1 的前兩格，
    /// 列基底於是取到 G1 的內容，原版行為）。回傳 `ax` = 列基底（`2C1Ch` + 列偏移），
    /// `di` = 基底 + 欄 `word_20107`，`dx` = 基底 + 27h，`cx` = 形狀寬。`bx` 保留。
    pub(crate) fn s17898(&mut self) {
        let mut bx = self.r.bx;
        if bx >= 0x14 {
            bx = 0x14;
        }
        let ax = self.ds.r16((bx << 1).wrapping_add(0x1B0A)).wrapping_add(0x2C1C);
        self.r.ax = ax;
        self.r.di = ax.wrapping_add(self.ds.r16(a(0x20107)));
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.dx = ax.wrapping_add(0x27);
    }

    /// `sub_178BB`：清 G2（`2C1Ch` 起 190h 個字組）。
    /// `178CEh` 起的碼在 `retn` 之後無跳入，不移植。
    pub(crate) fn s178bb(&mut self) {
        self.ds.fill16(0x2C1C, 0x190, 0);
        self.g2_slots.fill(0);
        self.r.ax = 0;
        self.r.di = 0x2C1Cu16.wrapping_add(0x320);
        self.r.cx = 0;
    }

    /// 邊界反彈的共同流程（`sub_17955`、`1798E`、`179C7`、`17A00`）。
    /// w0 為 0 不反彈。`w1FE71` 不是 `Y` 不動。否則先改回 `N`，w0 等於 `f1` 改成 `t1`、等於 `f2` 改成 `t2`，
    /// 都不是就再設回 `Y`。
    fn eb_bounce(&mut self, f1: u16, t1: u16, f2: u16, t2: u16) {
        if self.ds.r16(a(0x200F5)) == 0 {
            return;
        }
        if self.ds.r16(a(0x1FE71)) != 0x59 {
            return;
        }
        self.ds.w16(a(0x1FE71), 0x4E);
        let w0 = self.ds.r16(a(0x200F5));
        if w0 == f1 {
            self.ds.w16(a(0x200F5), t1);
        } else if w0 == f2 {
            self.ds.w16(a(0x200F5), t2);
        } else {
            self.ds.w16(a(0x1FE71), 0x59);
        }
    }

    /// `sub_17955`：出上界。朝向 8 變 6，2 變 4。
    pub(crate) fn s17955(&mut self) {
        self.eb_bounce(8, 6, 2, 4);
    }

    /// `sub_1798E`：出下界。朝向 4 變 2，6 變 8。
    pub(crate) fn s1798e(&mut self) {
        self.eb_bounce(4, 2, 6, 8);
    }

    /// `sub_179C7`：出右界。朝向 2 變 8，4 變 6。
    pub(crate) fn s179c7(&mut self) {
        self.eb_bounce(2, 8, 4, 6);
    }

    /// `sub_17A00`：出左界。朝向 6 變 4，8 變 2。
    pub(crate) fn s17a00(&mut self) {
        self.eb_bounce(6, 4, 8, 2);
    }
}
