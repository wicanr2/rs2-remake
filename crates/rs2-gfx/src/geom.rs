//! 顯示頁與暫存區的版面（規格 005「戰場大小」）。
//!
//! 原版 E 模式的顯示記憶體是 64 KB 的位址空間：頁 0（200 列 × 40 位元組）、暫存區 A（每列 80 位元組）、
//! 頁 1、暫存區 B，兩組各佔 8000h。戰場加大時遊戲區變寬（欄數 48 或 60，一欄 8 像素，高仍是 200 列），
//! 暫存區的列寬放大到 120 位元組（背景圖的週期是 40 位元組，視窗最多從相位 39 取 60 欄，要放得下三份），
//! 兩組各佔 10000h，位址用 17 位元。欄數 40 時每個值都等於原版，位址運算與 16 位元環繞逐位元相同。
//!
//! 位址運算一律先取 `u32` 再 [`Geom::wrap`]。命令裡的 16 位元欄位分兩種：頁與暫存區選擇（0 或 8000h）用
//! [`Geom::seg`] 換成這個版面的組位移；相對量（欄、位移、來源欄）用 [`Geom::rel`] 做有號延伸，
//! 欄數 40 時延伸後取 16 位元等於原版的環繞加法。

use rs2_game::BlitCmd;

/// 原版的欄數。
pub const ORIGINAL_COLS: u16 = 40;
/// 戰場加大時暫存區每列的位元組數。
const WIDE_SCRATCH_COLS: u16 = 120;

/// 顯示頁與暫存區的版面，由欄數決定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geom {
    cols: u16,
}

impl Default for Geom {
    fn default() -> Geom {
        Geom::ORIGINAL
    }
}

impl Geom {
    /// 原版：40 欄。
    pub const ORIGINAL: Geom = Geom { cols: ORIGINAL_COLS };

    pub const fn new(cols: u16) -> Geom {
        Geom { cols }
    }

    /// 顯示頁每列的位元組數，也是遊戲區的欄數。
    pub const fn cols(self) -> u16 {
        self.cols
    }

    /// 不是原版的 40 欄。
    pub const fn wide(self) -> bool {
        self.cols != ORIGINAL_COLS
    }

    /// 暫存區每列的位元組數：原版 80，戰場加大 120。
    pub const fn scols(self) -> u16 {
        if self.wide() { WIDE_SCRATCH_COLS } else { 0x50 }
    }

    /// 輸出的寬（像素）：一欄 8 像素。
    pub const fn px_width(self) -> usize {
        self.cols as usize * 8
    }

    /// 一個顯示頁的位元組數：200 列。
    pub const fn page_bytes(self) -> u32 {
        200 * self.cols as u32
    }

    /// 暫存區 A 的起點：緊接在頁 0 之後（原版 `1F40h`）。
    pub const fn scratch_base(self) -> u32 {
        self.page_bytes()
    }

    /// 第二組（頁 1 與暫存區 B）相對第一組的位移：原版 8000h，戰場加大 10000h。
    pub const fn set1(self) -> u32 {
        if self.wide() { 0x1_0000 } else { 0x8000 }
    }

    /// 位址遮罩：原版 16 位元，戰場加大 17 位元。
    pub const fn mask(self) -> u32 {
        if self.wide() { 0x1_FFFF } else { 0xFFFF }
    }

    /// 位址空間的位元組數。
    pub const fn mem_bytes(self) -> usize {
        self.mask() as usize + 1
    }

    /// 清暫存區的位元組數：原版從 `1F40h` 起 `417Eh` 位元組（209 列又 46 位元組）。
    /// 戰場加大時同樣是 209 列又 46 位元組，列寬換成這個版面的。
    pub const fn scratch_clear_bytes(self) -> u32 {
        209 * self.scols() as u32 + 46
    }

    /// 位址取模。
    #[inline]
    pub const fn wrap(self, v: u32) -> u32 {
        v & self.mask()
    }

    /// 命令的頁（`word_26872`）或暫存區選擇（`word_2685C`）換成位址空間裡的組位移。
    /// 原版值就是 16 位元偏移（慣例上是 0 或 8000h，其他值照原樣）；戰場加大時只看位元 15。
    #[inline]
    pub const fn seg(self, v: u16) -> u32 {
        if self.wide() {
            if v & 0x8000 != 0 { self.set1() } else { 0 }
        } else {
            v as u32
        }
    }

    /// 相對量（欄、來源欄、位移）：有號延伸成 `u32`，之後以 [`Geom::wrap`] 環繞相加。
    #[inline]
    pub const fn rel(v: u16) -> u32 {
        v as i16 as i32 as u32
    }

    /// 顯示頁的列位址表（`dseg:A00Ah`）：列 0 至 199 是 `cols * 列`，第 200 項起接著暫存區列表。
    pub const fn disp_row(self, row: u16) -> u32 {
        if row < 200 { row as u32 * self.cols as u32 } else { self.scratch_row(row - 200) }
    }

    /// 暫存區列位址表（`dseg:A19Ah`）：起點加 `scols * 列`，列號先取 16 位元再乘，與原版的 `wrapping_mul` 同。
    pub const fn scratch_row(self, row: u16) -> u32 {
        self.wrap(self.scratch_base().wrapping_add(row as u32 * self.scols() as u32))
    }

    /// 暫存區來源指標：`(word_26866 的低位元組) * 列寬 + word_26864 + 起點 + word_26868 + word_2685C`。
    /// 原版用 `mul bl`，只取 `word_26866` 的低位元組。
    pub fn scratch_src(self, c: &BlitCmd) -> u32 {
        let mut a = (c.src_y & 0xFF) as u32 * self.scols() as u32;
        a = a.wrapping_add(Geom::rel(c.src_x)).wrapping_add(self.scratch_base());
        a = a.wrapping_add(Geom::rel(c.shift)).wrapping_add(self.seg(c.scratch));
        self.wrap(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::{disp_row, scratch_row, scratch_src};
    use rs2_game::{BegGroup, Op, ShtGroup};

    fn cmd(src_x: u16, src_y: u16, shift: u16, scratch: u16) -> BlitCmd {
        BlitCmd {
            op: Op::ScratchReplace,
            x: 0,
            y: 0,
            index: 0,
            page: 0,
            scratch,
            src_x,
            src_y,
            shift,
            cx: 0,
            dx: 0,
            text_x: 0,
            text_y: 0,
            sht_group: ShtGroup::Bh1,
            mask_group: ShtGroup::Sht1,
            beg_group: BegGroup::Beg1,
            stride_reg: None,
            obj: None,
        }
    }

    #[test]
    fn the_original_geometry_is_the_64k_space_with_the_original_row_tables() {
        let g = Geom::ORIGINAL;
        assert_eq!((g.cols(), g.scols(), g.scratch_base(), g.set1(), g.mask()), (40, 80, 0x1F40, 0x8000, 0xFFFF));
        assert_eq!(g.scratch_clear_bytes(), 0x417E);
        for row in (0..=u16::MAX).step_by(7) {
            assert_eq!(g.disp_row(row), u32::from(disp_row(row)), "顯示頁列表第 {row} 項");
            assert_eq!(g.scratch_row(row), u32::from(scratch_row(row)), "暫存區列表第 {row} 項");
        }
    }

    #[test]
    fn the_original_scratch_source_matches_the_sink_for_signed_and_wrapping_values() {
        let g = Geom::ORIGINAL;
        for src_x in [0u16, 5, 0x28, 0xFFFF, 0xFFF0, 0x8000] {
            for shift in [0u16, 3, 0x27, 0xFFFF, 0xFFD8, 0x7FFF] {
                for src_y in [0u16, 1, 0x6E, 0xF3, 0x1FF, 0xFFFF] {
                    for scratch in [0u16, 0x8000] {
                        let c = cmd(src_x, src_y, shift, scratch);
                        assert_eq!(g.scratch_src(&c), u32::from(scratch_src(&c)), "{src_x:#x} {src_y:#x} {shift:#x} {scratch:#x}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_wide_geometry_keeps_both_sets_inside_the_17_bit_space() {
        for cols in [48u16, 60] {
            let g = Geom::new(cols);
            assert!(g.wide());
            assert_eq!((g.scols(), g.set1(), g.mask()), (120, 0x10000, 0x1FFFF));
            // 一組 = 頁 + 310 列暫存區，放得進 10000h。
            let top = g.scratch_row(309) + 120;
            assert!(top <= g.set1(), "cols {cols}：暫存區 B 的終點 {top:#x} 超過組位移");
            assert_eq!(g.seg(0), 0);
            assert_eq!(g.seg(0x8000), 0x10000);
            assert_eq!(g.disp_row(199), 199 * u32::from(cols));
            assert_eq!(g.scratch_row(1), g.scratch_base() + 120);
            let c = cmd(3, 2, 0xFFFF, 0x8000);
            assert_eq!(g.scratch_src(&c), g.wrap(0x10000 + g.scratch_base() + 2 * 120 + 3 - 1));
        }
    }
}
