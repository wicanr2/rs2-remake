//! 規格 001 的驗收。需要原版檔與收據，由 `tools/rust.sh verify` 帶 `--include-ignored` 跑。
//! 比對筆數為 0 視為失敗，免得缺檔時看起來像通過。

use rs2_game::{Fixes, Game, Input, Start};
use rs2_formats::{EXPECTED, parse_go, sha256_hex};
use rs2_trace::mirror::A0_EXEMPT;
use rs2_trace::{describe_diff, load_inputs, parse_sc, row_json, strip_emu};
use std::path::PathBuf;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh verify 跑")))
}

/// A1：每個圖庫項目的解碼結果與 Python 工具各自算的雜湊相同。
#[test]
#[ignore]
fn a1_formats() {
    let tsv = std::fs::read_to_string(env_path("RS2_OUT").join("pixel-hashes.tsv")).expect("pixel-hashes.tsv");
    let dir = env_path("RS2_ORIG");
    let mut checked = 0;
    for name in ["MY.GO", "SHT1.GO", "BH1.GO"] {
        let go = parse_go(&std::fs::read(dir.join(name)).unwrap()).unwrap();
        let mut secs: Vec<&Vec<rs2_formats::BankItem>> = Vec::new();
        if let Some(m) = &go.mask {
            secs.push(m);
        }
        secs.extend(go.planes.iter());
        for line in tsv.lines().filter(|l| l.starts_with(&format!("{name}\t"))) {
            let f: Vec<&str> = line.split('\t').collect();
            let (sec, idx): (usize, usize) = (f[1].parse().unwrap(), f[2].parse().unwrap());
            let it = &secs[sec][idx];
            assert_eq!(it.stride.to_string(), f[3], "{line}");
            assert_eq!(it.height.to_string(), f[4], "{line}");
            let mut buf = Vec::new();
            buf.extend_from_slice(&it.stride.to_le_bytes());
            buf.extend_from_slice(&it.height.to_le_bytes());
            buf.extend_from_slice(&it.data);
            assert_eq!(sha256_hex(&buf), f[5], "{line}");
            checked += 1;
        }
    }
    println!("A1 比對 {checked} 項");
    assert!(checked > 0);
}

/// A3：改任一輸入一個位元組就拒絕，並確認原檔是接受的（負對照要有正對照才有意義）。
#[test]
#[ignore]
fn a3_hash_reject() {
    let dir = env_path("RS2_ORIG");
    load_inputs(&dir).expect("未改動的原版要能載入");
    let tmp = env_path("RS2_OUT").join("a3-tmp");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    // 規格 002「輸入」列的全部檔案（EXPECTED）逐項改一個位元組。
    let names: Vec<&str> = EXPECTED.iter().map(|(n, _)| *n).collect();
    for &n in &names {
        std::fs::copy(dir.join(n), tmp.join(n)).unwrap();
        std::fs::set_permissions(tmp.join(n), std::fs::Permissions::from(std::os::unix::fs::PermissionsExt::from_mode(0o644))).unwrap();
    }
    load_inputs(&tmp).expect("複本要能載入");
    for &n in &names {
        let good = std::fs::read(tmp.join(n)).unwrap();
        let mut bad = good.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0x01;
        std::fs::write(tmp.join(n), &bad).unwrap();
        let e = load_inputs(&tmp).err().unwrap_or_else(|| panic!("改了 {n} 卻沒有拒絕"));
        assert!(e.contains(n), "錯誤訊息要寫出檔名：{e}");
        std::fs::write(tmp.join(n), &good).unwrap();
    }
    std::fs::remove_dir_all(&tmp).unwrap();
}

fn a0_check(start: Start, dump: &str) {
    let dir = env_path("RS2_ORIG");
    let inputs = load_inputs(&dir).unwrap();
    let g = Game::new(&inputs.image, &inputs.dm1, start, Fixes::NONE);
    let want = std::fs::read(env_path("RS2_DOSRUN").join(dump)).unwrap();
    assert_eq!(want.len(), 0x10000);
    let got_ds = inputs.mirror().dump(g.state());
    let got = got_ds.bytes();
    let mut bad = Vec::new();
    for i in 0..0x10000usize {
        if got[i] != want[i] && !A0_EXEMPT.iter().any(|&(lo, hi)| (lo as usize) <= i && i <= hi as usize) {
            bad.push(format!("dseg:{i:04X} 重製 {:02X} 原版 {:02X}", got[i], want[i]));
        }
    }
    assert!(bad.is_empty(), "{dump} 範圍外差異 {} 位元組：{:?}", bad.len(), &bad[..bad.len().min(20)]);
}

#[test]
#[ignore]
fn a0_demo_entry() {
    a0_check(Start::Demo, "a0_demo/demo_entry.ds");
}

#[test]
#[ignore]
fn a0_newgame_entry() {
    a0_check(Start::NewGame { difficulty: None }, "a0_newgame/newgame_entry.ds");
}

fn replay(start: Start, receipt: &str, fixes: Fixes) -> usize {
    let dir = env_path("RS2_ORIG");
    let inputs = load_inputs(&dir).unwrap();
    let m = inputs.mirror();
    let mut g = Game::new(&inputs.image, &inputs.dm1, start, fixes);
    let text = std::fs::read_to_string(env_path("RS2_DOSRUN").join(receipt)).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(!lines.is_empty(), "{receipt} 是空的");
    for (n, want) in lines.iter().enumerate() {
        let sc = parse_sc(want);
        let mut got: Option<String> = None;
        let f = g.step(&Input { scancodes: &sc }, &mut |g: &Game, pt: u32| {
            got = Some(row_json(&m, g, n as u64, pt, &sc));
        });
        let got = got.unwrap_or_else(|| panic!("第 {n} 步沒有取樣，exit={:?}", f.exit));
        let want = strip_emu(want);
        assert!(got == want, "{receipt} 第 {n} 筆不同\n{}", describe_diff(&got, want));
    }
    println!("{receipt} 比對 {} 筆", lines.len());
    lines.len()
}

/// A2：`DM1.LCK` 重播與收據逐幀相同。
#[test]
#[ignore]
fn a2_dm1_replay() {
    assert_eq!(replay(Start::Demo, "trace_dm1/trace-dm1.jsonl", Fixes::NONE), 640);
}

/// A4：真實掃描碼從第一關打到頭目與連爆，最後以 `C` 離開。
#[test]
#[ignore]
fn a4_boss_kill() {
    assert_eq!(replay(Start::NewGame { difficulty: None }, "boss1_kill/trace-s1.jsonl", Fixes::NONE), 1192);
}

/// A2b：`Fixes` 全開跑同兩段收據。收據碰不到缺陷條件，所以要和全關相同。
#[test]
#[ignore]
fn a2b_fixes_on_same_trace() {
    assert_eq!(replay(Start::Demo, "trace_dm1/trace-dm1.jsonl", Fixes::ALL), 640);
    assert_eq!(replay(Start::NewGame { difficulty: None }, "boss1_kill/trace-s1.jsonl", Fixes::ALL), 1192);
}
