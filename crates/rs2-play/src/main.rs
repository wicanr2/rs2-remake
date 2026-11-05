//! RS-2 重製的可玩前端：SDL2 視窗、鍵盤輸入、固定步長計時、工具列。規格見 `docs/spec/001-rust-first-stage.md`「前端」。
//!
//! 模擬與繪製命令在 `rs2-core`、`rs2-gfx`；這裡只做視窗與輸入。
//! 可單元測試的部分（輸入對照、自動重複、設定、工具列、計時、版面）不依賴 SDL 視窗。

mod app;
mod args;
mod audio;
mod controls;
mod font;
mod input;
mod pacer;
mod session;
mod settings;
mod toolbar;
mod view;

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
    match app::run(args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(rc) => ExitCode::from(rc as u8),
        Err(e) => {
            eprintln!("rs2-play: {e}");
            ExitCode::FAILURE
        }
    }
}
