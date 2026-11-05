//! 繪製與喇叭函式的樁：記下命令，並保留它們對 `dseg` 的寫入。
//! 真正的 E 模式 blit 在 `rs2-gfx`。

use crate::draw::{BlitCmd, DrawCmd, Op};
use crate::dseg::a;
use crate::game::Game;

impl Game {
    fn emit(&mut self, op: Op) {
        let d = &self.ds;
        let cmd = BlitCmd {
            op,
            x: d.r16(a(0x26880)),
            y: d.r16(a(0x26882)),
            index: d.r16(a(0x26888)),
            page: d.r16(a(0x26872)),
            scratch: d.r16(a(0x2685C)),
            src_x: d.r16(a(0x26864)),
            src_y: d.r16(a(0x26866)),
            shift: d.r16(a(0x26868)),
            cx: self.r.cx,
            dx: self.r.dx,
            text_x: d.r16(a(0x2688A)),
            text_y: d.r16(a(0x2688E)),
            sht_group: self.sht_group,
            obj: self.cur_obj,
        };
        self.draws.push(DrawCmd::Blit(cmd));
    }

    pub(crate) fn s18c0a(&mut self) {
        self.emit(Op::S18C0A)
    }
    pub(crate) fn s18ce2(&mut self) {
        self.emit(Op::S18CE2)
    }
    pub(crate) fn s18d46(&mut self) {
        self.emit(Op::S18D46)
    }
    pub(crate) fn s18d89(&mut self) {
        self.emit(Op::S18D89)
    }
    pub(crate) fn s18dcc(&mut self) {
        self.emit(Op::S18DCC)
    }
    pub(crate) fn s18e30(&mut self) {
        self.emit(Op::S18E30)
    }
    pub(crate) fn s18e73(&mut self) {
        self.emit(Op::S18E73)
    }
    pub(crate) fn s18eb6(&mut self) {
        self.emit(Op::S18EB6)
    }
    pub(crate) fn s18f1a(&mut self) {
        self.emit(Op::S18F1A)
    }
    pub(crate) fn s18f5d(&mut self) {
        self.emit(Op::S18F5D)
    }
    pub(crate) fn s18fc1(&mut self) {
        self.emit(Op::S18FC1)
    }
    pub(crate) fn s19025(&mut self) {
        self.emit(Op::S19025)
    }
    pub(crate) fn s19adc(&mut self) {
        self.emit(Op::S19ADC)
    }
    pub(crate) fn s19ba4(&mut self) {
        self.emit(Op::S19BA4)
    }
    pub(crate) fn s19cf9(&mut self) {
        self.emit(Op::S19CF9)
    }
    pub(crate) fn s19d08(&mut self) {
        self.emit(Op::S19D08)
    }
    pub(crate) fn s19da8(&mut self) {
        self.emit(Op::S19DA8)
    }
    pub(crate) fn s1a09f(&mut self) {
        self.emit(Op::S1A09F)
    }
    pub(crate) fn s1a287(&mut self) {
        self.emit(Op::S1A287)
    }
    pub(crate) fn s1929f(&mut self) {
        self.emit(Op::S1929F)
    }
    pub(crate) fn s1152d(&mut self) {
        self.emit(Op::S1152D)
    }

    /// `sub_1890D`：把 `cx` 轉成 5 位數字寫進 `268A1h` 起，再畫出來。
    pub(crate) fn s1890d(&mut self) {
        let mut v = self.r.cx;
        for (i, d) in [10000u16, 1000, 100, 10].into_iter().enumerate() {
            self.ds.w8(a(0x268A5) - i as u16, (v / d) as u8 + 0x30);
            v %= d;
        }
        self.ds.w8(a(0x268A1), v as u8 + 0x30);
        self.emit(Op::S1890D);
    }

    /// `sub_18599`：清第三關地形格 G4。
    pub(crate) fn s18599(&mut self) {
        self.ds.fill16(0x900C, 0x320, 0);
    }

    pub(crate) fn s18812(&mut self) {
        self.ds.w16(a(0x26872), 0);
    }

    pub(crate) fn s18819(&mut self) {
        self.ds.w16(a(0x26872), 0x8000);
    }

    /// `sub_19A37`：顯示頁 0，改畫頁 1。
    pub(crate) fn s19a37(&mut self) {
        self.s187c3();
        self.ds.w16(a(0x26872), 0x8000);
    }

    /// `sub_19A88`：顯示頁 1，改畫頁 0。
    pub(crate) fn s19a88(&mut self) {
        self.ds.w16(a(0x26872), 0);
    }

    /// `sub_187C3`：同步音效開關狀態。實際的音樂停止與自我修改喇叭碼不在本切片。
    fn s187c3(&mut self) {
        if self.ds.r8(a(0x2684E)) & 1 != 0 {
            self.ds.w8(a(0x1CBCE), b'N');
        } else {
            self.ds.w8(a(0x1CBCE), b'Y');
        }
    }

    /// `sub_18672`：依 `byte_2684F` 設喇叭參數。沒有任何一個位元成立時不動 `cx`。
    pub(crate) fn s18672(&mut self) {
        let f = self.ds.r8(a(0x2684F));
        let (w52, w54, w58, w50, w56) = if f & 1 != 0 {
            (0x32, 5, 0x32, 0, 0x1F40) // sub_186BA
        } else if f & 2 != 0 {
            (2, 1, 2, 7, 0x1F4) // sub_18717
        } else if f & 4 != 0 {
            (2, 1, 2, 0x1E, 0x3E8) // sub_186F8
        } else if f & 8 != 0 {
            (3, 2, 3, 0xC8, 0x2EE0) // sub_186D9
        } else {
            return;
        };
        self.ds.w16(a(0x26852), w52);
        self.ds.w16(a(0x26854), w54);
        self.ds.w16(a(0x26858), w58);
        self.ds.w16(a(0x26850), w50);
        self.ds.w16(a(0x26856), w56);
        self.r.cx = 0x2000;
    }
}
