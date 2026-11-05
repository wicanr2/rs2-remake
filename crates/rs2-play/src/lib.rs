//! RS-2 重製的可玩前端：SDL2 視窗、鍵盤輸入、固定步長計時、工具列。規格 001「前端」。
//!
//! 模擬與繪製命令在 `rs2-core`、`rs2-gfx`；這裡只做視窗與輸入。
//! 可單元測試的部分（輸入對照與來源所有權、鍵位與手把重綁、設定、工具列、設定畫面、中文字型、計時、版面、預測、觸控、手把、顯示模式）都不碰 SDL 子系統。
//!
//! 桌面版的入口是 `main.rs`，Android 的入口在 `rs2-android` crate，兩者都呼叫 [`app`]。

pub mod android;
pub mod app;
pub mod args;
pub mod audio;
pub mod controls;
pub mod font;
pub mod input;
pub mod keymap;
pub mod mode;
pub mod options;
pub mod pacer;
pub mod pad;
pub mod pak;
pub mod predict;
pub mod resources;
pub mod session;
pub mod settings;
pub mod toolbar;
pub mod touch;
pub mod ui;
pub mod view;
