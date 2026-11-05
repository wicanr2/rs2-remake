//! A2b：每個 `Fixes` 旗標一組合成情境。直接擺出觸發條件，關時重現原版結果，開時得到修正結果。
//! 不用原版檔：資料段全 0，只擺這個情境需要的值。

use crate::ebullet::Bullet;
use crate::game::{Core, Fixes, Start};
use crate::mirror::{enemy_record_link, pb_record_link};
use crate::pbullet::{Facing, PlayerBullet};
use crate::player::Pickup;

fn game(fixes: Fixes) -> Core {
    let image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
    Core::new(&image, &[], Start::Demo, fixes)
}

const G2: u16 = 0x2C1C;

/// 把敵彈槽 `slot` 的 1×1 形狀寫進 G2，回傳寫到的位址。走真正的 `stamp_grid2`。
fn write_bullet_cell(g: &mut Core, slot: u8, shape: u8) -> u16 {
    let s = 0x7000u16;
    g.ds.w16(s, 1);
    g.ds.w16(s + 2, 1);
    g.ds.w8(s + 4, shape);
    // 列表 `列 × 28h`。
    for r in 0..20u16 {
        g.ds.w16(0x1B0A + r * 2, r * 0x28);
    }
    let bullet = Bullet { shape: s, col: 3, row: 5, ..Bullet::default() };
    g.stamp_grid2(&bullet, 1, 1, slot);
    G2 + 5 * 0x28 + 3
}

/// 玩家碰到 `di` 的格：刪掉那一顆敵彈。
fn hit(g: &mut Core, di: u16) -> Option<u16> {
    g.remove_hit_bullet(g.ds.r8(di), di)
}

#[test]
fn bullet_slot_carry_off_reproduces_wraparound() {
    let mut g = game(Fixes::NONE);
    for s in g.eb.slots.iter_mut() {
        s.link = 0x1234;
    }
    // 槽 16：格值 01 + 16 = 11h，and 0Fh 減 1 得槽 0。
    let di = write_bullet_cell(&mut g, 16, 0x01);
    assert_eq!(g.ds.r8(di), 0x11);
    assert_eq!(hit(&mut g, di), Some(0), "原版把槽號 × 2 留在 bx");
    assert_eq!(g.eb.slots[0].link, 0, "原版刪掉槽 0");
    assert_eq!(g.eb.slots[16].link, 0x1234, "原版留下擊中的槽 16");
}

#[test]
fn bullet_slot_carry_off_slot15_clobbers_record() {
    let mut g = game(Fixes::NONE);
    g.eb.slots[15].bullet.row = 0x0A;
    // 槽 15：格值 01 + 15 = 10h，and 0Fh 減 1 得 FFh，寫到池指標表第 255 項，也就是槽 15 記錄的 w10。
    let di = write_bullet_cell(&mut g, 15, 0x01);
    assert_eq!(hit(&mut g, di), Some(0x1FE));
    assert_eq!(g.eb.slots[15].bullet.row, 0, "原版把 0 寫到 34B1h");
}

#[test]
fn bullet_slot_carry_on_removes_the_hit_bullet() {
    let mut g = game(Fixes::ALL);
    for s in g.eb.slots.iter_mut() {
        s.link = 0x1234;
    }
    g.eb.slots[15].bullet.row = 0x0A;
    for slot in [15u8, 16, 19] {
        g.eb.slots[slot as usize].link = 0x1234;
        let di = write_bullet_cell(&mut g, slot, 0x01);
        assert_eq!(hit(&mut g, di), Some(slot as u16 * 2));
        assert_eq!(g.eb.slots[slot as usize].link, 0, "要刪掉擊中的槽 {slot}");
        assert_eq!(g.eb.slots[0].link, 0x1234, "槽 0 不受影響");
        assert_eq!(g.eb.slots[15].bullet.row, 0x0A, "槽 15 的記錄不受影響");
    }
}

#[test]
fn bullet_slot_carry_on_leaves_bx_alone_when_no_slot_is_recorded() {
    let mut g = game(Fixes::ALL);
    assert_eq!(hit(&mut g, G2 + 7), None, "G2 這格沒有另存的槽號");
    assert_eq!(g.remove_hit_bullet(0x05, 0xFFFF), None, "位址在 G2 之外");
}

#[test]
fn bullet_slot_carry_on_keeps_item_classification() {
    // 道具形狀 81h 配上槽號 15 以上，原版進位成 90h，判定落空變成傷害。
    let mut off = game(Fixes::NONE);
    let di = write_bullet_cell(&mut off, 16, 0x81);
    let cell = off.ds.r8(di);
    assert_eq!(off.touch_bullet_cell(0, cell), None, "原版進位後判定落空");
    assert_eq!(off.pl.ship[0].hit, b'Y', "原版當成傷害");

    let mut on = game(Fixes::ALL);
    let di = write_bullet_cell(&mut on, 16, 0x81);
    let cell = on.ds.r8(di);
    assert_eq!(on.touch_bullet_cell(0, cell), Some(Pickup::Laser), "修正後判定成道具");
    assert_ne!(on.pl.ship[0].hit, b'Y');
}

#[test]
fn bullet_slot_carry_on_clears_with_g2() {
    let mut g = game(Fixes::ALL);
    let di = write_bullet_cell(&mut g, 16, 0x01);
    assert_eq!(g.g2_slots[(di - G2) as usize], 17);
    g.clear_grid2();
    assert_eq!(g.g2_slots[(di - G2) as usize], 0);
}

#[test]
fn armor_cell_off_writes_dseg_0020() {
    let mut g = game(Fixes::NONE);
    g.ds.w16(0x0020, 0x0165);
    g.strike(&PlayerBullet::default(), 0xFE);
    assert_eq!(g.ds.r16(0x0020), 0x0166, "原版以指標 0 加 20h 寫到 dseg:0020h");
}

#[test]
fn armor_cell_on_leaves_dseg_0020() {
    let mut g = game(Fixes::ALL);
    g.ds.w16(0x0020, 0x0165);
    let bullet = PlayerBullet::default();
    g.strike(&bullet, 0xFE);
    assert_eq!(g.ds.r16(0x0020), 0x0165);
    // 已釋放的槽（指標 0）也不寫。
    g.strike(&bullet, 0x03);
    assert_eq!(g.ds.r16(0x0020), 0x0165);
    // 活著的敵人照常受傷。
    g.en.slots[1].link = enemy_record_link(1);
    g.strike(&bullet, 0x02);
    assert_eq!(g.en.slots[1].enemy.damage, 0x0101);
}

#[test]
fn armor_cell_pod_contact_path() {
    // 砲座接觸與玩家彈共用 `damage_entity_at_cell`；整條掃描路徑在 `player/contact.rs` 的測試。
    let mut off = game(Fixes::NONE);
    off.ds.w16(0x0020, 0x0165);
    off.damage_entity_at_cell(0xFE, 1);
    assert_eq!(off.ds.r16(0x0020), 0x0166);

    let mut on = game(Fixes::ALL);
    on.ds.w16(0x0020, 0x0165);
    on.damage_entity_at_cell(0xFE, 1);
    assert_eq!(on.ds.r16(0x0020), 0x0165);
}

/// 射一次 `B` 武器，池內先佔滿 `occupied` 個槽。回傳 `word_1F1A8` 增加量。
fn fire_b(fixes: Fixes, occupied: u16, facing: u16) -> u16 {
    let mut g = game(fixes);
    for i in 0..occupied as usize {
        g.pb.slots[i].link = pb_record_link(i);
    }
    g.pb.seekers_in_flight = 0;
    g.fire_player_bullets(b'B' as u16, Facing::from_word(facing), 0, 0);
    g.pb.seekers_in_flight
}

#[test]
fn seeker_spawn_leak_off_always_adds_two() {
    for facing in [0x3E, 0x3C] {
        assert_eq!(fire_b(Fixes::NONE, 0, facing), 2);
        assert_eq!(fire_b(Fixes::NONE, 19, facing), 2, "只配到一顆仍加 2");
        assert_eq!(fire_b(Fixes::NONE, 20, facing), 2, "一顆都沒配到仍加 2");
    }
}

#[test]
fn seeker_spawn_leak_on_counts_actual_spawns() {
    for facing in [0x3E, 0x3C] {
        assert_eq!(fire_b(Fixes::ALL, 0, facing), 2);
        assert_eq!(fire_b(Fixes::ALL, 19, facing), 1);
        assert_eq!(fire_b(Fixes::ALL, 20, facing), 0);
    }
}

/// 第二關幀 260 的中頭目合成：沒被打中過（`byte_1CFAA` 是 `N`）時原版永遠卡住，修正後照常往下走。
#[test]
fn no_hit_hang_on_continues() {
    let mut g = game(Fixes::ALL);
    g.flow.integrity_mark = b'N';
    g.compose_stage2_midboss_art();
}

#[test]
#[should_panic(expected = "131CE")]
fn no_hit_hang_off_reproduces_the_hang() {
    let mut g = game(Fixes::NONE);
    g.flow.integrity_mark = b'N';
    g.compose_stage2_midboss_art();
}
