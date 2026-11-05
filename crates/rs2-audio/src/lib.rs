//! RS-2 的音訊：`ASMDRV` 驅動的移植、OPL2 合成、PC 喇叭。
//! 規格見 `docs/spec/003-audio.md`，驅動的行為見 `docs/re/13-audio-and-misc.md`。
//!
//! 不碰時間、檔案與音效裝置。呼叫端決定每產生多少取樣推進一個驅動 tick，所以可以和原版的暫存器寫入逐筆對拍。

pub mod driver;
pub mod engine;
pub mod opl2;
pub mod oracle;
pub mod speaker;

pub use driver::{Driver, Mix};
pub use engine::{Engine, Track};
pub use opl2::{NATIVE_RATE, Opl2};
pub use speaker::{SpkEv, SpkKind};
