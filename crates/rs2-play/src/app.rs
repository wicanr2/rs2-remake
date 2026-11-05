//! 主迴圈：視窗模式（SDL 事件、固定步長、補間顯示）與 `--headless-steps`（同一段步進與繪製程式，沒有事件）。

use crate::args::{Args, StartArg, ToggleName};
use crate::audio::{Output, WavOut};
use crate::controls::{Action, Controls};
use crate::input::{InputMapper, KeyResult};
use crate::pacer::Pacer;
use crate::session::{Begin, Outcome, Session, ViewOpts};
use crate::settings::{self, Settings};
use crate::toolbar::bar_height;
use crate::view::{Presenter, ShotInfo, window_size};
use rs2_audio::Engine;
use rs2_core::{Fixes, Start};
use rs2_gfx::{Assets, HdAssets};
use rs2_trace::{Inputs, load_inputs, parse_sc};
use sdl2::event::{Event, WindowEvent};
use sdl2::mouse::MouseButton;
use sdl2::render::WindowCanvas;
use sdl2::video::FullscreenType;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// 一次執行的共用資料。
struct Ctx {
    args: Args,
    inputs: Inputs,
    assets: Assets,
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
    video.window("RS-2 remake", w, h).position_centered().resizable().build().map_err(|e| format!("建立視窗失敗：{e}"))
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
    let inputs = load_inputs(&args.orig)?;
    let assets = Assets::load(&args.orig)?;
    eprintln!("[init] 原版檔雜湊驗證通過（{}）", args.orig.display());
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
    let cfg_dir = args.config.clone().or_else(|| {
        let xdg = std::env::var("XDG_CONFIG_HOME").ok();
        let home = std::env::var("HOME").ok();
        settings::default_dir(xdg.as_deref(), home.as_deref())
    });
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
    let hd_dir = if args.no_hd { None } else { args.hd.clone() };
    Ok(Ctx { args, inputs, assets, settings: st, cfg_dir, replay, hd_dir })
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
    Session::new(&cx.inputs, cx.assets.clone(), hd, begin_of(&cx.args), fixes, view)
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

fn toggle_action(t: ToggleName) -> Action {
    match t {
        ToggleName::Tween => Action::Tween,
        ToggleName::Counter => Action::Counter,
        ToggleName::Hd => Action::Gfx,
    }
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
    let mut ctl = Controls::new(cx.settings, cx.hd_dir.is_some());
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
    log_missing(&mut session);
    eprintln!("[headless] 起點 {:?}，{steps} 步，補間 {}，幀計數 {}，HD {}", cx.args.start, ctl.settings.tween, ctl.settings.counter, session.hd_effective());

    let mut hashes: Vec<String> = Vec::new();
    let mut shots = cx.args.shots.clone();
    let mut toggles = cx.args.toggles.clone();
    let mut stats = Stats::default();
    let mut shot_ok = true;
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
                ctl.apply(toggle_action(t));
                false
            } else {
                true
            }
        });
        session.set_view(ctl.view());
        let sc: &[u8] = cx.replay.get(done as usize).map(|v| v.as_slice()).unwrap_or(&[]);
        let t0 = Instant::now();
        let outcome = session.step(sc);
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
            let t1 = Instant::now();
            let (frame, fw, fh) = session.render(alpha);
            stats.render(t1.elapsed().as_micros() as u64);
            if let (Some(p), Some(c)) = (pres.as_mut(), canvas.as_mut()) {
                let shot_idx = if alpha == 1.0 { shots.iter().position(|(s, _)| *s == n) } else { None };
                let shot_path = shot_idx.map(|i| shots[i].1.clone());
                let t2 = Instant::now();
                let info = p.draw(c, frame, fw, fh, &ctl.toolbar(), &ctl.bar_state(), shot_path.as_deref())?;
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

/// 執行一個前端動作，並處理視窗大小、全螢幕與存檔。
fn do_action(a: Action, ctl: &mut Controls, session: &mut Session, canvas: &mut WindowCanvas, cfg: &Option<PathBuf>) {
    let eff = ctl.apply(a);
    session.set_view(ctl.view());
    if eff.resize {
        let win = canvas.window_mut();
        let (cw, ch) = win.size();
        let target = match a {
            Action::Scale => window_size(ctl.settings.scale, ctl.settings.toolbar),
            _ => {
                // 工具列顯示或隱藏：寬度不變，高度加減工具列。
                let bar = bar_height(cw);
                (cw, if ctl.settings.toolbar { ch + bar } else { ch.saturating_sub(bar).max(1) })
            }
        };
        if let Err(e) = win.set_size(target.0, target.1) {
            eprintln!("[視窗] 調整大小失敗：{e}");
        }
    }
    if eff.fullscreen {
        let mode = if ctl.fullscreen { FullscreenType::Desktop } else { FullscreenType::Off };
        if let Err(e) = canvas.window_mut().set_fullscreen(mode) {
            eprintln!("[視窗] 全螢幕切換失敗：{e}");
            ctl.fullscreen = !ctl.fullscreen;
        }
    }
    if eff.save {
        save_settings(cfg, &ctl.settings);
    }
}

fn loading_frame() -> Vec<u8> {
    let (w, h) = (320usize, 200usize);
    let mut buf = vec![0u8; w * h * 4];
    crate::font::fill_rect(&mut buf, w, h, 0, 0, w as i32, h as i32, [10, 12, 24, 255]);
    let t = "LOADING.";
    crate::font::draw_text(&mut buf, w, h, (w as i32 - crate::font::text_width(t, 2) as i32) / 2, 90, t, 2, [220, 220, 240, 255]);
    buf
}

fn run_window(cx: Ctx) -> Result<i32, String> {
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let timer = sdl.timer()?;
    let mut ctl = Controls::new(cx.settings, cx.hd_dir.is_some());
    let (w, h) = window_size(ctl.settings.scale, ctl.settings.toolbar);
    let (mut canvas, vsync) = make_canvas(&video, w, h)?;
    eprintln!("[window] 視訊驅動 {}，繪圖器 {}，垂直同步 {}", video.current_video_driver(), canvas.info().name, vsync);
    let tc = canvas.texture_creator();
    let mut pres = Presenter::new(&tc)?;
    // HD 預載在背景跑，視窗先顯示載入畫面。
    let hd_handle = cx.hd_dir.clone().map(|d| spawn_hd(d, cx.assets.clone()));
    if hd_handle.is_some() {
        pres.draw(&mut canvas, &loading_frame(), 320, 200, &ctl.toolbar(), &ctl.bar_state(), None)?;
    }
    let hd = finish_hd(hd_handle)?;
    let mut session = build_session(&cx, hd, ctl.view());
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

    let mut pump = sdl.event_pump()?;
    let mut mapper = InputMapper::new();
    let freq = timer.performance_frequency();
    let mut pacer = Pacer::new(freq, timer.performance_counter());
    let t_start = timer.performance_counter();
    let mut hashes: Vec<String> = Vec::new();
    let mut recorded: Vec<String> = Vec::new();
    let mut shots = cx.args.shots.clone();
    shots.sort_by_key(|s| s.0);
    let mut pending_shot: Option<(u64, PathBuf)> = None;
    let mut shot_ok = true;
    let mut stats = Stats::default();
    let mut dirty = true;
    let frame_budget = Duration::from_micros(16_000);
    let mut replay_note = !cx.replay.is_empty();

    'main: loop {
        let iter_start = Instant::now();
        let now = timer.performance_counter();
        if let Some(secs) = cx.args.max_secs {
            if (now - t_start) as f64 / freq as f64 >= secs {
                eprintln!("[window] --max-secs {secs} 到了，結束");
                break;
            }
        }
        for ev in pump.poll_iter() {
            let t_us = ticks_to_us(timer.performance_counter(), freq);
            match ev {
                Event::Quit { .. } => break 'main,
                Event::KeyDown { scancode: Some(sc), repeat, .. } => {
                    if let KeyResult::Hotkey(h) = mapper.key_down(sc, repeat, t_us) {
                        do_action(h.into(), &mut ctl, &mut session, &mut canvas, &cx.cfg_dir);
                        dirty = true;
                    }
                }
                Event::KeyUp { scancode: Some(sc), .. } => mapper.key_up(sc),
                Event::MouseButtonDown { mouse_btn: MouseButton::Left, x, y, .. } => {
                    let (ww, _) = canvas.output_size()?;
                    if let Some(b) = ctl.toolbar().hit(ww, x, y) {
                        do_action(b.into(), &mut ctl, &mut session, &mut canvas, &cx.cfg_dir);
                        dirty = true;
                    }
                }
                Event::Window { win_event, .. } => match win_event {
                    WindowEvent::FocusLost => mapper.release_all(),
                    WindowEvent::FocusGained => pacer.reset(timer.performance_counter()),
                    WindowEvent::Exposed | WindowEvent::SizeChanged(..) | WindowEvent::Restored | WindowEvent::Shown => dirty = true,
                    _ => {}
                },
                _ => {}
            }
        }

        let now = timer.performance_counter();
        mapper.poll(ticks_to_us(now, freq));
        pacer.update(now);
        while pacer.take_step() {
            let idx = session.steps() as usize;
            let sc: Vec<u8> = match cx.replay.get(idx) {
                Some(r) => {
                    mapper.discard_queue();
                    r.clone()
                }
                None => {
                    if replay_note {
                        eprintln!("[replay] 收據用完，改回鍵盤輸入（第 {idx} 步起）");
                        replay_note = false;
                    }
                    mapper.take()
                }
            };
            if cx.args.record.is_some() {
                let codes: Vec<String> = sc.iter().map(|c| c.to_string()).collect();
                recorded.push(format!("{{\"n\":{idx},\"sc\":[{}]}}", codes.join(",")));
            }
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
        log_missing(&mut session);

        if pending_shot.is_none() {
            if let Some(i) = shots.iter().position(|(n, _)| *n <= session.steps()) {
                pending_shot = Some(shots.remove(i));
            }
        }
        let animating = ctl.settings.tween;
        if dirty || animating || pending_shot.is_some() {
            // 存圖固定用 alpha 1.0：畫出剛跑完的那一步。
            let alpha = if pending_shot.is_some() { 1.0 } else { pacer.alpha() };
            let t1 = Instant::now();
            let (frame, fw, fh) = session.render(alpha);
            stats.render(t1.elapsed().as_micros() as u64);
            let shot_path = pending_shot.as_ref().map(|(_, p)| p.clone());
            let t2 = Instant::now();
            let info = pres.draw(&mut canvas, frame, fw, fh, &ctl.toolbar(), &ctl.bar_state(), shot_path.as_deref())?;
            stats.present(t2.elapsed().as_micros() as u64);
            if let (Some((s, path)), Some(info)) = (pending_shot.take(), info) {
                shot_ok &= report_shot(&path, info, s);
            }
            dirty = false;
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

    save_settings(&cx.cfg_dir, &ctl.settings);
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
