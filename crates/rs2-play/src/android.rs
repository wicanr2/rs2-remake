//! Android 的進入點：把 APK 裡的資料解到內部儲存，再進一般的主迴圈。符號匯出在 `rs2-android` crate。

use crate::{app, args, pak};
use sdl2::rwops::RWops;
use std::path::Path;
use std::time::Instant;

/// APK 的 `assets/` 底下的資料包。
pub const PAK_NAME: &str = "rs2-data.pak";

/// SDLActivity 呼叫的主程式。回傳結束碼。
pub fn main() -> i32 {
    // 視窗可以調整大小時 SDL 預設允許所有方向，這裡要明確鎖橫向。
    sdl2::hint::set("SDL_IOS_ORIENTATIONS", "LandscapeLeft LandscapeRight");
    match app::run_installing(install) {
        Ok(rc) => rc,
        Err(e) => {
            eprintln!("rs2-play: {e}");
            1
        }
    }
}

fn install(progress: &mut dyn FnMut(f32)) -> Result<args::Args, String> {
    // Android 上 SDL_GetPrefPath 回傳內部儲存（getFilesDir）底下的目錄，APK 所在的位置是唯讀的。
    let pref = sdl2::filesystem::pref_path("wicanr2", "rs2").map_err(|e| format!("取不到內部儲存：{e}"))?;
    let pref = Path::new(&pref);
    let root = pref.join("data");
    let mut rw = RWops::from_file(PAK_NAME, "rb").map_err(|e| format!("打不開 APK 內的 {PAK_NAME}：{e}"))?;
    let t0 = Instant::now();
    let fresh = pak::unpack(&mut rw, &root, progress)?;
    if fresh {
        eprintln!("[install] 資料解到 {}，{:.1} 秒", root.display(), t0.elapsed().as_secs_f32());
    } else {
        eprintln!("[install] 資料已經解過（{}）", root.display());
    }
    let mut a = args::parse(&[])?;
    a.orig = Some(root.join("original"));
    a.hd = Some(root.join("hd"));
    a.config = Some(pref.to_path_buf());
    Ok(a)
}
