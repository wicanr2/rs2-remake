//! `LayerSink` 驗收。需要原版檔與收據，由
//! `tools/rust.sh test -p rs2-trace --test layer -- --ignored --nocapture` 跑。比對筆數為 0 視為失敗。
//!
//! | 項目 | 內容 |
//! |---|---|
//! | L1 等價 | `hd: false`、補間關閉，每步與 `VgaSink::render_page(顯示頁)` 逐像素相同 |
//! | L2 補間不污染 | 每步在 alpha 0、0.25、0.5、0.75 渲染後，alpha 1.0 仍與 L1 相同 |
//! | L3 補間有效 | 真實命令：alpha 0.5 的像素位置在前後兩幀的中間 |
//! | L4 負對照 | x 加 1、丟命令，L1 的比對要失敗 |
//! | L5 HD 冒煙 | 輸出 PNG 到 `$RS2_OUT/shots/` 給人判讀，列出缺圖 |
//! | L6 效能 | `render` 與 `push_frame` 的耗時 |
//! | L8 全流程等價 | 全部 `wt_*` 收據，原版圖、不補間，與 `VgaSink::render_page(畫頁)` 逐像素相同；L8b 是補間版（alpha 0.5 渲染後 alpha 1.0 仍相同） |
//! | L9 全流程 HD | 全部 `wt_*` 收據用 HD 圖跑，`missing_hd()` 為空 |

use rs2_core::{BlitCmd, DrawCmd, Dseg, Fixes, Game, Input, ObjId, Op, Pool, Start};
use rs2_gfx::pal::{PALETTES, fingerprint};
use rs2_gfx::{Assets, HdAssets, LayerSink, RenderOpts, VgaSink, png};
use rs2_trace::{load_inputs, parse_sc};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 跑")))
}

const EXACT: RenderOpts = RenderOpts { interpolate: false, hd: false, show_counter: false, player: [None, None] };
const INTERP: RenderOpts = RenderOpts { interpolate: true, hd: false, show_counter: false, player: [None, None] };

/// 一段重播的全部命令：`Game::new` 後的初始化命令，加上每一步的命令。
struct Scen {
    name: &'static str,
    init: Vec<DrawCmd>,
    steps: Vec<Vec<DrawCmd>>,
}

fn collect(name: &'static str, start: Start, receipt: &str, n: usize) -> Scen {
    let dir = env_path("RS2_ORIG");
    let run = env_path("RS2_DOSRUN");
    let inputs = load_inputs(&dir).unwrap();
    let mut g = Game::new(&inputs.image, &inputs.dm1, start, Fixes::NONE);
    let init = g.take_draws();
    let text = std::fs::read_to_string(run.join(receipt)).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= n, "{receipt} 只有 {} 筆", lines.len());
    let mut steps = Vec::with_capacity(n);
    for line in lines.iter().take(n) {
        let sc = parse_sc(line);
        let f = g.step(&Input { scancodes: &sc }, &mut |_g: &Game, _pt: u32| {});
        steps.push(f.draws);
    }
    Scen { name, init, steps }
}

fn dm1() -> &'static Scen {
    static S: OnceLock<Scen> = OnceLock::new();
    S.get_or_init(|| collect("DM1", Start::Demo, "trace_dm1/trace-dm1.jsonl", 640))
}

fn boss() -> &'static Scen {
    static S: OnceLock<Scen> = OnceLock::new();
    S.get_or_init(|| collect("boss1_kill", Start::NewGame { difficulty: None }, "boss1_kill/trace-s1.jsonl", 1192))
}

fn assets() -> Assets {
    Assets::load(&env_path("RS2_ORIG")).unwrap()
}

// ---------------------------------------------------------------------------
// 調色盤指紋
// ---------------------------------------------------------------------------

/// 調色盤指紋表與映像一致：每個名稱的 DAC 表位址取自映像，重算指紋要等於表內常數。
#[test]
#[ignore]
fn palette_fingerprints_match_image() {
    let inputs = load_inputs(&env_path("RS2_ORIG")).unwrap();
    let ds = Dseg::from_image(&inputs.image);
    let mut checked = 0;
    for (name, addr, want) in PALETTES {
        let mut rgb = [0u8; 48];
        for (i, v) in rgb.iter_mut().enumerate() {
            *v = ds.r8(addr + i as u16);
        }
        assert_eq!(fingerprint(&rgb), want, "{name} 在 dseg:{addr:04X} 的指紋");
        checked += 1;
    }
    println!("調色盤指紋核對 {checked} 張");
    assert_eq!(checked, 18);
}

// ---------------------------------------------------------------------------
// L1、L2、L4：與 VgaSink 逐像素比對
// ---------------------------------------------------------------------------

#[derive(Default, Debug)]
struct Rep {
    frames: usize,
    px: usize,
    bad_px: usize,
    first_bad: Option<usize>,
    renders: usize,
}

type Tweak<'a> = &'a dyn Fn(usize, &[DrawCmd]) -> Vec<DrawCmd>;

/// 每步把命令餵給兩邊，比對 `LayerSink` 與 `VgaSink::render_page(顯示頁)`。
/// `tweak` 只改 `LayerSink` 收到的命令（負對照用）。`intermediate` 為真時，每步先在中間 alpha 渲染。
fn compare(sc: &Scen, intermediate: bool, tweak: Option<Tweak>) -> Rep {
    let a = assets();
    let mut vga = VgaSink::new(a.clone());
    let mut lay = LayerSink::new(a, None);
    for c in &sc.init {
        vga.apply(c);
    }
    lay.push_frame(&sc.init);
    let mut rep = Rep::default();
    let mut want = vec![0u8; 320 * 200 * 3];
    for (n, draws) in sc.steps.iter().enumerate() {
        for c in draws {
            vga.apply(c);
        }
        match tweak {
            Some(t) => lay.push_frame(&t(n, draws)),
            None => lay.push_frame(draws),
        }
        // 第一步畫頁 8000h，之後每個有命令的步交替（規格 001「一步的邊界」）。
        if !draws.is_empty() && tweak.is_none() {
            let page = if n % 2 == 0 { 0x8000 } else { 0 };
            assert_eq!(lay.draw_page(), page, "{} 第 {n} 步的畫頁", sc.name);
        }
        if intermediate {
            for al in [0.0, 0.25, 0.5, 0.75] {
                let (img, w, h) = lay.render(al, INTERP);
                assert_eq!((w, h, img.len()), (320, 200, 320 * 200 * 4));
                rep.renders += 1;
            }
        }
        vga.render_page(lay.draw_page(), &mut want);
        let opts = if intermediate { INTERP } else { EXACT };
        let (img, _, _) = lay.render(1.0, opts);
        let bad = (0..320 * 200).filter(|&i| img[4 * i..4 * i + 3] != want[3 * i..3 * i + 3] || img[4 * i + 3] != 255).count();
        rep.frames += 1;
        rep.px += 320 * 200;
        rep.renders += 1;
        if bad > 0 {
            rep.bad_px += bad;
            rep.first_bad.get_or_insert(n);
        }
    }
    assert_eq!(lay.anomalies(), vga.anomalies(), "{} 異常計數", sc.name);
    rep
}

fn show(sc: &Scen, what: &str, r: &Rep) {
    println!("{what} {}：比對 {} 幀、{} 像素、不同 {} 像素，首個不同幀 {:?}，render 呼叫 {} 次", sc.name, r.frames, r.px, r.bad_px, r.first_bad, r.renders);
}

#[test]
#[ignore]
fn l1_equivalence_with_vgasink() {
    let mut total = 0;
    for sc in [dm1(), boss()] {
        let r = compare(sc, false, None);
        show(sc, "L1", &r);
        assert!(r.frames > 0 && r.px > 0, "比對筆數為 0");
        assert_eq!(r.bad_px, 0, "{} 與 VgaSink 不同", sc.name);
        total += r.px;
    }
    assert_eq!(total, (640 + 1192) * 320 * 200);
    // 畫頁交叉核對：收據的 display_start 是取樣當下顯示的頁，LayerSink 的畫頁是另一頁。
    let run = env_path("RS2_DOSRUN").join("a0_demo");
    let sc = dm1();
    let a = assets();
    let mut lay = LayerSink::new(a, None);
    lay.push_frame(&sc.init);
    let mut checked = 0;
    for (n, d) in sc.steps.iter().enumerate() {
        lay.push_frame(d);
        if [0usize, 1, 99, 299, 620, 622, 639].contains(&n) {
            let meta = std::fs::read_to_string(run.join(format!("demo_n{n}.json"))).unwrap();
            let k = meta.find("\"display_start\"").unwrap();
            let v: u16 = meta[k..].split(':').nth(1).unwrap().split([',', '\n']).next().unwrap().trim().parse().unwrap();
            // 取樣點在翻頁之前，收據的 display_start 是上一幀畫好、正在顯示的頁；這一步畫的頁是翻頁後要顯示的頁。
            assert_eq!(lay.draw_page(), v ^ 0x8000, "demo_n{n} 的 display_start 與畫頁應互為另一頁");
            checked += 1;
        }
    }
    println!("L1 畫頁與收據 display_start 交叉核對 {checked} 筆");
    assert_eq!(checked, 7);
}

#[test]
#[ignore]
fn l2_interpolation_does_not_change_the_next_frame() {
    for sc in [dm1(), boss()] {
        let r = compare(sc, true, None);
        show(sc, "L2", &r);
        assert!(r.frames > 0 && r.px > 0 && r.renders >= r.frames * 5, "比對筆數為 0");
        assert_eq!(r.bad_px, 0, "{} 中間 alpha 渲染後 alpha 1.0 與 VgaSink 不同", sc.name);
    }
}

#[test]
#[ignore]
fn l4_negative_controls_fail_the_comparison() {
    // 找一個畫面上看得到的圖形命令：第 100 步之後第一個 OR 命令。
    for sc in [dm1(), boss()] {
        let (n0, k) = (100usize..sc.steps.len())
            .find_map(|n| {
                sc.steps[n]
                    .iter()
                    .position(|d| matches!(d, DrawCmd::Blit(b) if b.op == Op::S18F5D && b.obj.is_some()))
                    .map(|k| (n, k))
            })
            .expect("找不到可用的命令");
        // x 加 1（位元組，8 像素）。
        let shift = move |n: usize, d: &[DrawCmd]| -> Vec<DrawCmd> {
            let mut v = d.to_vec();
            if n == n0 {
                if let DrawCmd::Blit(b) = &mut v[k] {
                    b.x = b.x.wrapping_add(1);
                }
            }
            v
        };
        let r = compare(sc, false, Some(&shift));
        show(sc, "L4 x+1", &r);
        assert!(r.bad_px > 0, "{} x 加 1 卻沒有差異", sc.name);
        assert!(r.first_bad.unwrap() >= n0, "差異不應早於被改的那一步");
        // 丟掉一個命令。
        let drop = move |n: usize, d: &[DrawCmd]| -> Vec<DrawCmd> {
            let mut v = d.to_vec();
            if n == n0 {
                v.remove(k);
            }
            v
        };
        let r = compare(sc, false, Some(&drop));
        show(sc, "L4 丟命令", &r);
        assert!(r.bad_px > 0, "{} 丟命令卻沒有差異", sc.name);
        // 每 7 個命令丟一個（同 A5 的負對照）。
        let cnt = std::cell::Cell::new(0usize);
        let drop7 = |_n: usize, d: &[DrawCmd]| -> Vec<DrawCmd> {
            d.iter()
                .filter(|_| {
                    cnt.set(cnt.get() + 1);
                    cnt.get() % 7 != 0
                })
                .copied()
                .collect()
        };
        let r = compare(sc, false, Some(&drop7));
        show(sc, "L4 每 7 個丟 1 個", &r);
        assert!(r.bad_px > 1000, "{} 丟了命令卻幾乎沒有差異", sc.name);
    }
}

// ---------------------------------------------------------------------------
// L3：真實命令的補間位置
// ---------------------------------------------------------------------------

struct Cand {
    pool: Pool,
    frame: usize,
    prev: BlitCmd,
    cur: BlitCmd,
}

/// 原版圖庫項的寬（像素）與高。OR 平面命令用哪個圖庫由 `op` 與 `sht_group` 決定。
fn dims(a: &Assets, c: &BlitCmd) -> Option<(i32, i32)> {
    let go = match c.op {
        Op::S18CE2 | Op::S18EB6 | Op::S18DCC => &a.my,
        _ => a.sht(c.sht_group),
    };
    let it = go.planes[0].get(c.index as usize)?;
    Some((it.stride as i32 * 8, it.height as i32))
}

fn is_or_plane(op: Op) -> bool {
    matches!(op, Op::S18F5D | Op::S18CE2 | Op::S18EB6 | Op::S18DCC)
}

/// 連續 3 幀都以同一個 `ObjId`、同一個圖移動，且完整在遊戲區內的 OR 平面命令。
fn candidates(a: &Assets, sc: &Scen) -> Vec<Cand> {
    let find = |d: &[DrawCmd], o: ObjId, op: Op| -> Option<BlitCmd> {
        let mut it = d.iter().filter_map(|x| match x {
            DrawCmd::Blit(b) if b.obj == Some(o) && b.op == op => Some(*b),
            _ => None,
        });
        let first = it.next()?;
        it.next().is_none().then_some(first)
    };
    let mut out = Vec::new();
    for n in 2..sc.steps.len() {
        for d in &sc.steps[n] {
            let DrawCmd::Blit(c) = d else { continue };
            let Some(o) = c.obj else { continue };
            if !is_or_plane(c.op) {
                continue;
            }
            let (Some(c), Some(p), Some(pp)) = (find(&sc.steps[n], o, c.op), find(&sc.steps[n - 1], o, c.op), find(&sc.steps[n - 2], o, c.op)) else { continue };
            if p.index != c.index || pp.index != c.index {
                continue;
            }
            let Some((w, h)) = dims(a, &c) else { continue };
            let inside = |b: &BlitCmd| {
                let (x, y) = (b.x as i16 as i32 * 8, b.y as i16 as i32);
                x >= 0 && x + w <= 320 && y >= 0 && y + h <= 189
            };
            let mv = |u: &BlitCmd, v: &BlitCmd| ((v.x as i16 as i32 - u.x as i16 as i32) * 8, v.y as i16 as i32 - u.y as i16 as i32);
            let (d1, d2) = (mv(&p, &c), mv(&pp, &p));
            let in_range = |d: (i32, i32)| d.0.abs().max(d.1.abs()) <= 48 && d.0.abs() + d.1.abs() >= 2;
            if inside(&c) && inside(&p) && inside(&pp) && in_range(d1) && in_range(d2) {
                out.push(Cand { pool: o.pool, frame: n, prev: p, cur: c });
            }
        }
    }
    out
}

/// 色號 1 以上全白、0 黑，所以非黑像素就是圖的可見像素。
fn all_white_palette() -> Vec<DrawCmd> {
    let mut rgb = [63u8; 48];
    rgb[..3].copy_from_slice(&[0, 0, 0]);
    vec![DrawCmd::SetDac { start: 0, rgb }, DrawCmd::SetDac { start: 0x30, rgb }]
}

fn nonblack_bbox(img: &[u8]) -> Option<(i32, i32, i32, i32)> {
    let mut bb: Option<(i32, i32, i32, i32)> = None;
    for y in 0..200i32 {
        for x in 0..320i32 {
            if img[((y * 320 + x) * 4) as usize] != 0 {
                bb = Some(match bb {
                    None => (x, y, x, y),
                    Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x), d.max(y)),
                });
            }
        }
    }
    bb
}

fn clear(page: u16) -> DrawCmd {
    let mut c = prev_template(Op::S19CF9);
    c.page = page;
    DrawCmd::Blit(c)
}

fn prev_template(op: Op) -> BlitCmd {
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
        sht_group: rs2_core::ShtGroup::Sht1,
        mask_group: rs2_core::ShtGroup::Sht1,
        beg_group: rs2_core::BegGroup::Beg1,
        stride_reg: None,
        obj: None,
    }
}

/// 把實際的前後兩幀命令單獨放進乾淨的 LayerSink，量 alpha 0、0.5、1 的外框。
/// 回傳 `(外框@0, 外框@0.5, 外框@1)` 的左上角。
fn measure(a: &Assets, cand: &Cand) -> [(i32, i32); 3] {
    let mut s = LayerSink::new(a.clone(), None);
    s.push_frame(&all_white_palette());
    let (mut p, mut c) = (cand.prev, cand.cur);
    p.page = 0x8000;
    c.page = 0;
    s.push_frame(&[clear(0x8000), DrawCmd::Blit(p)]);
    s.push_frame(&[clear(0), DrawCmd::Blit(c)]);
    assert_eq!(s.interpolated_cmds(), 1, "目前幀的命令應配對到前一幀");
    let mut out = [(0, 0); 3];
    for (i, al) in [0.0, 0.5, 1.0].into_iter().enumerate() {
        let (img, _, _) = s.render(al, INTERP);
        let bb = nonblack_bbox(img).expect("圖應該有可見像素");
        out[i] = (bb.0, bb.1);
    }
    out
}

/// 與 `rs2_gfx` 的偏移公式相同：四捨五入遠離 0。
fn round_away(v: f32) -> i32 {
    v.round() as i32
}

#[test]
#[ignore]
fn l3_midpoint_with_real_commands() {
    let a = assets();
    let mut picked: Vec<(&'static str, Cand)> = Vec::new();
    let mut counts = std::collections::BTreeMap::new();
    for sc in [dm1(), boss()] {
        for c in candidates(&a, sc) {
            *counts.entry(format!("{:?}", c.pool)).or_insert(0usize) += 1;
            let kind = match c.pool {
                Pool::Entity => "實體",
                Pool::PlayerBullet | Pool::EnemyBullet => "子彈",
                _ => continue,
            };
            // 每種挑位移最大的一筆，中點才有別於兩端。
            let mag = |c: &Cand| (c.cur.x as i16 as i32 - c.prev.x as i16 as i32).abs() * 8 + (c.cur.y as i16 as i32 - c.prev.y as i16 as i32).abs();
            match picked.iter_mut().find(|(k, _)| *k == kind) {
                Some(slot) => {
                    if mag(&c) > mag(&slot.1) {
                        slot.1 = c;
                    }
                }
                None => picked.push((kind, c)),
            }
        }
    }
    println!("符合條件的候選（連續 3 幀同一物件同一圖移動）：{counts:?}");
    assert!(picked.iter().any(|(k, _)| *k == "實體"), "沒有找到實體候選");
    assert!(picked.iter().any(|(k, _)| *k == "子彈"), "沒有找到子彈候選");
    let mut checked = 0;
    for (kind, cand) in &picked {
        let [b0, bh, b1] = measure(&a, cand);
        // 用命令算期望：前後兩幀的位置（位元組轉像素）與位移。
        let (px, py) = (cand.prev.x as i16 as i32 * 8, cand.prev.y as i16 as i32);
        let (cx, cy) = (cand.cur.x as i16 as i32 * 8, cand.cur.y as i16 as i32);
        let (dx, dy) = (cx - px, cy - py);
        // 外框左上角相對命令位置的內縮量，前後兩端應相同（同一張圖）。
        let inset0 = (b0.0 - px, b0.1 - py);
        let inset1 = (b1.0 - cx, b1.1 - cy);
        assert_eq!(inset0, inset1, "{kind}：alpha 0 與 1 的內縮量應相同");
        // alpha 0.5 的期望：目前位置 + round(-位移 * 0.5)。
        let want = (cx + round_away(-(dx as f32) * 0.5) + inset1.0, cy + round_away(-(dy as f32) * 0.5) + inset1.1);
        assert_eq!(bh, want, "{kind}：alpha 0.5 的位置");
        // 落在兩端之間，位移 >= 2 的軸要嚴格在內。
        for (axis, lo, mid, hi, d) in [("x", b0.0, bh.0, b1.0, dx), ("y", b0.1, bh.1, b1.1, dy)] {
            assert!(mid >= lo.min(hi) && mid <= lo.max(hi), "{kind} {axis}：中點 {mid} 不在 [{lo}, {hi}]");
            assert!((2 * mid - (lo + hi)).abs() <= 1, "{kind} {axis}：中點 {mid} 不是 {lo} 與 {hi} 的中間");
            if d.abs() >= 2 {
                assert!(mid != lo && mid != hi, "{kind} {axis}：中點等於其中一端（位移 {d}）");
            }
        }
        println!(
            "L3 {kind}（{:?} 槽 {}，第 {} 步，圖 {}）：前 ({px},{py}) 後 ({cx},{cy})；外框左上角 alpha0 {b0:?} alpha0.5 {bh:?} alpha1 {b1:?}",
            cand.cur.obj.unwrap().pool,
            cand.cur.obj.unwrap().slot,
            cand.frame,
            cand.cur.index
        );
        checked += 1;
    }
    assert!(checked >= 2);

    // 完整管線：在候選那一步，alpha 0.5 的整幅畫面不等於 0 或 1。
    let sc = if candidates(&a, dm1()).iter().any(|c| c.pool == picked[0].1.pool && c.frame == picked[0].1.frame) { dm1() } else { boss() };
    let mut lay = LayerSink::new(a.clone(), None);
    lay.push_frame(&sc.init);
    let target = picked[0].1.frame;
    for d in sc.steps.iter().take(target + 1) {
        lay.push_frame(d);
    }
    let img0 = lay.render(0.0, INTERP).0.to_vec();
    let img5 = lay.render(0.5, INTERP).0.to_vec();
    let img1 = lay.render(1.0, INTERP).0.to_vec();
    let d05 = (0..320 * 200).filter(|&i| img0[4 * i..4 * i + 3] != img5[4 * i..4 * i + 3]).count();
    let d15 = (0..320 * 200).filter(|&i| img1[4 * i..4 * i + 3] != img5[4 * i..4 * i + 3]).count();
    println!("L3 完整管線 {} 第 {target} 步：此幀有 {} 個命令帶位移；alpha 0.5 與 0 相差 {d05} 像素、與 1 相差 {d15} 像素", sc.name, lay.interpolated_cmds());
    assert!(d05 > 0 && d15 > 0);
}

// ---------------------------------------------------------------------------
// L5：HD 冒煙
// ---------------------------------------------------------------------------

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, png::encode_rgba(w, h, rgba)).unwrap();
}

fn nearest4(src: &[u8]) -> Vec<u8> {
    let mut o = vec![0u8; 1280 * 800 * 4];
    for y in 0..800usize {
        for x in 0..1280usize {
            o[(y * 1280 + x) * 4..][..4].copy_from_slice(&src[((y / 4) * 320 + x / 4) * 4..][..4]);
        }
    }
    o
}

#[test]
#[ignore]
fn l5_hd_smoke() {
    let hd_dir = env_path("RS2_HD");
    let shots = env_path("RS2_OUT").join("shots");
    let a = assets();
    let mut written = Vec::new();
    for (sc, targets) in [(dm1(), vec![1usize, 300, 620, 639]), (boss(), vec![700usize])] {
        let hd = HdAssets::open(&hd_dir).unwrap();
        let mut lay = LayerSink::new(a.clone(), Some(hd));
        let t0 = Instant::now();
        lay.push_frame(&sc.init);
        for (n, d) in sc.steps.iter().enumerate() {
            lay.push_frame(d);
            if sc.name == "DM1" && n == 619 {
                assert_eq!(lay.palette_name(), Some("stage1"));
                assert!(!lay.loaded_hd().iter().any(|p| p.contains("stage1_boss")), "調色盤切換前就載入了 stage1_boss 變體");
            }
            if !targets.contains(&n) {
                continue;
            }
            let tag = format!("layer_{}_n{n:04}", sc.name.to_lowercase());
            let pal = lay.palette_name();
            let (hd_img, w, h) = lay.render(1.0, RenderOpts { interpolate: true, hd: true, show_counter: false, player: [None, None] });
            assert_eq!((w, h), (1280, 800));
            let hd_img = hd_img.to_vec();
            assert!(hd_img.chunks_exact(4).any(|p| p[..3] != [0, 0, 0]), "HD 全黑");
            let (orig, ow, oh) = lay.render(1.0, EXACT);
            assert_eq!((ow, oh), (320, 200));
            let orig = orig.to_vec();
            write_png(&shots.join(format!("{tag}_hd.png")), 1280, 800, &hd_img);
            write_png(&shots.join(format!("{tag}_orig.png")), 320, 200, &orig);
            write_png(&shots.join(format!("{tag}_orig4x.png")), 1280, 800, &nearest4(&orig));
            written.push(format!("{tag}_{{hd,orig,orig4x}}.png（調色盤 {pal:?}）"));
            if n == 300 || n == 700 {
                let (mid, _, _) = lay.render(0.5, RenderOpts { interpolate: true, hd: true, show_counter: false, player: [None, None] });
                write_png(&shots.join(format!("{tag}_hd_alpha05.png")), 1280, 800, mid);
                written.push(format!("{tag}_hd_alpha05.png"));
            }
        }
        println!("L5 {}：全部 {} 步（含載入 HD 圖）{:.1} 秒；palette {:?}", sc.name, sc.steps.len(), t0.elapsed().as_secs_f32(), lay.palette_name());
        println!("L5 {} missing_hd = {:?}", sc.name, lay.missing_hd());
        assert!(lay.missing_hd().is_empty(), "{} 第一關不該有缺圖：{:?}", sc.name, lay.missing_hd());
        let var: Vec<&String> = lay.loaded_hd().iter().filter(|p| p.contains("variants/stage1_boss/")).collect();
        println!("L5 {} 載入的 HD 檔 {} 個，其中 stage1_boss 變體 {} 個：{:?}", sc.name, lay.loaded_hd().len(), var.len(), var);
        assert!(!var.is_empty(), "{} 進入頭目調色盤後沒有載入任何 stage1_boss 變體", sc.name);
    }
    for w in &written {
        println!("L5 輸出 {}/{w}", shots.display());
    }
    assert!(written.len() >= 5);
}

/// `stage1_boss` 生效後，原生調色盤不同的圖改用 `variants/stage1_boss/`。
/// 依據：`S11141` 的 `SetDac` 表指紋辨識為 `stage1_boss`（見 `palette_fingerprints_match_image`），
/// 且 HD 資料夾有該調色盤的變體檔。這裡驗證辨識結果與切換時機。
#[test]
#[ignore]
fn l5b_boss_palette_is_detected_when_s11141_runs() {
    let sc = dm1();
    let a = assets();
    let mut lay = LayerSink::new(a, None);
    lay.push_frame(&sc.init);
    let mut names = Vec::new();
    for (n, d) in sc.steps.iter().enumerate() {
        lay.push_frame(d);
        let nm = lay.palette_name();
        if names.last().map(|(_, x)| x) != Some(&nm) {
            names.push((n, nm));
        }
    }
    println!("L5b DM1 調色盤名稱變化（步號, 名稱）：{names:?}");
    let seq: Vec<_> = names.iter().map(|x| x.1).collect();
    assert!(seq.contains(&Some("stage1")) && seq.contains(&Some("stage1_boss")), "{seq:?}");
    assert!(seq.iter().position(|x| *x == Some("stage1")) < seq.iter().position(|x| *x == Some("stage1_boss")));
}

// ---------------------------------------------------------------------------
// L6：效能
// ---------------------------------------------------------------------------

fn stats(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    (mean, v[v.len() / 2], *v.last().unwrap())
}

#[test]
#[ignore]
fn l6_render_timing() {
    let a = assets();
    let sc = dm1();
    // 原版圖。
    let mut lay = LayerSink::new(a.clone(), None);
    lay.push_frame(&sc.init);
    let (mut push_o, mut ren_o) = (Vec::new(), Vec::new());
    for d in &sc.steps {
        let t = Instant::now();
        lay.push_frame(d);
        push_o.push(t.elapsed().as_secs_f64() * 1e3);
        let t = Instant::now();
        let _ = lay.render(0.5, INTERP).0.len();
        ren_o.push(t.elapsed().as_secs_f64() * 1e3);
    }
    // 判準用 p99。這台機器是共用的，這支測試程式內也有其他重測試並行，單次最大值量到的是別人的負載，不是繪製本身。
    let mut sorted = ren_o.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p99 = sorted[sorted.len() * 99 / 100];
    let (m, p50, mx) = stats(ren_o);
    let (pm, _, pmx) = stats(push_o);
    println!("L6 原版圖 320x200：render(0.5, 補間) 平均 {m:.3} ms 中位 {p50:.3} ms p99 {p99:.3} ms 最大 {mx:.3} ms；push_frame 平均 {pm:.3} ms 最大 {pmx:.3} ms（DM1 640 幀）");
    assert!(p99 < 16.0, "p99 {p99:.3} ms 超過一幀");

    // HD。載入圖片的時間算在 push_frame。
    let hd = HdAssets::open(&env_path("RS2_HD")).unwrap();
    let mut lay = LayerSink::new(a, Some(hd));
    lay.push_frame(&sc.init);
    let (mut push_h, mut ren_h, mut ren_h1, mut ren_boss) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let hdo = RenderOpts { interpolate: true, hd: true, show_counter: false, player: [None, None] };
    for (n, d) in sc.steps.iter().enumerate() {
        let t = Instant::now();
        lay.push_frame(d);
        push_h.push(t.elapsed().as_secs_f64() * 1e3);
        let t = Instant::now();
        let _ = lay.render(0.5, hdo).0.len();
        let e = t.elapsed().as_secs_f64() * 1e3;
        ren_h.push(e);
        if n >= 623 {
            ren_boss.push(e);
        }
        let t = Instant::now();
        let _ = lay.render(1.0, RenderOpts { interpolate: false, ..hdo }).0.len();
        ren_h1.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let (bm, bp50, bmx) = stats(ren_boss);
    println!("L6 HD 頭目在畫面上（DM1 第 623 步起，合成圖整幅複製）：render(0.5, 補間) 平均 {bm:.3} ms 中位 {bp50:.3} ms 最大 {bmx:.3} ms");
    let (m, p50, mx) = stats(ren_h);
    let (m1, p501, mx1) = stats(ren_h1);
    let (pm, pp50, pmx) = stats(push_h);
    println!("L6 HD 1280x800：render(0.5, 補間) 平均 {m:.3} ms 中位 {p50:.3} ms 最大 {mx:.3} ms；render(1.0, 不補間) 平均 {m1:.3} ms 中位 {p501:.3} ms 最大 {mx1:.3} ms");
    println!("L6 HD push_frame（含首次載入 PNG）平均 {pm:.3} ms 中位 {pp50:.3} ms 最大 {pmx:.3} ms");
    println!("L6 條件：release（overflow-checks 開）、容器 --cpus 4、DM1 640 幀逐幀呼叫、輸出只取長度不複製");
}

// ---------------------------------------------------------------------------
// 診斷：每種命令有沒有 obj，補間會碰到哪些
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn l0_frame_summary() {
    for (sc, ns) in [(dm1(), vec![300usize, 639]), (boss(), vec![700usize])] {
        let mut lay = LayerSink::new(assets(), None);
        lay.push_frame(&sc.init);
        for (n, d) in sc.steps.iter().enumerate() {
            lay.push_frame(d);
            if !ns.contains(&n) {
                continue;
            }
            let mut by: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
            for c in d {
                if let DrawCmd::Blit(b) = c {
                    let e = by.entry(format!("{:?}/{}", b.op, b.obj.map(|o| format!("{:?}", o.pool)).unwrap_or("-".into()))).or_default();
                    e.0 += 1;
                }
            }
            println!("L0 {} 第 {n} 步：{} 個命令，{} 個帶位移；各 (op/pool)：{:?}", sc.name, d.len(), lay.interpolated_cmds(), by.iter().map(|(k, v)| format!("{k}×{}", v.0)).collect::<Vec<_>>());
        }
    }
}

/// PNG 解碼速度：頭目合成圖與背景圖（第一次用到時在 `push_frame` 裡解碼）。
#[test]
#[ignore]
fn l6b_png_decode_speed() {
    let dir = env_path("RS2_HD");
    let files = ["SHT1/0033.png", "SHT1/0034.png", "SHT1/0035.png", "BH1/0000.png", "BH1/0001.png", "BH1/0002.png", "BH1/0003.png", "BH1/0004.png", "MY/0026.png"];
    let (mut bytes_in, mut bytes_out, mut ms) = (0usize, 0usize, 0.0f64);
    for f in files {
        let data = std::fs::read(dir.join(f)).unwrap();
        let t = Instant::now();
        let img = png::decode(&data).unwrap();
        let e = t.elapsed().as_secs_f64() * 1e3;
        println!("L6b {f}: {}x{}，PNG {} KB，解碼 {e:.2} ms", img.w, img.h, data.len() / 1024);
        bytes_in += data.len();
        bytes_out += img.rgba.len();
        ms += e;
    }
    println!("L6b 合計：PNG {:.1} MB，RGBA {:.1} MB，解碼 {ms:.1} ms（{:.0} MB/s 輸出）", bytes_in as f64 / 1e6, bytes_out as f64 / 1e6, bytes_out as f64 / 1e6 / (ms / 1e3));
}

/// 自寫的 PNG 解碼器對 HD 資料夾的實檔：每個檔都能解，尺寸等於原版圖庫項的 4 倍，遮罩是灰階，
/// 有透明處的圖確實解出 alpha。印出整個圖庫（不只第一關用到的）缺哪些檔。
#[test]
#[ignore]
fn l7_hd_files_decode_and_match_original_sizes() {
    let dir = env_path("RS2_HD");
    let a = assets();
    let (mut decoded, mut with_alpha, mut bad) = (0usize, 0usize, Vec::<String>::new());
    let mut missing = Vec::<String>::new();
    let mut check = |rel: String, want: (usize, usize), gray: bool, missing_ok: bool| {
        let p = dir.join(&rel);
        let Ok(data) = std::fs::read(&p) else {
            if !missing_ok {
                missing.push(rel);
            }
            return;
        };
        match png::decode(&data) {
            Ok(img) => {
                decoded += 1;
                if (img.w, img.h) != want {
                    bad.push(format!("{rel}: {}x{}，預期 {}x{}", img.w, img.h, want.0, want.1));
                }
                if gray && !img.rgba.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255) {
                    bad.push(format!("{rel}: 遮罩不是不透明灰階"));
                }
                if !gray && img.rgba.chunks_exact(4).any(|p| p[3] < 255) {
                    with_alpha += 1;
                }
            }
            Err(e) => bad.push(format!("{rel}: 解碼失敗 {e}")),
        }
    };
    for (lib, go) in [("MY", &a.my), ("SHT1", &a.sht1), ("BH1", &a.bh1)] {
        for (i, it) in go.planes[0].iter().enumerate() {
            let want = (it.stride as usize * 32, it.height as usize * 4);
            check(format!("{lib}/{i:04}.png"), want, false, false);
            // 調色盤變體：資料夾內有哪些就檢查哪些。
            if let Ok(rd) = std::fs::read_dir(dir.join(lib).join("variants")) {
                for pal in rd.flatten() {
                    check(format!("{lib}/variants/{}/{i:04}.png", pal.file_name().to_string_lossy()), want, false, true);
                }
            }
        }
        if let Some(m) = &go.mask {
            for (i, it) in m.iter().enumerate() {
                check(format!("{lib}/masks/{i:04}.png"), (it.stride as usize * 32, it.height as usize * 4), true, false);
            }
        }
    }
    println!("L7 解碼成功 {decoded} 個檔，其中 {with_alpha} 個圖有透明像素；尺寸或格式不符 {} 個；整個圖庫缺檔 {} 個", bad.len(), missing.len());
    println!("L7 缺檔：{missing:?}");
    println!("L7 不符：{bad:?}");
    assert!(decoded > 300 && with_alpha > 100, "比對筆數過少");
    assert!(bad.is_empty(), "{bad:?}");
}

// ---------------------------------------------------------------------------
// L8、L9：全流程（規格 002 B6）
// ---------------------------------------------------------------------------
//
// 餵法與 `wait_replay` 相同：`rs2_trace::wait::replay_observed` 就是 `replay_with` 的同一份程式碼加一個觀察者，
// 輸入（掃描碼、poke、輪詢排程）、快照入口與逐筆比對都沒有另寫一份。收據必須每筆通過（`Verdict::Pass`），
// 觀察者看到的命令串才算是「通過 `wait_replay` 的那一批」。
//
// 起點：兩個接收端都用收據的入口平面與 DAC（`load_snapshot`）、BIOS 字型（或不設，見各測試）與 `word_2687E` 殘值。
// 兩個接收端都不設 `set_bank_dump`：片尾的越界圖號兩邊都不畫（各記一次異常，數量要相同）。
// 環境變數 `RS2_WAIT=子字串` 只跑名稱含它的收據（同 `tools/dos/wait_test.sh`）。

use rs2_core::a as ds_addr;
use rs2_trace::wait::{Receipt, ReplayEvent, ReplayOpts, Verdict, load_receipt, replay_observed};
use rs2_trace::Inputs;
use std::collections::BTreeMap;

fn receipt_dirs() -> Vec<PathBuf> {
    let root = env_path("RS2_DOSRUN");
    let only = std::env::var("RS2_WAIT").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("讀不到 {}：{e}", root.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("wt_") && n.contains(&only)))
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "{} 下沒有 wt_*{only}* 收據", root.display());
    dirs
}

fn entry_planes(rc: &Receipt) -> ([&[u8]; 4], [u8; 768]) {
    let planes = [&rc.planes[0][..], &rc.planes[1][..], &rc.planes[2][..], &rc.planes[3][..]];
    let dac: [u8; 768] = rc.dac[..].try_into().expect("768 位元組");
    (planes, dac)
}

fn entry_stride(rc: &Receipt) -> u16 {
    let o = ds_addr(0x2687E) as usize;
    u16::from_le_bytes([rc.entry_ds[o], rc.entry_ds[o + 1]])
}

fn flow_inputs() -> (Inputs, Assets) {
    let dir = env_path("RS2_ORIG");
    (load_inputs(&dir).expect("載入原版"), Assets::load(&dir).expect("載入圖庫"))
}

/// 全位址空間：`LayerSink` 的索引色記憶體與 `VgaSink` 四個平面不同的像素數，與第一個不同的像素位址（位元組位址 * 8 + 位元）。
fn mem_diff(vga: &VgaSink, lay: &LayerSink) -> (usize, Option<usize>) {
    let px = lay.index_memory();
    assert_eq!(px.len(), 0x10000 * 8);
    let (mut bad, mut first) = (0usize, None);
    for a in 0..0x10000usize {
        let b = [vga.plane(0)[a], vga.plane(1)[a], vga.plane(2)[a], vga.plane(3)[a]];
        for bit in 0..8usize {
            let sh = 7 - bit;
            let color = ((b[0] >> sh) & 1) | (((b[1] >> sh) & 1) << 1) | (((b[2] >> sh) & 1) << 2) | (((b[3] >> sh) & 1) << 3);
            if px[a * 8 + bit] != color {
                bad += 1;
                first.get_or_insert(a * 8 + bit);
            }
        }
    }
    (bad, first)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flow {
    /// 不補間，alpha 1.0。
    Exact,
    /// 補間開啟：每步先在 alpha 0、0.25、0.5、0.75 渲染，再比 alpha 1.0。
    Interp,
}

/// 一對接收端與它的比對結果。
struct Pair {
    label: &'static str,
    vga: VgaSink,
    lay: LayerSink,
    opts: RenderOpts,
    frames: usize,
    empty: usize,
    px: usize,
    bad_px: usize,
    first_bad: Option<usize>,
    renders: usize,
    mem_bad: usize,
    mem_first_bad: Option<(usize, usize)>,
    detail: Vec<String>,
    /// 疊加模式（沒有 BIOS 字型而且 `show_counter`）：`LayerSink` 的幀計數只在輸出時疊上，不寫進記憶體，
    /// 這一幀沒有幀計數命令時不顯示，`VgaSink` 則顯示兩頁記憶體裡殘留的舊數字（`layer.rs` 的設計，
    /// 單元測試 `frame_counter_is_an_overlay_and_never_enters_memory` 固定它）。所以這一對只要求計數矩形以外逐像素相同，
    /// 矩形內的差異另計（`rect_px`）；索引色記憶體在矩形處必然不同，不比。
    overlay: bool,
    rect_px: usize,
}

/// 幀計數矩形：欄 `COUNTER_X`（12h）起 5 欄、列 `COUNTER_Y`（BFh）起 8 列。
fn in_counter_rect(i: usize) -> bool {
    let (x, y) = (i % 320, i / 320);
    (0x12 * 8..0x12 * 8 + 40).contains(&x) && (0xBF..0xBF + 8).contains(&y)
}

impl Pair {
    /// `font`：兩邊都設收據的 BIOS 字型（幀計數照原版永遠畫）。否則兩邊都不設，`counter` 決定幀計數畫不畫
    /// （`VgaSink::show_counter` 與 `RenderOpts::show_counter` 同值）。
    fn new(assets: &Assets, rc: &Receipt, label: &'static str, font: bool, counter: bool, flow: Flow) -> Pair {
        let mut vga = VgaSink::new(assets.clone());
        let mut lay = LayerSink::new(assets.clone(), None);
        let (planes, dac) = entry_planes(rc);
        vga.load_snapshot(planes, &dac);
        lay.load_snapshot(planes, &dac);
        if font {
            vga.set_counter_font(&rc.font);
            lay.set_counter_font(&rc.font);
        }
        vga.show_counter = counter;
        vga.set_stride_reg(entry_stride(rc));
        lay.set_stride_reg(entry_stride(rc));
        let opts = RenderOpts { interpolate: flow == Flow::Interp, hd: false, show_counter: counter, player: [None, None] };
        Pair {
            label,
            vga,
            lay,
            opts,
            frames: 0,
            empty: 0,
            px: 0,
            bad_px: 0,
            first_bad: None,
            renders: 0,
            mem_bad: 0,
            mem_first_bad: None,
            detail: Vec::new(),
            overlay: !font && counter,
            rect_px: 0,
        }
    }

    /// 一個事件（`n`：0 是初始化命令，之後是第 n 步）。`lay_draws` 是 `LayerSink` 收到的命令，
    /// 正常等於 `draws`（`VgaSink` 收到的）；負對照才改它。
    fn event(&mut self, n: usize, draws: &[DrawCmd], lay_draws: &[DrawCmd], flow: Flow, want: &mut [u8]) {
        for c in draws {
            self.vga.apply(c);
        }
        self.lay.push_frame(lay_draws);
        if draws.is_empty() {
            // `push_frame` 忽略空串，畫面維持上一幀，上一次已經比過。
            self.empty += 1;
            return;
        }
        if flow == Flow::Interp {
            for al in [0.0, 0.25, 0.5, 0.75] {
                let (img, w, h) = self.lay.render(al, self.opts);
                assert_eq!((w, h, img.len()), (320, 200, 320 * 200 * 4));
                self.renders += 1;
            }
        }
        let page = self.lay.draw_page();
        self.vga.render_page(page, want);
        let opts = self.opts;
        let (img, _, _) = self.lay.render(1.0, opts);
        self.renders += 1;
        let bad_at = |want: &[u8]| (0..320 * 200).filter(|&i| img[4 * i..4 * i + 3] != want[3 * i..3 * i + 3] || img[4 * i + 3] != 255).collect::<Vec<usize>>();
        let all_bad = bad_at(want);
        let (bad, in_rect): (Vec<usize>, Vec<usize>) = if self.overlay { all_bad.into_iter().partition(|&i| !in_counter_rect(i)) } else { (all_bad, Vec::new()) };
        self.rect_px += in_rect.len();
        self.frames += 1;
        self.px += 320 * 200;
        if !bad.is_empty() {
            self.bad_px += bad.len();
            if self.first_bad.is_none() {
                self.first_bad = Some(n);
                let i = bad[0];
                let mut other = vec![0u8; 320 * 200 * 3];
                self.vga.render_page(page ^ 0x8000, &mut other);
                let other_bad = bad_at(&other).len();
                let mut ops: BTreeMap<String, usize> = BTreeMap::new();
                for d in draws {
                    let k = match d {
                        DrawCmd::Blit(b) => format!("{:?}", b.op),
                        DrawCmd::SetDac { .. } => "SetDac".to_string(),
                        DrawCmd::SetDacOne { .. } => "SetDacOne".to_string(),
                        DrawCmd::VramFill { .. } => "VramFill".to_string(),
                        DrawCmd::SetMapMask { .. } => "SetMapMask".to_string(),
                    };
                    *ops.entry(k).or_default() += 1;
                }
                self.detail.push(format!(
                    "  [{}] 事件 {n}：{} 個像素不同，第一個在 ({}, {})，LayerSink {:?} VgaSink {:?}；畫頁 {page:04X}，換成另一頁與 LayerSink 不同 {other_bad} 像素；命令 {} 個：{:?}",
                    self.label,
                    bad.len(),
                    i % 320,
                    i / 320,
                    &img[4 * i..4 * i + 4],
                    &want[3 * i..3 * i + 3],
                    draws.len(),
                    ops
                ));
            }
        }
        if flow == Flow::Exact && !self.overlay {
            let (mb, first) = mem_diff(&self.vga, &self.lay);
            if mb > 0 {
                self.mem_bad += mb;
                if self.mem_first_bad.is_none() {
                    self.mem_first_bad = Some((n, first.unwrap()));
                }
            }
        }
    }
}

/// 一份收據的結果。
struct FlowRep {
    name: String,
    verdict: String,
    pass: bool,
    rows: usize,
    total: usize,
    events: usize,
    cmds: usize,
    pairs: Vec<Pair>,
}

type LayTweak<'a> = &'a dyn Fn(usize, &[DrawCmd]) -> Vec<DrawCmd>;

fn run_flow(flow: Flow, pairs_spec: &[(&'static str, bool, bool)], inputs: &Inputs, assets: &Assets, rc: &Receipt, tweak: Option<LayTweak>) -> FlowRep {
    let mut pairs: Vec<Pair> = pairs_spec.iter().map(|&(l, f, c)| Pair::new(assets, rc, l, f, c, flow)).collect();
    let mut want = vec![0u8; 320 * 200 * 3];
    let (mut events, mut cmds) = (0usize, 0usize);
    let v = {
        let mut obs = |ev: ReplayEvent| {
            let draws = match ev {
                ReplayEvent::Init(d) | ReplayEvent::Step(d) => d,
                ReplayEvent::StepAt { .. } => return,
            };
            cmds += draws.len();
            let lay_draws = tweak.map(|t| t(events, draws));
            for p in pairs.iter_mut() {
                p.event(events, draws, lay_draws.as_deref().unwrap_or(draws), flow, &mut want);
            }
            events += 1;
        };
        replay_observed(inputs, assets, rc, &ReplayOpts::default(), &mut obs)
    };
    let (verdict, pass) = match &v {
        Verdict::Pass { rows } => (format!("通過 {rows} 筆"), *rows == rc.rows.len()),
        other => (format!("{other:?}"), false),
    };
    FlowRep { name: rc.name.clone(), verdict, pass, rows: v.compared(), total: rc.rows.len(), events, cmds, pairs }
}

fn flow_report(title: &str, reps: &[FlowRep]) -> (usize, usize, usize) {
    let (mut frames, mut px, mut bad) = (0usize, 0usize, 0usize);
    for r in reps {
        println!("{title} {}：收據 {}（{}/{} 筆），事件 {}，命令 {}", r.name, r.verdict, r.rows, r.total, r.events, r.cmds);
        for p in &r.pairs {
            println!(
                "{title}   [{}] 比對 {} 幀（空串 {}）、{} 像素、不同 {} 像素{}，首個不同事件 {:?}，render {} 次；{}；異常 Layer {} Vga {}",
                p.label,
                p.frames,
                p.empty,
                p.px,
                p.bad_px,
                if p.overlay { format!("（計數矩形內另有 {} 像素不同，設計如此）", p.rect_px) } else { String::new() },
                p.first_bad,
                p.renders,
                if p.overlay { "全位址空間索引色不比".to_string() } else { format!("全位址空間索引色不同 {} 像素（首個 {:?}）", p.mem_bad, p.mem_first_bad) },
                p.lay.anomalies(),
                p.vga.anomalies()
            );
            for d in &p.detail {
                println!("{title}{d}");
            }
            frames += p.frames;
            px += p.px;
            bad += p.bad_px;
        }
    }
    (frames, px, bad)
}

fn flow_assert(reps: &[FlowRep], mem: bool) {
    let mut fails: Vec<String> = Vec::new();
    for r in reps {
        if !r.pass {
            fails.push(format!("{}：收據沒有每筆通過：{}", r.name, r.verdict));
        }
        for p in &r.pairs {
            if p.frames == 0 || p.px == 0 {
                fails.push(format!("{} [{}]：比對筆數為 0", r.name, p.label));
            }
            if p.bad_px > 0 {
                fails.push(format!("{} [{}]：與 VgaSink 不同 {} 像素，首個不同事件 {:?}", r.name, p.label, p.bad_px, p.first_bad));
            }
            if mem && p.mem_bad > 0 {
                fails.push(format!("{} [{}]：全位址空間索引色與 VgaSink 不同 {} 像素，首個 {:?}", r.name, p.label, p.mem_bad, p.mem_first_bad));
            }
            if p.lay.anomalies() != p.vga.anomalies() {
                fails.push(format!("{} [{}]：異常計數 Layer {} Vga {}", r.name, p.label, p.lay.anomalies(), p.vga.anomalies()));
            }
        }
    }
    assert!(fails.is_empty(), "{} 項失敗：\n{}", fails.len(), fails.join("\n"));
}

/// B6：全流程收據，原版圖、不補間，`LayerSink` 與 `VgaSink::render_page(draw_page)` 逐像素相同。
///
/// 三對接收端同步比對：
/// - `font`：設 BIOS 字型，幀計數照原版永遠畫（收據的實況）；
/// - `nofont`：不設字型、幀計數不畫（`show_counter: false`）；
/// - `counter`：不設字型、幀計數畫（`show_counter: true`，`LayerSink` 在輸出時疊上）。
/// 另比整個 64 KB 位址空間（含暫存區與沒顯示的頁）的索引色與 `VgaSink` 四個平面，每個有命令的事件一次。
#[test]
#[ignore]
fn l8_full_flow_equivalence() {
    let (inputs, assets) = flow_inputs();
    let t0 = Instant::now();
    let mut reps = Vec::new();
    for dir in receipt_dirs() {
        let rc = load_receipt(&dir).expect("載入收據");
        let t = Instant::now();
        let r = run_flow(Flow::Exact, &[("font", true, false), ("nofont", false, false), ("counter", false, true)], &inputs, &assets, &rc, None);
        println!("L8 {} 耗時 {:.1} 秒", r.name, t.elapsed().as_secs_f32());
        reps.push(r);
    }
    let (frames, px, bad) = flow_report("L8", &reps);
    println!("L8 合計：收據 {} 份，比對 {frames} 幀、{px} 像素、不同 {bad} 像素；耗時 {:.1} 秒", reps.len(), t0.elapsed().as_secs_f32());
    assert!(frames > 0 && px > 0, "比對筆數為 0");
    flow_assert(&reps, true);
}

/// B6 補間部分：補間開啟，每步先在 alpha 0、0.25、0.5、0.75 渲染，alpha 1.0 仍與 `VgaSink` 逐像素相同（L2 的全流程版）。
#[test]
#[ignore]
fn l8b_full_flow_interpolation_does_not_change_the_next_frame() {
    let (inputs, assets) = flow_inputs();
    let t0 = Instant::now();
    let mut reps = Vec::new();
    for dir in receipt_dirs() {
        let rc = load_receipt(&dir).expect("載入收據");
        let t = Instant::now();
        let r = run_flow(Flow::Interp, &[("font", true, false), ("counter", false, true)], &inputs, &assets, &rc, None);
        println!("L8b {} 耗時 {:.1} 秒", r.name, t.elapsed().as_secs_f32());
        reps.push(r);
    }
    let (frames, px, bad) = flow_report("L8b", &reps);
    let renders: usize = reps.iter().flat_map(|r| r.pairs.iter()).map(|p| p.renders).sum();
    println!("L8b 合計：收據 {} 份，比對 {frames} 幀、{px} 像素、不同 {bad} 像素，render {renders} 次；耗時 {:.1} 秒", reps.len(), t0.elapsed().as_secs_f32());
    assert!(frames > 0 && px > 0 && renders >= frames * 5, "比對筆數為 0");
    flow_assert(&reps, false);
}

/// HD 缺圖清單的一項：`SHT1/variants/stage1_boss/0033.png（尺寸 ...）` 拆成圖庫、調色盤變體、圖號。
fn missing_key(m: &str) -> (String, String, String) {
    let path = m.split('（').next().unwrap_or(m);
    let parts: Vec<&str> = path.split('/').collect();
    let lib = parts.first().copied().unwrap_or("?").to_string();
    let idx = parts.last().copied().unwrap_or("?").trim_end_matches(".png").to_string();
    let var = match parts.as_slice() {
        [_, "variants", pal, _] => (*pal).to_string(),
        [_, "masks", _] => "（遮罩）".to_string(),
        _ => "（基本圖）".to_string(),
    };
    (lib, var, idx)
}

/// B6 HD 部分：全流程收據用 HD 圖跑，`missing_hd()` 的完整清單。
///
/// 每個事件都 `push_frame`（`hd::apply` 在這裡對每個命令查 HD 圖並記缺圖）。`render` 較貴，所以取樣：
/// 每 `HD_RENDER_EVERY` 個事件、每次調色盤名稱改變、以及每份收據的最後一個事件各 render 一次（alpha 0.5 補間與 alpha 1.0 不補間
/// 交替），檢查尺寸與非全黑。缺圖的偵測不受取樣影響，因為它只在 `push_frame` 內發生。
/// 越界圖號（片尾第四關殘留實體的 SHT5 圖號 9）在 `HdAssets::image` 內因原版圖庫沒有該項而回傳 `None`，不查檔，也不算缺圖。
const HD_RENDER_EVERY: usize = 24;

#[test]
#[ignore]
fn l9_full_flow_hd_coverage() {
    let hd_dir = env_path("RS2_HD");
    let (inputs, assets) = flow_inputs();
    let t0 = Instant::now();
    let mut all_missing: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
    let mut total_events = 0usize;
    let mut total_renders = 0usize;
    let mut palettes_seen: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut unnamed_frames = 0usize;
    let mut fails: Vec<String> = Vec::new();
    for dir in receipt_dirs() {
        let rc = load_receipt(&dir).expect("載入收據");
        let t = Instant::now();
        let hd = HdAssets::open(&hd_dir).unwrap();
        let mut lay = LayerSink::new(assets.clone(), Some(hd));
        let (planes, dac) = entry_planes(&rc);
        lay.load_snapshot(planes, &dac);
        lay.set_stride_reg(entry_stride(&rc));
        let (mut events, mut renders, mut last_pal) = (0usize, 0usize, None::<&'static str>);
        let mut rc_palettes: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut rc_unnamed = 0usize;
        let mut unnamed_span: Option<(usize, usize)> = None;
        let mut bad_render: Vec<String> = Vec::new();
        let v = {
            let mut obs = |ev: ReplayEvent<'_>| {
                let draws = match ev {
                    ReplayEvent::Init(d) | ReplayEvent::Step(d) => d,
                    ReplayEvent::StepAt { .. } => return,
                };
                lay.push_frame(draws);
                let pal = lay.palette_name();
                if !draws.is_empty() {
                    match pal {
                        Some(p) => *rc_palettes.entry(p).or_default() += 1,
                        None => {
                            rc_unnamed += 1;
                            unnamed_span = Some(unnamed_span.map_or((events, events), |(a, _)| (a, events)));
                        }
                    }
                }
                let sample = !draws.is_empty() && (events % HD_RENDER_EVERY == 0 || pal != last_pal);
                last_pal = pal;
                if sample {
                    let interp = (events / HD_RENDER_EVERY) % 2 == 0;
                    let (img, w, h) = lay.render(if interp { 0.5 } else { 1.0 }, RenderOpts { interpolate: interp, hd: true, show_counter: false, player: [None, None] });
                    if (w, h, img.len()) != (1280, 800, 1280 * 800 * 4) {
                        bad_render.push(format!("事件 {events}：輸出尺寸 {w}x{h}"));
                    }
                    renders += 1;
                }
                events += 1;
            };
            replay_observed(&inputs, &assets, &rc, &ReplayOpts::default(), &mut obs)
        };
        // 最後一個事件的畫面。
        let (img, w, h) = lay.render(1.0, RenderOpts { interpolate: false, hd: true, show_counter: false, player: [None, None] });
        renders += 1;
        let nonblack = img.chunks_exact(4).filter(|p| p[..3] != [0, 0, 0]).count();
        let missing: Vec<String> = lay.missing_hd().to_vec();
        println!(
            "L9 {}：收據 {}，事件 {events}，render {renders} 次（最後一幀 {w}x{h}，非黑 {nonblack} 像素），載入 HD 檔 {} 個，缺圖 {} 項，耗時 {:.1} 秒；調色盤名稱的幀數 {:?}，未辨識 {rc_unnamed}（事件範圍 {unnamed_span:?}）",
            rc.name,
            match &v {
                Verdict::Pass { rows } => format!("通過 {rows} 筆"),
                other => format!("{other:?}"),
            },
            lay.loaded_hd().len(),
            missing.len(),
            t.elapsed().as_secs_f32(),
            rc_palettes
        );
        for m in &missing {
            println!("L9   缺：{m}");
        }
        if !matches!(v, Verdict::Pass { rows } if rows == rc.rows.len()) {
            fails.push(format!("{}：收據沒有每筆通過：{v:?}", rc.name));
        }
        fails.extend(bad_render.into_iter().map(|b| format!("{}：{b}", rc.name)));
        if nonblack == 0 {
            fails.push(format!("{}：最後一幀 HD 全黑", rc.name));
        }
        for m in missing {
            all_missing.entry(missing_key(&m)).or_default().push(format!("{} {m}", rc.name));
        }
        total_events += events;
        total_renders += renders;
        for (k, n) in rc_palettes {
            *palettes_seen.entry(k).or_default() += n;
        }
        unnamed_frames += rc_unnamed;
    }
    println!("L9 合計：事件 {total_events}，render {total_renders} 次；耗時 {:.1} 秒", t0.elapsed().as_secs_f32());
    println!("L9 全流程出現的調色盤名稱（幀數）：{palettes_seen:?}；未辨識 {unnamed_frames} 幀");
    println!("L9 缺圖共 {} 項（圖庫, 調色盤變體, 圖號）：", all_missing.len());
    for (k, who) in &all_missing {
        println!("L9   {k:?}  <- {} 份收據：{:?}", who.len(), who.iter().map(|s| s.split(' ').next().unwrap_or("")).collect::<Vec<_>>());
    }
    assert!(total_events > 0 && total_renders > 0, "比對筆數為 0");
    assert!(fails.is_empty(), "{} 項失敗：\n{}", fails.len(), fails.join("\n"));
    assert!(all_missing.is_empty(), "HD 全流程缺圖 {} 項：{:?}", all_missing.len(), all_missing.keys().collect::<Vec<_>>());
}

/// 負對照：`LayerSink` 收到被改過的命令（一個 OR 圖形的 x 加 1、每 7 個命令丟 1 個、整個事件丟掉），
/// 全流程的比對必須失敗；沒改的基準必須零差異。證明比對在全流程路徑上不是空轉。
#[test]
#[ignore]
fn l8c_full_flow_negative_controls_fail() {
    let (inputs, assets) = flow_inputs();
    let rc = load_receipt(&env_path("RS2_DOSRUN").join("wt_stage2")).expect("載入收據");
    let spec = [("font", true, false)];
    let run = |tweak: Option<LayTweak>| run_flow(Flow::Exact, &spec, &inputs, &assets, &rc, tweak);

    let base = run(None);
    let b = &base.pairs[0];
    println!("L8c 基準 {}：比對 {} 幀、不同 {} 像素", base.name, b.frames, b.bad_px);
    assert!(base.pass && b.frames > 0 && b.bad_px == 0, "基準應零差異：{}", base.verdict);

    // x 加 1（位元組，8 像素）：第 100 個事件之後第一個帶物件的 OR 圖形。
    let hit = std::cell::Cell::new(None::<usize>);
    let shift = |n: usize, d: &[DrawCmd]| -> Vec<DrawCmd> {
        let mut v = d.to_vec();
        if n >= 100 && hit.get().is_none() {
            if let Some(DrawCmd::Blit(c)) = v.iter_mut().find(|x| matches!(x, DrawCmd::Blit(c) if c.op == Op::S18F5D && c.obj.is_some())) {
                c.x = c.x.wrapping_add(1);
                hit.set(Some(n));
            }
        }
        v
    };
    let r = run(Some(&shift));
    let p = &r.pairs[0];
    println!("L8c x+1（第 {:?} 個事件）：不同 {} 像素，首個不同事件 {:?}", hit.get(), p.bad_px, p.first_bad);
    assert!(hit.get().is_some(), "找不到可改的命令");
    assert!(p.bad_px > 0, "x 加 1 卻沒有差異");
    assert!(p.first_bad.unwrap() >= hit.get().unwrap(), "差異不應早於被改的那一步");

    // 每 7 個命令丟 1 個。
    let cnt = std::cell::Cell::new(0usize);
    let drop7 = |_n: usize, d: &[DrawCmd]| -> Vec<DrawCmd> {
        d.iter()
            .filter(|_| {
                cnt.set(cnt.get() + 1);
                cnt.get() % 7 != 0
            })
            .copied()
            .collect()
    };
    let r = run(Some(&drop7));
    let p = &r.pairs[0];
    println!("L8c 每 7 個丟 1 個：不同 {} 像素，首個不同事件 {:?}", p.bad_px, p.first_bad);
    assert!(p.bad_px > 1000, "丟了命令卻幾乎沒有差異");

    // 整個事件丟掉。
    let drop_ev = |n: usize, d: &[DrawCmd]| -> Vec<DrawCmd> { if n == 150 { Vec::new() } else { d.to_vec() } };
    let r = run(Some(&drop_ev));
    let p = &r.pairs[0];
    println!("L8c 丟掉第 150 個事件：不同 {} 像素，首個不同事件 {:?}", p.bad_px, p.first_bad);
    assert!(p.bad_px > 0 && p.first_bad.unwrap() >= 150, "丟掉整個事件卻沒有差異");
}
