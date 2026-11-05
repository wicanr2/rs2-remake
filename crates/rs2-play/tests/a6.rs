//! 規格 001 驗收 A6：前端開關不影響模擬。
//!
//! 每次都執行真正的 `rs2-play` 程式（`--headless-steps`），它走和視窗模式相同的 `Session::step` 與 `Session::render`，
//! 每步都呼叫 render（alpha 0.5 與 1.0）並經 SDL 貼圖與 `present`（`SDL_VIDEODRIVER=dummy`、`SDL_RENDER_DRIVER=software`），
//! 再取每步 `Dseg` 的 SHA-256 寫進 `--hash-out`。測試比對這些檔。
//!
//! 需要原版檔與收據，標 `#[ignore]`，由 `tools/rust.sh play-test` 帶 `--include-ignored` 跑。比對筆數為 0 視為失敗。
//!
//! | 項目 | 內容 |
//! |---|---|
//! | 四種組合 | 補間開關 × 幀計數開關，`boss1_kill` 1192 步與 `Start::Demo` 640 步，每步雜湊完全相同 |
//! | 中途切換 | 跑到一半用 `--toggle-at` 切換補間、幀計數，雜湊與不切換相同 |
//! | 正對照 | `--original-bugs`：每步整段 `Dseg` 的雜湊與 `rs2-core` 以 `Fixes::NONE` 重播同一份收據的結果相同 |
//! | 負對照 | 改收據一個掃描碼，雜湊必須不同，且不同的第一步不早於被改的那一步 |
//! | HD | 有 `RS2_HD` 才跑：HD 圖開關與中途切換，雜湊相同 |

use rs2_core::{Entry, Fixes, Game, Input, Start, boot_ds_with_dm1};
use rs2_formats::sha256_hex;
use rs2_gfx::Assets;
use rs2_trace::wait::{ReplayEvent, ReplayOpts, Verdict, load_receipt, replay_observed};
use rs2_trace::{load_inputs, parse_sc};
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rs2-play");
const BOSS_STEPS: usize = 1192;
const DEMO_STEPS: usize = 640;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh play-test 跑")))
}

fn out_dir() -> PathBuf {
    let d = env_path("RS2_OUT").join("play-a6");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn boss_receipt() -> PathBuf {
    env_path("RS2_DOSRUN").join("boss1_kill/trace-s1.jsonl")
}

struct Run {
    hashes: Vec<String>,
    stderr: String,
}

/// 執行 `rs2-play --orig ... <args> --hash-out <tag>.hash`，讀回每步雜湊。
fn run(tag: &str, args: &[&str]) -> Run {
    let out = out_dir();
    let hash = out.join(format!("{tag}.hash"));
    let _ = std::fs::remove_file(&hash);
    let o = Command::new(BIN)
        .arg("--orig")
        .arg(env_path("RS2_ORIG"))
        .args(args)
        .arg("--hash-out")
        .arg(&hash)
        .arg("--config")
        .arg(out.join("cfg"))
        .env("SDL_VIDEODRIVER", "dummy")
        .env("SDL_RENDER_DRIVER", "software")
        .output()
        .expect("啟動 rs2-play 失敗");
    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
    assert!(o.status.success(), "{tag} 失敗（{:?}）：\n{stderr}", o.status.code());
    let text = std::fs::read_to_string(&hash).unwrap_or_else(|e| panic!("{tag} 沒有產生 {}：{e}\n{stderr}", hash.display()));
    let hashes: Vec<String> = text.lines().map(String::from).collect();
    assert!(!hashes.is_empty(), "{tag} 雜湊檔是空的：\n{stderr}");
    assert!(hashes.iter().all(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())), "{tag} 有格式不對的雜湊");
    Run { hashes, stderr }
}

/// 這次執行確實走了 SDL 貼圖與 present，而且每步 render 兩次。
fn assert_full_path(tag: &str, r: &Run, steps: usize) {
    assert!(r.stderr.contains("presenter: sdl"), "{tag} 沒有走 SDL presenter（SDL_VIDEODRIVER=dummy 不可用？）：\n{}", r.stderr);
    let want = format!("render {} 次", steps * 2);
    assert!(r.stderr.contains(&want), "{tag} 預期每步 render 兩次（{want}）：\n{}", r.stderr);
}

fn on(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

/// `args` 之外的四種補間與幀計數組合，每步雜湊都和第一組相同。回傳比對的雜湊筆數。
fn four_combos(name: &str, base: &[&str], steps: usize) -> usize {
    let mut reference: Option<Vec<String>> = None;
    let mut compared = 0;
    for tween in [true, false] {
        for counter in [false, true] {
            let tag = format!("{name}-tween-{}-counter-{}", on(tween), on(counter));
            let mut args: Vec<&str> = base.to_vec();
            args.extend(["--tween", on(tween), "--counter", on(counter), "--headless-steps"]);
            let n = steps.to_string();
            args.push(&n);
            let r = run(&tag, &args);
            assert_eq!(r.hashes.len(), steps, "{tag} 的步數");
            assert_full_path(&tag, &r, steps);
            match &reference {
                None => reference = Some(r.hashes),
                Some(first) => {
                    for (i, (a, b)) in first.iter().zip(&r.hashes).enumerate() {
                        assert_eq!(a, b, "{tag} 第 {} 步的 Dseg 雜湊與第一組不同", i + 1);
                    }
                    compared += r.hashes.len();
                }
            }
        }
    }
    assert!(compared > 0, "比對筆數為 0");
    println!("A6 {name}：四種組合各 {steps} 步，與第一組比對 {compared} 筆，全部相同");
    compared
}

/// 以 `rs2-core` 直接重播收據，每步整段 `Dseg` 的雜湊。
fn core_replay_hashes(start: Start, fixes: Fixes, receipt: &Path, steps: usize) -> Vec<String> {
    let inputs = load_inputs(&env_path("RS2_ORIG")).unwrap();
    let mut g = Game::new(&inputs.image, &inputs.dm1, start, fixes);
    let text = std::fs::read_to_string(receipt).unwrap();
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()).take(steps) {
        let sc = parse_sc(line);
        g.step(&Input { scancodes: &sc }, &mut |_g: &Game, _pt: u32| {});
        out.push(sha256_hex(g.state_hash_input()));
    }
    out
}

#[test]
#[ignore]
fn a6_boss1_kill_four_combos() {
    let receipt = boss_receipt();
    let rp = receipt.to_str().unwrap();
    let n = four_combos("boss", &["--start", "newgame", "--replay", rp], BOSS_STEPS);
    assert_eq!(n, 3 * BOSS_STEPS);
}

#[test]
#[ignore]
fn a6_demo_four_combos() {
    let n = four_combos("demo", &["--start", "demo"], DEMO_STEPS);
    assert_eq!(n, 3 * DEMO_STEPS);
}

/// 跑到一半切換補間與幀計數，雜湊與不切換相同。
#[test]
#[ignore]
fn a6_midrun_toggles() {
    let receipt = boss_receipt();
    let rp = receipt.to_str().unwrap();
    let steps = BOSS_STEPS.to_string();
    let base = ["--start", "newgame", "--replay", rp, "--tween", "on", "--counter", "off", "--headless-steps", steps.as_str()];
    let plain = run("toggle-base", &base);
    let mut args = base.to_vec();
    args.extend(["--toggle-at", "100:tween", "--toggle-at", "300:counter", "--toggle-at", "301:tween", "--toggle-at", "900:counter", "--toggle-at", "1100:tween"]);
    let toggled = run("toggle-mid", &args);
    assert_full_path("toggle-mid", &toggled, BOSS_STEPS);
    assert_eq!(plain.hashes.len(), BOSS_STEPS);
    assert_eq!(plain.hashes, toggled.hashes, "中途切換改變了 Dseg 雜湊");
    println!("A6 中途切換：{} 筆相同", plain.hashes.len());
}

/// 正對照：`--original-bugs` 的每步整段 Dseg 雜湊等於 `rs2-core` 以 `Fixes::NONE` 重播的結果；
/// 預設（`Fixes::ALL`）也相同（規格 A2b：這兩段收據碰不到缺陷條件）。
#[test]
#[ignore]
fn a6_positive_control_matches_core_replay() {
    let receipt = boss_receipt();
    let rp = receipt.to_str().unwrap();
    let steps = BOSS_STEPS.to_string();
    let want = core_replay_hashes(Start::NewGame { difficulty: None }, Fixes::NONE, &receipt, BOSS_STEPS);
    assert_eq!(want.len(), BOSS_STEPS);
    let bugs = run("pos-original-bugs", &["--start", "newgame", "--replay", rp, "--original-bugs", "--headless-steps", &steps]);
    assert_eq!(bugs.hashes.len(), want.len());
    let mut compared = 0;
    for (i, (a, b)) in bugs.hashes.iter().zip(&want).enumerate() {
        assert_eq!(a, b, "第 {} 步：--original-bugs 與 rs2-core 重播的 Dseg 雜湊不同", i + 1);
        compared += 1;
    }
    assert!(compared > 0);
    let fixed = run("pos-fixes-all", &["--start", "newgame", "--replay", rp, "--headless-steps", &steps]);
    assert_eq!(fixed.hashes, want, "Fixes::ALL 在這段收據上應與原版行為相同");
    println!("A6 正對照：每步整段 Dseg 雜湊 {compared} 筆與 rs2-core（Fixes::NONE）相同，Fixes::ALL 同");
}

/// 負對照：改收據一個掃描碼，雜湊要不同。
#[test]
#[ignore]
fn a6_negative_control_changed_scancode() {
    let text = std::fs::read_to_string(boss_receipt()).unwrap();
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    // 挑第一個有掃描碼的行，把第一個碼加 1（環繞在 1 到 0x58）。
    let idx = lines.iter().position(|l| !parse_sc(l).is_empty()).expect("收據裡沒有任何掃描碼");
    let sc = parse_sc(&lines[idx]);
    let changed = if sc[0] >= 0x58 { sc[0] - 1 } else { sc[0] + 1 };
    let start = lines[idx].find("\"sc\":[").unwrap() + 6;
    let end = start + lines[idx][start..].find([',', ']']).unwrap();
    lines[idx].replace_range(start..end, &changed.to_string());
    assert_ne!(parse_sc(&lines[idx]), sc);
    let bad = out_dir().join("neg-receipt.jsonl");
    std::fs::write(&bad, lines.join("\n") + "\n").unwrap();

    let steps = BOSS_STEPS.to_string();
    let good_r = boss_receipt();
    let good = run("neg-good", &["--start", "newgame", "--replay", good_r.to_str().unwrap(), "--headless-steps", &steps]);
    let bad_r = run("neg-bad", &["--start", "newgame", "--replay", bad.to_str().unwrap(), "--headless-steps", &steps]);
    assert_eq!(good.hashes.len(), bad_r.hashes.len());
    let diff: Vec<usize> = (0..good.hashes.len()).filter(|&i| good.hashes[i] != bad_r.hashes[i]).collect();
    assert!(!diff.is_empty(), "改了第 {} 步的掃描碼，雜湊卻完全相同：A6 比對沒有鑑別力", idx + 1);
    assert!(diff[0] >= idx, "第 {} 步以前就不同，不合理", diff[0] + 1);
    assert_eq!(diff[0], idx, "第一個不同的步應該就是被改的那一步（int9 在步開頭處理）");
    println!("A6 負對照：改第 {} 步的掃描碼，第 {} 步起雜湊不同（共 {} 步不同）", idx + 1, diff[0] + 1, diff.len());

    // 第二組：第 300 步起拿掉全部掃描碼（玩家不再開火），後面的雜湊要大量不同。
    let mut lines2: Vec<String> = text.lines().map(String::from).collect();
    let mut dropped = 0;
    for l in lines2.iter_mut().skip(300) {
        let s = l.find("\"sc\":[").unwrap() + 6;
        let e = s + l[s..].find(']').unwrap();
        if e > s {
            dropped += 1;
            l.replace_range(s..e, "");
        }
    }
    assert!(dropped > 0, "第 300 步之後的收據沒有任何掃描碼");
    let bad2 = out_dir().join("neg-receipt2.jsonl");
    std::fs::write(&bad2, lines2.join("\n") + "\n").unwrap();
    let bad2_r = run("neg-bad2", &["--start", "newgame", "--replay", bad2.to_str().unwrap(), "--headless-steps", &steps]);
    let diff2: Vec<usize> = (0..good.hashes.len().min(bad2_r.hashes.len())).filter(|&i| good.hashes[i] != bad2_r.hashes[i]).collect();
    assert!(diff2.len() > 100, "拿掉 {dropped} 步的掃描碼後只有 {} 步不同", diff2.len());
    assert!(diff2[0] >= 299, "第 {} 步以前就不同，不合理", diff2[0] + 1);
    println!("A6 負對照 2：第 300 步起拿掉 {dropped} 步的掃描碼，第 {} 步起不同，共 {} 步不同", diff2[0] + 1, diff2.len());
}

/// 有 HD 資料夾才跑：HD 開關與中途切換，雜湊相同。
#[test]
#[ignore]
fn a6_hd_group() {
    let Ok(hd) = std::env::var("RS2_HD") else {
        println!("A6 HD：沒有 RS2_HD，略過");
        return;
    };
    let receipt = boss_receipt();
    let rp = receipt.to_str().unwrap();
    let steps = BOSS_STEPS.to_string();
    let base = run("hd-ref-orig", &["--start", "newgame", "--replay", rp, "--tween", "off", "--counter", "off", "--headless-steps", &steps]);
    assert_eq!(base.hashes.len(), BOSS_STEPS);
    let mut compared = 0;
    // HD 圖加補間與幀計數的組合。
    for (tween, counter) in [(true, false), (false, false), (true, true)] {
        let tag = format!("hd-tween-{}-counter-{}", on(tween), on(counter));
        let r = run(&tag, &["--start", "newgame", "--replay", rp, "--hd", &hd, "--tween", on(tween), "--counter", on(counter), "--headless-steps", &steps]);
        assert!(r.stderr.contains("HD true"), "{tag} 沒有啟用 HD：\n{}", r.stderr);
        assert_full_path(&tag, &r, BOSS_STEPS);
        assert_eq!(r.hashes, base.hashes, "{tag} 的 Dseg 雜湊與原版圖不同");
        compared += r.hashes.len();
    }
    // 中途切換 HD、補間、幀計數。
    let r = run(
        "hd-toggles",
        &[
            "--start", "newgame", "--replay", rp, "--hd", &hd, "--headless-steps", &steps, "--toggle-at", "50:hd", "--toggle-at", "200:tween", "--toggle-at", "400:hd",
            "--toggle-at", "700:counter", "--toggle-at", "1000:hd",
        ],
    );
    assert_eq!(r.hashes, base.hashes, "HD 中途切換改變了 Dseg 雜湊");
    compared += r.hashes.len();
    assert!(compared > 0);
    println!("A6 HD：{compared} 筆雜湊與原版圖相同");
}

/// 規格 005 E7：HD 介面（HUD、選單高亮、打擊回饋）的開關都不改每步雜湊。
/// 有 HD 資料夾才跑。打擊回饋預設開，HUD 與回饋在 HD 輸出時自動生效；原版圖當基準。
#[test]
#[ignore]
fn a6_hd_ui_hud_and_hit_feedback_do_not_change_the_hash() {
    let Ok(hd) = std::env::var("RS2_HD") else {
        println!("A6 HD 介面：沒有 RS2_HD，略過");
        return;
    };
    let receipt = boss_receipt();
    let rp = receipt.to_str().unwrap();
    let steps = BOSS_STEPS.to_string();
    let base = run("hdui-ref-orig", &["--start", "newgame", "--replay", rp, "--tween", "off", "--counter", "off", "--headless-steps", &steps]);
    assert_eq!(base.hashes.len(), BOSS_STEPS);
    let mut compared = 0;
    for (feedback, counter) in [(true, false), (false, false), (true, true)] {
        let tag = format!("hdui-feedback-{}-counter-{}", on(feedback), on(counter));
        let r = run(&tag, &["--start", "newgame", "--replay", rp, "--hd", &hd, "--feedback", on(feedback), "--counter", on(counter), "--headless-steps", &steps]);
        assert!(r.stderr.contains("HD true"), "{tag} 沒有啟用 HD：\n{}", r.stderr);
        assert_full_path(&tag, &r, BOSS_STEPS);
        assert_eq!(r.hashes, base.hashes, "{tag} 的 Dseg 雜湊與原版圖不同");
        compared += r.hashes.len();
    }
    // 中途切換打擊回饋與 HD。
    let r = run(
        "hdui-toggles",
        &["--start", "newgame", "--replay", rp, "--hd", &hd, "--headless-steps", &steps, "--toggle-at", "60:feedback", "--toggle-at", "300:hd", "--toggle-at", "500:feedback", "--toggle-at", "900:hd", "--toggle-at", "1000:feedback"],
    );
    assert_eq!(r.hashes, base.hashes, "打擊回饋與 HD 中途切換改變了 Dseg 雜湊");
    compared += r.hashes.len();
    // 標題選單高亮：從標題起跑（開場、示範、選單）。
    let t = TITLE_STEPS.to_string();
    let ref_title = run("hdui-title-ref", &["--tween", "off", "--headless-steps", &t]);
    let hd_title = run("hdui-title-hd", &["--hd", &hd, "--headless-steps", &t]);
    assert_eq!(hd_title.hashes, ref_title.hashes, "HD 的標題與選單改變了 Dseg 雜湊");
    compared += hd_title.hashes.len();
    assert!(compared > 0, "比對筆數為 0");
    println!("A6 HD 介面：{compared} 筆雜湊與原版圖相同（HUD、打擊回饋開關、中途切換、標題選單）");
}

const TITLE_STEPS: usize = 1500;

/// 規格 002 B7：用全流程收據的按鍵，`rs2-play` 從標題走完四關到片尾。
///
/// 每步的掃描碼來自 `replay_observed`（它同時確認這份輸入和原版逐筆相同），每步 `Dseg` 雜湊也來自那裡，
/// 所以這裡比到的是「前端走的路徑等於通過全流程對拍的路徑」。開補間、HD 全開時雜湊也要一樣，
/// 修正全開時不比雜湊，但要走完不出錯。
#[test]
#[ignore]
fn b7_play_from_title_to_ending() {
    let dir = env_path("RS2_ORIG");
    let inputs = load_inputs(&dir).unwrap();
    let assets = Assets::load(&dir).unwrap();
    let rc = load_receipt(&env_path("RS2_DOSRUN").join("wt_flow")).unwrap();
    let mut lines = String::new();
    let mut want: Vec<String> = Vec::new();
    let mut frames = 0u32;
    let opts = ReplayOpts { ds_report: false, locate_root: None, boot: true };
    let v = replay_observed(&inputs, &assets, &rc, &opts, &mut |ev| {
        if let ReplayEvent::StepAt { sc, ds, .. } = ev {
            let codes: Vec<String> = sc.iter().map(|c| c.to_string()).collect();
            lines.push_str(&format!("{{\"sc\":[{}]}}\n", codes.join(",")));
            want.push(sha256_hex(ds));
            frames = u16::from_le_bytes([ds[0x422], ds[0x423]]) as u32;
        }
    });
    assert!(matches!(v, Verdict::Pass { .. }), "全流程收據沒有通過");
    assert!(frames > 0, "最後沒有停在片尾");
    let replay = out_dir().join("flow-sc.jsonl");
    std::fs::write(&replay, lines).unwrap();
    let rp = replay.to_str().unwrap();
    let steps = want.len().to_string();

    let r = run("flow-orig", &["--original-bugs", "--replay", rp, "--tween", "off", "--headless-steps", &steps]);
    assert_eq!(r.hashes, want, "前端與 rs2-core 的 Dseg 雜湊不同");
    assert_full_path("flow-orig", &r, want.len());
    assert!(r.stderr.contains("ending_"), "沒有走到片尾的調色盤：\n{}", r.stderr);

    let r = run("flow-tween", &["--original-bugs", "--replay", rp, "--tween", "on", "--counter", "on", "--headless-steps", &steps]);
    assert_eq!(r.hashes, want, "補間與幀計數改變了 Dseg 雜湊");

    if let Ok(hd) = std::env::var("RS2_HD") {
        let r = run("flow-hd", &["--original-bugs", "--replay", rp, "--hd", &hd, "--headless-steps", &steps]);
        assert_eq!(r.hashes, want, "HD 改變了 Dseg 雜湊");
        assert!(r.stderr.contains("HD true"), "沒有啟用 HD：\n{}", r.stderr);
        assert!(!r.stderr.contains("缺圖："), "HD 有缺圖：\n{}", r.stderr);
    }

    let wav = out_dir().join("flow.wav");
    let r = run("flow-fixed", &["--replay", rp, "--headless-steps", &steps, "--wav-out", wav.to_str().unwrap()]);
    assert_eq!(r.hashes.len(), want.len());
    println!("B7 全流程：{} 步，前端雜湊與對拍路徑相同，修正全開也走完", want.len());
}

/// 規格 002 B7：正式入口。不按鍵從標題跑開場與示範，每步整段 `Dseg` 雜湊和 `rs2-core` 相同。
#[test]
#[ignore]
fn b7_title_matches_core() {
    let steps = TITLE_STEPS.to_string();
    let r = run("title-core", &["--original-bugs", "--headless-steps", &steps]);
    assert_eq!(r.hashes.len(), TITLE_STEPS);
    let inputs = load_inputs(&env_path("RS2_ORIG")).unwrap();
    let boot = boot_ds_with_dm1(&inputs.image, &inputs.dm1);
    let mut g = Game::from_snapshot(inputs.game_files(), boot.bytes(), Entry::Title, Fixes::NONE);
    for (i, want) in r.hashes.iter().enumerate() {
        g.step(&Input { scancodes: &[] }, &mut |_g: &Game, _pt: u32| {});
        assert_eq!(&sha256_hex(g.state_hash_input()), want, "第 {} 步", i + 1);
    }
    println!("B7 標題：{} 筆與 rs2-core 相同", r.hashes.len());
}

/// 規格 002 B7：前端開關不影響模擬，標題入口也一樣。
#[test]
#[ignore]
fn b7_title_four_combos() {
    let n = four_combos("title", &["--start", "title"], 800);
    assert_eq!(n, 3 * 800);
}

/// 規格 003 C5：音訊不影響模擬，而且真的有聲音。
#[test]
#[ignore]
fn c5_audio_does_not_touch_the_simulation() {
    let steps = 800.to_string();
    let wav = out_dir().join("title.wav");
    let _ = std::fs::remove_file(&wav);
    let a = run("title-noaudio", &["--no-audio", "--headless-steps", &steps]);
    let b = run("title-wav", &["--headless-steps", &steps, "--wav-out", wav.to_str().unwrap()]);
    assert_eq!(a.hashes, b.hashes);
    let d = std::fs::read(&wav).unwrap();
    assert_eq!(&d[..4], b"RIFF");
    let pcm: Vec<i16> = d[44..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
    assert!(pcm.len() > 49716 * 30, "只有 {} 取樣", pcm.len());
    let loud = pcm.iter().filter(|s| s.abs() > 500).count();
    assert!(loud > pcm.len() / 4, "{loud} / {}", pcm.len());
    println!("C5：{} 筆雜湊相同，音訊 {} 取樣，{loud} 個超過門檻", a.hashes.len(), pcm.len());
}

/// 工具本身：`--hash-out` 的行數等於步數，沒有 `--headless-steps` 以外的副作用。
#[test]
#[ignore]
fn a6_hash_file_has_one_line_per_step() {
    let r = run("lines", &["--start", "demo", "--headless-steps", "7"]);
    assert_eq!(r.hashes.len(), 7);
    // 每一步 Dseg 都在變（示範每幀至少推進幀計數）。
    let uniq: std::collections::HashSet<&String> = r.hashes.iter().collect();
    assert_eq!(uniq.len(), 7);
}
