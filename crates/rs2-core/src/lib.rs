//! RS-2 第一關的決定性模擬。照機器碼逐函式移植，不碰時間、檔案、亂數源或繪圖。
//! 規格：`docs/spec/001-rust-first-stage.md`。

mod ai;
mod blit;
mod boss_scroll;
mod cards;
pub mod draw;
pub mod dseg;
mod ebullet;
mod entity;
mod ending;
mod flow;
#[cfg(test)]
mod fix_tests;
mod fx;
pub mod game;
mod intro;
mod loader;
mod menu;
mod mirror;
mod pbullet;
mod player;
mod scene;
mod scenery;
mod stage1;
mod stage2;
mod stage3;
mod stage4;
mod stage_io;
mod title;
pub mod regs;
mod video;

pub use draw::{AudioCmd, BegGroup, BlitCmd, DrawCmd, MixFile, ObjId, Op, Pool, ShtGroup};
pub use dseg::{Dseg, a};
pub use game::{A0_EXEMPT, Core, DS_EXEMPT, Entry, Exit, Fixes, Frame, Game, GameFiles, HudStatus, Input, PlayerStatus, Rules, Start, boot_ds, boot_ds_with_dm1, ds_exempt};
pub use player::{WAVE_CHARGE_FULL, WAVE_LEVEL_1, WAVE_LEVEL_2, WAVE_LEVEL_3};
pub use regs::Regs;
