//! 差分模糊測試：`RS2_DOSRUN/fuzz_*` 下 dosgolem 跑原版錄的逐幀收據，與 Rust 重播逐筆比對。
//! 腳本與 poke 由 `tools/dos/gen_fuzz.py` 產生（`tools/dos/fuzz/gen.sh`），收據由
//! `tools/dos/run_scenario.sh fuzz-PROFILE-SEED fuzz_PROFILE-SEED` 錄製。
//!
//! 一個收據目錄的檔案：
//! - `trace-fz.jsonl`：逐幀紀錄，`sc` 欄是這一步之前送達 INT 9 的掃描碼。
//! - `trace-fz.end`：`rows=N exit=none|E|Q|C`，rs2dyn 的 `settle` 在最後一筆那半步走完後寫的，是原版自己的結論。
//! - `tools/dos/fuzz/PROFILE-SEED.pokes`：`n 線性位址 寬 值`，在第 n-1 筆取樣之後、第 n 步之前寫入資料段。
//!   n 超過實際筆數的列是主迴圈離開之後才送的，略過。
//! - `ds-fz-N.ds`：rs2dyn 的 `dsdump` 在第 N 筆取樣點存的整個 64 KB 資料段。重製在同一個取樣點
//!   （`on_sample` 回呼，該筆的 `sub_11097(2)` 之前）逐位元組比對，JSONL 沒涵蓋的變數也會被查到。
//!   豁免範圍見 `A0_EXEMPT` 與 `DS_EXTRA_EXEMPT`。
//!
//! 比對筆數為 0、收據缺 `.end`、收據筆數與 `.end` 不符，都算失敗。
//! 環境變數：`RS2_FUZZ` 給子字串時只跑名稱含它的收據；`RS2_DS_REPORT` 有值時資料段差異改為印出（含豁免）不判失敗，供探索用。

use rs2_core::{A0_EXEMPT, Exit, Fixes, Game, Input, Start, a};
use rs2_trace::{describe_diff, load_inputs, parse_sc, row_json, strip_emu, Inputs};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

struct Poke {
    row: usize,
    addr: u32,
    width: u8,
    val: u16,
}

struct End {
    rows: usize,
    exit: Option<Exit>,
}

#[derive(Default)]
struct Stats {
    rows: usize,
    pokes_applied: usize,
    pokes_total: usize,
    ds_checked: usize,
}

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 跑")))
}

/// 從 `CARGO_MANIFEST_DIR` 往上找含 `tools/dos/fuzz` 的目錄。
fn pokes_dir() -> PathBuf {
    let start = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("缺 CARGO_MANIFEST_DIR，要用 cargo test 跑"));
    for dir in start.ancestors() {
        let p = dir.join("tools/dos/fuzz");
        if p.is_dir() {
            return p;
        }
    }
    panic!("從 {} 往上找不到 tools/dos/fuzz", start.display());
}

fn num(s: &str) -> Result<u32, String> {
    let r = match s.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16),
        None => s.parse::<u32>(),
    };
    r.map_err(|e| format!("數字 {s:?} 解不開：{e}"))
}

fn load_pokes(path: &Path) -> Result<Vec<Poke>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("讀不到 {}：{e}", path.display()))?;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 4 {
            return Err(format!("{}:{}：要有 4 欄 `n 位址 寬 值`：{line}", path.display(), i + 1));
        }
        let width = num(f[2])? as u8;
        if width != 1 && width != 2 {
            return Err(format!("{}:{}：寬只能是 1 或 2：{line}", path.display(), i + 1));
        }
        out.push(Poke { row: num(f[0])? as usize, addr: num(f[1])?, width, val: num(f[3])? as u16 });
    }
    out.sort_by_key(|p| p.row); // 穩定排序，同一筆內維持檔案順序
    Ok(out)
}

fn load_end(path: &Path) -> Result<End, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("讀不到 {}：{e}（腳本沒有 settle？）", path.display()))?;
    let mut rows = None;
    let mut exit = None;
    for tok in text.split_whitespace() {
        if let Some(v) = tok.strip_prefix("rows=") {
            rows = Some(v.parse::<usize>().map_err(|e| format!("{}：rows 解不開：{e}", path.display()))?);
        } else if let Some(v) = tok.strip_prefix("exit=") {
            exit = Some(match v {
                "none" => None,
                "E" => Some(Exit::End),
                "Q" => Some(Exit::Quit),
                "C" => Some(Exit::Clear),
                _ => return Err(format!("{}：不認得的離開碼 {v:?}", path.display())),
            });
        }
    }
    match (rows, exit) {
        (Some(rows), Some(exit)) => Ok(End { rows, exit }),
        _ => Err(format!("{}：要有 rows= 與 exit=：{text:?}", path.display())),
    }
}

fn exit_name(e: Option<Exit>) -> &'static str {
    match e {
        None => "遊戲仍在進行",
        Some(Exit::End) => "離開 E（玩家全滅）",
        Some(Exit::Quit) => "離開 Q",
        Some(Exit::Clear) => "離開 C（過關）",
    }
}


/// 資料段比對的豁免範圍：規格 001 的 A0 豁免（啟動函式、文字畫面、音樂驅動、JOYMOUSE、堆疊殘值）。
/// 逐幀比對另外發現的原版專屬變數記在 `DS_EXTRA_EXEMPT`，每一項要寫理由。
const DS_EXTRA_EXEMPT: &[(u16, u16, &str)] = &[
    // 繪製函式的暫存字組 word_2685E、26860、26862（dseg:9C9E 到 9CA3）：只有 sub_19DA8、19E66、19F3B、19FF9、1A09F、1A144、1A1E6 讀寫。
    (0x9C9E, 0x9CA3, "blit 暫存"),
    // word_2688A：sub_1100A、1117F、162B1 等寫，sub_18993 與 sub_19A0B（blit）讀，是繪製參數。
    (0x9CCA, 0x9CCA, "繪製參數"),
    // byte_26890：sub_1890D、18993、189C1、162B1 寫，sub_18A1F、18A61（數字繪製）讀。
    (0x9CD0, 0x9CD0, "數字繪製暫存"),
    // 特效領域重寫（規格 004）：工作變數與參數改成區域變數、函式參數，與 `rs2_core::DS_EXEMPT` 的同名四項相同。
    // word_20B74：只有 sub_17C6C 寫後讀。
    (0x3FB4, 0x3FB5, "特效腳本指標暫存"),
    // word_20B76：sub_17CB6 寫，sub_17D2C 讀。
    (0x3FB6, 0x3FB7, "特效形狀指標"),
    // word_20B78：呼叫端寫，只有 sub_17A3A 讀；重製改成 EffectKind 參數。
    (0x3FB8, 0x3FB9, "特效字母"),
    // word_20B80：只有 sub_17CB6 寫，沒有指令讀。
    (0x3FC0, 0x3FC1, "特效腳本指標備份（只寫不讀）"),
    // 敵彈領域重寫（規格 004）：工作變數與呼叫參數改成區域變數、函式參數，與 `rs2_core::DS_EXEMPT` 的同名六項相同。
    // word_1F7C2 起 8 個字組：sub_1744A 一族（8 張轉向表）寫，只有 sub_175EA 讀。
    (0x2C02, 0x2C11, "敵彈轉向表"),
    // byte_1F7D2：sub_17631 寫，只有 sub_1779B 讀。
    (0x2C12, 0x2C12, "敵彈移動碼"),
    // word_1F7D3、word_1F7D5：sub_172DE 寫，只有 sub_16F9F 讀。
    (0x2C13, 0x2C16, "敵彈瞄準目標"),
    // byte_1F7DB：sub_17361 寫，只有 sub_17855 讀。
    (0x2C1B, 0x2C1B, "敵彈槽號"),
    // word_1FE6D、1FE6F：呼叫端寫，只有 sub_16D3B 一族讀寫；word_1FE71：sub_17631 一族寫，只有它們與 sub_17361 讀。
    (0x32AD, 0x32B2, "敵彈開火起點與釋放旗標"),
    // word_200F3 與工作區 3535h 起 15 個字組：sub_1739B 寫滿後才由 sub_173C5 一族讀；word_200F3 另是 sub_16D3B 一族寫、sub_16CEA 讀的樣板位址。
    (0x3533, 0x3552, "敵彈樣板位址與工作區"),
];

fn ds_exempt(i: usize) -> bool {
    let i = i as u16;
    A0_EXEMPT.iter().any(|&(lo, hi)| lo <= i && i <= hi) || DS_EXTRA_EXEMPT.iter().any(|&(lo, hi, _)| lo <= i && i <= hi)
}

/// 兩份資料段的差異（豁免範圍除外），相鄰的位元組合成一段，最多列 12 段。
/// 環境變數 `RS2_DS_REPORT` 有值時改為印出全部差異（含豁免），回傳空，供探索用。
fn ds_diff(got: &[u8], want: &[u8]) -> Vec<String> {
    let report = std::env::var_os("RS2_DS_REPORT").is_some();
    let mut out = Vec::new();
    let mut i = 0;
    while i < got.len() {
        if got[i] == want[i] || (!report && ds_exempt(i)) {
            i += 1;
            continue;
        }
        let lo = i;
        let mut hi = i;
        while hi + 1 < got.len() && (0..=2).any(|k| hi + 1 + k < got.len() && got[hi + 1 + k] != want[hi + 1 + k]) {
            hi += 1;
        }
        let show = |b: &[u8]| b[lo..=hi.min(lo + 7)].iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ");
        let ex = if (lo..=hi).all(ds_exempt) { "（豁免）" } else { "" };
        out.push(format!("  dseg:{lo:04X}..{hi:04X} ({}B){ex} 重製 {} 原版 {}", hi - lo + 1, show(got), show(want)));
        i = hi + 1;
    }
    if report {
        for l in &out {
            println!("DSDIFF{l}");
        }
        return Vec::new();
    }
    out.truncate(12);
    out
}

/// 收據目錄裡的 `ds-<名>-<n>.ds`（rs2dyn 的 dsdump）：筆數 → 檔案。
fn ds_files(dir: &Path, stem: &str) -> BTreeMap<usize, PathBuf> {
    let prefix = format!("ds-{stem}-");
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(n) = name.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".ds")).and_then(|r| r.parse::<usize>().ok()) {
            out.insert(n, e.path());
        }
    }
    out
}

/// 重播一份收據。回傳第一個差異的說明，或統計。
fn replay_one(inputs: &Inputs, trace: &Path, pokes: &[Poke], end: &End, dumps: &BTreeMap<usize, PathBuf>) -> Result<Stats, String> {
    let text = std::fs::read_to_string(trace).map_err(|e| format!("讀不到 {}：{e}", trace.display()))?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Err("收據是空的".into());
    }
    if lines.len() != end.rows {
        return Err(format!("收據有 {} 筆，但 .end 記 rows={}（收據被截斷或 settle 之後又有取樣）", lines.len(), end.rows));
    }
    let mut g = Game::new(&inputs.image, &inputs.dm1, Start::NewGame { difficulty: None }, Fixes::NONE);
    let mut st = Stats { pokes_total: pokes.len(), ..Stats::default() };
    let mut next_poke = 0;
    let mut last_exit = None;
    for (n, want) in lines.iter().enumerate() {
        while next_poke < pokes.len() && pokes[next_poke].row == n {
            let p = &pokes[next_poke];
            if p.width == 1 {
                g.poke8(a(p.addr), p.val as u8);
            } else {
                g.poke16(a(p.addr), p.val);
            }
            st.pokes_applied += 1;
            next_poke += 1;
        }
        let sc = parse_sc(want);
        // 按著 Esc 走到一輪結尾就進暫停，這一筆的掃描碼是暫停之後才送的（腳本用 untilpc 10ED8h 等它）。
        if g.ds.r8(0xA8CC) == 1 && g.at_round_end() {
            let f = g.step(&Input { scancodes: &[] }, &mut |_g: &Game, _pt: u32| {});
            if !f.paused {
                return Err(format!("第 {n} 筆前應該進暫停"));
            }
        }
        let mut got: Option<String> = None;
        let mut snap: Option<Vec<u8>> = None;
        let want_snap = dumps.contains_key(&n);
        let f = g.step(&Input { scancodes: &sc }, &mut |g: &Game, pt: u32| {
            got = Some(row_json(g, n as u64, pt, &sc));
            if want_snap {
                snap = Some(g.ds.bytes().to_vec());
            }
        });
        let Some(got) = got else {
            return Err(format!(
                "第 {n} 步重製沒有取樣（{}），但收據還有 {} 筆",
                exit_name(f.exit),
                lines.len() - n
            ));
        };
        let want = strip_emu(want);
        if got != want {
            return Err(format!("第 {n} 筆不同（原版幀計數見欄位 frame）\n{}", describe_diff(&got, want)));
        }
        if let (Some(snap), Some(path)) = (snap, dumps.get(&n)) {
            let want = std::fs::read(path).map_err(|e| format!("讀不到 {}：{e}", path.display()))?;
            if want.len() != snap.len() {
                return Err(format!("{} 長度 {}，應為 {}", path.display(), want.len(), snap.len()));
            }
            let diffs = ds_diff(&snap, &want);
            if !diffs.is_empty() {
                return Err(format!("第 {n} 筆取樣點的資料段與 {} 不同：\n{}", path.display(), diffs.join("\n")));
            }
            st.ds_checked += 1;
        }
        st.rows += 1;
        last_exit = if n + 1 == lines.len() { f.exit.or_else(|| g.settle()) } else { f.exit };
        if f.exit.is_some() && n + 1 < lines.len() {
            return Err(format!("重製在第 {n} 筆之後{}，但收據還有 {} 筆", exit_name(f.exit), lines.len() - n - 1));
        }
    }
    if last_exit != end.exit {
        return Err(format!(
            "收據最後一筆（第 {} 筆）之後，原版：{}；重製：{}",
            lines.len() - 1,
            exit_name(end.exit),
            exit_name(last_exit)
        ));
    }
    if let Some((&k, p)) = dumps.range(lines.len()..).next() {
        return Err(format!("{} 的筆數 {k} 超過收據的 {} 筆", p.display(), lines.len()));
    }
    // 略過的 poke 只能是最後一筆之後才送的。
    if let Some(p) = pokes.get(next_poke..).and_then(|r| r.iter().find(|p| p.row < lines.len())) {
        return Err(format!("poke 第 {} 筆沒有套用", p.row));
    }
    Ok(st)
}

/// 掃 `RS2_DOSRUN/fuzz_*`。`RS2_FUZZ` 給子字串時只跑名稱含它的收據。
#[test]
#[ignore]
fn fuzz_replay() {
    let dosrun = env_path("RS2_DOSRUN");
    let only = std::env::var("RS2_FUZZ").unwrap_or_default();
    let inputs = load_inputs(&env_path("RS2_ORIG")).expect("載入原版");
    let pokes_root = pokes_dir();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&dosrun)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("fuzz_") && n.contains(&only)))
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "{} 下沒有 fuzz_*{only}* 收據", dosrun.display());
    let mut fails: Vec<String> = Vec::new();
    let (mut total_rows, mut compared) = (0usize, 0usize);
    for dir in &dirs {
        let name = dir.file_name().unwrap().to_str().unwrap().strip_prefix("fuzz_").unwrap().to_string();
        let mut traces: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                n.starts_with("trace-") && n.ends_with(".jsonl")
            })
            .collect();
        traces.sort();
        if traces.is_empty() {
            fails.push(format!("{name}：目錄裡沒有 trace-*.jsonl"));
            continue;
        }
        for tr in traces {
            let res = load_pokes(&pokes_root.join(format!("{name}.pokes")))
                .and_then(|pk| load_end(&tr.with_extension("end")).map(|e| (pk, e)))
                .and_then(|(pk, e)| {
                    let stem = tr.file_stem().and_then(|s| s.to_str()).unwrap_or("").trim_start_matches("trace-").to_string();
                    replay_one(&inputs, &tr, &pk, &e, &ds_files(dir, &stem)).map(|st| (st, e.exit))
                });
            match res {
                Ok((st, exit)) => {
                    println!(
                        "{name}：比對 {} 筆，poke {}/{} 套用，資料段 {} 份，結束：{}",
                        st.rows,
                        st.pokes_applied,
                        st.pokes_total,
                        st.ds_checked,
                        exit_name(exit)
                    );
                    total_rows += st.rows;
                    compared += 1;
                }
                Err(e) => fails.push(format!("{name}：{e}")),
            }
        }
    }
    println!("fuzz 共 {compared} 份收據通過，{total_rows} 筆；失敗 {} 份", fails.len());
    for f in &fails {
        println!("失敗 {f}");
    }
    assert!(fails.is_empty(), "{} 份收據與重製不同", fails.len());
    assert!(total_rows > 0, "比對筆數為 0");
}
