//! 第三關與地圖捲動。
//! 逐函式移植自機器碼，查法見 `tools/q.sh`。規格：`docs/spec/002-full-game.md`。
//!
//! 只移植 E 模式（`byte_1CF74` = `E`、`byte_1CF75` = `N`）。CGA、MGA、純 EGA 的分支
//! （`sub_185AA`、`sub_18613`、`sub_1812E`、`sub_18086`）不移植。
//!
//! 地圖 `SHT.SCN` 原版由 `sub_1BF71` 解壓到遠段 `word_1CF6E`（`sub_13BE0` 配置），
//! 之後只有 `sub_17DF9`、`sub_182CD` 讀它，沒有任何程式寫入，所以重製在 `Core::map_seg` 放一份解壓結果。
//! 捲動直接操作顯示記憶體的部分（`sub_17EBA` 等）以 `Op` 命令交給接收端，語意見 `workspace/progress/ops-needed.md`。

use crate::draw::{Op, Pool};
use crate::dseg::a;
use crate::game::{Core, Req};

impl Core {
    /// `add word ptr [linear], v`。
    fn s3_addw(&mut self, linear: u32, v: u16) {
        let n = self.ds.r16(a(linear)).wrapping_add(v);
        self.ds.w16(a(linear), n);
    }

    /// `es:[bx]`，`es` = `word_1CF6E`（地圖遠段）。超出解壓結果的部分原版讀到遠記憶體殘值，這裡讀 0。
    /// 可達的最大位移約 5900h，遠小於解壓後的 24005 位元組，所以實際不會落到這裡。
    fn map_word(&self, off: u16) -> u16 {
        let o = off as usize;
        let lo = self.map_seg.get(o).copied().unwrap_or(0);
        let hi = self.map_seg.get(o + 1).copied().unwrap_or(0);
        u16::from_le_bytes([lo, hi])
    }

    /// `sub_1BF71` 的解壓部分：`sub_1C757` 把 `SHT.SCN` 的 RLE 串流解到 `word_1CF6E` 段。
    fn load_map(&mut self) {
        let f = self.files.clone();
        self.map_seg = if f.scn.len() >= 2 { rs2_formats::unrle(&f.scn, 0) } else { Vec::new() };
    }

    /// `mul word_25BC8` 與 `add word_25BC6`：`ax` = `258h` x 地圖列 + 地圖左緣欄，`dx` 是乘積高字組。
    fn map_row_col(&mut self) {
        self.r.bx = self.ds.r16(a(0x25BC8));
        let p = 0x258u32 * self.r.bx as u32;
        self.r.dx = (p >> 16) as u16;
        self.r.ax = (p as u16).wrapping_add(self.ds.r16(a(0x25BC6)));
    }

    /// `sub_13A32`：頭目前換圖。清暫存區 A，把 `SHT3` 第 43 項（x=0、y=0）寫進去。
    pub(crate) fn s13a32(&mut self) {
        self.s11504();
        self.ds.w16(a(0x2685C), 0);
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26882), 0);
        self.ds.w16(a(0x26888), 0x2B);
        self.s19adc();
    }

    /// `sub_13A51`：第三關完整入口，含載入與主迴圈。回傳時 `dx` 是離開碼（`E`、`Q`、`C`）。
    pub(crate) async fn s13a51(&mut self) {
        self.s13be0();
        self.s13ccd();
        self.s19a88();
        self.ds.w16(a(0x26872), 0x8000);
        // nullsub_4：被改成 retn 的載入進度條，只留下 cx。
        self.r.cx = 8;
        self.s1afbc();
        self.s1ae91();
        self.s1bf71();
        self.load_map();
        self.s18894();
        self.s13d32();
        self.s19a88();
        self.ds.w8(a(0x1CFDB), 0x43);
        self.s10e33();
        self.ds.w16(a(0x25BC4), 0);
        self.ds.w8(a(0x1CFAD), 0x4E);
        self.fx.reset_chain();
        self.s1aa86();
        self.s10ad4();
        self.ds.w8(a(0x2739A), 0x59);
        loop {
            self.s19a37();
            self.s13afa().await;
            self.s19a88();
            self.s13afa().await;
            self.s10d99();
            self.s10d8d();
            let dx = self.r.dx;
            if dx == 0x45 || dx == 0x51 || dx == 0x43 {
                break;
            }
            if self.ds.r8(a(0x1CFAD)) == 0x46 {
                // 13ACFh：過關連爆。
                let n = self.ds.r8(a(0x1CFAE)).wrapping_add(1);
                self.ds.w8(a(0x1CFAE), n);
                self.r.dx = 0x43;
                self.fx.shift_chain_left(3);
                if n > 0x14 {
                    break;
                }
                let page = self.ds.r16(a(0x26872));
                self.spawn_stage_clear_burst(page);
            }
            self.suspend(Req::Checkpoint).await;
        }
        // 13AC6h
        self.ds.w8(a(0x2739A), 0x4E);
        self.s10b12();
    }

    /// `sub_13AFA`：計數超過 469h 走頭目段。
    pub(crate) async fn s13afa(&mut self) {
        if self.ds.r16(a(0x1CFE2)) > 0x469 {
            self.s13b75().await;
        } else {
            self.s13b0a().await;
        }
    }

    /// `sub_13B0A`：普通段的一幀。頁 8000h 走 `sub_182CD`，頁 0 走 `sub_18337`。
    /// 取樣點在 `inc word_1CFE2` 之後（13B5Fh），所以排在等待之後。
    pub(crate) async fn s13b0a(&mut self) {
        self.ds.w16(a(0x2685C), 0);
        if self.ds.r16(a(0x26872)) != 0 {
            self.s182cd();
        } else {
            self.s18337();
        }
        self.s13b64();
        self.ds.w16(a(0x2685C), 0);
        self.s17d9a();
        self.s15d8c();
        self.s14458();
        self.s14f92();
        self.s164bb();
        self.update_enemy_bullets();
        self.update_player_bullets();
        self.s14704();
        self.s15232();
        self.s164d7();
        self.update_effects();
        self.s1100a();
        self.s162b1();
        self.clear_grid2();
        self.s10ecb().await;
        // sub_1117F 只有 MGA 分支。
        self.r.cx = 2;
        self.s11097().await;
        self.s3_addw(0x1CFE2, 1);
        self.suspend(Req::Sample(0x13B5F)).await;
    }

    /// `sub_13B64`：計數 0 時重設視窗位移，把地圖左側整片畫進暫存區 A。
    pub(crate) fn s13b64(&mut self) {
        if self.ds.r16(a(0x1CFE2)) != 0 {
            return;
        }
        self.ds.w16(a(0x25BC4), 0);
        self.s17df9();
    }

    /// `sub_13B75`：頭目段的一幀。計數到 46Ch 換合成圖並把關卡字母改成 `c`，超過 474h 釘回 474h。
    pub(crate) async fn s13b75(&mut self) {
        self.ds.w16(a(0x2685C), 0);
        self.s17d9a();
        self.s15924();
        self.s15d8c();
        self.s14458();
        self.s14f92();
        self.s164bb();
        self.update_enemy_bullets();
        self.update_player_bullets();
        self.s14704();
        self.s15232();
        self.s164d7();
        self.update_effects();
        // 13B9Fh 的 cmp word_26872, 0 與 jz $+2 沒有效果。
        self.s1100a();
        self.s162b1();
        self.clear_grid2();
        self.s10ecb().await;
        self.r.cx = 2;
        self.s11097().await;
        self.s3_addw(0x1CFE2, 1);
        self.suspend(Req::Sample(0x13BBB)).await;
        let c = self.ds.r16(a(0x1CFE2));
        if c == 0x46C {
            self.s13a32();
            self.ds.w8(a(0x1CFDB), 0x63);
            return;
        }
        if c > 0x474 {
            self.ds.w16(a(0x1CFE2), 0x474);
        }
    }

    /// `sub_13BE0`：配置 10 塊圖庫緩衝區。記憶體配置不移植，只留下 `dseg` 的檔名字串與計數
    /// （`dseg:336h` 起，A0 豁免範圍內），段位址 `word_1CF5C` 至 `word_1CF6E` 是配置結果，同樣在豁免範圍內。
    pub(crate) fn s13be0(&mut self) {
        self.s3_alloc_names(0x41, 0x2B);
        // 10 次 sub_10210，每次 inc byte_1CEF8 與 byte_1CF0A。
        self.s3_alloc_counts();
    }

    /// `sub_13CCD`：釋放同樣 10 塊。`sub_1026C` 的每次呼叫同樣 inc 兩個計數。
    pub(crate) fn s13ccd(&mut self) {
        self.s3_alloc_names(0x42, 0x2D);
        self.s3_alloc_counts();
    }

    fn s3_alloc_names(&mut self, c0: u8, c1: u8) {
        self.ds.w8(a(0x1CEF6), c0);
        self.ds.w8(a(0x1CF08), c0);
        self.ds.w8(a(0x1CEF7), c1);
        self.ds.w8(a(0x1CF09), c1);
        self.ds.w8(a(0x1CEF8), 0x30);
        self.ds.w8(a(0x1CF0A), 0x30);
    }

    fn s3_alloc_counts(&mut self) {
        for o in [a(0x1CEF8), a(0x1CF0A)] {
            let v = self.ds.r8(o).wrapping_add(10);
            self.ds.w8(o, v);
        }
    }

    /// `sub_13D32`：`stage3` 調色盤。純 EGA 分支（`byte_1CF75` = `Y`）不移植。
    pub(crate) fn s13d32(&mut self) {
        self.r.bx = 0;
        self.r.dx = 0x0D29;
        self.s13d61();
        self.r.bx = 0x30;
        self.r.dx = 0x0D29;
        self.s13d61();
    }

    /// `sub_13D61`：INT 10h AX=1012h，從 DAC 暫存器 `bx` 起寫 16 個，值在 `dseg:dx`。
    pub(crate) fn s13d61(&mut self) {
        self.set_dac(self.r.bx as u8, self.r.dx);
        self.r.cx = 0x10;
        self.r.ax = 0x1012;
    }

    /// `sub_15924`：頭目段背景。清頁後畫合成圖。
    pub(crate) fn s15924(&mut self) {
        self.s19cf9();
        self.s15983();
    }

    /// `sub_15983`：第三關頭目合成圖的捲動。計數不超過 46Ch 時重設腳本，之後逐幀讀腳本位元組（`word_1DB6D`）移動，
    /// 腳本結束位元組 0Fh 時設 `byte_1CFAD` = `F`（過關連爆）。結構同 `sub_15930`。
    pub(crate) fn s15983(&mut self) {
        if self.ds.r16(a(0x1CFE2)) <= 0x46C {
            self.ds.w16(a(0x1DB5F), 0x1E);
            self.ds.w16(a(0x1DB61), 0x28);
            self.ds.w16(a(0x1DB6D), 0x1692);
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
        self.s15af4();
        if self.ds.r8(a(0x1DB5A)) == 0x0F {
            self.ds.w8(a(0x1CFAD), 0x46);
        }
    }

    /// `sub_15AF4`：把暫存區 A（`word_2685C` = 0）的合成圖以水平位移 `-word_1DB5D` 畫到目前頁，共 C1h 列。
    pub(crate) fn s15af4(&mut self) {
        self.ds.w16(a(0x2685C), 0);
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

    /// `sub_17D9A`：第三關時間表（`dseg:68E0h`）。
    pub(crate) fn s17d9a(&mut self) {
        self.r.si = 0x68E0;
        self.s15d1c();
    }

    /// `sub_17DF9`：把地圖左側 42 欄 x 19 列圖塊畫進暫存區 A，並把圖塊索引寫進快取（`dseg:900Eh` 起，連續，每列 42 字組）。
    /// 快取的列距在這裡是 84 位元組，`sub_1839F` 寫入時是 80 位元組（G4 的列距），兩者不同，照原版。
    pub(crate) fn s17df9(&mut self) {
        // E 模式。單色走 sub_185AA。
        self.map_row_col();
        self.r.ax <<= 1;
        self.ds.w16(a(0x25BCA), self.r.ax);
        self.ds.w16(a(0x2685C), 0);
        let x0 = self.ds.r16(a(0x25BC4));
        self.ds.w16(a(0x26880), x0);
        self.ds.w16(a(0x26882), 0);
        self.r.cx = 0x13;
        self.r.si = 0x900E;
        for _ in 0..0x13 {
            let x0 = self.ds.r16(a(0x25BC4));
            self.ds.w16(a(0x26880), x0);
            for _ in 0..0x2A {
                let si = self.r.si;
                self.r.bx = self.ds.r16(a(0x25BCA));
                self.r.ax = self.map_word(self.r.bx);
                self.ds.w16(si, self.r.ax);
                self.ds.w16(a(0x26888), self.r.ax);
                self.s18ace();
                self.s3_addw(0x25BCA, 2);
                self.s3_addw(0x26880, 1);
                self.r.si = si.wrapping_add(2);
            }
            self.s3_addw(0x25BCA, 0x45C);
            self.s3_addw(0x26882, 0x0A);
        }
        self.r.cx = 0;
        self.s17fa0();
        self.s17e73();
    }

    /// `sub_17E73`：暫存區 A 轉成 B（位移 4 像素的版本），四個平面各做一次 `sub_17EBA` 與 `sub_17EEA`。
    pub(crate) fn s17e73(&mut self) {
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x2685C), 0);
        for bl in [1u16, 2, 4, 8] {
            self.r.bx = bl;
            self.s1882e();
            self.s17eba();
            self.s17eea();
        }
        self.s18878();
        // 17EB2h 的 out 3CEh, ax=3 把 GC 索引 3 設 0，sub_18878 已經設過。
    }

    /// `sub_17EBA`：A 的前 3E80h 位元組（200 列 x 80）複製到 B（偏移 +8000h），GC 旋轉 4、取代、位元遮罩 FFh。
    /// 讀寫的平面由前一個 `sub_1882E` 決定。
    pub(crate) fn s17eba(&mut self) {
        self.r.bx = 0;
        self.emit_op(Op::S17EBA);
    }

    /// `sub_17EEA`：B 就地左移 4 像素。每個位元組取下一個位元組的低半與自己的高半（位元遮罩 0Fh），共 3E80h 個。
    pub(crate) fn s17eea(&mut self) {
        self.r.bx = 0;
        self.emit_op(Op::S17EEA);
    }

    /// `sub_17F26`：A 的第 `word_26880` 欄（200 列）複製到 B 的同一欄，旋轉 4、取代、位元遮罩 FFh。
    pub(crate) fn s17f26(&mut self) {
        self.r.bx = 0;
        self.emit_op(Op::S17F26);
    }

    /// `sub_17F5D`：B 的第 `word_26880 - 1` 欄（200 列）取下一欄的低半，位元遮罩 0Fh，結束時還原 FFh。
    pub(crate) fn s17f5d(&mut self) {
        self.r.bx = 0;
        self.emit_op(Op::S17F5D);
    }

    /// `sub_17FA0`：暫存區 A 第 `word_25BC4` 欄起的 40 欄 x 190 列，以寫入模式 1 複製到目前顯示頁。
    pub(crate) fn s17fa0(&mut self) {
        let x = self.ds.r16(a(0x25BC4));
        self.ds.w16(a(0x26880), x);
        self.ds.w16(a(0x2685C), 0);
        self.set_obj(Pool::Background, 0);
        self.emit_op(Op::S17FA0);
        self.clear_obj();
        self.ds.w16(a(0x26880), 0);
        // sub_19683 在 cx <= 0 時把 bx 設成 12Ch。進來時的 cx 是呼叫端留下的 0。
        self.r.bx = if (self.r.cx as i16) <= 0 { 0x12C } else { 0 };
        self.r.cx = 0;
    }

    /// `sub_18031`：同 `sub_17FA0`，來源是暫存區 B（`word_2685C` = 8000h）。結束時 `word_2685C` 留在 8000h。
    pub(crate) fn s18031(&mut self) {
        let x = self.ds.r16(a(0x25BC4));
        self.ds.w16(a(0x26880), x);
        self.ds.w16(a(0x2685C), 0x8000);
        self.set_obj(Pool::Background, 0);
        self.emit_op(Op::S18031);
        self.clear_obj();
        self.ds.w16(a(0x26880), 0);
        self.r.bx = if (self.r.cx as i16) <= 0 { 0x12C } else { 0 };
        self.r.cx = 0;
    }

    /// `sub_180C6`：視窗位移加 1。到 28h 時歸 0，並把暫存區 A、B 各整體左移 28h 位元組（3B60h 位元組，寫入模式 1）。
    pub(crate) fn s180c6(&mut self) {
        self.s3_addw(0x25BC4, 1);
        if self.ds.r16(a(0x25BC4)) <= 0x27 {
            return;
        }
        self.ds.w16(a(0x25BC4), 0);
        self.r.bx = 0;
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x2685C), 0);
        self.emit_op(Op::S180C6);
        self.r.cx = 0;
    }

    /// `sub_182CD`：頁 8000h 的捲動步。推進視窗，地圖左緣加 1，G4 快取左移一個字組，
    /// 在視窗右緣外一欄（x = `word_25BC4` + 28h）取 19 個圖塊，經 `sub_1839F` 存進快取並寫進暫存區 A，再顯示。
    pub(crate) fn s182cd(&mut self) {
        // E 模式。單色走 sub_18613。
        self.s180c6();
        self.s3_addw(0x25BC6, 1);
        self.s1840e();
        self.map_row_col();
        self.r.ax = self.r.ax.wrapping_add(0x28) << 1;
        self.ds.w16(a(0x25BCA), self.r.ax);
        self.r.ax = self.ds.r16(a(0x25BC4)).wrapping_add(0x28);
        self.ds.w16(a(0x26880), self.r.ax);
        self.ds.w16(a(0x26882), 0);
        self.ds.w16(a(0x2684C), 0x50);
        self.r.cx = 0x13;
        for _ in 0..0x13 {
            self.r.bx = self.ds.r16(a(0x25BCA));
            self.r.ax = self.map_word(self.r.bx);
            self.s1839f();
            self.ds.w16(a(0x26888), self.r.ax);
            self.s18ace();
            self.s3_addw(0x25BCA, 0x4B0);
            self.s3_addw(0x26882, 0x0A);
        }
        self.r.cx = 0;
        self.s17fa0();
    }

    /// `sub_18337`：頁 0 的捲動步。對四個平面各把新欄與接縫同步到暫存區 B，再顯示 B。
    pub(crate) fn s18337(&mut self) {
        // E 模式。單色走 sub_18086。
        let x = self.ds.r16(a(0x25BC4)).wrapping_add(0x28);
        self.ds.w16(a(0x26880), x);
        self.ds.w16(a(0x2685C), 0);
        for bl in [1u16, 2, 4, 8] {
            self.r.bx = bl;
            self.s1882e();
            self.s17f26();
            self.s17f5d();
        }
        self.s18878();
        self.s18031();
        self.ds.w16(a(0x2685C), 0);
        // 1838Ch 的 out 3CEh ax=5、ax=3 把 GC 索引 5、3 設 0，前面的命令結束時已是 0。
    }

    /// `sub_1839F`：把 `ax`（圖塊索引）存進 G4 快取 `[word_2684C + 900Ch]`，指標加 50h（一列）。
    pub(crate) fn s1839f(&mut self) {
        self.r.bx = self.ds.r16(a(0x2684C));
        self.ds.w16(self.r.bx.wrapping_add(0x900C), self.r.ax);
        self.s3_addw(0x2684C, 0x50);
    }

    /// `sub_183EE`：只由 `183CBh` 呼叫，而 `183ADh` 起的整段在 `sub_1839F` 的 `retn` 之後，沒有入口。
    #[allow(dead_code)]
    pub(crate) fn s183ee(&mut self) {
        unreachable!("sub_183EE：死碼，沒有入口")
    }

    /// `sub_1840E`：G4（`900Ch`）與 G3（`964Ch`）各整體左移一個字組（各 320h 字組）。
    /// 兩次 `movsw` 的來源都讀過自己的尾端：G4 的最後一個字組取自 G3 的第一個，G3 的最後一個字組取自 `9C8Ch`
    /// （`word_2684C` 的位置）。最後 `stosb` 把 `9C8Ch` 寫 0，也就是清掉 `word_2684C` 的低位元組。
    pub(crate) fn s1840e(&mut self) {
        self.ds.copy16(0x900C, 0x900E, 0x320);
        self.ds.copy16(0x964C, 0x964E, 0x320);
        self.ds.w8(0x964Cu16.wrapping_add(0x640), 0);
        self.r.ax = 0;
        self.r.cx = 0;
        self.r.si = 0x964Eu16.wrapping_add(0x640);
        self.r.di = 0x964Cu16.wrapping_add(0x640).wrapping_add(1);
    }

    /// `sub_18086`：單色分支的顯示。`sub_18337` 在 `byte_1CF74` 不是 `E` 時才呼叫，不移植。
    #[allow(dead_code)]
    pub(crate) fn s18086(&mut self) {
        unreachable!("sub_18086：單色分支，E 模式不會呼叫")
    }

    /// `sub_18486`：只由 `1843Ch` 呼叫，而 `18430h` 起的垂直捲動段在 `sub_1840E` 的 `retn` 之後，沒有入口。
    #[allow(dead_code)]
    pub(crate) fn s18486(&mut self) {
        unreachable!("sub_18486：死碼，沒有入口")
    }

    /// `sub_184C0`：只由 `sub_18486` 呼叫，同屬死碼。
    #[allow(dead_code)]
    pub(crate) fn s184c0(&mut self) {
        unreachable!("sub_184C0：死碼，沒有入口")
    }

    /// `sub_184F3`：只由 `184EFh` 呼叫，而 `184E3h` 起的段落沒有入口。
    #[allow(dead_code)]
    pub(crate) fn s184f3(&mut self) {
        unreachable!("sub_184F3：死碼，沒有入口")
    }

    /// `sub_1852B`：只由 `sub_184F3` 呼叫，同屬死碼。
    #[allow(dead_code)]
    pub(crate) fn s1852b(&mut self) {
        unreachable!("sub_1852B：死碼，沒有入口")
    }
}

#[cfg(test)]
mod tests {
    use crate::dseg::a;
    use crate::game::{Entry, Fixes, Game, GameFiles, Input, Start};
    use std::path::PathBuf;

    /// 煙霧測試：以第一關初始化後的資料段從 `Entry::Stage3` 跑到頭目段，確認沒有溢位、越界或進到死碼。
    /// 不是對拍。對拍由 `rs2-trace` 的 wait 測試負責。需要原版目錄（`RS2_ORIG`）。
    #[test]
    #[ignore]
    fn stage3_smoke() {
        let dir = PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh test 跑"));
        let exe = std::fs::read(dir.join("S.EXE")).unwrap();
        let image = rs2_formats::unpack_s_exe(&exe).unwrap();
        let scn = std::fs::read(dir.join("SHT.SCN")).unwrap();
        assert_eq!(rs2_formats::unrle(&scn, 0).len(), 24005);
        // 第一關的初始狀態當資料段快照（標題流程設的旗標，例如玩家 1 存活 `byte_1CFDD`）。
        let start = Game::new(&image, &[], Start::NewGame { difficulty: None }, Fixes::NONE);
        let map = rs2_formats::unrle(&scn, 0);
        let files = GameFiles { image, scn, ..GameFiles::default() };
        let mut g = Game::from_snapshot(files, start.ds.bytes(), Entry::Stage3, Fixes::NONE);
        let mut samples = Vec::new();
        let mut cache_checked = 0usize;
        for _ in 0..2600 {
            // 玩家不死（沒有輸入，機體會被地形擠壓），才跑得到頭目段。
            g.poke16(a(0x1CFDE), 0xFF);
            let f = g.step(&Input::default(), &mut |g: &Game, pt: u32| {
                samples.push(pt);
                let frame = g.ds.r16(a(0x1CFE2));
                if pt == 0x13B5F && (100..1100).contains(&frame) {
                    // 捲動 40 步之後，G4 快取的 (列 r, 欄 c) 等於地圖 (r, 左緣欄 + c)。
                    // 例外是欄 0 的第 1 列起：新欄寫在 (r+1, 0)，線性位址上緊接 (r, 39)，下一步左移才進到 (r, 39)，
                    // 所以那 19 格是最新的一欄（地圖 (r-1, 左緣欄 + 40)）。
                    let l = g.ds.r16(a(0x25BC6)) as usize;
                    for r in 0..19usize {
                        for c in 0..40usize {
                            let got = g.ds.r16(0x900C + (r * 0x50 + c * 2) as u16);
                            let (mr, mc) = if c == 0 && r >= 1 { (r - 1, l + 40) } else { (r, l + c) };
                            let o = (mr * 0x258 + mc) * 2;
                            let want = u16::from_le_bytes([map[o], map[o + 1]]);
                            assert_eq!(got, want, "frame {frame} L {l} r {r} c {c}");
                        }
                    }
                    cache_checked += 1;
                }
            });
            if f.exit.is_some() {
                println!("離開 {:?}，計數 {}，hp {} {}，1CFDD {}", f.exit, g.ds.r16(a(0x1CFE2)), g.ds.r16(a(0x1CFDE)), g.ds.r16(a(0x1CFE0)), g.ds.r8(a(0x1CFDD)));
                break;
            }
        }
        let normal = samples.iter().filter(|&&p| p == 0x13B5F).count();
        let boss = samples.iter().filter(|&&p| p == 0x13BBB).count();
        println!("取樣 普通段 {normal} 頭目段 {boss}，最後 word_25BC6 = {}", g.ds.r16(a(0x25BC6)));
        println!("G4 快取檢查 {cache_checked} 個取樣");
        assert!(normal > 1000 && boss > 100 && cache_checked > 900);
    }
}
