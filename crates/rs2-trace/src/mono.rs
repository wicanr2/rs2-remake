//! 單色顯示模式（C：CGA，H：MGA）的全流程對拍（規格 006）。
//!
//! 收據目錄 `wm_<名稱>-c`、`wm_<名稱>-h` 由 `tools/dos/wm_run.sh` 錄製，格式同 E 的 `wt_<名稱>`（見 [`crate::wait`]），差別：
//! - 畫面欄位是 `mono`（顯示記憶體整塊的 SHA-256）、`msa`、`msb`（背景暫存區）與 H 的 `hpage`，不是 `vram`、`dac`。
//! - 入口傾印是 `entry.mono`（`B0000..BFFFF` 的 64 KB）、`entry.msa`、`entry.msb`，不是 `entry.p0..p3`。
//!
//! 重播用 E 模式的 `Game`（模擬與模式無關，`docs/re/14-display-modes.md` 第 5 節），繪製命令餵給 [`MonoSink`]。
//! 第三關地形讀 `SHT.SCR`（`Game::set_terrain_scr`）。資料段逐位元組比對，`DS_EXEMPT` 以外不能有差異；
//! C 的 `word_26872`（畫頁偏移）是 `4000h`，核心記 `8000h`，那兩個位元組不比。

use crate::Inputs;
use crate::mirror::{Dseg, Mirror, a, ds_exempt};
use crate::wait::{Dsd, Poke, Verdict, WaitRow, entry_of, parse_pokes, parse_row};
use rs2_core::{Exit, Fixes, Game, Input};
use rs2_gfx::{MonoAssets, MonoKind, MonoSink};
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

/// 一份單色收據。
#[derive(Clone)]
pub struct MonoReceipt {
    /// 目錄名，例如 `wm_stage3-h`。
    pub name: String,
    pub dir: PathBuf,
    pub kind: MonoKind,
    pub rows: Vec<WaitRow>,
    pub dsd: Vec<u8>,
    pub pokes: Vec<Poke>,
    pub entry_ds: Vec<u8>,
    /// 入口的 `B0000..BFFFF`，64 KB。
    pub vmem: Vec<u8>,
    pub sa: Vec<u8>,
    pub sb: Vec<u8>,
    pub font: Vec<u8>,
}

/// 目錄名決定模式：`-c` 結尾是 CGA，`-h` 是 MGA。
pub fn kind_of(name: &str) -> Result<MonoKind, String> {
    if name.ends_with("-c") {
        Ok(MonoKind::Cga)
    } else if name.ends_with("-h") {
        Ok(MonoKind::Mga)
    } else {
        Err(format!("收據 {name} 的名稱不是 -c 或 -h 結尾"))
    }
}

pub fn load_mono_receipt(dir: &Path) -> Result<MonoReceipt, String> {
    let name = dir.file_name().and_then(|n| n.to_str()).ok_or("收據目錄名")?.to_string();
    let kind = kind_of(&name)?;
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
        if r.mono.is_none() || r.msa.is_none() || r.msb.is_none() {
            return Err(format!("收據第 {i} 行沒有 mono、msa、msb 欄位，不是單色收據"));
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
    Ok(MonoReceipt {
        name,
        dir: dir.to_path_buf(),
        kind,
        rows,
        dsd: std::fs::read(stem.with_extension("dsd")).map_err(|e| format!(".dsd：{e}"))?,
        pokes,
        entry_ds,
        vmem: rd("entry.mono")?,
        sa: rd("entry.msa")?,
        sb: rd("entry.msb")?,
        font: rd("biosfont.bin")?,
    })
}

/// 單色模式下核心與原版資料段本來就不同的位元組（`DS_EXEMPT` 之外）：
/// - `word_2685A`（`9C9Ah`）：`sub_1A70A` 依 `word_2685C` 選的暫存區段，只有單色的暫存區 blit 讀，核心沒有這個函式。
/// - 檔名字串表（`A92Ah`～`AAE1h`，`DM*.LCK` 的資料從 `AAE2h` 起）：`sub_1C738` 把單色檔名的第 8 個字元就地換成 `B`、`H`、`R` 等，
///   只有檔案載入讀。E 只換 `SHT4.GO` 一處（`DS_EXEMPT` 的 `AA2Ch`），單色每個載入函式都換。
/// - `stage2_d` 調色盤的工作表（屬性表 `0C96h`，DAC `0CA7h` 起 48 位元組）：第二關幀 > 0A0h 時 E 每幀呼叫 `sub_13894`
///   把它逐步調暗（淡出）。C、H 的 `sub_13894` 在第一個比較（`137F8h` 型的 `byte_1CF74` 是 `E`）就 `retn`，
///   沒有調色盤，表維持映像的初值。這張表只有調色盤函式讀，不進模擬（規格 006「已知差異」）。
///
/// C 的畫頁偏移（`word_26872`，C 是 `4000h`，核心記 E 的 `8000h`）在 `DS_EXEMPT` 的 `9CB0h`～`9CB3h` 內，已豁免。
fn mono_exempt(_kind: MonoKind, off: u16) -> bool {
    matches!(off, 0x9C9A | 0x9C9B) || (0xA92A..=0xAAE1).contains(&off) || (0x0C96..=0x0CD6).contains(&off)
}

/// 兩份資料段的差異，`ds_exempt` 與單色豁免以外，相鄰的合成一段，最多列 12 段。
pub fn mono_ds_diff(kind: MonoKind, got: &[u8], want: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let skip = |i: usize| ds_exempt(i as u16) || mono_exempt(kind, i as u16);
    let mut i = 0;
    while i < got.len() {
        if got[i] == want[i] || skip(i) {
            i += 1;
            continue;
        }
        let lo = i;
        let mut hi = i;
        while hi + 1 < got.len() && (0..=2).any(|k| hi + 1 + k < got.len() && got[hi + 1 + k] != want[hi + 1 + k] && !skip(hi + 1 + k)) {
            hi += 1;
        }
        let show = |b: &[u8]| b[lo..=hi.min(lo + 7)].iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ");
        out.push(format!("  dseg:{lo:04X}..{hi:04X} ({}B) 重製 {} 原版 {}", hi - lo + 1, show(got), show(want)));
        i = hi + 1;
    }
    out.truncate(12);
    out
}

fn compare_row(rc: &MonoReceipt, g: &Game, got_ds: &Dseg, cx: u16, row: &WaitRow, want_ds: &[u8], sink: &MonoSink) -> Option<String> {
    let mut diffs: Vec<String> = Vec::new();
    if cx != row.cx {
        diffs.push(format!("欄位 cx：重製 {cx} 原版 {}", row.cx));
    }
    // H 的顯示頁（3B8h 的 bit 7）：第一次翻頁之前重製不知道。C 的 INT 10h AH=5 在收據端沒有可讀的狀態，不比。
    if let (MonoKind::Mga, Some(st), Some(want)) = (rc.kind, g.display_start(), row.hpage) {
        if st as u32 != want {
            diffs.push(format!("欄位 hpage：重製 {st} 原版 {want}"));
        }
    }
    let ds = mono_ds_diff(rc.kind, got_ds.bytes(), want_ds);
    if !ds.is_empty() {
        diffs.push(format!("欄位 資料段（前 {} 段）：\n{}", ds.len(), ds.join("\n")));
    }
    let checks = [
        ("mono", sink.vram_sha256(), row.mono.as_deref()),
        ("msa", sink.scratch_a_sha256(), row.msa.as_deref()),
        ("msb", sink.scratch_b_sha256(), row.msb.as_deref()),
    ];
    for (name, got, want) in checks {
        if Some(got.as_str()) != want {
            diffs.push(format!("欄位 {name}：重製 {got} 原版 {}", want.unwrap_or("（無）")));
        }
    }
    if diffs.is_empty() {
        return None;
    }
    Some(format!("第 {} 筆不同（ret={:x} cx={} frame={}）\n{}", row.n, row.ret, row.cx, row.frame, diffs.join("\n")))
}

/// 重播時順便量的統計（只讀，不影響比對）。
#[derive(Clone, Debug, Default)]
pub struct MonoStats {
    /// H：比對通過、而且已知顯示頁的筆數。
    pub hgc_rows: usize,
    /// 其中顯示頁的 0000h 組（偶數列）與 2000h 組（中間那條掃描線）在列 0 至 189 逐位元組相同的筆數。
    /// `sub_1117F` 在一幀畫完之後複製，所以顯示中的頁應該每一筆都相同（Hercules 的 3 條掃描線取自偶數列、偶數列、奇數列）。
    /// 只比到列 189：列 191 起的幀計數（`sub_1890D`）每幀同時寫兩頁，複製之後才寫進顯示中的那一頁，所以那幾列的 2000h 組是前一幀的值。
    pub hgc_bank1_is_copy: usize,
    /// H：顯示中的頁，第 `r` 個字元列（遊戲列 `2r`）的 2000h 組與 0000h 組不同的筆數，`r` 是 0 至 99。
    pub hgc_row_diff: Vec<usize>,
    /// 餵給接收端的繪製命令，依 `Op` 的名稱計次（其他命令用 `SetDac` 之類的名稱）。收據涵蓋了哪些單色分支看這裡。
    pub ops: std::collections::BTreeMap<String, u64>,
}

/// 重播的選項。
#[derive(Clone, Debug, Default)]
pub struct MonoReplayOpts {
    /// 收據根目錄；畫面不同時讀 `wl_<名稱>_<n>/` 的傾印定位。
    pub locate_root: Option<PathBuf>,
    /// 要統計就給一個，重播結束後讀它。
    pub stats: Option<Rc<RefCell<MonoStats>>>,
}

/// 建一個和 `replay_mono` 開始時相同的接收端（顯示記憶體、暫存區、BIOS 字型）。
pub fn make_sink(rc: &MonoReceipt, assets: &Arc<MonoAssets>) -> MonoSink {
    let mut s = MonoSink::new(rc.kind, Arc::clone(assets));
    s.load_snapshot(&rc.vmem, &rc.sa, &rc.sb);
    s.set_counter_font(&rc.font);
    s
}

/// 重播一份單色收據：資料段從入口傾印起，畫面從入口的顯示記憶體與暫存區起，`Fixes` 全關。
pub fn replay_mono(inputs: &Inputs, assets: &Arc<MonoAssets>, rc: &MonoReceipt, opts: &MonoReplayOpts) -> Verdict {
    let total = rc.rows.len();
    let entry = match entry_of(rc.name.strip_prefix("wm_").map(|s| format!("wt_{s}")).as_deref().unwrap_or(&rc.name)) {
        Ok(e) => e,
        Err(e) => return Verdict::Diff { at: 0, compared: 0, total, report: e },
    };
    let m = Mirror::from_dump(&rc.entry_ds);
    let mut g = Game::from_state(inputs.game_files(), m.load_bytes(&rc.entry_ds), entry, Fixes::NONE);
    g.set_terrain_scr(true);
    let mut sink = make_sink(rc, assets);
    for c in &g.take_draws() {
        sink.apply(c);
    }
    let mut want_ds = vec![0u8; 0x10000];
    let mut dsd = Dsd::new(&rc.dsd);
    let mut done = 0usize;
    let mut fed_row = usize::MAX;
    let mut fed_n = 0usize;
    let mut polls = 0usize;
    let mut polls_for = 0usize;
    let mut started = false;
    let mut next_poke = 0usize;
    let mut idle = 0usize;
    let mut last_done = 0usize;
    let mut verdict: Option<Verdict> = None;
    let mut cmds = Vec::new();
    let keep = opts.locate_root.is_some();
    while done < total {
        let row = &rc.rows[done];
        if fed_row != done {
            fed_row = done;
            fed_n = 0;
            while next_poke < rc.pokes.len() && rc.pokes[next_poke].row <= done {
                let p = rc.pokes[next_poke];
                if p.width == 1 {
                    m.poke8(g.state_mut(), a(p.addr), p.val as u8);
                } else {
                    m.poke16(g.state_mut(), a(p.addr), p.val);
                }
                next_poke += 1;
            }
        }
        let wait_pending = started && g.state().flow.clock.since_wait == 0;
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
                    diff = compare_row(rc, gg, &m.dump(gg.state()), cx, row, want, sink_ref);
                    if diff.is_none() {
                        if let (Some(st), MonoKind::Mga, Some(stats)) = (gg.display_start(), rc.kind, &opts.stats) {
                            let v = sink_ref.vram();
                            let p = st as usize;
                            let mut s = stats.borrow_mut();
                            s.hgc_rows += 1;
                            if v[p..p + 0x1DB0] == v[p + 0x2000..p + 0x3DB0] {
                                s.hgc_bank1_is_copy += 1;
                            }
                            // 全部 100 個字元列（列 0 至 199 的偶數列）各自 2000h 組與 0000h 組不同的筆數。
                            s.hgc_row_diff.resize(100, 0);
                            for r in 0..100usize {
                                if v[p + r * 0x50..p + r * 0x50 + 0x50] != v[p + 0x2000 + r * 0x50..p + 0x2000 + r * 0x50 + 0x50] {
                                    s.hgc_row_diff[r] += 1;
                                }
                            }
                        }
                    }
                    *done_ref += 1;
                })
            }))
        };
        if let Some(d) = diff {
            let at = done.saturating_sub(1);
            let mut report = d;
            if let Some(root) = &opts.locate_root {
                report.push_str(&locate_detail(rc, root, at as u64, &cmds, assets));
            }
            verdict = Some(Verdict::Diff { at, compared: at, total, report });
            break;
        }
        let frame = match res {
            Ok(f) => f,
            Err(p) => {
                if done >= total {
                    break;
                }
                let msg = if let Some(s) = p.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = p.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "<非字串的 panic>".to_string()
                };
                verdict = Some(Verdict::Diff { at: done, compared: done, total, report: format!("重製 panic：{msg}") });
                break;
            }
        };
        for c in &frame.draws {
            sink.apply(c);
            if keep {
                cmds.push(*c);
            }
            if let Some(stats) = &opts.stats {
                let name = match c {
                    rs2_core::DrawCmd::Blit(b) => format!("{:?}", b.op),
                    other => format!("{:?}", std::mem::discriminant(other)).replace("Discriminant", "Cmd"),
                };
                *stats.borrow_mut().ops.entry(name).or_default() += 1;
            }
        }
        if done != polls_for {
            polls = 0;
            polls_for = done;
        }
        if frame.exit.is_none() && g.state().flow.clock.since_wait != 0 {
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
                report: format!("重製連續 {idle} 步沒有走到下一次等待；原版第 {done} 筆 ret={:x} frame={}", rc.rows[done].ret, rc.rows[done].frame),
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
                verdict = Some(Verdict::Diff { at: done, compared: done, total, report: format!("重製在第 {done} 筆之前離開（{name}），原版還有 {} 筆", total - done) });
            }
            break;
        }
    }
    if sink.anomalies() > 0 {
        eprintln!("{}：繪製異常 {} 筆，例如 {:?}", rc.name, sink.anomalies(), sink.anomaly_log().first());
    }
    verdict.unwrap_or(Verdict::Pass { rows: done })
}

/// 畫面定位：`tools/dos/wm_locate.sh NAME N` 在 `wl_<NAME>_<N>/` 存了第 N 筆（收據的 n 欄）寫完時的 `.mono`、`.msa`、`.msb`。
/// 回傳不同的位元組段與第一個不同位元組的最後寫入命令。
fn locate_detail(rc: &MonoReceipt, root: &Path, n: u64, cmds: &[rs2_core::DrawCmd], assets: &Arc<MonoAssets>) -> String {
    let short = rc.name.strip_prefix("wm_").unwrap_or(&rc.name);
    let dir = root.join(format!("wl_{short}_{n}"));
    let base = format!("wt_{short}-w{n}");
    let Ok(mono) = std::fs::read(dir.join(format!("{base}.mono"))) else {
        return format!("\n  沒有 {} 的傾印，先跑 tools/dos/wm_locate.sh {short} {n}", dir.display());
    };
    let mut s = make_sink(rc, assets);
    let mut last: Vec<Option<usize>> = vec![None; 0x10000];
    let want: Vec<u8> = match rc.kind {
        MonoKind::Cga => mono[0x8000..].to_vec(),
        MonoKind::Mga => mono.clone(),
    };
    let mut prev = s.vram().to_vec();
    for (i, c) in cmds.iter().enumerate() {
        s.apply(c);
        let now = s.vram();
        for (j, (a, b)) in prev.iter().zip(now.iter()).enumerate() {
            if a != b {
                last[j] = Some(i);
            }
        }
        prev.copy_from_slice(now);
    }
    let got = s.vram();
    let mut out = String::new();
    let (mut spans, mut bytes) = (0usize, 0usize);
    let mut i = 0;
    while i < got.len() {
        if got[i] == want[i] {
            i += 1;
            continue;
        }
        let lo = i;
        let mut hi = i;
        while hi + 1 < got.len() && (0..=2).any(|k| hi + 1 + k < got.len() && got[hi + 1 + k] != want[hi + 1 + k]) {
            hi += 1;
        }
        bytes += (lo..=hi).filter(|&o| got[o] != want[o]).count();
        spans += 1;
        if spans <= 6 {
            let who = last[lo].map_or("（沒有命令寫過）".to_string(), |k| format!("命令 #{k} = {:?}", cmds[k]));
            out.push_str(&format!("\n  顯示記憶體 {lo:04X}..{hi:04X} 重製 {:02X} 原版 {:02X}，最後寫入：{who}", got[lo], want[lo]));
        }
        i = hi + 1;
    }
    out.push_str(&format!("\n  顯示記憶體共 {spans} 段、{bytes} 位元組不同"));
    out
}
