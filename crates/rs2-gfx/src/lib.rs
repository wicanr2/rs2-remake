//! 繪製命令的接收端。`VgaSink` 把 `rs2-core` 記下的 E 模式 blit 套到 4 個 64 KB 平面，逐像素重現原版。
//! `LayerSink` 吃同一批命令，輸出可補間、可換 HD 圖的 RGBA 畫面。
//! 規格見 `docs/spec/001-rust-first-stage.md`「繪製模型」。

mod assets;
mod font;
mod layer;
pub mod png;
mod sink;
pub mod vga;

pub use assets::Assets;
pub use layer::{HdAssets, INTERP_CROSS_OP, INTERP_MAX_PX, LayerSink, RenderOpts, pal};
pub use sink::{VgaSink, disp_row, locate, scratch_row};
