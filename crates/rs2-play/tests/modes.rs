//! 顯示模式（VGA、EGA、CGA、MGA）從命令列進去的端到端測試：執行真正的 `rs2-play`（`--headless-steps`），
//! 走和視窗模式相同的 `Session::step`、`Session::render` 與 SDL 貼圖（`SDL_VIDEODRIVER=dummy`、`SDL_RENDER_DRIVER=software`）。
//!
//! 需要原版檔與收據，標 `#[ignore]`，由 `tools/rust.sh play-test` 帶 `--include-ignored` 跑。比對筆數為 0 視為失敗。
//!
//! | 項目 | 內容 |
//! |---|---|
//! | 截圖 | `--mode X --shot-at`：四種模式的視窗截圖尺寸相同，遊戲畫面是 8:5（VGA、EGA）或 4:3（CGA、MGA）置中，框外全黑 |
//! | 切換 | `--mode`（起始模式）與 `--toggle-at N:mode`（中途切換）都不改每步的 `Dseg` 雜湊 |
//! | 設定 | 設定檔的 `mode` 啟動時還原，`--mode` 覆蓋，壞值退回 VGA 並警告 |
//! | 缺檔 | 單色圖庫缺檔或雜湊不符：CGA、MGA 退回 VGA 並說明，VGA 照常跑 |

use rs2_gfx::png;
use rs2_play::mode::DisplayMode;
use rs2_play::toolbar::desktop_height;
use rs2_play::view::{Fit, fit_rect_for, window_size};
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rs2-play");

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh play-test 跑")))
}

fn out_dir(sub: &str) -> PathBuf {
    let d = env_path("RS2_OUT").join("play-modes").join(sub);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Run {
    ok: bool,
    code: Option<i32>,
    hashes: Vec<String>,
    stderr: String,
}

/// 執行 `rs2-play --orig <orig> <args> --config <dir>/cfg --hash-out <dir>/h.hash`。
fn run_with(orig: &Path, dir: &Path, args: &[&str]) -> Run {
    let hash = dir.join("h.hash");
    let o = Command::new(BIN)
        .arg("--orig")
        .arg(orig)
        .args(args)
        .arg("--hash-out")
        .arg(&hash)
        .arg("--config")
        .arg(dir.join("cfg"))
        .env("SDL_VIDEODRIVER", "dummy")
        .env("SDL_RENDER_DRIVER", "software")
        .output()
        .expect("啟動 rs2-play 失敗");
    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
    let hashes = std::fs::read_to_string(&hash).map(|t| t.lines().map(String::from).collect()).unwrap_or_default();
    Run { ok: o.status.success(), code: o.status.code(), hashes, stderr }
}

fn run(dir: &Path, args: &[&str]) -> Run {
    run_with(&env_path("RS2_ORIG"), dir, args)
}

fn boss_receipt() -> PathBuf {
    env_path("RS2_DOSRUN").join("boss1_kill/trace-s1.jsonl")
}

const STEPS: usize = 1192;

fn fit_of(mode: DisplayMode) -> Fit {
    if mode.is_mono() { Fit::MONO } else { Fit::SD }
}

// ---------------------------------------------------------------------------
// 截圖
// ---------------------------------------------------------------------------

/// 每種模式存一張視窗截圖（含工具列），確認 `--mode` 搭配 `--shot-at` 能出圖，而且畫面的位置與比例正確。
#[test]
#[ignore]
fn shots_in_every_mode_have_the_right_geometry() {
    let dir = out_dir("shots");
    let rp = boss_receipt();
    let (ww, wh) = window_size(3, true);
    let bar = desktop_height(ww);
    let mut images: Vec<(DisplayMode, png::Image)> = Vec::new();
    for mode in DisplayMode::ALL {
        let shot = dir.join(format!("{}.png", mode.code()));
        let spec = format!("100:{}", shot.display());
        let r = run(&dir, &["--start", "newgame", "--replay", rp.to_str().unwrap(), "--mode", mode.code(), "--headless-steps", "120", "--shot-at", &spec]);
        assert!(r.ok, "{mode:?} 失敗（{:?}）：\n{}", r.code, r.stderr);
        assert!(r.stderr.contains(&format!("顯示模式 {}", mode.code())), "{mode:?}：日誌沒有顯示模式：\n{}", r.stderr);
        assert!(r.stderr.contains("[shot] 第 100 步") && !r.stderr.contains("（全黑，失敗）"), "{mode:?}：\n{}", r.stderr);
        let img = png::decode(&std::fs::read(&shot).expect("沒有截圖")).expect("PNG 解不開");
        assert_eq!((img.w as u32, img.h as u32), (ww, wh), "{mode:?}：截圖是視窗大小，不隨模式變");
        // 遊戲畫面的矩形：框內有內容，框外（工具列以下）全黑。
        let r = fit_rect_for(ww, wh, bar, fit_of(mode), false);
        let at = |x: usize, y: usize| &img.rgba[(y * img.w + x) * 4..(y * img.w + x) * 4 + 3];
        let (mut inside_lit, mut outside_lit) = (0usize, 0usize);
        for y in bar as usize..img.h {
            for x in 0..img.w {
                let inside = (x as i32) >= r.x && (x as i32) < r.x + r.w && (y as i32) >= r.y && (y as i32) < r.y + r.h;
                if at(x, y) != [0, 0, 0] {
                    if inside {
                        inside_lit += 1;
                    } else {
                        outside_lit += 1;
                    }
                }
            }
        }
        assert_eq!(outside_lit, 0, "{mode:?}：遊戲畫面矩形 {r:?} 之外不該有東西（比例不對或拉伸）");
        assert!(inside_lit > (r.w * r.h) as usize / 50, "{mode:?}：矩形內幾乎全黑（{inside_lit}）");
        if mode.is_mono() {
            assert!(r.w * 3 == r.h * 4 && r.x > 0, "{mode:?}：4:3 比視窗窄，左右要留黑邊：{r:?}");
        } else {
            assert_eq!((r.w, r.h), (960, 600), "{mode:?}：VGA、EGA 維持 8:5 填滿寬度");
        }
        images.push((mode, img));
    }
    // 單色的兩種畫面不同（CGA 是 200 列，MGA 把偶數列複製成 300 列），也和彩色不同。
    let by = |m: DisplayMode| &images.iter().find(|(x, _)| *x == m).unwrap().1.rgba;
    assert_ne!(by(DisplayMode::Cga), by(DisplayMode::Mga));
    assert_ne!(by(DisplayMode::Vga), by(DisplayMode::Cga));
    println!("四種模式的截圖都在 {}", dir.display());
}

// ---------------------------------------------------------------------------
// 切換不影響模擬
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn start_mode_and_midrun_mode_toggles_keep_the_dseg_hashes() {
    let dir = out_dir("hash");
    let rp = boss_receipt();
    let steps = STEPS.to_string();
    let base_args = ["--start", "newgame", "--replay", rp.to_str().unwrap(), "--headless-steps", steps.as_str()];
    let plain = run(&dir, &base_args);
    assert!(plain.ok, "{}", plain.stderr);
    assert_eq!(plain.hashes.len(), STEPS);
    assert!(plain.stderr.contains("顯示模式 vga"), "預設是 VGA：\n{}", plain.stderr);
    let mut compared = 0usize;
    // 起始模式。第一關沒有地形差異，雜湊要完全相同。
    for m in ["ega", "cga", "mga"] {
        let mut args = base_args.to_vec();
        args.extend(["--mode", m]);
        let r = run(&dir, &args);
        assert!(r.ok, "--mode {m}：{}", r.stderr);
        assert!(r.stderr.contains("presenter: sdl") && r.stderr.contains(&format!("顯示模式 {m}")), "--mode {m}：\n{}", r.stderr);
        assert_eq!(r.hashes, plain.hashes, "--mode {m} 改變了 Dseg 雜湊");
        compared += r.hashes.len();
    }
    // 中途切換：VGA→EGA→CGA→MGA→VGA→EGA，同時混入補間與幀計數的切換。
    let mut args = base_args.to_vec();
    args.extend([
        "--toggle-at", "100:mode", "--toggle-at", "300:mode", "--toggle-at", "301:tween", "--toggle-at", "500:mode", "--toggle-at", "700:mode", "--toggle-at", "800:counter", "--toggle-at",
        "900:mode",
    ]);
    let r = run(&dir, &args);
    assert!(r.ok, "{}", r.stderr);
    assert_eq!(r.hashes, plain.hashes, "中途切換顯示模式改變了 Dseg 雜湊");
    compared += r.hashes.len();
    // 切換進出單色的過程中，單色接收端沒有出現無法重現的命令。
    assert!(r.stderr.contains("單色接收端異常計數 CGA 0、MGA 0"), "單色接收端有異常：\n{}", r.stderr);
    assert!(compared >= 4 * STEPS, "比對筆數 {compared}");
    println!("模式切換：{compared} 筆雜湊與預設相同");
}

/// 全流程（標題、第一到第四關打完、片尾）用收據的真實輸入，中途一路切換顯示模式，每步 `Dseg` 雜湊與 `rs2-core` 重播相同。
///
/// 第三關的地形來源在進場時決定（已決定的取捨），所以進第三關之前的切換剛好繞一圈回到 VGA（途中經過 EGA、CGA、MGA，
/// 涵蓋第一、二關的全程），進場之後每 37 步換一個模式直到片尾（涵蓋第三關關內、第四關全程與片尾）。
#[test]
#[ignore]
fn full_flow_with_mode_switches_keeps_every_hash() {
    use rs2_trace::load_inputs;
    use rs2_trace::wait::{ReplayEvent, ReplayOpts, Verdict, load_receipt, replay_observed};
    let dir = out_dir("flow");
    let orig = env_path("RS2_ORIG");
    let inputs = load_inputs(&orig).unwrap();
    let assets = rs2_gfx::Assets::load(&orig).unwrap();
    let rc = load_receipt(&env_path("RS2_DOSRUN").join("wt_flow")).unwrap();
    let mut lines = String::new();
    let mut want: Vec<String> = Vec::new();
    let mut letters: Vec<u8> = Vec::new();
    let opts = ReplayOpts { ds_report: false, locate_root: None, boot: true };
    let v = replay_observed(&inputs, &assets, &rc, &opts, &mut |ev| {
        if let ReplayEvent::StepAt { sc, ds, .. } = ev {
            let codes: Vec<String> = sc.iter().map(|c| c.to_string()).collect();
            lines.push_str(&format!("{{\"sc\":[{}]}}\n", codes.join(",")));
            want.push(rs2_formats::sha256_hex(ds));
            letters.push(ds[rs2_core::a(0x1CFDB) as usize]);
        }
    });
    assert!(matches!(v, Verdict::Pass { .. }), "全流程收據沒有通過");
    let replay = dir.join("flow-sc.jsonl");
    std::fs::write(&replay, lines).unwrap();
    // 第三關進場那一步（字母第一次變成 C）。
    let n3 = letters.iter().position(|&l| l == b'C').expect("全流程沒有走到第三關");
    let n4 = letters.iter().position(|&l| l == b'D').expect("全流程沒有走到第四關");
    assert!(n3 > 500 && n4 > n3 + 100, "第三關進場 {n3}，第四關進場 {n4}");
    let steps = want.len();
    // 進場之前：四次切換平均分布（VGA→EGA→CGA→MGA→VGA）。進場之後：每 37 步一次，直到片尾。
    let mut toggles: Vec<String> = Vec::new();
    for i in 1..=4 {
        toggles.push(format!("{}:mode", n3 * i / 5 - 1));
    }
    let mut n = n3 + 20;
    while n < steps - 5 {
        toggles.push(format!("{n}:mode"));
        n += 37;
    }
    toggles.push(format!("{}:tween", n3 / 2));
    toggles.push(format!("{}:counter", n3 + 77));
    let n_after = toggles.iter().filter(|t| t.ends_with(":mode")).count() - 4;
    assert!(n_after >= 8, "全流程 {steps} 步，進場（{n3}）之後的切換只有 {n_after} 次");
    let (rp, st) = (replay.to_str().unwrap().to_string(), steps.to_string());
    let mut args: Vec<&str> = vec!["--original-bugs", "--replay", &rp, "--headless-steps", &st];
    for t in &toggles {
        args.extend(["--toggle-at", t.as_str()]);
    }
    let r = run(&dir, &args);
    assert!(r.ok, "失敗（{:?}）：\n{}", r.code, r.stderr);
    assert_eq!(r.hashes.len(), steps);
    let first_diff = (0..steps).find(|&i| r.hashes[i] != want[i]);
    assert_eq!(first_diff, None, "第 {} 步（第三關進場 {n3}、第四關進場 {n4}）的 Dseg 雜湊與 rs2-core 重播不同\n{}", first_diff.unwrap_or(0) + 1, r.stderr);
    assert!(r.stderr.contains("ending_"), "沒有走到片尾的調色盤：\n{}", r.stderr);
    println!("全流程 {steps} 步（第三關進場 {n3}，第四關進場 {n4}），{} 次切換，雜湊全部與 rs2-core 相同", toggles.len());
}

// ---------------------------------------------------------------------------
// 設定
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn the_saved_mode_is_restored_and_the_flag_overrides_it() {
    let dir = out_dir("settings");
    let cfg = dir.join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let args = ["--start", "demo", "--headless-steps", "5"];
    for mode in DisplayMode::ALL {
        std::fs::write(cfg.join("settings.txt"), format!("mode={}\n", mode.code())).unwrap();
        let r = run(&dir, &args);
        assert!(r.ok, "{}", r.stderr);
        assert!(r.stderr.contains(&format!("顯示模式 {}", mode.code())), "設定檔的 mode={} 沒有還原：\n{}", mode.code(), r.stderr);
        assert!(!r.stderr.contains("[設定] 警告"), "{}", r.stderr);
    }
    // 命令列覆蓋設定。
    std::fs::write(cfg.join("settings.txt"), "mode=mga\n").unwrap();
    let mut a = args.to_vec();
    a.extend(["--mode", "ega"]);
    let r = run(&dir, &a);
    assert!(r.ok && r.stderr.contains("顯示模式 ega"), "{}", r.stderr);
    // 壞值：警告，退回 VGA。
    std::fs::write(cfg.join("settings.txt"), "mode=hercules\ncounter=on\n").unwrap();
    let r = run(&dir, &args);
    assert!(r.ok, "{}", r.stderr);
    assert!(r.stderr.contains("[設定] 警告") && r.stderr.contains("mode"), "壞值要警告：\n{}", r.stderr);
    assert!(r.stderr.contains("顯示模式 vga"), "{}", r.stderr);
    // 命令列給了不認得的值：直接拒絕。
    let r = run(&dir, &["--mode", "hercules", "--headless-steps", "1"]);
    assert!(!r.ok && r.code == Some(2), "不認得的 --mode 要以 2 結束：{:?}\n{}", r.code, r.stderr);
}

// ---------------------------------------------------------------------------
// 缺檔
// ---------------------------------------------------------------------------

/// 造一個原版目錄的副本：除了 `except` 之外都是指向原檔的符號連結，`except` 由 `replace` 決定（`None` 是不放）。
#[cfg(unix)]
fn orig_copy(dir: &Path, except: &str, replace: Option<Vec<u8>>) -> PathBuf {
    let src = env_path("RS2_ORIG");
    let dst = dir.join(if replace.is_some() { "orig-corrupt" } else { "orig-missing" });
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::create_dir_all(&dst).unwrap();
    for e in std::fs::read_dir(&src).unwrap() {
        let e = e.unwrap();
        let name = e.file_name();
        if name.to_string_lossy() == except {
            continue;
        }
        std::os::unix::fs::symlink(e.path(), dst.join(&name)).unwrap();
    }
    if let Some(bytes) = replace {
        std::fs::write(dst.join(except), bytes).unwrap();
    }
    dst
}

#[cfg(unix)]
#[test]
#[ignore]
fn a_missing_or_corrupt_mono_bank_disables_cga_and_mga_only() {
    let dir = out_dir("missing");
    let rp = boss_receipt();
    let steps = "300".to_string();
    let base = run(&dir, &["--start", "newgame", "--replay", rp.to_str().unwrap(), "--headless-steps", &steps]);
    assert!(base.ok && base.hashes.len() == 300, "{}", base.stderr);
    assert!(base.stderr.contains("單色圖庫雜湊驗證通過"), "{}", base.stderr);

    let mut good = std::fs::read(env_path("RS2_ORIG").join("MY.BDB")).unwrap();
    good[10] ^= 0xFF;
    for (name, replace) in [("缺檔", None), ("雜湊不符", Some(good))] {
        let od = orig_copy(&dir, "MY.BDB", replace);
        for flag in [None, Some("cga"), Some("mga")] {
            let mut args = vec!["--start", "newgame", "--replay", rp.to_str().unwrap(), "--headless-steps", &steps];
            if let Some(m) = flag {
                args.extend(["--mode", m]);
            }
            let r = run_with(&od, &dir, &args);
            assert!(r.ok, "{name} {flag:?}：VGA 與 EGA 不能被單色圖庫拖垮（{:?}）\n{}", r.code, r.stderr);
            assert!(r.stderr.contains("單色圖庫載入失敗") && r.stderr.contains("MY.BDB"), "{name}：要說明哪個檔：\n{}", r.stderr);
            assert!(r.stderr.contains("顯示模式 vga"), "{name} {flag:?}：要退回 VGA：\n{}", r.stderr);
            if flag.is_some() {
                assert!(r.stderr.contains("改用 vga"), "{name} {flag:?}：要說明為什麼改成 VGA：\n{}", r.stderr);
            }
            assert_eq!(r.hashes, base.hashes, "{name} {flag:?}：模擬不受影響");
        }
        // 中途要求切到下一個模式（EGA），再下一個 CGA 被跳過。
        let r = run_with(&od, &dir, &["--start", "newgame", "--replay", rp.to_str().unwrap(), "--headless-steps", &steps, "--toggle-at", "50:mode", "--toggle-at", "100:mode", "--toggle-at", "150:mode"]);
        assert!(r.ok && r.hashes == base.hashes, "{name}：中途切換\n{}", r.stderr);
    }
}
