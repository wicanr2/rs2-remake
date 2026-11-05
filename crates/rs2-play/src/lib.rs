//! RS-2 重製的可玩前端：SDL2 視窗、鍵盤輸入、固定步長計時、工具列。規格 001「前端」。
//!
//! 模擬與繪製命令在 `rs2-core`、`rs2-gfx`；這裡只做視窗與輸入。
//! 可單元測試的部分（輸入對照、自動重複、設定、工具列、計時、版面、預測、觸控、手把）都不碰 SDL 子系統。
//!
//! 桌面版的入口是 `main.rs`，Android 的入口在 `rs2-android` crate，兩者都呼叫 [`app`]。

pub mod android;
pub mod app;
pub mod args;
pub mod audio;
pub mod controls;
pub mod font;
pub mod input;
pub mod pacer;
pub mod pad;
pub mod pak;
pub mod predict;
pub mod resources;
pub mod session;
pub mod settings;
pub mod toolbar;
pub mod touch;
pub mod view;
