//! 桌面版的程式入口。內容都在函式庫（`lib.rs`），Android 走 `rs2-android` crate。

use rs2_play::{app, args};
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match args::parse(&argv) {
        Ok(a) => a,
        Err(e) => {
            let help = argv.iter().any(|a| a == "-h" || a == "--help");
            eprintln!("{e}");
            return if help { ExitCode::SUCCESS } else { ExitCode::from(2) };
        }
    };
    if args.version {
        println!("rs2-play {}", args::VERSION);
        return ExitCode::SUCCESS;
    }
    match app::run(args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(rc) => ExitCode::from(rc as u8),
        Err(e) => {
            eprintln!("rs2-play: {e}");
            ExitCode::FAILURE
        }
    }
}
