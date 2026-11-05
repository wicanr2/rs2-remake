//! 單色 HD 接收端（`MonoHdSink`）對 `MonoSink` 的一致性，用單色收據（`wm_*`）的 14 份重播。
//! 需要原版檔、收據與 HD 資料夾，標 `#[ignore]`：
//! `tools/rust.sh test -p rs2-trace --test mono_hd -- --ignored --nocapture`。
//!
//! 每份收據跑一次 `Game`，同時餵這些接收端：
//! - `MonoSink`：已對這 14 份收據逐位元組對拍過（`tests/mono.rs`），這裡當基準。
//! - `MonoHdSink`（最近鄰）：HD 資料夾不存在，全部用 SD 圖庫項的最近鄰放大。嚴格驗證操作語意：
//!   每一次等待，顯示記憶體與兩塊暫存區的每個位元組放大後都要等於 HD 區塊；取樣點縮回 320x200 要逐像素相同。
//! - `MonoHdSink`（真 HD）：讀 `RS2_HD/mono` 的推導圖（HD 彩色圖轉灰階再抖動）。抖動圖樣與原版不同，不可能逐像素相同，
//!   統計取樣點兩個顯示頁的差異，與門檻比較。門檻的依據見 `GATE_*`。
//! - 兩個對照組，用來證明門檻抓得到結構錯誤：真 HD 但命令的 x 加 1（整體右移 8 個邏輯像素）、y 加 4（下移 4 列）；
//!   另一個對照是真 HD 對相隔一個取樣間隔的另一幀 SD。
//!
//! 環境變數：`RS2_ORIG` 原版目錄、`RS2_HD` HD 素材根目錄。收據根目錄同 `tests/mono.rs`。
//! `RS2_MONO=子字串` 只跑名稱含它的收據。`RS2_MONO_HD_STRIDE=N` 每 N 筆等待取樣一次（預設 10；`tools/rust.sh` 不會把它帶進容器，
//! 要改就直接改預設值）。狀態逐筆等待都比對，取樣只影響縮成 320x200 的畫面比對與統計。
//! 比對筆數為 0、任何一份收據缺少、真 HD 載入 0 張都算失敗。

use rs2_game::{DrawCmd, Exit, Fixes, Game, Input};
use rs2_gfx::{MonoAssets, MonoHdSink, MonoKind, MonoSink};
use rs2_trace::mirror::{Mirror, a};
use rs2_trace::mono::{MonoReceipt, load_mono_receipt};
use rs2_trace::wait::entry_of;
use rs2_trace::{Inputs, load_inputs};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 規格 006 的收據清單，同 `tests/mono.rs`。
const EXPECTED: [&str; 14] = [
    "wm_flow-c",
    "wm_flow-h",
    "wm_menu-c",
    "wm_menu-h",
    "wm_title-c",
    "wm_title-h",
    "wm_stage1-c",
    "wm_stage1-h",
    "wm_stage2-c",
    "wm_stage2-h",
    "wm_stage3-c",
    "wm_stage3-h",
    "wm_stage4-c",
    "wm_stage4-h",
];

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 跑")))
}

fn receipt_root() -> PathBuf {
    if let Some(p) = std::env::var_os("RS2_DOSRUN_MODES") {
        return PathBuf::from(p);
    }
    let local = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../workspace/dosrun-modes");
    if local.join(EXPECTED[0]).is_dir() {
        return local;
    }
    env_path("RS2_DOSRUN")
}

/// 核心的兩個頁偏移。
const PAGES: [u16; 2] = [0, 0x8000];

// ---- 縮成 320x200 ----

/// SD 顯示記憶體的邏輯畫面：320x200，每點是 2 個單色像素的亮點數乘 8（0、8、16），與 HD 的 0 至 16 同尺度。
/// 位址配置在這裡獨立寫一次：偶數列 `n/2 × 50h`，奇數列加 2000h（CGA）或 4000h（MGA），頁在 0 與 4000h（CGA）或 8000h（MGA）。
fn sd_logical(vram: &[u8], kind: MonoKind, core_page: u16) -> Vec<u8> {
    let page = match kind {
        MonoKind::Cga => {
            if core_page == 0 {
                0usize
            } else {
                0x4000
            }
        }
        MonoKind::Mga => core_page as usize,
    };
    let odd = match kind {
        MonoKind::Cga => 0x2000usize,
        MonoKind::Mga => 0x4000,
    };
    let mut out = vec![0u8; 320 * 200];
    for ly in 0..200usize {
        let base = page + (ly / 2) * 0x50 + (ly % 2) * odd;
        for lx in 0..320usize {
            let byte = vram[base + lx / 4];
            let m = 0x80u8 >> (2 * (lx % 4));
            let on = (byte & m != 0) as u8 + (byte & (m >> 1) != 0) as u8;
            out[ly * 320 + lx] = on * 8;
        }
    }
    out
}

/// HD 的 1280x800 RGBA 縮成 320x200：每個邏輯像素是 HD 4x4 區塊的亮點數（0 至 16）。
fn hd_logical(rgba: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 320 * 200];
    for ly in 0..200usize {
        for lx in 0..320usize {
            let mut n = 0u8;
            for r in 0..4 {
                let row = (ly * 4 + r) * 1280 + lx * 4;
                for c in 0..4 {
                    n += (rgba[(row + c) * 4] != 0) as u8;
                }
            }
            out[ly * 320 + lx] = n;
        }
    }
    out
}

/// `size`x`size` 邏輯像素一格的平均亮度（0.0 至 16.0）。
fn cells(img: &[u8], size: usize) -> Vec<f32> {
    let (nx, ny) = (320 / size, 200 / size);
    let mut out = vec![0f32; nx * ny];
    for cy in 0..ny {
        for cx in 0..nx {
            let mut s = 0u32;
            for y in 0..size {
                for x in 0..size {
                    s += img[(cy * size + y) * 320 + cx * size + x] as u32;
                }
            }
            out[cy * nx + cx] = s as f32 / (size * size) as f32;
        }
    }
    out
}

// ---- 統計 ----

#[derive(Clone, Copy, Debug, Default)]
struct Stat {
    /// 比對的邏輯像素數。
    px: u64,
    /// 亮度不同的像素數。
    diff: u64,
    /// 亮度差超過一半（> 8/16）的像素數。
    gross: u64,
    /// 亮度差的總和（以 1/16 為單位）。
    abs16: u64,
    /// 8x8、4x4 格平均亮度差（以 1/16 為單位）的總和與格數。
    c8_abs: f64,
    c8_n: u64,
    c4_abs: f64,
    c4_n: u64,
    /// 取樣點中 gross 比例最高的一個。
    worst_gross: f64,
}

impl Stat {
    fn add(&mut self, a: &[u8], b: &[u8]) {
        let (mut diff, mut gross, mut abs16) = (0u64, 0u64, 0u64);
        for (&x, &y) in a.iter().zip(b) {
            let d = (x as i32 - y as i32).unsigned_abs() as u64;
            diff += (d != 0) as u64;
            gross += (d > 8) as u64;
            abs16 += d;
        }
        self.px += a.len() as u64;
        self.diff += diff;
        self.gross += gross;
        self.abs16 += abs16;
        self.worst_gross = self.worst_gross.max(gross as f64 / a.len() as f64);
        for (x, y) in cells(a, 8).into_iter().zip(cells(b, 8)) {
            self.c8_abs += (x - y).abs() as f64;
            self.c8_n += 1;
        }
        for (x, y) in cells(a, 4).into_iter().zip(cells(b, 4)) {
            self.c4_abs += (x - y).abs() as f64;
            self.c4_n += 1;
        }
    }

    fn diff_ratio(&self) -> f64 {
        self.diff as f64 / self.px.max(1) as f64
    }

    /// 亮度差超過一半的像素比例。
    fn gross_ratio(&self) -> f64 {
        self.gross as f64 / self.px.max(1) as f64
    }

    /// 逐像素平均亮度差，0 至 1。
    fn mae(&self) -> f64 {
        self.abs16 as f64 / 16.0 / self.px.max(1) as f64
    }

    /// 8x8 格平均亮度的平均差，0 至 1。抖動只改變格內像素的分布，格平均亮度大致守恆。
    fn c8(&self) -> f64 {
        self.c8_abs / 16.0 / self.c8_n.max(1) as f64
    }

    /// 4x4 格平均亮度的平均差，0 至 1。
    fn c4(&self) -> f64 {
        self.c4_abs / 16.0 / self.c4_n.max(1) as f64
    }
}

/// 命令的位置位移，給對照組用。
fn shifted(c: &DrawCmd, dx: u16, dy: u16) -> DrawCmd {
    match c {
        DrawCmd::Blit(b) => {
            let mut b = *b;
            b.x = b.x.wrapping_add(dx);
            b.y = b.y.wrapping_add(dy);
            DrawCmd::Blit(b)
        }
        other => *other,
    }
}

/// 一份收據的接收端與統計。
struct Run {
    kind: MonoKind,
    sd: MonoSink,
    nn: MonoHdSink,
    hd: MonoHdSink,
    /// 對照組：命令的 x 加 1、y 加 4 的真 HD。
    hd_dx: MonoHdSink,
    hd_dy: MonoHdSink,
    /// 最近鄰且命令的 x 加 1：嚴格比對應該抓得到（不同像素數要大於 0），證明嚴格驗證有鑑別力。
    nn_dx: MonoHdSink,
    nn_dx_stat: Stat,
    stride: usize,
    waits: usize,
    samples: usize,
    nn_stat: Stat,
    hd_stat: Stat,
    ctl_dx: Stat,
    ctl_dy: Stat,
    /// 真 HD 對上一個取樣點的 SD（相隔 `stride` 筆等待，場景已變動）。
    ctl_time: Stat,
    prev_sd: Option<Vec<Vec<u8>>>,
    /// 各繪製命令套用的次數。
    ops: std::collections::BTreeMap<String, u64>,
}

impl Run {
    fn new(rc: &MonoReceipt, assets: &Arc<MonoAssets>, hd_dir: &Path, stride: usize) -> Run {
        let mk = |dir: &Path| {
            let mut s = MonoHdSink::with_kind(rc.kind, Arc::clone(assets), dir);
            s.load_snapshot(&rc.vmem, &rc.sa, &rc.sb);
            s.set_counter_font(&rc.font);
            s
        };
        let mut sd = MonoSink::new(rc.kind, Arc::clone(assets));
        sd.load_snapshot(&rc.vmem, &rc.sa, &rc.sb);
        sd.set_counter_font(&rc.font);
        Run {
            kind: rc.kind,
            sd,
            nn: mk(Path::new("/nonexistent-rs2-hd")),
            hd: mk(hd_dir),
            hd_dx: mk(hd_dir),
            hd_dy: mk(hd_dir),
            nn_dx: mk(Path::new("/nonexistent-rs2-hd")),
            nn_dx_stat: Stat::default(),
            stride,
            waits: 0,
            samples: 0,
            nn_stat: Stat::default(),
            hd_stat: Stat::default(),
            ctl_dx: Stat::default(),
            ctl_dy: Stat::default(),
            ctl_time: Stat::default(),
            prev_sd: None,
            ops: std::collections::BTreeMap::new(),
        }
    }

    fn apply(&mut self, cmds: &[DrawCmd]) {
        for c in cmds {
            let key = match c {
                DrawCmd::Blit(b) => format!("{:?}", b.op),
                DrawCmd::VramFill { .. } => "VramFill".to_string(),
                _ => "其他".to_string(),
            };
            *self.ops.entry(key).or_default() += 1;
            self.sd.apply(c);
            self.nn.apply(c);
            self.hd.apply(c);
            self.nn_dx.apply(&shifted(c, 1, 0));
            self.hd_dx.apply(&shifted(c, 1, 0));
            self.hd_dy.apply(&shifted(c, 0, 4));
        }
    }

    /// 第 `n` 次等待時（接收端是「這次等待之前」累積的全部命令）。
    fn wait(&mut self, n: usize, name: &str) {
        self.waits += 1;
        self.check_blocks(n, name);
        if n % self.stride == 0 {
            self.sample(n, name);
        }
    }

    /// 最近鄰 HD 的每個區塊必須等於 SD 位元組的放大。MGA 的 2000h 至 3FFFh（及 A000h 至 BFFFh）是 SD 專用的掃描線複本
    /// （`EndOfFrameCopy`，HD 不動作），不比。
    fn check_blocks(&self, n: usize, name: &str) {
        let skip = |off: usize| self.kind == MonoKind::Mga && (0x2000..0x4000).contains(&(off & 0x7FFF));
        let (sv, hv) = (self.sd.vram(), self.nn.vram_blocks());
        assert_eq!(sv.len(), hv.len());
        for i in 0..sv.len() {
            if !skip(i) && MonoHdSink::nn_block(sv[i]) != hv[i] {
                panic!("{name} 第 {n} 筆等待：顯示記憶體 {i:04X} SD {:02X} 的放大不等於最近鄰 HD 區塊 {:016X}", sv[i], hv[i]);
            }
        }
        for (what, s, h) in [("暫存區 A", self.sd.scratch_a(), self.nn.scratch_a_blocks()), ("暫存區 B", self.sd.scratch_b(), self.nn.scratch_b_blocks())] {
            for i in 0..s.len() {
                if MonoHdSink::nn_block(s[i]) != h[i] {
                    panic!("{name} 第 {n} 筆等待：{what} {i:04X} SD {:02X} 的放大不等於最近鄰 HD 區塊 {:016X}", s[i], h[i]);
                }
            }
        }
        assert_eq!(self.sd.anomalies(), self.nn.anomalies(), "{name} 第 {n} 筆等待：異常次數");
    }

    fn sample(&mut self, n: usize, name: &str) {
        self.samples += 1;
        let mut buf = Vec::new();
        let mut cur_sd = Vec::new();
        for (k, &page) in PAGES.iter().enumerate() {
            let sd = sd_logical(self.sd.vram(), self.kind, page);
            self.nn.render_rgba(page, &mut buf);
            let nn = hd_logical(&buf);
            if let Some(i) = (0..sd.len()).find(|&i| sd[i] != nn[i]) {
                panic!("{name} 第 {n} 筆等待、頁 {page:04X}：最近鄰 HD 縮回 320x200 在 ({}, {}) 為 {}，SD 為 {}", i % 320, i / 320, nn[i], sd[i]);
            }
            self.nn_stat.add(&sd, &nn);
            self.hd.render_rgba(page, &mut buf);
            let hd = hd_logical(&buf);
            self.hd_stat.add(&sd, &hd);
            self.nn_dx.render_rgba(page, &mut buf);
            self.nn_dx_stat.add(&sd, &hd_logical(&buf));
            self.hd_dx.render_rgba(page, &mut buf);
            self.ctl_dx.add(&sd, &hd_logical(&buf));
            self.hd_dy.render_rgba(page, &mut buf);
            self.ctl_dy.add(&sd, &hd_logical(&buf));
            if let Some(prev) = &self.prev_sd {
                self.ctl_time.add(&prev[k], &hd);
            }
            cur_sd.push(sd);
        }
        self.prev_sd = Some(cur_sd);
    }
}

/// 重播一份收據並餵所有接收端，邏輯同 `rs2_trace::mono::replay_mono`（輸入、poke、步進），不比對資料段與雜湊
/// （那些已由 `tests/mono.rs` 逐筆驗證）。回傳走完的等待筆數。
fn replay(inputs: &Inputs, rc: &MonoReceipt, run: &mut Run) -> Result<usize, String> {
    let total = rc.rows.len();
    let entry = entry_of(rc.name.strip_prefix("wm_").map(|s| format!("wt_{s}")).as_deref().unwrap_or(&rc.name))?;
    let m = Mirror::from_dump(&rc.entry_ds);
    let mut g = Game::from_state(inputs.game_files(), m.load_bytes(&rc.entry_ds), entry, Fixes::NONE);
    g.set_terrain_scr(true);
    run.apply(&g.take_draws());
    let mut done = 0usize;
    let (mut fed_row, mut fed_n, mut polls, mut polls_for) = (usize::MAX, 0usize, 0usize, 0usize);
    let (mut started, mut next_poke, mut idle, mut last_done) = (false, 0usize, 0usize, 0usize);
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
        let res = {
            let g_ref = &mut g;
            let (rows, done_ref, run_ref) = (&rc.rows, &mut done, &mut *run);
            let name = rc.name.as_str();
            catch_unwind(AssertUnwindSafe(|| {
                g_ref.step_full(&Input { scancodes: &sc }, &mut |_g: &Game, _pt: u32| {}, &mut |_gg: &Game, _cx: u16| {
                    if *done_ref >= rows.len() {
                        return;
                    }
                    run_ref.wait(*done_ref, name);
                    *done_ref += 1;
                })
            }))
        };
        let frame = match res {
            Ok(f) => f,
            Err(p) => {
                let msg = if let Some(s) = p.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = p.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "<非字串的 panic>".to_string()
                };
                if done >= total {
                    break;
                }
                return Err(format!("第 {done} 筆附近 panic：{msg}"));
            }
        };
        run.apply(&frame.draws);
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
            return Err(format!("連續 {idle} 步沒有走到下一次等待，停在第 {done} 筆"));
        }
        if let Some(e) = frame.exit {
            if done < total {
                let name = match e {
                    Exit::End => "E",
                    Exit::Quit => "Q",
                    Exit::Clear => "C",
                };
                return Err(format!("在第 {done} 筆之前離開（{name}），原版還有 {} 筆", total - done));
            }
            break;
        }
    }
    Ok(done)
}

#[test]
#[ignore]
fn mono_hd_matches_mono_sink() {
    let only = std::env::var("RS2_MONO").unwrap_or_default();
    let stride: usize = std::env::var("RS2_MONO_HD_STRIDE").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
    let root = receipt_root();
    let orig = env_path("RS2_ORIG");
    let hd_dir = env_path("RS2_HD");
    let inputs = load_inputs(&orig).expect("載入原版");
    let assets = Arc::new(MonoAssets::load(&orig).expect("載入單色圖庫"));
    let mut fails: Vec<String> = Vec::new();
    let (mut files, mut waits, mut samples) = (0usize, 0usize, 0usize);
    // 各指標：真 HD 的最大值，與三個對照組的最小值。
    let mut pos = [0f64; 3];
    let mut ctl = [[f64::MAX; 3]; 3];
    let mut worst_ratio = 0f64;
    let mut ops: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    println!("取樣間隔 {stride} 筆等待；每份收據取樣兩個顯示頁；SD 與 HD 都縮成 320x200 比較（亮度 0 至 16）");
    println!("最近鄰 HD：縮回 320x200 後與 SD 不同的像素數 / 比對的像素數（要為 0）");
    println!("真 HD：不同像素比例 | 差過半比例 | 逐像素平均亮度差 | 8x8 格平均亮度差 | 4x4 格平均亮度差 | 最差取樣的差過半比例 | HD 載入張數 缺圖數");
    println!("對照組（x+1、y+4、相隔一個取樣間隔的另一幀）：差過半比例 | 8x8 格 | 4x4 格");
    for n in EXPECTED.iter().filter(|n| n.contains(only.as_str())) {
        let rc = match load_mono_receipt(&root.join(n)) {
            Ok(rc) => rc,
            Err(e) => {
                fails.push(format!("{n}：收據載入失敗：{e}"));
                continue;
            }
        };
        let mut run = Run::new(&rc, &assets, &hd_dir, stride);
        match replay(&inputs, &rc, &mut run) {
            Ok(done) if done == rc.rows.len() => {}
            Ok(done) => fails.push(format!("{n}：只走完 {done}/{} 筆", rc.rows.len())),
            Err(e) => fails.push(format!("{n}：{e}")),
        }
        files += 1;
        waits += run.waits;
        samples += run.samples;
        let s = &run.hd_stat;
        println!(
            "{n:12} 等待 {:5} 取樣 {:3} | 最近鄰 {}/{} | 真 HD {:6.2}% {:6.2}% {:.4} {:.4} {:.4} {:6.2}% | HD {} 缺 {}",
            run.waits,
            run.samples,
            run.nn_stat.diff,
            run.nn_stat.px,
            s.diff_ratio() * 100.0,
            s.gross_ratio() * 100.0,
            s.mae(),
            s.c8(),
            s.c4(),
            s.worst_gross * 100.0,
            run.hd.hd_loaded(),
            run.hd.missing().len()
        );
        let ctls = [("x+1", &run.ctl_dx), ("y+4", &run.ctl_dy), ("另一幀", &run.ctl_time)];
        for (label, c) in ctls {
            println!("{:12}   對照 {label:4} | {:6.2}% {:.4} {:.4}", "", c.gross_ratio() * 100.0, c.c8(), c.c4());
        }
        if !run.hd.missing().is_empty() {
            println!("    缺圖：{:?}", &run.hd.missing()[..run.hd.missing().len().min(8)]);
        }
        if run.nn_stat.diff != 0 {
            fails.push(format!("{n}：最近鄰 HD 縮回 320x200 有 {} 個像素與 SD 不同", run.nn_stat.diff));
        }
        println!("{:12}   鑑別力：最近鄰且 x+1 的不同像素 {}/{}（要大於 0）", "", run.nn_dx_stat.diff, run.nn_dx_stat.px);
        if run.nn_dx_stat.diff == 0 {
            fails.push(format!("{n}：最近鄰且 x+1 的嚴格比對一個像素都沒抓到，比對沒有鑑別力"));
        }
        if run.samples == 0 || run.nn_stat.px == 0 {
            fails.push(format!("{n}：沒有任何取樣"));
        }
        if run.hd.hd_loaded() == 0 {
            fails.push(format!("{n}：真 HD 一張都沒載入（RS2_HD={}）", hd_dir.display()));
        }
        let metrics = |s: &Stat| [s.gross_ratio(), s.c8(), s.c4()];
        for (m, v) in metrics(s).into_iter().enumerate() {
            pos[m] = pos[m].max(v);
        }
        for (ci, c) in [&run.ctl_dx, &run.ctl_dy, &run.ctl_time].into_iter().enumerate() {
            if c.px > 0 {
                for (m, v) in metrics(c).into_iter().enumerate() {
                    ctl[ci][m] = ctl[ci][m].min(v);
                }
            }
        }
        let gates = [GATE_GROSS, GATE_C8, GATE_C4];
        for (m, (v, gate)) in metrics(s).into_iter().zip(gates).enumerate() {
            if v > gate {
                fails.push(format!("{n}：{} {v:.4} 超過門檻 {gate}", ["差過半比例", "8x8 格平均亮度差", "4x4 格平均亮度差"][m]));
            }
        }
        // 對位檢查：對的位置要比錯位的對照組更接近 SD（格平均亮度差，兩種格大小）。
        for (label, c) in [("x+1", &run.ctl_dx), ("y+4", &run.ctl_dy), ("另一幀", &run.ctl_time)] {
            if c.px == 0 {
                continue;
            }
            for (what, p, q) in [("8x8", s.c8(), c.c8()), ("4x4", s.c4(), c.c4())] {
                let ratio = p / q;
                worst_ratio = worst_ratio.max(ratio);
                if ratio >= GATE_REGISTRATION {
                    fails.push(format!("{n}：{what} 格平均亮度差 {p:.4} 沒有明顯小於對照 {label} 的 {q:.4}（比值 {ratio:.3}，門檻 {GATE_REGISTRATION}）"));
                }
            }
        }
        for (k, v) in &run.ops {
            *ops.entry(k.clone()).or_default() += v;
        }
    }
    println!("合計：{files} 份收據，{waits} 筆等待逐筆比對狀態，{samples} 個取樣點");
    println!("真 HD 最大值（差過半、8x8 格、4x4 格）：{:.4} {:.4} {:.4}", pos[0], pos[1], pos[2]);
    for (label, c) in ["x+1", "y+4", "另一幀"].into_iter().zip(ctl) {
        println!("對照 {label} 的最小值：{:.4} {:.4} {:.4}", c[0], c[1], c[2]);
    }
    println!("門檻：差過半 {GATE_GROSS}、8x8 格 {GATE_C8}、4x4 格 {GATE_C4}、對位比值 {GATE_REGISTRATION}（實測最大比值 {worst_ratio:.3}）");
    let unseen: Vec<&str> = ALL_OPS.iter().copied().filter(|o| !ops.contains_key(*o)).collect();
    println!("{files} 份收據用到的繪製命令：{} 種，沒用到：{unseen:?}", ops.len());
    println!("次數：{ops:?}");
    assert!(files > 0 && waits > 0 && samples > 0, "沒有比對任何一筆（RS2_MONO={only:?}，收據根 {}）", root.display());
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

/// 真 HD 對 SD 的門檻（逐收據的聚合值），依據是 14 份收據的實測（2026-11-05，取樣間隔 10，1494 取樣點，
/// 推導圖 `hd-variants-manifest.json`）：
///
/// | 指標 | 實測最大 | 發生在 | 門檻 |
/// |---|---|---|---|
/// | 亮度差超過一半的像素比例 | 0.0798 | `wm_menu-*` | 0.10 |
/// | 8x8 格平均亮度差 | 0.1318 | `wm_menu-*` | 0.16 |
/// | 4x4 格平均亮度差 | 0.1422 | `wm_menu-*` | 0.17 |
///
/// 門檻取實測最大值再放寬約兩成。重播是決定性的，數值不隨機，所以門檻只在 HD 圖重新產生或接收端行為改變時才會動。
/// 加入選單收據之前（12 份、間隔 10）最大值是 0.0697、0.1158、0.1265，間隔 50 是 0.0637、0.1134、0.1231。
/// 推導圖是 HD 彩色圖轉灰階再抖動，與原版 1 位元美術的色調、圖樣本來就不同，所以絕對門檻抓不到位移這類錯誤
/// （x+1、y+4 的對照組只比正確位置高約 5% 至 110%），它只防整體退化（圖全黑、反相、缺圖）。
/// 抓位置與索引錯誤靠兩處：最近鄰替換後逐像素相同的嚴格驗證，以及下面的對位比值。
const GATE_GROSS: f64 = 0.10;
const GATE_C8: f64 = 0.16;
const GATE_C4: f64 = 0.17;

/// 對位檢查：正確位置的格平均亮度差除以錯位對照組（x+1、y+4、另一幀）的值，每份收據兩種格大小都要小於它，
/// 也就是正確位置不得比錯位更差。實測最大 0.957（`wm_menu-*` 的 8x8 對 x+1，選單大片平坦、位移幾乎看不出來），
/// 其他收據最大 0.918（`wm_flow-*`）。
const GATE_REGISTRATION: f64 = 1.0;

const ALL_OPS: [&str; 43] = [
    "MyOverwrite", "MyOverlay", "MyCut", "MyFlash", "MyOverlayEffect", "MyCutEffect", "MyCutPlayerShot", "MyOverlayPlayerShot", "StageCut", "StageOverlay", "StageOverwrite", "StageFlash", "StageToScratch", "ScratchReplace", "ClearDrawPage",
    "ScratchReplaceShifted", "ScratchOr", "ScratchAnd", "FillRows", "BarCell", "Number", "ClearScratchB", "ClearScratchA", "ClearBothPages", "MyOverwriteInverted", "BegOverwrite", "BegInverted", "BegToScratch", "ScratchReplaceMirrored", "ScratchOrPairs",
    "ScratchOrMirrored", "ScratchLatchPairs", "ScratchAndPairs", "ScratchAndMirrored", "MapToScratch", "ScrollRotateCopy", "ScrollShiftLeft4", "ScrollRotateColumn", "ScrollMergeSeam", "ScrollShowA", "ScrollShowB", "ScrollShiftBothLeft", "EndOfFrameCopy",
];
