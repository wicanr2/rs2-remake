//! `LayerSink` 的單元測試：用自製的小圖庫，不含原版位元組。
//! - 差分模糊測試：隨機圖庫與隨機命令串，每幀把 `LayerSink`（補間關閉或 alpha 1.0）與 `VgaSink` 逐像素比對。
//! - 補間行為：座標落在前後兩幀的中間、世代不同不內插、背景依循環長度環繞、補間不污染下一幀。
//! - HD：自製 PNG 資料夾，驗證座標乘 4、缺圖退回原版放大並標記、尺寸不符視為缺圖。
//!
//! 與原版逐像素的對拍在 `rs2-trace` 的 `tests/layer.rs`。

use rs2_core::{BlitCmd, DrawCmd, ObjId, Op, Pool, ShtGroup};
use rs2_formats::{BankItem, GoFile};
use rs2_gfx::{Assets, HdAssets, LayerSink, RenderOpts, VgaSink, png};

// ---------------------------------------------------------------------------
// 工具
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u32) -> u32 {
        (self.next() >> 33) as u32 % n
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1) as u32) as i32
    }
    fn chance(&mut self, pct: u32) -> bool {
        self.below(100) < pct
    }
}

const ALL_OPS: [Op; 22] = [
    Op::S18C0A,
    Op::S18CE2,
    Op::S18D46,
    Op::S18D89,
    Op::S18DCC,
    Op::S18E30,
    Op::S18E73,
    Op::S18EB6,
    Op::S18F1A,
    Op::S18F5D,
    Op::S18FC1,
    Op::S19025,
    Op::S19ADC,
    Op::S19BA4,
    Op::S19CF9,
    Op::S19D08,
    Op::S19DA8,
    Op::S1A09F,
    Op::S1A287,
    Op::S1929F,
    Op::S1890D,
    Op::S1152D,
];

fn base_cmd(op: Op) -> BlitCmd {
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
        text_x: 0,
        text_y: 0,
        sht_group: ShtGroup::Sht1,
        obj: None,
    }
}

fn item(height: u16, stride: u16, data: Vec<u8>) -> BankItem {
    assert_eq!(data.len(), height as usize * stride as usize);
    BankItem { height, stride, data }
}

fn rand_item(r: &mut Rng, h_max: u32, s_max: u32) -> BankItem {
    let (h, s) = (1 + r.below(h_max) as u16, 1 + r.below(s_max) as u16);
    item(h, s, (0..h as usize * s as usize).map(|_| r.next() as u8).collect())
}

/// 隨機圖庫。四個平面的同一項大多同尺寸，偶爾不同，兩邊都要用同樣的方式處理。
fn rand_go(r: &mut Rng, n: usize, mask: bool, h_max: u32, s_max: u32) -> GoFile {
    let mut planes: [Vec<BankItem>; 4] = Default::default();
    for _ in 0..n {
        let first = rand_item(r, h_max, s_max);
        for (k, p) in planes.iter_mut().enumerate() {
            if k == 0 || r.chance(85) {
                let data = if k == 0 { first.data.clone() } else { (0..first.data.len()).map(|_| r.next() as u8).collect() };
                p.push(item(first.height, first.stride, data));
            } else {
                p.push(rand_item(r, h_max, s_max));
            }
        }
    }
    let mask = mask.then(|| (0..n).map(|_| rand_item(r, h_max, s_max)).collect());
    GoFile { mask, planes }
}

fn rand_assets(r: &mut Rng, n: usize) -> Assets {
    Assets { my: rand_go(r, n, true, 14, 6), sht1: rand_go(r, n, true, 14, 6), bh1: rand_go(r, n, false, 30, 40) }
}

fn rgb_of(vga: &VgaSink, page: u16) -> Vec<u8> {
    let mut buf = vec![0u8; 320 * 200 * 3];
    vga.render_page(page, &mut buf);
    buf
}

/// 回傳 RGBA 與 RGB 逐像素不同的像素數。
fn diff_px(rgba: &[u8], rgb: &[u8]) -> usize {
    assert_eq!(rgba.len() / 4, rgb.len() / 3);
    (0..rgb.len() / 3).filter(|&i| rgba[4 * i..4 * i + 3] != rgb[3 * i..3 * i + 3] || rgba[4 * i + 3] != 255).count()
}

const EXACT: RenderOpts = RenderOpts { interpolate: false, hd: false, show_counter: false };
const INTERP: RenderOpts = RenderOpts { interpolate: true, hd: false, show_counter: false };

// ---------------------------------------------------------------------------
// 差分模糊測試
// ---------------------------------------------------------------------------

struct Tracked {
    op: Op,
    obj: ObjId,
    x: i32,
    y: i32,
    index: u16,
    shift: i32,
    cx: u16,
}

fn rand_cmd(r: &mut Rng, nitems: u32, page: u16) -> BlitCmd {
    let mut c = base_cmd(ALL_OPS[r.below(ALL_OPS.len() as u32) as usize]);
    c.x = match r.below(10) {
        0 => r.range(-4, -1) as u16,
        1 => r.range(40, 46) as u16,
        _ => r.below(40) as u16,
    };
    c.y = r.range(-8, 205) as u16;
    c.index = r.below(nitems + 1) as u16;
    c.page = if r.chance(90) { page } else { page ^ 0x8000 };
    c.scratch = if r.chance(50) { 0 } else { 0x8000 };
    c.src_x = r.below(90) as u16;
    c.src_y = r.range(0, 300) as u16;
    c.shift = r.range(-50, 90) as u16;
    let cx_max = if r.chance(20) { 220 } else { 40 };
    c.cx = if r.chance(5) { r.range(-3, -1) as u16 } else { r.range(0, cx_max) as u16 };
    c.dx = r.below(16) as u16 | if r.chance(10) { 0x30 } else { 0 };
    c.text_x = if r.chance(50) { 0x12 } else { r.below(36) as u16 };
    c.text_y = if r.chance(50) { 0xBF } else { r.below(190) as u16 };
    c.sht_group = if r.chance(50) { ShtGroup::Bh1 } else { ShtGroup::Sht1 };
    c
}

fn set_dac(r: &mut Rng) -> Vec<DrawCmd> {
    let mut rgb = [0u8; 48];
    for v in rgb.iter_mut() {
        *v = r.below(64) as u8;
    }
    vec![DrawCmd::SetDac { start: 0, rgb }, DrawCmd::SetDac { start: 0x30, rgb }]
}

/// 跑 `frames` 幀隨機命令，每幀比對。`bad` 回傳不同的像素總數，`px` 回傳比對的像素數。
#[derive(Clone, Copy)]
enum Break {
    /// LayerSink 每 N 個命令少收一個。
    DropEvery(usize),
    /// LayerSink 收到的圖形命令（OR、覆寫）x 加 1。
    ShiftX,
}

fn fuzz_run(seed: u64, frames: usize, break_cmd: Option<Break>) -> (usize, usize, usize) {
    let mut r = Rng(seed);
    let n = 12usize;
    let assets = rand_assets(&mut r, n);
    let mut vga = VgaSink::new(assets.clone());
    let mut lay = LayerSink::new(assets, None);
    let mut tracked: Vec<Tracked> = [
        (Op::S18F5D, Pool::Entity),
        (Op::S18F1A, Pool::Entity),
        (Op::S18CE2, Pool::Player),
        (Op::S19BA4, Pool::Background),
        (Op::S19D08, Pool::Scenery),
        (Op::S19DA8, Pool::Background),
        (Op::S1A287, Pool::Background),
        (Op::S1A09F, Pool::Background),
    ]
    .iter()
    .enumerate()
    .map(|(i, &(op, pool))| Tracked {
        op,
        obj: ObjId { pool, slot: i as u8, generation: 0 },
        x: r.range(0, 30),
        y: r.range(0, 150),
        index: r.below(n as u32) as u16,
        shift: r.range(0, 39),
        cx: r.range(1, 30) as u16,
    })
    .collect();

    let (mut bad, mut px, mut fed) = (0usize, 0usize, 0usize);
    // 初始化命令：先把暫存區填滿隨機圖。
    let mut init = set_dac(&mut r);
    for _ in 0..30 {
        let mut c = rand_cmd(&mut r, n as u32, 0);
        c.op = if r.chance(70) { Op::S19ADC } else { Op::S1152D };
        c.y = r.below(250) as u16;
        c.x = r.below(80) as u16;
        init.push(DrawCmd::Blit(c));
    }
    let feed = |vga: &mut VgaSink, lay: &mut LayerSink, cmds: Vec<DrawCmd>, fed: &mut usize| {
        let mut to_layer = Vec::with_capacity(cmds.len());
        for c in &cmds {
            vga.apply(c);
            *fed += 1;
            match (break_cmd, c) {
                (Some(Break::DropEvery(k)), _) if *fed % k == 0 => {}
                (Some(Break::ShiftX), DrawCmd::Blit(b)) if matches!(b.op, Op::S18F5D | Op::S18CE2 | Op::S18FC1 | Op::S18C0A) => {
                    let mut b2 = *b;
                    b2.x = b2.x.wrapping_add(1);
                    to_layer.push(DrawCmd::Blit(b2));
                }
                _ => to_layer.push(*c),
            }
        }
        lay.push_frame(&to_layer);
    };
    feed(&mut vga, &mut lay, init, &mut fed);

    for f in 0..frames {
        let page = if f % 2 == 0 { 0x8000u16 } else { 0 };
        let mut cmds: Vec<DrawCmd> = Vec::new();
        if r.chance(10) {
            cmds.extend(set_dac(&mut r));
        }
        let mut clear = base_cmd(Op::S19CF9);
        clear.page = page;
        cmds.push(DrawCmd::Blit(clear));
        // 追蹤中的物件：每幀小幅移動，偶爾換世代或位移太大。
        for t in tracked.iter_mut() {
            if r.chance(10) {
                continue;
            }
            t.x = (t.x + r.range(-2, 2)).clamp(-2, 40);
            t.y = (t.y + r.range(-6, 6)).clamp(-4, 195);
            t.shift = (t.shift + 2) % 40;
            if r.chance(5) {
                t.obj.generation += 1;
            }
            if r.chance(8) {
                t.index = r.below(n as u32) as u16;
            }
            if t.op == Op::S1A287 {
                t.cx = (t.cx as i32 + r.range(-2, 3)).clamp(1, 60) as u16;
            }
            let mut c = base_cmd(t.op);
            c.x = t.x as u16;
            c.y = t.y as u16;
            c.index = t.index;
            c.page = page;
            c.shift = t.shift as u16;
            c.cx = if matches!(t.op, Op::S1A287) { t.cx } else { r.range(1, 40) as u16 };
            c.dx = r.below(16) as u16;
            c.obj = Some(t.obj);
            c.sht_group = if r.chance(50) { ShtGroup::Bh1 } else { ShtGroup::Sht1 };
            cmds.push(DrawCmd::Blit(c));
        }
        for _ in 0..r.below(25) {
            let mut c = rand_cmd(&mut r, n as u32, page);
            if r.chance(50) {
                c.obj = Some(ObjId { pool: Pool::Effect, slot: r.below(3) as u8, generation: 0 });
            }
            cmds.push(DrawCmd::Blit(c));
        }
        // 同順序打散一下，讓追蹤物件夾在隨機命令之間。
        for i in (1..cmds.len()).rev() {
            let j = 1 + r.below(i as u32) as usize;
            if i > 0 && r.chance(30) {
                cmds.swap(i, j.min(i));
            }
        }
        // 清頁命令維持在最前面，頁才一致。
        let at = cmds.iter().position(|d| matches!(d, DrawCmd::Blit(b) if b.op == Op::S19CF9 && b.page == page)).unwrap();
        cmds.swap(0, at);
        feed(&mut vga, &mut lay, cmds, &mut fed);
        let page = lay.draw_page();
        let want = rgb_of(&vga, page);
        // 補間關閉、補間開啟的 alpha 1.0 都要與 VgaSink 相同；中間的 alpha 只求不 panic，且不影響之後的幀。
        for (a, o) in [(1.0f32, EXACT), (0.0, INTERP), (0.25, INTERP), (0.5, INTERP), (0.75, INTERP), (1.0, INTERP), (1.0, EXACT)] {
            let (img, w, h) = lay.render(a, o);
            assert_eq!((w, h, img.len()), (320, 200, 320 * 200 * 4));
            if a == 1.0 {
                bad += diff_px(img, &want);
                px += 320 * 200;
            }
        }
    }
    if break_cmd.is_none() {
        assert_eq!(lay.anomalies(), vga.anomalies(), "異常計數應與 VgaSink 相同（seed {seed}）");
    }
    (bad, px, fed)
}

#[test]
fn fuzz_matches_vgasink_pixel_for_pixel() {
    let (mut total_px, mut total_cmds) = (0, 0);
    for seed in 1..=24u64 {
        let (bad, px, fed) = fuzz_run(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0x1234_5678_9ABC_DEF1), 40, None);
        assert_eq!(bad, 0, "seed {seed}：與 VgaSink 不同的像素數");
        total_px += px;
        total_cmds += fed;
    }
    println!("差分模糊測試：命令 {total_cmds}，比對像素 {total_px}");
    assert!(total_px > 0 && total_cmds > 1000);
}

/// 負對照：LayerSink 每 7 個命令少收一個，或圖形命令的 x 加 1，差分測試每組種子都必須看得出來。
#[test]
fn fuzz_negative_control_detects_dropped_or_shifted_commands() {
    for seed in 1..=12u64 {
        let (bad, _, _) = fuzz_run(0xABCD_0000 + seed, 30, Some(Break::DropEvery(7)));
        assert!(bad > 0, "seed {seed}：少收命令卻沒有差異");
        let (bad, _, _) = fuzz_run(0xABCD_0000 + seed, 30, Some(Break::ShiftX));
        assert!(bad > 0, "seed {seed}：x 加 1 卻沒有差異");
    }
}

// ---------------------------------------------------------------------------
// 補間行為
// ---------------------------------------------------------------------------

fn white_palette() -> Vec<DrawCmd> {
    // 色號 8 = 白，其餘 = 黑。色號 8 在屬性表對應 DAC 56，由起點 30h 的第 8 項寫入。
    let mut hi = [0u8; 48];
    hi[24..27].copy_from_slice(&[63, 63, 63]);
    vec![DrawCmd::SetDac { start: 0, rgb: [0; 48] }, DrawCmd::SetDac { start: 0x30, rgb: hi }]
}

fn solid_go(h: u16, stride: u16) -> GoFile {
    // 只填 plane 3（planes[0]），所以色號是 8。
    let full = vec![0xFF; h as usize * stride as usize];
    let zero = vec![0; h as usize * stride as usize];
    GoFile {
        mask: None,
        planes: [vec![item(h, stride, full)], vec![item(h, stride, zero.clone())], vec![item(h, stride, zero.clone())], vec![item(h, stride, zero)]],
    }
}

fn empty_go() -> GoFile {
    GoFile { mask: None, planes: Default::default() }
}

fn sprite_cmd(x: u16, y: u16, page: u16, obj: Option<ObjId>) -> DrawCmd {
    let mut c = base_cmd(Op::S18F5D);
    c.x = x;
    c.y = y;
    c.page = page;
    c.obj = obj;
    DrawCmd::Blit(c)
}

fn clear_cmd(page: u16) -> DrawCmd {
    let mut c = base_cmd(Op::S19CF9);
    c.page = page;
    DrawCmd::Blit(c)
}

/// 白色像素的外框 `(x0, y0, x1, y1)`（含），沒有白像素回傳 `None`。
fn white_bbox(img: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let mut bb: Option<(usize, usize, usize, usize)> = None;
    for y in 0..h {
        for x in 0..w {
            if img[(y * w + x) * 4] == 255 {
                bb = Some(match bb {
                    None => (x, y, x, y),
                    Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x), d.max(y)),
                });
            }
        }
    }
    bb
}

fn sprite_sink() -> LayerSink {
    let mut s = LayerSink::new(Assets { my: empty_go(), sht1: solid_go(4, 1), bh1: empty_go() }, None);
    s.push_frame(&white_palette());
    s
}

fn oid(gen_: u32) -> Option<ObjId> {
    Some(ObjId { pool: Pool::Entity, slot: 3, generation: gen_ })
}

#[test]
fn midpoint_lies_between_previous_and_current_frame() {
    let mut s = sprite_sink();
    s.push_frame(&[clear_cmd(0x8000), sprite_cmd(1, 10, 0x8000, oid(1))]);
    s.push_frame(&[clear_cmd(0), sprite_cmd(3, 14, 0, oid(1))]);
    // 前一幀 (8,10)、目前幀 (24,14)。
    for (a, want) in [(0.0, (8, 10)), (0.25, (12, 11)), (0.5, (16, 12)), (0.75, (20, 13)), (1.0, (24, 14))] {
        let (img, w, h) = s.render(a, INTERP);
        let bb = white_bbox(img, w, h).unwrap();
        assert_eq!((bb.0, bb.1), want, "alpha {a}");
        assert_eq!((bb.2 - bb.0 + 1, bb.3 - bb.1 + 1), (8, 4), "大小不變");
    }
    // 補間關閉永遠是目前幀的位置。
    let (img, w, h) = s.render(0.5, EXACT);
    assert_eq!(white_bbox(img, w, h).map(|b| (b.0, b.1)), Some((24, 14)));
}

#[test]
fn different_generation_or_no_previous_draws_at_current_position() {
    for (prev, cur, why) in [(oid(1), oid(2), "世代不同"), (None, oid(1), "前一幀沒有 obj"), (oid(1), None, "目前幀沒有 obj")] {
        let mut s = sprite_sink();
        s.push_frame(&[clear_cmd(0x8000), sprite_cmd(1, 10, 0x8000, prev)]);
        s.push_frame(&[clear_cmd(0), sprite_cmd(3, 14, 0, cur)]);
        assert_eq!(s.interpolated_cmds(), 0, "{why}");
        let (img, w, h) = s.render(0.5, INTERP);
        assert_eq!(white_bbox(img, w, h).map(|b| (b.0, b.1)), Some((24, 14)), "{why}");
    }
    // 位移超過門檻（7 欄 = 56 像素）也不內插。
    let mut s = sprite_sink();
    s.push_frame(&[clear_cmd(0x8000), sprite_cmd(1, 10, 0x8000, oid(1))]);
    s.push_frame(&[clear_cmd(0), sprite_cmd(8, 10, 0, oid(1))]);
    let (img, w, h) = s.render(0.5, INTERP);
    assert_eq!(white_bbox(img, w, h).map(|b| b.0), Some(64));
    // 剛好 6 欄（48 像素）在門檻上，要內插。
    let mut s = sprite_sink();
    s.push_frame(&[clear_cmd(0x8000), sprite_cmd(1, 10, 0x8000, oid(1))]);
    s.push_frame(&[clear_cmd(0), sprite_cmd(7, 10, 0, oid(1))]);
    let (img, w, h) = s.render(0.5, INTERP);
    assert_eq!(white_bbox(img, w, h).map(|b| b.0), Some(8 + 24));
}

#[test]
fn rendering_does_not_change_the_next_frame() {
    // 同樣三幀，一邊每幀都在中間 alpha 渲染，一邊完全不渲染；第三幀的結果必須相同。
    let frames = |i: u16| -> Vec<DrawCmd> {
        let page = if i % 2 == 0 { 0x8000 } else { 0 };
        vec![clear_cmd(page), sprite_cmd(1 + i, 10 + 2 * i, page, oid(1))]
    };
    let mut a = sprite_sink();
    let mut b = sprite_sink();
    for i in 0..4 {
        a.push_frame(&frames(i));
        b.push_frame(&frames(i));
        for al in [0.0, 0.3, 0.5, 0.9] {
            let _ = a.render(al, INTERP);
        }
    }
    let ia = a.render(1.0, EXACT).0.to_vec();
    let ib = b.render(1.0, EXACT).0.to_vec();
    assert_eq!(ia, ib);
    // 同一幀重複渲染不累積。
    let x1 = a.render(0.5, INTERP).0.to_vec();
    let x2 = a.render(0.5, INTERP).0.to_vec();
    assert_eq!(x1, x2);
}

/// 背景層：暫存區一列 80 位元組、同一張圖畫在 x = 0 與 x = 40，每幀位移 2 位元組（16 像素），38 → 0 環繞。
#[test]
fn background_shift_interpolates_across_the_wrap() {
    let pattern: Vec<u8> = (0..40u32).map(|i| (i * 37 + 11) as u8).collect();
    let zero = vec![0u8; 40];
    let bh1 = GoFile {
        mask: None,
        planes: [vec![item(1, 40, pattern.clone())], vec![item(1, 40, zero.clone())], vec![item(1, 40, zero.clone())], vec![item(1, 40, zero)]],
    };
    let mut s = LayerSink::new(Assets { my: empty_go(), sht1: empty_go(), bh1 }, None);
    let mut init = white_palette();
    for x in [0u16, 40] {
        let mut c = base_cmd(Op::S19ADC);
        c.x = x;
        c.sht_group = ShtGroup::Bh1;
        init.push(DrawCmd::Blit(c));
    }
    s.push_frame(&init);
    let layer = |shift: u16, page: u16| {
        let mut c = base_cmd(Op::S19BA4);
        c.cx = 1;
        c.shift = shift;
        c.page = page;
        c.obj = Some(ObjId { pool: Pool::Background, slot: 1, generation: 0 });
        DrawCmd::Blit(c)
    };
    s.push_frame(&[clear_cmd(0x8000), layer(38, 0x8000)]);
    s.push_frame(&[clear_cmd(0), layer(0, 0)]);
    let bit = |col: usize| (pattern[(col % 320) / 8] >> (7 - col % 8)) & 1;
    let mut seen = Vec::new();
    for (a, start) in [(0.0, 304usize), (0.5, 312), (1.0, 0)] {
        let (img, _, _) = s.render(a, INTERP);
        let row: Vec<u8> = (0..320).map(|k| (img[k * 4] == 255) as u8).collect();
        let want: Vec<u8> = (0..320).map(|k| bit(start + k)).collect();
        assert_eq!(row, want, "alpha {a}：第 0 列應從來源第 {start} 欄開始");
        seen.push(row);
    }
    assert!(seen[0] != seen[1] && seen[1] != seen[2], "中間值不能等於任何一端");
}

#[test]
fn frame_counter_is_an_overlay_and_never_enters_memory() {
    let mut s = sprite_sink();
    let mut c = base_cmd(Op::S1890D);
    c.cx = 12345;
    c.text_x = 0x12;
    c.text_y = 0xBF;
    let mut other = c;
    other.text_x = 2;
    other.text_y = 20;
    other.cx = 7;
    s.push_frame(&[clear_cmd(0), DrawCmd::Blit(c), DrawCmd::Blit(other)]);
    let off = s.render(1.0, EXACT).0.to_vec();
    let on = s.render(1.0, RenderOpts { show_counter: true, ..EXACT }).0.to_vec();
    let off2 = s.render(1.0, EXACT).0.to_vec();
    assert_eq!(off, off2, "關掉後恢復，沒有殘留");
    assert_ne!(off, on);
    let diff: Vec<usize> = (0..320 * 200).filter(|&i| off[4 * i..4 * i + 4] != on[4 * i..4 * i + 4]).collect();
    // 其他位置的數字（擊中計數）永遠畫，所以差異只在幀計數矩形：列 191～198、欄 144～183。
    assert!(!diff.is_empty());
    for i in diff {
        let (x, y) = (i % 320, i / 320);
        assert!((191..=198).contains(&y) && (144..184).contains(&x), "矩形外的差異 ({x},{y})");
    }
    // 另一個位置的數字有畫（色號 8 = 白，是我們的調色盤）。
    assert!(white_bbox(&off, 320, 200).is_some());
}

// ---------------------------------------------------------------------------
// HD
// ---------------------------------------------------------------------------

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rs2-layer-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn solid_png(w: usize, h: usize, rgba: [u8; 4]) -> Vec<u8> {
    png::encode_rgba(w, h, &rgba.repeat(w * h))
}

#[test]
fn hd_draws_at_four_times_and_interpolates() {
    let dir = temp_dir("hd4");
    std::fs::create_dir_all(dir.join("SHT1")).unwrap();
    // 項目 0 是 8x4 邏輯像素，HD 32x16。
    std::fs::write(dir.join("SHT1/0000.png"), solid_png(32, 16, [255, 0, 0, 255])).unwrap();
    let hd = HdAssets::open(&dir).unwrap();
    let mut s = LayerSink::new(Assets { my: empty_go(), sht1: solid_go(4, 1), bh1: empty_go() }, Some(hd));
    s.push_frame(&white_palette());
    s.push_frame(&[clear_cmd(0x8000), sprite_cmd(1, 10, 0x8000, oid(1))]);
    s.push_frame(&[clear_cmd(0), sprite_cmd(3, 14, 0, oid(1))]);
    let hdo = RenderOpts { hd: true, ..INTERP };
    let red_bbox = |img: &[u8], w: usize, h: usize| {
        let mut bb: Option<(usize, usize, usize, usize)> = None;
        for y in 0..h {
            for x in 0..w {
                let p = &img[(y * w + x) * 4..(y * w + x) * 4 + 4];
                if p == [255, 0, 0, 255] {
                    bb = Some(match bb {
                        None => (x, y, x, y),
                        Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x), d.max(y)),
                    });
                }
            }
        }
        bb
    };
    for (a, want) in [(0.0, (32, 40)), (0.5, (64, 48)), (1.0, (96, 56))] {
        let (img, w, h) = s.render(a, hdo);
        assert_eq!((w, h, img.len()), (1280, 800, 1280 * 800 * 4));
        let bb = red_bbox(img, w, h).unwrap();
        assert_eq!((bb.0, bb.1), want, "alpha {a}");
        assert_eq!((bb.2 - bb.0 + 1, bb.3 - bb.1 + 1), (32, 16));
        assert!(img.chunks_exact(4).all(|p| p[3] == 255), "HD 輸出不透明");
    }
    assert!(s.missing_hd().is_empty(), "{:?}", s.missing_hd());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hd_missing_and_wrong_size_fall_back_to_scaled_original_with_marker() {
    let dir = temp_dir("hdmiss");
    std::fs::create_dir_all(dir.join("SHT1")).unwrap();
    // 項目 0 缺檔；項目 1 尺寸錯誤；項目 2 檔案損毀。
    std::fs::write(dir.join("SHT1/0001.png"), solid_png(31, 16, [0, 255, 0, 255])).unwrap();
    std::fs::write(dir.join("SHT1/0002.png"), b"not a png").unwrap();
    let hd = HdAssets::open(&dir).unwrap();
    let go = {
        let mut g = solid_go(4, 1);
        for p in g.planes.iter_mut() {
            let it = p[0].clone();
            p.push(it.clone());
            p.push(it);
        }
        g
    };
    let mut s = LayerSink::new(Assets { my: empty_go(), sht1: go, bh1: empty_go() }, Some(hd));
    s.push_frame(&white_palette());
    let mut cmds = vec![clear_cmd(0)];
    for (i, x) in [(0u16, 2u16), (1, 8), (2, 14)] {
        let mut c = base_cmd(Op::S18F5D);
        c.index = i;
        c.x = x;
        c.y = 10;
        cmds.push(DrawCmd::Blit(c));
    }
    s.push_frame(&cmds);
    let (img, w, _) = s.render(1.0, RenderOpts { hd: true, ..EXACT });
    let px = |x: usize, y: usize| [img[(y * w + x) * 4], img[(y * w + x) * 4 + 1], img[(y * w + x) * 4 + 2]];
    for x0 in [2usize * 32, 8 * 32, 14 * 32] {
        assert_eq!(px(x0, 40), [255, 0, 255], "外框是洋紅");
        assert_eq!(px(x0 + 8, 48), [255, 255, 255], "內部是原版色號 8（白）的放大");
    }
    let m = s.missing_hd();
    assert_eq!(m.len(), 3, "{m:?}");
    assert!(m[0].starts_with("SHT1/0000.png"));
    assert!(m[1].contains("0001.png") && m[1].contains("尺寸"));
    assert!(m[2].contains("0002.png") && m[2].contains("解碼失敗"));
    // 沒有 HD 資料夾時，hd: true 輸出原版圖的 4 倍最近鄰。
    let mut plain = LayerSink::new(Assets { my: empty_go(), sht1: solid_go(4, 1), bh1: empty_go() }, None);
    plain.push_frame(&white_palette());
    plain.push_frame(&[clear_cmd(0), sprite_cmd(2, 10, 0, None)]);
    let small = plain.render(1.0, EXACT).0.to_vec();
    let (big, bw, bh) = plain.render(1.0, RenderOpts { hd: true, ..EXACT });
    assert_eq!((bw, bh), (1280, 800));
    for (x, y) in [(16, 10), (23, 13), (24, 13), (0, 0)] {
        assert_eq!(&big[((y * 4 + 2) * 1280 + x * 4 + 3) * 4..][..4], &small[(y * 320 + x) * 4..][..4]);
    }
    assert!(plain.missing_hd().is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// HD 在隨機命令下不 panic（含補間、缺圖、尺寸不符）。
#[test]
fn hd_fuzz_does_not_panic() {
    let dir = temp_dir("hdfuzz");
    let mut r = Rng(0xFEED_FACE_1234);
    let n = 6usize;
    let assets = rand_assets(&mut r, n);
    for (lib, go) in [("MY", &assets.my), ("SHT1", &assets.sht1), ("BH1", &assets.bh1)] {
        std::fs::create_dir_all(dir.join(lib).join("masks")).unwrap();
        for i in 0..n {
            let it = &go.planes[0][i];
            let (w, h) = (it.stride as usize * 32, it.height as usize * 4);
            // 一半的圖有 HD 檔，且 alpha 各種值。
            if i % 2 == 0 {
                let mut rgba = Vec::with_capacity(w * h * 4);
                for _ in 0..w * h {
                    rgba.extend_from_slice(&[r.next() as u8, r.next() as u8, r.next() as u8, [0, 128, 255][r.below(3) as usize]]);
                }
                std::fs::write(dir.join(lib).join(format!("{i:04}.png")), png::encode_rgba(w, h, &rgba)).unwrap();
            }
            if let Some(m) = &go.mask {
                if i % 3 != 0 {
                    let mi = &m[i];
                    let (w, h) = (mi.stride as usize * 32, mi.height as usize * 4);
                    let mut rgba = Vec::with_capacity(w * h * 4);
                    for _ in 0..w * h {
                        let v = [0, 100, 255][r.below(3) as usize];
                        rgba.extend_from_slice(&[v, v, v, 255]);
                    }
                    std::fs::write(dir.join(lib).join("masks").join(format!("{i:04}.png")), png::encode_rgba(w, h, &rgba)).unwrap();
                }
            }
        }
    }
    let hd = HdAssets::open(&dir).unwrap();
    let mut lay = LayerSink::new(assets, Some(hd));
    let mut init = set_dac(&mut r);
    for _ in 0..20 {
        let mut c = rand_cmd(&mut r, n as u32, 0);
        c.op = if r.chance(70) { Op::S19ADC } else { Op::S1152D };
        c.y = r.below(250) as u16;
        c.x = r.below(80) as u16;
        init.push(DrawCmd::Blit(c));
    }
    lay.push_frame(&init);
    let mut obj_pos = (5i32, 20i32);
    for f in 0..16usize {
        let page = if f % 2 == 0 { 0x8000u16 } else { 0 };
        let mut cmds = vec![clear_cmd(page)];
        obj_pos = (obj_pos.0 + 1, obj_pos.1 + 3);
        for op in [Op::S18F5D, Op::S18F1A, Op::S19BA4, Op::S1A09F, Op::S19D08] {
            let mut c = rand_cmd(&mut r, n as u32, page);
            c.op = op;
            c.x = obj_pos.0 as u16;
            c.y = obj_pos.1 as u16;
            c.page = page;
            c.cx = 20;
            c.shift = (f as u16 * 2) % 40;
            c.obj = Some(ObjId { pool: Pool::Background, slot: op as u8, generation: 0 });
            cmds.push(DrawCmd::Blit(c));
        }
        for _ in 0..12 {
            cmds.push(DrawCmd::Blit(rand_cmd(&mut r, n as u32, page)));
        }
        lay.push_frame(&cmds);
        for a in [0.0, 0.5, 1.0] {
            let (img, w, h) = lay.render(a, RenderOpts { hd: true, ..INTERP });
            assert_eq!((w, h, img.len()), (1280, 800, 1280 * 800 * 4));
        }
        let (_, w, h) = lay.render(0.5, RenderOpts { hd: false, ..INTERP });
        assert_eq!((w, h), (320, 200));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// 受擊閃白：前一幀是遮罩加平面，這一幀只有 `S19025`（SHT 遮罩的反相 OR，填成色號 15）。
/// 閃白幀仍要內插位置；閃白後的下一幀遮罩與平面共用同一組位移，相對位置不變。
#[test]
fn flash_frame_keeps_position_interpolation() {
    let mut go = solid_go(4, 1);
    go.mask = Some(vec![item(4, 1, vec![0x00; 4])]); // 全是剪影
    let mut s = LayerSink::new(Assets { my: empty_go(), sht1: go, bh1: empty_go() }, None);
    // 色號 8 與 15 都是白。
    let mut hi = [0u8; 48];
    hi[24..27].copy_from_slice(&[63, 63, 63]);
    hi[45..48].copy_from_slice(&[63, 63, 63]);
    s.push_frame(&[DrawCmd::SetDac { start: 0, rgb: [0; 48] }, DrawCmd::SetDac { start: 0x30, rgb: hi }]);
    let at = |op: Op, x: u16, y: u16, page: u16| {
        let mut c = base_cmd(op);
        c.x = x;
        c.y = y;
        c.page = page;
        c.obj = oid(1);
        DrawCmd::Blit(c)
    };
    s.push_frame(&[clear_cmd(0x8000), at(Op::S18F1A, 1, 10, 0x8000), at(Op::S18F5D, 1, 11, 0x8000)]);
    s.push_frame(&[clear_cmd(0), at(Op::S19025, 3, 14, 0)]);
    let (img, w, h) = s.render(0.5, INTERP);
    let bb = white_bbox(img, w, h).unwrap();
    let want = if rs2_gfx::INTERP_CROSS_OP { (16, 12) } else { (24, 14) };
    assert_eq!((bb.0, bb.1), want, "閃白幀的位置");
    // 閃白後的下一幀：遮罩與平面一起內插，白色外框的位置在前後兩幀中間。
    s.push_frame(&[clear_cmd(0x8000), at(Op::S18F1A, 5, 18, 0x8000), at(Op::S18F5D, 5, 19, 0x8000)]);
    let (img, w, h) = s.render(0.5, INTERP);
    let bb = white_bbox(img, w, h).unwrap();
    // 遮罩在 y = 18 AND 掉空頁沒有可見效果，可見的是平面：前一幀閃白 (24,14)，這一幀平面 (40,19)，平面位置 = 遮罩位置加 1 列。
    let want = if rs2_gfx::INTERP_CROSS_OP { (32, 16 + 1) } else { (40, 19) };
    assert_eq!((bb.0, bb.1), want, "閃白之後那一幀的平面位置");
}
