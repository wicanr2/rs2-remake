//! 命令列參數。解析不依賴 SDL，可單元測試。

use crate::mode::DisplayMode;
use crate::poster::PosterMode;
use crate::ui::panel::Page;
use std::path::PathBuf;

/// 版本字串。發行包建置時由環境變數 `RS2_VERSION` 帶入（例如 `v.0.9.0-20261105`），沒有就用 crate 版本。
pub const VERSION: &str = match option_env!("RS2_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

pub const USAGE: &str = "\
用法：rs2-play [選項]

  --orig DIR            原版目錄，啟動時驗全部檔案的 SHA-256。
                        不給時依序找：環境變數 RS2_DATA、發行包內建位置
                        （AppImage 的 $APPDIR/usr/share/rs2/original、Windows 執行檔旁的 data/original、
                        macOS 的 .app/Contents/Resources/original）
  --hd DIR              HD 素材資料夾；有給時預設用 HD 圖。不給時依序找：
                        RS2_DATA/hd、發行包內建位置的 hd，找到就預設用 HD 圖
  --no-hd               忽略 --hd 與自動找到的 HD，只用原版圖
  --version             印出版本後結束

環境變數 RS2_DATA：資料根（底下有 original/ 與 hd/），或直接指向原版目錄。
  --start title|newgame|demo
                        起點，預設 title。newgame、demo 直接進第一關，第一關結束就離開（除錯與回歸用）
  --scale N             視窗倍率（1 至 8），預設讀設定檔，沒有就 3
  --original-bugs       不修正原版缺陷（Fixes::NONE）；預設全部修正
  --no-audio            不開音效裝置
  --wav-out FILE        --headless-steps 時把這段遊戲的音訊依遊戲時間寫成 WAV
  --config DIR          設定目錄（settings.txt）；預設 Linux 為 $XDG_CONFIG_HOME/rs2-remake（沒有就 ~/.config/rs2-remake），
                        Windows 為 %APPDATA%\\rs2-remake，macOS 為 ~/Library/Application Support/rs2-remake
  --replay FILE         用收據的 sc 欄餵輸入（收據用完後改回鍵盤）
  --headless-steps N    不進視窗迴圈，經同一段步進與繪製程式跑 N 步後結束
  --hash-out FILE       每步一行 Dseg 的 SHA-256
  --shot-at N:FILE      第 N 步之後把畫面（含工具列）存成 PNG，可重複
  --tween on|off        覆寫補間開關的初值
  --feel on|off         覆寫玩家機預測與短按保留的初值（F5）
  --feedback on|off     覆寫打擊回饋選項的初值（只在 HD 畫面生效）
  --touch               畫觸控控制鈕並用手機版工具列（Android 預設，桌面版驗證用）
  --counter on|off      覆寫幀計數開關的初值
  --poster off|classic|modern
                        紀念海報，覆寫設定檔；可在顯示設定切換，預設 off
  --mode vga|ega|cga|mga
                        起始的顯示模式，覆寫設定檔（驗證與截圖用）。cga、mga 需要單色圖庫，缺檔時退回 vga
  --toggle-at N:NAME    第 N 步之後切換開關，NAME 為 tween、counter、hd、feel、feedback 或 mode（mode 是換下一個顯示模式，測試用，可重複）
  --max-secs S          視窗模式跑 S 秒後自行結束（驗證用）
  --record FILE         視窗模式結束時寫出每步實際送進遊戲的掃描碼（JSONL，每步一行，可再用 --replay 重播）
  --open-panel 頁名      開設定畫面到指定頁（驗證用）：手把、手把按鍵、鍵盤、遊戲、顯示，或 pad、padkeys、keyboard、game、display；
                        「無」或 none 是關閉。第一次出現的 --shot-at 步數到時才開，可重複，第 k 個 --open-panel
                        配第 k 個 --shot-at（設定畫面開著時前端暫停，步數不再增加，後面的圖不等步數）
  --dp X                手機版每 dp 的像素數（驗證用，預設讀螢幕 DPI，讀不到用 2）
  --fake-pads           列出兩支假的手把與一支不支援的搖桿，沒有實體手把也能看手把頁的版面（驗證用）

視窗內：F1 補間、F2 幀計數、F3 原貌／HD、F4 倍率（2、3、4、5）、F5 手感（新或原版時序）、F6 顯示模式（VGA、EGA、CGA、MGA）、
F8 設定畫面換頁、F10 設定畫面、F11 全螢幕。工具列的按鈕與設定畫面見規格 005。
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartArg {
    Title,
    NewGame,
    Demo,
}

/// 測試用的開關切換目標。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToggleName {
    Tween,
    Counter,
    Hd,
    Feel,
    /// 換下一個顯示模式（VGA、EGA、CGA、MGA 輪流）。
    Mode,
    /// 打擊回饋選項（規格 005）。
    Feedback,
}

#[derive(Clone, Debug)]
pub struct Args {
    /// 命令列指定的原版目錄；`None` 時由 `resources::resolve` 依序尋找。
    pub orig: Option<PathBuf>,
    pub hd: Option<PathBuf>,
    pub no_hd: bool,
    pub version: bool,
    pub start: StartArg,
    pub scale: Option<u32>,
    pub original_bugs: bool,
    pub no_audio: bool,
    pub wav_out: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub replay: Option<PathBuf>,
    pub headless_steps: Option<u64>,
    pub hash_out: Option<PathBuf>,
    pub shots: Vec<(u64, PathBuf)>,
    pub tween: Option<bool>,
    pub feel: Option<bool>,
    /// 起始的顯示模式，覆寫設定檔。
    pub mode: Option<DisplayMode>,
    /// 覆寫打擊回饋選項的初值。
    pub feedback: Option<bool>,
    pub poster: Option<PosterMode>,
    /// 畫觸控控制鈕並用手機版工具列。Android 一律開著，桌面版給驗證用。
    pub touch: bool,
    pub counter: Option<bool>,
    pub toggles: Vec<(u64, ToggleName)>,
    pub max_secs: Option<f64>,
    pub record: Option<PathBuf>,
    /// 驗證用：第 k 個值配第 k 個 `--shot-at`，存圖時設定畫面開在那一頁，`None` 是關閉。
    pub open_panels: Vec<Option<Page>>,
    /// 驗證用：手機版每 dp 的像素數。
    pub dp: Option<f32>,
    /// 驗證用：沒有實體手把也列出兩支假的手把與一支不支援的搖桿，看手把頁的版面。
    pub fake_pads: bool,
}

fn onoff(flag: &str, v: &str) -> Result<bool, String> {
    match v {
        "on" | "1" | "true" => Ok(true),
        "off" | "0" | "false" => Ok(false),
        _ => Err(format!("{flag} 要是 on 或 off，收到 {v}")),
    }
}

fn step_and_rest<'a>(flag: &str, v: &'a str) -> Result<(u64, &'a str), String> {
    let (n, rest) = v.split_once(':').ok_or_else(|| format!("{flag} 的格式是 步數:值，收到 {v}"))?;
    let n: u64 = n.parse().map_err(|_| format!("{flag} 的步數不是數字：{n}"))?;
    if rest.is_empty() {
        return Err(format!("{flag} 缺值：{v}"));
    }
    Ok((n, rest))
}

/// 解析 `args`（不含程式名稱）。`--help` 回傳 `Err(USAGE)`。
pub fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        orig: None,
        hd: None,
        no_hd: false,
        version: false,
        start: StartArg::Title,
        scale: None,
        original_bugs: false,
        no_audio: false,
        wav_out: None,
        config: None,
        replay: None,
        headless_steps: None,
        hash_out: None,
        shots: Vec::new(),
        tween: None,
        feel: None,
        mode: None,
        feedback: None,
        poster: None,
        touch: cfg!(target_os = "android"),
        counter: None,
        toggles: Vec::new(),
        max_secs: None,
        record: None,
        open_panels: Vec::new(),
        dp: None,
        fake_pads: false,
    };
    let mut it = args.iter();
    macro_rules! val {
        ($name:expr) => {
            it.next().ok_or_else(|| format!("{} 缺參數", $name))
        };
    }
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "-h" | "--help" => return Err(USAGE.to_string()),
            "--orig" => a.orig = Some(PathBuf::from(val!("--orig")?)),
            "--hd" => a.hd = Some(PathBuf::from(val!("--hd")?)),
            "--no-hd" => a.no_hd = true,
            "--version" => a.version = true,
            "--start" => {
                a.start = match val!("--start")?.as_str() {
                    "title" => StartArg::Title,
                    "newgame" => StartArg::NewGame,
                    "demo" => StartArg::Demo,
                    v => return Err(format!("--start 要是 title、newgame 或 demo，收到 {v}")),
                }
            }
            "--scale" => {
                let v = val!("--scale")?;
                let n: u32 = v.parse().map_err(|_| format!("--scale 不是數字：{v}"))?;
                if !(1..=8).contains(&n) {
                    return Err(format!("--scale 要在 1 到 8 之間，收到 {n}"));
                }
                a.scale = Some(n);
            }
            "--original-bugs" => a.original_bugs = true,
            "--no-audio" => a.no_audio = true,
            "--wav-out" => a.wav_out = Some(PathBuf::from(val!("--wav-out")?)),
            "--config" => a.config = Some(PathBuf::from(val!("--config")?)),
            "--replay" => a.replay = Some(PathBuf::from(val!("--replay")?)),
            "--headless-steps" => {
                let v = val!("--headless-steps")?;
                a.headless_steps = Some(v.parse().map_err(|_| format!("--headless-steps 不是數字：{v}"))?);
            }
            "--hash-out" => a.hash_out = Some(PathBuf::from(val!("--hash-out")?)),
            "--shot-at" => {
                let (n, path) = step_and_rest("--shot-at", val!("--shot-at")?)?;
                a.shots.push((n, PathBuf::from(path)));
            }
            "--tween" => a.tween = Some(onoff("--tween", val!("--tween")?)?),
            "--feel" => a.feel = Some(onoff("--feel", val!("--feel")?)?),
            "--mode" => {
                let v = val!("--mode")?;
                a.mode = Some(DisplayMode::from_code(v).ok_or_else(|| format!("--mode 要是 vga、ega、cga 或 mga，收到 {v}"))?);
            }
            "--feedback" => a.feedback = Some(onoff("--feedback", val!("--feedback")?)?),
            "--poster" => {
                let v = val!("--poster")?;
                a.poster = Some(PosterMode::from_code(v).ok_or_else(|| format!("--poster 要是 off、classic 或 modern，收到 {v}"))?);
            }
            "--touch" => a.touch = true,
            "--counter" => a.counter = Some(onoff("--counter", val!("--counter")?)?),
            "--toggle-at" => {
                let (n, name) = step_and_rest("--toggle-at", val!("--toggle-at")?)?;
                let t = match name {
                    "tween" => ToggleName::Tween,
                    "counter" => ToggleName::Counter,
                    "hd" => ToggleName::Hd,
                    "feel" => ToggleName::Feel,
                    "mode" => ToggleName::Mode,
                    "feedback" => ToggleName::Feedback,
                    v => return Err(format!("--toggle-at 的 NAME 要是 tween、counter、hd、feel、feedback 或 mode，收到 {v}")),
                };
                a.toggles.push((n, t));
            }
            "--max-secs" => {
                let v = val!("--max-secs")?;
                let s: f64 = v.parse().map_err(|_| format!("--max-secs 不是數字：{v}"))?;
                if !(s > 0.0 && s.is_finite()) {
                    return Err(format!("--max-secs 要大於 0，收到 {v}"));
                }
                a.max_secs = Some(s);
            }
            "--record" => a.record = Some(PathBuf::from(val!("--record")?)),
            "--open-panel" => {
                let v = val!("--open-panel")?;
                if v == "無" || v.eq_ignore_ascii_case("none") {
                    a.open_panels.push(None);
                } else {
                    match Page::from_name(v) {
                        Some(p) => a.open_panels.push(Some(p)),
                        None => return Err(format!("--open-panel 的頁名要是 手把、手把按鍵、鍵盤、遊戲、顯示（或 none），收到 {v}")),
                    }
                }
            }
            "--fake-pads" => a.fake_pads = true,
            "--dp" => {
                let v = val!("--dp")?;
                let d: f32 = v.parse().map_err(|_| format!("--dp 不是數字：{v}"))?;
                if !(0.5..=8.0).contains(&d) {
                    return Err(format!("--dp 要在 0.5 到 8 之間，收到 {v}"));
                }
                a.dp = Some(d);
            }
            other => return Err(format!("不認得的參數 {other}\n\n{USAGE}")),
        }
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Args, String> {
        let v: Vec<String> = s.split_whitespace().map(String::from).collect();
        parse(&v)
    }

    #[test]
    fn minimal_and_defaults() {
        let a = p("--orig /o").unwrap();
        assert_eq!(a.orig, Some(PathBuf::from("/o")));
        assert_eq!(a.start, StartArg::Title);
        assert!(!a.original_bugs && !a.no_hd && !a.version);
        assert!(a.hd.is_none() && a.scale.is_none() && a.replay.is_none());
    }

    #[test]
    fn orig_is_optional() {
        // 發行包直接雙擊不帶參數；原版目錄由 resources::resolve 找。
        let a = p("").unwrap();
        assert!(a.orig.is_none() && a.hd.is_none());
        let a = p("--start demo --no-hd").unwrap();
        assert!(a.orig.is_none() && a.no_hd);
        assert!(p("--version").unwrap().version);
    }

    #[test]
    fn version_string_is_not_empty() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn full_set() {
        let a = p("--orig /o --hd /h --start demo --scale 4 --original-bugs --config /c --replay /r.jsonl \
                   --headless-steps 640 --hash-out /x --shot-at 50:/a.png --shot-at 120:/b.png \
                   --tween off --counter on --toggle-at 7:hd --max-secs 15 --record /rec.jsonl")
            .unwrap();
        assert_eq!(a.orig, Some(PathBuf::from("/o")));
        assert_eq!(a.record, Some(PathBuf::from("/rec.jsonl")));
        assert_eq!(a.hd, Some(PathBuf::from("/h")));
        assert_eq!(a.start, StartArg::Demo);
        assert_eq!(a.scale, Some(4));
        assert!(a.original_bugs);
        assert_eq!(a.headless_steps, Some(640));
        assert_eq!(a.shots, vec![(50, PathBuf::from("/a.png")), (120, PathBuf::from("/b.png"))]);
        assert_eq!(a.tween, Some(false));
        assert_eq!(a.counter, Some(true));
        assert_eq!(a.toggles, vec![(7, ToggleName::Hd)]);
        assert_eq!(a.max_secs, Some(15.0));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(p("--start menu").is_err());
        assert!(p("--orig /o --scale 0").is_err());
        assert!(p("--orig /o --scale 9").is_err());
        assert!(p("--orig /o --scale x").is_err());
        assert!(p("--orig /o --start menu").is_err());
        assert!(p("--orig /o --shot-at 50").is_err());
        assert!(p("--orig /o --shot-at x:/a.png").is_err());
        assert!(p("--orig /o --tween maybe").is_err());
        assert!(p("--orig /o --toggle-at 3:fog").is_err());
        assert!(p("--orig /o --bogus").is_err());
        assert!(p("--orig").is_err());
        assert!(p("--orig /o --max-secs 0").is_err());
        assert!(p("--open-panel").is_err());
        assert!(p("--open-panel 不存在的頁").is_err());
        assert!(p("--dp 0").is_err() && p("--dp x").is_err() && p("--dp 9").is_err());
    }

    #[test]
    fn open_panel_and_dp_for_validation() {
        let a = p("--open-panel 手把 --open-panel 鍵盤 --open-panel none --open-panel game --dp 2.5 --touch").unwrap();
        assert_eq!(a.open_panels, vec![Some(Page::Pad), Some(Page::Keyboard), None, Some(Page::Game)]);
        assert_eq!(a.dp, Some(2.5));
        assert!(a.touch);
        let a = p("--open-panel 手把按鍵 --open-panel 顯示 --open-panel 無").unwrap();
        assert_eq!(a.open_panels, vec![Some(Page::PadKeys), Some(Page::Display), None]);
        assert!(p("--orig /o").unwrap().open_panels.is_empty());
    }

    #[test]
    fn mode_flag_and_mode_toggle() {
        assert_eq!(p("--orig /o").unwrap().mode, None, "不給就讀設定檔");
        for m in DisplayMode::ALL {
            assert_eq!(p(&format!("--mode {}", m.code())).unwrap().mode, Some(m));
        }
        assert_eq!(p("--mode CGA").unwrap().mode, Some(DisplayMode::Cga), "不分大小寫");
        let a = p("--mode mga --headless-steps 10 --shot-at 5:/a.png --toggle-at 3:mode --toggle-at 7:mode").unwrap();
        assert_eq!(a.mode, Some(DisplayMode::Mga));
        assert_eq!(a.toggles, vec![(3, ToggleName::Mode), (7, ToggleName::Mode)]);
        assert_eq!(a.shots, vec![(5, PathBuf::from("/a.png"))], "--mode 可以搭配 --shot-at");
        for bad in ["--mode", "--mode hercules", "--mode 4", "--mode vga2"] {
            assert!(p(bad).is_err(), "{bad}");
        }
        assert!(USAGE.contains("--mode") && USAGE.contains("F6"));
    }

    #[test]
    fn help_returns_usage() {
        assert!(p("--help").unwrap_err().contains("--orig"));
    }
}
