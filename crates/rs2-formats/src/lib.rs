//! RS-2 原版檔案的解碼。不做檔案 I/O 以外的任何事，也不內含原版位元組。
//! 格式依據見 `docs/re/02-file-formats.md`。

pub mod bank;
pub mod lzexe;
pub mod manifest;
pub mod sha256;

pub use bank::{BankItem, GoFile, parse_bank, parse_go, unrle};
pub use lzexe::{UnpackError, unpack_s_exe};
pub use manifest::{DSEG_IMAGE_OFFSET, EXPECTED, UNPACKED_SHA256, check_hash};
pub use sha256::{sha256, sha256_hex};
