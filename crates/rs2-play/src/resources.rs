//! 資源尋找：原版目錄與 HD 資料夾。不依賴 SDL，環境由 [`Env`] 注入，所以三個平台的規則都能單元測試。
//!
//! 資料根（data root）底下是 `original/`（原版檔案）與 `hd/`（HD 素材），發行包內建的位置固定如下：
//!
//! | 平台 | 資料根 |
//! |---|---|
//! | Linux AppImage | `$APPDIR/usr/share/rs2`，沒有 `APPDIR` 時用執行檔往上一層的 `share/rs2` |
//! | Windows | 執行檔旁的 `data` |
//! | macOS | `.app/Contents/Resources`（執行檔在 `Contents/MacOS`） |
//!
//! 尋找順序：命令列（`--orig`、`--hd`）、環境變數 `RS2_DATA`、內建位置。命令列給了就不再往下找。
//! `RS2_DATA` 可以指向資料根，也可以直接指向原版目錄。

use std::path::{Path, PathBuf};

/// 判斷一個目錄是不是原版目錄用的檔案。雜湊由 `load_inputs` 驗，這裡只確認它存在。
pub const ORIG_MARKER: &str = "S.EXE";
/// 資料根底下的子目錄名。
pub const ORIG_SUBDIR: &str = "original";
pub const HD_SUBDIR: &str = "hd";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
    MacOs,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }
}

/// 尋找資源與設定目錄會用到的環境。空字串視同沒有。
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub rs2_data: Option<String>,
    pub appdir: Option<String>,
    pub exe: Option<PathBuf>,
    pub xdg_config_home: Option<String>,
    pub home: Option<String>,
    pub appdata: Option<String>,
    pub userprofile: Option<String>,
}

impl Env {
    pub fn from_process() -> Env {
        let var = |n: &str| std::env::var(n).ok().filter(|v| !v.is_empty());
        Env {
            rs2_data: var("RS2_DATA"),
            appdir: var("APPDIR"),
            exe: std::env::current_exe().ok(),
            xdg_config_home: var("XDG_CONFIG_HOME"),
            home: var("HOME"),
            appdata: var("APPDATA"),
            userprofile: var("USERPROFILE"),
        }
    }
}

pub(crate) fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|v| !v.is_empty())
}

/// 發行包內建的資料根，依優先序排列且不重複。
pub fn bundled_roots(os: Os, env: &Env) -> Vec<PathBuf> {
    let exe_dir = env.exe.as_deref().and_then(Path::parent);
    let mut v: Vec<PathBuf> = Vec::new();
    match os {
        Os::Linux => {
            if let Some(a) = non_empty(&env.appdir) {
                v.push(Path::new(a).join("usr").join("share").join("rs2"));
            }
            if let Some(up) = exe_dir.and_then(Path::parent) {
                v.push(up.join("share").join("rs2"));
            }
        }
        Os::Windows => {
            if let Some(d) = exe_dir {
                v.push(d.join("data"));
            }
        }
        Os::MacOs => {
            if let Some(up) = exe_dir.and_then(Path::parent) {
                v.push(up.join("Resources"));
            }
        }
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    v.retain(|p| {
        if seen.contains(p) {
            false
        } else {
            seen.push(p.clone());
            true
        }
    });
    v
}

/// 原版目錄的檢查。通過回傳 `Ok`，否則回傳原因。
fn check_orig(dir: &Path) -> Result<(), &'static str> {
    if !dir.exists() {
        Err("不存在")
    } else if !dir.is_dir() {
        Err("不是目錄")
    } else if !dir.join(ORIG_MARKER).is_file() {
        Err("沒有 S.EXE")
    } else {
        Ok(())
    }
}

fn check_hd(dir: &Path) -> Result<(), &'static str> {
    if !dir.exists() {
        Err("不存在")
    } else if !dir.is_dir() {
        Err("不是目錄")
    } else {
        Ok(())
    }
}

/// 一個找過的位置，錯誤訊息用。
struct Tried {
    label: &'static str,
    path: Option<PathBuf>,
    result: String,
}

fn describe(tried: &[Tried]) -> String {
    tried
        .iter()
        .map(|t| match &t.path {
            Some(p) => format!("  {}：{}（{}）", t.label, p.display(), t.result),
            None => format!("  {}：{}", t.label, t.result),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 尋找結果。`notes` 是要印給使用者看的來源說明。
#[derive(Debug)]
pub struct Resolved {
    pub orig: PathBuf,
    pub hd: Option<PathBuf>,
    pub notes: Vec<String>,
}

const LABEL_CLI_ORIG: &str = "命令列 --orig";
const LABEL_CLI_HD: &str = "命令列 --hd";
const LABEL_ENV: &str = "環境變數 RS2_DATA";
const LABEL_BUNDLED: &str = "內建位置";

/// 候選位置，依優先序。`sub` 是資料根底下的子目錄。
fn candidates(sub: &str, env: &Env, os: Os, allow_direct: bool) -> Vec<(&'static str, PathBuf)> {
    let mut c: Vec<(&'static str, PathBuf)> = Vec::new();
    if let Some(d) = non_empty(&env.rs2_data) {
        c.push((LABEL_ENV, Path::new(d).join(sub)));
        if allow_direct {
            c.push((LABEL_ENV, PathBuf::from(d)));
        }
    }
    for r in bundled_roots(os, env) {
        c.push((LABEL_BUNDLED, r.join(sub)));
    }
    c
}

/// 決定原版目錄與 HD 資料夾。原版找不到時錯誤訊息列出所有找過的位置；HD 找不到不是錯誤，改用原版圖。
pub fn resolve(
    cli_orig: Option<&Path>,
    cli_hd: Option<&Path>,
    no_hd: bool,
    env: &Env,
    os: Os,
) -> Result<Resolved, String> {
    let mut notes = Vec::new();

    let orig = match cli_orig {
        Some(p) => {
            notes.push(format!("原版目錄：{}（{LABEL_CLI_ORIG}）", p.display()));
            p.to_path_buf()
        }
        None => {
            let mut tried = vec![Tried { label: LABEL_CLI_ORIG, path: None, result: "未指定".into() }];
            if non_empty(&env.rs2_data).is_none() {
                tried.push(Tried { label: LABEL_ENV, path: None, result: "未設定".into() });
            }
            let mut found = None;
            for (label, p) in candidates(ORIG_SUBDIR, env, os, true) {
                match check_orig(&p) {
                    Ok(()) => {
                        found = Some((label, p));
                        break;
                    }
                    Err(why) => tried.push(Tried { label, path: Some(p), result: why.into() }),
                }
            }
            if bundled_roots(os, env).is_empty() {
                tried.push(Tried { label: LABEL_BUNDLED, path: None, result: "取不到執行檔位置".into() });
            }
            match found {
                Some((label, p)) => {
                    notes.push(format!("原版目錄：{}（{label}）", p.display()));
                    p
                }
                None => {
                    return Err(format!(
                        "找不到原版資料。已找過：\n{}\n\
                         可以用 --orig <原版目錄>，或設定環境變數 RS2_DATA（資料根，底下有 {ORIG_SUBDIR}/ 與 {HD_SUBDIR}/，\
                         或直接指向原版目錄）。完整版發行包已內建資料。",
                        describe(&tried)
                    ));
                }
            }
        }
    };

    let hd = if no_hd {
        notes.push("HD：--no-hd，用原版圖".into());
        None
    } else if let Some(p) = cli_hd {
        notes.push(format!("HD 資料夾：{}（{LABEL_CLI_HD}）", p.display()));
        Some(p.to_path_buf())
    } else {
        let mut tried = Vec::new();
        let mut found = None;
        for (label, p) in candidates(HD_SUBDIR, env, os, false) {
            match check_hd(&p) {
                Ok(()) => {
                    found = Some((label, p));
                    break;
                }
                Err(why) => tried.push(Tried { label, path: Some(p), result: why.into() }),
            }
        }
        match found {
            Some((label, p)) => {
                notes.push(format!("HD 資料夾：{}（{label}），預設使用", p.display()));
                Some(p)
            }
            None => {
                if tried.is_empty() {
                    notes.push("HD：沒有指定也沒有內建位置，用原版圖".into());
                } else {
                    notes.push(format!("HD：沒有找到，用原版圖。已找過：\n{}", describe(&tried)));
                }
                None
            }
        }
    };

    Ok(Resolved { orig, hd, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(name: &str) -> Tmp {
            let d = std::env::temp_dir().join(format!("rs2-play-res-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Tmp(d)
        }
        fn p(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
        /// 造一個資料根：original/S.EXE，hd 選用。
        fn data_root(&self, rel: &str, hd: bool) -> PathBuf {
            let root = self.p(rel);
            std::fs::create_dir_all(root.join(ORIG_SUBDIR)).unwrap();
            std::fs::write(root.join(ORIG_SUBDIR).join(ORIG_MARKER), b"x").unwrap();
            if hd {
                std::fs::create_dir_all(root.join(HD_SUBDIR)).unwrap();
            }
            root
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn env_exe(exe: &str) -> Env {
        Env { exe: Some(PathBuf::from(exe)), ..Env::default() }
    }

    #[test]
    fn marker_is_a_real_expected_file() {
        assert!(rs2_formats::EXPECTED.iter().any(|(n, _)| *n == ORIG_MARKER));
    }

    #[test]
    fn linux_appimage_roots() {
        // AppImage 掛載後：APPDIR 與執行檔位置推出同一個資料根，只留一份。
        let mut e = env_exe("/tmp/.mount_rs2abc/usr/bin/rs2-play");
        e.appdir = Some("/tmp/.mount_rs2abc".into());
        assert_eq!(bundled_roots(Os::Linux, &e), vec![PathBuf::from("/tmp/.mount_rs2abc/usr/share/rs2")]);
        // 沒有 APPDIR（直接跑解開的 AppDir 內的執行檔）也找得到。
        e.appdir = None;
        assert_eq!(bundled_roots(Os::Linux, &e), vec![PathBuf::from("/tmp/.mount_rs2abc/usr/share/rs2")]);
        // 解開成 squashfs-root 後 APPDIR 指向別處，兩個都列出，APPDIR 優先。
        e.appdir = Some("/home/u/squashfs-root".into());
        assert_eq!(
            bundled_roots(Os::Linux, &e),
            vec![PathBuf::from("/home/u/squashfs-root/usr/share/rs2"), PathBuf::from("/tmp/.mount_rs2abc/usr/share/rs2")]
        );
        // /usr/bin 的一般安裝：/usr/share/rs2。
        assert_eq!(bundled_roots(Os::Linux, &env_exe("/usr/bin/rs2-play")), vec![PathBuf::from("/usr/share/rs2")]);
    }

    #[test]
    fn windows_root_is_data_next_to_exe() {
        let e = env_exe("C:/Games/RS-2/rs2-play.exe");
        assert_eq!(bundled_roots(Os::Windows, &e), vec![PathBuf::from("C:/Games/RS-2/data")]);
        // Windows 不看 APPDIR。
        let mut e2 = e.clone();
        e2.appdir = Some("/tmp/x".into());
        assert_eq!(bundled_roots(Os::Windows, &e2), vec![PathBuf::from("C:/Games/RS-2/data")]);
    }

    #[test]
    fn macos_root_is_contents_resources() {
        let e = env_exe("/Applications/RS-2.app/Contents/MacOS/rs2-play");
        assert_eq!(bundled_roots(Os::MacOs, &e), vec![PathBuf::from("/Applications/RS-2.app/Contents/Resources")]);
        // 路徑含空白與非 ASCII 也照常。
        let e = env_exe("/Users/甲 乙/Apps/RS-2 remake.app/Contents/MacOS/rs2-play");
        assert_eq!(
            bundled_roots(Os::MacOs, &e),
            vec![PathBuf::from("/Users/甲 乙/Apps/RS-2 remake.app/Contents/Resources")]
        );
    }

    #[test]
    fn no_exe_means_no_bundled_roots() {
        for os in [Os::Linux, Os::Windows, Os::MacOs] {
            assert!(bundled_roots(os, &Env::default()).is_empty(), "{os:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_native_paths() {
        let e = Env { exe: Some(PathBuf::from(r"C:\Games\RS-2\rs2-play.exe")), ..Env::default() };
        assert_eq!(bundled_roots(Os::Windows, &e), vec![PathBuf::from(r"C:\Games\RS-2\data")]);
        let d = Tmp::new("winnative");
        let root = d.data_root("RS-2\\data", true);
        let exe = d.p("RS-2").join("rs2-play.exe");
        let e = Env { exe: Some(exe), ..Env::default() };
        let r = resolve(None, None, false, &e, Os::Windows).unwrap();
        assert_eq!(r.orig, root.join(ORIG_SUBDIR));
        assert_eq!(r.hd, Some(root.join(HD_SUBDIR)));
    }

    #[test]
    fn bundled_orig_and_hd_are_found_on_each_os() {
        let d = Tmp::new("bundled");
        let linux_root = d.data_root("lx/usr/share/rs2", true);
        let win_root = d.data_root("win/data", true);
        let mac_root = d.data_root("mac/RS-2.app/Contents/Resources", true);
        let cases = [
            (Os::Linux, d.p("lx/usr/bin/rs2-play"), linux_root),
            (Os::Windows, d.p("win/rs2-play.exe"), win_root),
            (Os::MacOs, d.p("mac/RS-2.app/Contents/MacOS/rs2-play"), mac_root),
        ];
        for (os, exe, root) in cases {
            let r = resolve(None, None, false, &Env { exe: Some(exe), ..Env::default() }, os).unwrap();
            assert_eq!(r.orig, root.join(ORIG_SUBDIR), "{os:?}");
            assert_eq!(r.hd, Some(root.join(HD_SUBDIR)), "{os:?}");
            assert!(r.notes.iter().any(|n| n.contains("內建位置")), "{:?}", r.notes);
        }
    }

    #[test]
    fn priority_is_cli_then_env_then_bundled() {
        let d = Tmp::new("prio");
        let bundled = d.data_root("lx/usr/share/rs2", true);
        let envroot = d.data_root("envdata", true);
        let mut e = env_exe(d.p("lx/usr/bin/rs2-play").to_str().unwrap());
        // 只有內建。
        let r = resolve(None, None, false, &e, Os::Linux).unwrap();
        assert_eq!(r.orig, bundled.join(ORIG_SUBDIR));
        // RS2_DATA 勝過內建，原版與 HD 都是。
        e.rs2_data = Some(envroot.to_str().unwrap().into());
        let r = resolve(None, None, false, &e, Os::Linux).unwrap();
        assert_eq!(r.orig, envroot.join(ORIG_SUBDIR));
        assert_eq!(r.hd, Some(envroot.join(HD_SUBDIR)));
        // 命令列勝過 RS2_DATA，而且不檢查內容（讓 load_inputs 報具體缺哪個檔）。
        let cli_o = d.p("nowhere-orig");
        let cli_h = d.p("nowhere-hd");
        let r = resolve(Some(&cli_o), Some(&cli_h), false, &e, Os::Linux).unwrap();
        assert_eq!(r.orig, cli_o);
        assert_eq!(r.hd, Some(cli_h));
        // 只給 --orig 時 HD 仍照順序找。
        let r = resolve(Some(&cli_o), None, false, &e, Os::Linux).unwrap();
        assert_eq!(r.hd, Some(envroot.join(HD_SUBDIR)));
    }

    #[test]
    fn rs2_data_may_point_straight_at_the_original_dir() {
        let d = Tmp::new("direct");
        let root = d.data_root("flat", false);
        let direct = root.join(ORIG_SUBDIR);
        let e = Env { rs2_data: Some(direct.to_str().unwrap().into()), ..Env::default() };
        let r = resolve(None, None, false, &e, Os::Linux).unwrap();
        assert_eq!(r.orig, direct);
        assert_eq!(r.hd, None, "{:?}", r.notes);
    }

    #[test]
    fn env_without_valid_data_falls_through_to_bundled() {
        let d = Tmp::new("fall");
        let bundled = d.data_root("lx/usr/share/rs2", false);
        let mut e = env_exe(d.p("lx/usr/bin/rs2-play").to_str().unwrap());
        e.rs2_data = Some(d.p("empty").to_str().unwrap().into());
        let r = resolve(None, None, false, &e, Os::Linux).unwrap();
        assert_eq!(r.orig, bundled.join(ORIG_SUBDIR));
    }

    #[test]
    fn no_hd_and_missing_hd() {
        let d = Tmp::new("nohd");
        let with_hd = d.data_root("a/usr/share/rs2", true);
        let e = env_exe(d.p("a/usr/bin/rs2-play").to_str().unwrap());
        let r = resolve(None, Some(&with_hd), true, &e, Os::Linux).unwrap();
        assert_eq!(r.hd, None, "--no-hd 勝過 --hd");
        let without = d.data_root("b/usr/share/rs2", false);
        let e = env_exe(d.p("b/usr/bin/rs2-play").to_str().unwrap());
        let r = resolve(None, None, false, &e, Os::Linux).unwrap();
        assert_eq!(r.orig, without.join(ORIG_SUBDIR));
        assert_eq!(r.hd, None);
        assert!(r.notes.iter().any(|n| n.contains("沒有找到") && n.contains("hd")), "{:?}", r.notes);
    }

    #[test]
    fn not_found_lists_every_path_tried() {
        let d = Tmp::new("notfound");
        // 路徑逐段 join：Windows 上混用 / 與 \ 的字串比對不會相等（Path 比較是逐段的，字串不是）。
        let lx = d.p("lx");
        let share_orig = lx.join("usr").join("share").join("rs2").join("original");
        std::fs::create_dir_all(&share_orig).unwrap(); // 目錄在，但沒有 S.EXE
        let exe = lx.join("usr").join("bin").join("rs2-play");
        let mut e = env_exe(exe.to_str().unwrap());
        e.rs2_data = Some(d.p("gone").to_str().unwrap().into());
        let err = resolve(None, None, false, &e, Os::Linux).unwrap_err();
        assert!(err.contains("命令列 --orig：未指定"), "{err}");
        assert!(err.contains(&format!("{}（不存在）", d.p("gone").join("original").display())), "{err}");
        assert!(err.contains(&format!("{}（不存在）", d.p("gone").display())), "{err}");
        assert!(err.contains(&format!("{}（沒有 S.EXE）", share_orig.display())), "{err}");
        assert!(err.contains("--orig") && err.contains("RS2_DATA"), "{err}");
        // 什麼都沒設、取不到執行檔位置。
        let err = resolve(None, None, false, &Env::default(), Os::Windows).unwrap_err();
        assert!(err.contains("RS2_DATA：未設定") && err.contains("取不到執行檔位置"), "{err}");
    }

    #[test]
    fn marker_must_be_a_file() {
        let d = Tmp::new("markerdir");
        std::fs::create_dir_all(d.p("o/S.EXE")).unwrap(); // S.EXE 是目錄
        let e = Env { rs2_data: Some(d.p("o").to_str().unwrap().into()), ..Env::default() };
        assert!(resolve(None, None, false, &e, Os::Linux).is_err());
    }
}
