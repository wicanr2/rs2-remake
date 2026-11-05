//! 無視窗跑重播：輸入檔驗證、與 dosgolem 同格式的逐幀 JSONL、與收據比對。
//! 欄位定義見 `docs/re/10-dynamic-receipts.md` 第 8.3 節，順序與 `cmd/rs2dyn/trace_full.go` 相同。
//! 全流程對拍（等待點收據）在 [`wait`]。

pub mod mirror;
pub mod mono;
pub mod wait;

use mirror::{Dseg, Mirror, a};
use rs2_core::{Game, GameFiles, State, Tables};
use rs2_formats::{EXPECTED, UNPACKED_SHA256, check_hash, unpack_s_exe};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

pub struct Inputs {
    /// 解包後的 `S.EXE` 映像。
    pub image: Vec<u8>,
    pub dm1: Vec<u8>,
    /// 規格 002 列的全部檔案（`EXPECTED`），含 `S.EXE` 的原始位元組，鍵是檔名。
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Inputs {
    /// 原版映像的資料段對應。
    pub fn mirror(&self) -> Mirror {
        Mirror::new(&self.image)
    }

    /// 正式遊戲的開機狀態（標題入口，載入展示錄影）。映像不是已知的原版就 `panic`。
    pub fn boot_state(&self) -> State {
        let tables = Tables::parse(&self.image).expect("解包映像已驗過雜湊");
        rs2_core::boot_state(std::rc::Rc::new(tables), &self.dm1)
    }

    /// 交給 `Game::from_state` 的檔案：解包映像、兩份展示錄影、第三關地圖。
    pub fn game_files(&self) -> GameFiles {
        let get = |n: &str| self.files.get(n).cloned().unwrap_or_default();
        GameFiles { image: self.image.clone(), dm1: get("DM1.LCK"), dm2: get("DM2.LCK"), scn: get("SHT.SCN"), scr: get("SHT.SCR") }
    }
}

/// 讀原版目錄並驗雜湊。`EXPECTED` 的每個檔案都要在，任何一個不符就回傳錯誤，錯誤訊息寫出檔名與預期雜湊。
pub fn load_inputs(dir: &Path) -> Result<Inputs, String> {
    let mut files = BTreeMap::new();
    for (name, want) in EXPECTED {
        let data = std::fs::read(dir.join(name)).map_err(|e| format!("讀不到 {name}：{e}"))?;
        check_hash(name, &data, want)?;
        files.insert(name.to_string(), data);
    }
    let image = unpack_s_exe(&files["S.EXE"]).map_err(|e| e.to_string())?;
    check_hash("解包映像", &image, UNPACKED_SHA256)?;
    let dm1 = files["DM1.LCK"].clone();
    Ok(Inputs { image, dm1, files })
}

fn kv(s: &mut String, k: &str, v: u32) {
    let _ = write!(s, "\"{k}\":{v}");
}

const PLAYER1: [(&str, u32, bool); 14] = [
    ("col", 0x1DACB, true),
    ("row", 0x1DAC9, true),
    ("x", 0x1DAC7, true),
    ("y", 0x1DAC5, true),
    ("img", 0x1DABD, true),
    ("hit", 0x1DAC0, false),
    ("inv", 0x1DAC1, false),
    ("die", 0x1DAC3, false),
    ("hp", 0x1CFDE, true),
    ("wpn", 0x1DAA9, true),
    ("pod1", 0x1DA8D, false),
    ("pod2", 0x1DA8E, false),
    ("face", 0x1DA8F, true),
    ("chg", 0x1DA8B, true),
];

const PLAYER2: [(&str, u32, bool); 14] = [
    ("col", 0x1DB39, true),
    ("row", 0x1DB37, true),
    ("x", 0x1DB21, true),
    ("y", 0x1DB1F, true),
    ("img", 0x1DB17, true),
    ("hit", 0x1DB1A, false),
    ("inv", 0x1DB1B, false),
    ("die", 0x1DB1D, false),
    ("hp", 0x1CFE0, true),
    ("wpn", 0x1DB0B, true),
    ("pod1", 0x1DAEF, false),
    ("pod2", 0x1DAF0, false),
    ("face", 0x1DAF1, true),
    ("chg", 0x1DAED, true),
];

fn player(s: &mut String, d: &Dseg, fields: &[(&str, u32, bool)]) {
    s.push('{');
    for (i, (k, addr, word)) in fields.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let v = if *word { d.r16(a(*addr)) as u32 } else { d.r8(a(*addr)) as u32 };
        kv(s, k, v);
    }
    s.push('}');
}

fn words(s: &mut String, g: &Dseg, key: &str, off: u16, n: u16) {
    let _ = write!(s, "\"{key}\":[");
    for i in 0..n {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{}", g.r16(off.wrapping_add(i * 2)));
    }
    s.push(']');
}

/// 指標表與每個非 0 指標指向的記錄。記錄從指標讀，不是從槽號算。
fn pool(s: &mut String, g: &Dseg, tkey: &str, rkey: &str, tbl: u16, slots: u16, wc: u16) {
    words(s, g, tkey, tbl, slots);
    let _ = write!(s, ",\"{rkey}\":[");
    let mut first = true;
    for i in 0..slots {
        let p = g.r16(tbl.wrapping_add(i * 2));
        if p == 0 {
            continue;
        }
        if !first {
            s.push(',');
        }
        first = false;
        let _ = write!(s, "{{\"s\":{i},\"w\":[");
        for k in 0..wc {
            if k > 0 {
                s.push(',');
            }
            let _ = write!(s, "{}", g.r16(p.wrapping_add(k * 2)));
        }
        s.push_str("]}");
    }
    s.push(']');
}

/// 一筆逐幀紀錄，不含 `emu`。`emu` 是模擬器專屬欄位，比對時忽略。
pub fn row_json(m: &Mirror, g: &Game, n: u64, pt: u32, sc: &[u8]) -> String {
    let dump = m.dump(g.state());
    let d = &dump;
    let g = d;
    let mut s = String::with_capacity(2048);
    s.push('{');
    let _ = write!(s, "\"n\":{n},");
    kv(&mut s, "frame", d.r16(0x0422) as u32);
    let _ = write!(s, ",\"pt\":\"{pt:x}\",");
    kv(&mut s, "page", d.r16(a(0x26872)) as u32);
    s.push(',');
    kv(&mut s, "in1", d.r8(a(0x1DAA4)) as u32);
    s.push(',');
    kv(&mut s, "in2", d.r8(a(0x1DB06)) as u32);
    s.push_str(",\"sc\":[");
    for (i, k) in sc.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{k}");
    }
    s.push_str("],");
    kv(&mut s, "alive", d.r8(0x041D) as u32);
    s.push(',');
    kv(&mut s, "clear", d.r8(0x03ED) as u32);
    s.push_str(",\"p1\":");
    player(&mut s, g, &PLAYER1);
    s.push_str(",\"p2\":");
    player(&mut s, g, &PLAYER2);
    s.push_str(",\"bg\":[");
    for (i, x) in [0x1E6B8u32, 0x1E6BA, 0x1E6BC, 0x1E6BE].iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{}", d.r16(a(*x)));
    }
    s.push_str("],");
    pool(&mut s, g, "ent_t", "ent", 0x2294, 15, 24);
    s.push(',');
    pool(&mut s, g, "pb_t", "pb", 0x2607, 20, 16);
    s.push(',');
    pool(&mut s, g, "eb_t", "eb", 0x32B3, 20, 15);
    s.push(',');
    words(&mut s, g, "fx_x", 0x3FC2, 30);
    s.push(',');
    words(&mut s, g, "fx_y", 0x3FFE, 30);
    s.push(',');
    words(&mut s, g, "fx_a", 0x403A, 30);
    s.push(',');
    words(&mut s, g, "g_st", 0x19C8, 10);
    s.push(',');
    words(&mut s, g, "g_x", 0x19A0, 10);
    s.push(',');
    words(&mut s, g, "g_y", 0x19B4, 10);
    s.push(',');
    words(&mut s, g, "g_dx", 0x198C, 10);
    s.push_str(",\"misc\":{");
    kv(&mut s, "seek", d.r16(a(0x1F1A8)) as u32);
    s.push(',');
    kv(&mut s, "dhp", d.r16(a(0x1E6B2)) as u32);
    s.push(',');
    kv(&mut s, "dfire", d.r16(a(0x1E6B4)) as u32);
    s.push(',');
    kv(&mut s, "boom", d.r8(a(0x1CFAE)) as u32);
    s.push(',');
    kv(&mut s, "demo", d.r8(a(0x1CFDC)) as u32);
    s.push(',');
    kv(&mut s, "stage", d.r8(a(0x1CFDB)) as u32);
    s.push(',');
    kv(&mut s, "nodmg", d.r16(a(0x273A5)) as u32);
    s.push(',');
    kv(&mut s, "fx0", d.r16(a(0x20B7A)) as u32);
    s.push(',');
    kv(&mut s, "fx1", d.r16(a(0x20B7C)) as u32);
    s.push(',');
    kv(&mut s, "fx2", d.r16(a(0x20B7E)) as u32);
    s.push('}');
    s
}

/// 去掉一行收據尾端的 `emu` 物件與收尾括號，留下可比對的部分。
pub fn strip_emu(line: &str) -> &str {
    match line.rfind(",\"emu\":") {
        Some(i) => &line[..i],
        None => line.trim_end().trim_end_matches('}'),
    }
}

/// 一筆收據裡的 `sc` 陣列。
pub fn parse_sc(line: &str) -> Vec<u8> {
    let Some(i) = line.find("\"sc\":[") else { return Vec::new() };
    let rest = &line[i + 6..];
    let end = rest.find(']').unwrap_or(0);
    rest[..end].split(',').filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect()
}

/// 找出兩行第一個不同處，回傳可讀的欄位路徑與兩邊的片段。
pub fn describe_diff(got: &str, want: &str) -> String {
    let i = got.bytes().zip(want.bytes()).take_while(|(x, y)| x == y).count();
    let head = &got[..i];
    let key_at = head.rfind("\":").map(|e| {
        let b = head[..e].rfind('"').map(|p| p + 1).unwrap_or(0);
        head[b..e].to_string()
    });
    let mut path = key_at.unwrap_or_default();
    if let Some(wpos) = head.rfind("\"w\":[") {
        let tail = &head[wpos + 5..];
        if !tail.contains(']') {
            let idx = tail.matches(',').count();
            let sl = head[..wpos].rfind("\"s\":").map(|p| {
                let r = &head[p + 4..wpos];
                r.trim_end_matches(',').to_string()
            });
            let owner = head[..wpos]
                .rfind(":[{")
                .map(|p| {
                    let b = head[..p].rfind('"').unwrap_or(0);
                    let c = head[..b].rfind('"').map(|q| q + 1).unwrap_or(0);
                    head[c..b].to_string()
                })
                .unwrap_or_default();
            path = format!("{owner}[槽 {}].w{idx}", sl.unwrap_or_default());
        }
    } else if let Some(open) = head.rfind(":[") {
        let tail = &head[open + 2..];
        if !tail.contains(']') {
            path = format!("{path}[{}]", tail.matches(',').count());
        }
    }
    let frag = |s: &str| s.get(i.saturating_sub(12)..(i + 28).min(s.len())).unwrap_or("").to_string();
    format!("欄位 {path}\n  重製 …{}…\n  原版 …{}…", frag(got), frag(want))
}
