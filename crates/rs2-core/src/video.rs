//! 繪製與喇叭函式的樁：記下命令，並保留它們對 `dseg` 的寫入。
//! 真正的 E 模式 blit 在 `rs2-gfx`。

use crate::draw::{AudioCmd, BlitCmd, DrawCmd, Op};
use crate::dseg::a;
use crate::game::Core;

/// `word_2687E` 的記號：上一個繪製命令之後邏輯沒有寫過。形狀寬不會是這個值。
const STRIDE_UNTOUCHED: u16 = 0xFFFF;

impl Core {
    /// `sub_1117F`：只有 MGA（Hercules）有動作。命令只記頁偏移，不動 `word_2687E` 的記號，
    /// 所以 E 模式的其他命令與資料段完全不受影響。
    pub(crate) fn s1117f(&mut self) {
        let mut c = self.blit_cmd(Op::S1117F);
        c.stride_reg = None;
        c.obj = None;
        self.draws.push(DrawCmd::Blit(c));
    }

    fn emit(&mut self, op: Op) {
        let cmd = self.blit_cmd(op);
        // 繪製對這個字的寫入在接收端，核心不會更新。留記號，才分得出之後邏輯有沒有再寫。
        self.ds.w16(a(0x2687E), STRIDE_UNTOUCHED);
        self.draws.push(DrawCmd::Blit(cmd));
    }

    fn blit_cmd(&self, op: Op) -> BlitCmd {
        let d = &self.ds;
        BlitCmd {
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
            mask_group: self.mask_group,
            beg_group: self.beg_group,
            stride_reg: (d.r16(a(0x2687E)) != STRIDE_UNTOUCHED).then(|| d.r16(a(0x2687E))),
            obj: self.cur_obj,
        }
    }

    /// 寫繪製參數：欄 `word_26880`、像素列 `word_26882`、圖庫索引 `word_26888`。下一個繪製命令帶走它們。
    pub(crate) fn set_blit_params(&mut self, x: u16, y_px: u16, index: u16) {
        self.ds.w16(a(0x26888), index);
        self.ds.w16(a(0x26880), x);
        self.ds.w16(a(0x26882), y_px);
    }

    /// 邏輯寫 `word_2687E`（形狀的寬）。下一個繪製命令把它記成 `stride_reg`。
    pub(crate) fn set_stride_reg(&mut self, v: u16) {
        self.ds.w16(a(0x2687E), v);
    }

    /// 測試用：邏輯寫過、還沒被繪製命令帶走的形狀寬；沒有則 `None`。
    #[cfg(test)]
    pub(crate) fn pending_stride(&self) -> Option<u16> {
        let v = self.ds.r16(a(0x2687E));
        (v != STRIDE_UNTOUCHED).then_some(v)
    }

    /// 測試用：回到「上一個繪製命令之後邏輯沒有寫過」的狀態。
    #[cfg(test)]
    pub(crate) fn forget_stride(&mut self) {
        self.ds.w16(a(0x2687E), STRIDE_UNTOUCHED);
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
    pub(crate) fn s11504(&mut self) {
        self.emit(Op::S11504)
    }
    pub(crate) fn s18894(&mut self) {
        self.emit(Op::S18894)
    }
    pub(crate) fn s18c6e(&mut self) {
        self.emit(Op::S18C6E)
    }
    pub(crate) fn s19068(&mut self) {
        self.emit(Op::S19068)
    }
    pub(crate) fn s190cc(&mut self) {
        self.emit(Op::S190CC)
    }
    pub(crate) fn s19b40(&mut self) {
        self.emit(Op::S19B40)
    }
    pub(crate) fn s19c4f(&mut self) {
        self.emit(Op::S19C4F)
    }
    pub(crate) fn s19e66(&mut self) {
        self.emit(Op::S19E66)
    }
    pub(crate) fn s19f3b(&mut self) {
        self.emit(Op::S19F3B)
    }
    pub(crate) fn s19ff9(&mut self) {
        self.emit(Op::S19FF9)
    }
    pub(crate) fn s1a144(&mut self) {
        self.emit(Op::S1A144)
    }
    pub(crate) fn s1a1e6(&mut self) {
        self.emit(Op::S1A1E6)
    }
    pub(crate) fn s18ace(&mut self) {
        self.emit(Op::S18ACE)
    }

    /// `sub_18878`：map mask 回到 0Fh，GC 回到覆寫。
    pub(crate) fn s18878(&mut self) {
        self.draws.push(DrawCmd::SetMapMask { mask: 0x0F });
    }

    /// `sub_1882E`：`bl` 是 1、2、4、8，對應平面 3、2、1、0。
    pub(crate) fn s1882e(&mut self) {
        let bl = self.r.bl();
        let (bh, m) = if bl & 1 != 0 {
            (3, 8)
        } else if bl & 2 != 0 {
            (2, 4)
        } else if bl & 4 != 0 {
            (1, 2)
        } else {
            (0, 1)
        };
        self.r.set_bh(bh);
        self.r.set_bl(m);
        self.draws.push(DrawCmd::SetMapMask { mask: m });
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
        self.display_start = Some(0);
        self.ds.w16(a(0x26872), 0x8000);
    }

    /// `sub_19A88`：顯示頁 1，改畫頁 0。
    pub(crate) fn s19a88(&mut self) {
        self.display_start = Some(0x8000);
        self.ds.w16(a(0x26872), 0);
    }

    /// `10AD4h`：`byte_1CBCE` 是 `Y` 才播放。沒有 AdLib 時原版把這裡改成 `retn`，重製一律當作有。
    pub(crate) fn s10ad4(&mut self) {
        if self.ds.r8(a(0x1CBCE)) == b'Y' {
            self.audio.push(AudioCmd::Play(self.mix));
        }
    }

    /// `10B12h`：`byte_1CBCE` 是 `Y` 才停止。
    pub(crate) fn s10b12(&mut self) {
        if self.ds.r8(a(0x1CBCE)) == b'Y' {
            self.audio.push(AudioCmd::Stop);
        }
    }

    /// `sub_187C3`：同步音效開關。靜音時把 `sub_187B3`、`sub_187BB` 改成 `retn`，所以喇叭閘門不再動。
    fn s187c3(&mut self) {
        if self.ds.r8(a(0x2684E)) & 1 != 0 {
            if self.ds.r8(a(0x1CBCE)) != b'N' {
                self.audio.push(AudioCmd::Speaker(false));
                self.s10b12();
                self.ds.w8(a(0x1CBCE), b'N');
            }
        } else if self.ds.r8(a(0x1CBCE)) != b'Y' {
            self.audio.push(AudioCmd::Speaker(true));
            self.ds.w8(a(0x1CBCE), b'Y');
            self.s10ad4();
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

// 以下是第三關地圖捲動新增的顯示記憶體命令（尾端新增，語意見 `workspace/progress/ops-needed.md`）。
impl Core {
    /// 記下 `S17EBA` 至 `S180C6` 這類直接操作顯示記憶體的命令。參數照原版留在 `word_26880`、`word_2685C`、`word_26872`。
    pub(crate) fn emit_op(&mut self, op: Op) {
        self.emit(op)
    }
}
