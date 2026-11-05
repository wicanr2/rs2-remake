//! 全流程對拍（規格 002）：讀 `rs2dyn waittrace` 的收據，重製在每次等待結束的同一點比對完整狀態。
//!
//! 一份收據是目錄 `wt_<名稱>/`：
//! - `wait-wt_<名稱>.jsonl`：每次 `sub_11097` 離開等待（`1109Dh`）一筆，欄位見 `tools/dos` 的 `wait.go` 註解。
//! - `wait-wt_<名稱>.dsd`：資料段差異，第 0 筆是完整 64 KB，之後每筆只存和上一筆不同的位元組。
//! - `wait-wt_<名稱>.pokes`：waittrace 開著時的 poke，`n 位址 寬 值`，n 是 poke 當時已寫出的筆數。
//! - `entry.ds`、`entry.p0..p3`、`entry.json`：入口（`untilpc`）的資料段、四個平面與 DAC。
//! - `biosfont.bin`：BIOS 8x8 字型（`F000:FA6Eh` 起 2048 位元組），第一關的幀計數用它。
//!
//! 餵法：第 n 次等待恢復時餵第 n 筆的 `sc`（原版的鍵落在等待裡，`rs2dyn` 的 waitsync 保證）。輪詢迴圈沒有等待，
//! 重製的協程在 `Req::Poll` 交回控制權：下一個要比對的那筆的 `sc` 在第一個輪詢步全部先餵進去，不重複餵。
//! 兩種情況都是「餵 `rows[已比對筆數].sc` 一次」，所以不必分辨 `Frame` 是從等待還是輪詢回來。
//!
//! 比對點是 `Game::step_full` 的 `on_wait`：`cx`、資料段（`DS_EXEMPT` 以外逐位元組）、四個平面的雜湊、DAC 與屬性暫存器的雜湊。
//! `on_wait` 拿到的 `Game` 只有資料段，繪製命令在 `Frame.draws`；`step_full` 回傳後才餵 sink，所以 `on_wait` 時 sink 裡是
//! 「這次等待之前」累積的全部命令，剛好對應原版在等待結束時的畫面。
//!
//! 移植還在進行，`unimplemented!` 的 panic 接起來回報成「未移植 sub_X」，不算比對失敗。

use crate::{Inputs, parse_sc};
use rs2_core::{AudioCmd, DrawCmd, Entry, Exit, Fixes, Game, Input, ShtGroup, a, boot_ds_with_dm1, ds_exempt};
use rs2_gfx::{Assets, BankDump, VgaSink};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Once;

/// 收據的一筆。
#[derive(Clone, Debug)]
pub struct WaitRow {
    pub n: u64,
    /// 呼叫 `sub_11097` 的位址（返回位址，IDA 線性位址）。
    pub ret: u32,
    pub cx: u16,
    pub frame: u16,
    /// 上一筆之後送達 INT 9 的掃描碼。
    pub sc: Vec<u8>,
    /// `sc` 每個鍵送達時距上一筆已經過幾個 INT 8（選用欄位；舊收據沒有，空陣列）。輪詢迴圈裡的鍵靠它排進對的 tick。
    pub sct: Vec<u32>,
    pub vram: String,
    pub dac: String,
    pub start: u32,
    /// 這筆等待時的指令數（選用欄位），把音訊事件對到第幾次等待。
    pub step: Option<u64>,
    /// 單色模式（C、H）的欄位（`tools/dosgolem-patches/0008`），E 的收據沒有。`mono` 是顯示記憶體整塊的 SHA-256，
    /// `msa`、`msb` 是背景暫存區 A、B，`hpage` 是 H 的顯示頁起點（0 或 8000h）。
    pub mono: Option<String>,
    pub msa: Option<String>,
    pub msb: Option<String>,
    pub hpage: Option<u32>,
}

/// 欄位值的起點：`"key":` 之後略過空白（收據的 jsonl 沒有空白，entry.json 是縮排過的 JSON）。
fn value_start<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\":");
    let i = line.find(&pat)? + pat.len();
    Some(line[i..].trim_start())
}

fn field_u64(line: &str, key: &str) -> Option<u64> {
    let rest = value_start(line, key)?;
    let end = rest.find([',', '}', '\n']).unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

fn field_str<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = value_start(line, key)?.strip_prefix('"')?;
    Some(&rest[..rest.find('"')?])
}

/// `"key":[1,2,3]` 的整數陣列；沒有這個欄位回傳空陣列。
fn parse_list(line: &str, key: &str) -> Vec<u32> {
    let Some(rest) = value_start(line, key).and_then(|r| r.strip_prefix('[')) else { return Vec::new() };
    let end = rest.find(']').unwrap_or(0);
    rest[..end].split(',').filter(|t| !t.is_empty()).filter_map(|t| t.trim().parse().ok()).collect()
}

pub fn parse_row(line: &str) -> Result<WaitRow, String> {
    let bad = |k: &str| format!("收據一行缺欄位 {k}：{line}");
    Ok(WaitRow {
        n: field_u64(line, "n").ok_or_else(|| bad("n"))?,
        ret: u32::from_str_radix(field_str(line, "ret").ok_or_else(|| bad("ret"))?, 16).map_err(|e| format!("ret：{e}"))?,
        cx: field_u64(line, "cx").ok_or_else(|| bad("cx"))? as u16,
        frame: field_u64(line, "frame").ok_or_else(|| bad("frame"))? as u16,
        sc: parse_sc(line),
        sct: parse_list(line, "sct"),
        vram: field_str(line, "vram").ok_or_else(|| bad("vram"))?.to_string(),
        dac: field_str(line, "dac").ok_or_else(|| bad("dac"))?.to_string(),
        start: field_u64(line, "start").ok_or_else(|| bad("start"))? as u32,
        step: field_u64(line, "step"),
        mono: field_str(line, "mono").map(str::to_string),
        msa: field_str(line, "msa").map(str::to_string),
        msb: field_str(line, "msb").map(str::to_string),
        hpage: field_u64(line, "hpage").map(|v| v as u32),
    })
}

/// `.dsd` 的循序讀取。
pub struct Dsd<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Dsd<'a> {
    pub fn new(data: &'a [u8]) -> Dsd<'a> {
        Dsd { data, pos: 0 }
    }

    /// 讀下一筆並套到 `ds`（64 KB）。筆序要等於 `expect`。回傳段數。
    pub fn apply_next(&mut self, ds: &mut [u8], expect: u64) -> Result<u32, String> {
        let d = self.data;
        let need = |p: usize, n: usize| -> Result<(), String> {
            if p + n > d.len() { Err(format!(".dsd 在第 {expect} 筆被截斷（位移 {p}，需要 {n}）")) } else { Ok(()) }
        };
        need(self.pos, 8)?;
        let n = u32::from_le_bytes(d[self.pos..self.pos + 4].try_into().unwrap()) as u64;
        let count = u32::from_le_bytes(d[self.pos + 4..self.pos + 8].try_into().unwrap());
        if n != expect {
            return Err(format!(".dsd 的筆序是 {n}，預期 {expect}"));
        }
        self.pos += 8;
        for _ in 0..count {
            need(self.pos, 4)?;
            let off = u16::from_le_bytes([d[self.pos], d[self.pos + 1]]) as usize;
            let len = u16::from_le_bytes([d[self.pos + 2], d[self.pos + 3]]) as usize;
            self.pos += 4;
            need(self.pos, len)?;
            if off + len > ds.len() {
                return Err(format!(".dsd 第 {expect} 筆的段超出資料段（偏移 {off:#x} 長 {len:#x}）"));
            }
            ds[off..off + len].copy_from_slice(&d[self.pos..self.pos + len]);
            self.pos += len;
        }
        Ok(count)
    }
}

/// `wait-NAME.pokes` 的一行：第 `row` 筆等待結束之前寫入。
#[derive(Clone, Copy, Debug)]
pub struct Poke {
    pub row: usize,
    pub addr: u32,
    pub width: u8,
    pub val: u16,
}

fn num(s: &str) -> Result<u32, String> {
    let r = match s.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16),
        None => s.parse::<u32>(),
    };
    r.map_err(|e| format!("數字 {s:?} 解不開：{e}"))
}

pub fn parse_pokes(text: &str) -> Result<Vec<Poke>, String> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 4 {
            return Err(format!(".pokes 第 {} 行要有 4 欄 `n 位址 寬 值`：{line}", i + 1));
        }
        let width = num(f[2])? as u8;
        if width != 1 && width != 2 {
            return Err(format!(".pokes 第 {} 行寬只能是 1 或 2：{line}", i + 1));
        }
        out.push(Poke { row: num(f[0])? as usize, addr: num(f[1])?, width, val: num(f[3])? as u16 });
    }
    out.sort_by_key(|p| p.row); // 穩定排序，同一筆內維持檔案順序
    Ok(out)
}

/// 一份收據目錄的內容。
#[derive(Clone)]
pub struct Receipt {
    /// 目錄名，例如 `wt_flow`。
    pub name: String,
    pub dir: PathBuf,
    pub rows: Vec<WaitRow>,
    pub dsd: Vec<u8>,
    pub pokes: Vec<Poke>,
    pub entry_ds: Vec<u8>,
    pub planes: [Vec<u8>; 4],
    /// 入口的 256 色 DAC，768 位元組（6 位元值）。
    pub dac: Vec<u8>,
    pub font: Vec<u8>,
    /// `wait-NAME.bk`（腳本的 `waitbanks`）：片尾畫越界圖號時的圖庫記憶體。
    pub banks: Option<Vec<u8>>,
}

/// 目錄名決定入口：`wt_title`、`wt_flow`、`wt_menu`、`wt_p2`、`wt_mute` 從標題開始，`wt_stageN`、`wt_fzN-*` 從第 N 關入口，
/// `wt_ending` 從片尾入口。
pub fn entry_of(name: &str) -> Result<Entry, String> {
    let s = name.strip_prefix("wt_").unwrap_or(name);
    let head = s.split(['-', '_']).next().unwrap_or(s);
    Ok(match head {
        "title" | "flow" | "menu" | "p2" | "mute" => Entry::Title,
        "stage1" | "fz1" | "pause" => Entry::Stage1,
        "stage2" | "fz2" => Entry::Stage2,
        "stage3" | "fz3" => Entry::Stage3,
        "stage4" | "fz4" => Entry::Stage4,
        "ending" => Entry::Ending,
        _ => return Err(format!("不認得收據 {name} 的入口")),
    })
}

fn hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("十六進位字串長度是奇數".into());
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|e| e.to_string())).collect()
}

/// 讀入口 json 的 `dac256_hex`。
fn read_dac(json: &str) -> Result<Vec<u8>, String> {
    let dac = hex_bytes(field_str(json, "dac256_hex").ok_or("entry.json 沒有 dac256_hex")?)?;
    if dac.len() != 768 {
        return Err(format!("dac256_hex 應為 768 位元組，實際 {}", dac.len()));
    }
    Ok(dac)
}

pub fn load_receipt(dir: &Path) -> Result<Receipt, String> {
    let name = dir.file_name().and_then(|n| n.to_str()).ok_or("收據目錄名")?.to_string();
    let rd = |f: &str| std::fs::read(dir.join(f)).map_err(|e| format!("讀不到 {}：{e}", dir.join(f).display()));
    let mut jsonl: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("讀不到 {}：{e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("wait-") && n.ends_with(".jsonl")))
        .collect();
    jsonl.sort();
    if jsonl.len() != 1 {
        return Err(format!("{} 應有一個 wait-*.jsonl，實際 {} 個", dir.display(), jsonl.len()));
    }
    let stem = jsonl[0].with_extension("");
    let text = std::fs::read_to_string(&jsonl[0]).map_err(|e| e.to_string())?;
    let rows = text.lines().filter(|l| !l.trim().is_empty()).map(parse_row).collect::<Result<Vec<_>, _>>()?;
    if rows.is_empty() {
        return Err("收據是空的".into());
    }
    for (i, r) in rows.iter().enumerate() {
        if r.n != i as u64 {
            return Err(format!("收據第 {i} 行的 n 是 {}", r.n));
        }
    }
    let pokes = match std::fs::read_to_string(stem.with_extension("pokes")) {
        Ok(t) => parse_pokes(&t)?,
        Err(_) => Vec::new(),
    };
    let entry_ds = rd("entry.ds")?;
    if entry_ds.len() != 0x10000 {
        return Err(format!("entry.ds 長度 {}", entry_ds.len()));
    }
    Ok(Receipt {
        name,
        dir: dir.to_path_buf(),
        rows,
        dsd: std::fs::read(stem.with_extension("dsd")).map_err(|e| format!(".dsd：{e}"))?,
        pokes,
        entry_ds,
        planes: [rd("entry.p0")?, rd("entry.p1")?, rd("entry.p2")?, rd("entry.p3")?],
        dac: read_dac(&String::from_utf8_lossy(&rd("entry.json")?))?,
        font: rd("biosfont.bin")?,
        banks: std::fs::read(stem.with_extension("bk")).ok(),
    })
}

/// 比對結果。
#[derive(Clone, Debug)]
pub enum Verdict {
    /// 全部筆數相同。
    Pass { rows: usize },
    /// 比到 `compared` 筆之後遇到沒移植的函式。
    Unported { func: String, compared: usize, total: usize },
    /// 第 `at` 筆（0 起算）不同。`report` 是欄位與位元組。
    Diff { at: usize, compared: usize, total: usize, report: String },
}

impl Verdict {
    pub fn compared(&self) -> usize {
        match self {
            Verdict::Pass { rows } => *rows,
            Verdict::Unported { compared, .. } | Verdict::Diff { compared, .. } => *compared,
        }
    }
}

static HOOK: Once = Once::new();

/// 安靜掉「未移植」的 panic 訊息，其他 panic 照舊印。
fn install_quiet_hook() {
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if unported_func(&panic_text(info.payload())).is_none() {
                prev(info);
            }
        }));
    });
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = p.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "<非字串的 panic>".to_string()
    }
}

/// `unimplemented!("sub_X")` 與 `todo!("sub_X")` 的訊息，回傳 `sub_X`。
fn unported_func(msg: &str) -> Option<String> {
    let rest = msg.strip_prefix("not implemented").or_else(|| msg.strip_prefix("not yet implemented"))?;
    Some(rest.trim_start_matches(':').trim().to_string())
}

/// 兩份資料段的差異（`ds_exempt` 以外），相鄰的合成一段，最多列 12 段。`report` 為真時連豁免範圍一起列（不判失敗）。
pub fn ds_diff(got: &[u8], want: &[u8], report: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < got.len() {
        if got[i] == want[i] || (!report && ds_exempt(i as u16)) {
            i += 1;
            continue;
        }
        let lo = i;
        let mut hi = i;
        while hi + 1 < got.len() && (0..=2).any(|k| hi + 1 + k < got.len() && got[hi + 1 + k] != want[hi + 1 + k]) {
            hi += 1;
        }
        let show = |b: &[u8]| b[lo..=hi.min(lo + 7)].iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ");
        let ex = if (lo..=hi).all(|o| ds_exempt(o as u16)) { "（豁免）" } else { "" };
        out.push(format!("  dseg:{lo:04X}..{hi:04X} ({}B){ex} 重製 {} 原版 {}", hi - lo + 1, show(got), show(want)));
        i = hi + 1;
    }
    if !report {
        out.truncate(12);
    }
    out
}


/// 畫面定位（`tools/dos/wait_locate.sh NAME N` 的傾印）。目錄 `wl_<名稱>_<n>/` 在收據根目錄下，裡面有
/// `wt_<名稱>-w<n>.p0..p3`。回傳不同的位元組段（每個平面最多 4 段，共列 12 段）與第一個不同位元組的最後寫入命令。
pub struct LocateCtx<'a> {
    /// 收據根目錄（`RS2_DOSRUN`）。
    pub root: &'a Path,
    /// 到目前為止套用過的全部繪製命令，依序。
    pub cmds: &'a [DrawCmd],
    /// 建一個和 `replay` 開始時相同的 sink（平面、DAC、字型與殘值）。
    pub fresh: &'a dyn Fn() -> VgaSink,
}

fn locate_detail(ctx: &LocateCtx, name: &str, n: u64, sink: &VgaSink) -> String {
    let short = name.strip_prefix("wt_").unwrap_or(name);
    let dir = ctx.root.join(format!("wl_{short}_{n}"));
    let mut dumps: Vec<Vec<u8>> = Vec::new();
    for p in 0..4 {
        match std::fs::read(dir.join(format!("{name}-w{n}.p{p}"))) {
            Ok(d) if d.len() == 0x10000 => dumps.push(d),
            _ => return format!("  沒有 {} 的平面傾印，先跑 tools/dos/wait_locate.sh {short} {n}", dir.display()),
        }
    }
    let mut out = Vec::new();
    let mut first: Option<(usize, u16)> = None;
    for (p, dump) in dumps.iter().enumerate() {
        let got = sink.plane(p);
        let (mut i, mut spans, mut bytes) = (0usize, 0usize, 0usize);
        while i < got.len() {
            if got[i] == dump[i] {
                i += 1;
                continue;
            }
            let lo = i;
            let mut hi = i;
            while hi + 1 < got.len() && (0..=2).any(|k| hi + 1 + k < got.len() && got[hi + 1 + k] != dump[hi + 1 + k]) {
                hi += 1;
            }
            bytes += (lo..=hi).filter(|&o| got[o] != dump[o]).count();
            spans += 1;
            first.get_or_insert((p, lo as u16));
            if spans <= 4 {
                out.push(format!("  平面 {p} {lo:04X}..{hi:04X} {}（重製 {:02X} 原版 {:02X}）", rs2_gfx::locate(lo as u16), got[lo], dump[lo]));
            }
            i = hi + 1;
        }
        if spans > 0 {
            out.push(format!("  平面 {p} 共 {spans} 段、{bytes} 位元組不同"));
        }
    }
    if let Some((p, off)) = first {
        let mut s = (ctx.fresh)();
        s.watch(p, off);
        for c in ctx.cmds {
            s.apply(c);
        }
        match s.last_write_seq() {
            Some(i) => {
                out.push(format!("  平面 {p} {off:04X} 最後寫入：命令 #{i} = {:?}", ctx.cmds[i as usize]));
                for (j, c) in ctx.cmds.iter().enumerate().skip(i as usize + 1).take(24) {
                    if let DrawCmd::Blit(b) = c {
                        out.push(format!("    後續 #{j} {:?} index={} x={} y={} cx={} page={:04X}", b.op, b.index, b.x, b.y, b.cx, b.page));
                    }
                }
                for m in s.anomaly_log().iter().rev().take(8) {
                    out.push(format!("    異常 {m}"));
                }
                // 前面幾個命令的摘要：sub_19683 跳過 y < 0 的列時用 word_2687E 推進來源指標，它是最近一次載入的圖庫項的每列位元組數，
                // 這個殘值取決於前面載圖庫項的命令（ds 的 9CBEh 在 DS_EXEMPT，重製不維護）。
                for j in i.saturating_sub(6) as usize..i as usize {
                    if let DrawCmd::Blit(b) = &ctx.cmds[j] {
                        out.push(format!("    前 {} 個 #{j} {:?} index={} x={} y={} cx={} page={:04X}", i as usize - j, b.op, b.index, b.x, b.y as i16, b.cx, b.page));
                    }
                }
                // 診斷：把 word_2687E 的殘值（`set_stride_reg`）在這個命令之前改成 0 至 0x80，哪些值能讓這個位元組和傾印相同。
                let want = dumps[p][off as usize];
                let mut fits: Vec<u16> = Vec::new();
                for v in 0..=0x80u16 {
                    let mut t = (ctx.fresh)();
                    for c in &ctx.cmds[..i as usize] {
                        t.apply(c);
                    }
                    t.set_stride_reg(v);
                    t.apply(&ctx.cmds[i as usize]);
                    if t.plane(p)[off as usize] == want {
                        fits.push(v);
                    }
                }
                out.push(format!("  診斷：命令 #{i} 之前把 word_2687E 殘值設成這些值，這個位元組就和傾印相同：{fits:?}"));
            }
            None => out.push(format!("  平面 {p} {off:04X} 整串命令都沒有寫過這個位元組")),
        }
    } else {
        out.push("  傾印的四個平面和重製相同：差異在 DAC 或屬性暫存器，不在平面".to_string());
    }
    out.join("\n")
}

/// 比一筆。回傳不同處的說明。`locate` 非空時，畫面不同就用傾印找出不同的偏移與最後寫入的命令。
fn compare_row(
    name: &str,
    g: &Game,
    cx: u16,
    row: &WaitRow,
    want_ds: &[u8],
    sink: &VgaSink,
    report: bool,
    locate: Option<&LocateCtx>,
) -> Option<String> {
    let mut diffs: Vec<String> = Vec::new();
    if cx != row.cx {
        diffs.push(format!("欄位 cx：重製 {cx} 原版 {}", row.cx));
    }
    // 顯示起點：第一次翻頁之前重製不知道，從快照入口進來時也一樣。
    if let Some(st) = g.display_start() {
        if st as u32 != row.start {
            diffs.push(format!("欄位 start：重製 {st} 原版 {}", row.start));
        }
    }
    let ds = ds_diff(g.ds.bytes(), want_ds, report);
    if report {
        for l in &ds {
            println!("DSDIFF n={}{l}", row.n);
        }
    } else if !ds.is_empty() {
        diffs.push(format!("欄位 資料段（前 {} 段）：\n{}", ds.len(), ds.join("\n")));
    }
    let v = sink.vram_sha256();
    let vram_bad = v != row.vram;
    if vram_bad {
        diffs.push(format!("欄位 vram：重製 {v} 原版 {}", row.vram));
    }
    let d = sink.dac_sha256();
    let dac_bad = d != row.dac;
    if dac_bad {
        diffs.push(format!("欄位 dac：重製 {d} 原版 {}", row.dac));
    }
    if diffs.is_empty() {
        return None;
    }
    let mut s = format!("第 {} 筆不同（ret={:x} cx={} frame={}）\n", row.n, row.ret, row.cx, row.frame);
    s.push_str(&diffs.join("\n"));
    if vram_bad || dac_bad {
        let short = name.strip_prefix("wt_").unwrap_or(name);
        match locate {
            Some(ctx) => {
                s.push('\n');
                s.push_str(&locate_detail(ctx, name, row.n, sink));
            }
            None => s.push_str(&format!(
                "\n  畫面不同：tools/dos/wait_locate.sh {short} {}，再用 RS2_WAIT_LOCATE=1 重跑本測試",
                row.n
            )),
        }
    }
    Some(s)
}

/// 重播的選項。
#[derive(Clone, Debug, Default)]
pub struct ReplayOpts {
    /// 印出資料段差異（含豁免）而不判失敗。
    pub ds_report: bool,
    /// 收據根目錄；有值時，畫面不同就讀 `wl_<名稱>_<n>/` 的平面傾印（`tools/dos/wait_locate.sh`）定位。
    pub locate_root: Option<PathBuf>,
    /// 資料段從 `boot_ds_with_dm1` 開始，不用收據的入口傾印。畫面仍從入口傾印開始，
    /// 因為開機畫面屬於啟動程序，不在移植範圍。只適用從標題開始的收據。
    pub boot: bool,
}

/// 重播一份收據。`Fixes` 全關。`ds_report` 為真時印出資料段差異（含豁免）而不判失敗。
pub fn replay(inputs: &Inputs, assets: &Assets, rc: &Receipt, ds_report: bool) -> Verdict {
    replay_with(inputs, assets, rc, &ReplayOpts { ds_report, locate_root: None, boot: false })
}

/// `replay_observed` 交給觀察者的事件：重製的繪製命令，依序。
#[derive(Clone, Copy, Debug)]
pub enum ReplayEvent<'a> {
    /// `Game::from_snapshot` 後 `take_draws()` 的初始化命令。
    Init(&'a [DrawCmd]),
    /// 一步（`step_full` 一次）回傳的繪製命令，已套用到 `replay_with` 自己的 `VgaSink`。
    Step(&'a [DrawCmd]),
    /// 同一步的繪製與音訊命令，`next_row` 是這一步結束時下一筆要比的等待。
    /// 原版對應的事件落在第 `next_row - 1` 筆與第 `next_row` 筆等待之間。
    /// `sc` 是這一步送進去的掃描碼，`ds` 是這一步之後的整段資料段。
    StepAt { sc: &'a [u8], draws: &'a [DrawCmd], audio: &'a [AudioCmd], next_row: usize, ds: &'a [u8] },
}

/// 同 `replay_with`，另把每個繪製命令串依序交給 `obs`。輸入餵法、快照入口與比對與 `replay_with` 完全相同
/// （`replay_with` 就是不帶觀察者的同一份程式碼），所以觀察者看到的命令串就是通過 `wait_replay` 的那一批。
/// 觀察者在該步的命令套到內部 `VgaSink` 之後被呼叫，不影響比對。
pub fn replay_observed(inputs: &Inputs, assets: &Assets, rc: &Receipt, opts: &ReplayOpts, obs: &mut dyn FnMut(ReplayEvent)) -> Verdict {
    replay_core(inputs, assets, rc, opts, Some(obs))
}

pub fn replay_with(inputs: &Inputs, assets: &Assets, rc: &Receipt, opts: &ReplayOpts) -> Verdict {
    replay_core(inputs, assets, rc, opts, None)
}

fn replay_core(inputs: &Inputs, assets: &Assets, rc: &Receipt, opts: &ReplayOpts, mut obs: Option<&mut dyn FnMut(ReplayEvent)>) -> Verdict {
    install_quiet_hook();
    let ds_report = opts.ds_report;
    let total = rc.rows.len();
    let entry = match entry_of(&rc.name) {
        Ok(e) => e,
        Err(e) => return Verdict::Diff { at: 0, compared: 0, total, report: e },
    };
    let boot = boot_ds_with_dm1(&inputs.image, &inputs.dm1);
    let start_ds: &[u8] = if opts.boot { boot.bytes() } else { &rc.entry_ds };
    let mut g = Game::from_snapshot(inputs.game_files(), start_ds, entry, Fixes::NONE);
    // 入口的畫面：四個平面、DAC、BIOS 字型，與 word_2687E 的殘值（最近一次載入的圖庫項每列位元組數，從入口的資料段取）。
    let make_sink = || {
        let mut s = VgaSink::new(assets.clone());
        let planes = [&rc.planes[0][..], &rc.planes[1][..], &rc.planes[2][..], &rc.planes[3][..]];
        let dac: [u8; 768] = rc.dac[..].try_into().expect("768 位元組");
        s.load_snapshot(planes, &dac);
        s.set_counter_font(&rc.font);
        s.set_stride_reg(u16::from_le_bytes([rc.entry_ds[a(0x2687E) as usize], rc.entry_ds[a(0x2687E) as usize + 1]]));
        if let Some(b) = &rc.banks {
            s.set_bank_dump(BankDump::parse(ShtGroup::Sht5, b).expect("圖庫傾印"));
        }
        s
    };
    let mut sink = make_sink();
    let keep = opts.locate_root.is_some();
    let mut cmds: Vec<DrawCmd> = Vec::new();
    let init = g.take_draws();
    for c in &init {
        sink.apply(c);
        if keep {
            cmds.push(*c);
        }
    }
    if let Some(o) = obs.as_mut() {
        o(ReplayEvent::Init(&init));
    }

    let mut want_ds = vec![0u8; 0x10000];
    let mut dsd = Dsd::new(&rc.dsd);
    let mut done = 0usize; // 已比對的筆數，也是下一個要比對的筆序
    let mut fed_row = usize::MAX; // 正在餵哪一筆的鍵與 poke
    let mut fed_n = 0usize; // rows[fed_row].sc 已經餵了幾個
    let mut polls = 0usize; // 上一筆等待結束之後的輪詢步數，每步 1 個 tick
    let mut polls_for = 0usize; // polls 是為哪一筆計的
    let mut started = false;
    let mut next_poke = 0usize;
    let mut idle = 0usize; // 連續沒有進展的步數
    let mut last_done = 0usize;
    let mut verdict: Option<Verdict> = None;

    while done < total {
        // 這一步要餵的：下一個要比對的那筆的 sc 與 poke。
        // 上次交回控制權如果是從 sub_11097（s11097 歸零 word_273B9 才交回），這一步一開頭就是這次等待結束，原版這段期間
        // 送達的鍵全部在等待裡，一次餵完。否則是輪詢迴圈：每個鍵依 `sct`（距上一筆的 tick 數）排進對的 tick，
        // 收據沒有 `sct`（舊收據）就在第一個輪詢步全部餵進去。
        let row = &rc.rows[done];
        if fed_row != done {
            fed_row = done;
            fed_n = 0;
            while next_poke < rc.pokes.len() && rc.pokes[next_poke].row <= done {
                let p = rc.pokes[next_poke];
                if p.width == 1 {
                    g.poke8(a(p.addr), p.val as u8);
                } else {
                    g.poke16(a(p.addr), p.val);
                }
                next_poke += 1;
            }
        }
        let wait_pending = started && g.ds.r16(a(0x273B9)) == 0;
        let use_sct = row.sct.len() == row.sc.len();
        let mut sc: Vec<u8> = Vec::new();
        while fed_n < row.sc.len() && (wait_pending || !use_sct || row.sct[fed_n] as usize <= polls) {
            sc.push(row.sc[fed_n]);
            fed_n += 1;
        }
        started = true;
        let mut diff: Option<String> = None;
        let res = {
            let g_ref = &mut g;
            let sink_ref = &sink;
            let (rows, want, dsd_ref, done_ref) = (&rc.rows, &mut want_ds, &mut dsd, &mut done);
            catch_unwind(AssertUnwindSafe(|| {
                g_ref.step_full(&Input { scancodes: &sc }, &mut |_g: &Game, _pt: u32| {}, &mut |gg: &Game, cx: u16| {
                    if diff.is_some() || *done_ref >= rows.len() {
                        return;
                    }
                    let row = &rows[*done_ref];
                    if let Err(e) = dsd_ref.apply_next(want, row.n) {
                        diff = Some(format!("第 {} 筆：{e}", row.n));
                        return;
                    }
                    let ctx = opts.locate_root.as_deref().map(|root| LocateCtx { root, cmds: &cmds, fresh: &make_sink });
                    diff = compare_row(&rc.name, gg, cx, row, want, sink_ref, ds_report, ctx.as_ref());
                    *done_ref += 1;
                })
            }))
        };
        // 不同的優先於後面的 panic：比對在這一步開頭，panic 是比對之後繼續跑才發生的。
        if let Some(d) = diff {
            verdict = Some(Verdict::Diff { at: done.saturating_sub(1), compared: done.saturating_sub(1), total, report: d });
            break;
        }
        let frame = match res {
            Ok(f) => f,
            Err(p) => {
                if done >= total {
                    break; // 最後一筆已比完，後面才遇到的未移植函式不算
                }
                let msg = panic_text(&*p);
                verdict = Some(match unported_func(&msg) {
                    Some(func) => Verdict::Unported { func, compared: done, total },
                    None => Verdict::Diff { at: done, compared: done, total, report: format!("重製 panic：{msg}") },
                });
                break;
            }
        };
        for c in &frame.draws {
            sink.apply(c);
            if keep {
                cmds.push(*c);
            }
        }
        if let Some(o) = obs.as_mut() {
            o(ReplayEvent::StepAt { sc: &sc, draws: &frame.draws, audio: &frame.audio, next_row: done, ds: g.state_hash_input() });
            o(ReplayEvent::Step(&frame.draws));
        }
        // 輪詢步數：這一步內有等待結束就重算；交回控制權時 word_273B9 不為 0 表示是輪詢迴圈的一個 tick。
        if done != polls_for {
            polls = 0;
            polls_for = done;
        }
        if frame.exit.is_none() && g.ds.r16(a(0x273B9)) != 0 {
            polls += 1;
        }
        if done == last_done {
            idle += 1;
        } else {
            idle = 0;
            last_done = done;
        }
        if idle > 4000 {
            verdict = Some(Verdict::Diff {
                at: done,
                compared: done,
                total,
                report: format!("重製連續 {idle} 步沒有走到下一次等待（卡在輪詢迴圈？）；原版第 {done} 筆 ret={:x} frame={}", rc.rows[done].ret, rc.rows[done].frame),
            });
            break;
        }
        if let Some(e) = frame.exit {
            if done < total {
                let name = match e {
                    Exit::End => "E",
                    Exit::Quit => "Q",
                    Exit::Clear => "C",
                };
                verdict = Some(Verdict::Diff {
                    at: done,
                    compared: done,
                    total,
                    report: format!("重製在第 {done} 筆之前離開（{name}），原版還有 {} 筆", total - done),
                });
            }
            break;
        }
    }
    if sink.anomalies() > 0 {
        eprintln!("{}：繪製異常 {} 筆，例如 {:?}", rc.name, sink.anomalies(), sink.anomaly_log().first());
    }
    verdict.unwrap_or(Verdict::Pass { rows: done })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: &str = r#"{"n":3,"ret":"119f9","cx":5,"frame":12,"sc":[29,46,174],"sct":[0,1,40],"vram":"ab12","dac":"cd34","start":32768}"#;

    #[test]
    fn row_fields() {
        let r = parse_row(ROW).unwrap();
        assert_eq!((r.n, r.ret, r.cx, r.frame, r.start), (3, 0x119f9, 5, 12, 32768));
        assert_eq!(r.sc, vec![29, 46, 174]);
        assert_eq!(r.sct, vec![0, 1, 40]);
        assert_eq!((r.vram.as_str(), r.dac.as_str()), ("ab12", "cd34"));
    }

    #[test]
    fn row_without_optional_fields() {
        let r = parse_row(r#"{"n":0,"ret":"1abcd","cx":1,"frame":0,"sc":[],"vram":"x","dac":"y","start":0}"#).unwrap();
        assert!(r.sc.is_empty() && r.sct.is_empty());
        assert!(parse_row(r#"{"n":0,"cx":1}"#).is_err());
    }

    #[test]
    fn dsd_roundtrip_and_errors() {
        // 第 0 筆：兩段各 4 位元組（只為測試縮短）；第 1 筆：一段 2 位元組。
        let mut d = Vec::new();
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&2u32.to_le_bytes());
        for (off, bytes) in [(0u16, [1u8, 2, 3, 4]), (4, [5, 6, 7, 8])] {
            d.extend_from_slice(&off.to_le_bytes());
            d.extend_from_slice(&4u16.to_le_bytes());
            d.extend_from_slice(&bytes);
        }
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&3u16.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&[0xAA, 0xBB]);
        let mut ds = vec![0u8; 16];
        let mut r = Dsd::new(&d);
        assert_eq!(r.apply_next(&mut ds, 0).unwrap(), 2);
        assert_eq!(&ds[..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(r.apply_next(&mut ds, 1).unwrap(), 1);
        assert_eq!(&ds[..8], &[1, 2, 3, 0xAA, 0xBB, 6, 7, 8]);
        // 筆序不符與截斷都要報錯。
        assert!(Dsd::new(&d).apply_next(&mut ds, 1).is_err());
        assert!(Dsd::new(&d[..10]).apply_next(&mut ds, 0).is_err());
    }

    #[test]
    fn pokes_and_entries() {
        let p = parse_pokes("# 註解\n116 0x1E6B2 2 0x1\n5 0x1DA8D 1 0x59\n").unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].row, p[0].width, p[0].val), (5, 1, 0x59), "依筆序排序");
        assert!(parse_pokes("1 2 3").is_err());
        assert!(parse_pokes("1 0x10 4 0x1").is_err(), "寬只能是 1 或 2");
        assert!(matches!(entry_of("wt_p2"), Ok(Entry::Title)));
        assert!(matches!(entry_of("wt_fz3-laser-2"), Ok(Entry::Stage3)));
        assert!(matches!(entry_of("wt_ending"), Ok(Entry::Ending)));
        assert!(entry_of("wt_xyz").is_err());
    }

    #[test]
    fn unported_messages() {
        assert_eq!(unported_func("not implemented: sub_11556").as_deref(), Some("sub_11556"));
        assert_eq!(unported_func("not yet implemented: sub_X").as_deref(), Some("sub_X"));
        assert_eq!(unported_func("index out of bounds"), None);
    }

    #[test]
    fn ds_diff_skips_exempt() {
        let (a, mut b) = (vec![0u8; 0x10000], vec![0u8; 0x10000]);
        b[0xA7F9] = 2; // 計時器，豁免
        assert!(ds_diff(&a, &b, false).is_empty());
        b[0x0100] = 1; // 不在豁免範圍
        let d = ds_diff(&a, &b, false);
        assert_eq!(d.len(), 1);
        assert!(d[0].contains("0100"));
        assert!(ds_diff(&a, &b, true).len() == 2, "report 模式連豁免一起列");
    }
}
