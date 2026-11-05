//! 命令列參數。解析不依賴 SDL，可單元測試。

use std::path::PathBuf;

pub const USAGE: &str = "\
用法：rs2-play --orig <原版目錄> [選項]

  --orig DIR            原版目錄（必要），啟動時驗全部檔案的 SHA-256
  --hd DIR              HD 素材資料夾（選用）；有給時預設用 HD 圖
  --no-hd               忽略 --hd，只用原版圖
  --start title|newgame|demo
                        起點，預設 title。newgame、demo 直接進第一關，第一關結束就離開（除錯與回歸用）
  --scale N             視窗倍率（1 至 8），預設讀設定檔，沒有就 3
  --original-bugs       不修正原版缺陷（Fixes::NONE）；預設全部修正
  --no-audio            不開音效裝置
  --wav-out FILE        --headless-steps 時把這段遊戲的音訊依遊戲時間寫成 WAV
  --config DIR          設定目錄（settings.txt）；預設 $XDG_CONFIG_HOME/rs2-remake
  --replay FILE         用收據的 sc 欄餵輸入（收據用完後改回鍵盤）
  --headless-steps N    不進視窗迴圈，經同一段步進與繪製程式跑 N 步後結束
  --hash-out FILE       每步一行 Dseg 的 SHA-256
  --shot-at N:FILE      第 N 步之後把畫面（含工具列）存成 PNG，可重複
  --tween on|off        覆寫補間開關的初值
  --counter on|off      覆寫幀計數開關的初值
  --toggle-at N:NAME    第 N 步之後切換開關，NAME 為 tween、counter 或 hd（測試用，可重複）
  --max-secs S          視窗模式跑 S 秒後自行結束（驗證用）
  --record FILE         視窗模式結束時寫出每步實際送進遊戲的掃描碼（JSONL，每步一行，可再用 --replay 重播）

視窗內：F1 補間、F2 幀計數、F3 原版圖與 HD、F4 倍率（2、3、4、5）、F10 工具列、F11 全螢幕。
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
}

#[derive(Clone, Debug)]
pub struct Args {
    pub orig: PathBuf,
    pub hd: Option<PathBuf>,
    pub no_hd: bool,
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
    pub counter: Option<bool>,
    pub toggles: Vec<(u64, ToggleName)>,
    pub max_secs: Option<f64>,
    pub record: Option<PathBuf>,
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
        orig: PathBuf::new(),
        hd: None,
        no_hd: false,
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
        counter: None,
        toggles: Vec::new(),
        max_secs: None,
        record: None,
    };
    let mut have_orig = false;
    let mut it = args.iter();
    macro_rules! val {
        ($name:expr) => {
            it.next().ok_or_else(|| format!("{} 缺參數", $name))
        };
    }
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "-h" | "--help" => return Err(USAGE.to_string()),
            "--orig" => {
                a.orig = PathBuf::from(val!("--orig")?);
                have_orig = true;
            }
            "--hd" => a.hd = Some(PathBuf::from(val!("--hd")?)),
            "--no-hd" => a.no_hd = true,
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
            "--counter" => a.counter = Some(onoff("--counter", val!("--counter")?)?),
            "--toggle-at" => {
                let (n, name) = step_and_rest("--toggle-at", val!("--toggle-at")?)?;
                let t = match name {
                    "tween" => ToggleName::Tween,
                    "counter" => ToggleName::Counter,
                    "hd" => ToggleName::Hd,
                    v => return Err(format!("--toggle-at 的 NAME 要是 tween、counter 或 hd，收到 {v}")),
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
            other => return Err(format!("不認得的參數 {other}\n\n{USAGE}")),
        }
    }
    if !have_orig {
        return Err(format!("缺 --orig\n\n{USAGE}"));
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
        assert_eq!(a.orig, PathBuf::from("/o"));
        assert_eq!(a.start, StartArg::Title);
        assert!(!a.original_bugs && !a.no_hd);
        assert!(a.hd.is_none() && a.scale.is_none() && a.replay.is_none());
    }

    #[test]
    fn full_set() {
        let a = p("--orig /o --hd /h --start demo --scale 4 --original-bugs --config /c --replay /r.jsonl \
                   --headless-steps 640 --hash-out /x --shot-at 50:/a.png --shot-at 120:/b.png \
                   --tween off --counter on --toggle-at 7:hd --max-secs 15 --record /rec.jsonl")
            .unwrap();
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
        assert!(p("--start demo").unwrap_err().contains("--orig"));
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
    }

    #[test]
    fn help_returns_usage() {
        assert!(p("--help").unwrap_err().contains("--orig"));
    }
}
