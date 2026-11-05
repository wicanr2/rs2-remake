//! 玩家輸入、移動、命中、砲座與繪製（含玩家 2），以及頭目段背景 `sub_15906`。
//! 逐函式移植自機器碼，查法見 `tools/q.sh`。規格：`docs/spec/001-rust-first-stage.md`。
//!
//! 玩家 1 與玩家 2 的函式一一平行，位址對照：
//! `14458`～`14D68` 對 `14FC5`～`15896`（玩家 1 的本體在 `1DAC5h`～`1DAC9h`，玩家 2 在 `1DB1Fh`～`1DB37h`）。
//! 兩邊都要照各自的機器碼寫，不共用，因為有些常數與次序不同（例如砲座像素偏移 `32h` 對 `14h`）。
//!
//! 暫存器照原版走 `self.r`。`mov ax, seg dseg` 與 `mov ds/es, ax` 不移植，
//! 其後 `ax` 的殘值在原版隨載入位址變動，沒有任何讀取端依賴它。
//! `push`/`pop` 配對用區域變數保存還原。

use crate::draw::Pool;
use crate::dseg::a;
use crate::fx::EffectKind;
use crate::game::Core;
use crate::pbullet::Facing;

impl Core {
    /// `loop`：`cx` 減 1，不為 0 則回傳 true（跳回）。
    fn pl_loop(&mut self) -> bool {
        self.r.cx = self.r.cx.wrapping_sub(1);
        self.r.cx != 0
    }

    /// `lodsw`：`ax` = `[si]`，`si` 加 2。
    fn pl_lodsw(&mut self) {
        self.r.ax = self.ds.r16(self.r.si);
        self.r.si = self.r.si.wrapping_add(2);
    }

    /// `add word ptr [linear], v`（`sub` 以 `wrapping_neg` 傳入）。
    fn pl_addw(&mut self, linear: u32, v: u16) {
        let n = self.ds.r16(a(linear)).wrapping_add(v);
        self.ds.w16(a(linear), n);
    }

    /// `or byte ptr [linear], v`。
    fn pl_or8(&mut self, linear: u32, v: u8) {
        let n = self.ds.r8(a(linear)) | v;
        self.ds.w8(a(linear), n);
    }

    /// 按鍵表的位元組等於 1（按下）。
    fn pl_k(&self, linear: u32) -> bool {
        self.ds.r8(a(linear)) == 1
    }

    // ---------------------------------------------------------------
    // 玩家 1
    // ---------------------------------------------------------------

    /// `sub_14458`：玩家 1 每幀入口：示範輸入、輸入與移動、錄製。`byte_1CFAD` 為 `F` 時略過。
    pub(crate) fn s14458(&mut self) {
        if self.ds.r8(a(0x1CFAD)) == 0x46 {
            return;
        }
        self.ds.w8(a(0x1DAA4), 0);
        self.s14497();
        self.s14501();
        self.s14474();
    }

    /// `sub_14474`：非示範時把本幀的輸入位元 `byte_1DAA4` 錄到 `dseg:0AAE2h` 加幀計數處。
    pub(crate) fn s14474(&mut self) {
        if self.ds.r8(a(0x1CFDC)) == 0x59 {
            return;
        }
        if self.ds.r16(a(0x1CFE2)) > 0x5DB {
            return;
        }
        self.r.si = 0xAAE2u16.wrapping_add(self.ds.r16(a(0x1CFE2)));
        self.r.set_al(self.ds.r8(a(0x1DAA4)));
        self.ds.w8(self.r.si, self.r.al());
    }

    /// `sub_14497`：示範時由 `dseg:0AAE2h[幀計數]` 還原玩家 1 的方向與開火旗標。
    pub(crate) fn s14497(&mut self) {
        if self.ds.r8(a(0x1CFDC)) == 0x4E {
            return;
        }
        self.ds.w8(a(0x274DB), 0);
        self.ds.w8(a(0x274D6), 0);
        self.ds.w8(a(0x274D8), 0);
        self.ds.w8(a(0x274D3), 0);
        self.ds.w8(a(0x274C1), 0);
        self.ds.w8(a(0x273A7), 0x59);
        self.ds.w8(a(0x273AA), 0x4E);
        self.r.si = 0xAAE2u16.wrapping_add(self.ds.r16(a(0x1CFE2)));
        self.r.set_al(self.ds.r8(self.r.si));
        let al = self.r.al();
        if al & 1 != 0 {
            self.ds.w8(a(0x274D3), 1);
        }
        if al & 2 != 0 {
            self.ds.w8(a(0x274DB), 1);
        }
        if al & 4 != 0 {
            self.ds.w8(a(0x274D8), 1);
        }
        if al & 8 != 0 {
            self.ds.w8(a(0x274D6), 1);
        }
        if al & 0x10 != 0 {
            self.ds.w8(a(0x273A9), 0x59);
        }
        if al & 0x20 != 0 {
            self.ds.w8(a(0x273AA), 0x59);
        }
    }

    /// `sub_14501`：玩家 1 的命中結算、方向鍵、轉向、充能與開火。
    pub(crate) fn s14501(&mut self) {
        if self.ds.r8(a(0x1CFDD)) & 1 == 0 {
            return;
        }
        self.ds.w16(a(0x1DABD), 9);
        self.ds.w8(a(0x1DAAB), 0x4E);
        // 14515：byte_1DAC0 = N 時略過受擊。
        if self.ds.r8(a(0x1DAC0)) != 0x4E {
            // byte ptr word_273A5 = Y（作弊）時不減血。
            if self.ds.r8(a(0x273A5)) != 0x59 {
                self.r.ax = self.ds.r16(a(0x1CFDE));
                self.r.ax >>= 1;
                self.ds.w16(a(0x1CFDE), self.r.ax);
            }
            self.s11336();
            self.ds.w8(a(0x1DAC1), 6);
            if self.ds.r16(a(0x1CFDE)) == 0 {
                self.s14679();
                return;
            }
        }
        // 1453F
        self.ds.w8(a(0x1DAC3), 0x0F);
        self.ds.w8(a(0x1DAC0), 0x4E);
        // 垂直：上（274D2、274D3、274D4）優先於下（274DA、274DB、274DC）。
        if self.pl_k(0x274D2) || self.pl_k(0x274D3) || self.pl_k(0x274D4) {
            self.s146ac();
            self.pl_or8(0x1DAA4, 1);
        } else if self.pl_k(0x274DA) || self.pl_k(0x274DB) || self.pl_k(0x274DC) {
            self.s146c3();
            self.pl_or8(0x1DAA4, 2);
        }
        // 水平：左（274DA、274D6、274D2）優先於右（274DC、274D8、274D4）。
        if self.pl_k(0x274DA) || self.pl_k(0x274D6) || self.pl_k(0x274D2) {
            self.s146f2();
            self.pl_or8(0x1DAA4, 8);
        } else if self.pl_k(0x274DC) || self.pl_k(0x274D8) || self.pl_k(0x274D4) {
            self.s146da();
            self.pl_or8(0x1DAA4, 4);
        }
        // 145D1：轉向鍵。
        if self.ds.r8(a(0x273AA)) == 0x59 {
            self.ds.w8(a(0x273AA), 0x4E);
            self.pl_or8(0x1DAA4, 0x20);
            if self.ds.r16(a(0x1DA8F)) != 0x3E {
                self.ds.w16(a(0x1DA8F), 0x3E);
            } else {
                self.ds.w16(a(0x1DA8F), 0x3C);
            }
        }
        // 145F8：右 Shift 不在按住（byte_274C1 != 1）：結算充能。
        if self.ds.r8(a(0x274C1)) != 1 {
            if self.ds.r16(a(0x1DA8B)) >= 0x0E {
                self.fire_sweep(self.ds.r16(a(0x1DACB)));
            }
            self.ds.w16(a(0x1DA8B), 0);
        }
        // 14621
        if self.ds.r8(a(0x273A9)) != 0x59 {
            self.ds.w8(a(0x1DAA5), 0x4E);
            return;
        }
        self.ds.w8(a(0x273A9), 0x4E);
        if self.ds.r8(a(0x273A7)) != 0x59 {
            // 按住不放：第二座砲座存在時才累計充能。
            if self.ds.r8(a(0x1DA8E)) != 0x4E {
                self.pl_addw(0x1DA8B, 1);
            }
            return;
        }
        // 14647：新按，主機射一發普通彈。
        self.ds.w16(a(0x1DA8B), 0);
        self.ds.w8(a(0x273A7), 0x4E);
        self.fire_player_bullets(0, Facing::Right, self.ds.r16(a(0x1DACB)), self.ds.r16(a(0x1DAC9)));
        self.pl_or8(0x1DAA4, 0x10);
        self.ds.w8(a(0x1DAA5), 0x59);
    }

    /// `sub_14679`：玩家 1 血量歸零後的死亡動畫，15 幀後清存活位元 0。
    pub(crate) fn s14679(&mut self) {
        let v = self.ds.r8(a(0x1DAC3)).wrapping_sub(1);
        self.ds.w8(a(0x1DAC3), v);
        if v == 0 {
            // 146A0
            let f = self.ds.r8(a(0x1CFDD)) & 0xF0;
            self.ds.w8(a(0x1CFDD), f);
            self.ds.w8(a(0x1DAC3), 6);
            return;
        }
        self.s146c3();
        self.s146da();
        let (x, y) = (self.ds.r16(a(0x1DACB)), self.ds.r16(a(0x1DAC9)));
        self.spawn_effect(EffectKind::PlayerDeath, x, y);
    }

    /// `sub_146AC`：玩家 1 上移，列大於 0（有號）才動，圖 0Bh。
    pub(crate) fn s146ac(&mut self) {
        if (self.ds.r16(a(0x1DAC9)) as i16) <= 0 {
            return;
        }
        self.pl_addw(0x1DAC9, 1u16.wrapping_neg());
        self.pl_addw(0x1DAC5, 0x0Au16.wrapping_neg());
        self.ds.w16(a(0x1DABD), 0x0B);
    }

    /// `sub_146C3`：玩家 1 下移，列小於 10h（無號）才動，圖 7。
    pub(crate) fn s146c3(&mut self) {
        if self.ds.r16(a(0x1DAC9)) >= 0x10 {
            return;
        }
        self.pl_addw(0x1DAC9, 1);
        self.pl_addw(0x1DAC5, 0x0A);
        self.ds.w16(a(0x1DABD), 7);
    }

    /// `sub_146DA`：玩家 1 右移，欄小於 21h（無號）才動，尾焰旗標 `Y`。
    pub(crate) fn s146da(&mut self) {
        if self.ds.r16(a(0x1DACB)) >= 0x21 {
            return;
        }
        self.pl_addw(0x1DACB, 2);
        self.pl_addw(0x1DAC7, 2);
        self.ds.w8(a(0x1DAAB), 0x59);
    }

    /// `sub_146F2`：玩家 1 左移，欄大於 1（有號）才動。
    pub(crate) fn s146f2(&mut self) {
        if (self.ds.r16(a(0x1DACB)) as i16) <= 1 {
            return;
        }
        self.pl_addw(0x1DACB, 2u16.wrapping_neg());
        self.pl_addw(0x1DAC7, 2u16.wrapping_neg());
    }

    /// `sub_14704`：玩家 1 的碰撞、繪製（本體、尾焰、發射閃光、撿取光柱、砲座）。
    pub(crate) fn s14704(&mut self) {
        if self.ds.r8(a(0x1CFDD)) & 1 == 0 {
            return;
        }
        self.s1480a();
        self.r.ax = self.ds.r16(a(0x1DAC7));
        self.ds.w16(a(0x26880), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DAC5));
        self.ds.w16(a(0x26882), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DABD));
        self.ds.w16(a(0x26888), self.r.ax);
        self.set_obj(Pool::Player, 0);
        self.s18d46();
        if self.ds.r8(a(0x1DAC1)) != 0 {
            // 14748：無敵倒數，只畫遮罩，跳過所有碰撞。
            let v = self.ds.r8(a(0x1DAC1)).wrapping_sub(1);
            self.ds.w8(a(0x1DAC1), v);
            if self.ds.r16(a(0x26872)) != 0 {
                self.s18d89();
            }
            self.clear_obj();
            return;
        }
        self.pl_addw(0x26882, 1);
        self.s18ce2();
        self.s147bc();
        self.s147e1();
        self.s1478b();
        if self.ds.r8(a(0x1DAAB)) != 0x59 {
            self.clear_obj();
            return;
        }
        // 14757：尾焰。
        self.ds.w16(a(0x26888), 0x0D);
        self.pl_addw(0x26882, 0x0Au16.wrapping_neg());
        if self.ds.r16(a(0x26872)) != 0 {
            self.ds.w16(a(0x26888), 0x0C);
        }
        self.r.ax = self.ds.r16(a(0x1DAC7)).wrapping_sub(4);
        self.ds.w16(a(0x26880), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DAC5)).wrapping_sub(2);
        self.ds.w16(a(0x26882), self.r.ax);
        self.s18d46();
        self.s18ce2();
        self.s14a59();
        self.clear_obj();
    }

    /// `sub_1478B`：本幀開火時在機頭畫發射閃光（圖 11h，位置加 3、3），x、y 還原。
    pub(crate) fn s1478b(&mut self) {
        if self.ds.r8(a(0x1DAA5)) != 0x59 {
            return;
        }
        let ax = self.ds.r16(a(0x26880));
        let bx = self.ds.r16(a(0x26882));
        self.ds.w16(a(0x26888), 0x11);
        self.pl_addw(0x26880, 3);
        self.pl_addw(0x26882, 3);
        self.s18d46();
        self.s18ce2();
        self.r.bx = bx;
        self.r.ax = ax;
        self.ds.w16(a(0x26880), ax);
        self.ds.w16(a(0x26882), bx);
    }

    /// `sub_147BC`：撿取光柱，`word_1DAA6` 為 y，大於 0（有號）即結束，每幀加 28h，圖 8。
    pub(crate) fn s147bc(&mut self) {
        if (self.ds.r16(a(0x1DAA6)) as i16) > 0 {
            return;
        }
        self.pl_addw(0x1DAA6, 0x28);
        self.r.ax = self.ds.r16(a(0x1DAC7));
        self.ds.w16(a(0x26880), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DAA6));
        self.ds.w16(a(0x26882), self.r.ax);
        self.ds.w16(a(0x26888), 8);
        self.s18d46();
        self.s18ce2();
    }

    /// `sub_147E1`：玩家 1 的碰撞：標髒格、對 G1、對 G4、對 G2，受擊時播特效 `S`。
    pub(crate) fn s147e1(&mut self) {
        self.s14a2c();
        self.s14b1e();
        self.s14a90();
        self.s14c85();
        if self.ds.r8(a(0x1DAC0)) == 0x4E {
            return;
        }
        let (x, y) = (self.ds.r16(a(0x1DACB)), self.ds.r16(a(0x1DAC9)));
        self.spawn_effect(EffectKind::SparkRight, x, y);
    }

    /// `sub_1480A`：玩家 1 的砲座：位置、碰撞、繪製、發射。砲座編號 0、1。
    pub(crate) fn s1480a(&mut self) {
        if self.ds.r8(a(0x1DA8D)) != 0x59 {
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DA98));
        self.ds.w16(a(0x1DA9C), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DA9A));
        self.ds.w16(a(0x1DA9E), self.r.ax);
        self.s14b63();
        self.s14bae();
        self.r.ax = self.ds.r16(a(0x1DAA0));
        self.ds.w16(a(0x26880), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DAA2)).wrapping_add(5);
        self.ds.w16(a(0x26882), self.r.ax);
        let y0 = self.r.ax; // push ax
        self.r.ax = self.ds.r16(a(0x1DA94));
        self.ds.w16(a(0x26888), self.r.ax);
        self.set_obj(Pool::Pod, 0);
        self.s18d46();
        self.s18ce2();
        if self.ds.r8(a(0x1DA8E)) != 0x4E {
            self.pl_addw(0x26882, 0x32);
            self.set_obj(Pool::Pod, 1);
            self.s18d46();
            self.s18ce2();
            self.r.ax = self.ds.r16(a(0x1DA98));
            self.ds.w16(a(0x1DA9C), self.r.ax);
            self.r.ax = self.ds.r16(a(0x1DA9A)).wrapping_add(5);
            self.ds.w16(a(0x1DA9E), self.r.ax);
            self.s14b63();
            self.s14bae();
        }
        self.clear_obj();
        // 14867
        self.r.ax = y0; // pop ax
        self.ds.w16(a(0x26882), self.r.ax);
        self.s14908();
        if self.ds.r8(a(0x1DAA5)) != 0x59 {
            return;
        }
        // 14876：砲座開火。
        let facing = Facing::from_word(self.ds.r16(a(0x1DA8F)));
        let (col, row, weapon) = (self.ds.r16(a(0x1DA98)), self.ds.r16(a(0x1DA9A)), self.ds.r16(a(0x1DAA9)));
        self.fire_player_bullets(weapon, facing, col, row);
        if self.ds.r8(a(0x1DA8E)) != 0x4E {
            self.fire_player_bullets(weapon, facing, col, row.wrapping_add(5));
        }
        // 148B0：噴焰（圖 0Ah），朝左 148B5，朝右 148DF。
        if self.ds.r16(a(0x1DA8F)) != 0x3E {
            self.pl_addw(0x26880, 1u16.wrapping_neg());
            self.pl_addw(0x26882, 2);
        } else {
            self.pl_addw(0x26880, 2);
            self.pl_addw(0x26882, 2);
        }
        self.ds.w16(a(0x26888), 0x0A);
        self.set_obj(Pool::Pod, 0);
        self.s18d46();
        self.s18ce2();
        if self.ds.r8(a(0x1DA8E)) != 0x4E {
            self.pl_addw(0x26882, 0x32);
            self.set_obj(Pool::Pod, 1);
            self.s18d46();
            self.s18ce2();
        }
        self.clear_obj();
    }

    /// `sub_14908`：砲座位置與圖框。充能小於 6 用圖 1Eh 或 24h（5 格循環），否則交給 `sub_1496D`。
    pub(crate) fn s14908(&mut self) {
        if self.ds.r16(a(0x1DA8B)) >= 6 {
            self.s1496d();
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DAC5)).wrapping_sub(0x1E);
        self.ds.w16(a(0x1DAA2), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DAC7)).wrapping_add(3);
        self.ds.w16(a(0x1DAA0), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DAC9)).wrapping_sub(3);
        self.ds.w16(a(0x1DA9A), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DACB)).wrapping_add(3);
        self.ds.w16(a(0x1DA98), self.r.ax);
        self.pl_addw(0x1DA96, 1);
        if self.ds.r16(a(0x1DA96)) >= 5 {
            self.ds.w16(a(0x1DA96), 0);
        }
        if self.ds.r16(a(0x1DA8F)) == 0x3E {
            // 14963
            self.r.ax = self.ds.r16(a(0x1DA96)).wrapping_add(0x1E);
            self.ds.w16(a(0x1DA94), self.r.ax);
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DA96)).wrapping_add(0x24);
        self.ds.w16(a(0x1DA94), self.r.ax);
        self.pl_addw(0x1DA98, 5u16.wrapping_neg());
        self.pl_addw(0x1DAA0, 5u16.wrapping_neg());
    }

    /// `sub_1496D`：充能 6 至 13 的砲座圖框（31h 起 3 格循環），14 以上交給 `sub_149D2`。
    pub(crate) fn s1496d(&mut self) {
        if self.ds.r16(a(0x1DA8B)) >= 0x0E {
            self.s149d2();
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DAC5)).wrapping_sub(0x1E);
        self.ds.w16(a(0x1DAA2), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DAC7)).wrapping_add(3);
        self.ds.w16(a(0x1DAA0), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DAC9)).wrapping_sub(3);
        self.ds.w16(a(0x1DA9A), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DACB)).wrapping_add(3);
        self.ds.w16(a(0x1DA98), self.r.ax);
        self.pl_addw(0x1DA96, 1);
        if self.ds.r16(a(0x1DA96)) >= 3 {
            self.ds.w16(a(0x1DA96), 0);
        }
        if self.ds.r16(a(0x1DA8F)) == 0x3E {
            // 149C8
            self.r.ax = self.ds.r16(a(0x1DA96)).wrapping_add(0x31);
            self.ds.w16(a(0x1DA94), self.r.ax);
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DA96)).wrapping_add(0x31);
        self.ds.w16(a(0x1DA94), self.r.ax);
        self.pl_addw(0x1DA98, 5u16.wrapping_neg());
        self.pl_addw(0x1DAA0, 5u16.wrapping_neg());
    }

    /// `sub_149D2`：充能 14 以上的砲座圖框（34h 起 3 格循環）。
    pub(crate) fn s149d2(&mut self) {
        self.r.ax = self.ds.r16(a(0x1DAC5)).wrapping_sub(0x1E);
        self.ds.w16(a(0x1DAA2), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DAC7)).wrapping_add(3);
        self.ds.w16(a(0x1DAA0), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DAC9)).wrapping_sub(3);
        self.ds.w16(a(0x1DA9A), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DACB)).wrapping_add(3);
        self.ds.w16(a(0x1DA98), self.r.ax);
        self.pl_addw(0x1DA96, 1);
        if self.ds.r16(a(0x1DA96)) >= 3 {
            self.ds.w16(a(0x1DA96), 0);
        }
        if self.ds.r16(a(0x1DA8F)) == 0x3E {
            // 14A22
            self.r.ax = self.ds.r16(a(0x1DA96)).wrapping_add(0x34);
            self.ds.w16(a(0x1DA94), self.r.ax);
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DA96)).wrapping_add(0x34);
        self.ds.w16(a(0x1DA94), self.r.ax);
        self.pl_addw(0x1DA98, 5u16.wrapping_neg());
        self.pl_addw(0x1DAA0, 5u16.wrapping_neg());
    }

    /// `sub_14A2C`：在 G3（`dseg:964Ch` 起）以玩家 1 的位置標 2 列 4 欄的髒格（值 1）。
    pub(crate) fn s14a2c(&mut self) {
        self.r.ax = self.ds.r16(a(0x1DAC9));
        self.r.bx = 0x28;
        // mul bl：ax = al * bl。
        self.r.ax = (self.r.al() as u16) * (self.r.bl() as u16);
        self.r.ax = self.r.ax.wrapping_add(self.ds.r16(a(0x1DACB)));
        self.r.ax = self.r.ax.wrapping_add(1);
        self.r.ax <<= 1;
        self.r.bx = self.r.ax;
        self.r.cx = 2;
        self.r.ax = 1;
        loop {
            let outer = self.r.cx;
            self.r.cx = 4;
            loop {
                // [bx-69B4h]
                self.ds.w16(self.r.bx.wrapping_sub(0x69B4), self.r.ax);
                self.r.bx = self.r.bx.wrapping_add(2);
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.cx = outer;
            self.r.bx = self.r.bx.wrapping_add(0x48);
            if !self.pl_loop() {
                break;
            }
        }
    }

    /// `sub_14A59`：把玩家 1 的髒格形狀（`dseg:0F1Eh`）以 OR 疊進 G3（`964Ch`～`9C3Ch`）。
    pub(crate) fn s14a59(&mut self) {
        self.r.si = 0x0F1E;
        self.pl_lodsw();
        self.ds.w16(a(0x2687E), self.r.ax);
        self.pl_lodsw();
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1DAC9)).wrapping_sub(1);
        loop {
            let outer = self.r.cx;
            self.s14ad7();
            loop {
                if !(self.r.di > 0x9C3C || self.r.di < 0x964C) {
                    self.r.set_al(self.ds.r8(self.r.si));
                    let v = self.ds.r8(self.r.di) | self.r.al();
                    self.ds.w8(self.r.di, v);
                }
                self.r.di = self.r.di.wrapping_add(2);
                self.r.si = self.r.si.wrapping_add(1);
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = outer;
            if !self.pl_loop() {
                break;
            }
        }
    }

    /// `sub_14A90`：玩家 1 對 G4 地形的碰撞，只在第三關（`byte_1CFDB` = `C`）。
    pub(crate) fn s14a90(&mut self) {
        if self.ds.r8(a(0x1CFDB)) != 0x43 {
            return;
        }
        self.r.si = 0x0F1E;
        self.pl_lodsw();
        self.ds.w16(a(0x2687E), self.r.ax);
        self.pl_lodsw();
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1DAC9));
        self.r.cx = 1;
        loop {
            let outer = self.r.cx;
            self.s14afc();
            loop {
                if !(self.r.di > 0x95FC || self.r.di < 0x900C) {
                    self.r.ax = self.ds.r16(self.r.di);
                    if self.r.ax != 0 {
                        self.ds.w8(a(0x1DAC0), 0x59);
                    }
                }
                self.r.di = self.r.di.wrapping_add(2);
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = outer;
            if !self.pl_loop() {
                break;
            }
        }
    }

    /// `sub_14AD7`：G3 的列位址。`di` = `964Ch` + 列表（`21C2h`）+ (欄 - 4) * 2 + 1，`cx` = 寬。`bx` 保留。
    pub(crate) fn s14ad7(&mut self) {
        let saved = self.r.bx;
        if self.r.bx >= 0x14 {
            self.r.bx = 0x14;
        }
        self.r.bx <<= 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x21C2));
        self.r.ax = self.r.ax.wrapping_add(0x964C);
        self.r.di = self.r.ax;
        self.r.ax = self.ds.r16(a(0x1DACB));
        self.r.ax = self.r.ax.wrapping_sub(4);
        self.r.ax <<= 1;
        self.r.ax = self.r.ax.wrapping_add(1);
        self.r.di = self.r.di.wrapping_add(self.r.ax);
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.bx = saved;
    }

    /// `sub_14AFC`：G4 的列位址。`di` = `900Ch` + 列表（`21C2h`）+ 欄 * 2 + 1，`cx` = 寬。`bx` 保留。
    pub(crate) fn s14afc(&mut self) {
        let saved = self.r.bx;
        if self.r.bx >= 0x14 {
            self.r.bx = 0x14;
        }
        self.r.bx <<= 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x21C2));
        self.r.ax = self.r.ax.wrapping_add(0x900C);
        self.r.di = self.r.ax;
        self.r.ax = self.ds.r16(a(0x1DACB));
        self.r.ax <<= 1;
        self.r.ax = self.r.ax.wrapping_add(1);
        self.r.di = self.r.di.wrapping_add(self.r.ax);
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.bx = saved;
    }

    /// `sub_14B1E`：玩家 1 本體（形狀 `0F0Dh`）對 G1：任何非 0 且非 FFh 的格設 `byte_1DAC0` = `Y`。
    pub(crate) fn s14b1e(&mut self) {
        self.r.si = 0x0F0D;
        self.pl_lodsw();
        self.ds.w16(a(0x2687E), self.r.ax);
        self.pl_lodsw();
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1DAC9));
        loop {
            let outer = self.r.cx;
            self.s14c61();
            loop {
                let saved_ax = self.r.ax;
                if !(self.r.di >= 0x217C
                    || self.r.di <= 0x1B32
                    || self.r.di >= self.r.dx
                    || self.r.di <= self.r.ax)
                {
                    self.r.set_al(self.ds.r8(self.r.di));
                    if self.r.al() != 0 && self.r.al() != 0xFF {
                        self.ds.w8(a(0x1DAC0), 0x59);
                    }
                }
                self.r.di = self.r.di.wrapping_add(1);
                self.r.ax = saved_ax;
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = outer;
            if !self.pl_loop() {
                break;
            }
        }
    }

    /// `sub_14B63`：玩家 1 砲座（形狀 `0F16h`）對 G1，記錄最後掃到的格值於 `byte_1DA8A`，再呼叫 `sub_14C18`。
    pub(crate) fn s14b63(&mut self) {
        self.ds.w8(a(0x1DA8A), 0);
        self.r.si = 0x0F16;
        self.pl_lodsw();
        self.ds.w16(a(0x2687E), self.r.ax);
        self.pl_lodsw();
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1DA9E));
        loop {
            let outer = self.r.cx;
            self.s14c3d();
            loop {
                let saved_ax = self.r.ax;
                if !(self.r.di >= 0x217C
                    || self.r.di <= 0x1B32
                    || self.r.di >= self.r.dx
                    || self.r.di <= self.r.ax)
                {
                    self.r.set_al(self.ds.r8(self.r.di));
                    if self.r.al() != 0 && self.r.al() != 0xFF {
                        self.ds.w8(a(0x1DA8A), self.r.al());
                    }
                }
                self.r.di = self.r.di.wrapping_add(1);
                self.r.ax = saved_ax;
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = outer;
            if !self.pl_loop() {
                break;
            }
        }
        self.s14c18();
    }

    /// `sub_14BAE`：玩家 1 砲座（用本體形狀 `0F0Dh`）對 G2：格值高三位含 E0h 位元者略過，其餘刪除該敵彈。
    /// `sub_14D54` 會改寫 `bx`，之後的列計數沿用殘值（原版行為）。
    pub(crate) fn s14bae(&mut self) {
        self.r.si = 0x0F0D;
        self.pl_lodsw();
        self.ds.w16(a(0x2687E), self.r.ax);
        self.pl_lodsw();
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1DA9E));
        loop {
            let outer = self.r.cx;
            self.s14bf4();
            loop {
                let (saved_cx, saved_ax) = (self.r.cx, self.r.ax);
                if !(self.r.di >= 0x3266
                    || self.r.di <= 0x2C1C
                    || self.r.di >= self.r.dx
                    || self.r.di <= self.r.ax)
                {
                    self.r.set_al(self.ds.r8(self.r.di));
                    if self.r.al() != 0 && self.r.al() & 0xE0 == 0 {
                        if let Some(bx) = self.remove_hit_bullet(self.r.al(), self.r.di) {
                            self.r.bx = bx;
                        }
                    }
                }
                self.r.di = self.r.di.wrapping_add(1);
                self.r.ax = saved_ax;
                self.r.cx = saved_cx;
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = outer;
            if !self.pl_loop() {
                break;
            }
        }
    }

    /// `sub_14BF4`：G2 的列位址（玩家 1 砲座）。`di` = `2C1Ch` + 列表（`1B0Ah`，`bx` 不小於 17Ch 取 1F4h）+ `word_1DA9C`，
    /// `dx` = 列基底 + 45h，`ax` = 列基底，`cx` = 寬。`bx` 保留。
    pub(crate) fn s14bf4(&mut self) {
        let saved = self.r.bx;
        if self.r.bx >= 0x17C {
            self.r.bx = 0x1F4;
        }
        self.r.bx <<= 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x1B0A));
        self.r.ax = self.r.ax.wrapping_add(0x2C1C);
        self.r.dx = self.r.ax;
        self.r.di = self.r.ax;
        self.r.di = self.r.di.wrapping_add(self.ds.r16(a(0x1DA9C)));
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.dx = self.r.dx.wrapping_add(0x45);
        self.r.bx = saved;
    }

    /// `sub_14C18`：砲座碰到的敵人（`byte_1DA8A` 為槽號加 1）：w16 的低位元組加 1，高位元組設 1。
    /// 裝甲格 FEh 沒有檢查，會取到指標 0 而把字組寫到 `dseg:0020h`（`11` 第 5.3 節）。
    /// 敵人池指標表與傷害字組經 `entity_link`、`add_entity_damage` 存取（`entity.rs`）。
    pub(crate) fn s14c18(&mut self) {
        self.r.set_al(self.ds.r8(a(0x1DA8A)));
        if self.r.al() == 0xFF {
            return;
        }
        if self.r.al() == 0 {
            return;
        }
        self.r.set_al(self.r.al().wrapping_sub(1));
        self.r.set_ah(0);
        self.r.bx = self.r.ax;
        self.r.bx <<= 1;
        self.r.ax = self.entity_link(self.r.al());
        // armor_cell_index：裝甲格 FEh 與已釋放的槽不寫傷害。
        if self.fixes.armor_cell_index && (self.r.bx == 0x1FA || self.r.ax == 0) {
            return;
        }
        let link = self.r.ax;
        self.r.si = link.wrapping_add(0x20);
        self.r.ax = self.add_entity_damage(link, 1);
    }

    /// `sub_14C3D`：G1 的列位址（玩家 1 砲座）。`di` = `1B32h` + 列表 + `word_1DA9C`，`dx` = 列基底 + 45h。
    pub(crate) fn s14c3d(&mut self) {
        let saved = self.r.bx;
        if self.r.bx >= 0x17C {
            self.r.bx = 0x1F4;
        }
        self.r.bx <<= 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x1B0A));
        self.r.ax = self.r.ax.wrapping_add(0x1B32);
        self.r.dx = self.r.ax;
        self.r.di = self.r.ax;
        self.r.di = self.r.di.wrapping_add(self.ds.r16(a(0x1DA9C)));
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.dx = self.r.dx.wrapping_add(0x45);
        self.r.bx = saved;
    }

    /// `sub_14C61`：G1 的列位址（玩家 1 本體）。`di` = `1B32h` + 列表 + `word_1DACB`，`dx` = 列基底 + 45h。
    pub(crate) fn s14c61(&mut self) {
        let saved = self.r.bx;
        if self.r.bx >= 0x17C {
            self.r.bx = 0x1F4;
        }
        self.r.bx <<= 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x1B0A));
        self.r.ax = self.r.ax.wrapping_add(0x1B32);
        self.r.dx = self.r.ax;
        self.r.di = self.r.ax;
        self.r.di = self.r.di.wrapping_add(self.ds.r16(a(0x1DACB)));
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.dx = self.r.dx.wrapping_add(0x45);
        self.r.bx = saved;
    }

    /// `sub_14C85`：玩家 1 本體（形狀 `0F0Dh`，列加 1）對 G2：非 0 的格刪除該敵彈，再判定道具或傷害。
    /// `sub_14D54` 改寫 `bx`，之後的列計數沿用殘值（原版行為）。
    pub(crate) fn s14c85(&mut self) {
        self.ds.w8(a(0x1DAA8), 0x4E);
        self.r.si = 0x0F0D;
        self.pl_lodsw();
        self.ds.w16(a(0x2687E), self.r.ax);
        self.pl_lodsw();
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1DAC9));
        self.r.bx = self.r.bx.wrapping_add(1);
        loop {
            let outer = self.r.cx;
            self.s14d68();
            loop {
                let (saved_cx, saved_ax) = (self.r.cx, self.r.ax);
                if !(self.r.di >= 0x3266
                    || self.r.di <= 0x2C1C
                    || self.r.di >= self.r.dx
                    || self.r.di <= self.r.ax)
                {
                    self.r.set_al(self.ds.r8(self.r.di));
                    if self.r.al() != 0 {
                        if let Some(bx) = self.remove_hit_bullet(self.r.al(), self.r.di) {
                            self.r.bx = bx;
                        }
                        self.s14cd4();
                    }
                }
                self.r.di = self.r.di.wrapping_add(1);
                self.r.ax = saved_ax;
                self.r.cx = saved_cx;
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = outer;
            if !self.pl_loop() {
                break;
            }
        }
        self.s14d04();
    }

    /// `sub_14CD4`：以格值高四位判定。80h、40h、C0h、20h 是道具，記到 `byte_1DAA8` 並設光柱 `word_1DAA6` = FF42h，其餘設 `byte_1DAC0` = `Y`。
    pub(crate) fn s14cd4(&mut self) {
        self.r.set_ah(self.r.al());
        self.r.set_ah(self.r.ah() & 0xF0);
        let ah = self.r.ah();
        if ah == 0x80 || ah == 0x40 || ah == 0xC0 || ah == 0x20 {
            self.ds.w8(a(0x1DAA8), ah);
            self.ds.w16(a(0x1DAA6), 0xFF42);
            return;
        }
        self.ds.w8(a(0x1DAC0), 0x59);
    }

    /// `sub_14D04`：依撿到的道具升級。先啟用砲座 1、砲座 2，之後依 80h、40h、C0h、20h 換武器 `L`、`B`、`K`、`S`。
    pub(crate) fn s14d04(&mut self) {
        self.r.set_al(self.ds.r8(a(0x1DAA8)));
        let al = self.r.al();
        if al == 0x4E {
            return;
        }
        if self.ds.r8(a(0x1DA8D)) == 0x4E {
            self.ds.w8(a(0x1DA8D), 0x59);
            return;
        }
        if self.ds.r8(a(0x1DA8E)) == 0x4E {
            self.ds.w8(a(0x1DA8E), 0x59);
            return;
        }
        if al == 0x80 {
            self.ds.w16(a(0x1DAA9), 0x4C);
        } else if al == 0x40 {
            self.ds.w16(a(0x1DAA9), 0x42);
        } else if al == 0xC0 {
            self.ds.w16(a(0x1DAA9), 0x4B);
        } else if al == 0x20 {
            self.ds.w16(a(0x1DAA9), 0x53);
        }
    }

    /// `sub_14D68`：G2 的列位址（玩家 1 本體）。`di` = `2C1Ch` + 列表 + `word_1DACB`，`dx` = 列基底 + 45h。
    pub(crate) fn s14d68(&mut self) {
        let saved = self.r.bx;
        if self.r.bx >= 0x17C {
            self.r.bx = 0x1F4;
        }
        self.r.bx <<= 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x1B0A));
        self.r.ax = self.r.ax.wrapping_add(0x2C1C);
        self.r.dx = self.r.ax;
        self.r.di = self.r.ax;
        self.r.di = self.r.di.wrapping_add(self.ds.r16(a(0x1DACB)));
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.dx = self.r.dx.wrapping_add(0x45);
        self.r.bx = saved;
    }

    /// `sub_14F47`：每關開始的玩家 1 位置（欄 5、列 10）。第一關或示範時另重設武器、血量、砲座與朝向。
    pub(crate) fn s14f47(&mut self) {
        self.ds.w16(a(0x1DACB), 5);
        self.ds.w16(a(0x1DAC9), 0x0A);
        self.r.ax = self.ds.r16(a(0x1DACB));
        self.ds.w16(a(0x1DAC7), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DAC9));
        self.r.bx = 0x0A;
        self.r.ax = (self.r.al() as u16) * (self.r.bl() as u16);
        self.ds.w16(a(0x1DAC5), self.r.ax);
        if self.ds.r8(a(0x1CFDC)) != 0x59 && self.ds.r8(a(0x1CFDB)) != 0x41 {
            return;
        }
        // 14F73
        self.ds.w16(a(0x1DAA9), 0);
        self.ds.w16(a(0x1CFDE), 0xFF);
        self.ds.w8(a(0x1DA8D), 0x4E);
        self.ds.w8(a(0x1DA8E), 0x4E);
        self.ds.w16(a(0x1DA8F), 0x3C);
    }

    // ---------------------------------------------------------------
    // 玩家 2
    // ---------------------------------------------------------------

    /// `sub_14F92`：玩家 2 每幀入口。`byte_1CFAD` 為 `F` 時略過。
    pub(crate) fn s14f92(&mut self) {
        if self.ds.r8(a(0x1CFAD)) == 0x46 {
            return;
        }
        self.ds.w8(a(0x1DB06), 0);
        self.s14fc5();
        self.s1502f();
        self.s14fae();
    }

    /// `sub_14FAE`：玩家 2 沒有錄製碼，只剩條件比較與段暫存器載入，沒有任何寫入。
    pub(crate) fn s14fae(&mut self) {}

    /// `sub_14FC5`：示範時由 `dseg:0AAE3h[幀計數]` 還原玩家 2 的方向與開火旗標。
    pub(crate) fn s14fc5(&mut self) {
        if self.ds.r8(a(0x1CFDC)) == 0x4E {
            return;
        }
        self.ds.w8(a(0x274BB), 0);
        self.ds.w8(a(0x274AC), 0);
        self.ds.w8(a(0x274AE), 0);
        self.ds.w8(a(0x2749F), 0);
        self.ds.w8(a(0x274B5), 0);
        self.ds.w8(a(0x273A8), 0x59);
        self.ds.w8(a(0x273AC), 0x59);
        self.r.si = 0xAAE3u16.wrapping_add(self.ds.r16(a(0x1CFE2)));
        self.r.set_al(self.ds.r8(self.r.si));
        let al = self.r.al();
        if al & 1 != 0 {
            self.ds.w8(a(0x2749F), 1);
        }
        if al & 2 != 0 {
            self.ds.w8(a(0x274BB), 1);
        }
        if al & 4 != 0 {
            self.ds.w8(a(0x274AE), 1);
        }
        if al & 8 != 0 {
            self.ds.w8(a(0x274AC), 1);
        }
        if al & 0x10 != 0 {
            self.ds.w8(a(0x273AB), 0x59);
        }
        if al & 0x20 != 0 {
            self.ds.w8(a(0x273AC), 0x59);
        }
    }

    /// `sub_1502F`：玩家 2 的命中結算、方向鍵、轉向、充能與開火。
    pub(crate) fn s1502f(&mut self) {
        if self.ds.r8(a(0x1CFDD)) & 0x10 == 0 {
            return;
        }
        self.ds.w16(a(0x1DB17), 0x10);
        self.ds.w8(a(0x1DB0D), 0x4E);
        if self.ds.r8(a(0x1DB1A)) != 0x4E {
            if self.ds.r8(a(0x273A5)) != 0x59 {
                self.r.ax = self.ds.r16(a(0x1CFE0));
                self.r.ax >>= 1;
                self.ds.w16(a(0x1CFE0), self.r.ax);
            }
            self.s1130d();
            self.ds.w8(a(0x1DB1B), 6);
            if self.ds.r16(a(0x1CFE0)) == 0 {
                self.s151a7();
                return;
            }
        }
        // 1506D
        self.ds.w8(a(0x1DB1D), 0x0F);
        self.ds.w8(a(0x1DB1A), 0x4E);
        // 垂直：上（2749E、2749F、274A0）優先於下（274BA、274BB、274BC）。
        if self.pl_k(0x2749E) || self.pl_k(0x2749F) || self.pl_k(0x274A0) {
            self.s151da();
            self.pl_or8(0x1DB06, 1);
        } else if self.pl_k(0x274BA) || self.pl_k(0x274BB) || self.pl_k(0x274BC) {
            self.s151f1();
            self.pl_or8(0x1DB06, 2);
        }
        // 水平：左（274BA、274AC、2749E）優先於右（274BC、274AE、274A0）。
        if self.pl_k(0x274BA) || self.pl_k(0x274AC) || self.pl_k(0x2749E) {
            self.s15220();
            self.pl_or8(0x1DB06, 8);
        } else if self.pl_k(0x274BC) || self.pl_k(0x274AE) || self.pl_k(0x274A0) {
            self.s15208();
            self.pl_or8(0x1DB06, 4);
        }
        // 150FF：轉向鍵。
        if self.ds.r8(a(0x273AC)) == 0x59 {
            self.ds.w8(a(0x273AC), 0x4E);
            self.pl_or8(0x1DB06, 0x20);
            if self.ds.r16(a(0x1DAF1)) != 0x3E {
                self.ds.w16(a(0x1DAF1), 0x3E);
            } else {
                self.ds.w16(a(0x1DAF1), 0x3C);
            }
        }
        // 15126：左 Shift 不在按住（byte_274B5 != 1）：結算充能。
        if self.ds.r8(a(0x274B5)) != 1 {
            if self.ds.r16(a(0x1DAED)) >= 0x0E {
                self.fire_sweep(self.ds.r16(a(0x1DB39)));
            }
            self.ds.w16(a(0x1DAED), 0);
        }
        // 1514F
        if self.ds.r8(a(0x273AB)) != 0x59 {
            self.ds.w8(a(0x1DB07), 0x4E);
            return;
        }
        self.ds.w8(a(0x273AB), 0x4E);
        if self.ds.r8(a(0x273A8)) != 0x59 {
            if self.ds.r8(a(0x1DAF0)) != 0x4E {
                self.pl_addw(0x1DAED, 1);
            }
            return;
        }
        // 15175：新按，主機射一發普通彈。
        self.ds.w16(a(0x1DAED), 0);
        self.ds.w8(a(0x273A8), 0x4E);
        self.fire_player_bullets(0, Facing::Right, self.ds.r16(a(0x1DB39)), self.ds.r16(a(0x1DB37)));
        self.pl_or8(0x1DB06, 0x10);
        self.ds.w8(a(0x1DB07), 0x59);
    }

    /// `sub_151A7`：玩家 2 血量歸零後的死亡動畫，15 幀後清存活位元 4。
    pub(crate) fn s151a7(&mut self) {
        let v = self.ds.r8(a(0x1DB1D)).wrapping_sub(1);
        self.ds.w8(a(0x1DB1D), v);
        if v == 0 {
            // 151CE
            let f = self.ds.r8(a(0x1CFDD)) & 0x0F;
            self.ds.w8(a(0x1CFDD), f);
            self.ds.w8(a(0x1DB1D), 6);
            return;
        }
        self.s151f1();
        self.s15208();
        let (x, y) = (self.ds.r16(a(0x1DB39)), self.ds.r16(a(0x1DB37)));
        self.spawn_effect(EffectKind::PlayerDeath, x, y);
    }

    /// `sub_151DA`：玩家 2 上移，圖 0Eh。
    pub(crate) fn s151da(&mut self) {
        if (self.ds.r16(a(0x1DB37)) as i16) <= 0 {
            return;
        }
        self.pl_addw(0x1DB37, 1u16.wrapping_neg());
        self.pl_addw(0x1DB1F, 0x0Au16.wrapping_neg());
        self.ds.w16(a(0x1DB17), 0x0E);
    }

    /// `sub_151F1`：玩家 2 下移，圖 12h。
    pub(crate) fn s151f1(&mut self) {
        if self.ds.r16(a(0x1DB37)) >= 0x10 {
            return;
        }
        self.pl_addw(0x1DB37, 1);
        self.pl_addw(0x1DB1F, 0x0A);
        self.ds.w16(a(0x1DB17), 0x12);
    }

    /// `sub_15208`：玩家 2 右移，尾焰旗標 `Y`。
    pub(crate) fn s15208(&mut self) {
        if self.ds.r16(a(0x1DB39)) >= 0x21 {
            return;
        }
        self.pl_addw(0x1DB39, 2);
        self.pl_addw(0x1DB21, 2);
        self.ds.w8(a(0x1DB0D), 0x59);
    }

    /// `sub_15220`：玩家 2 左移。
    pub(crate) fn s15220(&mut self) {
        if (self.ds.r16(a(0x1DB39)) as i16) <= 1 {
            return;
        }
        self.pl_addw(0x1DB39, 2u16.wrapping_neg());
        self.pl_addw(0x1DB21, 2u16.wrapping_neg());
    }

    /// `sub_15232`：玩家 2 的碰撞與繪製。玩家編號 1。
    pub(crate) fn s15232(&mut self) {
        if self.ds.r8(a(0x1CFDD)) & 0x10 == 0 {
            return;
        }
        self.s15338();
        self.r.ax = self.ds.r16(a(0x1DB21));
        self.ds.w16(a(0x26880), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB1F));
        self.ds.w16(a(0x26882), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB17));
        self.ds.w16(a(0x26888), self.r.ax);
        self.set_obj(Pool::Player, 1);
        self.s18d46();
        if self.ds.r8(a(0x1DB1B)) != 0 {
            // 15276
            let v = self.ds.r8(a(0x1DB1B)).wrapping_sub(1);
            self.ds.w8(a(0x1DB1B), v);
            if self.ds.r16(a(0x26872)) != 0 {
                self.s18d89();
            }
            self.clear_obj();
            return;
        }
        self.pl_addw(0x26882, 1);
        self.s18ce2();
        self.s152ea();
        self.s1530f();
        self.s152b9();
        if self.ds.r8(a(0x1DB0D)) != 0x59 {
            self.clear_obj();
            return;
        }
        // 15285：尾焰。
        self.ds.w16(a(0x26888), 0x0D);
        self.pl_addw(0x26882, 0x0Au16.wrapping_neg());
        if self.ds.r16(a(0x26872)) != 0 {
            self.ds.w16(a(0x26888), 0x0C);
        }
        self.r.ax = self.ds.r16(a(0x1DB21)).wrapping_sub(4);
        self.ds.w16(a(0x26880), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB1F)).wrapping_sub(2);
        self.ds.w16(a(0x26882), self.r.ax);
        self.s18d46();
        self.s18ce2();
        self.s15587();
        self.clear_obj();
    }

    /// `sub_152B9`：玩家 2 本幀開火時的發射閃光（圖 0Ah，位置加 3、6），x、y 還原。
    pub(crate) fn s152b9(&mut self) {
        if self.ds.r8(a(0x1DB07)) != 0x59 {
            return;
        }
        let ax = self.ds.r16(a(0x26880));
        let bx = self.ds.r16(a(0x26882));
        self.ds.w16(a(0x26888), 0x0A);
        self.pl_addw(0x26880, 3);
        self.pl_addw(0x26882, 6);
        self.s18d46();
        self.s18ce2();
        self.r.bx = bx;
        self.r.ax = ax;
        self.ds.w16(a(0x26880), ax);
        self.ds.w16(a(0x26882), bx);
    }

    /// `sub_152EA`：玩家 2 的撿取光柱，`word_1DB08` 為 y。
    pub(crate) fn s152ea(&mut self) {
        if (self.ds.r16(a(0x1DB08)) as i16) > 0 {
            return;
        }
        self.pl_addw(0x1DB08, 0x28);
        self.r.ax = self.ds.r16(a(0x1DB21));
        self.ds.w16(a(0x26880), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB08));
        self.ds.w16(a(0x26882), self.r.ax);
        self.ds.w16(a(0x26888), 8);
        self.s18d46();
        self.s18ce2();
    }

    /// `sub_1530F`：玩家 2 的碰撞，受擊時播特效 `S`。
    pub(crate) fn s1530f(&mut self) {
        self.s1555a();
        self.s1564c();
        self.s155be();
        self.s157b3();
        if self.ds.r8(a(0x1DB1A)) == 0x4E {
            return;
        }
        let (x, y) = (self.ds.r16(a(0x1DB39)), self.ds.r16(a(0x1DB37)));
        self.spawn_effect(EffectKind::SparkRight, x, y);
    }

    /// `sub_15338`：玩家 2 的砲座：位置、碰撞、繪製、發射。砲座編號 2、3。
    pub(crate) fn s15338(&mut self) {
        if self.ds.r8(a(0x1DAEF)) != 0x59 {
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DAFA));
        self.ds.w16(a(0x1DAFE), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DAFC));
        self.ds.w16(a(0x1DB00), self.r.ax);
        self.s15691();
        self.s156dc();
        self.r.ax = self.ds.r16(a(0x1DB02));
        self.ds.w16(a(0x26880), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB04)).wrapping_add(2);
        self.ds.w16(a(0x26882), self.r.ax);
        let y0 = self.r.ax; // push ax
        self.r.ax = self.ds.r16(a(0x1DAF6));
        self.ds.w16(a(0x26888), self.r.ax);
        self.set_obj(Pool::Pod, 2);
        self.s18d46();
        self.s18ce2();
        if self.ds.r8(a(0x1DAF0)) != 0x4E {
            self.pl_addw(0x26882, 0x14);
            self.set_obj(Pool::Pod, 3);
            self.s18d46();
            self.s18ce2();
            self.r.ax = self.ds.r16(a(0x1DAFA));
            self.ds.w16(a(0x1DAFE), self.r.ax);
            self.r.ax = self.ds.r16(a(0x1DAFC)).wrapping_add(2);
            self.ds.w16(a(0x1DB00), self.r.ax);
            self.s15691();
            self.s156dc();
        }
        self.clear_obj();
        // 15395
        self.r.ax = y0; // pop ax
        self.ds.w16(a(0x26882), self.r.ax);
        self.s15436();
        if self.ds.r8(a(0x1DB07)) != 0x59 {
            return;
        }
        // 153A4：砲座開火。第二座砲座在第一座下方 2 列（玩家 1 是 5 列）。
        let facing = Facing::from_word(self.ds.r16(a(0x1DAF1)));
        let (col, row, weapon) = (self.ds.r16(a(0x1DAFA)), self.ds.r16(a(0x1DAFC)), self.ds.r16(a(0x1DB0B)));
        self.fire_player_bullets(weapon, facing, col, row);
        if self.ds.r8(a(0x1DAF0)) != 0x4E {
            self.fire_player_bullets(weapon, facing, col, row.wrapping_add(2));
        }
        // 153DE：噴焰（圖 0Ah）。
        if self.ds.r16(a(0x1DAF1)) != 0x3E {
            self.pl_addw(0x26880, 1u16.wrapping_neg());
            self.pl_addw(0x26882, 2);
        } else {
            self.pl_addw(0x26880, 2);
            self.pl_addw(0x26882, 2);
        }
        self.ds.w16(a(0x26888), 0x0A);
        self.set_obj(Pool::Pod, 2);
        self.s18d46();
        self.s18ce2();
        if self.ds.r8(a(0x1DAF0)) != 0x4E {
            self.pl_addw(0x26882, 0x14);
            self.set_obj(Pool::Pod, 3);
            self.s18d46();
            self.s18ce2();
        }
        self.clear_obj();
    }

    /// `sub_15436`：玩家 2 砲座位置與圖框，充能小於 6 用圖 1Eh 或 24h，否則交給 `sub_1549B`。
    pub(crate) fn s15436(&mut self) {
        if self.ds.r16(a(0x1DAED)) >= 6 {
            self.s1549b();
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DB1F)).wrapping_sub(0x0A);
        self.ds.w16(a(0x1DB04), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB21)).wrapping_add(3);
        self.ds.w16(a(0x1DB02), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB37)).wrapping_sub(1);
        self.ds.w16(a(0x1DAFC), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB39)).wrapping_add(3);
        self.ds.w16(a(0x1DAFA), self.r.ax);
        self.pl_addw(0x1DAF8, 1);
        if self.ds.r16(a(0x1DAF8)) >= 5 {
            self.ds.w16(a(0x1DAF8), 0);
        }
        if self.ds.r16(a(0x1DAF1)) == 0x3E {
            // 15491
            self.r.ax = self.ds.r16(a(0x1DAF8)).wrapping_add(0x1E);
            self.ds.w16(a(0x1DAF6), self.r.ax);
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DAF8)).wrapping_add(0x24);
        self.ds.w16(a(0x1DAF6), self.r.ax);
        self.pl_addw(0x1DAFA, 5u16.wrapping_neg());
        self.pl_addw(0x1DB02, 5u16.wrapping_neg());
    }

    /// `sub_1549B`：玩家 2 充能 6 至 13 的砲座圖框（31h 起），14 以上交給 `sub_15500`。
    pub(crate) fn s1549b(&mut self) {
        if self.ds.r16(a(0x1DAED)) >= 0x0E {
            self.s15500();
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DB1F)).wrapping_sub(0x0A);
        self.ds.w16(a(0x1DB04), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB21)).wrapping_add(3);
        self.ds.w16(a(0x1DB02), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB37)).wrapping_sub(1);
        self.ds.w16(a(0x1DAFC), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB39)).wrapping_add(3);
        self.ds.w16(a(0x1DAFA), self.r.ax);
        self.pl_addw(0x1DAF8, 1);
        if self.ds.r16(a(0x1DAF8)) >= 3 {
            self.ds.w16(a(0x1DAF8), 0);
        }
        if self.ds.r16(a(0x1DAF1)) == 0x3E {
            // 154F6
            self.r.ax = self.ds.r16(a(0x1DAF8)).wrapping_add(0x31);
            self.ds.w16(a(0x1DAF6), self.r.ax);
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DAF8)).wrapping_add(0x31);
        self.ds.w16(a(0x1DAF6), self.r.ax);
        self.pl_addw(0x1DAFA, 5u16.wrapping_neg());
        self.pl_addw(0x1DB02, 5u16.wrapping_neg());
    }

    /// `sub_15500`：玩家 2 充能 14 以上的砲座圖框（34h 起）。
    pub(crate) fn s15500(&mut self) {
        self.r.ax = self.ds.r16(a(0x1DB1F)).wrapping_sub(0x0A);
        self.ds.w16(a(0x1DB04), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB21)).wrapping_add(3);
        self.ds.w16(a(0x1DB02), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB37)).wrapping_sub(1);
        self.ds.w16(a(0x1DAFC), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB39)).wrapping_add(3);
        self.ds.w16(a(0x1DAFA), self.r.ax);
        self.pl_addw(0x1DAF8, 1);
        if self.ds.r16(a(0x1DAF8)) >= 3 {
            self.ds.w16(a(0x1DAF8), 0);
        }
        if self.ds.r16(a(0x1DAF1)) == 0x3E {
            // 15550
            self.r.ax = self.ds.r16(a(0x1DAF8)).wrapping_add(0x34);
            self.ds.w16(a(0x1DAF6), self.r.ax);
            return;
        }
        self.r.ax = self.ds.r16(a(0x1DAF8)).wrapping_add(0x34);
        self.ds.w16(a(0x1DAF6), self.r.ax);
        self.pl_addw(0x1DAFA, 5u16.wrapping_neg());
        self.pl_addw(0x1DB02, 5u16.wrapping_neg());
    }

    /// `sub_1555A`：在 G3 以玩家 2 的位置標 2 列 4 欄的髒格（值 1）。
    pub(crate) fn s1555a(&mut self) {
        self.r.ax = self.ds.r16(a(0x1DB37));
        self.r.bx = 0x28;
        self.r.ax = (self.r.al() as u16) * (self.r.bl() as u16);
        self.r.ax = self.r.ax.wrapping_add(self.ds.r16(a(0x1DB39)));
        self.r.ax = self.r.ax.wrapping_add(1);
        self.r.ax <<= 1;
        self.r.bx = self.r.ax;
        self.r.cx = 2;
        self.r.ax = 1;
        loop {
            let outer = self.r.cx;
            self.r.cx = 4;
            loop {
                self.ds.w16(self.r.bx.wrapping_sub(0x69B4), self.r.ax);
                self.r.bx = self.r.bx.wrapping_add(2);
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.cx = outer;
            self.r.bx = self.r.bx.wrapping_add(0x48);
            if !self.pl_loop() {
                break;
            }
        }
    }

    /// `sub_15587`：玩家 2 的髒格形狀（`dseg:0F8Dh`）以 OR 疊進 G3。
    pub(crate) fn s15587(&mut self) {
        self.r.si = 0x0F8D;
        self.pl_lodsw();
        self.ds.w16(a(0x2687E), self.r.ax);
        self.pl_lodsw();
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1DB37)).wrapping_sub(1);
        loop {
            let outer = self.r.cx;
            self.s15605();
            loop {
                if !(self.r.di > 0x9C3C || self.r.di < 0x964C) {
                    self.r.set_al(self.ds.r8(self.r.si));
                    let v = self.ds.r8(self.r.di) | self.r.al();
                    self.ds.w8(self.r.di, v);
                }
                self.r.di = self.r.di.wrapping_add(2);
                self.r.si = self.r.si.wrapping_add(1);
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = outer;
            if !self.pl_loop() {
                break;
            }
        }
    }

    /// `sub_155BE`：玩家 2 對 G4 地形的碰撞，只在第三關。
    pub(crate) fn s155be(&mut self) {
        if self.ds.r8(a(0x1CFDB)) != 0x43 {
            return;
        }
        self.r.si = 0x0F8D;
        self.pl_lodsw();
        self.ds.w16(a(0x2687E), self.r.ax);
        self.pl_lodsw();
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1DB37));
        self.r.cx = 1;
        loop {
            let outer = self.r.cx;
            self.s1562a();
            loop {
                if !(self.r.di > 0x95FC || self.r.di < 0x900C) {
                    self.r.ax = self.ds.r16(self.r.di);
                    if self.r.ax != 0 {
                        self.ds.w8(a(0x1DB1A), 0x59);
                    }
                }
                self.r.di = self.r.di.wrapping_add(2);
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = outer;
            if !self.pl_loop() {
                break;
            }
        }
    }

    /// `sub_15605`：G3 的列位址（玩家 2）。
    pub(crate) fn s15605(&mut self) {
        let saved = self.r.bx;
        if self.r.bx >= 0x14 {
            self.r.bx = 0x14;
        }
        self.r.bx <<= 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x21C2));
        self.r.ax = self.r.ax.wrapping_add(0x964C);
        self.r.di = self.r.ax;
        self.r.ax = self.ds.r16(a(0x1DB39));
        self.r.ax = self.r.ax.wrapping_sub(4);
        self.r.ax <<= 1;
        self.r.ax = self.r.ax.wrapping_add(1);
        self.r.di = self.r.di.wrapping_add(self.r.ax);
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.bx = saved;
    }

    /// `sub_1562A`：G4 的列位址（玩家 2）。
    pub(crate) fn s1562a(&mut self) {
        let saved = self.r.bx;
        if self.r.bx >= 0x14 {
            self.r.bx = 0x14;
        }
        self.r.bx <<= 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x21C2));
        self.r.ax = self.r.ax.wrapping_add(0x900C);
        self.r.di = self.r.ax;
        self.r.ax = self.ds.r16(a(0x1DB39));
        self.r.ax <<= 1;
        self.r.ax = self.r.ax.wrapping_add(1);
        self.r.di = self.r.di.wrapping_add(self.r.ax);
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.bx = saved;
    }

    /// `sub_1564C`：玩家 2 本體（形狀 `0F7Bh`）對 G1，設 `byte_1DB1A` = `Y`。
    pub(crate) fn s1564c(&mut self) {
        self.r.si = 0x0F7B;
        self.pl_lodsw();
        self.ds.w16(a(0x2687E), self.r.ax);
        self.pl_lodsw();
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1DB37));
        loop {
            let outer = self.r.cx;
            self.s1578f();
            loop {
                let saved_ax = self.r.ax;
                if !(self.r.di >= 0x217C
                    || self.r.di <= 0x1B32
                    || self.r.di >= self.r.dx
                    || self.r.di <= self.r.ax)
                {
                    self.r.set_al(self.ds.r8(self.r.di));
                    if self.r.al() != 0 && self.r.al() != 0xFF {
                        self.ds.w8(a(0x1DB1A), 0x59);
                    }
                }
                self.r.di = self.r.di.wrapping_add(1);
                self.r.ax = saved_ax;
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = outer;
            if !self.pl_loop() {
                break;
            }
        }
    }

    /// `sub_15691`：玩家 2 砲座（形狀 `0F85h`）對 G1，記錄於 `byte_1DAEC`，再呼叫 `sub_15746`。
    pub(crate) fn s15691(&mut self) {
        self.ds.w8(a(0x1DAEC), 0);
        self.r.si = 0x0F85;
        self.pl_lodsw();
        self.ds.w16(a(0x2687E), self.r.ax);
        self.pl_lodsw();
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1DB00));
        loop {
            let outer = self.r.cx;
            self.s1576b();
            loop {
                let saved_ax = self.r.ax;
                if !(self.r.di >= 0x217C
                    || self.r.di <= 0x1B32
                    || self.r.di >= self.r.dx
                    || self.r.di <= self.r.ax)
                {
                    self.r.set_al(self.ds.r8(self.r.di));
                    if self.r.al() != 0 && self.r.al() != 0xFF {
                        self.ds.w8(a(0x1DAEC), self.r.al());
                    }
                }
                self.r.di = self.r.di.wrapping_add(1);
                self.r.ax = saved_ax;
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = outer;
            if !self.pl_loop() {
                break;
            }
        }
        self.s15746();
    }

    /// `sub_156DC`：玩家 2 砲座（用本體形狀 `0F7Bh`）對 G2。`sub_15882` 改寫 `bx`，之後的列計數沿用殘值（原版行為）。
    pub(crate) fn s156dc(&mut self) {
        self.r.si = 0x0F7B;
        self.pl_lodsw();
        self.ds.w16(a(0x2687E), self.r.ax);
        self.pl_lodsw();
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1DB00));
        loop {
            let outer = self.r.cx;
            self.s15722();
            loop {
                let (saved_cx, saved_ax) = (self.r.cx, self.r.ax);
                if !(self.r.di >= 0x3266
                    || self.r.di <= 0x2C1C
                    || self.r.di >= self.r.dx
                    || self.r.di <= self.r.ax)
                {
                    self.r.set_al(self.ds.r8(self.r.di));
                    if self.r.al() != 0 && self.r.al() & 0xE0 == 0 {
                        if let Some(bx) = self.remove_hit_bullet(self.r.al(), self.r.di) {
                            self.r.bx = bx;
                        }
                    }
                }
                self.r.di = self.r.di.wrapping_add(1);
                self.r.ax = saved_ax;
                self.r.cx = saved_cx;
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = outer;
            if !self.pl_loop() {
                break;
            }
        }
    }

    /// `sub_15722`：G2 的列位址（玩家 2 砲座，`word_1DAFE`）。
    pub(crate) fn s15722(&mut self) {
        let saved = self.r.bx;
        if self.r.bx >= 0x17C {
            self.r.bx = 0x1F4;
        }
        self.r.bx <<= 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x1B0A));
        self.r.ax = self.r.ax.wrapping_add(0x2C1C);
        self.r.dx = self.r.ax;
        self.r.di = self.r.ax;
        self.r.di = self.r.di.wrapping_add(self.ds.r16(a(0x1DAFE)));
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.dx = self.r.dx.wrapping_add(0x45);
        self.r.bx = saved;
    }

    /// `sub_15746`：玩家 2 砲座碰到的敵人（`byte_1DAEC`）加 1 傷害，同 `sub_14C18`（含 FEh 的表外寫入）。
    pub(crate) fn s15746(&mut self) {
        self.r.set_al(self.ds.r8(a(0x1DAEC)));
        if self.r.al() == 0xFF {
            return;
        }
        if self.r.al() == 0 {
            return;
        }
        self.r.set_al(self.r.al().wrapping_sub(1));
        self.r.set_ah(0);
        self.r.bx = self.r.ax;
        self.r.bx <<= 1;
        self.r.ax = self.entity_link(self.r.al());
        // armor_cell_index：裝甲格 FEh 與已釋放的槽不寫傷害。
        if self.fixes.armor_cell_index && (self.r.bx == 0x1FA || self.r.ax == 0) {
            return;
        }
        let link = self.r.ax;
        self.r.si = link.wrapping_add(0x20);
        self.r.ax = self.add_entity_damage(link, 1);
    }

    /// `sub_1576B`：G1 的列位址（玩家 2 砲座，`word_1DAFE`）。
    pub(crate) fn s1576b(&mut self) {
        let saved = self.r.bx;
        if self.r.bx >= 0x17C {
            self.r.bx = 0x1F4;
        }
        self.r.bx <<= 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x1B0A));
        self.r.ax = self.r.ax.wrapping_add(0x1B32);
        self.r.dx = self.r.ax;
        self.r.di = self.r.ax;
        self.r.di = self.r.di.wrapping_add(self.ds.r16(a(0x1DAFE)));
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.dx = self.r.dx.wrapping_add(0x45);
        self.r.bx = saved;
    }

    /// `sub_1578F`：G1 的列位址（玩家 2 本體，`word_1DB39`）。
    pub(crate) fn s1578f(&mut self) {
        let saved = self.r.bx;
        if self.r.bx >= 0x17C {
            self.r.bx = 0x1F4;
        }
        self.r.bx <<= 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x1B0A));
        self.r.ax = self.r.ax.wrapping_add(0x1B32);
        self.r.dx = self.r.ax;
        self.r.di = self.r.ax;
        self.r.di = self.r.di.wrapping_add(self.ds.r16(a(0x1DB39)));
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.dx = self.r.dx.wrapping_add(0x45);
        self.r.bx = saved;
    }

    /// `sub_157B3`：玩家 2 本體（形狀 `0F7Bh`，列加 1）對 G2：刪除敵彈，再判定道具或傷害。
    /// `sub_15882` 改寫 `bx`，之後的列計數沿用殘值（原版行為）。
    pub(crate) fn s157b3(&mut self) {
        self.ds.w8(a(0x1DB0A), 0x4E);
        self.r.si = 0x0F7B;
        self.pl_lodsw();
        self.ds.w16(a(0x2687E), self.r.ax);
        self.pl_lodsw();
        self.r.cx = self.r.ax;
        self.r.bx = self.ds.r16(a(0x1DB37));
        self.r.bx = self.r.bx.wrapping_add(1);
        loop {
            let outer = self.r.cx;
            self.s15896();
            loop {
                let (saved_cx, saved_ax) = (self.r.cx, self.r.ax);
                if !(self.r.di >= 0x3266
                    || self.r.di <= 0x2C1C
                    || self.r.di >= self.r.dx
                    || self.r.di <= self.r.ax)
                {
                    self.r.set_al(self.ds.r8(self.r.di));
                    if self.r.al() != 0 {
                        if let Some(bx) = self.remove_hit_bullet(self.r.al(), self.r.di) {
                            self.r.bx = bx;
                        }
                        self.s15802();
                    }
                }
                self.r.di = self.r.di.wrapping_add(1);
                self.r.ax = saved_ax;
                self.r.cx = saved_cx;
                if !self.pl_loop() {
                    break;
                }
            }
            self.r.bx = self.r.bx.wrapping_add(1);
            self.r.cx = outer;
            if !self.pl_loop() {
                break;
            }
        }
        self.s15832();
    }

    /// `sub_15802`：玩家 2 的道具或傷害判定，同 `sub_14CD4`。
    pub(crate) fn s15802(&mut self) {
        self.r.set_ah(self.r.al());
        self.r.set_ah(self.r.ah() & 0xF0);
        let ah = self.r.ah();
        if ah == 0x80 || ah == 0x40 || ah == 0xC0 || ah == 0x20 {
            self.ds.w8(a(0x1DB0A), ah);
            self.ds.w16(a(0x1DB08), 0xFF42);
            return;
        }
        self.ds.w8(a(0x1DB1A), 0x59);
    }

    /// `sub_15832`：玩家 2 依撿到的道具升級，同 `sub_14D04`。
    pub(crate) fn s15832(&mut self) {
        self.r.set_al(self.ds.r8(a(0x1DB0A)));
        let al = self.r.al();
        if al == 0x4E {
            return;
        }
        if self.ds.r8(a(0x1DAEF)) == 0x4E {
            self.ds.w8(a(0x1DAEF), 0x59);
            return;
        }
        if self.ds.r8(a(0x1DAF0)) == 0x4E {
            self.ds.w8(a(0x1DAF0), 0x59);
            return;
        }
        if al == 0x80 {
            self.ds.w16(a(0x1DB0B), 0x4C);
        } else if al == 0x40 {
            self.ds.w16(a(0x1DB0B), 0x42);
        } else if al == 0xC0 {
            self.ds.w16(a(0x1DB0B), 0x4B);
        } else if al == 0x20 {
            self.ds.w16(a(0x1DB0B), 0x53);
        }
    }

    /// `sub_15896`：G2 的列位址（玩家 2 本體，`word_1DB39`）。
    pub(crate) fn s15896(&mut self) {
        let saved = self.r.bx;
        if self.r.bx >= 0x17C {
            self.r.bx = 0x1F4;
        }
        self.r.bx <<= 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x1B0A));
        self.r.ax = self.r.ax.wrapping_add(0x2C1C);
        self.r.dx = self.r.ax;
        self.r.di = self.r.ax;
        self.r.di = self.r.di.wrapping_add(self.ds.r16(a(0x1DB39)));
        self.r.cx = self.ds.r16(a(0x2687E));
        self.r.dx = self.r.dx.wrapping_add(0x45);
        self.r.bx = saved;
    }

    /// `sub_158BA`：每關開始的玩家 2 位置（欄 5、列 10）。第一關或示範時另重設武器、血量、砲座與朝向（3Eh）。
    pub(crate) fn s158ba(&mut self) {
        self.ds.w16(a(0x1DB39), 5);
        self.ds.w16(a(0x1DB37), 0x0A);
        self.r.ax = self.ds.r16(a(0x1DB39));
        self.ds.w16(a(0x1DB21), self.r.ax);
        self.r.ax = self.ds.r16(a(0x1DB37));
        self.r.bx = 0x0A;
        self.r.ax = (self.r.al() as u16) * (self.r.bl() as u16);
        self.ds.w16(a(0x1DB1F), self.r.ax);
        if self.ds.r8(a(0x1CFDC)) != 0x59 && self.ds.r8(a(0x1CFDB)) != 0x41 {
            return;
        }
        // 158E6
        self.ds.w16(a(0x1DB0B), 0);
        self.ds.w16(a(0x1CFE0), 0xFF);
        self.ds.w8(a(0x1DAEF), 0x4E);
        self.ds.w8(a(0x1DAF0), 0x4E);
        self.ds.w16(a(0x1DAF1), 0x3E);
    }

    // ---------------------------------------------------------------
    // 頭目段背景
    // ---------------------------------------------------------------

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
        self.pl_addw(0x1DB6B, 1);
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
        self.pl_addw(0x1DB5B, 0x0Au16.wrapping_neg());
    }

    /// `sub_15C2B`：`word_1DB5B` 加 0Ah。
    pub(crate) fn s15c2b(&mut self) {
        self.pl_addw(0x1DB5B, 0x0A);
    }

    /// `sub_15C31`：`word_1DB5D` 減 1。
    pub(crate) fn s15c31(&mut self) {
        self.pl_addw(0x1DB5D, 1u16.wrapping_neg());
    }

    /// `sub_15C36`：`word_1DB5D` 加 1。
    pub(crate) fn s15c36(&mut self) {
        self.pl_addw(0x1DB5D, 1);
    }
}
