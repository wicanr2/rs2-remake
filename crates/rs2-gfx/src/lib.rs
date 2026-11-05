//! 繪製命令的接收端。`VgaSink` 把 `rs2-game` 記下的 E 模式 blit 套到 4 個 64 KB 平面，逐像素重現原版。
//! `LayerSink` 吃同一批命令，輸出可補間、可換 HD 圖的 RGBA 畫面。
//! 規格見 `docs/spec/001-rust-first-stage.md`「繪製模型」。

mod assets;
mod font;
mod geom;
mod layer;
mod mono;
mod mono_hd;
pub mod png;
pub mod speaker;
mod sink;
pub mod vga;

pub use assets::Assets;
pub use geom::Geom;
pub use mono::{MONO_EXPECTED, MonoAssets, MonoBank, MonoKind, MonoSink, SCRATCH_SIZE};
pub use mono_hd::{MONO_HD_H, MONO_HD_W, MonoHdSink};
pub use layer::{HdAssets, HdLib, HdSet, INTERP_CROSS_OP, INTERP_MAX_PX, LayerSink, RenderOpts, pal};
pub use sink::{ATTR_REGS, BankDump, VgaSink, disp_row, ega_rgb16, locate, scratch_row};
