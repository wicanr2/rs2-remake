//! 規格 001 驗收 A5：補間關閉，`a0_demo` 七個時點的 4 個平面與 6 位元 DAC 都和傾印相同。
//! 平面 3 排除幀計數矩形（兩頁的列 191～198、位元組欄 18～22）。
//! 需要原版檔與收據，由 `tools/rust.sh test -p rs2-trace --test a5 -- --ignored --nocapture` 跑。
//! 比對筆數為 0 視為失敗。

use rs2_core::{DrawCmd, Dseg, Fixes, Game, Input, Start};
use rs2_gfx::{Assets, VgaSink, disp_row, locate, scratch_row};
use rs2_trace::{load_inputs, parse_sc};
use std::path::PathBuf;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 跑")))
}

/// 取樣時點：`on_sample` 回呼的第 n 筆，也就是第 n 步。
const TARGETS: [usize; 7] = [0, 1, 99, 299, 620, 622, 639];

/// 規格 A5 唯一的豁免：幀計數矩形，平面 3，兩頁的列 191～198、位元組欄 18～22。
fn excluded(plane: usize, off: usize) -> bool {
    if plane != 3 {
        return false;
    }
    // 頁 0 的偏移是 列*40+欄，頁 1 再加 8000h。兩頁各佔 8000 位元組。
    let o = off & 0x7FFF;
    if o >= 8000 {
        return false;
    }
    let (row, col) = (o / 40, o % 40);
    (191..=198).contains(&row) && (18..=22).contains(&col)
}

/// 取 `demo_n{n}.json` 的 `shown16_dac6`：16 組 6 位元 RGB。
fn read_shown16(path: &std::path::Path) -> [[u8; 3]; 16] {
    let text = std::fs::read_to_string(path).unwrap();
    let key = text.find("\"shown16_dac6\"").expect("沒有 shown16_dac6");
    let rest = &text[key..];
    let start = rest.find('[').unwrap();
    let mut depth = 0;
    let mut nums: Vec<u8> = Vec::new();
    let mut cur: Option<u32> = None;
    for ch in rest[start..].chars() {
        match ch {
            '[' => depth += 1,
            ']' => {
                if let Some(v) = cur.take() {
                    nums.push(v as u8);
                }
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            '0'..='9' => cur = Some(cur.unwrap_or(0) * 10 + ch.to_digit(10).unwrap()),
            _ => {
                if let Some(v) = cur.take() {
                    nums.push(v as u8);
                }
            }
        }
    }
    assert_eq!(nums.len(), 48, "shown16_dac6 應有 48 個數字");
    let mut o = [[0u8; 3]; 16];
    for i in 0..16 {
        o[i] = [nums[3 * i], nums[3 * i + 1], nums[3 * i + 2]];
    }
    o
}

fn json_field<'a>(text: &'a str, key: &str) -> &'a str {
    let k = text.find(&format!("\"{key}\"")).unwrap_or_else(|| panic!("沒有 {key}"));
    let rest = &text[k..];
    let colon = rest.find(':').unwrap();
    let v = rest[colon + 1..].trim_start();
    let end = v.find([',', '\n']).unwrap();
    v[..end].trim().trim_matches('"')
}

/// `render_page` 的檢查：dosgolem 的畫面雜湊 `rgb_sha256` 是顯示頁 320x200 RGB 的 SHA-256，
/// 6 位元轉 8 位元用 `v << 2 | v >> 4`，而 `render_page` 依規定用 `(v * 255 + 31) / 63`。
/// 所以先把 `render_page` 的輸出按 6 位元值一對一還原，再用 dosgolem 的式子轉回去才能比雜湊。
/// 幀計數矩形的字型不同（原版 BIOS 字型、重製自繪），矩形內的像素改由傾印的平面算出。
fn check_render(sink: &VgaSink, dir: &std::path::Path, tag: &str, out: &mut Outcome) {
    let meta = std::fs::read_to_string(dir.join(format!("{tag}.json"))).unwrap();
    let page: u16 = json_field(&meta, "display_start").parse().unwrap();
    let want_hash = json_field(&meta, "rgb_sha256").to_string();
    let mut img = vec![0u8; 320 * 200 * 3];
    sink.render_page(page, &mut img);
    let to8 = |v: u32| (v * 255 + 31) / 63;
    let mut inv = [0u8; 256];
    for v in 0..64u32 {
        inv[to8(v) as usize] = v as u8;
    }
    let theirs = |v: u8| (v << 2) | (v >> 4);
    for b in img.iter_mut() {
        *b = theirs(inv[*b as usize]);
    }
    let dump: Vec<Vec<u8>> = (0..4).map(|p| std::fs::read(dir.join(format!("{tag}.p{p}"))).unwrap()).collect();
    let ac = [0u8, 1, 2, 3, 4, 5, 6, 7, 56, 57, 58, 59, 60, 61, 62, 63];
    for row in 191..=198usize {
        for x in 144..184usize {
            let a = page as usize + row * 40 + x / 8;
            let sh = 7 - (x % 8);
            let mut color = 0usize;
            for p in 0..4 {
                color |= (((dump[p][a] >> sh) & 1) as usize) << p;
            }
            let d = sink.dac(ac[color]);
            let o = (row * 320 + x) * 3;
            for ch in 0..3 {
                img[o + ch] = theirs(d[ch]);
            }
        }
    }
    let got = rs2_formats::sha256_hex(&img);
    out.render_checks += 1;
    if got != want_hash {
        out.failures.push(format!("{tag} 顯示頁 {page:04X} 的 RGB 雜湊不同：重製 {got} 原版 {want_hash}"));
    }
}

/// 回報用：重跑同一串命令，監看那個位元組，找出最後寫到它的命令。
fn last_writer(assets: &Assets, cmds: &[DrawCmd], plane: usize, off: u16) -> String {
    let mut s = VgaSink::new(assets.clone());
    s.watch(plane, off);
    for c in cmds {
        s.apply(c);
    }
    match s.last_write_seq() {
        Some(i) => format!("最後寫入：命令 #{i} = {:?}", cmds[i as usize]),
        None => "整串命令都沒有寫過這個位元組".to_string(),
    }
}

/// 列位址表的規則核對：重製用 `40 * 列`、`1F40h + 80 * 列` 計算，要和映像的 `dseg:A00Ah`、`A19Ah` 逐項相同。
#[test]
#[ignore]
fn row_tables_match_image() {
    let inputs = load_inputs(&env_path("RS2_ORIG")).unwrap();
    let ds = Dseg::from_image(&inputs.image);
    let mut checked = 0;
    for r in 0..400u16 {
        assert_eq!(ds.r16(0xA00A + 2 * r), disp_row(r), "顯示頁列表第 {r} 項");
        checked += 1;
    }
    for r in 0..330u16 {
        assert_eq!(ds.r16(0xA19A + 2 * r), scratch_row(r), "暫存區列表第 {r} 項");
        checked += 1;
    }
    println!("列位址表核對 {checked} 項");
    assert!(checked > 0);
}

struct Outcome {
    compared_bytes: usize,
    compared_dac: usize,
    /// 不含豁免矩形的不同位元組總數。
    diff_total: usize,
    failures: Vec<String>,
    ops: std::collections::BTreeMap<String, usize>,
    max_rows: (u16, u16),
    anomalies: u64,
    /// 第 639 步平面 3 的幀計數矩形內非零位元組數。
    counter_rect_nonzero: usize,
    render_checks: usize,
}

/// 跑 `Start::Demo` 640 步，把命令餵給 `VgaSink`，在 `TARGETS` 比對。
/// `drop_every` 只給負對照用：每 N 個命令丟掉一個，比對應該失敗。
fn run_a5(show_counter: bool, drop_every: Option<usize>) -> Outcome {
    let dir = env_path("RS2_ORIG");
    let run = env_path("RS2_DOSRUN");
    let inputs = load_inputs(&dir).unwrap();
    let assets = Assets::load(&dir).unwrap();
    let mut g = Game::new(&inputs.image, &inputs.dm1, Start::Demo, Fixes::NONE);
    let mut sink = VgaSink::new(assets.clone());
    sink.show_counter = show_counter;
    let mut cmds: Vec<DrawCmd> = Vec::new();
    let mut ops: std::collections::BTreeMap<String, usize> = Default::default();
    let mut feed = |sink: &mut VgaSink, cmds: &mut Vec<DrawCmd>, c: DrawCmd| {
        let name = match &c {
            DrawCmd::Blit(b) => format!("{:?}", b.op),
            DrawCmd::SetDac { .. } => "SetDac".to_string(),
            DrawCmd::SetDacOne { .. } => "SetDacOne".to_string(),
            DrawCmd::VramFill { .. } => "VramFill".to_string(),
            DrawCmd::SetMapMask { .. } => "SetMapMask".to_string(),
        };
        *ops.entry(name).or_default() += 1;
        cmds.push(c);
        if let Some(k) = drop_every {
            if cmds.len() % k == 0 {
                return;
            }
        }
        sink.apply(&c);
    };
    for c in g.take_draws() {
        feed(&mut sink, &mut cmds, c);
    }

    let text = std::fs::read_to_string(run.join("trace_dm1/trace-dm1.jsonl")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 640);

    let mut out = Outcome {
        compared_bytes: 0,
        compared_dac: 0,
        diff_total: 0,
        failures: Vec::new(),
        ops: Default::default(),
        max_rows: (0, 0),
        anomalies: 0,
        counter_rect_nonzero: 0,
        render_checks: 0,
    };
    for n in 0..=639usize {
        let sc = parse_sc(lines[n]);
        let mut sampled = false;
        let frame = g.step(&Input { scancodes: &sc }, &mut |_g: &Game, _pt: u32| sampled = true);
        assert!(sampled, "第 {n} 步沒有取樣，exit={:?}", frame.exit);
        for c in frame.draws {
            feed(&mut sink, &mut cmds, c);
        }
        if !TARGETS.contains(&n) {
            continue;
        }
        let tag = format!("demo_n{n}");
        let mut diff_n = 0usize;
        for p in 0..4 {
            let want = std::fs::read(run.join("a0_demo").join(format!("{tag}.p{p}"))).unwrap();
            assert_eq!(want.len(), 0x10000, "{tag}.p{p}");
            let got = sink.plane(p);
            let mut first: Option<usize> = None;
            let mut cnt = 0usize;
            for o in 0..0x10000usize {
                if excluded(p, o) {
                    if n == 639 && p == 3 && got[o] != 0 {
                        out.counter_rect_nonzero += 1;
                    }
                    continue;
                }
                out.compared_bytes += 1;
                if got[o] != want[o] {
                    cnt += 1;
                    first.get_or_insert(o);
                }
            }
            diff_n += cnt;
            if let Some(o) = first {
                // 命令序號與 `cmds` 的下標一致，丟命令的負對照不在乎這個。
                let why = if drop_every.is_some() { String::new() } else { last_writer(&assets, &cmds, p, o as u16) };
                out.failures.push(format!(
                    "n={n} 平面 {p} 不同 {cnt} 位元組，第一個在偏移 {o:04X}（{}）重製 {:02X} 原版 {:02X}；{why}",
                    locate(o as u16),
                    got[o],
                    want[o],
                ));
            }
        }
        out.diff_total += diff_n;
        if drop_every.is_none() && !show_counter {
            check_render(&sink, &run.join("a0_demo"), &tag, &mut out);
        }
        let want_dac = read_shown16(&run.join("a0_demo").join(format!("{tag}.json")));
        let got_dac = sink.shown16();
        out.compared_dac += 16;
        if got_dac != want_dac {
            let i = (0..16).find(|&i| got_dac[i] != want_dac[i]).unwrap();
            out.failures.push(format!("n={n} DAC 色號 {i} 重製 {:?} 原版 {:?}", got_dac[i], want_dac[i]));
        }
        println!("n={n}: 平面不同 {diff_n} 位元組，累計命令 {}", sink.applied());
    }
    out.ops = ops;
    out.max_rows = sink.max_table_rows();
    out.anomalies = sink.anomalies();
    if sink.anomalies() > 0 {
        println!("無法重現的命令 {} 次 {:?}", sink.anomalies(), sink.anomaly_log());
    }
    out
}

fn report(o: &Outcome) {
    println!(
        "A5 比對平面 {} 位元組、DAC {} 組，不同 {} 位元組；列表查詢最大列號 顯示頁 {} 暫存區 {}",
        o.compared_bytes, o.compared_dac, o.diff_total, o.max_rows.0, o.max_rows.1
    );
    println!("顯示頁 RGB 雜湊核對 {} 筆", o.render_checks);
    println!("命令分布 {:?}", o.ops);
}

#[test]
#[ignore]
fn a5_screen() {
    let o = run_a5(false, None);
    report(&o);
    assert!(o.compared_bytes > 0 && o.compared_dac > 0 && o.render_checks > 0, "比對筆數為 0");
    assert_eq!(o.compared_bytes, TARGETS.len() * 4 * 0x10000 - TARGETS.len() * 2 * 8 * 5, "比對範圍應只扣掉幀計數矩形");
    assert!(o.max_rows.0 < 400 && o.max_rows.1 < 330, "列位址表查詢超出核對範圍");
    assert_eq!(o.anomalies, 0);
    assert!(o.failures.is_empty(), "A5 失敗 {} 項：\n{}", o.failures.len(), o.failures.join("\n"));
}

/// 幀計數打開時，矩形之外的內容不受影響，矩形內有畫東西。
#[test]
#[ignore]
fn a5_screen_with_counter() {
    let o = run_a5(true, None);
    report(&o);
    println!("幀計數矩形內非零位元組 {}", o.counter_rect_nonzero);
    assert!(o.compared_bytes > 0);
    assert!(o.counter_rect_nonzero > 0, "打開幀計數卻沒有畫");
    assert!(o.failures.is_empty(), "A5 失敗 {} 項：\n{}", o.failures.len(), o.failures.join("\n"));
}

/// 頭目擊殺收據的平面傾印時點：`a5_boss/boss_{N}` 是第 N 筆取樣寫出之後，對應第 N−1 步。
const BOSS_TARGETS: [usize; 12] = [333, 621, 623, 633, 733, 933, 1134, 1142, 1151, 1152, 1170, 1192];

/// 頭目合成圖第一次寫進暫存區 B 的步數。`sub_10C3D` 在這一步結束時才清掉過場殘圖，
/// 取樣點在它之前，所以這一步之前的傾印不比暫存區 B（規格 001「初始狀態」）。
const SCRATCH_B_VALID_FROM: usize = 621;

/// A5 的頭目部分：頭目合成圖、損壞貼片、血條、連爆。新局入口，用 `boss1_kill` 的掃描碼。
#[test]
#[ignore]
fn a5_boss_screen() {
    let dir = env_path("RS2_ORIG");
    let run = env_path("RS2_DOSRUN");
    let inputs = load_inputs(&dir).unwrap();
    let assets = Assets::load(&dir).unwrap();
    let mut g = Game::new(&inputs.image, &inputs.dm1, Start::NewGame { difficulty: None }, Fixes::NONE);
    let mut sink = VgaSink::new(assets);
    for c in g.take_draws() {
        sink.apply(&c);
    }
    let text = std::fs::read_to_string(run.join("boss1_kill/trace-s1.jsonl")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1192);
    let (mut compared, mut skipped_b, mut dac) = (0usize, 0usize, 0usize);
    let mut failures: Vec<String> = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let sc = parse_sc(line);
        let mut sampled = false;
        let frame = g.step(&Input { scancodes: &sc }, &mut |_g: &Game, _pt: u32| sampled = true);
        assert!(sampled, "第 {n} 步沒有取樣，exit={:?}", frame.exit);
        for c in &frame.draws {
            sink.apply(c);
        }
        if !BOSS_TARGETS.contains(&(n + 1)) {
            continue;
        }
        let tag = format!("boss_{}", n + 1);
        for p in 0..4 {
            let want = std::fs::read(run.join("a5_boss").join(format!("{tag}.p{p}"))).unwrap();
            assert_eq!(want.len(), 0x10000, "{tag}.p{p}");
            let got = sink.plane(p);
            let (mut cnt, mut first) = (0usize, None);
            for o in 0..0x10000usize {
                if excluded(p, o) {
                    continue;
                }
                if n < SCRATCH_B_VALID_FROM && o >= 0x9F40 {
                    skipped_b += 1;
                    continue;
                }
                compared += 1;
                if got[o] != want[o] {
                    cnt += 1;
                    first.get_or_insert(o);
                }
            }
            if let Some(o) = first {
                failures.push(format!(
                    "{tag} 平面 {p} 不同 {cnt} 位元組，第一個在偏移 {o:04X}（{}）重製 {:02X} 原版 {:02X}",
                    locate(o as u16),
                    got[o],
                    want[o],
                ));
            }
        }
        let want_dac = read_shown16(&run.join("a5_boss").join(format!("{tag}.json")));
        dac += 16;
        if sink.shown16() != want_dac {
            failures.push(format!("{tag} DAC 不同"));
        }
    }
    println!("A5 頭目：比對平面 {compared} 位元組、DAC {dac} 組；第 621 步前略過暫存區 B {skipped_b} 位元組");
    assert!(compared > 0 && dac > 0, "比對筆數為 0");
    assert!(failures.is_empty(), "A5 頭目失敗 {} 項：\n{}", failures.len(), failures.join("\n"));
}

/// 負對照：每 7 個命令丟一個，比對必須看得出差異。
#[test]
#[ignore]
fn a5_negative_control() {
    let o = run_a5(false, Some(7));
    println!("負對照：丟命令後不同 {} 位元組，失敗項 {}", o.diff_total, o.failures.len());
    assert!(o.diff_total > 1000, "丟了命令卻幾乎沒有差異，比對沒有鑑別力");
    assert!(!o.failures.is_empty());
}
