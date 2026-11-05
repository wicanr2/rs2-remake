//! RS-2 第一關的決定性模擬。照機器碼逐函式移植，不碰時間、檔案、亂數源或繪圖。
//! 規格：`docs/spec/001-rust-first-stage.md`。

pub mod draw;
pub mod dseg;
mod ebullet;
mod entity;
mod flow;
#[cfg(test)]
mod fix_tests;
mod fx;
pub mod game;
mod pbullet;
mod player;
pub mod regs;
mod video;

pub use draw::{BlitCmd, DrawCmd, ObjId, Op, Pool, ShtGroup};
pub use dseg::{Dseg, a};
pub use game::{A0_EXEMPT, Exit, Fixes, Frame, Game, Input, Start};
pub use regs::Regs;
