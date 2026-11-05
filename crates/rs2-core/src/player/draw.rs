//! 玩家的繪製與砲座：每幀依序是砲座（碰撞、畫、更新位置、發射）、本體、撿取光柱、碰撞、發射閃光、尾焰。
//!
//! 所有圖都用 `MY.GO` 圖庫的遮罩（AND）加四平面（OR）兩個命令畫成一組。繪製命令的位置、圖與形狀寬的順序
//! 都要和原版一致，前端的補間與預測靠物件編號（本體 `Pool::Player` 槽 0、1，砲座 `Pool::Pod` 槽 0 至 3，
//! 玩家 1 用 0、1，玩家 2 用 2、3）。

use super::{BEAM_STEP, FACING_RIGHT, ROW_PIXELS, STYLES, YES};
use crate::draw::Pool;
use crate::game::Core;
use crate::player::PlayerShape;
use crate::pbullet::Facing;
use crate::video::MyCopy;

/// 撿取光柱、發射閃光、砲座噴焰、尾焰的圖庫索引。
const BEAM_IMAGE: u16 = 8;
const EXHAUST_IMAGE: u16 = 0x0A;
const FLAME_IMAGE: u16 = 0x0D;
/// 尾焰在奇數繪製頁換成另一張，兩張交替閃爍。
const FLAME_IMAGE_ODD_PAGE: u16 = 0x0C;
/// 尾焰畫在本體左邊 4 欄、上方 2 列像素。
const FLAME_DX: u16 = 4;
const FLAME_DY: u16 = 2;
/// 尾焰的髒格從本體左邊 4 欄、上方 1 列起。
const FLAME_CELLS_DX: u16 = 4;
const FLAME_CELLS_DY: u16 = 1;
/// 本體的四平面比遮罩低 1 像素列。
const BODY_PLANES_DY: u16 = 1;
/// 砲座噴焰相對砲座位置：朝右往右 2 欄，朝左往左 1 欄，兩個朝向都往下 2 列像素。
const EXHAUST_RIGHT_DX: u16 = 2;
const EXHAUST_LEFT_DX: u16 = 1;
const EXHAUST_DY: u16 = 2;

impl Core {
    /// 畫一位在場的玩家（原版 `sub_14704`、`sub_15232`）。受擊後的無敵幀只畫遮罩，奇數頁再閃白，跳過所有碰撞。
    pub(crate) fn draw_player(&mut self, who: usize) {
        if !self.pl.is_alive(who) {
            return;
        }
        self.fly_pods(who);
        let p = self.pl.ship[who];
        self.set_blit_params(p.blit_x, p.blit_y, p.image);
        self.set_obj(Pool::Player, who as u8);
        self.cut_my(MyCopy::General);
        if p.invincible != 0 {
            self.pl.ship[who].invincible = p.invincible.wrapping_sub(1);
            if self.draw_page() != 0 {
                self.flash_my();
            }
            self.clear_obj();
            return;
        }
        self.set_blit_params(p.blit_x, p.blit_y.wrapping_add(BODY_PLANES_DY), p.image);
        self.overlay_my(MyCopy::General);
        // 發射閃光畫在「上一個畫的東西」的位置：沒有光柱時是四平面的位置，光柱在畫時是光柱的位置。
        let mut pen = (p.blit_x, p.blit_y.wrapping_add(BODY_PLANES_DY));
        if let Some(beam) = self.draw_pickup_beam(who) {
            pen = beam;
        }
        self.collide_player(who);
        self.draw_muzzle_flash(who, pen);
        if p.flame == YES {
            self.draw_flame(who);
        }
        self.clear_obj();
    }

    /// 撿取光柱（原版 `sub_147BC`、`sub_152EA`）：`beam_y` 不大於 0（有號）時每幀加 [`BEAM_STEP`] 並畫一道圖 8，
    /// 撿到道具後約 5 幀。回傳畫的位置，沒畫回傳 `None`。
    fn draw_pickup_beam(&mut self, who: usize) -> Option<(u16, u16)> {
        let p = &mut self.pl.ship[who];
        if (p.beam_y as i16) > 0 {
            return None;
        }
        p.beam_y = p.beam_y.wrapping_add(BEAM_STEP);
        let at = (p.blit_x, p.beam_y);
        self.set_blit_params(at.0, at.1, BEAM_IMAGE);
        self.cut_my(MyCopy::General);
        self.overlay_my(MyCopy::General);
        Some(at)
    }

    /// 本幀開火時在機頭畫發射閃光（原版 `sub_1478B`、`sub_152B9`），位置是 `pen` 加玩家的閃光位移。
    fn draw_muzzle_flash(&mut self, who: usize, pen: (u16, u16)) {
        if self.pl.ship[who].fired != YES {
            return;
        }
        let muzzle = STYLES[who].muzzle;
        self.set_blit_params(pen.0.wrapping_add(muzzle.dx), pen.1.wrapping_add(muzzle.dy), muzzle.image);
        self.cut_my(MyCopy::General);
        self.overlay_my(MyCopy::General);
    }

    /// 尾焰（原版 `14757h`、`15285h`）：畫在本體左上方，同時把尾焰的髒格疊進 G3。
    fn draw_flame(&mut self, who: usize) {
        let p = self.pl.ship[who];
        let image = if self.draw_page() != 0 { FLAME_IMAGE_ODD_PAGE } else { FLAME_IMAGE };
        self.set_blit_params(p.blit_x.wrapping_sub(FLAME_DX), p.blit_y.wrapping_sub(FLAME_DY), image);
        self.cut_my(MyCopy::General);
        self.overlay_my(MyCopy::General);
        self.mark_flame_cells(who);
    }

    /// 把尾焰的髒格形狀（3 × 3）以 `or` 疊進 G3（原版 `sub_14A59`、`sub_15587`），左上角在本體的左邊 4 欄、上方 1 列。
    fn mark_flame_cells(&mut self, who: usize) {
        let p = self.pl.ship[who];
        let tables = self.tables();
        self.stamp_shape(
            tables.player_shape(PlayerShape::Flame, who),
            p.col.wrapping_sub(FLAME_CELLS_DX),
            p.row.wrapping_sub(FLAME_CELLS_DY),
        );
    }

    /// 砲座（原版 `sub_1480A`、`sub_15338`）。第一座在場才處理，順序是：
    ///
    /// 1. 第一座對 G1、G2 碰撞，畫第一座；第二座在場就畫第二座，再做第二座的碰撞（位置在第一座下方）。
    /// 2. 更新位置與圖（[`Core::advance_pods`]）。畫與碰撞用的是更新前的值，下一幀才換。
    /// 3. 本幀開火就從更新後的位置射出，並在更新前的位置畫噴焰。
    fn fly_pods(&mut self, who: usize) {
        let p = self.pl.ship[who];
        if !p.pods.first_flying() {
            return;
        }
        let style = &STYLES[who].pod;
        let gap_px = style.gap_rows * ROW_PIXELS;
        let (x, y) = (p.pods.blit_x, p.pods.blit_y.wrapping_add(style.draw_dy));
        let second = p.pods.second_flying();
        self.pod_collide(who, p.pods.col, p.pods.row);
        self.draw_pod(who, 0, (x, y), p.pods.image);
        if second {
            self.draw_pod(who, 1, (x, y.wrapping_add(gap_px)), p.pods.image);
            self.pod_collide(who, p.pods.col, p.pods.row.wrapping_add(style.gap_rows));
        }
        self.advance_pods(who);
        let p = self.pl.ship[who];
        if p.fired != YES {
            return;
        }
        let facing = Facing::from_word(p.pods.facing);
        // 細步的半列位置：子彈的繪製 y 跟著砲座。原版恆為 0。
        self.enh.spawn_dy = self.pod_sub_row(who);
        self.fire_player_bullets(p.weapon, facing, p.pods.col, p.pods.row);
        if second {
            self.fire_player_bullets(p.weapon, facing, p.pods.col, p.pods.row.wrapping_add(style.gap_rows));
        }
        self.enh.spawn_dy = 0;
        let dx = if p.pods.facing != FACING_RIGHT { 0u16.wrapping_sub(EXHAUST_LEFT_DX) } else { EXHAUST_RIGHT_DX };
        let at = (x.wrapping_add(dx), y.wrapping_add(EXHAUST_DY));
        self.draw_pod(who, 0, at, EXHAUST_IMAGE);
        if second {
            self.draw_pod(who, 1, (at.0, at.1.wrapping_add(gap_px)), EXHAUST_IMAGE);
        }
    }

    /// 砲座在 (`col`, `row`) 的碰撞：先對 G1（傷害實體），再對 G2（擋敵彈）。
    fn pod_collide(&mut self, who: usize, col: u16, row: u16) {
        self.pod_touches_entities(who, col, row);
        self.pod_shields_bullets(who, col, row);
    }

    /// 畫第 `which` 座砲座（0 或 1），物件編號是玩家的兩個砲座槽。
    fn draw_pod(&mut self, who: usize, which: u8, at: (u16, u16), image: u16) {
        self.set_blit_params(at.0, at.1, image);
        self.set_obj(Pool::Pod, (2 * who) as u8 + which);
        self.cut_my(MyCopy::General);
        self.overlay_my(MyCopy::General);
        self.clear_obj();
    }

    /// 砲座的位置與圖框，每幀畫完之後更新（原版 `sub_14908`、`sub_1496D`、`sub_149D2` 與玩家 2 的 `sub_15436`、`sub_1549B`、`sub_15500`）。
    ///
    /// 位置在本體的右後方：欄加 3、繪製位置 x 加 3，列與 y 依 [`super::PodStyle`] 往上；朝左再往左 5。
    /// 圖框動畫的段數與起點隨充能變，三段的上限與圖庫索引在 [`POD_FRAMES`]：
    /// 充能小於 6 時用 5 格循環，朝右 1Eh 起、朝左 24h 起；6 至 13 用 31h 起 3 格，14 以上用 34h 起 3 格，
    /// 這兩段朝左朝右用同一張圖。
    ///
    /// 砲座像 Force（規格 005）分離時，位置由 [`Core::fly_free_pods`] 算，這裡只換圖框；接回的那一幀由這裡接手位置。
    pub(super) fn advance_pods(&mut self, who: usize) {
        // 原版恆為 false（強化狀態只在 `force_pod` 開著時離開附著）。
        if self.pods_detached(who) {
            self.fly_free_pods(who);
        }
        let detached = self.pods_detached(who);
        let style = &STYLES[who].pod;
        // 波動砲充能中，圖框看換算後的值；否則就是原版的充能計數。
        let charge = self.pod_display_charge(who);
        let p = &mut self.pl.ship[who];
        let frames = POD_FRAMES.iter().rev().find(|f| charge >= f.min_charge).expect("最小的一段從 0 起");
        if !detached {
            p.pods.blit_y = p.blit_y.wrapping_sub(style.y_offset);
            p.pods.blit_x = p.blit_x.wrapping_add(POD_DX);
            p.pods.row = p.row.wrapping_sub(style.row_offset);
            p.pods.col = p.col.wrapping_add(POD_DX);
        }
        p.pods.anim = p.pods.anim.wrapping_add(1);
        if p.pods.anim >= frames.count {
            p.pods.anim = 0;
        }
        if p.pods.facing == FACING_RIGHT {
            p.pods.image = p.pods.anim.wrapping_add(frames.right_image);
        } else {
            p.pods.image = p.pods.anim.wrapping_add(frames.left_image);
            if !detached {
                p.pods.col = p.pods.col.wrapping_sub(POD_LEFT_SHIFT);
                p.pods.blit_x = p.pods.blit_x.wrapping_sub(POD_LEFT_SHIFT);
            }
        }
    }
}

/// 砲座在本體右後方的欄數（格網與繪製位置同值）。
pub(super) const POD_DX: u16 = 3;
/// 朝左時砲座再往左的欄數。
pub(super) const POD_LEFT_SHIFT: u16 = 5;

/// 砲座圖框動畫的一段：充能不小於 `min_charge` 時用。圖框是 `起點 + 動畫計數`，計數到 `count` 歸零。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PodFrames {
    pub min_charge: u16,
    pub count: u16,
    pub right_image: u16,
    pub left_image: u16,
}

/// 三段砲座圖框，依充能由小到大。
pub(super) const POD_FRAMES: [PodFrames; 3] = [
    PodFrames { min_charge: 0, count: 5, right_image: 0x1E, left_image: 0x24 },
    PodFrames { min_charge: 6, count: 3, right_image: 0x31, left_image: 0x31 },
    PodFrames { min_charge: 0x0E, count: 3, right_image: 0x34, left_image: 0x34 },
];

#[cfg(test)]
mod tests {
    use super::super::test_support::{core, place, put_g1, set_shapes, stride};
    use super::super::{FACING_LEFT, NO};
    use super::*;
    use crate::grid::GRID3_START;
    use crate::draw::{BlitCmd, DrawCmd, Op};

    fn blits(c: &mut Core) -> Vec<BlitCmd> {
        std::mem::take(&mut c.draws)
            .into_iter()
            .filter_map(|d| match d {
                DrawCmd::Blit(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    fn summary(b: &[BlitCmd]) -> Vec<(Op, u16, u16, u16)> {
        b.iter().map(|b| (b.op, b.x, b.y, b.index)).collect()
    }

    fn fresh() -> Core {
        let mut c = core();
        set_shapes(&mut c);
        c.draws.clear();
        c
    }

    #[test]
    fn a_plain_body_is_one_mask_then_four_planes_one_row_lower() {
        let mut c = fresh();
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].image = 9;
        c.draw_player(0);
        let cmds = blits(&mut c);
        assert_eq!(summary(&cmds), vec![(Op::S18D46, 7, 80, 9), (Op::S18CE2, 7, 81, 9)]);
        assert!(cmds.iter().all(|b| b.obj.map(|o| (o.pool, o.slot)) == Some((Pool::Player, 0))), "物件編號是玩家槽 0");
        assert_eq!(cmds.iter().map(|b| b.stride_reg).collect::<Vec<_>>(), vec![None, None], "碰撞在畫之後，這兩個命令沒有帶形狀寬");
        assert_eq!(stride(&c), Some(5), "碰撞最後掃 G2 的本體形狀，留下寬 5 給下一個命令");
        let mut c = fresh();
        place(&mut c, 1, 7, 8);
        c.pl.ship[1].image = 0x10;
        c.draw_player(1);
        let cmds = blits(&mut c);
        assert_eq!(summary(&cmds), vec![(Op::S18D46, 7, 80, 0x10), (Op::S18CE2, 7, 81, 0x10)]);
        assert!(cmds.iter().all(|b| b.obj.map(|o| (o.pool, o.slot)) == Some((Pool::Player, 1))), "玩家 2 是槽 1");
    }

    #[test]
    fn nobody_is_drawn_when_not_alive() {
        let mut c = fresh();
        c.pl.alive = 0;
        c.draw_player(0);
        c.draw_player(1);
        assert!(c.draws.is_empty());
    }

    #[test]
    fn the_invincible_frames_draw_only_the_mask_flash_on_odd_pages_and_skip_the_collisions() {
        for (page, want) in [(0u16, vec![Op::S18D46]), (0x8000, vec![Op::S18D46, Op::S18D89])] {
            let mut c = fresh();
            place(&mut c, 0, 7, 8);
            c.set_draw_page(page);
            c.pl.ship[0].invincible = 3;
            put_g1(&mut c, 8, 8, 1);
            c.draw_player(0);
            let ops: Vec<Op> = blits(&mut c).iter().map(|b| b.op).collect();
            assert_eq!(ops, want, "頁 {page:#x}");
            assert_eq!(c.pl.ship[0].invincible, 2);
            assert_eq!(c.pl.ship[0].hit, NO, "無敵時不碰撞");
        }
    }

    #[test]
    fn the_flame_is_drawn_left_and_above_the_body_and_marks_g3() {
        let mut c = fresh();
        place(&mut c, 0, 9, 8);
        c.pl.ship[0].flame = YES;
        c.pl.ship[0].image = 9;
        c.draw_player(0);
        let cmds = blits(&mut c);
        assert_eq!(
            summary(&cmds),
            vec![(Op::S18D46, 9, 80, 9), (Op::S18CE2, 9, 81, 9), (Op::S18D46, 5, 78, 0x0D), (Op::S18CE2, 5, 78, 0x0D)],
            "尾焰在 x - 4、y - 2，偶數頁用圖 0Dh"
        );
        // 尾焰的髒格：形狀 3 × 3 全 1，左上角欄 5、列 7。列 7 只有尾焰會蓋到（本體的髒格從列 8 起）。
        for col in 5u16..8 {
            assert_eq!(c.grids.peek8(GRID3_START + 0x50 * 7 + 2 * col + 2), 1, "列 7 欄 {col}");
        }
        assert_eq!(c.grids.peek8(GRID3_START + 0x50 * 7 + 2 * 8 + 2), 0, "欄 8 在 3 欄之外");
        let mut c = fresh();
        place(&mut c, 0, 9, 8);
        c.pl.ship[0].flame = YES;
        c.set_draw_page(0x8000);
        c.draw_player(0);
        let flame_image = blits(&mut c)[2].index;
        assert_eq!(flame_image, 0x0C, "奇數頁換另一張尾焰");
    }

    #[test]
    fn the_muzzle_flash_follows_the_last_thing_drawn_which_is_the_beam_when_it_is_active() {
        let mut c = fresh();
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].fired = YES;
        c.draw_player(0);
        let s = summary(&blits(&mut c));
        assert_eq!(s[2..], [(Op::S18D46, 10, 84, 0x11), (Op::S18CE2, 10, 84, 0x11)], "沒有光柱：四平面位置 (7, 81) 加 (3, 3)");
        // 光柱在畫：位置 (x, 光柱 y)，閃光跟著光柱的 y。
        let mut c = fresh();
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].fired = YES;
        c.pl.ship[0].beam_y = 0xFF42;
        c.draw_player(0);
        let s = summary(&blits(&mut c));
        assert_eq!(c.pl.ship[0].beam_y, 0xFF42u16.wrapping_add(0x28));
        let beam_y = 0xFF6Au16;
        assert_eq!(
            s[2..],
            [(Op::S18D46, 7, beam_y, 8), (Op::S18CE2, 7, beam_y, 8), (Op::S18D46, 10, beam_y + 3, 0x11), (Op::S18CE2, 10, beam_y + 3, 0x11)],
            "原版的閃光接在光柱後面，y 跟光柱"
        );
        // 玩家 2 的閃光：圖 0Ah，位移 (3, 6)。
        let mut c = fresh();
        place(&mut c, 1, 7, 8);
        c.pl.ship[1].fired = YES;
        c.draw_player(1);
        let s = summary(&blits(&mut c));
        assert_eq!(s[2..], [(Op::S18D46, 10, 87, 0x0A), (Op::S18CE2, 10, 87, 0x0A)]);
    }

    #[test]
    fn the_beam_runs_five_frames_after_a_pickup_and_then_stops() {
        let mut c = fresh();
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].beam_y = 0xFF42;
        let mut ys = Vec::new();
        for _ in 0..7 {
            c.draw_player(0);
            let cmds = blits(&mut c);
            ys.push(cmds.iter().filter(|b| b.index == 8).map(|b| b.y as i16).next());
        }
        assert_eq!(ys, vec![Some(-150), Some(-110), Some(-70), Some(-30), Some(10), None, None], "5 幀，y 每幀加 40");
    }

    #[test]
    fn pods_are_drawn_with_the_stale_position_and_advance_afterwards() {
        let mut c = fresh();
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].pods = super::super::Pods { first: YES, second: NO, facing: FACING_RIGHT, image: 0x20, anim: 4, col: 3, row: 2, blit_x: 30, blit_y: 40 };
        c.draw_player(0);
        let cmds = blits(&mut c);
        assert_eq!(summary(&cmds)[..2], [(Op::S18D46, 30, 45, 0x20), (Op::S18CE2, 30, 45, 0x20)], "畫更新前的位置，y 加 5");
        assert_eq!((cmds[0].obj.map(|o| o.pool), cmds[0].obj.map(|o| o.slot)), (Some(Pool::Pod), Some(0)));
        // 更新後：欄 = 本體欄 + 3，列 = 本體列 - 3，朝右不位移，動畫計數 4 + 1 = 5 歸零。
        let pods = c.pl.ship[0].pods;
        assert_eq!((pods.col, pods.row, pods.blit_x, pods.blit_y, pods.anim, pods.image), (10, 5, 10, 80 - 0x1E, 0, 0x1E));
    }

    #[test]
    fn the_second_pod_is_drawn_below_the_first_and_each_player_has_its_own_pod_slots() {
        for (who, gap_px, draw_dy, first_slot) in [(0usize, 50u16, 5u16, 0u8), (1, 20, 2, 2)] {
            let mut c = fresh();
            place(&mut c, who, 7, 8);
            c.pl.ship[who].pods = super::super::Pods { first: YES, second: YES, facing: FACING_RIGHT, image: 0x20, anim: 0, col: 3, row: 2, blit_x: 30, blit_y: 40 };
            c.draw_player(who);
            let cmds = blits(&mut c);
            let s = summary(&cmds);
            assert_eq!(s[..4], [
                (Op::S18D46, 30, 40 + draw_dy, 0x20),
                (Op::S18CE2, 30, 40 + draw_dy, 0x20),
                (Op::S18D46, 30, 40 + draw_dy + gap_px, 0x20),
                (Op::S18CE2, 30, 40 + draw_dy + gap_px, 0x20),
            ]);
            let slots: Vec<u8> = cmds[..4].iter().map(|b| b.obj.expect("有物件").slot).collect();
            assert_eq!(slots, vec![first_slot, first_slot, first_slot + 1, first_slot + 1], "玩家 {} 的砲座槽", who + 1);
            assert!(cmds[..4].iter().all(|b| b.obj.expect("有物件").pool == Pool::Pod));
        }
    }

    #[test]
    fn advancing_the_pods_follows_the_charge_stages_and_the_facing() {
        // (充能, 朝向, 動畫計數起點) -> (圖, 欄偏移)
        let cases = [
            (0u16, FACING_RIGHT, 0u16, 0x1F, 0i16),
            (5, FACING_RIGHT, 3, 0x22, 0),
            (5, FACING_RIGHT, 4, 0x1E, 0),
            (0, FACING_LEFT, 0, 0x25, -5),
            (6, FACING_RIGHT, 0, 0x32, 0),
            (13, FACING_LEFT, 1, 0x33, -5),
            (13, FACING_LEFT, 2, 0x31, -5),
            (14, FACING_RIGHT, 0, 0x35, 0),
            (99, FACING_LEFT, 2, 0x34, -5),
        ];
        for (charge, facing, anim, image, shift) in cases {
            let mut c = fresh();
            place(&mut c, 0, 7, 8);
            c.pl.ship[0].charge = charge;
            c.pl.ship[0].pods.facing = facing;
            c.pl.ship[0].pods.anim = anim;
            c.advance_pods(0);
            let pods = c.pl.ship[0].pods;
            assert_eq!(pods.image, image, "充能 {charge} 朝 {facing:#x} 計數 {anim}");
            assert_eq!(pods.col as i16, 10 + shift, "欄加 3，朝左再減 5");
            assert_eq!(pods.blit_x as i16, 10 + shift);
        }
        // 玩家 2 的列與 y 偏移不同。
        let mut c = fresh();
        place(&mut c, 1, 7, 8);
        c.advance_pods(1);
        let pods = c.pl.ship[1].pods;
        assert_eq!((pods.row, pods.blit_y), (7, 80 - 0x0A));
    }

    #[test]
    fn a_firing_frame_spawns_the_pod_bullets_at_the_new_position_and_draws_the_exhaust_at_the_old_one() {
        let mut c = fresh();
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].fired = YES;
        c.pl.ship[0].weapon = u16::from(b'K');
        c.pl.ship[0].pods = super::super::Pods { first: YES, second: YES, facing: FACING_RIGHT, image: 0x20, anim: 0, col: 3, row: 2, blit_x: 30, blit_y: 40 };
        c.draw_player(0);
        let active: Vec<(u16, u16)> = c.pb.slots.iter().filter(|s| s.is_active()).map(|s| (s.bullet.col, s.bullet.row)).collect();
        assert_eq!(active, vec![(10, 5), (10, 10)], "新位置 (欄 10, 列 5) 和下方 5 列");
        let s = summary(&blits(&mut c));
        // 前四個是砲座本體，接著噴焰：朝右 x + 2、y + 5 + 2，第二座再加 50。
        assert_eq!(s[4..8], [
            (Op::S18D46, 32, 47, 0x0A),
            (Op::S18CE2, 32, 47, 0x0A),
            (Op::S18D46, 32, 97, 0x0A),
            (Op::S18CE2, 32, 97, 0x0A),
        ]);
        // 朝左：x - 1。
        let mut c = fresh();
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].fired = YES;
        c.pl.ship[0].pods = super::super::Pods { first: YES, second: NO, facing: FACING_LEFT, image: 0x20, anim: 0, col: 3, row: 2, blit_x: 30, blit_y: 40 };
        c.draw_player(0);
        let s = summary(&blits(&mut c));
        assert_eq!(s[2..4], [(Op::S18D46, 29, 47, 0x0A), (Op::S18CE2, 29, 47, 0x0A)]);
    }

    #[test]
    fn pod_scans_write_the_shape_widths_in_the_original_order() {
        // 第一座：砲座碰撞寫 2（G1）再寫 5（G2），第一個繪製命令帶 5，其餘沒有；
        // 第二座的碰撞在畫之後，留下的 5 由下一個命令（本體遮罩）帶走。
        let mut c = fresh();
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].pods = super::super::Pods { first: YES, second: YES, facing: FACING_RIGHT, image: 0x20, anim: 0, col: 3, row: 2, blit_x: 30, blit_y: 40 };
        c.draw_player(0);
        let strides: Vec<Option<u16>> = blits(&mut c).iter().map(|b| b.stride_reg).collect();
        assert_eq!(strides[..5], [Some(5), None, None, None, Some(5)], "砲座 1 帶 5、砲座 2 沒有、本體遮罩帶第二座碰撞的 5");
    }
}
