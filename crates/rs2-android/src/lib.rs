//! Android 入口：SDLActivity 載入 libmain.so 後呼叫 `SDL_main`。其餘全部在 `rs2-play`。
//!
//! 這個 crate 只放 `rs2-play` 寫不了的兩件事，因為它們需要 `unsafe`，而 `rs2-play` 禁用 `unsafe`：
//! 匯出 C 符號，以及把 stdout、stderr 接到 logcat（Android 上這兩個預設丟棄，診斷訊息全靠它們）。

use std::ffi::{CString, c_char, c_int, c_void};

unsafe extern "C" {
    fn pipe(fds: *mut c_int) -> c_int;
    fn dup2(old: c_int, new: c_int) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize;
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

const ANDROID_LOG_INFO: c_int = 4;
const ANDROID_LOG_FATAL: c_int = 7;
const TAG: &std::ffi::CStr = c"rs2";

fn log_line(prio: c_int, line: &str) {
    if let Ok(text) = CString::new(line) {
        // 安全性：兩個指標都是以 NUL 結尾的有效字串，呼叫期間存活。
        unsafe {
            __android_log_write(prio, TAG.as_ptr(), text.as_ptr());
        }
    }
}

/// 把 stdout、stderr 接到一條管線，背景執行緒逐行轉寫到 logcat。失敗就算了，遊戲照跑。
fn redirect_stdio_to_logcat() {
    let mut fds: [c_int; 2] = [0; 2];
    // 安全性：`fds` 是兩個 c_int 的有效緩衝區，`dup2` 的參數都是有效的檔案描述元。
    unsafe {
        if pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        dup2(fds[1], 1);
        dup2(fds[1], 2);
    }
    let rd = fds[0];
    std::thread::spawn(move || {
        let mut pending: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            // 安全性：`buf` 在呼叫期間有效，長度正確。
            let n = unsafe { read(rd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                return;
            }
            pending.extend_from_slice(&buf[..n as usize]);
            while let Some(i) = pending.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = pending.drain(..=i).collect();
                log_line(ANDROID_LOG_INFO, String::from_utf8_lossy(&line[..line.len() - 1]).as_ref());
            }
        }
    });
}

/// SDLActivity 呼叫的入口。
///
/// # Safety
/// 由 SDL 以 C 呼叫慣例呼叫，參數沒有用到。
#[unsafe(no_mangle)]
pub extern "C" fn SDL_main(_argc: c_int, _argv: *mut *mut c_char) -> c_int {
    redirect_stdio_to_logcat();
    std::panic::set_hook(Box::new(|info| log_line(ANDROID_LOG_FATAL, &format!("當掉：{info}"))));
    match std::panic::catch_unwind(rs2_play::android::main) {
        Ok(rc) => rc,
        Err(_) => 101,
    }
}
