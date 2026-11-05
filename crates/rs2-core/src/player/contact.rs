//! 玩家的碰撞、撿取與升級。
//!
//! 本體與砲座各用一張形狀表（[`PlayerShape`]）掃格網：
//!
//! | 誰 | 形狀 | 掃哪個格網 | 結果 |
//! |---|---|---|---|
//! | 本體 | 5 × 1，位置 (欄, 列) | G1 實體 | 任何格值非 0 且非 FFh 就受擊 |
//! | 本體 | 3 寬，一列（第三關） | G4 地形 | 字組非 0 就受擊 |
//! | 本體 | 5 × 1，位置 (欄, 列 + 1) | G2 敵彈 | 非 0 的格刪除那顆敵彈；道具記下撿取，其他受擊 |
//! | 砲座 | 2 × 2 | G1 實體 | 最後掃到的實體格受 1 點傷害，砲座不受傷 |
//! | 砲座 | 5 × 1（本體的形狀） | G2 敵彈 | 格值高三位全為 0 的刪除那顆敵彈，道具不吃 |
//!
//! 另外本體在 G3 標出自己佔的髒格（[`Core::mark_body_cells`]）。
//!
//! 玩家掃描的格網窗與一般的格網存取不同（[`Core::scan_cell`]、`mirror.rs`），列號表外時掃到的是格網之外的資料，
//! 這裡不另外處理，照 [`Core::scan_footprint`] 走。

use super::{BEAM_START, NO, YES};
use crate::fx::{EffectKind, loop_count};
use crate::game::Core;
use crate::mirror::{Footprint, PlayerShape, ScanGrid};

/// 砲座碰到實體時造成的傷害。
const POD_DAMAGE: u8 = 1;
/// G2 格值的高三位：非 0 的是道具球，砲座不吃。
const ITEM_BITS: u8 = 0xE0;
/// G1 格值：存在但不造成接觸（見 `entity.rs`）。
const INERT_CELL: u8 = 0xFF;

/// 道具球的種類，由 G2 格值的高四位決定（文件第 5.4 節）。第一、二個道具啟用砲座，之後換武器。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pickup {
    /// 80h：雷射 `L`。
    Laser,
    /// 40h：追蹤彈 `B`。
    Seeker,
    /// C0h：大彈 `K`。
    Big,
    /// 20h：散彈 `S`。
    Spread,
}

impl Pickup {
    /// 格值高四位是 80h、40h、C0h、20h 的是道具。
    /// 道具形狀的格值是 81h、41h、C1h、21h，加上敵彈槽號後槽號 15 以上會進位，判定落空（見 `Core::stamp_grid2`）。
    pub(super) fn from_cell(cell: u8) -> Option<Pickup> {
        match cell & 0xF0 {
            0x80 => Some(Pickup::Laser),
            0x40 => Some(Pickup::Seeker),
            0xC0 => Some(Pickup::Big),
            0x20 => Some(Pickup::Spread),
            _ => None,
        }
    }

    /// 撿到之後的武器字母。
    pub(super) const fn weapon(self) -> u16 {
        match self {
            Pickup::Laser => b'L' as u16,
            Pickup::Seeker => b'B' as u16,
            Pickup::Big => b'K' as u16,
            Pickup::Spread => b'S' as u16,
        }
    }
}

impl Core {
    /// 本體的碰撞（原版 `sub_147E1`、`sub_1530F`）：標髒格，對 G1、G4、G2 掃描，受擊就在位置播特效。
    pub(super) fn collide_player(&mut self, who: usize) {
        self.mark_body_cells(who);
        self.body_touches_entities(who);
        self.body_touches_terrain(who);
        self.body_touches_bullets(who);
        let p = self.pl.ship[who];
        if p.hit != NO {
            self.spawn_effect(EffectKind::SparkRight, p.col, p.row);
        }
    }

    /// 在 G3 標出本體佔的 2 列 × 4 欄髒格（原版 `sub_14A2C`、`sub_1555A`），值 1。
    pub(super) fn mark_body_cells(&mut self, who: usize) {
        const ROWS: u16 = 2;
        const COLS: u16 = 4;
        const DIRTY: u16 = 1;
        let p = self.pl.ship[who];
        self.grid3_mark_block(p.row, p.col, ROWS, COLS, DIRTY);
    }

    /// 本體對 G1（原版 `sub_14B1E`、`sub_1564C`）：任何非 0 且非 FFh 的格都算受擊，不分哪個實體。
    pub(super) fn body_touches_entities(&mut self, who: usize) {
        let shape = self.player_footprint(who, PlayerShape::Body);
        self.set_stride_reg(shape.width);
        let p = self.pl.ship[who];
        let mut touched = false;
        self.scan_footprint(ScanGrid::Entities, p.row, p.col, shape, |_, _, cell| {
            touched |= cell != 0 && cell != INERT_CELL;
            None
        });
        if touched {
            self.pl.ship[who].hit = YES;
        }
    }

    /// 第三關：本體對 G4 地形（原版 `sub_14A90`、`sub_155BE`）。只掃本體所在的一列，寬取尾焰形狀的寬；
    /// 讀奇數位址的字組（見 [`Core::grid4_cell_addr`]），非 0 就算撞到。其他關不檢查，也不寫形狀寬。
    pub(super) fn body_touches_terrain(&mut self, who: usize) {
        if !self.on_terrain_stage() {
            return;
        }
        let shape = self.player_footprint(who, PlayerShape::Flame);
        self.set_stride_reg(shape.width);
        let p = self.pl.ship[who];
        let mut at = self.grid4_cell_addr(p.row, p.col);
        for _ in 0..loop_count(shape.width) {
            if self.grid4_contains(at) && self.grid4_word(at) != 0 {
                self.pl.ship[who].hit = YES;
            }
            at = at.wrapping_add(2);
        }
    }

    /// 本體對 G2（原版 `sub_14C85`、`sub_157B3`）：形狀的位置是 (欄, 列 + 1)。非 0 的格先刪除那顆敵彈，
    /// 再依格值判定（[`Core::touch_bullet_cell`]）。掃完依撿到的道具升級（[`Core::upgrade`]）。
    pub(super) fn body_touches_bullets(&mut self, who: usize) {
        let shape = self.player_footprint(who, PlayerShape::Body);
        self.set_stride_reg(shape.width);
        let p = self.pl.ship[who];
        let mut picked = None;
        self.scan_footprint(ScanGrid::Bullets, p.row.wrapping_add(1), p.col, shape, |core, addr, cell| {
            if cell == 0 {
                return None;
            }
            let row = core.remove_hit_bullet(cell, addr);
            if let Some(pickup) = core.touch_bullet_cell(who, cell) {
                picked = Some(pickup);
            }
            row
        });
        self.upgrade(who, picked);
    }

    /// 本體碰到 G2 格值為 `cell` 的格（原版 `sub_14CD4`、`sub_15802`）：道具球回傳種類，並重設撿取光柱；
    /// 其他格（含進位後落空的道具）設受擊旗標。
    pub(crate) fn touch_bullet_cell(&mut self, who: usize, cell: u8) -> Option<Pickup> {
        let pickup = Pickup::from_cell(cell);
        match pickup {
            Some(_) => self.pl.ship[who].beam_y = BEAM_START,
            None => self.pl.ship[who].hit = YES,
        }
        pickup
    }

    /// 依撿到的道具升級（原版 `sub_14D04`、`sub_15832`）。沒撿到什麼就不動；撿到時依序：
    /// 第一座砲座啟用、第二座砲座啟用，之後兩座都在才換武器。武器型別不變的前兩次只多砲座。
    /// 掃描中撿到多個道具時以最後一個為準。
    pub(super) fn upgrade(&mut self, who: usize, picked: Option<Pickup>) {
        let Some(pickup) = picked else { return };
        let p = &mut self.pl.ship[who];
        if p.pods.first == NO {
            p.pods.first = YES;
        } else if p.pods.second == NO {
            p.pods.second = YES;
        } else {
            p.weapon = pickup.weapon();
        }
    }

    /// 砲座對 G1（原版 `sub_14B63`、`sub_15691` 與 `sub_14C18`、`sub_15746`）：掃形狀，最後一個非 0 非 FFh 的格值
    /// 就是接觸的實體，對它加 1 點傷害（共用玩家彈的判斷，含裝甲格與已釋放槽的表外寫入）。砲座本身不受傷。
    /// (`col`, `row`) 是砲座要掃的位置：第一座在砲座位置，第二座在下方 [`super::PodStyle::gap_rows`] 列。
    pub(super) fn pod_touches_entities(&mut self, who: usize, col: u16, row: u16) {
        let shape = self.player_footprint(who, PlayerShape::Pod);
        self.set_stride_reg(shape.width);
        let mut contact = 0;
        self.scan_footprint(ScanGrid::Entities, row, col, shape, |_, _, cell| {
            if cell != 0 && cell != INERT_CELL {
                contact = cell;
            }
            None
        });
        if contact != 0 {
            self.damage_entity_at_cell(contact, POD_DAMAGE);
        }
    }

    /// 砲座對 G2（原版 `sub_14BAE`、`sub_156DC`）：用的是本體的形狀（5 × 1）。格值非 0 且高三位全為 0 的才刪除那顆敵彈，
    /// 道具球不吃。
    pub(super) fn pod_shields_bullets(&mut self, who: usize, col: u16, row: u16) {
        let shape = self.player_footprint(who, PlayerShape::Body);
        self.set_stride_reg(shape.width);
        self.scan_footprint(ScanGrid::Bullets, row, col, shape, |core, addr, cell| {
            if cell != 0 && cell & ITEM_BITS == 0 { core.remove_hit_bullet(cell, addr) } else { None }
        });
    }

    /// 形狀表的寬與高。
    pub(super) fn player_footprint(&self, who: usize, shape: PlayerShape) -> Footprint {
        let at = shape.table_offset(who);
        Footprint { width: self.table_word(at), height: self.table_word(at.wrapping_add(2)) }
    }

    /// 從 (`row`, `col`) 起逐列逐格掃 `shape` 的矩形，對窗內的每一格呼叫 `visit(核心, 位址, 格值)`。
    /// 每一列重算列起點（[`Core::scan_row_start`]），位置在窗外的格略過（[`Core::scan_cell`]）。
    ///
    /// `visit` 回傳 `Some(列)` 時，後面的列計數改從這個值起算。原版刪除敵彈的 `sub_14D54`、`sub_15882`
    /// 把槽號乘 2 留在掃描迴圈用來數列的暫存器（`bx`），之後的列沿用這個殘值（[`Core::remove_hit_bullet`] 的回傳值）。
    /// 玩家用的形狀掃 G2 時都只有一列，殘值沒有下一列可影響，收據分辨不出來；形狀表若有多列就會影響，照原版保留。
    fn scan_footprint(&mut self, grid: ScanGrid, mut row: u16, col: u16, shape: Footprint, mut visit: impl FnMut(&mut Core, u16, u8) -> Option<u16>) {
        for _ in 0..loop_count(shape.height) {
            let start = self.scan_row_start(grid, row);
            let mut addr = start.wrapping_add(col);
            for _ in 0..loop_count(shape.width) {
                if let Some(cell) = self.scan_cell(grid, start, addr) {
                    if let Some(next) = visit(self, addr, cell) {
                        row = next;
                    }
                }
                addr = addr.wrapping_add(1);
            }
            row = row.wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{core, place, put_g1, put_g2, set_shapes, stride};
    use super::*;
    use crate::game::Fixes;
    use crate::mirror::GRID3_START;

    fn shaped() -> Core {
        let mut c = core();
        set_shapes(&mut c);
        c
    }

    #[test]
    fn pickup_kinds_come_from_the_high_nibble_and_pick_the_documented_weapons() {
        for (cell, want) in [(0x81, Pickup::Laser), (0x41, Pickup::Seeker), (0xC1, Pickup::Big), (0x21, Pickup::Spread), (0x8F, Pickup::Laser)] {
            assert_eq!(Pickup::from_cell(cell), Some(want), "格值 {cell:#x}");
        }
        for cell in [0x01u8, 0x11, 0x31, 0x90, 0xA1, 0x61, 0xFF, 0] {
            assert_eq!(Pickup::from_cell(cell), None, "格值 {cell:#x} 不是道具");
        }
        assert_eq!([Pickup::Laser, Pickup::Seeker, Pickup::Big, Pickup::Spread].map(Pickup::weapon), [0x4C, 0x42, 0x4B, 0x53]);
        // 道具形狀 81h 加槽號 15 以上進位成 90h（原版缺陷），判定落空變成傷害。
        let mut c = core();
        place(&mut c, 0, 7, 8);
        assert_eq!(c.touch_bullet_cell(0, 0x90), None);
        assert_eq!(c.pl.ship[0].hit, YES);
        assert_eq!(c.pl.ship[0].beam_y, 2, "不是道具就不動光柱");
    }

    #[test]
    fn touching_an_item_starts_the_beam_and_anything_else_is_a_hit() {
        let mut c = core();
        place(&mut c, 1, 7, 8);
        assert_eq!(c.touch_bullet_cell(1, 0x83), Some(Pickup::Laser));
        assert_eq!((c.pl.ship[1].beam_y, c.pl.ship[1].hit), (BEAM_START, NO));
        assert_eq!(c.touch_bullet_cell(1, 0x03), None);
        assert_eq!(c.pl.ship[1].hit, YES);
    }

    #[test]
    fn upgrades_go_first_pod_then_second_pod_then_weapon_and_the_weapon_only_changes_last() {
        let mut c = core();
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].pods.first = NO;
        c.pl.ship[0].pods.second = NO;
        c.pl.ship[0].weapon = 0;
        c.upgrade(0, None);
        assert_eq!((c.pl.ship[0].pods.first, c.pl.ship[0].pods.second, c.pl.ship[0].weapon), (NO, NO, 0), "沒撿到不動");
        c.upgrade(0, Some(Pickup::Seeker));
        assert_eq!((c.pl.ship[0].pods.first, c.pl.ship[0].pods.second, c.pl.ship[0].weapon), (YES, NO, 0), "第一個道具啟用砲座 1");
        c.upgrade(0, Some(Pickup::Seeker));
        assert_eq!((c.pl.ship[0].pods.first, c.pl.ship[0].pods.second, c.pl.ship[0].weapon), (YES, YES, 0), "第二個啟用砲座 2");
        for (pickup, letter) in [(Pickup::Laser, b'L'), (Pickup::Seeker, b'B'), (Pickup::Big, b'K'), (Pickup::Spread, b'S')] {
            c.upgrade(0, Some(pickup));
            assert_eq!(c.pl.ship[0].weapon, u16::from(letter));
        }
        // 玩家 2 的升級只動自己的砲座與武器。
        place(&mut c, 1, 7, 8);
        c.pl.ship[1].pods.first = NO;
        let p1 = c.pl.ship[0];
        c.upgrade(1, Some(Pickup::Big));
        assert_eq!((c.pl.ship[1].pods.first, c.pl.ship[1].weapon), (YES, 0));
        assert_eq!(c.pl.ship[0], p1);
    }

    #[test]
    fn the_body_is_hit_by_any_entity_cell_except_empty_and_ffh_and_never_by_cells_outside_the_window() {
        let mut c = shaped();
        place(&mut c, 0, 7, 8);
        // 本體 5 × 1，欄 7 至 11，列 8。
        put_g1(&mut c, 9, 8, 0xFF);
        c.body_touches_entities(0);
        assert_eq!(c.pl.ship[0].hit, NO, "FFh 不算");
        put_g1(&mut c, 12, 8, 3);
        put_g1(&mut c, 9, 9, 3);
        c.body_touches_entities(0);
        assert_eq!(c.pl.ship[0].hit, NO, "形狀之外的格不算");
        put_g1(&mut c, 11, 8, 0xFE);
        c.body_touches_entities(0);
        assert_eq!(c.pl.ship[0].hit, YES, "裝甲格 FEh 也算（只有玩家彈不記錄）");
        // 玩家 2 掃自己的位置。
        place(&mut c, 1, 20, 3);
        c.body_touches_entities(1);
        assert_eq!(c.pl.ship[1].hit, NO);
        put_g1(&mut c, 22, 3, 1);
        c.body_touches_entities(1);
        assert_eq!(c.pl.ship[1].hit, YES);
        // 掃描記下形狀寬，供下一個繪製命令使用。
        assert_eq!(stride(&c), Some(5));
    }

    #[test]
    fn a_pod_damages_the_last_entity_cell_it_touches_and_the_second_pod_scans_lower() {
        let mut c = shaped();
        place(&mut c, 0, 7, 8);
        for slot in [0usize, 1] {
            c.en.slots[slot].link = crate::mirror::enemy_record_link(slot);
        }
        // 砲座 2 × 2，位置 (欄 10, 列 5)：後掃到的覆蓋前面（列主序，最後是 (11, 6)）。
        put_g1(&mut c, 10, 5, 1);
        put_g1(&mut c, 11, 6, 2);
        put_g1(&mut c, 10, 6, 0xFF);
        c.pod_touches_entities(0, 10, 5);
        assert_eq!((c.en.slots[0].enemy.damage, c.en.slots[1].enemy.damage), (0, 0x0101), "格值 2 是槽 1，加 1 傷害");
        assert_eq!(c.pl.ship[0].hit, NO, "砲座不受傷");
        // 沒有接觸就沒有傷害。
        c.en.slots[1].enemy.damage = 0;
        c.pod_touches_entities(0, 14, 5);
        assert_eq!(c.en.slots[1].enemy.damage, 0);
        // 位置往下 5 列（第二座砲座）就掃到別的格。
        put_g1(&mut c, 10, 10, 1);
        c.pod_touches_entities(0, 10, 10);
        assert_eq!(c.en.slots[0].enemy.damage, 0x0101);
    }

    #[test]
    fn pod_contact_with_the_armor_cell_writes_outside_the_pool_unless_fixed() {
        for (fixes, want) in [(Fixes::NONE, 0x0166u16), (Fixes::ALL, 0x0165)] {
            let mut c = shaped();
            c.fixes = fixes;
            c.ds.w16(0x0020, 0x0165);
            place(&mut c, 0, 7, 8);
            put_g1(&mut c, 10, 5, 0xFE);
            c.pod_touches_entities(0, 10, 5);
            assert_eq!(c.ds.r16(0x0020), want, "裝甲格取到索引 FDh，指標 0 加 20h 是 dseg:0020h");
        }
    }

    #[test]
    fn the_body_removes_every_bullet_it_touches_and_a_pod_spares_the_items() {
        let mut c = shaped();
        place(&mut c, 0, 7, 8);
        for s in c.eb.slots.iter_mut() {
            s.link = 0x1234;
        }
        // 本體的 G2 位置是 (欄 7 至 11, 列 9)。格值低四位是槽號 + 1。
        put_g2(&mut c, 8, 9, 0x01);
        put_g2(&mut c, 10, 9, 0x02);
        c.body_touches_bullets(0);
        assert_eq!((c.eb.slots[0].link, c.eb.slots[1].link, c.eb.slots[2].link), (0, 0, 0x1234), "槽 0、1 被刪");
        assert_eq!(c.pl.ship[0].hit, YES, "非道具算受擊");
        // 砲座：道具球（高三位非 0）不吃，普通敵彈刪除。
        let mut c = shaped();
        place(&mut c, 0, 7, 8);
        for s in c.eb.slots.iter_mut() {
            s.link = 0x1234;
        }
        put_g2(&mut c, 8, 4, 0x82);
        put_g2(&mut c, 9, 4, 0x03);
        c.pod_shields_bullets(0, 7, 4);
        assert_eq!((c.eb.slots[1].link, c.eb.slots[2].link), (0x1234, 0), "道具球 82h 不刪，03h 刪槽 2");
    }

    #[test]
    fn body_pickup_scan_upgrades_once_per_frame_and_the_last_item_wins() {
        let mut c = shaped();
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].pods.first = YES;
        c.pl.ship[0].pods.second = YES;
        put_g2(&mut c, 8, 9, 0x81);
        put_g2(&mut c, 10, 9, 0x21);
        c.body_touches_bullets(0);
        assert_eq!(c.pl.ship[0].weapon, u16::from(b'S'), "後掃到的道具為準");
        assert_eq!((c.pl.ship[0].hit, c.pl.ship[0].beam_y), (NO, BEAM_START));
    }

    #[test]
    fn terrain_collision_reads_g4_only_on_the_third_stage_and_only_the_players_row() {
        let mut c = shaped();
        place(&mut c, 0, 7, 8);
        let at = c.grid4_cell_addr(8, 7);
        c.ds.w16(at + 2, 0x0102);
        c.body_touches_terrain(0);
        assert_eq!(c.pl.ship[0].hit, NO, "不是第三關不檢查");
        assert_eq!(stride(&c), None, "也不寫形狀寬");
        c.set_stage_letter(b'C');
        c.body_touches_terrain(0);
        assert_eq!(c.pl.ship[0].hit, YES);
        assert_eq!(stride(&c), Some(3), "形狀寬取尾焰形狀的 3");
        // 上一列、下一列的地形不算。
        let mut c = shaped();
        place(&mut c, 0, 7, 8);
        c.set_stage_letter(b'C');
        let (above, below) = (c.grid4_cell_addr(7, 7), c.grid4_cell_addr(9, 7));
        c.ds.w16(above, 0x0001);
        c.ds.w16(below, 0x0001);
        c.body_touches_terrain(0);
        assert_eq!(c.pl.ship[0].hit, NO);
    }

    #[test]
    fn collide_marks_the_dirty_cells_and_plays_the_spark_when_hit() {
        let mut c = shaped();
        place(&mut c, 1, 7, 8);
        c.collide_player(1);
        // 2 列 4 欄，每格一個字組 0001h，位址 = G3 起點 + 2 × (列 × 28h + 欄 + 1)。
        for (row, col) in [(8u16, 7u16), (8, 10), (9, 7), (9, 10)] {
            let at = GRID3_START + 2 * (row * 0x28 + col + 1);
            assert_eq!(c.ds.r16(at), 1, "列 {row} 欄 {col}");
        }
        assert_eq!(c.ds.r16(GRID3_START + 2 * (8 * 0x28 + 11 + 1)), 0, "第 5 欄不標");
        assert!(c.fx.slots.iter().all(|s| !s.is_active()), "沒受擊不播特效");
        put_g1(&mut c, 8, 8, 1);
        c.collide_player(1);
        assert_eq!(c.pl.ship[1].hit, YES);
        assert_eq!(c.fx.slots.iter().filter(|s| s.is_active()).count(), 1, "受擊在位置播特效");
        assert_eq!(c.fx.cursor, (7, 8));
    }

    #[test]
    fn a_footprint_scan_keeps_the_row_counter_left_by_a_bullet_removal() {
        // 形狀 1 欄 3 列，從列 2 起。第一列刪除槽 4 的敵彈，槽號 × 2 = 8 留作列計數，所以第二列掃列 9（不是列 3）；
        // 第二列刪除槽 1 的敵彈，列計數變成 2，所以第三列掃列 3。沒有這個殘值的話會掃列 2、3、4。
        let mut c = core();
        for s in c.eb.slots.iter_mut() {
            s.link = 0x1234;
        }
        put_g2(&mut c, 7, 2, 0x05);
        put_g2(&mut c, 7, 9, 0x02);
        put_g2(&mut c, 7, 3, 0x03);
        put_g2(&mut c, 7, 4, 0x07);
        let shape = Footprint { width: 1, height: 3 };
        let mut seen = Vec::new();
        c.scan_footprint(ScanGrid::Bullets, 2, 7, shape, |core, addr, cell| {
            seen.push(cell);
            core.remove_hit_bullet(cell, addr)
        });
        assert_eq!(seen, vec![0x05, 0x02, 0x03]);
        let freed: Vec<usize> = (0..8).filter(|&i| c.eb.slots[i].link == 0).collect();
        assert_eq!(freed, vec![1, 2, 4], "槽 4、1、2 被刪，列 4 的 07h（槽 6）沒被掃到");
    }

    #[test]
    fn the_scan_window_ignores_cells_outside_the_row_and_the_grid() {
        let mut c = core();
        // 欄 0 在列起點上，嚴格在列起點之後才算。
        put_g1(&mut c, 0, 5, 7);
        let shape = Footprint { width: 3, height: 1 };
        let mut seen = Vec::new();
        c.scan_footprint(ScanGrid::Entities, 5, 0, shape, |_, _, cell| {
            seen.push(cell);
            None
        });
        assert_eq!(seen, vec![0, 0], "欄 0 不掃，欄 1、2 掃");
        // 負列（無號大於等於 17Ch）取列基底表索引 1F4h，等於列 0 的起點。
        put_g1(&mut c, 3, 0, 9);
        let mut seen = Vec::new();
        c.scan_footprint(ScanGrid::Entities, 0xFFFD, 3, Footprint { width: 1, height: 1 }, |_, _, cell| {
            seen.push(cell);
            None
        });
        assert_eq!(seen, vec![9]);
    }
}
