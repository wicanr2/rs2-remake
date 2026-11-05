//! `VgaSink` 的單元測試：用自製的小圖庫驗證裁切、寫入方式與平面對應。
//! 不含原版位元組；與原版逐像素的對拍在 `first_frame.rs` 與 `rs2-trace` 的 A5。

use rs2_core::{BlitCmd, DrawCmd, Op, ShtGroup};
use rs2_formats::{BankItem, GoFile};
use rs2_gfx::{Assets, VgaSink};

fn item(height: u16, stride: u16, data: &[u8]) -> BankItem {
    assert_eq!(data.len(), height as usize * stride as usize);
    BankItem { height, stride, data: data.to_vec() }
}

fn empty_go() -> GoFile {
    GoFile { mask: None, planes: [vec![], vec![], vec![], vec![]] }
}

/// 四個平面區段各一個項目 0；`planes[k]` 對應 VGA 平面 `3 - k`。
fn go(items: [BankItem; 4], mask: Option<BankItem>) -> GoFile {
    let [a, b, c, d] = items;
    GoFile { mask: mask.map(|m| vec![m]), planes: [vec![a], vec![b], vec![c], vec![d]] }
}

fn sink(my: GoFile) -> VgaSink {
    VgaSink::new(Assets { my, sht1: empty_go(), bh1: empty_go() })
}

fn cmd(op: Op) -> BlitCmd {
    BlitCmd {
        op,
        x: 0,
        y: 0,
        index: 0,
        page: 0,
        scratch: 0,
        src_x: 0,
        src_y: 0,
        shift: 0,
        cx: 0,
        dx: 0,
        text_x: 0x12,
        text_y: 0xBF,
        sht_group: ShtGroup::Sht1,
        obj: None,
    }
}

fn blit(s: &mut VgaSink, c: BlitCmd) {
    s.apply(&DrawCmd::Blit(c));
}

fn all_planes(h: u16, w: u16, data: &[u8]) -> [BankItem; 4] {
    [item(h, w, data), item(h, w, data), item(h, w, data), item(h, w, data)]
}

#[test]
fn plane_order_first_section_is_plane_3() {
    let g = go(
        [item(1, 1, &[0x11]), item(1, 1, &[0x22]), item(1, 1, &[0x33]), item(1, 1, &[0x44])],
        None,
    );
    let mut s = sink(g);
    blit(&mut s, BlitCmd { x: 3, y: 5, ..cmd(Op::S18C0A) });
    let off = 5 * 40 + 3;
    assert_eq!([s.plane(3)[off], s.plane(2)[off], s.plane(1)[off], s.plane(0)[off]], [0x11, 0x22, 0x33, 0x44]);
    assert_eq!(s.anomalies(), 0);
}

#[test]
fn or_keeps_existing_bits_and_mov_replaces() {
    let mut s = sink(go(all_planes(1, 2, &[0xF0, 0x01]), None));
    blit(&mut s, BlitCmd { x: 1, ..cmd(Op::S18C0A) }); // 覆寫：欄 1、2 = F0、01
    blit(&mut s, BlitCmd { x: 0, ..cmd(Op::S18CE2) }); // OR：欄 0、1 |= F0、01
    assert_eq!([s.plane(0)[0], s.plane(0)[1], s.plane(0)[2]], [0xF0, 0xF1, 0x01]);
    // 覆寫會蓋掉原有位元。
    blit(&mut s, BlitCmd { x: 0, ..cmd(Op::S18C0A) });
    assert_eq!([s.plane(0)[0], s.plane(0)[1], s.plane(0)[2]], [0xF0, 0x01, 0x01]);
}

#[test]
fn negative_y_skips_leading_rows_and_advances_source() {
    let g = go(all_planes(4, 1, &[1, 2, 3, 4]), None);
    let mut s = sink(g);
    blit(&mut s, BlitCmd { y: 0xFFFE, ..cmd(Op::S18C0A) });
    assert_eq!([s.plane(0)[0], s.plane(0)[40], s.plane(0)[80]], [3, 4, 0]);
}

#[test]
fn rows_below_189_are_not_drawn() {
    let g = go(all_planes(4, 1, &[1, 2, 3, 4]), None);
    let mut s = sink(g);
    blit(&mut s, BlitCmd { y: 188, ..cmd(Op::S18C0A) });
    assert_eq!([s.plane(1)[188 * 40], s.plane(1)[189 * 40], s.plane(1)[190 * 40], s.plane(1)[191 * 40]], [1, 2, 0, 0]);
}

#[test]
fn row_is_clipped_to_its_own_40_bytes() {
    let g = go(all_planes(2, 3, &[1, 2, 3, 4, 5, 6]), None);
    let mut s = sink(g);
    blit(&mut s, BlitCmd { x: 39, ..cmd(Op::S18C0A) });
    // 第 0 列只有欄 39 落在列內；第 1 列的欄 0 不能被 2、3 溢寫。
    assert_eq!([s.plane(0)[39], s.plane(0)[40], s.plane(0)[41], s.plane(0)[79]], [1, 0, 0, 4]);
}

#[test]
fn negative_x_clips_left_side_without_touching_previous_row() {
    let g = go(all_planes(1, 2, &[7, 8]), None);
    let mut s = sink(g);
    blit(&mut s, BlitCmd { x: 0xFFFF, ..cmd(Op::S18C0A) });
    assert_eq!(s.plane(0)[0], 8);
    assert_eq!(s.plane(0)[0xFFFF], 0);
    let mut s = sink(go(all_planes(1, 2, &[7, 8]), None));
    blit(&mut s, BlitCmd { x: 0xFFFF, y: 1, page: 0x8000, ..cmd(Op::S18C0A) });
    assert_eq!([s.plane(0)[0x8000 + 40], s.plane(0)[0x8000 + 39]], [8, 0]);
}

#[test]
fn mask_and_clears_bits_in_all_planes_and_inverted_mask_sets_them() {
    let mask = item(1, 1, &[0x0F]);
    let mut s = sink(go(all_planes(1, 1, &[0xFF]), Some(mask.clone())));
    blit(&mut s, cmd(Op::S18C0A));
    blit(&mut s, cmd(Op::S18D46));
    for p in 0..4 {
        assert_eq!(s.plane(p)[0], 0x0F, "平面 {p}");
    }
    // 閃白：反相遮罩 OR 進去，剪影外（遮罩為 0 的位元）全平面設 1。
    let mut s = sink(go(all_planes(1, 1, &[0x00]), Some(mask)));
    blit(&mut s, cmd(Op::S18D89));
    for p in 0..4 {
        assert_eq!(s.plane(p)[0], 0xF0, "平面 {p}");
    }
}

#[test]
fn scratch_write_drops_last_byte_of_odd_stride_and_moves_source_by_even_bytes() {
    let g = go([item(2, 3, &[1, 2, 3, 4, 5, 6]), item(2, 3, &[0; 6]), item(2, 3, &[0; 6]), item(2, 3, &[0; 6])], None);
    // 暫存區來源是 BH1／SHT1 組，這裡把圖庫放在 sht1。
    let mut s = VgaSink::new(Assets { my: empty_go(), sht1: g, bh1: empty_go() });
    blit(&mut s, BlitCmd { x: 2, y: 1, scratch: 0x8000, ..cmd(Op::S19ADC) });
    let base = 0x1F40 + 80 + 0x8000 + 2;
    assert_eq!([s.plane(3)[base], s.plane(3)[base + 1], s.plane(3)[base + 2]], [1, 2, 0]);
    // 第 2 列來源從第 3 個位元組開始（每列前進 2），不是第 4 個。
    assert_eq!([s.plane(3)[base + 80], s.plane(3)[base + 81]], [3, 4]);
}

#[test]
fn scratch_group_selects_bh_or_sht() {
    let sht = go(all_planes(1, 2, &[0xAA, 0xAA]), None);
    let bh = go(all_planes(1, 2, &[0x55, 0x55]), None);
    let mut s = VgaSink::new(Assets { my: empty_go(), sht1: sht, bh1: bh });
    blit(&mut s, BlitCmd { sht_group: ShtGroup::Bh1, ..cmd(Op::S19ADC) });
    assert_eq!(s.plane(0)[0x1F40], 0x55);
    blit(&mut s, BlitCmd { sht_group: ShtGroup::Sht1, y: 1, ..cmd(Op::S19ADC) });
    assert_eq!(s.plane(0)[0x1F40 + 80], 0xAA);
}

#[test]
fn fill_uses_write_mode_2_and_stops_at_row_189() {
    let mut s = sink(empty_go());
    blit(&mut s, BlitCmd { y: 187, cx: 5, dx: 5, ..cmd(Op::S1A287) });
    for r in [187usize, 188, 189] {
        for c in [0usize, 39] {
            let o = r * 40 + c;
            assert_eq!([s.plane(0)[o], s.plane(1)[o], s.plane(2)[o], s.plane(3)[o]], [0xFF, 0, 0xFF, 0], "列 {r} 欄 {c}");
        }
    }
    assert_eq!(s.plane(0)[190 * 40], 0);
    // 寫入模式回到 0：之後的覆寫 blit 正常。
    let mut s = sink(go(all_planes(1, 1, &[0x3C]), None));
    blit(&mut s, BlitCmd { cx: 1, dx: 15, ..cmd(Op::S1A287) });
    blit(&mut s, cmd(Op::S18C0A));
    assert_eq!(s.plane(2)[0], 0x3C);
}

#[test]
fn blood_bar_writes_both_pages_and_ignores_row_limit() {
    let mut s = sink(empty_go());
    blit(&mut s, BlitCmd { x: 2, y: 190, dx: 3, page: 0x8000, ..cmd(Op::S1929F) });
    for r in 190..197usize {
        for base in [0usize, 0x8000] {
            let o = base + r * 40 + 2;
            assert_eq!([s.plane(0)[o], s.plane(1)[o], s.plane(2)[o], s.plane(3)[o]], [0xFF, 0xFF, 0, 0], "列 {r} 基底 {base:X}");
        }
    }
    assert_eq!(s.plane(0)[197 * 40 + 2], 0);
}

#[test]
fn clear_page_clears_190_rows_only() {
    let mut s = sink(empty_go());
    blit(&mut s, BlitCmd { cx: 190, dx: 15, page: 0x8000, ..cmd(Op::S1A287) });
    blit(&mut s, BlitCmd { cx: 190, dx: 15, ..cmd(Op::S1A287) });
    blit(&mut s, BlitCmd { x: 2, y: 190, dx: 15, page: 0x8000, ..cmd(Op::S1929F) });
    blit(&mut s, BlitCmd { page: 0x8000, ..cmd(Op::S19CF9) });
    assert_eq!(s.plane(0)[0x8000], 0);
    assert_eq!(s.plane(3)[0x8000 + 189 * 40 + 39], 0);
    assert_eq!(s.plane(0)[0x8000 + 190 * 40 + 2], 0xFF, "介面列不清");
    assert_eq!(s.plane(0)[0], 0xFF, "另一頁不受影響");
}

fn scratch_pattern() -> VgaSink {
    // 暫存區 A 第 0、1 列：每個平面 4 位元組，值依平面區分。
    let mk = |base: u8| item(2, 4, &[base + 1, base + 2, base + 3, base + 4, base + 5, base + 6, base + 7, base + 8]);
    let g = go([mk(0x30), mk(0x20), mk(0x10), mk(0x00)], None);
    let mut s = VgaSink::new(Assets { my: empty_go(), sht1: g, bh1: empty_go() });
    blit(&mut s, cmd(Op::S19ADC));
    s
}

#[test]
fn scratch_copy_is_latch_copy_of_all_planes_with_stride_80() {
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { y: 3, cx: 2, ..cmd(Op::S19BA4) });
    for (p, base) in [(3usize, 0x30u8), (2, 0x20), (1, 0x10), (0, 0x00)] {
        let o = 3 * 40;
        assert_eq!(&s.plane(p)[o..o + 5], &[base + 1, base + 2, base + 3, base + 4, 0], "平面 {p} 列 3");
        let o = 4 * 40;
        assert_eq!(&s.plane(p)[o..o + 5], &[base + 5, base + 6, base + 7, base + 8, 0], "平面 {p} 列 4");
    }
}

#[test]
fn scratch_copy_with_shift_skips_columns_outside_0_to_4f() {
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { y: 0, cx: 1, shift: 0xFFFE, ..cmd(Op::S19D08) });
    // 位移 -2：目的欄 0、1 不寫，欄 2 起取暫存區欄 0 起。
    assert_eq!(&s.plane(0)[0..6], &[0, 0, 1, 2, 3, 4]);
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { y: 0, cx: 1, shift: 2, ..cmd(Op::S19D08) });
    assert_eq!(&s.plane(0)[0..4], &[3, 4, 0, 0]);
}

#[test]
fn scratch_or_is_per_plane_and_and_uses_stale_read_plane() {
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { cx: 1, dx: 15, ..cmd(Op::S1A287) }); // 頁 0 列 0 填色 15（全 FF）
    blit(&mut s, BlitCmd { y: 0, cx: 1, ..cmd(Op::S19DA8) }); // OR 後仍是 FF
    assert_eq!(s.plane(3)[0], 0xFF);
    // AND 讀的是 GC 索引 4 殘留的平面（四平面 blit 的最後一次選了平面 0），所以遮罩取平面 0 的暫存區。
    blit(&mut s, BlitCmd { y: 0, cx: 1, ..cmd(Op::S1A09F) });
    let m = [1u8, 2, 3, 4];
    for p in 0..4 {
        assert_eq!(&s.plane(p)[0..5], &[m[0], m[1], m[2], m[3], 0], "平面 {p}");
    }
}

#[test]
fn counter_is_drawn_only_when_enabled_and_only_in_plane_3() {
    let mut s = sink(empty_go());
    blit(&mut s, BlitCmd { cx: 12345, ..cmd(Op::S1890D) });
    assert!(s.plane(3).iter().all(|&b| b == 0), "預設不畫");
    s.show_counter = true;
    blit(&mut s, BlitCmd { cx: 12345, ..cmd(Op::S1890D) });
    let o = 191 * 40 + 18;
    assert_eq!(s.plane(3)[o], 0x18, "第一位數字 1 的第一列");
    assert_eq!(s.plane(3)[o + 0x8000], 0x18, "兩頁都畫");
    assert!((0..3).all(|p| s.plane(p).iter().all(|&b| b == 0)));
}

#[test]
fn dac_and_render() {
    let mut s = sink(empty_go());
    let mut rgb = [0u8; 48];
    rgb[3..6].copy_from_slice(&[63, 0, 32]);
    s.apply(&DrawCmd::SetDac { start: 0, rgb });
    let mut hi = [0u8; 48];
    hi[24..27].copy_from_slice(&[10, 20, 30]); // 索引 0x30 + 8 = 56，色號 8
    s.apply(&DrawCmd::SetDac { start: 0x30, rgb: hi });
    assert_eq!(s.dac(1), [63, 0, 32]);
    assert_eq!(s.dac(56), [10, 20, 30]);
    assert_eq!(s.shown16()[1], [63, 0, 32]);
    assert_eq!(s.shown16()[8], [10, 20, 30]);
    // 頁 0 列 0 欄 0 填色 1，頁 1 同位置填色 8。
    blit(&mut s, BlitCmd { cx: 1, dx: 1, ..cmd(Op::S1A287) });
    blit(&mut s, BlitCmd { cx: 1, dx: 8, page: 0x8000, ..cmd(Op::S1A287) });
    let mut img = vec![0u8; 320 * 200 * 3];
    s.render_page(0, &mut img);
    assert_eq!(&img[0..3], &[255, 0, 130]);
    assert_eq!(&img[7 * 3..7 * 3 + 3], &[255, 0, 130]);
    s.render_page(0x8000, &mut img);
    assert_eq!(&img[0..3], &[40, 81, 121]);
}
