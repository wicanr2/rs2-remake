//! 主迴圈與幀流程、進入關卡、熱鍵、血條、INT 9。對照 `sub_10CD0` 與 `sub_10EEA`。

use crate::draw::{DrawCmd, ShtGroup};
use crate::dseg::a;
use crate::game::{Exit, Frame, Game, Input};

/// 取樣點回呼：`(狀態, 取樣位址)`。第一關是 `10F98h`、`10FE3h`。
pub type Sample<'a> = &'a mut dyn FnMut(&Game, u32);

impl Game {
    /// `10CD0h` 至 `10D11h`：進入第一關。檔案載入由接收端負責，這裡保留順序與 `dseg` 寫入。
    pub(crate) fn enter_stage(&mut self) {
        self.s18812();
        self.s18819();
        self.sht_group = ShtGroup::Bh1;
        self.s10b65();
        self.s18819();
        self.sht_group = ShtGroup::Sht1;
        self.ds.w8(a(0x1CFDB), b'A');
        self.s11112();
        self.s10e33();
        self.ds.w8(a(0x1CFAD), b'N');
        self.ds.w16(a(0x20B7A), 0);
    }

    /// 一個邏輯幀，等於主迴圈的一半（`10D29h` 至 `10D8Bh`）。
    pub fn step(&mut self, input: &Input, on_sample: Sample) -> Frame {
        if let Some(e) = self.finished {
            return Frame { draws: Vec::new(), exit: Some(e), paused: false };
        }
        for &sc in input.scancodes {
            self.int9(sc);
        }
        if self.paused {
            // sub_10ECB 的等待迴圈：等 A8CCh 離開 2。
            if self.ds.r8(0xA8CC) == 2 {
                return Frame { draws: self.take_draws(), exit: None, paused: true };
            }
            self.ds.w8(0xA8CC, 0);
            self.ds.w8(0xA862, 0);
            self.paused = false;
            if let Some(e) = self.round_end() {
                self.finished = Some(e);
                return Frame { draws: self.take_draws(), exit: Some(e), paused: false };
            }
        }
        let mut exit = None;
        if !self.second_page {
            self.s19a37();
            self.s10fea();
            self.s10eea(on_sample);
            self.second_page = true;
        } else {
            self.s19a88();
            self.s10eea(on_sample);
            self.second_page = false;
            if self.ds.r8(0xA8CC) == 1 {
                // sub_10ECB：Esc 暫停，下一步起等待。
                self.ds.w8(0xA8CC, 2);
                self.paused = true;
                return Frame { draws: self.take_draws(), exit: None, paused: false };
            }
            exit = self.round_end();
        }
        self.finished = exit;
        Frame { draws: self.take_draws(), exit, paused: false }
    }

    /// `10D3Bh` 至 `10D8Bh`：熱鍵、存活、過關連爆。
    fn round_end(&mut self) -> Option<Exit> {
        self.s10d99();
        self.s10d8d();
        match self.r.dx as u8 {
            b'E' => return Some(self.leave(Exit::End)),
            b'Q' => return Some(self.leave(Exit::Quit)),
            b'C' => return Some(self.leave(Exit::Clear)),
            _ => {}
        }
        if self.ds.r8(a(0x1CFAD)) == b'F' {
            let n = self.ds.r8(a(0x1CFAE)).wrapping_add(1);
            self.ds.w8(a(0x1CFAE), n);
            self.r.dx = b'C' as u16;
            let v = self.ds.r16(a(0x20B7A)).wrapping_sub(3);
            self.ds.w16(a(0x20B7A), v);
            if n > 0x14 {
                return Some(self.leave(Exit::Clear));
            }
            self.ds.w16(a(0x20B78), 0x65);
            if self.ds.r16(a(0x26872)) != 0 {
                self.ds.w16(a(0x20B78), 0x45);
            }
            self.s17a3a();
        }
        None
    }

    /// `10D59h`：音樂停止不在本切片。
    fn leave(&mut self, e: Exit) -> Exit {
        self.ds.w8(a(0x2739A), b'N');
        e
    }

    /// `sub_10FEA`。`3ECh` 在進入關卡前已是 `Y`，回標題的分支不會走到。
    fn s10fea(&mut self) {
        let v = self.ds.r16(0x1B02);
        if v >= 0xD2 {
            self.ds.w16(0x1B02, v.wrapping_sub(1));
        }
    }

    /// `sub_10EEA`。
    fn s10eea(&mut self, on_sample: Sample) {
        if self.ds.r8(0x041C) == b'Y' && self.ds.r16(0x0422) == 0x28A {
            self.s10e33();
            self.ds.w16(0x0422, 0);
        }
        if self.ds.r16(0x0422) == 1 {
            self.s110e3();
        }
        if self.ds.r16(0x0422) == 0x26C {
            self.s11141();
        }
        if self.ds.r16(0x0422) <= 0x26C {
            self.s10f41(on_sample);
            return;
        }
        self.s10f9f(on_sample);
        if self.ds.r16(0x0422) == 0x26E {
            self.s10c3d();
            return;
        }
        if self.ds.r16(0x0422) == 0x28A {
            self.ds.w16(0x0422, 0x288);
        }
    }

    /// `sub_10F41`：普通段的一幀。
    fn s10f41(&mut self, on_sample: Sample) {
        self.s17d82();
        self.s15c3c();
        self.s15d8c();
        self.s14458();
        self.s16302();
        self.s15c92();
        self.s164bb();
        self.s17361();
        self.s1673e();
        self.s14704();
        self.s14f92();
        self.s15232();
        self.s164d7();
        self.s17c9a();
        self.s1100a();
        let c = self.ds.r16(0x0422).wrapping_add(1);
        self.ds.w16(0x0422, c);
        // sub_1117F 只有 MGA 分支。
        self.s162b1();
        self.s178bb();
        self.ds.w16(0x9CC0, 0x1B);
        self.ds.w16(0x9CC2, 0xAC);
        self.ds.w16(0x9CC8, 0x1A);
        self.clear_obj();
        self.s18c0a();
        self.ds.w8(0x9C8F, 4);
        self.s18672();
        on_sample(self, 0x10F98);
    }

    /// `sub_10F9F`：頭目段的一幀。
    fn s10f9f(&mut self, on_sample: Sample) {
        self.s17d82();
        self.s15c3c();
        self.s15d8c();
        self.s14458();
        self.s15906();
        self.s164bb();
        self.s164d7();
        self.s15c92();
        self.s17361();
        self.s1673e();
        self.s14704();
        self.s14f92();
        self.s15232();
        self.s17c9a();
        self.s1100a();
        let c = self.ds.r16(0x0422).wrapping_add(1);
        self.ds.w16(0x0422, c);
        self.s162b1();
        self.s178bb();
        on_sample(self, 0x10FE3);
    }

    /// `sub_10D8D`。
    fn s10d8d(&mut self) {
        if self.ds.r8(a(0x1CFDD)) == 0 {
            self.r.dx = b'E' as u16;
        }
    }

    /// `sub_10D99`：熱鍵。Ctrl+D（`10AD4h`，音樂）與 Ctrl+W、Alt+W（`sub_1BDF7`，傾印畫面）不在本切片。
    fn s10d99(&mut self) {
        // sub_10DF8：只保留清旗標的部分。
        if self.ds.r8(a(0x274C3)) == 1 && self.ds.r8(a(0x2749C)) == 1 {
            self.ds.w8(a(0x2749C), 0);
            self.ds.w8(a(0x274C3), 0);
        }
        self.r.dx = b'N' as u16;
        if self.ds.r8(a(0x274A8)) == 1 {
            if self.ds.r8(a(0x2749E)) == 1 {
                self.ds.w8(0x041C, b'N');
                self.r.dx = b'E' as u16;
            }
            if self.ds.r8(a(0x2749B)) == 1 {
                self.r.dx = b'Q' as u16;
                return;
            }
            if self.ds.r8(a(0x274AB)) == 1 {
                self.r.dx = b'N' as u16;
                return;
            }
            if self.ds.r8(a(0x274B9)) == 1 {
                self.r.dx = b'C' as u16;
            }
            return;
        }
        if self.ds.r8(0x041C) == b'Y' && self.ds.r8(a(0x27422)) != 0 {
            self.r.dx = b'E' as u16;
        }
    }

    /// `sub_10E33`。
    pub(crate) fn s10e33(&mut self) {
        self.s18599();
        self.ds.w16(0x9006, 0);
        self.ds.w16(0x9008, 0);
        self.ds.w16(0x0422, 0);
        self.ds.w16(0x1B02, 0x10E);
        self.ds.w16(0x042A, 0);
        self.r.ax = 0;
        self.ds.fill16(0x2294, 0x10, 0);
        self.ds.fill16(0x32B3, 0x14, 0);
        self.ds.fill16(0x2607, 0x14, 0);
        self.r.cx = 0;
        self.r.di = 0x2607 + 0x28;
        for o in [0xA91Bu16, 0xA916, 0xA918, 0xA913, 0xA901] {
            self.ds.w8(o, 0);
        }
        self.ds.w16(0x042A, 0);
        self.s14f47();
        self.s158ba();
        self.ds.w16(0x9004, 0);
        self.ds.w16(0x25E8, 0);
        self.ds.w8(0x03ED, b'N');
        self.ds.w8(0x03EE, 0);
        for o in [0xA8E8u16, 0xA8DE, 0xA8EB, 0xA8DC, 0xA8F9] {
            self.ds.w8(o, 0);
        }
    }

    /// `sub_10B65`：把 `BH1` 的五層寫進暫存區 A。
    fn s10b65(&mut self) {
        self.ds.w16(a(0x2685C), 0);
        self.s18812();
        let put = |g: &mut Game, x: u16, y: u16, i: u16| {
            g.ds.w16(a(0x26880), x);
            g.ds.w16(a(0x26882), y);
            g.ds.w16(a(0x26888), i);
            g.s19adc();
        };
        put(self, 0, 0, 0);
        put(self, 0x28, 0, 0);
        put(self, 0, 0x28, 2);
        put(self, 0x28, 0x28, 2);
        self.ds.w16(a(0x26880), 0);
        for _ in 0..0x0A {
            self.ds.w16(a(0x26882), 0x6E);
            self.ds.w16(a(0x26888), 1);
            self.s19adc();
            let x = self.ds.r16(a(0x26880)).wrapping_add(4);
            self.ds.w16(a(0x26880), x);
        }
        put(self, 0, 0xC8, 3);
        put(self, 0x28, 0xC8, 3);
        put(self, 0, 0xF3, 4);
        put(self, 0x28, 0xF3, 4);
    }

    /// `sub_10C3D`：清暫存區 B，把 `SHT1` 第 33、34、35 項畫進去。
    fn s10c3d(&mut self) {
        self.clear_obj();
        self.s1152d();
        self.ds.w16(a(0x2685C), 0x8000);
        for (x, y, i) in [(0u16, 0u16, 0x21u16), (0x12, 0x22, 0x22), (0x26, 0x0D, 0x23)] {
            self.ds.w16(a(0x26880), x);
            self.ds.w16(a(0x26882), y);
            self.ds.w16(a(0x26888), i);
            self.s19adc();
        }
    }

    fn set_dac(&mut self, start: u8, table: u16) {
        let mut rgb = [0u8; 48];
        for (i, v) in rgb.iter_mut().enumerate() {
            *v = self.ds.r8(table.wrapping_add(i as u16));
        }
        self.draws.push(DrawCmd::SetDac { start, rgb });
    }

    /// `sub_11112`：`loader` 調色盤。純 EGA 分支不在本切片。
    fn s11112(&mut self) {
        self.set_dac(0, 0x5A9);
        self.set_dac(0x30, 0x5A9);
    }

    /// `sub_110E3`：`stage1` 調色盤。
    fn s110e3(&mut self) {
        self.set_dac(0, 0x4FD);
        self.set_dac(0x30, 0x4FD);
    }

    /// `sub_11141`：`stage1_boss` 調色盤。
    fn s11141(&mut self) {
        self.set_dac(0, 0x53E);
        self.set_dac(0x30, 0x53E);
    }

    /// `sub_1100A`：幀計數。
    fn s1100a(&mut self) {
        self.ds.w16(0x9CCA, 0x12);
        self.ds.w16(0x9CCE, 0xBF);
        self.r.cx = self.ds.r16(0x0422);
        self.clear_obj();
        self.s1890d();
    }

    /// `sub_110A4`：固定亂數表，全域索引 `42Ah`。回傳 `ax`，改動 `bx`。
    pub(crate) fn s110a4(&mut self) {
        let mut ax = self.ds.r16(0x042A).wrapping_add(1);
        if ax >= 0x4B {
            ax = 0;
        }
        self.ds.w16(0x042A, ax);
        self.r.bx = ax << 1;
        self.r.ax = self.ds.r16(self.r.bx.wrapping_add(0x042C));
    }

    /// `sub_1130D`：玩家 2 的血條。
    pub(crate) fn s1130d(&mut self) {
        self.ds.w16(a(0x26880), 6);
        self.ds.w16(a(0x26882), 0xBE);
        self.r.ax = self.ds.r16(a(0x1CFE0)) & 0x00FF;
        for _ in 0..8 {
            let saved = self.r.ax;
            self.s1135f();
            self.r.ax = saved >> 1;
            let x = self.ds.r16(a(0x26880)).wrapping_add(1);
            self.ds.w16(a(0x26880), x);
        }
    }

    /// `sub_11336`：玩家 1 的血條。
    pub(crate) fn s11336(&mut self) {
        self.ds.w16(a(0x26880), 0x22);
        self.ds.w16(a(0x26882), 0xBE);
        self.r.ax = self.ds.r16(a(0x1CFDE)) & 0x00FF;
        for _ in 0..8 {
            let saved = self.r.ax;
            self.s1135f();
            self.r.ax = saved >> 1;
            let x = self.ds.r16(a(0x26880)).wrapping_sub(1);
            self.ds.w16(a(0x26880), x);
        }
    }

    /// `sub_1135F`：血條一格，`al` 的位元 0 決定亮暗。
    fn s1135f(&mut self) {
        self.s11375();
        self.r.dx = if self.r.al() & 1 == 0 { 0 } else { 0x0B };
        self.clear_obj();
        self.s1929f();
    }

    /// `sub_11375`：校驗 `dseg:7F4h` 起 400 個字組。通過後把自己改成 `retn`，所以只有第一次會改寫 `ax`。
    /// 不通過時原版跳到 `sub_113B2`，輸入雜湊已驗過，正常流程走不到。
    fn s11375(&mut self) {
        if self.s11375_patched {
            return;
        }
        // sub_12F95
        let mut si = 0x07F4u16;
        let mut dx = 0x64u16;
        for _ in 0..0x190 {
            dx = dx.wrapping_add(self.ds.r16(si));
            si = si.wrapping_add(2);
        }
        self.r.si = si;
        self.r.cx = 0;
        self.r.di = 0;
        self.r.ax = dx;
        self.r.dx = 0x59;
        if self.r.ax != 0x227C {
            return;
        }
        self.r.set_al(0xC3);
        self.s11375_patched = true;
        self.ds.w8(a(0x1CFAA), self.r.dl());
    }

    /// INT 9（`1A83Dh`）。
    pub fn int9(&mut self, sc: u8) {
        self.ds.w8(a(0x2742C), sc);
        if sc & 0x80 == 0 {
            let ascii = self.ds.r8(0xA86Du16.wrapping_add(sc as u16));
            self.ds.w8(a(0x27422), ascii);
            self.ds.w8(0xA8CBu16.wrapping_add(sc as u16), 1);
        } else {
            self.ds.w8(a(0x2742C), 0);
            self.ds.w8(0xA8CBu16.wrapping_add((sc & 0x7F) as u16), 2);
        }
        self.s1a8f4();
        self.s1a9b5();
        if self.ds.r8(a(0x274A8)) == 1 {
            if self.ds.r8(a(0x274AF)) == 1 {
                self.ds.w8(a(0x274AF), 0);
                self.ds.w8(a(0x1DAAC), b'Y');
            }
            if self.ds.r8(a(0x274B0)) == 1 {
                self.ds.w8(a(0x274B0), 0);
                self.ds.w8(a(0x1DAAC), b'N');
            }
        }
        // sub_1A891：音效開關。
        if self.ds.r16(a(0x2739B)) >= 5
            && self.ds.r8(a(0x2739A)) != b'N'
            && self.ds.r8(a(0x274AA)) == 1
        {
            let v = self.ds.r8(a(0x2684E)) ^ 1;
            self.ds.w8(a(0x2684E), v);
            self.ds.w8(a(0x274AA), 0);
            self.ds.w8(a(0x27422), 0);
            self.ds.w16(a(0x2739B), 0);
        }
    }

    /// `sub_1A8F4`：開發者秘技序列。
    fn s1a8f4(&mut self) {
        if self.ds.r8(a(0x274C3)) == 1 {
            self.ds.w8(a(0x274C3), 0);
            self.ds.w16(a(0x2739F), 0);
            self.ds.w16(a(0x273A1), 0);
            self.ds.w16(a(0x273A3), 0);
            self.ds.w16(a(0x273A5), 0x4E);
        }
        for (key, count) in [(0x2748Du32, 0x2739Fu32), (0x2748E, 0x273A1), (0x2748F, 0x273A3)] {
            if self.ds.r8(a(key)) == 1 {
                self.ds.w8(a(key), 0);
                let v = self.ds.r16(a(count)).wrapping_add(1);
                self.ds.w16(a(count), v);
            }
        }
        if self.ds.r8(a(0x274D7)) == 1 {
            self.ds.w8(a(0x274D7), 0);
            if self.ds.r16(a(0x2739F)) == 1 && self.ds.r16(a(0x273A1)) == 2 && self.ds.r16(a(0x273A3)) == 3 {
                self.ds.w16(a(0x2739D), 0x59);
            }
        }
        if self.ds.r8(a(0x274C4)) == 1 {
            self.ds.w8(a(0x274C4), 0);
            if self.ds.r16(a(0x2739D)) == 0x59 {
                self.ds.w16(a(0x1CFDE), 0xFF);
                self.ds.w16(a(0x1CFE0), 0xFF);
                self.ds.w16(a(0x2739D), 0x4E);
            }
        }
        if self.ds.r8(a(0x274C2)) == 1 {
            self.ds.w8(a(0x274C2), 0);
            if self.ds.r16(a(0x2739D)) == 0x59 {
                self.ds.w16(a(0x273A5), 0x59);
            }
        }
    }

    /// `sub_1A9B5`：把按鍵狀態轉成開火與轉向旗標。
    fn s1a9b5(&mut self) {
        if self.ds.r8(a(0x274C1)) == 2 {
            self.ds.w8(a(0x273A7), b'Y');
        }
        if self.ds.r8(a(0x274B5)) == 2 {
            self.ds.w8(a(0x273A8), b'Y');
        }
        if self.ds.r8(a(0x274C1)) == 1 {
            self.ds.w8(a(0x273A9), b'Y');
        }
        if self.ds.r8(a(0x274B5)) == 1 {
            self.ds.w8(a(0x273AB), b'Y');
        }
        if self.ds.r8(a(0x274C0)) == 1 {
            self.ds.w8(a(0x273AA), b'Y');
            self.ds.w8(a(0x274C0), 2);
        }
        if self.ds.r8(a(0x274B7)) == 1 {
            self.ds.w8(a(0x273AC), b'Y');
            self.ds.w8(a(0x274B7), 2);
        }
    }
}
