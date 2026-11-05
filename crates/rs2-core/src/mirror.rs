//! 領域狀態與原版資料段位移之間的對應（`Mirror`）。
//!
//! 重寫的方法見 `docs/spec/004-rewrite-and-delivery.md`：一個領域一個領域從平坦的 `Dseg` 搬進有型別的結構。
//! 搬完的領域，狀態住在結構裡，`Core::ds` 中對應的位元組不再維護。全流程對拍要逐位元組比資料段，
//! 所以每個領域實作 [`Mirror`]：
//!
//! - `load`：從資料段傾印讀入。`Game::from_snapshot`、`Game::restart`、`poke` 之後用。
//! - `store`：寫回原版位移。交回控制權時（`Core::suspend`）、`Game.ds` 給對拍比較用。
//!
//! 規則：
//!
//! - 原版位移只准出現在這個檔案與註解。領域的邏輯程式不得有 `a(0x...)` 或資料段位移常數。
//! - `load` 與 `store` 必須互為反函數：任何資料段內容讀進再寫回，位元組不變。所以結構的欄位用原版的字組寬度，
//!   不做「合理化」的轉換（例如把指標換成列舉）。轉換留到表格階段。
//! - 領域內先寫後讀的工作變數不進結構，改成區域變數或參數，位移列進 `DS_EXEMPT`（`game.rs`）並寫明讀寫它的函式。
//! - 跨呼叫保留的殘值（例如特效的位置游標）是狀態，要進結構並 `store`。
//!
//! 本檔後半是「尚未搬走的共用資料段」：搬完的領域還要讀寫其他領域擁有的 `Dseg` 變數（唯讀表格、格網），
//! 一律經這裡的具名存取函式。那個領域搬完時，把存取函式換成真正的結構。

use crate::dseg::{Dseg, a};
use crate::fx::{Effect, Effects, POOL_SIZE, Script};
use crate::game::Core;

/// 一個領域的狀態與資料段之間的對應。
pub(crate) trait Mirror {
    /// 從資料段傾印讀入這個領域的全部狀態。
    fn load(&mut self, ds: &Dseg);
    /// 把這個領域的全部狀態寫回原版位移。
    fn store(&self, ds: &mut Dseg);
}

impl Core {
    /// 所有已搬走的領域從 `self.ds` 讀入。
    pub(crate) fn load_mirrors(&mut self) {
        self.fx.load(&self.ds);
    }

    /// 所有已搬走的領域寫回 `self.ds`。
    pub(crate) fn store_mirrors(&mut self) {
        self.fx.store(&mut self.ds);
    }
}

// ---- 特效池 ----

/// 過關連爆的起點欄，`word_20B7A`。關卡初始化寫 0，過關連爆每輪減 3。
const FX_CHAIN_BASE_X: u16 = a(0x20B7A);
/// 位置游標，`word_20B7C`、`word_20B7E`。生成與每格更新都會寫，單顆 `E` 沿用上一次的值。
const FX_CURSOR_X: u16 = a(0x20B7C);
const FX_CURSOR_Y: u16 = a(0x20B7E);
/// 池的三個平行陣列，各 30 個字組：x（`3FC2h`）、y（`3FFEh`）、腳本讀取位置（`403Ah`）。
const FX_POOL_X: u16 = 0x3FC2;
const FX_POOL_Y: u16 = 0x3FFE;
const FX_POOL_SCRIPT: u16 = 0x403A;

impl Mirror for Effects {
    fn load(&mut self, ds: &Dseg) {
        self.chain_base_x = ds.r16(FX_CHAIN_BASE_X);
        self.cursor = (ds.r16(FX_CURSOR_X), ds.r16(FX_CURSOR_Y));
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let o = (i * 2) as u16;
            *slot = Effect {
                x: ds.r16(FX_POOL_X + o),
                y: ds.r16(FX_POOL_Y + o),
                script_pos: ds.r16(FX_POOL_SCRIPT + o),
            };
        }
    }

    fn store(&self, ds: &mut Dseg) {
        ds.w16(FX_CHAIN_BASE_X, self.chain_base_x);
        ds.w16(FX_CURSOR_X, self.cursor.0);
        ds.w16(FX_CURSOR_Y, self.cursor.1);
        for (i, slot) in self.slots.iter().enumerate() {
            let o = (i * 2) as u16;
            ds.w16(FX_POOL_X + o, slot.x);
            ds.w16(FX_POOL_Y + o, slot.y);
            ds.w16(FX_POOL_SCRIPT + o, slot.script_pos);
        }
    }
}

const _: () = assert!(POOL_SIZE == 30, "池大小與原版三個陣列的長度一致");

// ---- 尚未搬走的共用資料段 ----

impl Script {
    /// 動畫表在資料段的位移。唯讀表格，表格階段改成解析 `S.EXE` 得到的結構。
    pub(crate) const fn table_offset(self) -> u16 {
        match self {
            Script::SparkRight => 0x4094,
            Script::SparkLeft => 0x40B2,
            Script::BigExplosion => 0x40D0,
            Script::Blast => 0x40EE,
            Script::Trail => 0x413C,
            Script::SeekerTrail => 0x4162,
            Script::PlayerDeath => 0x4188,
        }
    }
}

/// G3 格網（碰撞用的髒格，每列 50h 位元組，奇數位元組）的範圍。
const GRID3_START: u16 = 0x964C;
const GRID3_END: u16 = 0x9C3C;
/// 列起點表（`21C2h`）：每列一個字組。列 0 至 13h 從 0001h 起每列加 50h，列 14h 是越界列（0059h）。G3 與 G4 共用。
const GRID_ROW_TABLE: u16 = 0x21C2;
/// 列號（無號）大於等於這個值一律當作這一列。
const GRID_LAST_ROW: u16 = 0x14;

impl Core {
    /// G3 的格位址（原版 `sub_17D5F`）：`列` 大於等於 14h 改取 14h，位址 = G3 起點 + 列起點表 + 2 × 欄 + 1。
    /// 全部以 16 位元環繞運算，所以越界的位址由 [`Core::grid3_or`] 擋掉。
    pub(crate) fn grid3_cell_addr(&self, row: u16, col: u16) -> u16 {
        let row = row.min(GRID_LAST_ROW);
        let row_start = self.ds.r16(row.wrapping_shl(1).wrapping_add(GRID_ROW_TABLE));
        row_start.wrapping_add(GRID3_START).wrapping_add(col.wrapping_shl(1).wrapping_add(1))
    }

    /// 以 `or` 寫進 G3 的一格。位址落在 G3 範圍之外的格略過。
    pub(crate) fn grid3_or(&mut self, addr: u16, v: u8) {
        if (GRID3_START..=GRID3_END).contains(&addr) {
            let old = self.ds.r8(addr);
            self.ds.w8(addr, old | v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Fixes, Start};

    fn core() -> Core {
        let image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
        Core::new(&image, &[], Start::Demo, Fixes::NONE)
    }

    #[test]
    fn grid3_or_ignores_addresses_outside_g3() {
        let mut c = core();
        c.grid3_or(GRID3_START, 0x03);
        c.grid3_or(GRID3_START, 0x04);
        c.grid3_or(GRID3_END, 0x80);
        assert_eq!((c.ds.r8(GRID3_START), c.ds.r8(GRID3_END)), (0x07, 0x80), "範圍內的格以 or 累積，兩端都算");
        let before = c.ds.bytes().to_vec();
        for addr in [GRID3_START - 1, GRID3_END + 1, 0, 0xFFFF] {
            c.grid3_or(addr, 0xFF);
        }
        assert_eq!(c.ds.bytes(), &before[..], "範圍外的格不寫");
    }

    #[test]
    fn grid3_rows_from_14h_share_the_out_of_range_row() {
        let mut c = core();
        c.ds.w16(GRID_ROW_TABLE + 2 * 0x13, 0x05F1);
        c.ds.w16(GRID_ROW_TABLE + 2 * 0x14, 0x0059);
        assert_eq!(c.grid3_cell_addr(0x14, 3), c.grid3_cell_addr(0x1234, 3));
        assert_eq!(c.grid3_cell_addr(0xFFFF, 3), c.grid3_cell_addr(0x14, 3), "負的列號也是越界列");
        assert_ne!(c.grid3_cell_addr(0x13, 3), c.grid3_cell_addr(0x14, 3), "列 13h 是最後一個一般列");
    }

    #[test]
    fn grid3_cell_addr_is_start_plus_row_table_plus_two_cols_plus_one() {
        let mut c = core();
        c.ds.w16(GRID_ROW_TABLE + 2 * 5, 0x0101);
        assert_eq!(c.grid3_cell_addr(5, 7), GRID3_START + 0x0101 + 2 * 7 + 1);
        // 環繞：欄乘 2 超過 16 位元時高位元丟掉。
        assert_eq!(c.grid3_cell_addr(5, 0x8001), GRID3_START + 0x0101 + 2 + 1);
    }
}
