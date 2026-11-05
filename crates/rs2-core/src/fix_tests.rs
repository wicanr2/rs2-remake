//! A2b：每個 `Fixes` 旗標一組合成情境。直接擺出觸發條件，關時重現原版結果，開時得到修正結果。
//! 不用原版檔：資料段全 0，只擺這個情境需要的值。

use crate::dseg::a;
use crate::game::{Core, Fixes, Start};

fn game(fixes: Fixes) -> Core {
    let image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
    Core::new(&image, &[], Start::Demo, fixes)
}

const G2: u16 = 0x2C1C;
const EB_TABLE: u16 = 0x32B3;

/// 把敵彈槽 `slot` 的 1×1 形狀寫進 G2，回傳寫到的位址。走真正的 `sub_17855`。
fn write_bullet_cell(g: &mut Core, slot: u8, shape: u8) -> u16 {
    let s = 0x7000u16;
    g.ds.w16(s, 1);
    g.ds.w16(s + 2, 1);
    g.ds.w8(s + 4, shape);
    g.ds.w16(a(0x20111), s);
    g.ds.w16(a(0x20109), 5);
    g.ds.w16(a(0x20107), 3);
    g.ds.w8(a(0x1F7DB), slot);
    // 列表 `列 × 28h`。
    for r in 0..20u16 {
        g.ds.w16(0x1B0A + r * 2, r * 0x28);
    }
    g.s17855();
    G2 + 5 * 0x28 + 3
}

#[test]
fn bullet_slot_carry_off_reproduces_wraparound() {
    let mut g = game(Fixes::NONE);
    for i in 0..20u16 {
        g.ds.w16(EB_TABLE + i * 2, 0x1234);
    }
    // 槽 16：格值 01 + 16 = 11h，and 0Fh 減 1 得槽 0。
    let di = write_bullet_cell(&mut g, 16, 0x01);
    assert_eq!(g.ds.r8(di), 0x11);
    g.r.di = di;
    g.r.set_al(g.ds.r8(di));
    g.s14d54();
    assert_eq!(g.ds.r16(EB_TABLE), 0, "原版刪掉槽 0");
    assert_eq!(g.ds.r16(EB_TABLE + 32), 0x1234, "原版留下擊中的槽 16");
}

#[test]
fn bullet_slot_carry_off_slot15_clobbers_record() {
    let mut g = game(Fixes::NONE);
    g.ds.w16(0x34B1, 0x0A);
    // 槽 15：格值 01 + 15 = 10h，and 0Fh 減 1 得 FFh，寫到 34B1h。
    let di = write_bullet_cell(&mut g, 15, 0x01);
    g.r.di = di;
    g.r.set_al(g.ds.r8(di));
    g.s14d54();
    assert_eq!(g.ds.r16(0x34B1), 0, "原版把 0 寫到 34B1h");
}

#[test]
fn bullet_slot_carry_on_removes_the_hit_bullet() {
    let mut g = game(Fixes::ALL);
    for i in 0..20u16 {
        g.ds.w16(EB_TABLE + i * 2, 0x1234);
    }
    g.ds.w16(0x34B1, 0x0A);
    for slot in [15u8, 16, 19] {
        g.ds.w16(EB_TABLE + slot as u16 * 2, 0x1234);
        let di = write_bullet_cell(&mut g, slot, 0x01);
        g.r.di = di;
        g.r.set_al(g.ds.r8(di));
        g.s14d54();
        assert_eq!(g.ds.r16(EB_TABLE + slot as u16 * 2), 0, "要刪掉擊中的槽 {slot}");
        assert_eq!(g.ds.r16(EB_TABLE), 0x1234, "槽 0 不受影響");
        assert_eq!(g.ds.r16(0x34B1), 0x0A, "34B1h 不受影響");
    }
}

#[test]
fn bullet_slot_carry_on_keeps_item_classification() {
    // 道具形狀 81h 配上槽號 15 以上，原版進位成 90h，判定落空變成傷害。
    let mut off = game(Fixes::NONE);
    let di = write_bullet_cell(&mut off, 16, 0x81);
    off.r.set_al(off.ds.r8(di));
    off.s14cd4();
    assert_eq!(off.ds.r8(a(0x1DAC0)), b'Y', "原版當成傷害");

    let mut on = game(Fixes::ALL);
    let di = write_bullet_cell(&mut on, 16, 0x81);
    on.r.set_al(on.ds.r8(di));
    on.s14cd4();
    assert_eq!(on.ds.r8(a(0x1DAA8)), 0x80, "修正後判定成道具");
}

#[test]
fn bullet_slot_carry_on_clears_with_g2() {
    let mut g = game(Fixes::ALL);
    let di = write_bullet_cell(&mut g, 16, 0x01);
    assert_eq!(g.g2_slots[(di - G2) as usize], 17);
    g.s178bb();
    assert_eq!(g.g2_slots[(di - G2) as usize], 0);
}

#[test]
fn armor_cell_off_writes_dseg_0020() {
    let mut g = game(Fixes::NONE);
    g.ds.w16(0x0020, 0x0165);
    g.ds.w8(a(0x1F1C0), 0xFE);
    g.s16946();
    assert_eq!(g.ds.r16(0x0020), 0x0166, "原版以指標 0 加 20h 寫到 dseg:0020h");
}

#[test]
fn armor_cell_on_leaves_dseg_0020() {
    let mut g = game(Fixes::ALL);
    g.ds.w16(0x0020, 0x0165);
    g.ds.w8(a(0x1F1C0), 0xFE);
    g.s16946();
    assert_eq!(g.ds.r16(0x0020), 0x0165);
    // 已釋放的槽（指標 0）也不寫。
    g.ds.w8(a(0x1F1C0), 0x03);
    g.s16946();
    assert_eq!(g.ds.r16(0x0020), 0x0165);
    // 活著的實體照常受傷。
    let ent = 0x22B4u16;
    g.ds.w16(0x2294 + 2, ent);
    g.ds.w16(ent + 0x20, 0);
    g.ds.w8(a(0x1F1C0), 0x02);
    g.s16946();
    assert_eq!(g.ds.r16(ent + 0x20), 0x0101);
}

#[test]
fn armor_cell_pod_contact_path() {
    let mut off = game(Fixes::NONE);
    off.ds.w16(0x0020, 0x0165);
    off.ds.w8(a(0x1DA8A), 0xFE);
    off.s14c18();
    assert_eq!(off.ds.r16(0x0020), 0x0166);

    let mut on = game(Fixes::ALL);
    on.ds.w16(0x0020, 0x0165);
    on.ds.w8(a(0x1DA8A), 0xFE);
    on.s14c18();
    assert_eq!(on.ds.r16(0x0020), 0x0165);
}

/// 射一次 `B` 武器，池內先佔滿 `occupied` 個槽。回傳 `word_1F1A8` 增加量。
fn fire_b(fixes: Fixes, occupied: u16, facing: u16) -> u16 {
    let mut g = game(fixes);
    for i in 0..occupied {
        g.ds.w16(0x2607 + i * 2, 0x262F + i * 0x20);
    }
    g.ds.w16(a(0x1F1A8), 0);
    g.ds.w16(a(0x1F1BC), b'B' as u16);
    g.ds.w16(a(0x1F1BE), facing);
    g.s165cf();
    g.ds.r16(a(0x1F1A8))
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
    g.ds.w8(a(0x1CFAA), b'N');
    g.s1316c();
}

#[test]
#[should_panic(expected = "131CE")]
fn no_hit_hang_off_reproduces_the_hang() {
    let mut g = game(Fixes::NONE);
    g.ds.w8(a(0x1CFAA), b'N');
    g.s1316c();
}
