//! 主迴圈與幀流程、進入關卡、熱鍵、血條、INT 9。對照 `sub_10CD0` 與 `sub_10EEA`。

use crate::draw::{DrawCmd, ShtGroup};
use crate::dseg::a;
use crate::entity::Timetable;
use crate::game::{Core, Exit, Req};

impl Core {
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
        self.fx.reset_chain();
    }

    /// `10D29h` 至 `10D8Bh`：第一關主迴圈。回傳離開原因。
    pub(crate) async fn s10d29(&mut self) -> Exit {
        loop {
            self.s19a37();
            self.s10fea();
            self.s10eea().await;
            self.s19a88();
            self.port.borrow_mut().at_round_end = true;
            self.s10eea().await;
            self.port.borrow_mut().at_round_end = false;
            self.s10ecb().await;
            let out = self.round_end();
            if let Some(e) = out {
                return e;
            }
            self.suspend(Req::Checkpoint).await;
        }
    }

    /// `sub_10ECB`：Esc 暫停。按下後等到 `A8CCh` 離開 2。
    pub(crate) async fn s10ecb(&mut self) {
        if self.ds.r8(0xA8CC) != 1 {
            return;
        }
        self.ds.w8(0xA8CC, 2);
        while self.ds.r8(0xA8CC) == 2 {
            self.suspend(Req::Poll { paused: true }).await;
        }
        self.ds.w8(0xA8CC, 0);
        self.ds.w8(0xA862, 0);
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
            self.fx.shift_chain_left(3);
            if n > 0x14 {
                return Some(self.leave(Exit::Clear));
            }
            let page = self.ds.r16(a(0x26872));
            self.spawn_stage_clear_burst(page);
        }
        None
    }

    /// `10D59h`。
    fn leave(&mut self, e: Exit) -> Exit {
        self.ds.w8(a(0x2739A), b'N');
        self.s10b12();
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
    async fn s10eea(&mut self) {
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
            self.s10f41().await;
            return;
        }
        self.s10f9f().await;
        if self.ds.r16(0x0422) == 0x26E {
            self.s10c3d();
            return;
        }
        if self.ds.r16(0x0422) == 0x28A {
            self.ds.w16(0x0422, 0x288);
        }
    }

    /// `sub_10F41`：普通段的一幀。
    async fn s10f41(&mut self) {
        self.spawn_scheduled(Timetable::Stage1);
        self.spawn_scenery();
        self.update_enemies();
        self.update_player(0);
        self.s16302();
        self.draw_scenery();
        self.draw_enemies();
        self.update_enemy_bullets();
        self.update_player_bullets();
        self.draw_player(0);
        self.update_player(1);
        self.draw_player(1);
        self.draw_enemy_overlays();
        self.update_effects();
        self.s1100a();
        let c = self.ds.r16(0x0422).wrapping_add(1);
        self.ds.w16(0x0422, c);
        self.s1117f();
        self.clear_grid1();
        self.clear_grid2();
        self.ds.w16(0x9CC0, 0x1B);
        self.ds.w16(0x9CC2, 0xAC);
        self.ds.w16(0x9CC8, 0x1A);
        self.clear_obj();
        self.s18c0a();
        self.ds.w8(0x9C8F, 4);
        self.s18672();
        self.suspend(Req::Sample(0x10F98)).await;
        self.r.cx = 2;
        self.s11097().await;
    }

    /// `sub_10F9F`：頭目段的一幀。
    async fn s10f9f(&mut self) {
        self.spawn_scheduled(Timetable::Stage1);
        self.spawn_scenery();
        self.update_enemies();
        self.update_player(0);
        self.s15906();
        self.draw_enemies();
        self.draw_enemy_overlays();
        self.draw_scenery();
        self.update_enemy_bullets();
        self.update_player_bullets();
        self.draw_player(0);
        self.update_player(1);
        self.draw_player(1);
        self.update_effects();
        self.s1100a();
        let c = self.ds.r16(0x0422).wrapping_add(1);
        self.ds.w16(0x0422, c);
        self.s1117f();
        self.clear_grid1();
        self.clear_grid2();
        self.suspend(Req::Sample(0x10FE3)).await;
        self.r.cx = 2;
        self.s11097().await;
    }

    /// `sub_11097`：等 INT 8 累積到 `cx` 個 tick。模擬不花時間，所以進來時計數是上次歸零後的值。
    /// INT 9 落在這段等待裡，協程恢復時才處理。
    pub(crate) async fn s11097(&mut self) {
        let cx = self.r.cx;
        let mut ticks = 0;
        while self.ds.r16(0xA7F9) < cx {
            self.int8();
            ticks += 1;
        }
        self.ds.w16(0xA7F9, 0);
        self.suspend(Req::Wait { ticks }).await;
        self.suspend(Req::WaitDone { cx }).await;
    }

    /// 原版的忙等迴圈每轉一圈呼叫一次：過一個 tick，交回控制權等按鍵。
    pub(crate) async fn poll_tick(&mut self) {
        self.int8();
        self.suspend(Req::Poll { paused: false }).await;
    }

    /// `sub_10CD0`：第一關完整入口，含載入。回傳時 `dx` 是離開碼。
    pub(crate) async fn s10cd0(&mut self) {
        self.s19a37();
        self.s18812();
        self.s19a88();
        self.s18819();
        self.s1af3b();
        self.s10b65();
        self.s18819();
        self.s1ae3b();
        self.s1bed5();
        self.ds.w8(a(0x1CFDB), b'A');
        self.s18894();
        self.s11112();
        self.s10e33();
        self.ds.w8(a(0x1CFAD), b'N');
        self.fx.reset_chain();
        if self.ds.r8(a(0x1CFAB)) != b'N' {
            self.s1aa52();
            self.s10ad4();
            self.ds.w8(a(0x2739A), b'Y');
        }
        let e = self.s10d29().await;
        self.r.dx = match e {
            Exit::End => b'E',
            Exit::Quit => b'Q',
            Exit::Clear => b'C',
        } as u16;
    }

    /// INT 8（`1A7B5h`）對 `dseg` 的效果。反竄改分支在 `byte_273AD` = `Y` 時不執行，JOYMOUSE 輪詢不支援。
    pub(crate) fn int8(&mut self) {
        for o in [a(0x273B7), a(0x273B9), a(0x2739B)] {
            let v = self.ds.r16(o).wrapping_add(1);
            self.ds.w16(o, v);
        }
        let v = self.ds.r16(a(0x1D3A8)).wrapping_add(1);
        self.ds.w16(a(0x1D3A8), if v >= 0x19 { 0 } else { v });
        let v = self.ds.r16(a(0x1D3AE)).wrapping_add(1);
        self.ds.w16(a(0x1D3AE), if v >= 0x1E { 0 } else { v });
    }

    /// `sub_10D8D`。
    pub(crate) fn s10d8d(&mut self) {
        if !self.any_player_alive() {
            self.r.dx = b'E' as u16;
        }
    }

    /// `sub_10D99`：熱鍵。Ctrl+W、Alt+W（`sub_1BDF7`，傾印畫面）不在本切片。
    pub(crate) fn s10d99(&mut self) {
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
                self.s10ad4();
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
        self.en.release_all();
        self.eb.release_all();
        self.pb.release_all();
        self.r.cx = 0;
        self.r.di = 0x2607 + 0x28;
        for o in [0xA91Bu16, 0xA916, 0xA918, 0xA913, 0xA901] {
            self.ds.w8(o, 0);
        }
        self.ds.w16(0x042A, 0);
        self.place_players_for_stage();
        self.ds.w16(0x9004, 0);
        self.pb.seekers_in_flight = 0;
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
        let put = |g: &mut Core, x: u16, y: u16, i: u16| {
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

    pub(crate) fn set_dac(&mut self, start: u8, table: u16) {
        self.set_dac_attr(start, table, table.wrapping_sub(0x11));
    }

    /// `SetDac` 加上同一個調色盤函式的 EGA 屬性表（`attr`，17 位元組）。每個調色盤函式對 DAC 的兩次寫入（`start` 為 0 與 30h）
    /// 只在第一次附上屬性表，原版的 EGA 分支只呼叫一次 INT 10h AX=1002h。屬性表在 DAC 表之前 11h 位元組，只有 `loader`
    /// （DAC 與 `title_d` 共用 `5A9h`，屬性表在 `56Eh`）例外。
    pub(crate) fn set_dac_attr(&mut self, start: u8, table: u16, attr: u16) {
        let mut rgb = [0u8; 48];
        for (i, v) in rgb.iter_mut().enumerate() {
            *v = self.ds.r8(table.wrapping_add(i as u16));
        }
        self.draws.push(DrawCmd::SetDac { start, rgb });
        if start == 0 {
            let mut regs = [0u8; 17];
            for (i, v) in regs.iter_mut().enumerate() {
                *v = self.ds.r8(attr.wrapping_add(i as u16));
            }
            self.draws.push(DrawCmd::EgaAttr { regs });
        }
    }

    /// `sub_11112`：`loader` 調色盤。純 EGA 分支不在本切片。
    fn s11112(&mut self) {
        self.set_dac_attr(0, 0x5A9, 0x56E);
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
    pub(crate) fn s1100a(&mut self) {
        self.ds.w16(0x9CCA, 0x12);
        self.ds.w16(0x9CCE, 0xBF);
        self.r.cx = self.ds.r16(0x0422);
        self.clear_obj();
        self.s1890d();
    }

    /// 玩家（0 是玩家 1，1 是玩家 2）的血條，血量的低 8 位元由低到高各畫一格（原版 `sub_11336`、`sub_1130D`）。
    /// 玩家 1 的條在右側，x = 22h 往左畫；玩家 2 的在左側，x = 6 往右畫。y 都是 BEh。
    /// 只在受擊時重畫（`Core::take_hit`）。
    pub(crate) fn draw_health_bar(&mut self, who: usize) {
        let (first_x, step) = if who == 0 { (0x22u16, 1u16.wrapping_neg()) } else { (6, 1) };
        self.ds.w16(a(0x26880), first_x);
        self.ds.w16(a(0x26882), 0xBE);
        self.r.ax = self.pl.ship[who].hp & 0x00FF;
        for _ in 0..8 {
            let saved = self.r.ax;
            self.s1135f();
            self.r.ax = saved >> 1;
            let x = self.ds.r16(a(0x26880)).wrapping_add(step);
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
                self.refill_health();
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
