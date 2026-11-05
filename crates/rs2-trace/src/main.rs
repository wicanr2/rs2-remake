//! `rs2-trace replay --start demo|newgame --rows N [--compare 收據] [--out 檔] [--fixes none|all] [--allow-draws]`
//! `rs2-trace a0 --start demo|newgame --dump 傾印`
//!
//! 原版目錄由環境變數 `RS2_ORIG` 指定。比對遇到第一個不同的幀就停，回報欄位路徑。

use rs2_game::{Fixes, Game, Input, Start};
use rs2_trace::mirror::{A0_EXEMPT, Mirror};
use rs2_trace::{describe_diff, load_inputs, parse_sc, row_json, strip_emu};
use std::path::PathBuf;
use std::process::ExitCode;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let orig = PathBuf::from(std::env::var("RS2_ORIG").map_err(|_| "缺環境變數 RS2_ORIG".to_string())?);
    let inputs = load_inputs(&orig)?;
    let start = match arg(args, "--start").as_deref() {
        Some("demo") => Start::Demo,
        Some("newgame") => Start::NewGame { difficulty: None },
        _ => return Err("--start 要是 demo 或 newgame".into()),
    };
    let fixes = match arg(args, "--fixes").as_deref() {
        Some("all") => Fixes::ALL,
        _ => Fixes::NONE,
    };
    let mut game = Game::new(&inputs.image, &inputs.dm1, start, fixes);
    let mirror = inputs.mirror();
    match args.first().map(String::as_str) {
        Some("a0") => a0(&mirror, &game, &arg(args, "--dump").ok_or("缺 --dump")?),
        Some("replay") => replay(&mirror, &mut game, args),
        _ => Err("子命令要是 replay 或 a0".into()),
    }
}

fn a0(mirror: &Mirror, game: &Game, dump: &str) -> Result<(), String> {
    let want = std::fs::read(dump).map_err(|e| format!("讀不到 {dump}：{e}"))?;
    let got_dump = mirror.dump(game.state());
    let got = got_dump.bytes();
    let mut bad = Vec::new();
    let mut exempt = 0usize;
    for i in 0..0x10000usize {
        if got[i] != want[i] {
            if A0_EXEMPT.iter().any(|&(lo, hi)| (lo as usize) <= i && i <= hi as usize) {
                exempt += 1;
            } else {
                bad.push(i);
            }
        }
    }
    println!("A0 {dump}：豁免範圍內差異 {exempt} 位元組，範圍外 {} 位元組", bad.len());
    for &i in bad.iter().take(40) {
        println!("  dseg:{i:04X} 線性 {:05X} 重製 {:02X} 原版 {:02X}", 0x1CBC0 + i, got[i], want[i]);
    }
    if bad.is_empty() { Ok(()) } else { Err("A0 不通過".into()) }
}

fn replay(mirror: &Mirror, game: &mut Game, args: &[String]) -> Result<(), String> {
    let rows: u64 = arg(args, "--rows").ok_or("缺 --rows")?.parse().map_err(|_| "--rows 不是數字")?;
    let want: Option<Vec<String>> = match arg(args, "--compare") {
        Some(p) => Some(
            std::fs::read_to_string(&p)
                .map_err(|e| format!("讀不到 {p}：{e}"))?
                .lines()
                .map(String::from)
                .collect(),
        ),
        None => None,
    };
    let mut out = String::new();
    let mut n = 0u64;
    let mut mismatch: Option<String> = None;
    while n < rows {
        let sc: Vec<u8> = want.as_ref().and_then(|w| w.get(n as usize)).map(|l| parse_sc(l)).unwrap_or_default();
        let mut sampled: Option<String> = None;
        let frame = game.step(&Input { scancodes: &sc }, &mut |g: &Game, pt: u32| {
            sampled = Some(row_json(mirror, g, n, pt, &sc));
        });
        let Some(line) = sampled else {
            if frame.exit.is_some() {
                println!("第 {n} 步離開：{:?}", frame.exit);
                break;
            }
            continue;
        };
        if let Some(w) = want.as_ref().and_then(|w| w.get(n as usize)) {
            if strip_emu(w) != line {
                mismatch = Some(format!("第 {n} 筆不同（原版幀計數見下）\n{}", describe_diff(&line, strip_emu(w))));
                out.push_str(&line);
                out.push('\n');
                break;
            }
        }
        out.push_str(&line);
        out.push('\n');
        n += 1;
        if frame.exit.is_some() {
            println!("第 {n} 筆之後離開：{:?}", frame.exit);
            break;
        }
    }
    if let Some(p) = arg(args, "--out") {
        std::fs::write(&p, &out).map_err(|e| format!("寫不了 {p}：{e}"))?;
    }
    println!("完成 {n} 筆");
    match mismatch {
        Some(m) => Err(m),
        None => Ok(()),
    }
}
