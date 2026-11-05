//! 本機設定：`settings.txt`，每行 `key=value`。讀不到用預設，壞掉的值略過並回報警告。
//!
//! | 鍵 | 值 |
//! |---|---|
//! | `tween`、`counter`、`hd`、`toolbar`、`feel` | `on` 或 `off` |
//! | `scale` | 1 到 8 |
//! | `mode` | `vga`、`ega`、`cga` 或 `mga`（顯示模式，F6）。讀不到或不認得就是 `vga` |
//! | `opt.<id>` | 遊戲選項（`options::OPTIONS`）。開關是 `on`、`off`，戰場大小是 `100`、`120`、`150` |
//! | `key.p1.up` 等 | 鍵盤對應，值是 `ui::text::KEYS` 的存檔名稱，空值是未綁定。欄位 `up down left right upleft upright downleft downright fire turn split` |
//! | `pad.fire` 等 | 手把按鍵，值是兩個按鈕 `A,RT`，`-` 是空。功能 `fire turn split pause confirm menu` |
//!
//! 舊版的檔（只有前六個鍵）照樣讀得進來，新欄位用預設。手把指派不存：裝置編號每次啟動都會變。

use crate::keymap::{Act, Btn, KeyMap, PadFn, PadMap};
use crate::mode::DisplayMode;
use crate::options::{self, Options, FEEL_INDEX, HIT_FEEDBACK_INDEX, INVINCIBLE_INDEX, OPTIONS, index_of};
use rs2_game::{FieldSize, Rules};
use crate::resources::{Env, Os, non_empty};
use crate::ui::text;
use std::io::Write;
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
    /// 工具列顯示。
    pub toolbar: bool,
    /// 玩家機預測與短按保留（F5）。關掉就是原版時序，模擬不受影響。
    pub feel: bool,
    /// 顯示模式（F6）：VGA、EGA、CGA、MGA。切換只換畫面，不影響模擬。
    pub mode: DisplayMode,
    /// 兩人的鍵盤對應。
    pub keys: KeyMap,
    /// 手把按鍵。
    pub pad: PadMap,
    /// 遊戲選項。`feel` 那一格不用，值在 `feel`。
    pub opts: Options,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            tween: true,
            counter: false,
            hd: true,
            scale: DEFAULT_SCALE,
            toolbar: true,
            feel: true,
            mode: DisplayMode::Vga,
            keys: KeyMap::default(),
            pad: PadMap::default(),
            opts: Options::default(),
        }
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

    /// 選項 `i` 的目前值。手感的值在 `feel`。
    pub fn opt_value(&self, i: usize) -> u8 {
        if i == FEEL_INDEX { self.feel as u8 } else { self.opts.get(i) }
    }

    pub fn set_opt_value(&mut self, i: usize, v: u8) {
        if i == FEEL_INDEX {
            self.feel = v != 0;
        } else {
            self.opts.set(i, v);
        }
    }

    /// 無敵開著。
    pub fn invincible(&self) -> bool {
        self.opts.get(INVINCIBLE_INDEX) != 0
    }

    /// 打擊回饋開著（只在 HD 畫面生效）。
    pub fn hit_feedback(&self) -> bool {
        self.opts.get(HIT_FEEDBACK_INDEX) != 0
    }

    /// 戰場大小選項的值（0、1、2 對應 100%、120%、150%）。沒實作算 100%。
    pub fn field_size(&self) -> FieldSize {
        let i = index_of("field_size").expect("選項表沒有 field_size");
        if OPTIONS[i].implemented { FieldSize::from_index(self.opt_value(i)) } else { FieldSize::Normal }
    }

    /// 交給核心的強化選項。沒實作的選項一律算關，把 `options.rs` 那一列的 `implemented` 打開才生效。
    ///
    /// 戰場大小只支援 VGA、EGA：顯示模式是 CGA、MGA 時一律 100%（進關時讀，所以關內切換模式不影響這一關）。
    pub fn rules(&self) -> Rules {
        let on = |id: &str| {
            let i = index_of(id).expect("選項表沒有這個 id");
            OPTIONS[i].implemented && self.opt_value(i) != 0
        };
        Rules {
            field: if self.mode.is_mono() { FieldSize::Normal } else { self.field_size() },
            wave_charge: on("wave_charge"),
            auto_fire: on("auto_fire"),
            force_pod: on("force_pod"),
            fine_step: on("fine_step"),
            ai_aim: on("ai_aim"),
            ai_fire: on("ai_fire"),
            ai_react: on("ai_react"),
            ai_boss: on("ai_boss"),
        }
    }

    /// 有「已實作而且改變規則」的選項開著。沒實作的選項值再怎麼寫都不算，因為它不會生效。
    pub fn rules_enhanced(&self) -> bool {
        OPTIONS.iter().enumerate().any(|(i, o)| o.implemented && o.changes_rules && o.is_active(self.opt_value(i)))
    }

    pub fn to_text(&self) -> String {
        let mut s = format!(
            "# RS-2 remake 設定\ntween={}\ncounter={}\nhd={}\nscale={}\ntoolbar={}\nfeel={}\nmode={}\n",
            b(self.tween),
            b(self.counter),
            b(self.hd),
            self.scale,
            b(self.toolbar),
            b(self.feel),
            self.mode.code()
        );
        s.push_str("# 遊戲選項（標開發中的選項，值先存起來，這一版還沒有實作）\n");
        for (i, o) in OPTIONS.iter().enumerate() {
            if i != FEEL_INDEX {
                s.push_str(&format!("opt.{}={}\n", o.id, o.save_value(self.opts.get(i))));
            }
        }
        s.push_str("# 鍵盤（空值是未綁定）\n");
        s.push_str(&self.keys.to_lines());
        s.push_str("# 手把按鍵（-是空欄位）\n");
        s.push_str(&self.pad.to_lines());
        s
    }

    /// 解析文字。壞掉的行或值略過並寫進警告，其餘欄位維持預設。
    pub fn parse(src: &str) -> (Settings, Vec<String>) {
        let mut s = Settings::default();
        let mut warn = Vec::new();
        let mut key_entries: Vec<(usize, Act, Option<sdl2::keyboard::Scancode>)> = Vec::new();
        let mut pad_entries: Vec<(PadFn, [Option<Btn>; 2])> = Vec::new();
        for (i, raw) in src.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                warn.push(format!("第 {} 行沒有 =，略過：{line}", i + 1));
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            let n = i + 1;
            let mut flag = |slot: &mut bool| match parse_bool(v) {
                Some(x) => *slot = x,
                None => warn.push(format!("第 {n} 行 {k} 的值 {v:?} 不是 on 或 off，略過")),
            };
            match k {
                "tween" => flag(&mut s.tween),
                "counter" => flag(&mut s.counter),
                "hd" => flag(&mut s.hd),
                "toolbar" => flag(&mut s.toolbar),
                "feel" => flag(&mut s.feel),
                "scale" => match v.parse::<u32>() {
                    Ok(x) if (1..=8).contains(&x) => s.scale = x,
                    _ => warn.push(format!("第 {n} 行 scale 的值 {v:?} 不在 1 到 8，略過")),
                },
                "mode" => match DisplayMode::from_code(v) {
                    Some(m) => s.mode = m,
                    None => warn.push(format!("第 {n} 行 mode 的值 {v:?} 不是 vga、ega、cga 或 mga，用 vga")),
                },
                _ if k.starts_with("opt.") => match options::index_of(&k[4..]) {
                    Some(idx) => match OPTIONS[idx].parse_value(v) {
                        Some(x) => s.set_opt_value(idx, x),
                        None => warn.push(format!("第 {n} 行 {k} 的值 {v:?} 不合法，略過")),
                    },
                    None => warn.push(format!("第 {n} 行的選項 {k:?} 不認得，略過")),
                },
                _ if k.starts_with("key.") => match KeyMap::parse_slot(&k[4..]) {
                    Some((p, a)) => {
                        if v.is_empty() {
                            key_entries.push((p, a, None));
                        } else {
                            match text::key_from_name(v) {
                                Some(key) => key_entries.push((p, a, Some(key))),
                                None => warn.push(format!("第 {n} 行 {k} 的鍵 {v:?} 不認得或不能綁定，略過")),
                            }
                        }
                    }
                    None => warn.push(format!("第 {n} 行的鍵盤欄位 {k:?} 不認得，略過")),
                },
                _ if k.starts_with("pad.") => match PadFn::from_save_name(&k[4..]) {
                    Some(f) => match PadMap::parse_value(v) {
                        Ok(bs) => pad_entries.push((f, bs)),
                        Err(e) => warn.push(format!("第 {n} 行 {k}：{e}，略過")),
                    },
                    None => warn.push(format!("第 {n} 行的手把功能 {k:?} 不認得，略過")),
                },
                _ => warn.push(format!("第 {n} 行的鍵 {k:?} 不認得，略過")),
            }
        }
        let (keys, kw) = KeyMap::from_entries(&key_entries);
        let (pad, pw) = PadMap::from_entries(&pad_entries);
        s.keys = keys;
        s.pad = pad;
        warn.extend(kw);
        warn.extend(pw);
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

    /// 原子寫入：先寫暫存檔並 `sync_all`，再改名蓋過原檔，中途被中斷也不會留下寫一半的設定。
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{FILE_NAME}.tmp"));
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(self.to_text().as_bytes())?;
            f.sync_all()?;
        }
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
    use sdl2::keyboard::Scancode as K;

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
        assert!(!s.invincible() && !s.rules_enhanced());
        assert_eq!(s.keys, KeyMap::default());
        assert_eq!(s.pad, PadMap::default());
    }

    #[test]
    fn text_roundtrip_of_the_basic_switches() {
        for tween in [false, true] {
            for counter in [false, true] {
                for hd in [false, true] {
                    for toolbar in [false, true] {
                        for scale in [1, 2, 3, 4, 5, 8] {
                            for feel in [false, true] {
                                let s = Settings { tween, counter, hd, scale, toolbar, feel, ..Settings::default() };
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
    fn text_roundtrip_of_options_keys_and_pad() {
        let mut s = Settings::default();
        s.set_opt_value(0, 2);
        s.set_opt_value(1, 1);
        s.set_opt_value(options::INVINCIBLE_INDEX, 1);
        s.set_opt_value(4, 0);
        s.feel = false;
        s.keys.clear(0, Act::Up);
        s.keys.bind(0, Act::Fire, K::Space).unwrap();
        s.keys.bind(1, Act::Split, K::Kp5).unwrap();
        s.pad.clear(PadFn::Fire, 0);
        s.pad.bind(PadFn::Fire, 0, Btn::L3).unwrap();
        let (back, warn) = Settings::parse(&s.to_text());
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(back, s);
        assert_eq!(back.opt_value(0), 2);
        assert!(back.invincible() && back.rules_enhanced());
        // 文字裡每個選項都有一行，手感只寫一次（舊的 feel= 鍵）。
        let t = s.to_text();
        for (i, o) in OPTIONS.iter().enumerate() {
            let want = format!("opt.{}=", o.id);
            assert_eq!(t.contains(&want), i != FEEL_INDEX, "{}", o.id);
        }
        assert!(t.contains("key.p1.up=\n"), "未綁定寫成空值");
    }

    #[test]
    fn display_mode_roundtrips_through_text_and_file() {
        for mode in DisplayMode::ALL {
            let s = Settings { mode, ..Settings::default() };
            assert!(s.to_text().contains(&format!("mode={}\n", mode.code())));
            let (back, warn) = Settings::parse(&s.to_text());
            assert_eq!(back.mode, mode);
            assert!(warn.is_empty(), "{warn:?}");
            let d = tmpdir(&format!("mode-{}", mode.code()));
            s.save(&d).unwrap();
            let (loaded, w) = Settings::load(&d);
            assert_eq!(loaded.mode, mode, "原子寫入後讀回");
            assert!(w.is_empty() && !d.join("settings.txt.tmp").exists());
            std::fs::remove_dir_all(&d).unwrap();
        }
        assert_eq!(Settings::default().mode, DisplayMode::Vga, "預設是 VGA");
    }

    #[test]
    fn unknown_or_missing_mode_falls_back_to_vga_with_a_warning() {
        for bad in ["mode=hercules", "mode=", "mode=VGA2", "mode=1"] {
            let (s, w) = Settings::parse(&format!("counter=on\n{bad}\n"));
            assert_eq!(s.mode, DisplayMode::Vga, "{bad}");
            assert!(s.counter, "{bad}：其他欄位照收");
            assert!(w.iter().any(|m| m.contains("mode")), "{bad} 要有警告：{w:?}");
        }
        // 不分大小寫；舊檔沒有 mode 這一行，沒有警告。
        assert_eq!(Settings::parse("mode=CGA\n").0.mode, DisplayMode::Cga);
        let (s, w) = Settings::parse("tween=off\n");
        assert_eq!(s.mode, DisplayMode::Vga);
        assert!(w.is_empty());
    }

    #[test]
    fn old_files_without_the_new_fields_still_load() {
        let old = "# RS-2 remake 設定\ntween=off\ncounter=on\nhd=off\nscale=4\ntoolbar=off\nfeel=off\n";
        let (s, w) = Settings::parse(old);
        assert!(w.is_empty(), "{w:?}");
        assert!(!s.tween && s.counter && !s.hd && !s.toolbar && !s.feel);
        assert_eq!(s.scale, 4);
        assert_eq!(s.keys, KeyMap::default());
        assert_eq!(s.pad, PadMap::default());
        assert_eq!(s.opts, Options::default());
    }

    #[test]
    fn rules_follow_implemented_options_only() {
        let mut st = Settings::default();
        assert_eq!(st.rules(), Rules::ORIGINAL, "預設全關");
        for (i, o) in OPTIONS.iter().enumerate() {
            if o.kind == options::Kind::Toggle && o.changes_rules {
                st.set_opt_value(i, 1);
            }
        }
        let want = |id: &str| OPTIONS[index_of(id).unwrap()].implemented;
        let r = st.rules();
        assert_eq!(r.wave_charge, want("wave_charge"));
        assert_eq!(r.ai_boss, want("ai_boss"));
    }

    #[test]
    fn only_implemented_options_count_as_enhanced_rules() {
        let mut s = Settings::default();
        // 沒實作的選項，檔案裡寫成開也不算強化規則，因為它不會生效。
        for (i, o) in OPTIONS.iter().enumerate() {
            if !o.implemented {
                s.set_opt_value(i, 1);
            }
        }
        assert!(!s.rules_enhanced());
        assert_eq!(s.rules(), Rules::ORIGINAL, "沒實作的選項不交給核心");
        s.set_opt_value(options::INVINCIBLE_INDEX, 1);
        assert!(s.rules_enhanced() && s.invincible());
        s.set_opt_value(options::INVINCIBLE_INDEX, 0);
        assert!(!s.rules_enhanced());
        // 手感不改變規則。
        s.feel = false;
        assert!(!s.rules_enhanced());
    }

    #[test]
    fn file_roundtrip_and_missing_file() {
        let d = tmpdir("file");
        let (s, w) = Settings::load(&d);
        assert_eq!(s, Settings::default());
        assert!(w.is_empty(), "檔案不存在不算警告：{w:?}");
        let mut want = Settings { tween: false, counter: true, hd: false, scale: 5, toolbar: false, feel: false, ..Settings::default() };
        want.set_opt_value(options::INVINCIBLE_INDEX, 1);
        want.keys.clear(1, Act::Up);
        want.save(&d).unwrap();
        let (got, w) = Settings::load(&d);
        assert_eq!(got, want);
        assert!(w.is_empty());
        assert!(!d.join("settings.txt.tmp").exists(), "原子寫入不留暫存檔");
        // 再存一次（覆蓋舊檔）。
        want.counter = false;
        want.save(&d).unwrap();
        assert_eq!(Settings::load(&d).0, want);
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
    fn bad_new_fields_are_skipped_with_warnings_and_keep_the_rest() {
        let text = "opt.invincible=on\nopt.field_size=130\nopt.nope=on\nkey.p1.up=Banana\nkey.p1.down=W\nkey.p2.down=W\nkey.p3.up=A\nkey.p1.fire=F1\n\
                    pad.fire=A\npad.turn=B,Q\npad.pause=Start,-\npad.dance=A,B\npad.confirm=Start,-\n";
        let (s, w) = Settings::parse(text);
        assert!(s.invincible());
        assert_eq!(s.opt_value(0), 0, "壞值維持預設");
        assert_eq!(s.keys.get(0, Act::Up), Some(K::Up), "不認得的鍵名略過，欄位用預設");
        assert_eq!(s.keys.get(0, Act::Down), Some(K::W));
        assert_eq!(s.keys.get(1, Act::Down), None, "重複的鍵略過");
        assert_eq!(s.keys.get(0, Act::Fire), Some(K::RShift), "F1 不能綁，欄位用預設");
        assert_eq!(s.pad.get(PadFn::Pause, 0), Some(Btn::Start));
        assert_eq!(s.pad.get(PadFn::Confirm, 0), None, "Start 重複略過");
        assert_eq!(s.pad.get(PadFn::Fire, 0), Some(Btn::A), "壞行略過，功能用預設");
        for needle in ["opt.field_size", "opt.nope", "Banana", "p3.up", "F1", "pad.fire", "pad.turn", "pad.dance", "Start"] {
            assert!(w.iter().any(|m| m.contains(needle)), "缺 {needle} 的警告：{w:?}");
        }
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
