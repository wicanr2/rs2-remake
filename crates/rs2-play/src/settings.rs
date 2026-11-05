//! 本機設定：`settings.txt`，每行 `key=value`。讀不到用預設，壞掉的值略過並回報警告。

use crate::resources::{Env, Os, non_empty};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "settings.txt";
/// F4 循環的倍率。
pub const SCALES: [u32; 4] = [2, 3, 4, 5];
pub const DEFAULT_SCALE: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// 畫面補間（F1）。
    pub tween: bool,
    /// 幀計數（F2）。
    pub counter: bool,
    /// 用 HD 圖（F3）。沒有 HD 資料夾時不生效。
    pub hd: bool,
    /// 視窗倍率（F4），遊戲畫面 320×200 的整數倍。
    pub scale: u32,
    /// 工具列顯示（F10）。
    pub toolbar: bool,
    /// 玩家機預測與短按保留（F5）。關掉就是原版時序，模擬不受影響。
    pub feel: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { tween: true, counter: false, hd: true, scale: DEFAULT_SCALE, toolbar: true, feel: true }
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "on" | "1" | "true" | "yes" => Some(true),
        "off" | "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

fn b(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

impl Settings {
    /// F4：2、3、4、5 循環。目前值不在清單內（例如命令列給 1 或 8）就取清單中比它大的第一個，沒有就回到 2。
    pub fn next_scale(cur: u32) -> u32 {
        SCALES.iter().copied().find(|&s| s > cur).unwrap_or(SCALES[0])
    }

    pub fn to_text(&self) -> String {
        format!(
            "# RS-2 remake 設定\ntween={}\ncounter={}\nhd={}\nscale={}\ntoolbar={}\nfeel={}\n",
            b(self.tween),
            b(self.counter),
            b(self.hd),
            self.scale,
            b(self.toolbar),
            b(self.feel)
        )
    }

    /// 解析文字。壞掉的行或值略過並寫進警告，其餘欄位維持預設。
    pub fn parse(text: &str) -> (Settings, Vec<String>) {
        let mut s = Settings::default();
        let mut warn = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                warn.push(format!("第 {} 行沒有 =，略過：{line}", i + 1));
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            let mut flag = |slot: &mut bool| match parse_bool(v) {
                Some(x) => *slot = x,
                None => warn.push(format!("第 {} 行 {k} 的值 {v:?} 不是 on 或 off，略過", i + 1)),
            };
            match k {
                "tween" => flag(&mut s.tween),
                "counter" => flag(&mut s.counter),
                "hd" => flag(&mut s.hd),
                "toolbar" => flag(&mut s.toolbar),
                "feel" => flag(&mut s.feel),
                "scale" => match v.parse::<u32>() {
                    Ok(n) if (1..=8).contains(&n) => s.scale = n,
                    _ => warn.push(format!("第 {} 行 scale 的值 {v:?} 不在 1 到 8，略過", i + 1)),
                },
                _ => warn.push(format!("第 {} 行的鍵 {k:?} 不認得，略過", i + 1)),
            }
        }
        (s, warn)
    }

    /// 讀 `dir/settings.txt`。檔案不存在是常態，不算警告；其他讀取錯誤列為警告，用預設。
    pub fn load(dir: &Path) -> (Settings, Vec<String>) {
        let path = dir.join(FILE_NAME);
        match std::fs::read_to_string(&path) {
            Ok(t) => Settings::parse(&t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), Vec::new()),
            Err(e) => (Settings::default(), vec![format!("讀不到 {}：{e}，用預設值", path.display())]),
        }
    }

    /// 先寫暫存檔再改名，避免寫到一半被中斷留下殘檔。
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{FILE_NAME}.tmp"));
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, dir.join(FILE_NAME))
    }
}

/// Linux 的預設設定目錄：`$XDG_CONFIG_HOME/rs2-remake`，沒有就 `$HOME/.config/rs2-remake`。
pub fn default_dir(xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    match (xdg, home) {
        (Some(x), _) if !x.is_empty() => Some(Path::new(x).join("rs2-remake")),
        (_, Some(h)) if !h.is_empty() => Some(Path::new(h).join(".config").join("rs2-remake")),
        _ => None,
    }
}

/// 各平台的預設設定目錄。發行包所在的位置（AppImage 掛載點、`.app`、解壓目錄）可能唯讀，設定不寫在那裡。
///
/// | 平台 | 目錄 |
/// |---|---|
/// | Linux | `$XDG_CONFIG_HOME/rs2-remake`，沒有就 `~/.config/rs2-remake` |
/// | Windows | `%APPDATA%\rs2-remake`，沒有 `APPDATA` 就 `%USERPROFILE%\AppData\Roaming\rs2-remake` |
/// | macOS | `~/Library/Application Support/rs2-remake` |
pub fn config_dir(os: Os, env: &Env) -> Option<PathBuf> {
    match os {
        Os::Linux => default_dir(non_empty(&env.xdg_config_home), non_empty(&env.home)),
        Os::Windows => non_empty(&env.appdata)
            .map(|a| Path::new(a).join("rs2-remake"))
            .or_else(|| non_empty(&env.userprofile).map(|u| Path::new(u).join("AppData").join("Roaming").join("rs2-remake"))),
        Os::MacOs => non_empty(&env.home).map(|h| Path::new(h).join("Library").join("Application Support").join("rs2-remake")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rs2-play-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn defaults_match_the_spec() {
        let s = Settings::default();
        assert!(s.tween && !s.counter && s.hd && s.toolbar && s.feel);
        assert_eq!(s.scale, 3);
    }

    #[test]
    fn text_roundtrip() {
        for tween in [false, true] {
            for counter in [false, true] {
                for hd in [false, true] {
                    for toolbar in [false, true] {
                        for scale in [1, 2, 3, 4, 5, 8] {
                            for feel in [false, true] {
                                let s = Settings { tween, counter, hd, scale, toolbar, feel };
                                let (back, warn) = Settings::parse(&s.to_text());
                                assert_eq!(back, s);
                                assert!(warn.is_empty(), "{warn:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn file_roundtrip_and_missing_file() {
        let d = tmpdir("file");
        let (s, w) = Settings::load(&d);
        assert_eq!(s, Settings::default());
        assert!(w.is_empty(), "檔案不存在不算警告：{w:?}");
        let want = Settings { tween: false, counter: true, hd: false, scale: 5, toolbar: false, feel: false };
        want.save(&d).unwrap();
        let (got, w) = Settings::load(&d);
        assert_eq!(got, want);
        assert!(w.is_empty());
        assert!(!d.join("settings.txt.tmp").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn bad_values_are_skipped_with_warnings() {
        let text = "tween=maybe\ncounter=on\nscale=99\nhd=off\nscale=abc\nnonsense\nfoo=1\ntoolbar = 0 \n# 註解\n\n";
        let (s, w) = Settings::parse(text);
        // 壞掉的值維持預設，好的值照收。
        assert!(s.tween, "tween 的壞值要維持預設 on");
        assert!(s.counter);
        assert!(!s.hd);
        assert!(!s.toolbar);
        assert_eq!(s.scale, 3);
        assert_eq!(w.len(), 5, "{w:?}");
        assert!(w.iter().any(|m| m.contains("tween")));
        assert!(w.iter().any(|m| m.contains("scale")));
        assert!(w.iter().any(|m| m.contains("foo")));
        assert!(w.iter().any(|m| m.contains("nonsense")));
    }

    #[test]
    fn garbage_file_gives_defaults() {
        let d = tmpdir("garbage");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(FILE_NAME), [0xFF, 0xFE, 0x00, 0x80]).unwrap();
        let (s, w) = Settings::load(&d);
        assert_eq!(s, Settings::default());
        assert!(!w.is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn scale_cycle() {
        assert_eq!(Settings::next_scale(2), 3);
        assert_eq!(Settings::next_scale(3), 4);
        assert_eq!(Settings::next_scale(4), 5);
        assert_eq!(Settings::next_scale(5), 2);
        assert_eq!(Settings::next_scale(1), 2);
        assert_eq!(Settings::next_scale(8), 2);
    }

    #[test]
    fn default_dir_rules() {
        assert_eq!(default_dir(Some("/x"), Some("/h")), Some(PathBuf::from("/x/rs2-remake")));
        assert_eq!(default_dir(Some(""), Some("/h")), Some(PathBuf::from("/h/.config/rs2-remake")));
        assert_eq!(default_dir(None, Some("/h")), Some(PathBuf::from("/h/.config/rs2-remake")));
        assert_eq!(default_dir(None, None), None);
    }

    fn env(xdg: Option<&str>, home: Option<&str>, appdata: Option<&str>, profile: Option<&str>) -> Env {
        let s = |v: Option<&str>| v.map(String::from);
        Env { xdg_config_home: s(xdg), home: s(home), appdata: s(appdata), userprofile: s(profile), ..Env::default() }
    }

    #[test]
    fn config_dir_per_platform() {
        // Linux：XDG 優先，其次 ~/.config，空字串視同沒有。
        let e = env(Some("/x"), Some("/h"), None, None);
        assert_eq!(config_dir(Os::Linux, &e), Some(PathBuf::from("/x/rs2-remake")));
        let e = env(Some(""), Some("/h"), None, None);
        assert_eq!(config_dir(Os::Linux, &e), Some(PathBuf::from("/h/.config/rs2-remake")));
        assert_eq!(config_dir(Os::Linux, &env(None, None, None, None)), None);
        // Windows：%APPDATA%，沒有時退到 %USERPROFILE%\AppData\Roaming，不看 HOME 與 XDG。
        let e = env(Some("/x"), Some("/h"), Some("C:/Users/u/AppData/Roaming"), Some("C:/Users/u"));
        assert_eq!(config_dir(Os::Windows, &e), Some(PathBuf::from("C:/Users/u/AppData/Roaming/rs2-remake")));
        let e = env(None, Some("/h"), None, Some("C:/Users/u"));
        assert_eq!(config_dir(Os::Windows, &e), Some(PathBuf::from("C:/Users/u/AppData/Roaming/rs2-remake")));
        let e = env(None, Some("/h"), Some(""), None);
        assert_eq!(config_dir(Os::Windows, &e), None);
        // macOS：~/Library/Application Support，不看 XDG。
        let e = env(Some("/x"), Some("/Users/u"), None, None);
        assert_eq!(config_dir(Os::MacOs, &e), Some(PathBuf::from("/Users/u/Library/Application Support/rs2-remake")));
        assert_eq!(config_dir(Os::MacOs, &env(Some("/x"), None, None, None)), None);
    }

    #[test]
    fn save_into_unwritable_place_is_an_error_not_a_panic() {
        // 發行包在唯讀位置時，存設定要回傳錯誤讓上層印警告。這裡用「父層是檔案」模擬建不了目錄。
        let d = tmpdir("ro");
        std::fs::create_dir_all(&d).unwrap();
        let blocker = d.join("file");
        std::fs::write(&blocker, b"x").unwrap();
        assert!(Settings::default().save(&blocker.join("cfg")).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
