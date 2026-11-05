//! 戰場大小（規格 005「戰場大小」）從設定檔進去的端到端測試：執行真正的 `rs2-play`（`--headless-steps`），
//! 走和視窗模式相同的設定讀取、`Session::step`、`Session::render` 與 SDL 貼圖
//! （`SDL_VIDEODRIVER=dummy`、`SDL_RENDER_DRIVER=software`）。
//!
//! 需要原版檔與收據，標 `#[ignore]`，由 `tools/rust.sh play-test` 帶 `--include-ignored` 跑。
//!
//! | 項目 | 內容 |
//! |---|---|
//! | 截圖 | 設定檔 `opt.field_size=120`、`150`：視窗截圖的遊戲畫面矩形是 `Fit::frame(欄數)`，框外全黑 |
//! | 決定性 | 同一份設定與輸入跑兩次，每步 `Dseg` 雜湊相同；寬戰場的雜湊和 100% 不同 |
//! | 不動原版 | 設定 100% 等於沒設；展示錄影與單色模式（CGA、MGA）即使設了 150% 也和沒設的雜湊相同 |

use rs2_gfx::png;
use rs2_play::toolbar::desktop_height;
use rs2_play::view::{Fit, fit_rect_for, window_size};
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rs2-play");

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh play-test 跑")))
}

fn out_dir(sub: &str) -> PathBuf {
    let d = env_path("RS2_OUT").join("play-field").join(sub);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("cfg")).unwrap();
    d
}

struct Run {
    ok: bool,
    code: Option<i32>,
    hashes: Vec<String>,
    stderr: String,
}

/// 執行 `rs2-play --orig <orig> <args> --config <dir>/cfg --hash-out <dir>/h.hash`。設定檔先由呼叫端寫好。
fn run(dir: &Path, args: &[&str]) -> Run {
    let hash = dir.join("h.hash");
    let _ = std::fs::remove_file(&hash);
    let o = Command::new(BIN)
        .arg("--orig")
        .arg(env_path("RS2_ORIG"))
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

fn set(dir: &Path, text: &str) {
    std::fs::write(dir.join("cfg").join("settings.txt"), text).unwrap();
}

fn boss_receipt() -> PathBuf {
    env_path("RS2_DOSRUN").join("boss1_kill/trace-s1.jsonl")
}

/// 設定檔的值（120、150）對應的欄數。
fn cols_of(opt: &str) -> u16 {
    match opt {
        "100" => 40,
        "120" => 48,
        _ => 60,
    }
}

/// 戰場加大時的視窗截圖：遊戲畫面的矩形是 `Fit::frame(欄數)` 置中，框外（工具列以下）全黑，框內有內容。
/// 有 HD 資料夾時再用 HD 圖各存一張（矩形相同，只有濾鏡不同）。
#[test]
#[ignore]
fn settings_file_field_size_gives_the_wide_window_geometry() {
    let dir = out_dir("shots");
    let rp = boss_receipt();
    let (ww, wh) = window_size(3, true);
    let bar = desktop_height(ww);
    let hd = std::env::var("RS2_HD").ok();
    for opt in ["120", "150"] {
        for use_hd in [false, true] {
            if use_hd && hd.is_none() {
                println!("沒有 RS2_HD，略過 HD 截圖");
                continue;
            }
            set(&dir, &format!("opt.field_size={opt}\n"));
            let cols = cols_of(opt);
            let shot = dir.join(format!("{opt}-{}.png", if use_hd { "hd" } else { "sd" }));
            let spec = format!("130:{}", shot.display());
            let mut args = vec!["--start", "newgame", "--replay", rp.to_str().unwrap(), "--headless-steps", "150", "--shot-at", &spec];
            match (&hd, use_hd) {
                (Some(h), true) => args.extend(["--hd", h.as_str()]),
                _ => args.push("--no-hd"),
            }
            let r = run(&dir, &args);
            assert!(r.ok, "{opt}% 失敗（{:?}）：\n{}", r.code, r.stderr);
            assert!(r.stderr.contains("[shot] 第 130 步") && !r.stderr.contains("（全黑，失敗）"), "{opt}%：\n{}", r.stderr);
            let img = png::decode(&std::fs::read(&shot).expect("沒有截圖")).expect("PNG 解不開");
            assert_eq!((img.w as u32, img.h as u32), (ww, wh), "{opt}%：截圖是視窗大小");
            let rect = fit_rect_for(ww, wh, bar, Fit::frame(cols, use_hd), false);
            let at = |x: usize, y: usize| &img.rgba[(y * img.w + x) * 4..(y * img.w + x) * 4 + 3];
            let (mut inside_lit, mut outside_lit) = (0usize, 0usize);
            for y in bar as usize..img.h {
                for x in 0..img.w {
                    let inside = (x as i32) >= rect.x && (x as i32) < rect.x + rect.w && (y as i32) >= rect.y && (y as i32) < rect.y + rect.h;
                    if at(x, y) != [0, 0, 0] {
                        if inside {
                            inside_lit += 1;
                        } else {
                            outside_lit += 1;
                        }
                    }
                }
            }
            assert_eq!(outside_lit, 0, "{opt}% hd={use_hd}：遊戲畫面矩形 {rect:?} 之外不該有東西");
            assert!(inside_lit > (rect.w * rect.h) as usize / 4, "{opt}% hd={use_hd}：矩形內幾乎全黑（{inside_lit} / {}）", rect.w * rect.h);
            // 比例：欄數 × 8 比 200。
            assert_eq!(i64::from(rect.w) * 200, i64::from(rect.h) * i64::from(cols) * 8, "{opt}%：矩形比例 {rect:?}");
            println!("{opt}% hd={use_hd}：矩形 {rect:?}，截圖 {}", shot.display());
        }
    }
}

/// 同一份設定與輸入跑兩次，每步雜湊相同；寬戰場和 100% 不同；設定 100% 等於沒設。
#[test]
#[ignore]
fn settings_file_field_size_is_deterministic_and_100_percent_is_the_original() {
    let dir = out_dir("hash");
    let rp = boss_receipt();
    let args = ["--start", "newgame", "--replay", rp.to_str().unwrap(), "--headless-steps", "400", "--no-hd"];
    set(&dir, "");
    let plain = run(&dir, &args);
    assert!(plain.ok && plain.hashes.len() == 400, "{}", plain.stderr);
    set(&dir, "opt.field_size=100\n");
    let hundred = run(&dir, &args);
    assert!(hundred.ok);
    assert_eq!(hundred.hashes, plain.hashes, "設定 100% 等於沒設");
    for opt in ["120", "150"] {
        set(&dir, &format!("opt.field_size={opt}\n"));
        let a = run(&dir, &args);
        let b = run(&dir, &args);
        assert!(a.ok && b.ok, "{opt}%：{}", a.stderr);
        assert_eq!(a.hashes.len(), 400);
        assert_eq!(a.hashes, b.hashes, "{opt}%：同樣的設定與輸入，每步雜湊要相同");
        assert_ne!(a.hashes, plain.hashes, "{opt}%：寬戰場要和 100% 不同");
        println!("{opt}%：400 步雜湊兩次相同");
    }
}

/// 展示錄影與單色模式一律 100%：即使設了 150%，每步雜湊和沒設的相同。
#[test]
#[ignore]
fn settings_file_field_size_does_not_touch_the_demo_or_the_mono_modes() {
    let dir = out_dir("demo-mono");
    let rp = boss_receipt();
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("展示錄影", vec!["--start", "demo", "--headless-steps", "300", "--no-hd"]),
        ("CGA", vec!["--start", "newgame", "--replay", rp.to_str().unwrap(), "--mode", "cga", "--headless-steps", "300"]),
        ("MGA", vec!["--start", "newgame", "--replay", rp.to_str().unwrap(), "--mode", "mga", "--headless-steps", "300"]),
    ];
    for (name, args) in cases {
        set(&dir, "");
        let plain = run(&dir, &args);
        assert!(plain.ok && plain.hashes.len() == 300, "{name}：{}", plain.stderr);
        set(&dir, "opt.field_size=150\n");
        let on = run(&dir, &args);
        assert!(on.ok, "{name}：{}", on.stderr);
        assert_eq!(on.hashes, plain.hashes, "{name}：設了 150% 也要和沒設的每步雜湊相同");
        println!("{name}：300 筆雜湊與沒設相同");
    }
}
