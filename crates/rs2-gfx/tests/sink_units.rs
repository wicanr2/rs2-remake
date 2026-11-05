//! `VgaSink` 的單元測試：用自製的小圖庫驗證裁切、寫入方式與平面對應。
//! 不含原版位元組；與原版逐像素的對拍在 `first_frame.rs` 與 `rs2-trace` 的 A5。

use rs2_core::{BegGroup, BlitCmd, DrawCmd, Op, ShtGroup};
use rs2_formats::{BankItem, GoFile};
use rs2_gfx::{ATTR_REGS, Assets, VgaSink};

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
    VgaSink::new(Assets { my, ..Assets::empty() })
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
        mask_group: ShtGroup::Sht1,
        beg_group: rs2_core::BegGroup::Beg1,
        stride_reg: None,
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
    let mut s = VgaSink::new(Assets { sht1: g, ..Assets::empty() });
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
    let mut s = VgaSink::new(Assets { sht1: sht, bh1: bh, ..Assets::empty() });
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
    let mut s = VgaSink::new(Assets { sht1: g, ..Assets::empty() });
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

// ---------------------------------------------------------------------------
// 全遊戲新增的命令（規格 002）。預期值都是照 `tools/q.sh func sub_X` 讀到的機器碼手算。
// ---------------------------------------------------------------------------

fn planes_at(s: &VgaSink, off: usize) -> [u8; 4] {
    [s.plane(0)[off], s.plane(1)[off], s.plane(2)[off], s.plane(3)[off]]
}

#[test]
fn map_mask_and_vram_fill_select_planes_and_restore_after_blits() {
    let mut s = sink(empty_go());
    s.apply(&DrawCmd::SetMapMask { mask: 4 });
    s.apply(&DrawCmd::VramFill { off: 100, words: 2, value: 0xA55A });
    // rep stosw：低位元組在前，只寫平面 2。
    assert_eq!(planes_at(&s, 100), [0, 0, 0x5A, 0]);
    assert_eq!(planes_at(&s, 101), [0, 0, 0xA5, 0]);
    assert_eq!(planes_at(&s, 103), [0, 0, 0xA5, 0]);
    assert_eq!(planes_at(&s, 104), [0; 4]);
    s.apply(&DrawCmd::SetMapMask { mask: 0x0F });
    s.apply(&DrawCmd::VramFill { off: 200, words: 1, value: 0x1234 });
    assert_eq!(planes_at(&s, 200), [0x34; 4]);
    assert_eq!(planes_at(&s, 201), [0x12; 4]);
    // 偏移環繞 16 位元。
    s.apply(&DrawCmd::VramFill { off: 0xFFFE, words: 2, value: 0x00FF });
    assert_eq!(planes_at(&s, 0xFFFE), [0xFF; 4]);
    assert_eq!(planes_at(&s, 0xFFFF), [0; 4]);
    assert_eq!(planes_at(&s, 0), [0xFF; 4]);
    // 四平面 blit 結束時回到 0Fh。
    s.apply(&DrawCmd::SetMapMask { mask: 1 });
    blit(&mut s, cmd(Op::S1929F));
    s.apply(&DrawCmd::VramFill { off: 300, words: 1, value: 0x0001 });
    assert_eq!(planes_at(&s, 300), [1, 0, 0, 0], "S1929F 不動 map mask");
    let g = go(all_planes(1, 1, &[0xFF]), None);
    let mut s = sink(g);
    s.apply(&DrawCmd::SetMapMask { mask: 2 });
    blit(&mut s, cmd(Op::S18C0A));
    s.apply(&DrawCmd::VramFill { off: 300, words: 1, value: 0x0001 });
    assert_eq!(planes_at(&s, 300), [1; 4], "sub_18878 把 map mask 設回 0Fh");
}

#[test]
fn clear_scratch_a_and_b_follow_the_current_map_mask_and_do_not_restore_it() {
    let mut s = sink(empty_go());
    // 填到比清除範圍多一個字組，才分得出範圍的終點。
    s.apply(&DrawCmd::VramFill { off: 0x1F40, words: 0x20C0, value: 0xFFFF });
    s.apply(&DrawCmd::VramFill { off: 0x9F40, words: 0x20C0, value: 0xFFFF });
    s.apply(&DrawCmd::SetMapMask { mask: 2 });
    blit(&mut s, cmd(Op::S11504));
    // 範圍是 1F40h 起 417Eh 位元組：最後一個被清的是 60BDh，60BEh 保留。
    assert_eq!(planes_at(&s, 0x1F40), [0xFF, 0, 0xFF, 0xFF]);
    assert_eq!(planes_at(&s, 0x60BD), [0xFF, 0, 0xFF, 0xFF]);
    assert_eq!(planes_at(&s, 0x60BE), [0xFF; 4]);
    assert_eq!(planes_at(&s, 0x9F40), [0xFF; 4], "暫存區 B 不動");
    // 沒有 sub_18878：map mask 仍是 2，接著清 B 也只清平面 1。
    blit(&mut s, cmd(Op::S1152D));
    assert_eq!(planes_at(&s, 0x9F40), [0xFF, 0, 0xFF, 0xFF]);
    assert_eq!(planes_at(&s, 0xE0BD), [0xFF, 0, 0xFF, 0xFF]);
    assert_eq!(planes_at(&s, 0xE0BE), [0xFF; 4]);
}

#[test]
fn clear_both_pages_restores_map_mask_and_stops_at_the_page_end() {
    let mut s = sink(empty_go());
    s.apply(&DrawCmd::VramFill { off: 0, words: 0x1000, value: 0xFFFF });
    s.apply(&DrawCmd::VramFill { off: 0x8000, words: 0x1000, value: 0xFFFF });
    s.apply(&DrawCmd::SetMapMask { mask: 1 });
    blit(&mut s, cmd(Op::S18894));
    for base in [0usize, 0x8000] {
        assert_eq!(planes_at(&s, base), [0; 4]);
        assert_eq!(planes_at(&s, base + 0x1F3F), [0; 4], "200 列的最後一個位元組");
        assert_eq!(planes_at(&s, base + 0x1F40), [0xFF; 4], "頁後面不清");
    }
    s.apply(&DrawCmd::VramFill { off: 10, words: 1, value: 0x0077 });
    assert_eq!(planes_at(&s, 10), [0x77; 4], "map mask 設回 0Fh");
}

#[test]
fn inverted_overwrite_replaces_each_plane_with_the_complement() {
    let items = [item(1, 1, &[0x11]), item(1, 1, &[0x22]), item(1, 1, &[0x33]), item(1, 1, &[0x44])];
    let mut s = sink(go(items.clone(), None));
    blit(&mut s, BlitCmd { x: 2, y: 3, ..cmd(Op::S18C0A) });
    blit(&mut s, BlitCmd { x: 2, y: 3, ..cmd(Op::S18C6E) });
    let o = 3 * 40 + 2;
    // 取代，不是 OR：平面 3、2、1、0 分別是 11、22、33、44 的補數。
    assert_eq!([s.plane(3)[o], s.plane(2)[o], s.plane(1)[o], s.plane(0)[o]], [0xEE, 0xDD, 0xCC, 0xBB]);
    // sub_190CC 同一個列迴圈，來源是 BEG1／J 組。
    let mut s = VgaSink::new(Assets { j3: go(items, None), ..Assets::empty() });
    blit(&mut s, BlitCmd { x: 1, beg_group: BegGroup::J3, ..cmd(Op::S190CC) });
    assert_eq!([s.plane(3)[1], s.plane(2)[1], s.plane(1)[1], s.plane(0)[1]], [0xEE, 0xDD, 0xCC, 0xBB]);
    assert_eq!(s.anomalies(), 0);
}

#[test]
fn beg_group_selects_the_library_for_overwrite_and_scratch() {
    let j1 = go(all_planes(1, 2, &[0x11, 0x11]), None);
    let j2 = go(all_planes(1, 2, &[0x22, 0x22]), None);
    let mut s = VgaSink::new(Assets { j1, j2, ..Assets::empty() });
    blit(&mut s, BlitCmd { x: 3, beg_group: BegGroup::J2, ..cmd(Op::S19068) });
    assert_eq!([s.plane(3)[3], s.plane(0)[4]], [0x22, 0x22]);
    blit(&mut s, BlitCmd { x: 3, y: 1, beg_group: BegGroup::J1, ..cmd(Op::S19068) });
    assert_eq!(s.plane(2)[40 + 3], 0x11);
    // sub_19B40 寫進暫存區，不裁切：x = 70 仍寫在暫存區列內。
    blit(&mut s, BlitCmd { x: 70, y: 2, scratch: 0x8000, beg_group: BegGroup::J2, ..cmd(Op::S19B40) });
    let o = 0x1F40 + 2 * 80 + 0x8000 + 70;
    assert_eq!([s.plane(1)[o], s.plane(1)[o + 1]], [0x22, 0x22]);
    // 缺的庫是異常，不 panic。
    blit(&mut s, BlitCmd { beg_group: BegGroup::J4, ..cmd(Op::S19068) });
    assert!(s.anomalies() > 0);
}

#[test]
fn sht_mask_comes_from_mask_group_and_planes_from_sht_group() {
    // SHT4 的遮罩不在 .GO 內，載入後放在 sht4.mask；遮罩段與平面段是兩組獨立的變數。
    let sht2 = go(all_planes(1, 1, &[0xFF]), Some(item(1, 1, &[0xF0])));
    let sht4 = go(all_planes(1, 1, &[0x0F]), Some(item(1, 1, &[0x3C])));
    let mut s = VgaSink::new(Assets { sht2, sht4, ..Assets::empty() });
    blit(&mut s, BlitCmd { sht_group: ShtGroup::Sht2, mask_group: ShtGroup::Sht4, ..cmd(Op::S18FC1) });
    blit(&mut s, BlitCmd { sht_group: ShtGroup::Sht2, mask_group: ShtGroup::Sht4, ..cmd(Op::S18F1A) });
    assert_eq!(planes_at(&s, 0), [0x3C; 4], "平面 FF 經 SHT4 遮罩 3C");
    // BH 沒有遮罩區段：mask_group 指到 BH 是異常，畫面不變。
    let before = s.anomalies();
    blit(&mut s, BlitCmd { mask_group: ShtGroup::Bh2, ..cmd(Op::S18F1A) });
    assert_eq!(s.anomalies(), before + 1);
    assert_eq!(planes_at(&s, 0), [0x3C; 4]);
}

#[test]
fn map_tile_writes_ten_rows_of_one_byte_ignoring_the_item_stride() {
    let rows: Vec<u8> = (1..=10).collect();
    let mut g = go([item(10, 1, &rows), item(10, 1, &[0; 10]), item(10, 1, &[0; 10]), item(10, 1, &[0; 10])], None);
    // 第二個圖塊：每列 2 位元組、10 列，原版仍連續讀前 10 個位元組。
    let wide: Vec<u8> = (1..=20).collect();
    g.planes[0].push(item(10, 2, &wide));
    for p in 1..4 {
        g.planes[p].push(item(10, 2, &[0; 20]));
    }
    // 第三個圖塊只有 3 位元組：後面的列沒有資料，記異常並寫 0。
    g.planes[0].push(item(3, 1, &[9, 9, 9]));
    for p in 1..4 {
        g.planes[p].push(item(3, 1, &[0; 3]));
    }
    let mut s = VgaSink::new(Assets { map: g, ..Assets::empty() });
    blit(&mut s, BlitCmd { x: 3, y: 2, scratch: 0x8000, index: 0, ..cmd(Op::S18ACE) });
    let base = 0x1F40 + 2 * 80 + 0x8000 + 3;
    for r in 0..10usize {
        assert_eq!(planes_at(&s, base + 0x50 * r), [0, 0, 0, r as u8 + 1], "列 {r}");
    }
    assert_eq!(s.plane(3)[base + 0x50 * 10], 0, "只有 10 列");
    assert_eq!(s.plane(3)[base + 1], 0, "只有 1 位元組寬");
    blit(&mut s, BlitCmd { x: 10, y: 0, index: 1, ..cmd(Op::S18ACE) });
    let b2 = 0x1F40 + 10;
    assert_eq!([s.plane(3)[b2], s.plane(3)[b2 + 0x50], s.plane(3)[b2 + 0x50 * 9]], [1, 2, 10], "連續讀，不跳 stride");
    assert_eq!(s.anomalies(), 0);
    blit(&mut s, BlitCmd { x: 20, y: 0, index: 2, ..cmd(Op::S18ACE) });
    assert_eq!(s.plane(3)[0x1F40 + 20 + 0x50 * 2], 9);
    assert_eq!(s.plane(3)[0x1F40 + 20 + 0x50 * 3], 0);
    assert_eq!(s.anomalies(), 7 * 4, "四個平面的項都只有 3 位元組，各缺 7 列");
}

#[test]
fn mirrored_copy_walks_the_destination_upward_and_stops_below_row_zero() {
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { y: 3, cx: 2, ..cmd(Op::S19C4F) });
    for (p, base) in [(3usize, 0x30u8), (2, 0x20), (1, 0x10), (0, 0x00)] {
        assert_eq!(&s.plane(p)[3 * 40..3 * 40 + 5], &[base + 1, base + 2, base + 3, base + 4, 0], "平面 {p}：列 3 取來源列 0");
        assert_eq!(&s.plane(p)[2 * 40..2 * 40 + 5], &[base + 5, base + 6, base + 7, base + 8, 0], "平面 {p}：列 2 取來源列 1");
    }
    // 列號遞減，小於 0 就停：y = 1、cx = 4 只畫列 1、列 0。
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { y: 1, cx: 4, ..cmd(Op::S19C4F) });
    assert_eq!(s.plane(3)[40], 0x31);
    assert_eq!(s.plane(3)[0], 0x35);
    assert_eq!(s.plane(3)[0xFFFF - 39], 0, "列 -1 沒有被畫");
    // 沒有 189 的上限：y = 195 的列照畫（sub_19C4F 只檢查 bx < 0）。
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { y: 195, cx: 1, ..cmd(Op::S19C4F) });
    assert_eq!(s.plane(3)[195 * 40], 0x31);
    // 模式回到 0：之後的覆寫 blit 正常。
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { y: 3, cx: 1, ..cmd(Op::S19C4F) });
    s.apply(&DrawCmd::VramFill { off: 4000, words: 1, value: 0x0042 });
    assert_eq!(planes_at(&s, 4000), [0x42; 4]);
}

#[test]
fn word_or_touches_one_word_per_row_and_only_checks_the_start_byte() {
    let mut s = scratch_pattern();
    s.apply(&DrawCmd::VramFill { off: 0, words: 40, value: 0x8080 });
    blit(&mut s, BlitCmd { y: 0, cx: 2, ..cmd(Op::S19E66) });
    for (p, base) in [(3usize, 0x30u8), (2, 0x20), (1, 0x10), (0, 0x00)] {
        assert_eq!(&s.plane(p)[0..4], &[0x80 | (base + 1), 0x80 | (base + 2), 0x80, 0x80], "平面 {p} 列 0");
        assert_eq!(&s.plane(p)[40..43], &[0x80 | (base + 5), 0x80 | (base + 6), 0x80], "平面 {p} 列 1");
    }
    // x = 39：字組起點在列內最後一欄，高位元組寫進下一列的欄 0。
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { x: 39, y: 0, cx: 1, ..cmd(Op::S19E66) });
    assert_eq!([s.plane(3)[39], s.plane(3)[40]], [0x31, 0x32]);
    // x = 40：起點超過列起點 + 27h，整個字組不寫。
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { x: 40, y: 0, cx: 1, ..cmd(Op::S19E66) });
    assert!((0..4).all(|p| s.plane(p)[40..42].iter().all(|&b| b == 0) && s.plane(p)[0] == 0));
}

#[test]
fn mirrored_or_and_mirrored_and_use_ascending_row_count_with_descending_destination() {
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { y: 3, cx: 2, ..cmd(Op::S19F3B) });
    for (p, base) in [(3usize, 0x30u8), (2, 0x20), (1, 0x10), (0, 0x00)] {
        assert_eq!(&s.plane(p)[3 * 40..3 * 40 + 5], &[base + 1, base + 2, base + 3, base + 4, 0], "平面 {p}：列 3 OR 來源列 0");
        assert_eq!(&s.plane(p)[2 * 40..2 * 40 + 5], &[base + 5, base + 6, base + 7, base + 8, 0], "平面 {p}：列 2 OR 來源列 1");
    }
    // 列號遞增、目的遞減：y = 188 時 bx 只到 189，所以只畫 2 列，不是 cx = 5 列。
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { y: 188, cx: 5, ..cmd(Op::S19F3B) });
    assert_eq!(s.plane(3)[188 * 40], 0x31);
    assert_eq!(s.plane(3)[187 * 40], 0x35);
    assert_eq!(s.plane(3)[186 * 40], 0, "第三列的 bx = 190 超過 189");
    // S1A1E6：AND 版。先把三列填滿 FF，來源讀取平面是最後一次選的平面 0（見 and_rows）。
    let mut s = scratch_pattern();
    s.apply(&DrawCmd::VramFill { off: 0, words: 200, value: 0xFFFF });
    blit(&mut s, BlitCmd { y: 3, cx: 2, ..cmd(Op::S1A1E6) });
    for p in 0..4 {
        assert_eq!(&s.plane(p)[3 * 40..3 * 40 + 5], &[1, 2, 3, 4, 0], "平面 {p}：列 3 AND 來源列 0 的平面 0");
        assert_eq!(&s.plane(p)[2 * 40..2 * 40 + 5], &[5, 6, 7, 8, 0], "平面 {p}：列 2 AND 來源列 1 的平面 0");
        assert_eq!(s.plane(p)[4 * 40], 0xFF, "列 4 沒動");
    }
}

#[test]
fn latch_copy_of_two_bytes_checks_every_byte_against_the_row_end() {
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { y: 4, cx: 2, ..cmd(Op::S19FF9) });
    for (p, base) in [(3usize, 0x30u8), (2, 0x20), (1, 0x10), (0, 0x00)] {
        assert_eq!(&s.plane(p)[4 * 40..4 * 40 + 4], &[base + 1, base + 2, 0, 0], "平面 {p} 列 4");
        assert_eq!(&s.plane(p)[5 * 40..5 * 40 + 4], &[base + 5, base + 6, 0, 0], "平面 {p} 列 5：來源每列前進 80");
    }
    // x = 39：第一個位元組在列內，第二個超過列尾而不寫，也不溢到下一列（和 sub_19E66 不同）。
    let mut s = scratch_pattern();
    blit(&mut s, BlitCmd { x: 39, y: 0, cx: 1, ..cmd(Op::S19FF9) });
    assert_eq!([s.plane(3)[39], s.plane(3)[40]], [0x31, 0]);
    // 寫入模式 1 只寫 map mask 內的平面，結束後寫入模式回 0，map mask 不變。
    let mut s = scratch_pattern();
    s.apply(&DrawCmd::SetMapMask { mask: 8 });
    blit(&mut s, BlitCmd { y: 0, cx: 1, ..cmd(Op::S19FF9) });
    assert_eq!(planes_at(&s, 0), [0, 0, 0, 0x31]);
    s.apply(&DrawCmd::VramFill { off: 3000, words: 1, value: 0x0042 });
    assert_eq!(planes_at(&s, 3000), [0, 0, 0, 0x42], "map mask 還是 8");
}

#[test]
fn and_of_two_bytes_reads_the_stale_plane_and_follows_map_mask() {
    let mut s = scratch_pattern();
    s.apply(&DrawCmd::VramFill { off: 0, words: 40, value: 0xFFFF });
    blit(&mut s, BlitCmd { y: 0, cx: 2, ..cmd(Op::S1A144) });
    // 讀取平面是 scratch_pattern 最後一次選的平面 0，來源位元組 1、2（列 0）與 5、6（列 1）。
    for p in 0..4 {
        assert_eq!(&s.plane(p)[0..4], &[1, 2, 0xFF, 0xFF], "平面 {p} 列 0");
        assert_eq!(&s.plane(p)[40..44], &[5, 6, 0xFF, 0xFF], "平面 {p} 列 1");
    }
    // sub_1882E 選平面 1：讀取平面 = 1、寫入平面 = 1。只有平面 1 被 AND，來源取平面 1 的暫存區（0x11、0x12）。
    let mut s = scratch_pattern();
    s.apply(&DrawCmd::VramFill { off: 0, words: 40, value: 0xFFFF });
    s.apply(&DrawCmd::SetMapMask { mask: 2 });
    blit(&mut s, BlitCmd { y: 0, cx: 1, ..cmd(Op::S1A144) });
    assert_eq!(&s.plane(1)[0..3], &[0x11, 0x12, 0xFF]);
    for p in [0, 2, 3] {
        assert_eq!(&s.plane(p)[0..3], &[0xFF; 3], "平面 {p} 不在 map mask 內");
    }
    // sub_18878 收尾：map mask 回 0Fh。
    s.apply(&DrawCmd::VramFill { off: 3000, words: 1, value: 0x0042 });
    assert_eq!(planes_at(&s, 3000), [0x42; 4]);
}

#[test]
fn counter_uses_the_bios_font_and_is_always_drawn_once_it_is_set() {
    let mut font = vec![0u8; 2048];
    // 字元 '1'（31h）到 '5'（35h）的 8 列各設成可辨識的值。
    for (d, ch) in (b'1'..=b'5').enumerate() {
        for r in 0..8usize {
            font[ch as usize * 8 + r] = (0x10 * (d as u8 + 1)) | r as u8;
        }
    }
    let mut s = sink(empty_go());
    s.set_counter_font(&font);
    assert!(!s.show_counter);
    blit(&mut s, BlitCmd { cx: 12345, ..cmd(Op::S1890D) });
    for (i, want_hi) in [0x10u8, 0x20, 0x30, 0x40, 0x50].into_iter().enumerate() {
        for r in 0..8usize {
            let o = (191 + r) * 40 + 18 + i;
            assert_eq!(s.plane(3)[o], want_hi | r as u8, "位數 {i} 列 {r}");
            assert_eq!(s.plane(3)[o + 0x8000], want_hi | r as u8, "兩頁都畫");
        }
    }
    assert!((0..3).all(|p| s.plane(p).iter().all(|&b| b == 0)));
    // 十進位逐位：0 用字元 '0'（30h），這份字型是全 0。
    let mut s = sink(empty_go());
    s.set_counter_font(&font);
    blit(&mut s, BlitCmd { cx: 1, text_x: 2, text_y: 20, ..cmd(Op::S1890D) });
    assert_eq!(s.plane(3)[20 * 40 + 2 + 4], 0x10, "個位數 '1' 的第 0 列");
    assert_eq!(s.plane(3)[20 * 40 + 2], 0, "萬位數 '0'");
}

#[test]
fn snapshot_hashes_cover_all_four_planes_and_the_full_dac_plus_attribute_registers() {
    let mut s = sink(empty_go());
    let planes: Vec<Vec<u8>> = (0..4u8).map(|p| (0..0x10000u32).map(|i| (i as u8).wrapping_mul(p + 3) ^ p).collect()).collect();
    let mut dac = [0u8; 768];
    for (i, v) in dac.iter_mut().enumerate() {
        *v = (i * 7 % 64) as u8;
    }
    s.load_snapshot([&planes[0], &planes[1], &planes[2], &planes[3]], &dac);
    for p in 0..4 {
        assert_eq!(s.plane(p), &planes[p][..]);
    }
    let cat: Vec<u8> = planes.iter().flatten().copied().collect();
    assert_eq!(s.vram_sha256(), rs2_formats::sha256_hex(&cat));
    let mut d = dac.to_vec();
    d.extend_from_slice(&ATTR_REGS);
    assert_eq!(d.len(), 785);
    assert_eq!(s.dac_sha256(), rs2_formats::sha256_hex(&d));
    assert_eq!(ATTR_REGS, [0, 1, 2, 3, 4, 5, 6, 7, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0]);
    // 高位元會被遮成 6 位元。
    let mut dac2 = dac;
    dac2[0] |= 0xC0;
    s.load_snapshot([&planes[0], &planes[1], &planes[2], &planes[3]], &dac2);
    assert_eq!(s.dac_sha256(), rs2_formats::sha256_hex(&d));
    // SetDac 的 start 是 DAC 索引（含 30h 起的第二份），SetDacOne 只動一個。
    let before = s.dac_sha256();
    s.apply(&DrawCmd::SetDacOne { index: 200, rgb: [1, 2, 0x43] });
    assert_eq!(s.dac(200), [1, 2, 3]);
    assert_ne!(s.dac_sha256(), before);
    let mut rgb = [0u8; 48];
    rgb[0..3].copy_from_slice(&[9, 8, 7]);
    s.apply(&DrawCmd::SetDac { start: 0xF8, rgb });
    assert_eq!(s.dac(0xF8), [9, 8, 7]);
    assert_eq!(s.dac(0xFF), [0, 0, 0], "整表 16 色寫到 F8 起，環繞回 0 到 7");
    // 命令序號不被快照改變，只是位址空間換了；暫存器回到預設。
    s.apply(&DrawCmd::SetMapMask { mask: 1 });
    s.load_snapshot([&planes[0], &planes[1], &planes[2], &planes[3]], &dac);
    s.apply(&DrawCmd::VramFill { off: 5, words: 1, value: 0x0101 });
    assert_eq!(planes_at(&s, 5), [1; 4]);
}

/// 第三關捲動用的暫存區：平面 3 的暫存區 A 列 0 欄 0 起 31 32 33 34，列 1 欄 0 起 35 36 37 38。
fn scroll_scratch() -> VgaSink {
    scratch_pattern()
}

#[test]
fn scroll_pair_shifts_scratch_b_left_by_four_pixels_for_the_selected_plane() {
    let mut s = scroll_scratch();
    s.apply(&DrawCmd::SetMapMask { mask: 8 });
    blit(&mut s, cmd(Op::S17EBA));
    // 旋轉 4：每個位元組高低半對調。
    let b = 0x9F40usize;
    assert_eq!([s.plane(3)[b], s.plane(3)[b + 1], s.plane(3)[b + 4]], [0x13, 0x23, 0x00]);
    blit(&mut s, cmd(Op::S17EEA));
    // B[i] = (A[i] << 4) | (A[i + 1] >> 4)：B 相對 A 左移 4 像素，列與列之間接續。
    assert_eq!([s.plane(3)[b], s.plane(3)[b + 1], s.plane(3)[b + 2], s.plane(3)[b + 3], s.plane(3)[b + 4]], [0x13, 0x23, 0x33, 0x40, 0x00]);
    assert_eq!(s.plane(3)[b + 79], 0x03, "列尾取下一列欄 0 的高半位元組（35h >> 4）");
    assert_eq!(s.plane(3)[b + 80], 0x53, "列 1 欄 0：35h << 4 | 36h >> 4");
    // 只有選到的平面變動；A 區原樣。
    for p in 0..3 {
        assert!(s.plane(p)[b..b + 8].iter().all(|&v| v == 0), "平面 {p} 不在 map mask 內");
    }
    assert_eq!(s.plane(3)[0x1F40], 0x31);
    // 範圍是 3E80h 位元組（200 列）。
    assert_eq!(s.plane(3)[b + 0x3E80], 0);
    // 資料旋轉回到 0：緊接的普通寫入不旋轉。
    s.apply(&DrawCmd::VramFill { off: 0, words: 1, value: 0x0081 });
    assert_eq!(s.plane(3)[0], 0x81);
}

#[test]
fn rotation_stays_until_the_second_half_of_the_pair() {
    let mut s = sink(empty_go());
    s.apply(&DrawCmd::SetMapMask { mask: 8 });
    blit(&mut s, cmd(Op::S17EBA));
    s.apply(&DrawCmd::VramFill { off: 0, words: 1, value: 0x0081 });
    assert_eq!(s.plane(3)[0], 0x18, "GC 索引 3 還是 04h，寫入資料旋轉 4");
    blit(&mut s, cmd(Op::S17EEA));
    s.apply(&DrawCmd::VramFill { off: 1, words: 1, value: 0x0081 });
    assert_eq!(s.plane(3)[1], 0x81);
}

#[test]
fn scroll_column_pair_rotates_one_column_and_patches_the_seam() {
    // 平面 3 的暫存區 A：列 0 = 11 22 34 44，列 1 = 55 66 78 88。
    let p3 = item(2, 4, &[0x11, 0x22, 0x34, 0x44, 0x55, 0x66, 0x78, 0x88]);
    let z = || item(2, 4, &[0; 8]);
    let mut s = VgaSink::new(Assets { sht1: go([p3, z(), z(), z()], None), ..Assets::empty() });
    blit(&mut s, cmd(Op::S19ADC));
    s.apply(&DrawCmd::SetMapMask { mask: 8 });
    blit(&mut s, BlitCmd { x: 2, ..cmd(Op::S17F26) });
    let b = 0x9F40usize;
    assert_eq!([s.plane(3)[b + 2], s.plane(3)[b + 80 + 2], s.plane(3)[b + 160 + 2]], [0x43, 0x87, 0]);
    blit(&mut s, BlitCmd { x: 2, ..cmd(Op::S17F5D) });
    // 左邊一欄的低半位元組取自右邊一欄（已旋轉過）的低半位元組。
    assert_eq!([s.plane(3)[b + 1], s.plane(3)[b + 80 + 1]], [0x03, 0x07]);
    assert_eq!(s.plane(3)[b + 2], 0x43, "右邊一欄不變");
    assert_eq!(s.plane(3)[b + 3], 0, "再右邊不動");
    // 200 列。
    assert_eq!(s.plane(3)[b + 199 * 80 + 1], 0);
}

#[test]
fn scroll_view_copies_forty_columns_of_190_rows_from_the_chosen_scratch() {
    let mut s = scroll_scratch();
    blit(&mut s, BlitCmd { x: 1, page: 0, ..cmd(Op::S17FA0) });
    // 來源從暫存區 A 列 0 的欄 1 起，列距 80；目的連續，列距 40。
    assert_eq!([s.plane(3)[0], s.plane(3)[1], s.plane(3)[2], s.plane(3)[3]], [0x32, 0x33, 0x34, 0]);
    assert_eq!([s.plane(3)[40], s.plane(3)[41], s.plane(3)[42], s.plane(3)[43]], [0x36, 0x37, 0x38, 0]);
    assert_eq!([s.plane(0)[0], s.plane(0)[1]], [2, 3], "四個平面一起搬");
    // 190 列。
    s.apply(&DrawCmd::VramFill { off: 190 * 40, words: 1, value: 0x7777 });
    blit(&mut s, BlitCmd { x: 0, page: 0, ..cmd(Op::S17FA0) });
    assert_eq!(s.plane(3)[190 * 40], 0x77, "第 190 列不被寫");
    // sub_18031：來源是暫存區 B（word_2685C = 8000h），目的是頁 1。
    let mut s = scroll_scratch();
    blit(&mut s, BlitCmd { x: 0, y: 5, scratch: 0x8000, ..cmd(Op::S19ADC) });
    blit(&mut s, BlitCmd { x: 0, page: 0x8000, scratch: 0x8000, ..cmd(Op::S18031) });
    let d = 0x8000 + 5 * 40;
    assert_eq!(&s.plane(3)[d..d + 5], &[0x31, 0x32, 0x33, 0x34, 0], "暫存區 B 列 5 在頁 1 列 5");
    assert_eq!(&s.plane(3)[d + 40..d + 44], &[0x35, 0x36, 0x37, 0x38]);
    assert_eq!(s.plane(3)[5 * 40], 0, "頁 0 不動");
}

#[test]
fn scroll_left_moves_both_scratch_areas_by_forty_bytes() {
    let mut s = sink(empty_go());
    s.apply(&DrawCmd::VramFill { off: 0x1F40 + 40, words: 1, value: 0xBBAA });
    s.apply(&DrawCmd::VramFill { off: 0x1F40 + 80, words: 1, value: 0xDDCC });
    s.apply(&DrawCmd::VramFill { off: 0x9F40 + 40, words: 1, value: 0x2211 });
    blit(&mut s, cmd(Op::S180C6));
    // 欄 40 起的內容左移 28h 位元組到欄 0；下一列欄 0 起的內容移到本列欄 40（位元組串流連續）。
    assert_eq!(planes_at(&s, 0x1F40), [0xAA; 4]);
    assert_eq!(planes_at(&s, 0x1F40 + 1), [0xBB; 4]);
    assert_eq!(planes_at(&s, 0x1F40 + 40), [0xCC; 4]);
    assert_eq!(planes_at(&s, 0x1F40 + 41), [0xDD; 4]);
    assert_eq!(planes_at(&s, 0x9F40), [0x11; 4]);
    assert_eq!(planes_at(&s, 0x9F40 + 1), [0x22; 4]);
    // 寫入模式回 0。
    s.apply(&DrawCmd::VramFill { off: 7, words: 1, value: 0x0033 });
    assert_eq!(planes_at(&s, 7), [0x33; 4]);
}
