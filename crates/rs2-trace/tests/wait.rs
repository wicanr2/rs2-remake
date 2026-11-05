//! 規格 002 的全流程對拍。需要原版檔與收據，標 `#[ignore]`：
//! `tools/rust.sh test -p rs2-trace --test wait -- --ignored --nocapture`
//!
//! 環境變數：`RS2_ORIG` 原版目錄、`RS2_DOSRUN` 收據根目錄（`tools/rust.sh` 會帶）。
//! `RS2_WAIT=子字串` 只跑名稱含它的收據（例如 `RS2_WAIT=stage2`）。
//! `RS2_DS_REPORT=1`：資料段差異改為印出（含豁免範圍），不判失敗，供探索豁免用。
//!
//! 結果分三類：通過（全部筆數相同）、未移植（比到某一筆之後遇到 `unimplemented!`）、不同（有一筆不同或重製 panic）。
//! 有「不同」才失敗；比對筆數合計為 0 也失敗（缺收據或全部尚未移植時不能看起來像通過）。

use rs2_core::{boot_ds_with_dm1, ds_exempt};
use rs2_gfx::Assets;
use rs2_trace::wait::{Receipt, ReplayOpts, Verdict, ds_diff, load_receipt, replay, replay_with};
use rs2_trace::{Inputs, load_inputs};
use std::path::PathBuf;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 跑")))
}

/// `RS2_DOSRUN/wt_*` 收據目錄，依名稱排序。`only` 非空時只留名稱含它的。
fn receipt_dirs(only: &str) -> Vec<PathBuf> {
    let root = env_path("RS2_DOSRUN");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("讀不到 {}：{e}", root.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("wt_") && n.contains(only)))
        .collect();
    dirs.sort();
    dirs
}

fn describe(v: &Verdict) -> String {
    match v {
        Verdict::Pass { rows } => format!("通過，比對 {rows} 筆"),
        Verdict::Unported { func, compared, total } => format!("未移植 {func}，比對 {compared}/{total} 筆"),
        Verdict::Diff { at, compared, total, report } => format!("不同於第 {at} 筆，比對 {compared}/{total} 筆\n{report}"),
    }
}

fn setup() -> (Inputs, Assets) {
    let dir = env_path("RS2_ORIG");
    (load_inputs(&dir).expect("載入原版"), Assets::load(&dir).expect("載入圖庫"))
}

/// 掃全部收據。
#[test]
#[ignore]
fn wait_replay() {
    let only = std::env::var("RS2_WAIT").unwrap_or_default();
    let ds_report = std::env::var_os("RS2_DS_REPORT").is_some();
    // RS2_WAIT_LOCATE=1：畫面不同時讀 tools/dos/wait_locate.sh 產生的 wl_<名稱>_<n>/ 傾印，找出不同的偏移與最後寫入的命令。
    let locate_root = std::env::var_os("RS2_WAIT_LOCATE").map(|_| env_path("RS2_DOSRUN"));
    let opts = ReplayOpts { ds_report, locate_root, boot: false };
    let dirs = receipt_dirs(&only);
    assert!(!dirs.is_empty(), "{} 下沒有 wt_*{only}* 收據", env_path("RS2_DOSRUN").display());
    let (inputs, assets) = setup();
    let (mut pass, mut unported, mut diff) = (0usize, 0usize, 0usize);
    let (mut compared, mut total) = (0usize, 0usize);
    let mut fails: Vec<String> = Vec::new();
    for dir in &dirs {
        let name = dir.file_name().unwrap().to_str().unwrap().to_string();
        let rc = match load_receipt(dir) {
            Ok(rc) => rc,
            Err(e) => {
                diff += 1;
                fails.push(format!("{name}：收據載入失敗：{e}"));
                println!("{name}：收據載入失敗：{e}");
                continue;
            }
        };
        let v = replay_with(&inputs, &assets, &rc, &opts);
        println!("{name}：{}", describe(&v));
        compared += v.compared();
        total += rc.rows.len();
        match &v {
            Verdict::Pass { .. } => pass += 1,
            Verdict::Unported { .. } => unported += 1,
            Verdict::Diff { .. } => {
                diff += 1;
                fails.push(format!("{name}：{}", describe(&v)));
            }
        }
    }
    println!("統計：通過 {pass}、未移植 {unported}、不同 {diff}；比對 {compared}/{total} 筆");
    assert!(fails.is_empty(), "{} 份收據與重製不同", fails.len());
    assert!(compared > 0, "比對筆數為 0（全部收據都在第一筆之前就遇到未移植的函式？）");
}

/// 正式遊戲的路徑：資料段從 `boot_ds_with_dm1` 開始，不用入口傾印，四份從標題開始的收據每筆仍要相同。
/// `DS_EXEMPT` 裡如果有邏輯真的會讀的狀態，這裡會變成不同或 panic。
#[test]
#[ignore]
fn b3_flow_from_boot_state() {
    let (inputs, assets) = setup();
    let opts = ReplayOpts { ds_report: false, locate_root: None, boot: true };
    let mut compared = 0;
    for name in ["wt_title", "wt_menu", "wt_p2", "wt_flow"] {
        let rc = load_receipt(&env_path("RS2_DOSRUN").join(name)).expect("載入收據");
        let v = replay_with(&inputs, &assets, &rc, &opts);
        println!("{name}：{}", describe(&v));
        assert!(matches!(v, Verdict::Pass { .. }), "{name}：{}", describe(&v));
        compared += v.compared();
    }
    assert!(compared > 0);
}

/// 負對照：改一個位元組、丟一筆、改一個雜湊，都要變成「不同」。對象是第一份比得過 3 筆以上的收據；原檔本身要先是「沒有不同」。
#[test]
#[ignore]
fn wait_negative_controls() {
    let only = std::env::var("RS2_WAIT_NEG").unwrap_or_default();
    let (inputs, assets) = setup();
    let mut subject: Option<Receipt> = None;
    for dir in receipt_dirs(&only) {
        let Ok(rc) = load_receipt(&dir) else { continue };
        let v = replay(&inputs, &assets, &rc, false);
        println!("負對照候選 {}：{}", rc.name, describe(&v));
        if !matches!(v, Verdict::Diff { .. }) && v.compared() >= 3 {
            subject = Some(rc);
            break;
        }
    }
    let rc = subject.expect("沒有收據能比對 3 筆以上，負對照沒有對象（先移植到至少一份收據能開始比對）");
    println!("負對照對象：{}（{} 筆）", rc.name, rc.rows.len());

    // 正對照：未改動的收據不是「不同」。
    let base = replay(&inputs, &assets, &rc, false);
    assert!(!matches!(base, Verdict::Diff { .. }), "未改動的收據不該不同：{}", describe(&base));
    let want_at_least = base.compared();

    // 1. 改資料段一個位元組：.dsd 第 0 筆是兩段各 32 KB，資料從位移 8 + 4 開始。挑 dseg:0100h（不在豁免範圍）。
    assert!(!ds_exempt(0x0100));
    let mut m = rc.clone();
    m.dsd[8 + 4 + 0x0100] ^= 0x01;
    let v = replay(&inputs, &assets, &m, false);
    assert!(matches!(v, Verdict::Diff { at: 0, .. }), "改資料段 1 位元組應在第 0 筆不同：{}", describe(&v));

    // 2. 丟一筆：第 1 筆。
    let mut m = rc.clone();
    m.rows.remove(1);
    let v = replay(&inputs, &assets, &m, false);
    assert!(matches!(v, Verdict::Diff { .. }), "丟一筆應不同：{}", describe(&v));

    // 3. 改畫面雜湊一個字元：第 2 筆。
    let mut m = rc.clone();
    let h = &mut m.rows[2].vram;
    let c = if h.starts_with('0') { "1" } else { "0" };
    h.replace_range(0..1, c);
    let v = replay(&inputs, &assets, &m, false);
    assert!(matches!(v, Verdict::Diff { at: 2, .. }), "改 vram 雜湊應在第 2 筆不同：{}", describe(&v));

    // 4. 改 DAC 雜湊。
    let mut m = rc.clone();
    let h = &mut m.rows[1].dac;
    let c = if h.starts_with('0') { "1" } else { "0" };
    h.replace_range(0..1, c);
    let v = replay(&inputs, &assets, &m, false);
    assert!(matches!(v, Verdict::Diff { at: 1, .. }), "改 dac 雜湊應在第 1 筆不同：{}", describe(&v));

    // 5. 改 cx。
    let mut m = rc.clone();
    m.rows[1].cx ^= 1;
    let v = replay(&inputs, &assets, &m, false);
    assert!(matches!(v, Verdict::Diff { at: 1, .. }), "改 cx 應在第 1 筆不同：{}", describe(&v));
    println!("負對照 5 項都變成不同（正對照比對 {want_at_least} 筆）");
}

/// B1：`boot_ds` 與標題入口傾印相同，差異只在 `DS_EXEMPT`。標題入口的收據都從同一個入口開始，逐份比。
#[test]
#[ignore]
fn b1_boot_state() {
    let inputs = load_inputs(&env_path("RS2_ORIG")).expect("載入原版");
    let boot = boot_ds_with_dm1(&inputs.image, &inputs.dm1);
    // 分類用的傾印（不進版控）：RS2_OUT/boot_ds.bin，tools/dos 的分析腳本拿它和 entry.ds、映像三方比。
    if let Some(o) = std::env::var_os("RS2_OUT") {
        let _ = std::fs::write(PathBuf::from(o).join("boot_ds.bin"), boot.bytes());
    }
    let mut checked = 0;
    let mut bad: Vec<String> = Vec::new();
    for name in ["wt_title", "wt_flow", "wt_menu", "wt_p2"] {
        let p: PathBuf = env_path("RS2_DOSRUN").join(name).join("entry.ds");
        let Ok(want) = std::fs::read(&p) else { continue };
        assert_eq!(want.len(), 0x10000, "{name}/entry.ds 長度");
        let report = std::env::var_os("RS2_DS_REPORT").is_some();
        let diffs = ds_diff(boot.bytes(), &want, report);
        if !diffs.is_empty() {
            bad.push(format!("{name}：boot_ds 與入口傾印在豁免範圍外有差異：\n{}", diffs.join("\n")));
        }
        checked += 1;
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert!(checked > 0, "比對筆數為 0：找不到任何標題入口的 entry.ds");
    println!("B1 比對 {checked} 份標題入口傾印");
    // boot_ds 不碰標題寫的四個值（Core::new 為了直接進第一關而設）：維持映像值。
    let img = rs2_core::Dseg::from_image(&inputs.image);
    for o in [0x041Cu16, 0x041D, 0x03EC, 0x03EB] {
        assert_eq!(boot.r8(o), img.r8(o), "dseg:{o:04X} 應維持映像值");
    }
}

