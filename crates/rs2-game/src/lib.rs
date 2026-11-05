//! RS-2 的決定性模擬。不碰時間、檔案、亂數源或繪圖，所以固定輸入就能逐幀重播。
//! 規格：`docs/spec/001-rust-first-stage.md`、`docs/spec/004-rewrite-and-delivery.md`。
//!
//! 遊戲的狀態是 [`State`]，唯讀表格是 [`Tables`]（啟動時從 `S.EXE` 解包映像解析，雜湊不符就拒絕）。
//! 遊戲不認識原版的 64 KB 資料段，資料段與狀態之間的對應在 `rs2-trace` 的 `Mirror`。

mod ai;
pub mod boss_scroll;
mod cards;
pub mod draw;
pub mod ebullet;
mod ending;
pub mod entity;
#[cfg(test)]
mod fix_tests;
pub mod flow;
pub mod fx;
pub mod game;
pub mod grid;
mod intro;
pub mod loader;
pub mod menu;
pub mod pbullet;
pub mod player;
mod scene;
pub mod scenery;
pub mod sound;
pub mod stage1;
pub mod stage2;
pub mod stage3;
pub mod stage4;
pub mod tables;
pub mod title;
pub mod video;

pub use draw::{AudioCmd, BegGroup, BlitCmd, DrawCmd, MixFile, ObjId, Op, Pool, ShtGroup};
pub use game::{Core, Entry, Exit, Fixes, Frame, Game, GameFiles, HudStatus, Input, PlayerStatus, Rules, Start, State, boot_state};
pub use grid::Grids;
pub use player::{WAVE_CHARGE_FULL, WAVE_LEVEL_1, WAVE_LEVEL_2, WAVE_LEVEL_3};
pub use tables::Tables;
