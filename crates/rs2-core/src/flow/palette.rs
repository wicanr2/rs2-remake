//! 調色盤。每個場景的調色盤是資料段的一張 65 位元組表：17 位元組的 EGA 屬性表接 48 位元組的 DAC 表。
//! 設定函式把 DAC 表寫兩次（起點 0 與 30h），EGA 屬性表只附在第一次。

use crate::draw::DrawCmd;
use crate::game::Core;
use crate::mirror::Palette;

/// 一個調色盤函式對 DAC 的第二次寫入的起點。
const SECOND_DAC_START: u8 = 0x30;
/// DAC 表的位元組數：16 個暫存器 × 3。
const DAC_BYTES: usize = 48;
/// EGA 屬性表的位元組數。
const ATTR_BYTES: usize = 17;
/// 屬性表在 DAC 表之前這麼多位元組。
const ATTR_BEFORE_DAC: u16 = 0x11;

impl Core {
    /// 設場景的調色盤：DAC 表寫兩次，EGA 屬性表跟著第一次。
    pub(crate) fn load_palette(&mut self, palette: Palette) {
        let (table, attr) = (palette.table_offset(), palette.attr_offset());
        self.set_dac_attr(0, table, attr);
        self.set_dac(SECOND_DAC_START, table);
    }

    /// 把 DAC 表（資料段 `table` 起 48 位元組）寫進 DAC 暫存器，起點 `start`。屬性表在 `table` 之前 11h 位元組。
    pub(crate) fn set_dac(&mut self, start: u8, table: u16) {
        self.set_dac_attr(start, table, table.wrapping_sub(ATTR_BEFORE_DAC));
    }

    /// 同 [`Core::set_dac`]，屬性表位址另給（`loader` 的 DAC 與 `title_d` 共用，屬性表在別處）。
    /// `start` 為 0 時附上屬性表，原版的 EGA 分支只呼叫一次 INT 10h AX=1002h。
    pub(crate) fn set_dac_attr(&mut self, start: u8, table: u16, attr: u16) {
        let rgb = self.table_bytes::<DAC_BYTES>(table);
        self.draws.push(DrawCmd::SetDac { start, rgb });
        if start == 0 {
            let regs = self.table_bytes::<ATTR_BYTES>(attr);
            self.draws.push(DrawCmd::EgaAttr { regs });
        }
    }

    /// 同 [`Core::set_dac`]，DAC 表的前幾個位元組由呼叫端給（第二關淡出的工作表在結構裡），其餘讀資料段。
    pub(crate) fn set_dac_overlay(&mut self, start: u8, table: u16, overlay: &[u8]) {
        let mut rgb = self.table_bytes::<DAC_BYTES>(table);
        rgb[..overlay.len()].copy_from_slice(overlay);
        self.draws.push(DrawCmd::SetDac { start, rgb });
        if start == 0 {
            let regs = self.table_bytes::<ATTR_BYTES>(table.wrapping_sub(ATTR_BEFORE_DAC));
            self.draws.push(DrawCmd::EgaAttr { regs });
        }
    }
}
