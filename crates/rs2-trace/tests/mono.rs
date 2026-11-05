//! 規格 006 的單色模式對拍：`MonoSink` 對 C（CGA）、H（MGA）收據的每一筆等待。
//! 需要原版檔與收據，標 `#[ignore]`：`tools/rust.sh test -p rs2-trace --test mono -- --ignored --nocapture`。
//!
//! 環境變數：`RS2_ORIG` 原版目錄。收據根目錄依序找 `RS2_DOSRUN_MODES`、worktree 的 `workspace/dosrun-modes`、`RS2_DOSRUN`。
//! `RS2_MONO=子字串` 只跑名稱含它的收據。`RS2_MONO_LOCATE=1`：不同時讀 `wl_<名稱>_<n>/`（`tools/dos/wm_locate.sh`）定位。
//!
//! 比對筆數為 0 或任何一份收據缺少都算失敗。

use rs2_gfx::MonoAssets;
use rs2_trace::load_inputs;
use rs2_trace::mono::{MonoReplayOpts, MonoStats, load_mono_receipt, replay_mono};
use rs2_trace::wait::Verdict;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// 規格 006 的收據清單：標題、第一到第四關、全流程（四段 J 過場與片尾）、選單（反白），C、H 各一份。
const EXPECTED: [&str; 14] = [
    "wm_flow-c",
    "wm_flow-h",
    "wm_menu-c",
    "wm_menu-h",
    "wm_title-c",
    "wm_title-h",
    "wm_stage1-c",
    "wm_stage1-h",
    "wm_stage2-c",
    "wm_stage2-h",
    "wm_stage3-c",
    "wm_stage3-h",
    "wm_stage4-c",
    "wm_stage4-h",
];

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 跑")))
}

/// 單色收據的根目錄。
fn receipt_root() -> PathBuf {
    if let Some(p) = std::env::var_os("RS2_DOSRUN_MODES") {
        return PathBuf::from(p);
    }
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../workspace/dosrun-modes");
    if local.join(EXPECTED[0]).is_dir() {
        return local;
    }
    env_path("RS2_DOSRUN")
}

fn describe(v: &Verdict) -> String {
    match v {
        Verdict::Pass { rows } => format!("通過，比對 {rows} 筆"),
        Verdict::Unported { func, compared, total } => format!("未移植 {func}，比對 {compared}/{total} 筆"),
        Verdict::Diff { at, compared, total, report } => format!("不同於第 {at} 筆，比對 {compared}/{total} 筆\n{report}"),
    }
}

/// EGA 的屬性表：核心在第一關開場記的 `DrawCmd::EgaAttr`，要等於 `dosgolem` 的 EGA 收據（`mode_e`，
/// `tools/dos/scenarios/mode-e.txt` 錄的）在第一關幀 5 讀到的屬性控制器暫存器 17 個值（`ac` 欄位）。
/// 收據的屬性值是原版 EGA 分支（`sub_110E3` 的 `1002h`）自己寫的，不是重製的推論。
/// 不比 dosgolem 的 DAC 顏色：dosgolem 在 VGA 的 EGA 模式用的預設 DAC 不是 EGA 的 rgbRGB 色表，
/// 色表用 `ega_rgb16` 的定義（6 位元 rgbRGB 每通道 `2 × 主 + 副` 個 55h），見 `rs2-gfx` 的單元測試。
#[test]
#[ignore]
fn ega_attribute_table_matches_the_original() {
    use rs2_core::{DrawCmd, Fixes, Game, Input, Start};
    let text = {
        let p = env_path("RS2_DOSRUN").join("mode_e/e_stage1_start.json");
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("讀不到 {}：{e}", p.display()))
    };
    let list = &text[text.find("\"ac\"").expect("沒有 ac 欄位")..];
    let ac: Vec<u8> = list[list.find('[').unwrap() + 1..list.find(']').unwrap()].split(',').map(|t| t.trim().parse().unwrap()).collect();
    assert_eq!(ac.len(), 17);
    let inputs = load_inputs(&env_path("RS2_ORIG")).expect("載入原版");
    let mut g = Game::new(&inputs.image, &inputs.dm1, Start::NewGame { difficulty: None }, Fixes::NONE);
    let mut regs: Vec<[u8; 17]> = Vec::new();
    let mut collect = |draws: &[DrawCmd]| {
        for d in draws {
            if let DrawCmd::EgaAttr { regs: r } = d {
                regs.push(*r);
            }
        }
    };
    collect(&g.take_draws());
    for _ in 0..40 {
        let f = g.step(&Input { scancodes: &[] }, &mut |_g: &Game, _pt: u32| {});
        collect(&f.draws);
    }
    assert!(regs.len() >= 2, "第一關開場至少記 loader 與 stage1 兩次屬性表，實際 {}", regs.len());
    // loader（`56Eh`）是 16 色的恆等表，之後是 stage1（`4ECh`），邊框色是 0。
    assert_eq!(regs[0][..16], (0u8..16).collect::<Vec<_>>()[..], "loader 的屬性表");
    assert_eq!(regs.last().unwrap()[..], ac[..], "stage1 的屬性表：核心 EgaAttr 對 dosgolem 收據的 ac");
}

#[test]
#[ignore]
fn mono_receipts_exist() {
    let root = receipt_root();
    for n in EXPECTED {
        assert!(root.join(n).join("entry.mono").is_file(), "缺收據 {}", root.join(n).display());
    }
}

/// 每一筆等待：資料段、顯示記憶體、兩塊暫存區、H 的顯示頁都和原版相同。
#[test]
#[ignore]
fn mono_replay() {
    let only = std::env::var("RS2_MONO").unwrap_or_default();
    let root = receipt_root();
    let stats = Rc::new(RefCell::new(MonoStats::default()));
    let opts = MonoReplayOpts {
        locate_root: std::env::var_os("RS2_MONO_LOCATE").map(|_| root.clone()),
        stats: Some(Rc::clone(&stats)),
    };
    let orig = env_path("RS2_ORIG");
    let inputs = load_inputs(&orig).expect("載入原版");
    let assets = Arc::new(MonoAssets::load(&orig).expect("載入單色圖庫"));
    let (mut compared, mut total, mut files) = (0usize, 0usize, 0usize);
    let mut fails: Vec<String> = Vec::new();
    for n in EXPECTED.iter().filter(|n| n.contains(only.as_str())) {
        let dir = root.join(n);
        let rc = match load_mono_receipt(&dir) {
            Ok(rc) => rc,
            Err(e) => {
                fails.push(format!("{n}：收據載入失敗：{e}"));
                println!("{n}：收據載入失敗：{e}");
                continue;
            }
        };
        let before = stats.borrow().clone();
        let v = replay_mono(&inputs, &assets, &rc, &opts);
        println!("{n}：{}", describe(&v));
        let after = stats.borrow().clone();
        if after.hgc_rows > before.hgc_rows {
            let (copy, rows) = (after.hgc_bank1_is_copy - before.hgc_bank1_is_copy, after.hgc_rows - before.hgc_rows);
            println!("  Hercules 2000h 組是複本：{copy}/{rows}");
            let diff_rows: Vec<String> = (0..100)
                .filter_map(|r| {
                    let (a, b) = (after.hgc_row_diff.get(r).copied().unwrap_or(0), before.hgc_row_diff.get(r).copied().unwrap_or(0));
                    (a > b).then(|| format!("{}:{}", 2 * r, a - b))
                })
                .collect();
            if !diff_rows.is_empty() {
                println!("  2000h 組與 0000h 組不同的遊戲列（列號:筆數）：{}", diff_rows.join(" "));
            }
            // 第二至四關的幀函式把 `sub_1117F` 放在最後一個繪製之後，所以每一筆都必須是複本。
            // 第一關的 `sub_10F41` 在 10F72h 複製，之後 10F8Dh 的 `sub_18C0A` 還會畫，所以只有一部分是複本（標題的示範錄影同）。
            if ["stage2", "stage3", "stage4"].iter().any(|s| n.contains(s)) {
                assert_eq!(copy, rows, "{n}：顯示中的頁，2000h 組應是 0000h 組的複本");
            }
        }
        compared += v.compared().max(if matches!(v, Verdict::Pass { .. }) { rc.rows.len() } else { 0 });
        total += rc.rows.len();
        files += 1;
        if !matches!(v, Verdict::Pass { .. }) {
            fails.push(format!("{n}：{}", describe(&v)));
        }
    }
    println!("單色對拍：{files} 份收據，比對 {compared}/{total} 筆，失敗 {} 份", fails.len());
    let st = stats.borrow().clone();
    println!("Hercules 的 2000h 組是否為 0000h 組的複本（顯示中的頁，比對通過的筆數）：{}/{}", st.hgc_bank1_is_copy, st.hgc_rows);
    // 收據涵蓋的繪製命令（C、H 各重播一次，所以每個計次是兩份收據集合的總和）。
    let covered: Vec<String> = st.ops.iter().map(|(k, v)| format!("{k}={v}")).collect();
    println!("涵蓋的繪製命令 {} 種：{}", covered.len(), covered.join(" "));
    if only.is_empty() {
        // 這些命令在單色路徑各有自己的分支（`docs/re/14-display-modes.md` 第 4 節）。沒有涵蓋的在 `docs/spec/006-display-modes.md` 的「未涵蓋」列出。
        for must in ["S18C0A", "S18C6E", "S18CE2", "S18D46", "S18F1A", "S18F5D", "S19068", "S19ADC", "S19B40", "S19BA4", "S19D08", "S19CF9", "S1A287", "S1929F", "S1890D", "S18ACE", "S17FA0", "S18031", "S180C6", "S1117F", "S11504", "S1152D"] {
            assert!(st.ops.contains_key(must), "收據沒有涵蓋 {must}");
        }
    }
    assert!(files > 0 && compared > 0, "沒有比對任何一筆（RS2_MONO={only:?}，收據根 {}）", root.display());
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}
