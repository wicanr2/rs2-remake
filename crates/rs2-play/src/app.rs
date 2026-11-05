//! 主迴圈：視窗模式（SDL 事件、固定步長、補間顯示）與 `--headless-steps`（同一段步進與繪製程式，沒有事件）。

use crate::args::{Args, StartArg, ToggleName};
use crate::audio::{Output, WavOut};
use crate::controls::{Action, Controls, Effect, PanelChange, PanelOpen, Resize};
use crate::input::{Hotkey, InputMapper, KeyResult, Source};
use crate::keymap::{Btn, KeyMap, PadMap, split_pseudo};
use crate::pacer::Pacer;
use crate::pad::{Devices, PadEvent, PadMapper, STICK_ENGAGE, STICK_RELEASE};
use crate::resources::{self, Env, Os};
use crate::session::{Begin, Outcome, Session, ViewOpts};
use crate::options::HIT_FEEDBACK_INDEX;
use crate::settings::{self, Settings};
use crate::toolbar::{BarState, Button, desktop_height, metrics};
use crate::touch::TouchMapper;
use crate::ui::panel::{self, Cx, Page, Panel, PanelOut};
use crate::ui::{self, text};
use crate::view::{Fit, Overlay, Presenter, ShotInfo, UiLayer, window_size};
use rs2_audio::Engine;
use rs2_core::{Fixes, Start};
use rs2_gfx::{Assets, HdAssets, MonoAssets};
use rs2_trace::{Inputs, load_inputs, parse_sc};
use sdl2::controller::{Axis, GameController};
use sdl2::event::{Event, WindowEvent};
use sdl2::joystick::Joystick;
use sdl2::keyboard::{Mod, Scancode};
use sdl2::mouse::MouseButton;
use sdl2::render::WindowCanvas;
use sdl2::video::FullscreenType;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// 一次執行的共用資料。
struct Ctx {
    args: Args,
    inputs: Inputs,
    assets: Assets,
    /// CGA、MGA 用的單色圖庫。載入失敗是 `None`：這兩種模式不能選，VGA 與 EGA 不受影響。
    mono: Option<Arc<MonoAssets>>,
    settings: Settings,
    cfg_dir: Option<PathBuf>,
    replay: Vec<Vec<u8>>,
    hd_dir: Option<PathBuf>,
}

type HdResult = Result<(HdAssets, usize, usize, Duration, usize), String>;

fn hd_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 4)
}

/// 背景開啟並預載 HD 資料夾。
fn spawn_hd(dir: PathBuf, assets: Assets) -> JoinHandle<HdResult> {
    std::thread::spawn(move || {
        let mut hd = HdAssets::open(&dir)?;
        let threads = hd_threads();
        let t0 = Instant::now();
        let (files, bytes) = hd.preload(&assets, threads);
        Ok((hd, files, bytes, t0.elapsed(), threads))
    })
}

/// 等 HD 預載完成並印耗時與缺圖。
fn finish_hd(h: Option<JoinHandle<HdResult>>) -> Result<Option<HdAssets>, String> {
    let Some(h) = h else { return Ok(None) };
    let (hd, files, bytes, dt, threads) = h.join().map_err(|_| "HD 預載執行緒當掉".to_string())??;
    let missing = hd.missing();
    eprintln!(
        "[hd] 預載 {files} 檔、{:.1} MiB，耗時 {:.2} 秒（{threads} 執行緒），缺圖 {} 項",
        bytes as f64 / (1024.0 * 1024.0),
        dt.as_secs_f64(),
        missing.len()
    );
    for m in missing.iter().take(30) {
        eprintln!("[hd] 缺圖：{m}");
    }
    if missing.len() > 30 {
        eprintln!("[hd] 另有 {} 項缺圖未列出", missing.len() - 30);
    }
    Ok(Some(hd))
}

fn ticks_to_us(t: u64, freq: u64) -> u64 {
    (t as u128 * 1_000_000 / freq.max(1) as u128) as u64
}

fn make_window(video: &sdl2::VideoSubsystem, w: u32, h: u32) -> Result<sdl2::video::Window, String> {
    let mut b = video.window("RS-2 remake", w, h);
    if cfg!(target_os = "android") {
        // 全螢幕才會隱藏狀態列與導覽列，否則導覽列蓋住右側的控制鈕，SDL 回報的輸出大小也不扣除它。
        b.fullscreen_desktop();
    } else {
        b.position_centered().resizable();
    }
    b.build().map_err(|e| format!("建立視窗失敗：{e}"))
}

/// 先試要求垂直同步的繪圖器；建不起來（`into_canvas` 會吃掉視窗）就重建視窗，用一般繪圖器。
/// 第二個回傳值是繪圖器是否真的有垂直同步。
fn make_canvas(video: &sdl2::VideoSubsystem, w: u32, h: u32) -> Result<(WindowCanvas, bool), String> {
    let has_vsync = |c: &WindowCanvas| c.info().flags & (sdl2::sys::SDL_RendererFlags::SDL_RENDERER_PRESENTVSYNC as u32) != 0;
    match make_window(video, w, h)?.into_canvas().present_vsync().build() {
        Ok(c) => {
            let v = has_vsync(&c);
            Ok((c, v))
        }
        Err(e) => {
            eprintln!("[window] 要求垂直同步的繪圖器建立失敗（{e}），改用一般繪圖器");
            let c = make_window(video, w, h)?.into_canvas().build().map_err(|e| format!("建立 SDL 繪圖器失敗：{e}"))?;
            let v = has_vsync(&c);
            Ok((c, v))
        }
    }
}

fn begin_of(a: &Args) -> Begin {
    match a.start {
        StartArg::Title => Begin::Title,
        StartArg::NewGame => Begin::Stage1(Start::NewGame { difficulty: None }),
        StartArg::Demo => Begin::Stage1(Start::Demo),
    }
}

fn make_engine(inputs: &Inputs) -> Result<Engine, String> {
    let get = |n: &str| inputs.files.get(n).ok_or_else(|| format!("缺 {n}"));
    Engine::new(get("A.MIX")?, get("B.MIX")?)
}

fn prepare(args: Args) -> Result<Ctx, String> {
    let os = Os::current();
    let env = Env::from_process();
    let found = resources::resolve(args.orig.as_deref(), args.hd.as_deref(), args.no_hd, &env, os)?;
    for n in &found.notes {
        eprintln!("[資源] {n}");
    }
    let inputs = load_inputs(&found.orig)?;
    let assets = Assets::load(&found.orig)?;
    eprintln!("[init] 原版檔雜湊驗證通過（{}）", found.orig.display());
    // 單色圖庫（CGA、MGA）另外載入：缺檔或雜湊不符時只有這兩種模式不能用，不拖累 VGA 與 EGA。
    let mono = match MonoAssets::load(&found.orig) {
        Ok(m) => {
            if !m.unverified.is_empty() {
                eprintln!("[單色] 警告：{} 個圖庫沒有預期雜湊：{:?}", m.unverified.len(), m.unverified);
            }
            eprintln!("[單色] 單色圖庫雜湊驗證通過（CGA、MGA 可用）");
            Some(Arc::new(m))
        }
        Err(e) => {
            eprintln!("[單色] 單色圖庫載入失敗：{e}；CGA、MGA 不能選，VGA 與 EGA 不受影響");
            None
        }
    };
    let replay = match &args.replay {
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("讀不到收據 {}：{e}", p.display()))?;
            let r: Vec<Vec<u8>> = text.lines().filter(|l| !l.trim().is_empty()).map(parse_sc).collect();
            if r.is_empty() {
                return Err(format!("收據 {} 是空的", p.display()));
            }
            eprintln!("[init] 收據 {}：{} 步", p.display(), r.len());
            r
        }
        None => Vec::new(),
    };
    let cfg_dir = args.config.clone().or_else(|| settings::config_dir(os, &env));
    let (mut st, warns) = match &cfg_dir {
        Some(d) => Settings::load(d),
        None => (Settings::default(), Vec::new()),
    };
    for w in warns {
        eprintln!("[設定] 警告：{w}");
    }
    if let Some(s) = args.scale {
        st.scale = s;
    }
    if let Some(t) = args.tween {
        st.tween = t;
    }
    if let Some(c) = args.counter {
        st.counter = c;
    }
    if let Some(f) = args.feel {
        st.feel = f;
    }
    if let Some(m) = args.mode {
        st.mode = m;
    }
    if let Some(f) = args.feedback {
        st.set_opt_value(HIT_FEEDBACK_INDEX, f as u8);
    }
    let hd_dir = found.hd;
    Ok(Ctx { args, inputs, assets, mono, settings: st, cfg_dir, replay, hd_dir })
}

/// 前端開關狀態。告知單色圖庫能不能用：不能用時，設定或 `--mode` 要求的 CGA、MGA 退回 VGA。
fn new_controls(cx: &Ctx) -> Controls {
    let mut ctl = Controls::new(cx.settings, cx.hd_dir.is_some());
    if ctl.set_mono_available(cx.mono.is_some()) {
        eprintln!("[單色] 要求的顯示模式 {} 需要單色圖庫，改用 vga", cx.settings.mode.code());
    }
    ctl
}

/// 進入點。回傳程式結束碼。
pub fn run(args: Args) -> Result<i32, String> {
    let cx = prepare(args)?;
    match cx.args.headless_steps {
        Some(n) => run_headless(cx, n),
        None => run_window(cx),
    }
}

fn build_session(cx: &Ctx, hd: Option<HdAssets>, view: ViewOpts) -> Session {
    let fixes = if cx.args.original_bugs { Fixes::NONE } else { Fixes::ALL };
    Session::new(&cx.inputs, cx.assets.clone(), cx.mono.clone(), hd, begin_of(&cx.args), fixes, view)
}

/// 無敵（規格 005）：開著時每步呼叫，從沒開過就不呼叫，所以全部選項為預設時模擬與原版逐位元組相同。
/// 關掉時只還原一次。
fn apply_invincible(session: &mut Session, on: bool, applied: &mut bool) {
    if on {
        session.set_invincible(true);
        *applied = true;
    } else if *applied {
        session.set_invincible(false);
        *applied = false;
    }
}

// ---------------------------------------------------------------------------
// 共用：統計、雜湊、存圖
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Stats {
    steps: u64,
    step_us: u64,
    step_max_us: u64,
    renders: u64,
    render_us: u64,
    render_max_us: u64,
    presents: u64,
    present_us: u64,
    present_max_us: u64,
}

impl Stats {
    fn step(&mut self, us: u64) {
        self.steps += 1;
        self.step_us += us;
        self.step_max_us = self.step_max_us.max(us);
    }
    fn render(&mut self, us: u64) {
        self.renders += 1;
        self.render_us += us;
        self.render_max_us = self.render_max_us.max(us);
    }
    fn present(&mut self, us: u64) {
        self.presents += 1;
        self.present_us += us;
        self.present_max_us = self.present_max_us.max(us);
    }
    fn report(&self) {
        // 行程記憶體尖峰（Linux 的 /proc/self/status，讀不到就略過）。
        if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
            if let Some(l) = s.lines().find(|l| l.starts_with("VmHWM:")) {
                eprintln!("[perf] 記憶體尖峰 {}", l.trim_start_matches("VmHWM:").trim());
            }
        }
        let avg = |tot: u64, n: u64| if n == 0 { 0.0 } else { tot as f64 / n as f64 / 1000.0 };
        eprintln!(
            "[perf] 步進 {} 次，平均 {:.2} ms（最大 {:.2}）；render {} 次，平均 {:.2} ms（最大 {:.2}）；繪製加 present {} 次，平均 {:.2} ms（最大 {:.2}）",
            self.steps,
            avg(self.step_us, self.steps),
            self.step_max_us as f64 / 1000.0,
            self.renders,
            avg(self.render_us, self.renders),
            self.render_max_us as f64 / 1000.0,
            self.presents,
            avg(self.present_us, self.presents),
            self.present_max_us as f64 / 1000.0,
        );
    }
}

fn report_shot(path: &Path, info: ShotInfo, step: u64) -> bool {
    let ok = info.nonblack > 0;
    eprintln!(
        "[shot] 第 {step} 步 {}：{}x{}，非黑像素 {:.1}%{}",
        path.display(),
        info.w,
        info.h,
        info.ratio() * 100.0,
        if ok { "" } else { "（全黑，失敗）" }
    );
    ok
}

fn write_hashes(path: &Option<PathBuf>, lines: &[String]) -> Result<(), String> {
    if let Some(p) = path {
        let mut s = lines.join("\n");
        if !lines.is_empty() {
            s.push('\n');
        }
        std::fs::write(p, s).map_err(|e| format!("寫不了 {}：{e}", p.display()))?;
        eprintln!("[hash] {} 行 -> {}", lines.len(), p.display());
    }
    Ok(())
}

fn log_missing(session: &mut Session) {
    for m in session.new_missing_hd() {
        eprintln!("[hd] 缺圖：{m}");
    }
}

/// 測試用的切換。打擊回饋是設定畫面的選項，沒有熱鍵，所以直接改設定。
fn apply_toggle(ctl: &mut Controls, t: ToggleName) {
    let a = match t {
        ToggleName::Tween => Action::Tween,
        ToggleName::Counter => Action::Counter,
        ToggleName::Hd => Action::Gfx,
        ToggleName::Feel => Action::Feel,
        ToggleName::Mode => Action::Mode,
        ToggleName::Feedback => {
            let v = ctl.settings.hit_feedback();
            ctl.settings.set_opt_value(HIT_FEEDBACK_INDEX, (!v) as u8);
            return;
        }
    };
    ctl.apply(a);
}

/// 收據的 `sc` 欄裡，分離鍵的虛擬碼（`keymap::split_pseudo`）表示那位玩家的分離鍵這一步按著。
/// 分離鍵沒有原版掃描碼（規格 005「砲座像 Force」），收據用它重播，不送進遊戲。回傳 (分離鍵狀態, 真正的掃描碼)。
fn split_codes(sc: &[u8]) -> ([bool; 2], Vec<u8>) {
    let mut held = [false; 2];
    let rest = sc
        .iter()
        .copied()
        .filter(|&c| match (0..2).find(|&p| c == split_pseudo(p)) {
            Some(p) => {
                held[p] = true;
                false
            }
            None => true,
        })
        .collect();
    (held, rest)
}

/// 把前端的開關交給 `Session`：畫面選項、HUD 右端的標籤、打擊回饋。
fn sync_session(session: &mut Session, ctl: &Controls) {
    session.set_view(ctl.view());
    session.set_hud_flags(ctl.hud_flags());
    session.set_hit_feedback(ctl.settings.hit_feedback());
    session.set_rules(ctl.settings.rules());
}

// ---------------------------------------------------------------------------
// --headless-steps
// ---------------------------------------------------------------------------

/// 不進事件迴圈，其餘與視窗模式相同：每步 `Session::step`，再各 render 一次 alpha 0.5 與 1.0 並交給 `Presenter`。
/// 有 `SDL_VIDEODRIVER=dummy` 時走完整的 SDL 貼圖路徑；沒有視訊就只做 render。
fn run_headless(cx: Ctx, steps: u64) -> Result<i32, String> {
    let sdl = sdl2::init().ok();
    let video = sdl.as_ref().and_then(|s| s.video().ok());
    let mut canvas: Option<WindowCanvas> = None;
    let mut ctl = new_controls(&cx);
    if let Some(v) = &video {
        let (w, h) = window_size(ctl.settings.scale, ctl.settings.toolbar);
        let built = v.window("RS-2 remake (headless)", w, h).hidden().build().map_err(|e| e.to_string()).and_then(|win| {
            win.into_canvas().build().map_err(|e| e.to_string())
        });
        match built {
            Ok(c) => {
                eprintln!("[headless] presenter: sdl（視訊驅動 {}，繪圖器 {}）", v.current_video_driver(), c.info().name);
                canvas = Some(c);
            }
            Err(e) => eprintln!("[headless] presenter: none（建立視窗失敗：{e}）"),
        }
    } else {
        eprintln!("[headless] presenter: none（SDL 視訊不可用）");
    }
    let tc = canvas.as_ref().map(|c| c.texture_creator());
    let mut pres = match &tc {
        Some(t) => Some(Presenter::new(t)?),
        None => None,
    };
    let hd = finish_hd(cx.hd_dir.clone().map(|d| spawn_hd(d, cx.assets.clone())))?;
    let mut session = build_session(&cx, hd, ctl.view());
    sync_session(&mut session, &ctl);
    log_missing(&mut session);
    eprintln!(
        "[headless] 起點 {:?}，{steps} 步，補間 {}，幀計數 {}，HD {}，顯示模式 {}",
        cx.args.start,
        ctl.settings.tween,
        ctl.settings.counter,
        session.hd_effective(),
        session.mode().code()
    );

    let mut hashes: Vec<String> = Vec::new();
    let mut shots = cx.args.shots.clone();
    let mut toggles = cx.args.toggles.clone();
    let mut stats = Stats::default();
    let mut shot_ok = true;
    let mut god_applied = false;
    let t_all = Instant::now();
    let mut stop_reason = "步數用完".to_string();
    let mut wav = match &cx.args.wav_out {
        Some(_) => Some(WavOut::new(make_engine(&cx.inputs)?)),
        None => None,
    };
    for _ in 0..steps {
        let done = session.steps();
        // 第 N 步之後的切換：在 steps == N 時、下一步之前套用。
        toggles.retain(|&(n, t)| {
            if n == done {
                apply_toggle(&mut ctl, t);
                false
            } else {
                true
            }
        });
        sync_session(&mut session, &ctl);
        // 沒有真實時鐘：介面層的時間用步數換算（一步約 110 毫秒），同樣的輸入得到同樣的畫面。
        session.set_clock_ms(session.steps() * 110);
        apply_invincible(&mut session, ctl.settings.invincible(), &mut god_applied);
        let (split, sc) = split_codes(cx.replay.get(done as usize).map(|v| v.as_slice()).unwrap_or(&[]));
        session.set_split_held(split);
        let t0 = Instant::now();
        let outcome = session.step(&sc);
        stats.step(t0.elapsed().as_micros() as u64);
        if let Some(w) = &mut wav {
            for m in session.take_audio() {
                w.send(&m);
            }
            w.advance(session.ticks());
        }
        hashes.push(session.dseg_hash());
        let n = session.steps();
        // 每步各畫一次 alpha 0.5 與 1.0（補間關閉時 alpha 沒有作用，但仍走同一段程式）。
        for alpha in [0.5f32, 1.0] {
            let shake = session.shake_offset();
            let t1 = Instant::now();
            let fit = session.fit();
            let (frame, fw, fh) = session.render(alpha);
            stats.render(t1.elapsed().as_micros() as u64);
            if let (Some(p), Some(c)) = (pres.as_mut(), canvas.as_mut()) {
                let shot_idx = if alpha == 1.0 { shots.iter().position(|(s, _)| *s == n) } else { None };
                let shot_path = shot_idx.map(|i| shots[i].1.clone());
                let t2 = Instant::now();
                p.shake = shake;
                let info = p.draw(c, frame, fw, fh, fit, &ctl.toolbar(), &ctl.bar_state(), None, None, shot_path.as_deref())?;
                stats.present(t2.elapsed().as_micros() as u64);
                if let (Some(i), Some(info)) = (shot_idx, info) {
                    let (s, path) = shots.remove(i);
                    shot_ok &= report_shot(&path, info, s);
                }
            }
        }
        log_missing(&mut session);
        if let Outcome::Ended(e) = outcome {
            stop_reason = format!("{e:?}");
            break;
        }
    }
    eprintln!("[headless] 結束：{stop_reason}，共 {} 步，{:.2} 秒", session.steps(), t_all.elapsed().as_secs_f64());
    if let (Some(w), Some(p)) = (&wav, &cx.args.wav_out) {
        w.write(p)?;
        eprintln!("[headless] 音訊寫到 {}", p.display());
    }
    let (a1, a2) = session.anomalies();
    eprintln!("[headless] 繪製異常計數 VgaSink {a1}、LayerSink {a2}，調色盤 {:?}", session.palette_name());
    if let Some((c, h)) = session.mono_anomalies() {
        eprintln!("[headless] 單色接收端異常計數 CGA {c}、MGA {h}");
    }
    stats.report();
    write_hashes(&cx.args.hash_out, &hashes)?;
    let mut rc = 0;
    if !shots.is_empty() {
        for (s, p) in &shots {
            eprintln!("[shot] 第 {s} 步沒有到達或沒有視窗，未存 {}", p.display());
        }
        rc = 4;
    }
    if !shot_ok {
        rc = 4;
    }
    Ok(rc)
}

// ---------------------------------------------------------------------------
// 視窗模式
// ---------------------------------------------------------------------------

fn save_settings(dir: &Option<PathBuf>, s: &Settings) {
    if let Some(d) = dir {
        if let Err(e) = s.save(d) {
            eprintln!("[設定] 警告：寫不了 {}：{e}", d.display());
        }
    }
}

/// 狀態列訊息顯示多久。
const NOTICE_US: u64 = 6_000_000;

/// 視窗模式的前端狀態：開關、設定畫面、手把清單與指派，以及三個按鍵來源的所有權。
struct Front {
    ctl: Controls,
    panel: Option<Panel>,
    last_page: Page,
    devs: Devices,
    pads: PadMapper,
    mapper: InputMapper,
    touch: Option<TouchMapper>,
    /// 狀態列的訊息與到期時間（微秒）。
    notice: Option<(String, u64)>,
    /// 設定畫面外部的狀態有變（裝置、設定、熱鍵）就加一，畫面要重畫。
    panel_gen: u64,
    /// 重綁手把按鈕時，LT、RT 兩個扳機目前有沒有按著（任何一支）。
    trig_down: [bool; 2],
    /// 已經交給 `mapper` 與 `pads` 的對應。
    synced: (KeyMap, PadMap),
    /// 滑鼠移過、按著的工具列按鈕（規格 005：移過變亮，按下變深）。
    hover_btn: Option<Button>,
    down_btn: Option<Button>,
    /// 設定畫面開啟的時間（毫秒），淡入用。
    panel_opened_ms: u64,
}

impl Front {
    fn new(ctl: Controls, touch: Option<TouchMapper>) -> Front {
        let mut mapper = InputMapper::new();
        mapper.set_keymap(ctl.settings.keys);
        let mut pads = PadMapper::new();
        pads.set_map(ctl.settings.pad);
        let synced = (ctl.settings.keys, ctl.settings.pad);
        Front { ctl, panel: None, last_page: Page::Game, devs: Devices::new(), pads, mapper, touch, notice: None, panel_gen: 0, trig_down: [false; 2], synced, hover_btn: None, down_btn: None, panel_opened_ms: 0 }
    }

    fn say(&mut self, s: String, now_us: u64) {
        eprintln!("[前端] {s}");
        self.notice = Some((s, now_us + NOTICE_US));
        self.panel_gen += 1;
    }

    /// 訊息過期就清掉。回傳有沒有清。
    fn expire_notice(&mut self, now_us: u64) -> bool {
        if matches!(&self.notice, Some((_, until)) if now_us >= *until) {
            self.notice = None;
            return true;
        }
        false
    }

    fn bar_state(&self) -> BarState {
        let mut b = self.ctl.bar_state();
        if let Some((s, _)) = &self.notice {
            b.notice = s.clone();
        }
        b.hover = self.hover_btn;
        b.down = self.down_btn;
        b
    }

    /// 手把或觸控的事件交給所有權計數。回傳有沒有按暫停鈕。
    fn feed(&mut self, src: Source, events: Vec<PadEvent>, now_us: u64) -> bool {
        let mut pause = false;
        for e in events {
            // Android 沒有別的地方看得到輸入，煙霧測試與回報問題都靠這行。
            if cfg!(target_os = "android") {
                eprintln!("[輸入] {src:?} {e:?}");
            }
            match e {
                PadEvent::Down(c) => self.mapper.code_down(src, c, now_us),
                PadEvent::Up(c) => self.mapper.code_up(src, c),
                PadEvent::Pause => pause = true,
            }
        }
        pause
    }

    /// 讓輸入層跟上目前的狀態：鍵盤與手把的對應、手把指派、前端暫停時擋住輸入。
    fn sync(&mut self, now_us: u64) {
        if self.synced.0 != self.ctl.settings.keys {
            self.mapper.set_keymap(self.ctl.settings.keys);
            self.synced.0 = self.ctl.settings.keys;
        }
        if self.synced.1 != self.ctl.settings.pad {
            self.pads.set_map(self.ctl.settings.pad);
            self.synced.1 = self.ctl.settings.pad;
        }
        let ev = self.pads.sync(&self.devs);
        if !ev.is_empty() {
            self.feed(Source::Pad, ev, now_us);
        }
        let want = self.ctl.frontend_paused();
        if want != self.mapper.is_blocked() {
            if want {
                // 先把手把與觸控自己的狀態清乾淨，再擋住。
                self.release_pads_and_touch(now_us);
                self.mapper.block();
            } else {
                self.mapper.unblock();
            }
        }
    }

    fn release_pads_and_touch(&mut self, now_us: u64) {
        let ev = self.pads.release_all();
        self.feed(Source::Pad, ev, now_us);
        let ev = self.touch.as_mut().map(|t| t.release_all()).unwrap_or_default();
        self.feed(Source::Touch, ev, now_us);
    }

    /// 視窗失焦、切到背景：放開全部來源，取消重綁。
    fn release_everything(&mut self, now_us: u64) {
        self.release_pads_and_touch(now_us);
        self.mapper.release_all();
        self.trig_down = [false; 2];
        if let Some(p) = self.panel.as_mut() {
            p.focus_lost();
        }
        self.panel_gen += 1;
    }

    fn open_panel(&mut self, target: PanelOpen, size: (u32, u32)) {
        let page = match target {
            PanelOpen::Controls => {
                if self.devs.list().is_empty() { Page::Keyboard } else { Page::Pad }
            }
            PanelOpen::Game => Page::Game,
            PanelOpen::Last => self.last_page,
        };
        self.show_page(page, size);
    }

    fn show_page(&mut self, page: Page, size: (u32, u32)) {
        match self.panel.as_mut() {
            Some(p) => p.set_page(page),
            None => self.panel = Some(Panel::new(page, size.0 as i32, size.1 as i32, self.ctl.mobile, self.ctl.dp)),
        }
        self.ctl.panel_open = true;
    }

    fn close_panel(&mut self) {
        if let Some(p) = self.panel.take() {
            self.last_page = p.page();
        }
        self.ctl.panel_open = false;
    }
}

/// 設定畫面的大小：視窗寬度，高度是工具列以下。
fn panel_size(canvas: &WindowCanvas, ctl: &Controls) -> (u32, u32) {
    let (ww, wh) = canvas.output_size().unwrap_or((960, 600));
    (ww, wh.saturating_sub(ctl.toolbar().height(ww)))
}

/// 處理一個動作的後續：視窗大小、全螢幕、授權頁、存檔、開關設定畫面，並讓輸入層跟上。回傳要不要離開。
fn apply_effect(eff: Effect, front: &mut Front, session: &mut Session, canvas: &mut WindowCanvas, cfg: &Option<PathBuf>, now_us: u64) -> bool {
    sync_session(session, &front.ctl);
    if eff.about {
        // Android 的 AboutActivity 收這個網址（android/AndroidManifest.xml）。桌面版沒有對應程式，開不了就算了。
        if let Err(e) = sdl2::url::open_url("rs2about://license") {
            eprintln!("[關於] 開不了授權頁面：{e}");
        }
    }
    match eff.resize {
        Resize::None => {}
        Resize::Scale => {
            let target = window_size(front.ctl.settings.scale, front.ctl.settings.toolbar);
            if let Err(e) = canvas.window_mut().set_size(target.0, target.1) {
                eprintln!("[視窗] 調整大小失敗：{e}");
            }
        }
        Resize::Toolbar => {
            // 工具列顯示或隱藏：寬度不變，高度加減工具列與狀態列。
            let win = canvas.window_mut();
            let (cw, ch) = win.size();
            let bar = desktop_height(cw);
            let h = if front.ctl.settings.toolbar { ch + bar } else { ch.saturating_sub(bar).max(1) };
            if let Err(e) = win.set_size(cw, h) {
                eprintln!("[視窗] 調整大小失敗：{e}");
            }
        }
    }
    if eff.fullscreen {
        let mode = if front.ctl.fullscreen { FullscreenType::Desktop } else { FullscreenType::Off };
        if let Err(e) = canvas.window_mut().set_fullscreen(mode) {
            eprintln!("[視窗] 全螢幕切換失敗：{e}");
            front.ctl.fullscreen = !front.ctl.fullscreen;
        }
    }
    if eff.save {
        save_settings(cfg, &front.ctl.settings);
    }
    match eff.panel {
        PanelChange::Open(t) => {
            let size = panel_size(canvas, &front.ctl);
            if front.panel.is_none() {
                front.panel_opened_ms = now_us / 1000;
            }
            front.open_panel(t, size);
        }
        PanelChange::Close => front.close_panel(),
        PanelChange::None => {}
    }
    front.panel_gen += 1;
    front.sync(now_us);
    eff.quit
}

fn do_action(a: Action, front: &mut Front, session: &mut Session, canvas: &mut WindowCanvas, cfg: &Option<PathBuf>, now_us: u64) -> bool {
    let eff = front.ctl.apply(a);
    apply_effect(eff, front, session, canvas, cfg, now_us)
}

/// 設定畫面處理完一個輸入之後：要關就關，其餘後續照一般動作處理。
fn finish_panel(out: PanelOut, front: &mut Front, session: &mut Session, canvas: &mut WindowCanvas, cfg: &Option<PathBuf>, now_us: u64) -> bool {
    let mut eff = out.eff;
    if out.close {
        eff.merge(front.ctl.apply(Action::ClosePanel));
    }
    apply_effect(eff, front, session, canvas, cfg, now_us)
}

/// 對設定畫面做一件事。沒有設定畫面時什麼都不做。
fn with_panel(front: &mut Front, f: impl FnOnce(&mut Panel, &mut Cx) -> PanelOut) -> PanelOut {
    let Front { panel, ctl, devs, .. } = front;
    match panel {
        Some(p) => f(p, &mut Cx { ctl, devs }),
        None => PanelOut::default(),
    }
}

/// 每 dp 個像素。讀不到螢幕 DPI 時當 2。
fn dp_of(video: &sdl2::VideoSubsystem) -> f32 {
    video.display_dpi(0).map(|(d, _, _)| d / 160.0).ok().filter(|d| *d >= 0.5).unwrap_or(2.0)
}

fn run_window(cx: Ctx) -> Result<i32, String> {
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let (w, h) = window_size(cx.settings.scale, cx.settings.toolbar);
    let (canvas, vsync) = make_canvas(&video, w, h)?;
    let tc = canvas.texture_creator();
    let pres = Presenter::new(&tc)?;
    window_loop(cx, &sdl, &video, canvas, vsync, pres)
}

/// 視窗建好之後才決定資料位置的進入點。Android 要先把資料解到內部儲存，過程要畫進度。
/// `install` 收到回報進度（0 到 1）的函式，回傳命令列參數。
pub fn run_installing(install: impl FnOnce(&mut dyn FnMut(f32)) -> Result<Args, String>) -> Result<i32, String> {
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let (w, h) = window_size(settings::DEFAULT_SCALE, false);
    let (mut canvas, vsync) = make_canvas(&video, w, h)?;
    let tc = canvas.texture_creator();
    let mut pres = Presenter::new(&tc)?;
    let args = {
        let toolbar = crate::toolbar::Toolbar::mobile(false);
        let state = Controls::new(Settings::default(), false).bar_state();
        let mut pump = sdl.event_pump()?;
        let mut shown = -1i32;
        let mut draw = |f: f32| {
            // 每 1% 畫一次，事件也要抽，否則系統會判定沒有回應。
            let pct = (f * 100.0) as i32;
            if pct != shown {
                shown = pct;
                pump.pump_events();
                if let Err(e) = pres.draw(&mut canvas, &ui::loading::install_frame(f), 1280, 800, Fit::HD, &toolbar, &state, None, None, None) {
                    eprintln!("[install] 畫不了進度：{e}");
                }
            }
        };
        install(&mut draw)?
    };
    let cx = prepare(args)?;
    window_loop(cx, &sdl, &video, canvas, vsync, pres)
}

fn window_loop<'a>(
    cx: Ctx,
    sdl: &sdl2::Sdl,
    video: &sdl2::VideoSubsystem,
    mut canvas: WindowCanvas,
    vsync: bool,
    mut pres: Presenter<'a>,
) -> Result<i32, String> {
    let timer = sdl.timer()?;
    let mut ctl = new_controls(&cx);
    ctl.mobile = cx.args.touch;
    ctl.dp = cx.args.dp.unwrap_or_else(|| dp_of(video));
    if ctl.mobile && cx.args.open_panels.is_empty() {
        // 手機上工具列預設收起來，按 ☰ 才展開。
        ctl.settings.toolbar = false;
    }
    eprintln!("[window] 視訊驅動 {}，繪圖器 {}，垂直同步 {}", video.current_video_driver(), canvas.info().name, vsync);
    // HD 預載在背景跑，視窗先顯示載入畫面。
    let hd_handle = cx.hd_dir.clone().map(|d| spawn_hd(d, cx.assets.clone()));
    let mut pump = sdl.event_pump()?;
    if let Some(h) = &hd_handle {
        // 載入畫面（規格 005）：中文字樣加圓角進度條，預載沒有進度可報，條裡一段光帶來回。
        let t0 = Instant::now();
        loop {
            pump.pump_events();
            let phase = (t0.elapsed().as_secs_f32() / 1.6).fract();
            pres.draw(&mut canvas, &ui::loading::hd_frame(phase), 1280, 800, Fit::HD, &ctl.toolbar(), &ctl.bar_state(), None, None, None)?;
            if h.is_finished() {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    let hd = finish_hd(hd_handle)?;
    let mut session = build_session(&cx, hd, ctl.view());
    sync_session(&mut session, &ctl);
    log_missing(&mut session);
    let output = if cx.args.no_audio {
        None
    } else {
        match sdl.audio().and_then(|sys| Output::open(&sys, make_engine(&cx.inputs)?)) {
            Ok(o) => Some(o),
            Err(e) => {
                eprintln!("[audio] 沒有聲音：{e}");
                None
            }
        }
    };

    // 手把。子系統開不起來（沒有裝置節點的環境）就只用鍵盤。手把不自動指派，要在設定畫面指派。
    let pad_sys = match sdl.game_controller() {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("[手把] 無法使用：{e}");
            None
        }
    };
    let joy_sys = sdl.joystick().ok();
    let mut opened: Vec<GameController> = Vec::new();
    let mut joys: Vec<Joystick> = Vec::new();
    let touch_map = if ctl.mobile {
        let (ww, wh) = canvas.output_size()?;
        Some(TouchMapper::new(ww, wh, ctl.dp))
    } else {
        None
    };
    let mut front = Front::new(ctl, touch_map);
    if cx.args.fake_pads {
        // 驗證用：不接實體手把也能看手把頁的版面。
        front.devs.add(9001, "Fake Pad One", true);
        front.devs.add(9002, "Fake Pad Two", true);
        front.devs.add(9003, "Fake Joystick", false);
        let _ = front.devs.assign(0, 9001);
        front.sync(0);
    }
    let freq = timer.performance_frequency();
    let mut pacer = Pacer::new(freq, timer.performance_counter());
    let t_start = timer.performance_counter();
    let mut hashes: Vec<String> = Vec::new();
    let mut recorded: Vec<String> = Vec::new();
    let mut shots = cx.args.shots.clone();
    shots.sort_by_key(|s| s.0);
    let mut pending_shot: Option<(u64, PathBuf)> = None;
    let mut shots_taken = 0usize;
    let mut shot_ok = true;
    let mut stats = Stats::default();
    let mut dirty = true;
    let frame_budget = Duration::from_micros(16_000);
    let mut replay_note = !cx.replay.is_empty();
    let mut last_draw = timer.performance_counter();
    let mut logged_pos: Option<(i32, i32)> = None;
    // 手機上每 5 秒記一次實際速度。邏輯應該是一秒 9.1 步，繪製跟螢幕更新率。
    let mut perf_t = Instant::now();
    let mut perf_base = (0u64, 0u64, 0u64, 0u64);
    let mut was_paused = false;
    let mut god_applied = false;
    // 設定畫面與暫停標示的點陣，內容沒變就不重畫、不重傳貼圖。
    let mut panel_img: Vec<u8> = Vec::new();
    let mut panel_img_key = u64::MAX;
    let mut banner: Option<(Vec<u8>, i32, i32, i32)> = None;

    'main: loop {
        let iter_start = Instant::now();
        let now = timer.performance_counter();
        // 手機上的分離鈕跟著「砲座像 Force」選項。
        let force_pod = front.ctl.settings.rules().force_pod;
        if front.touch.as_ref().is_some_and(|t| t.split_button() != force_pod) {
            let (ww, wh) = canvas.output_size()?;
            let dp = front.ctl.dp;
            let t_us = ticks_to_us(now, freq);
            let ev = front.touch.as_mut().map(|t| t.set_split_button(force_pod, ww, wh, dp)).unwrap_or_default();
            front.feed(Source::Touch, ev, t_us);
            dirty = true;
        }
        if let Some(secs) = cx.args.max_secs {
            if (now - t_start) as f64 / freq as f64 >= secs {
                eprintln!("[window] --max-secs {secs} 到了，結束");
                break;
            }
        }
        let mut any_event = false;
        for ev in pump.poll_iter() {
            let t_us = ticks_to_us(timer.performance_counter(), freq);
            // 搖桿、滑鼠與手指的移動很頻繁，一般不用為它們重畫；設定畫面開著時拖曳要捲動，所以照畫。
            if front.panel.is_some() || !matches!(ev, Event::ControllerAxisMotion { .. } | Event::MouseMotion { .. } | Event::FingerMotion { .. }) {
                any_event = true;
            }
            match ev {
                Event::Quit { .. } => break 'main,
                Event::KeyDown { scancode: Some(sc), repeat, keymod, .. } => {
                    let shift = keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
                    // 不管有沒有設定畫面都先交給輸入層記錄實體按鍵，設定畫面開著時它擋住不送。
                    let r = front.mapper.key_down(sc, repeat, t_us);
                    if front.panel.is_some() {
                        let nav = matches!(sc, Scancode::Tab | Scancode::Up | Scancode::Down | Scancode::Left | Scancode::Right | Scancode::PageUp | Scancode::PageDown);
                        if repeat && !nav {
                            continue;
                        }
                        let capturing = front.panel.as_ref().is_some_and(|p| p.capturing());
                        // 重綁中連 F 鍵都是候選的新鍵（會被拒絕），不執行熱鍵。
                        let quit = match r {
                            KeyResult::Hotkey(Hotkey::Page) if !capturing => {
                                with_panel(&mut front, |p, _| {
                                    p.next_page(if shift { -1 } else { 1 });
                                    PanelOut::default()
                                });
                                false
                            }
                            KeyResult::Hotkey(h) if !capturing => match Action::from_hotkey(h) {
                                Some(a) => do_action(a, &mut front, &mut session, &mut canvas, &cx.cfg_dir, t_us),
                                None => false,
                            },
                            _ => {
                                let out = with_panel(&mut front, |p, c| p.key_down(sc, shift, c));
                                finish_panel(out, &mut front, &mut session, &mut canvas, &cx.cfg_dir, t_us)
                            }
                        };
                        if quit {
                            break 'main;
                        }
                    } else if let KeyResult::Hotkey(h) = r {
                        if let Some(a) = Action::from_hotkey(h) {
                            if do_action(a, &mut front, &mut session, &mut canvas, &cx.cfg_dir, t_us) {
                                break 'main;
                            }
                        }
                    }
                }
                Event::KeyUp { scancode: Some(sc), .. } => {
                    front.mapper.key_up(sc);
                    if let Some(p) = front.panel.as_mut() {
                        p.key_up(sc);
                    }
                }
                Event::ControllerDeviceAdded { which, .. } => {
                    if let Some(sys) = &pad_sys {
                        match sys.open(which) {
                            Ok(c) => {
                                let name = c.name();
                                front.devs.add(c.instance_id(), &name, true);
                                front.say(text::pad_added(&name), t_us);
                                opened.push(c);
                            }
                            Err(e) => eprintln!("[手把] 開不了第 {which} 支：{e}"),
                        }
                    }
                }
                Event::ControllerDeviceRemoved { which, .. } => {
                    let ev = front.pads.detach(which);
                    front.feed(Source::Pad, ev, t_us);
                    if let Some((dev, was)) = front.devs.remove(which) {
                        front.say(text::pad_removed(&dev.name, was), t_us);
                    }
                    opened.retain(|c| c.instance_id() != which);
                }
                // 沒有標準配置的搖桿：列出來並明示不支援。有標準配置的由 ControllerDeviceAdded 處理。
                Event::JoyDeviceAdded { which, .. } => {
                    if let (Some(js), Some(gs)) = (&joy_sys, &pad_sys) {
                        if !gs.is_game_controller(which) {
                            match js.open(which) {
                                Ok(j) => {
                                    let name = j.name();
                                    front.devs.add(j.instance_id(), &name, false);
                                    front.say(text::pad_unsupported_added(&name), t_us);
                                    joys.push(j);
                                }
                                Err(e) => eprintln!("[手把] 開不了搖桿 {which}：{e}"),
                            }
                        }
                    }
                }
                Event::JoyDeviceRemoved { which, .. } => {
                    if front.devs.find(which).is_some_and(|d| !d.supported) {
                        if let Some((dev, _)) = front.devs.remove(which) {
                            front.say(text::pad_removed(&dev.name, None), t_us);
                        }
                        joys.retain(|j| j.instance_id() != which);
                    }
                }
                Event::ControllerButtonDown { which, button, .. } => {
                    if front.panel.as_ref().is_some_and(|p| p.capturing()) {
                        if let Some(b) = Btn::from_sdl(button) {
                            let out = with_panel(&mut front, |p, c| p.pad_button(b, c));
                            finish_panel(out, &mut front, &mut session, &mut canvas, &cx.cfg_dir, t_us);
                        }
                    }
                    let ev = front.pads.button(which, button, true);
                    if front.feed(Source::Pad, ev, t_us) && do_action(Action::TogglePause, &mut front, &mut session, &mut canvas, &cx.cfg_dir, t_us) {
                        break 'main;
                    }
                }
                Event::ControllerButtonUp { which, button, .. } => {
                    let ev = front.pads.button(which, button, false);
                    front.feed(Source::Pad, ev, t_us);
                }
                Event::ControllerAxisMotion { which, axis, value, .. } => {
                    // 重綁手把按鈕時，扳機過了門檻算按下 LT、RT。
                    if front.panel.as_ref().is_some_and(|p| p.capturing()) {
                        let (i, btn) = match axis {
                            Axis::TriggerLeft => (0, Some(Btn::LT)),
                            Axis::TriggerRight => (1, Some(Btn::RT)),
                            _ => (0, None),
                        };
                        if let Some(btn) = btn {
                            if !front.trig_down[i] && value >= STICK_ENGAGE {
                                front.trig_down[i] = true;
                                let out = with_panel(&mut front, |p, c| p.pad_button(btn, c));
                                finish_panel(out, &mut front, &mut session, &mut canvas, &cx.cfg_dir, t_us);
                            } else if front.trig_down[i] && value < STICK_RELEASE {
                                front.trig_down[i] = false;
                            }
                        }
                    }
                    let ev = front.pads.axis(which, axis, value);
                    front.feed(Source::Pad, ev, t_us);
                }
                Event::MouseButtonDown { mouse_btn: MouseButton::Left, x, y, .. } => {
                    let (ww, _) = canvas.output_size()?;
                    let tb = front.ctl.toolbar();
                    let bar_h = tb.height(ww) as i32;
                    if y < bar_h {
                        if let Some(b) = tb.hit(ww, x, y) {
                            front.down_btn = Some(b);
                            dirty = true;
                            if let Some(a) = Action::from_button(b) {
                                if do_action(a, &mut front, &mut session, &mut canvas, &cx.cfg_dir, t_us) {
                                    break 'main;
                                }
                            }
                        }
                    } else if let Some(p) = front.panel.as_mut() {
                        p.press(x, y - bar_h);
                    }
                }
                Event::MouseMotion { mousestate, x, y, .. } => {
                    let (ww, _) = canvas.output_size()?;
                    let tb = front.ctl.toolbar();
                    let bar_h = tb.height(ww) as i32;
                    // 滑鼠移過：工具列的按鈕與設定畫面的欄位變亮。觸控合成的滑鼠事件不追蹤，免得放開後還亮著。
                    if !front.ctl.mobile {
                        let hb = if y < bar_h { tb.hit(ww, x, y) } else { None };
                        if hb != front.hover_btn {
                            front.hover_btn = hb;
                            dirty = true;
                        }
                        let Front { panel, ctl, devs, .. } = &mut front;
                        if let Some(p) = panel.as_mut() {
                            if y >= bar_h {
                                p.hover_at(x, y - bar_h, ctl, devs);
                            } else {
                                p.clear_hover();
                            }
                        }
                    }
                    if mousestate.left() && front.panel.is_some() {
                        let Front { panel, ctl, devs, .. } = &mut front;
                        if let Some(p) = panel.as_mut() {
                            p.drag(y - bar_h, ctl, devs);
                        }
                    }
                }
                Event::MouseButtonUp { mouse_btn: MouseButton::Left, x, y, .. } => {
                    if front.down_btn.take().is_some() {
                        dirty = true;
                    }
                    if front.panel.is_some() {
                        let (ww, _) = canvas.output_size()?;
                        let bar_h = front.ctl.toolbar().height(ww) as i32;
                        let out = with_panel(&mut front, |p, c| p.release(x, y - bar_h, c));
                        if finish_panel(out, &mut front, &mut session, &mut canvas, &cx.cfg_dir, t_us) {
                            break 'main;
                        }
                    }
                }
                Event::MouseWheel { y, .. } => {
                    let Front { panel, ctl, devs, .. } = &mut front;
                    if let Some(p) = panel.as_mut() {
                        p.wheel(y, ctl, devs);
                    }
                }
                Event::FingerDown { finger_id, x, y, .. } => {
                    // 設定畫面開著時，觸控的操作走滑鼠事件（SDL 會把觸控合成滑鼠事件），控制鈕不作用。
                    if front.panel.is_none() && front.touch.is_some() {
                        let (ww, wh) = canvas.output_size()?;
                        let (px, py) = (x * ww as f32, y * wh as f32);
                        let in_bar = front.ctl.toolbar().height(ww) as f32 > py;
                        let playable = session.in_play();
                        let t = front.touch.as_mut().map(|tm| tm.down(finger_id, px, py, playable, in_bar));
                        if let Some(t) = t {
                            front.feed(Source::Touch, t.keys, t_us);
                            if t.toggle_toolbar && do_action(Action::Toolbar, &mut front, &mut session, &mut canvas, &cx.cfg_dir, t_us) {
                                break 'main;
                            }
                            // Android 的煙霧測試從 logcat 取按鈕位置，不在測試裡重算版面。
                            if t.toggle_toolbar && cfg!(target_os = "android") && front.ctl.settings.toolbar {
                                let (ww, _) = canvas.output_size()?;
                                for (b, r) in front.ctl.toolbar().layout(ww).buttons {
                                    eprintln!("[工具列] {b:?} {} {}", r.x + r.w / 2, r.y + r.h / 2);
                                }
                            }
                            if t.toggle_pause && do_action(Action::TogglePause, &mut front, &mut session, &mut canvas, &cx.cfg_dir, t_us) {
                                break 'main;
                            }
                        }
                    }
                }
                Event::FingerMotion { finger_id, x, y, .. } => {
                    if front.touch.is_some() {
                        let (ww, wh) = canvas.output_size()?;
                        let ev = front.touch.as_mut().map(|tm| tm.moved(finger_id, x * ww as f32, y * wh as f32)).unwrap_or_default();
                        front.feed(Source::Touch, ev, t_us);
                    }
                }
                Event::FingerUp { finger_id, .. } => {
                    let ev = front.touch.as_mut().map(|tm| tm.up(finger_id)).unwrap_or_default();
                    front.feed(Source::Touch, ev, t_us);
                }
                // Android 切到背景：放開全部的鍵。切回前景：不補步。繪圖裝置可能重建。
                Event::AppWillEnterBackground { .. } | Event::AppDidEnterBackground { .. } => front.release_everything(t_us),
                Event::AppDidEnterForeground { .. } => {
                    pacer.reset(timer.performance_counter());
                    last_draw = timer.performance_counter();
                    dirty = true;
                }
                Event::RenderTargetsReset { .. } | Event::RenderDeviceReset { .. } => {
                    pres.invalidate()?;
                    dirty = true;
                }
                Event::Window { win_event, .. } => match win_event {
                    WindowEvent::FocusLost => front.release_everything(t_us),
                    WindowEvent::Leave => {
                        if front.hover_btn.take().is_some() {
                            dirty = true;
                        }
                        if let Some(p) = front.panel.as_mut() {
                            p.clear_hover();
                        }
                    }
                    WindowEvent::FocusGained => pacer.reset(timer.performance_counter()),
                    WindowEvent::SizeChanged(..) => {
                        if front.touch.is_some() {
                            let (ww, wh) = canvas.output_size()?;
                            let dp = front.ctl.dp;
                            let ev = front.touch.as_mut().map(|tm| tm.resize(ww, wh, dp)).unwrap_or_default();
                            front.feed(Source::Touch, ev, t_us);
                        }
                        dirty = true;
                    }
                    WindowEvent::Exposed | WindowEvent::Restored | WindowEvent::Shown => dirty = true,
                    _ => {}
                },
                _ => {}
            }
            // 每個事件之後輸入層都要跟上：開關設定畫面或前端暫停的那一刻起，後面的事件就不能再送進遊戲。
            front.sync(t_us);
            if front.panel.is_some() {
                front.panel_gen += 1;
            }
        }
        if any_event {
            dirty = true;
        }

        let now = timer.performance_counter();
        let now_us = ticks_to_us(now, freq);
        let now_ms = now_us / 1000;
        // 介面層（HUD、回饋、面板淡入）的時間。不影響模擬時間。
        session.set_clock_ms(now_ms);
        if front.expire_notice(now_us) {
            front.panel_gen += 1;
            dirty = true;
        }
        front.mapper.set_hold_short(front.ctl.settings.feel);
        // 前端暫停：Pacer 不步進，輸入不送進遊戲。回到執行時重設計時，不補步。
        let paused = front.ctl.frontend_paused();
        if paused != was_paused {
            if !paused {
                pacer.reset(now);
                last_draw = now;
            }
            eprintln!("[前端] {}（第 {} 步，按暫停 {}，設定畫面 {}）", if paused { "前端暫停" } else { "繼續" }, session.steps(), front.ctl.paused, front.ctl.panel_open);
            was_paused = paused;
            dirty = true;
        }
        if !paused {
            front.mapper.poll(now_us);
            pacer.update(now);
            while pacer.take_step() {
                let idx = session.steps() as usize;
                let (split, sc): ([bool; 2], Vec<u8>) = match cx.replay.get(idx) {
                    Some(r) => {
                        front.mapper.discard_queue();
                        split_codes(r)
                    }
                    None => {
                        if replay_note {
                            eprintln!("[replay] 收據用完，改回鍵盤輸入（第 {idx} 步起）");
                            replay_note = false;
                        }
                        (front.mapper.take_split(), front.mapper.take())
                    }
                };
                if cx.args.record.is_some() {
                    // 分離鍵的虛擬碼也記下來，重播才會一樣。
                    let mut codes: Vec<String> = sc.iter().map(|c| c.to_string()).collect();
                    codes.extend((0..2).filter(|&p| split[p]).map(|p| split_pseudo(p).to_string()));
                    recorded.push(format!("{{\"n\":{idx},\"sc\":[{}]}}", codes.join(",")));
                }
                apply_invincible(&mut session, front.ctl.settings.invincible(), &mut god_applied);
                session.set_split_held(split);
                let t0 = Instant::now();
                let outcome = session.step(&sc);
                pacer.set_next(session.ticks());
                stats.step(t0.elapsed().as_micros() as u64);
                for m in session.take_audio() {
                    if let Some(o) = &output {
                        o.send(m);
                    }
                }
                if cx.args.hash_out.is_some() {
                    hashes.push(session.dseg_hash());
                }
                dirty = true;
                if let Outcome::Ended(e) = outcome {
                    eprintln!("[game] 遊戲結束 {e:?}（第 {} 步）", session.steps());
                    break 'main;
                }
            }
        }
        log_missing(&mut session);
        if ctl.mobile {
            let elapsed = perf_t.elapsed().as_secs_f32();
            if elapsed >= 5.0 {
                let (steps, renders, render_us, present_us) = (stats.steps - perf_base.0, stats.renders - perf_base.1, stats.render_us - perf_base.2, stats.present_us - perf_base.3);
                eprintln!(
                    "[perf] {:.1} 步/秒，{:.1} 幀/秒，render {:.1} ms，貼圖加 present {:.1} ms",
                    steps as f32 / elapsed,
                    renders as f32 / elapsed,
                    render_us as f32 / renders.max(1) as f32 / 1000.0,
                    present_us as f32 / renders.max(1) as f32 / 1000.0,
                );
                perf_t = Instant::now();
                perf_base = (stats.steps, stats.renders, stats.render_us, stats.present_us);
            }
            // Android 的煙霧測試從 logcat 確認觸控真的移動了玩家機。
            let pos = session.player_pos();
            if pos != logged_pos {
                match pos {
                    Some((x, y)) => eprintln!("[play] 玩家 1 在 ({x},{y})"),
                    None => eprintln!("[play] 玩家 1 不能控制"),
                }
                logged_pos = pos;
            }
        }

        if pending_shot.is_none() {
            // 驗證用：有 --open-panel 時，第一張圖等到步數，其後的圖不再等（設定畫面開著時步數不會增加）。
            let chain = !cx.args.open_panels.is_empty() && shots_taken > 0;
            if let Some(i) = shots.iter().position(|(n, _)| *n <= session.steps() || chain) {
                pending_shot = Some(shots.remove(i));
                if let Some(plan) = cx.args.open_panels.get(shots_taken).copied() {
                    match plan {
                        Some(page) => {
                            let size = panel_size(&canvas, &front.ctl);
                            if front.panel.is_none() {
                                // 存圖要看到完整的畫面，不拍到淡入的中途。
                                front.panel_opened_ms = now_ms.saturating_sub(panel::FADE_IN_MS + 1);
                            }
                            front.show_page(page, size);
                        }
                        None => front.close_panel(),
                    }
                    front.panel_gen += 1;
                    front.sync(now_us);
                }
                dirty = true;
            }
        }
        let paused = front.ctl.frontend_paused();
        let animating = (front.ctl.settings.tween || session.hd_effective()) && !paused;
        if dirty || animating || pending_shot.is_some() {
            // 存圖固定用 alpha 1.0：畫出剛跑完的那一步。
            let alpha = if pending_shot.is_some() { 1.0 } else { pacer.alpha() };
            // 玩家機預測看的是下一步會生效的按鍵。回放中的輸入不是鍵盤來的，不預測。暫停時玩家機不動，不預測。
            let draw_now = timer.performance_counter();
            if !paused && cx.replay.get(session.steps() as usize).is_none() {
                let dt = (draw_now - last_draw) as f64 / freq as f64;
                session.predict(&front.mapper.next_state(), dt as f32, pacer.step_secs());
            }
            last_draw = draw_now;
            let in_play = session.in_play();
            let (ww, wh) = canvas.output_size()?;
            let tb = front.ctl.toolbar();
            let bar_h = tb.height(ww);
            let shake = session.shake_offset();
            let t1 = Instant::now();
            let fit = session.fit();
            let (frame, fw, fh) = session.render(alpha);
            stats.render(t1.elapsed().as_micros() as u64);
            let shot_path = pending_shot.as_ref().map(|(_, p)| p.clone());
            let t2 = Instant::now();
            // 設定畫面開著時觸控鈕不畫（它蓋在最上面）。
            let overlay = if front.panel.is_some() { None } else { front.touch.as_ref().map(|t| Overlay { controls: t.controls(), playable: in_play }) };
            // 設定畫面；沒有的話，工具列收起來又暫停時畫一個「已暫停」標示。
            let mut layer: Option<UiLayer<'_>> = None;
            let mut fading = false;
            let (ctl, devs) = (&front.ctl, &front.devs);
            if let Some(p) = front.panel.as_mut() {
                p.resize(ww as i32, wh.saturating_sub(bar_h) as i32, ctl.mobile, ctl.dp);
                // 開啟後 120 毫秒淡入（規格 005），分 8 級重畫；淡入的級數也是圖的一部分。
                let step = panel::fade_step(now_ms.saturating_sub(front.panel_opened_ms));
                fading = step < panel::FADE_STEPS;
                let key = front.panel_gen.wrapping_mul(1_000_003).wrapping_add(p.version()).wrapping_mul(16).wrapping_add(step as u64);
                if key != panel_img_key {
                    panel_img = p.render_faded(ctl, devs, step as f32 / panel::FADE_STEPS as f32);
                    panel_img_key = key;
                }
                let (pw, ph) = p.size();
                layer = Some(UiLayer { px: &panel_img, w: pw as u32, h: ph as u32, x: 0, y: bar_h as i32, key });
            } else if ctl.paused && !tb.visible {
                let fh_px = metrics(ww, ctl.mobile, ctl.dp).fh;
                if banner.as_ref().map(|b| b.3) != Some(fh_px) {
                    let (px, w, h) = ui::banner(text::ST_PAUSED, fh_px);
                    banner = Some((px, w, h, fh_px));
                }
                if let Some((px, w, h, f)) = banner.as_ref() {
                    layer = Some(UiLayer { px, w: *w as u32, h: *h as u32, x: *f / 2, y: *f / 2, key: (1u64 << 62) | *f as u64 });
                }
            }
            let bar_state = front.bar_state();
            // 打擊回饋的畫面震動：只改輸出位置。
            pres.shake = shake;
            let info = pres.draw(&mut canvas, frame, fw, fh, fit, &tb, &bar_state, overlay, layer, shot_path.as_deref())?;
            stats.present(t2.elapsed().as_micros() as u64);
            if let (Some((s, path)), Some(info)) = (pending_shot.take(), info) {
                shot_ok &= report_shot(&path, info, s);
                shots_taken += 1;
            }
            dirty = fading;
            // 有垂直同步時 present 自己會等；仍然加一道下限，免得繪圖器宣稱有同步卻沒等時空轉。
            let budget = if vsync { Duration::from_micros(2_000) } else { frame_budget };
            let spent = iter_start.elapsed();
            if spent < budget {
                std::thread::sleep(budget - spent);
            }
        } else {
            let wait = Duration::from_micros((pacer.until_next() as u128 * 1_000_000 / freq.max(1) as u128).min(4_000) as u64);
            std::thread::sleep(wait.max(Duration::from_micros(500)));
        }
    }

    save_settings(&cx.cfg_dir, &front.ctl.settings);
    stats.report();
    write_hashes(&cx.args.hash_out, &hashes)?;
    if let Some(p) = &cx.args.record {
        let mut s = recorded.join("\n");
        s.push('\n');
        std::fs::write(p, s).map_err(|e| format!("寫不了 {}：{e}", p.display()))?;
        eprintln!("[record] {} 步 -> {}", recorded.len(), p.display());
    }
    let mut rc = 0;
    if !shots.is_empty() || pending_shot.is_some() {
        for (s, p) in shots.iter().chain(pending_shot.iter()) {
            eprintln!("[shot] 第 {s} 步沒有到達，未存 {}（實際跑到第 {} 步）", p.display(), session.steps());
        }
        rc = 4;
    }
    if !shot_ok {
        rc = 4;
    }
    Ok(rc)
}
