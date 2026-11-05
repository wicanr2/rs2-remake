//! 一局遊戲：`Game` 加四個繪製接收端，以及視窗模式與 `--headless-steps` 共用的步進與繪製路徑。
//! 不依賴 SDL，所以主迴圈以外的邏輯（模式選擇、排程）可以直接測。
//!
//! `Game::step` 回傳的 `draws` 同時餵給 `VgaSink`（補間關閉的原版圖）、`LayerSink`（補間與 HD）和兩個 `MonoSink`
//! （CGA、MGA），四邊都隨時是最新的，所以 F1、F3、F6 切換不需要重播，切換當下的畫面一定是對的。
//! 畫面顯示的選擇只影響 `render`，不碰 `Game`。
//!
//! # 顯示模式（`docs/re/14-display-modes.md`、規格 006）
//!
//! | 模式 | `render` 的來源 | 輸出 |
//! |---|---|---|
//! | VGA | 補間關且原版圖：`VgaSink::render_page`（逐像素）；否則 `LayerSink` | 320×200，HD 1280×800 |
//! | EGA | 補間關且原版圖：`VgaSink::render_page_ega`（逐像素）；否則 `LayerSink` 的每個像素量化到 EGA 16 色中最近的一色 | 同 VGA |
//! | CGA | `MonoSink`（`MonoKind::Cga`）；補間與 HD 暫不支援，固定原版時序、原版圖 | 640×200 |
//! | MGA | `MonoSink`（`MonoKind::Mga`） | 640×300 |
//!
//! 模式只影響兩件和模擬有關的事，都在這裡處理：
//!
//! - 第三關地形來源。`Game::set_terrain_scr(mode.is_mono())` 在建立時與每次切換模式時呼叫，核心在進第三關時讀取。
//!   一局裡每個第三關各用進場當下的模式，關內切換只換畫面（已決定的取捨）。
//! - PC 喇叭附帶音。原版只有 E 的特效 blit 驅動喇叭，CGA、MGA 沒有，所以單色模式不產生 `AudioMsg::Speaker`。

use rs2_audio::{SfxKind, SpkEv};
use rs2_core::{AudioCmd, DrawCmd, Exit, Fixes, Game, HudStatus, Input, MixFile, Pool, Rules, Start};
use rs2_formats::sha256_hex;
use rs2_gfx::speaker::speaker_events;
use rs2_gfx::{Assets, HdAssets, LayerSink, MonoAssets, MonoHdSink, MonoKind, MonoSink, RenderOpts, VgaSink};
use rs2_trace::Inputs;
use rs2_trace::mirror::Mirror;
use std::sync::Arc;

use crate::feedback::{Feedback, FxEvent, Scanner};
use crate::mode::DisplayMode;
use crate::predict::{Dir, Predictor, Reach};
use crate::ui::hud::{self, HudFlags, HudIcons, HudState, IconImg};
use crate::view::Fit;

/// 遊戲從哪裡開始。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Begin {
    /// 正式遊戲：從標題開始，走完四關與片尾。
    Title,
    /// 直接進第一關，結束就離開。除錯與 A6 回歸用。
    Stage1(Start),
}

/// 交給音訊端的訊息，依遊戲發生的順序。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioMsg {
    Play(MixFile),
    Stop,
    /// 一段繪圖附帶音。靜音時不產生。
    Speaker(Vec<SpkEv>),
    /// 打擊回饋的音效層（規格 005）。只在 HD 畫面且打擊回饋開著時產生。
    Sfx(SfxKind),
}

/// 顯示選項。`hd` 只有在 `Session::hd_available()` 時才生效。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewOpts {
    pub tween: bool,
    pub counter: bool,
    pub hd: bool,
    /// 玩家機預測。只在補間開著時有作用，補間關著的畫面要逐像素等於原版。
    pub feel: bool,
    /// 顯示模式。CGA、MGA 需要單色圖庫（`Session::new` 的 `mono`），沒有就當 VGA。
    pub mode: DisplayMode,
}

/// `Session::step` 之後主迴圈要做的事。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    /// 遊戲結束，主迴圈離開。
    Ended(Exit),
}

/// 單色接收端：CGA 與 MGA 各一個，永遠一起餵。
struct MonoPair {
    cga: MonoSink,
    mga: MonoSink,
}

impl MonoPair {
    fn new(assets: Arc<MonoAssets>) -> MonoPair {
        MonoPair { cga: MonoSink::new(MonoKind::Cga, Arc::clone(&assets)), mga: MonoSink::new(MonoKind::Mga, assets) }
    }

    fn sink(&self, mode: DisplayMode) -> Option<&MonoSink> {
        match mode {
            DisplayMode::Cga => Some(&self.cga),
            DisplayMode::Mga => Some(&self.mga),
            _ => None,
        }
    }
}

/// 單色的 HD 接收端：CGA 與 MGA 各一個（幀計數在 CGA 只寫第一頁，兩個畫面不同），永遠一起餵。
/// 要有單色圖庫與 HD 資料夾才有。HD 圖（`<HD 資料夾>/mono/`）用到才讀，缺的圖退回 SD 圖庫項的 4 倍放大。
struct MonoHdPair {
    cga: MonoHdSink,
    mga: MonoHdSink,
}

impl MonoHdPair {
    fn new(assets: Arc<MonoAssets>, hd_dir: &std::path::Path) -> MonoHdPair {
        MonoHdPair {
            cga: MonoHdSink::with_kind(MonoKind::Cga, Arc::clone(&assets), hd_dir),
            mga: MonoHdSink::with_kind(MonoKind::Mga, assets, hd_dir),
        }
    }

    fn sink(&self, mode: DisplayMode) -> Option<&MonoHdSink> {
        match mode {
            DisplayMode::Cga => Some(&self.cga),
            DisplayMode::Mga => Some(&self.mga),
            _ => None,
        }
    }
}

/// 把 RGBA 的每個像素換成 EGA 16 色中最近（RGB 歐氏距離，同距取編號小的）的一色，結果是精確的最近色。
///
/// 逐像素搜 16 色，另有直接映射的快取（鍵是完整的 24 位元色，色表變了就清掉），所以 HD 圖裡大片的平色不重算。
struct EgaQuant {
    pal: [[u8; 3]; 16],
    /// 快取：槽位 `hash(rgb)` 存色與結果。色是 `u32::MAX` 表示空。
    key: Vec<u32>,
    val: Vec<u8>,
}

impl EgaQuant {
    const SLOTS: usize = 1 << 16;

    fn new() -> EgaQuant {
        EgaQuant { pal: [[0; 3]; 16], key: Vec::new(), val: Vec::new() }
    }

    fn nearest(pal: &[[u8; 3]; 16], r: u8, g: u8, b: u8) -> u8 {
        let mut best = (i32::MAX, 0u8);
        for (i, c) in pal.iter().enumerate() {
            let d = (r as i32 - c[0] as i32).pow(2) + (g as i32 - c[1] as i32).pow(2) + (b as i32 - c[2] as i32).pow(2);
            if d < best.0 {
                best = (d, i as u8);
            }
        }
        best.1
    }

    /// 原地量化。alpha 不動。
    fn apply(&mut self, pal: &[[u8; 3]; 16], rgba: &mut [u8]) {
        if self.key.len() != Self::SLOTS || self.pal != *pal {
            self.pal = *pal;
            self.key = vec![u32::MAX; Self::SLOTS];
            self.val = vec![0; Self::SLOTS];
        }
        for p in rgba.chunks_exact_mut(4) {
            let rgb = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
            let slot = (rgb.wrapping_mul(0x9E37_79B1) >> 16) as usize;
            let idx = if self.key[slot] == rgb {
                self.val[slot]
            } else {
                let i = Self::nearest(&self.pal, p[0], p[1], p[2]);
                self.key[slot] = rgb;
                self.val[slot] = i;
                i
            };
            p[..3].copy_from_slice(&self.pal[idx as usize]);
        }
    }
}

pub struct Session {
    game: Game,
    /// 狀態與原版資料段的對應。只給 `dseg_hash`（對拍與 `--hash-out`）和測試用，遊戲與畫面都不讀它。
    mirror: Mirror,
    vga: VgaSink,
    layer: LayerSink,
    /// 單色圖庫載入成功才有。沒有時 CGA、MGA 退回 VGA。
    mono: Option<MonoPair>,
    /// 單色圖庫與 HD 資料夾都有才有。單色模式選了 HD 時 `render` 用它。
    mono_hd: Option<MonoHdPair>,
    /// 實際使用的顯示模式。`view.mode` 是要求的模式，單色圖庫缺檔時兩者不同。
    mode: DisplayMode,
    ega_quant: EgaQuant,
    view: ViewOpts,
    hd_available: bool,
    steps: u64,
    /// 上一步回報的 `Frame.ticks`。
    ticks: u16,
    /// 上一步沒有繪製命令（輪詢、暫停），畫面停在最新一幀，不補間。
    hold: bool,
    /// 目前交給核心的強化選項。
    rules: Rules,
    /// 喇叭閘門函式沒有被改成 `retn`。
    speaker_on: bool,
    assets: Assets,
    audio: Vec<AudioMsg>,
    rgb: Vec<u8>,
    frame: Vec<u8>,
    logged_missing: usize,
    predictor: Predictor,
    /// 目前有真人輸入（`predict` 呼叫過）。回放與 `--headless-steps` 沒有，照一般補間畫。
    live_input: bool,
    /// 最新一幀畫了玩家機。標題與過場沒有畫，那裡按鍵不會移動任何東西。
    player_drawn: [bool; 2],
    /// 玩家的砲座已脫離機身（規格 005）。分離的砲座不套用機身的預測偏移。
    free_pods: [bool; 2],
    /// 介面層（HUD、打擊回饋）的狀態，全部只看繪製命令與玩家狀態，不碰模擬。見 `feedback`、`ui::hud`。
    ui: UiState,
}

/// HD 介面的執行狀態。時間來自呼叫端傳入的毫秒，不讀時鐘。
struct UiState {
    clock_ms: u64,
    scanner: Scanner,
    feedback: Feedback,
    feedback_on: bool,
    hud_state: HudState,
    hud_flags: HudFlags,
    hud_icons: HudIcons,
    /// 圖示載入時的調色盤名稱，換了調色盤（換關）就重載。
    hud_icons_pal: Option<Option<&'static str>>,
    /// 上一步每位玩家的血量亮格數，用來偵測死亡。
    prev_cells: [usize; 2],
}

impl UiState {
    fn new() -> UiState {
        UiState {
            clock_ms: 0,
            scanner: Scanner::new(),
            feedback: Feedback::new(),
            feedback_on: true,
            hud_state: HudState::new(),
            hud_flags: HudFlags::default(),
            hud_icons: HudIcons::default(),
            hud_icons_pal: None,
            prev_cells: [0; 2],
        }
    }
}

/// `MY` 圖庫第 `index` 項的邏輯像素寬高。
fn my_item_size(a: &Assets, index: u16) -> (i32, i32) {
    a.my.planes[0].get(index as usize).map(|it| (it.stride as i32 * 8, it.height as i32)).unwrap_or((16, 16))
}

impl Session {
    /// 建立並把進入關卡的初始化命令餵給接收端。
    ///
    /// `mono` 是 CGA、MGA 用的單色圖庫，`None`（載入失敗）時 `view.mode` 是 CGA 或 MGA 也退回 VGA，
    /// 呼叫端用 `mono_available` 決定要不要讓使用者選。
    pub fn new(
        inputs: &Inputs,
        assets: Assets,
        mono: Option<Arc<MonoAssets>>,
        hd: Option<HdAssets>,
        begin: Begin,
        fixes: Fixes,
        view: ViewOpts,
    ) -> Session {
        let mut game = match begin {
            Begin::Title => Game::boot(inputs.game_files(), fixes),
            Begin::Stage1(start) => Game::new(&inputs.image, &inputs.dm1, start, fixes),
        };
        let init = game.take_draws();
        let hd_available = hd.is_some();
        let mode = if view.mode.is_mono() && mono.is_none() { DisplayMode::Vga } else { view.mode };
        game.set_terrain_scr(mode.is_mono());
        let mono_hd = match (&mono, hd.as_ref()) {
            (Some(m), Some(h)) => Some(MonoHdPair::new(Arc::clone(m), h.dir())),
            _ => None,
        };
        let mut s = Session {
            game,
            mirror: inputs.mirror(),
            vga: VgaSink::new(assets.clone()),
            layer: LayerSink::new(assets.clone(), hd),
            mono: mono.map(MonoPair::new),
            mono_hd,
            mode,
            ega_quant: EgaQuant::new(),
            view,
            hd_available,
            steps: 0,
            ticks: 0,
            hold: false,
            rules: Rules::ORIGINAL,
            speaker_on: true,
            assets,
            audio: Vec::new(),
            rgb: vec![0; 320 * 200 * 3],
            frame: Vec::new(),
            logged_missing: 0,
            predictor: Predictor::new(),
            live_input: false,
            player_drawn: [false; 2],
            free_pods: [false; 2],
            ui: UiState::new(),
        };
        // HD 畫面的標題選單選中列用圓角高亮條代替取反（只影響 HD 引擎，原版圖與 VgaSink 不變）。
        s.layer.set_menu_highlight(true);
        s.set_counter(view.counter);
        s.feed(&init);
        s
    }

    /// 幀計數開關給所有接收端。
    fn set_counter(&mut self, on: bool) {
        self.vga.show_counter = on;
        if let Some(m) = &mut self.mono {
            m.cga.show_counter = on;
            m.mga.show_counter = on;
        }
        if let Some(m) = &mut self.mono_hd {
            m.cga.show_counter = on;
            m.mga.show_counter = on;
        }
    }

    /// 單色圖庫載入成功，CGA、MGA 選得到。
    pub fn mono_available(&self) -> bool {
        self.mono.is_some()
    }

    /// 實際使用的顯示模式。單色圖庫缺檔時要求 CGA、MGA 會是 VGA。
    pub fn mode(&self) -> DisplayMode {
        self.mode
    }

    /// 這一步玩家的按鍵會不會移動玩家機。
    fn playable(&self, p: usize) -> bool {
        self.player_drawn[p] && self.game.player_status(p).controllable
    }

    /// 把玩家機的邏輯位置交給預測器。空的命令串（暫停中）不改變 `player_drawn`。
    fn sync_predictor(&mut self, draws: &[DrawCmd]) {
        if !draws.is_empty() {
            self.player_drawn = [false; 2];
            for d in draws {
                if let DrawCmd::Blit(b) = d {
                    if let Some(o) = b.obj {
                        if o.pool == Pool::Player {
                            self.player_drawn[(o.slot as usize).min(1)] = true;
                        }
                    }
                }
            }
        }
        for p in 0..2 {
            let st = self.game.player_status(p);
            let pos = self.playable(p).then_some((st.x as i32 * 8, st.y as i32));
            self.predictor.on_step(p, pos);
            self.free_pods[p] = st.pods_detached;
        }
    }

    /// 過了 `dt` 秒，推進玩家機預測。`state` 是下一步時每個 set 1 碼是否按著，`step_secs` 是這一步到下一步的時間。
    /// 只影響 `render`，不碰 `Game`。
    pub fn predict(&mut self, state: &[bool; 128], dt: f32, step_secs: f32) {
        self.live_input = true;
        for p in 0..2 {
            let st = self.game.player_status(p);
            let reach = Reach { up: st.can_up, down: st.can_down, left: st.can_left, right: st.can_right };
            // 暫停中邏輯不會動，不累積。
            let dir = if self.hold { Dir::default() } else { crate::predict::dir_of(state, p) };
            // 細步移動（規格 005）：下一步走半步或整步，看方向和上一個邏輯步採用的方向是否相同。
            let size = crate::predict::next_step(st.fine_step, st.last_move, dir);
            self.predictor.advance_sized(p, dir, reach, dt, step_secs, size);
        }
    }

    /// 玩家 1 的邏輯位置（邏輯像素）。不在畫面上時是 `None`。
    pub fn player_pos(&self) -> Option<(i32, i32)> {
        let st = self.game.player_status(0);
        self.playable(0).then_some((st.x as i32 * 8, st.y as i32))
    }

    /// 有玩家機在畫面上而且可以控制，觸控層用它決定右手兩顆鈕的作用。
    pub fn in_play(&self) -> bool {
        self.playable(0) || self.playable(1)
    }

    /// 這一幀要給繪製層的玩家機偏移。預測只作用在 VGA、EGA 的 `LayerSink` 路徑，單色模式固定原版時序，沒有預測。
    fn player_offsets(&self) -> [Option<(i32, i32)>; 2] {
        if self.live_input && self.view.feel && self.view.tween && !self.mode.is_mono() {
            [self.predictor.offset_hd(0), self.predictor.offset_hd(1)]
        } else {
            [None, None]
        }
    }

    /// 同一批繪製命令餵給全部接收端，不管現在顯示哪一個模式，所以隨時可以切換。
    fn feed(&mut self, draws: &[DrawCmd]) {
        for c in draws {
            self.vga.apply(c);
        }
        if let Some(m) = &mut self.mono {
            for c in draws {
                m.cga.apply(c);
                m.mga.apply(c);
            }
        }
        if let Some(m) = &mut self.mono_hd {
            for c in draws {
                m.cga.apply(c);
                m.mga.apply(c);
            }
        }
        self.layer.push_frame(draws);
    }

    /// 跑一步。`scancodes` 是上一步到這一步之間送達的 set 1 碼。
    pub fn step(&mut self, scancodes: &[u8]) -> Outcome {
        let frame = self.game.step(&Input { scancodes }, &mut |_g: &Game, _pt: u32| {});
        self.steps += 1;
        self.ticks = frame.ticks;
        self.hold = frame.draws.is_empty();
        for c in &frame.audio {
            match *c {
                AudioCmd::Play(m) => self.audio.push(AudioMsg::Play(m)),
                AudioCmd::Stop => self.audio.push(AudioMsg::Stop),
                AudioCmd::Speaker(on) => self.speaker_on = on,
            }
        }
        // 原版只有 E 模式（VGA、EGA）的特效 blit 驅動喇叭，單色模式沒有（docs/re/14-display-modes.md 5.4）。
        if self.speaker_on && self.mode.speaker() {
            for d in &frame.draws {
                if let DrawCmd::Blit(b) = d {
                    let ev = speaker_events(&self.assets, b);
                    if !ev.is_empty() {
                        self.audio.push(AudioMsg::Speaker(ev));
                    }
                }
            }
        }
        self.feed(&frame.draws);
        self.sync_predictor(&frame.draws);
        self.scan_feedback(&frame.draws);
        match frame.exit {
            None => Outcome::Continue,
            Some(e) => Outcome::Ended(e),
        }
    }

    // -----------------------------------------------------------------------
    // HD 介面：HUD 與打擊回饋（規格 005）。只讀模擬的結果，不寫回。
    // -----------------------------------------------------------------------

    /// 介面層的時鐘（毫秒，單調遞增）。HUD 的淡出與呼吸、震動、光暈都以它為準，所以呼叫端傳什麼時間就畫什麼，單元測試不必等。
    pub fn set_clock_ms(&mut self, ms: u64) {
        self.ui.clock_ms = ms;
    }

    /// HUD 右端的標籤與充能條是否顯示。幀計數的值由 `ViewOpts::counter` 決定。
    pub fn set_hud_flags(&mut self, f: HudFlags) {
        self.ui.hud_flags = f;
    }

    /// 打擊回饋選項（規格 005）。實際生效還要 HD 畫面：原貌模式不生效。
    pub fn set_hit_feedback(&mut self, on: bool) {
        if !on {
            self.ui.feedback.clear();
        }
        self.ui.feedback_on = on;
    }

    /// 打擊回饋實際生效：選項開著而且正在用 HD 圖。
    pub fn feedback_active(&self) -> bool {
        self.ui.feedback_on && self.hd_effective()
    }

    /// 畫面震動的位移，HD 像素（輸出 1280×800 的座標）。沒在震動或不是 HD 是 (0, 0)。
    pub fn shake_offset(&self) -> (i32, i32) {
        if self.feedback_active() { self.ui.feedback.shake_offset(self.ui.clock_ms) } else { (0, 0) }
    }

    /// 掃這一步的繪製命令與玩家血量，產生回饋事件。選項關閉時只維持基準，不累積事件。
    fn scan_feedback(&mut self, draws: &[DrawCmd]) {
        let assets = &self.assets;
        let mut events = self.ui.scanner.scan(draws, &|i| my_item_size(assets, i));
        for p in 0..2 {
            let st = self.game.hud_status(p);
            let n = if st.active { hud::hp_cells(st.hp) } else { 0 };
            if self.ui.prev_cells[p] > 0 && n == 0 && st.active {
                events.push(FxEvent::PlayerDown);
            }
            self.ui.prev_cells[p] = n;
        }
        if self.feedback_active() {
            self.ui.feedback.ingest(self.ui.clock_ms, &events);
            for k in self.ui.feedback.take_sfx() {
                self.audio.push(AudioMsg::Sfx(k));
            }
        }
    }

    /// 兩位玩家的 HUD 狀態。
    pub fn hud_status(&self) -> [HudStatus; 2] {
        [self.game.hud_status(0), self.game.hud_status(1)]
    }

    /// 武器圖示依目前調色盤從 HD 道具球載入（`MY` 第 42 到 45 項）。沒有 HD 資料時留空，HUD 畫字母圓形。
    fn ensure_hud_icons(&mut self) {
        let pal = self.layer.palette_name();
        if self.ui.hud_icons_pal == Some(pal) {
            return;
        }
        let mut icons = HudIcons::default();
        for (slot, idx) in hud::WEAPON_MY_INDEX.iter().enumerate() {
            icons.balls[slot] = self.layer.my_image(*idx).map(|(rgba, w, h)| IconImg { w, h, rgba });
        }
        self.ui.hud_icons = icons;
        self.ui.hud_icons_pal = Some(pal);
    }

    /// HD 畫面上的介面層：命中光暈，再蓋 HUD。HUD 只在玩家機在畫面上時畫（標題與過場原版也是黑的）。
    fn draw_hd_ui(&mut self) {
        if self.feedback_active() {
            let t = self.ui.clock_ms;
            self.ui.feedback.draw_glows(&mut self.frame, 1280, 800, t);
        }
        let st = self.hud_status();
        self.ui.hud_state.update(self.ui.clock_ms, &st);
        if !self.player_drawn.iter().any(|d| *d) {
            return;
        }
        self.ensure_hud_icons();
        let flags = HudFlags { counter: if self.view.counter { self.layer.counter_value() } else { None }, ..self.ui.hud_flags };
        let bar = hud::render(&self.ui.hud_state, &st, &flags, &self.ui.hud_icons, self.ui.clock_ms);
        hud::overlay(&mut self.frame, &bar);
    }

    /// 取走累積的音訊訊息。
    pub fn take_audio(&mut self) -> Vec<AudioMsg> {
        std::mem::take(&mut self.audio)
    }

    /// 上一步回報的 `Frame.ticks`：到下一步要等幾個 tick。
    pub fn ticks(&self) -> u16 {
        self.ticks
    }

    /// 已呼叫 `step` 的次數（含暫停步與重新開始之後）。
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// 換顯示選項。模式變了就同步第三關的地形來源（核心在進第三關時讀）；其餘只影響 `render`。
    pub fn set_view(&mut self, v: ViewOpts) {
        self.view = v;
        self.set_counter(v.counter);
        self.set_mode(v.mode);
    }

    /// 換顯示模式。單色圖庫缺檔時要求 CGA、MGA 會被拒絕（回傳假），維持原來的模式。
    ///
    /// 只換 `render` 選哪一個接收端，並呼叫 `Game::set_terrain_scr`（單色讀 `SHT.SCR`，E 讀 `SHT.SCN`）。
    /// 接收端隨時是最新的，所以不用重播，切換當下的畫面是對的。
    pub fn set_mode(&mut self, m: DisplayMode) -> bool {
        if m.is_mono() && self.mono.is_none() {
            return false;
        }
        self.view.mode = m;
        self.mode = m;
        self.game.set_terrain_scr(m.is_mono());
        true
    }

    /// 目前這一幀在視窗裡的比例與縮放濾鏡，要和 `render` 的輸出一起交給 `Presenter::draw`。
    pub fn fit(&self) -> Fit {
        match self.mode {
            DisplayMode::Cga | DisplayMode::Mga => Fit::MONO,
            DisplayMode::Vga | DisplayMode::Ega => {
                if self.hd_effective() {
                    Fit::HD
                } else {
                    Fit::SD
                }
            }
        }
    }

    /// 強化選項（規格 005）：變動時才交給核心，全關時不碰模擬。
    pub fn set_rules(&mut self, rules: Rules) {
        if self.rules != rules {
            self.rules = rules;
            self.game.set_rules(rules);
        }
    }

    /// 砲座分離鍵（規格 005「砲座像 Force」）這一步是否按著。每步跑之前呼叫，核心在按下的邊緣發射、收回砲座。
    /// 選項沒開時核心不看它。
    pub fn set_split_held(&mut self, held: [bool; 2]) {
        for (p, h) in held.into_iter().enumerate() {
            self.game.set_split_held(p, h);
        }
    }

    /// 無敵（規格 005）：呼叫 `Game::set_invincible`（原版作秘技的旗標 `word_273A5`）。
    /// 呼叫端開著時每步呼叫，從沒開過就不呼叫（原版模式不碰資料段）。
    pub fn set_invincible(&mut self, on: bool) {
        self.game.set_invincible(on);
    }

    /// 實際用 HD 圖：選了 HD 而且有 HD 資料夾。單色模式還要有單色圖庫（`MonoHdSink`，圖是 `mono/` 的推導結果）。
    pub fn hd_effective(&self) -> bool {
        self.view.hd && self.hd_available && (!self.mode.is_mono() || self.mono_hd.is_some())
    }

    /// `Game` 狀態對應的原版資料段（`rs2-trace` 的 `Mirror`）的 SHA-256（十六進位）。
    /// 每次呼叫傾印一份 64 KB，只有 `--hash-out` 與測試用。
    pub fn dseg_hash(&self) -> String {
        sha256_hex(self.ds().bytes())
    }

    /// 狀態對應的資料段傾印。
    pub(crate) fn ds(&self) -> rs2_trace::mirror::Dseg {
        self.mirror.dump(self.game.state())
    }

    /// 測試用：在資料段位移寫一個位元組（寫進狀態，下一步套用）。
    #[cfg(test)]
    pub(crate) fn poke8(&mut self, offset: u16, value: u8) {
        self.mirror.poke8(self.game.state_mut(), offset, value);
    }

    /// 測試用：寫一個字組。
    #[cfg(test)]
    pub(crate) fn poke16(&mut self, offset: u16, value: u16) {
        self.mirror.poke16(self.game.state_mut(), offset, value);
    }

    /// 上次呼叫之後新出現的缺圖。
    pub fn new_missing_hd(&mut self) -> Vec<String> {
        let m = self.layer.missing_hd();
        let out = m[self.logged_missing.min(m.len())..].to_vec();
        self.logged_missing = m.len();
        out
    }

    /// 診斷用計數。
    pub fn anomalies(&self) -> (u64, u64) {
        (self.vga.anomalies(), self.layer.anomalies())
    }

    /// 診斷用計數：單色接收端（CGA, MGA）。沒有單色圖庫時是 `None`。
    pub fn mono_anomalies(&self) -> Option<(u64, u64)> {
        self.mono.as_ref().map(|m| (m.cga.anomalies(), m.mga.anomalies()))
    }

    pub fn palette_name(&self) -> Option<&'static str> {
        self.layer.palette_name()
    }

    /// 輸出目前畫面的 RGBA 與尺寸。依顯示模式（見檔頭的表）：
    ///
    /// - VGA，補間關且原版圖：`VgaSink::render_page(取樣時畫的那一頁)`，320×200，逐像素等於原版。
    /// - VGA 其餘：`LayerSink::render(alpha, ..)`，原版圖 320×200，HD 1280×800。
    /// - EGA，補間關且原版圖：`VgaSink::render_page_ega`，320×200，逐像素。
    /// - EGA 其餘：`LayerSink` 的輸出，每個像素量化到目前 EGA 16 色中最近的一色。這是強推論，待使用者看畫面決定
    ///   （補間與 HD 的畫面沒有 EGA 的原版對照）。
    /// - CGA、MGA：`MonoSink::render_rgba(取樣時畫的那一頁)`，640×200 或 640×300。補間與玩家機預測固定關閉。
    pub fn render(&mut self, alpha: f32) -> (&[u8], usize, usize) {
        let hd = self.hd_effective();
        let alpha = if self.hold { 1.0 } else { alpha };
        let (w, h);
        if let Some(sink) = self.mono.as_ref().and_then(|m| m.sink(self.mode)) {
            // 單色：補間不支援（固定原版時序）。HD 時用 `MonoHdSink`，1280×800，CGA 與 MGA 的 HD 畫面比例都是 4:3
            // （SD 的 640×300 垂直 1.5 倍是 MGA 顯示的產物，HD 不重現）。
            let page = self.layer.draw_page();
            match self.mono_hd.as_ref().and_then(|m| m.sink(self.mode)).filter(|_| hd) {
                Some(hd_sink) => (w, h) = hd_sink.render_rgba(page, &mut self.frame),
                None => (w, h) = sink.render_rgba(page, &mut self.frame),
            }
        } else if !self.view.tween && !hd {
            let page = self.layer.draw_page();
            if self.mode == DisplayMode::Ega {
                self.vga.render_page_ega(page, &mut self.rgb);
            } else {
                self.vga.render_page(page, &mut self.rgb);
            }
            self.frame.resize(320 * 200 * 4, 0);
            for (dst, src) in self.frame.chunks_exact_mut(4).zip(self.rgb.chunks_exact(3)) {
                dst[..3].copy_from_slice(src);
                dst[3] = 255;
            }
            (w, h) = (320, 200);
        } else {
            let player = self.player_offsets();
            // 分離的砲座不吃機身的預測偏移（規格 005「砲座像 Force」）。
            self.layer.set_free_pods(self.free_pods);
            let opts = RenderOpts { interpolate: self.view.tween, hd, show_counter: self.view.counter, player };
            let (img, iw, ih) = self.layer.render(alpha, opts);
            self.frame.clear();
            self.frame.extend_from_slice(img);
            if self.mode == DisplayMode::Ega {
                let pal = self.vga.ega_colors();
                self.ega_quant.apply(&pal, &mut self.frame);
            }
            (w, h) = (iw, ih);
        }
        // 規格 005「HD 介面」：HD 畫面才有現代化的 HUD 與回饋。原貌模式（上面第一個分支）不碰，逐像素等於 `VgaSink`。
        if hd && (w, h) == (1280, 800) {
            self.draw_hd_ui();
            if self.mode.is_mono() {
                desaturate(&mut self.frame);
            }
        }
        (&self.frame, w, h)
    }
}

/// 單色畫面沒有彩色：HUD 與命中光暈轉成灰階。遊戲畫面本來就只有黑與白，轉完不變。
fn desaturate(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let luma = (u32::from(px[0]) * 77 + u32::from(px[1]) * 150 + u32::from(px[2]) * 29) >> 8;
        px[..3].fill(luma as u8);
    }
}

/// 顯示模式的測試（切換不影響模擬、切換當下顯示正確等），放在子模組才讀得到 `Session` 的私有欄位。
#[cfg(test)]
#[path = "session_modes_tests.rs"]
mod modes_tests;

/// 射擊與操控選項（規格 005）的測試：決定性、全關等於原版、四個選項組合、預測、截圖。
#[cfg(test)]
#[path = "session_shoot_tests.rs"]
mod shoot_tests;

#[cfg(test)]
mod tests {
    //! 需要原版檔，標 `#[ignore]`，由 `tools/rust.sh play-test` 帶 `--include-ignored` 跑。

    use super::*;
    use rs2_trace::load_inputs;
    use std::path::PathBuf;

    const VIEW: ViewOpts = ViewOpts { tween: true, counter: false, hd: false, feel: true, mode: DisplayMode::Vga };
    const RIGHT: u8 = 0x4D;
    const LEFT: u8 = 0x4B;

    fn orig() -> PathBuf {
        PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh play-test 跑"))
    }

    #[test]
    fn desaturate_keeps_black_and_white_and_grays_the_rest() {
        let mut px = [0, 0, 0, 255, 255, 255, 255, 255, 70, 200, 160, 255];
        desaturate(&mut px);
        assert_eq!(&px[..8], &[0, 0, 0, 255, 255, 255, 255, 255], "遊戲畫面的黑白不變");
        assert_eq!(px[8], px[9]);
        assert_eq!(px[9], px[10]);
        assert!(px[8] > 0 && px[8] < 255);
    }

    fn open(begin: Begin, view: ViewOpts) -> Session {
        let dir = orig();
        let inputs = load_inputs(&dir).unwrap();
        let assets = Assets::load(&dir).unwrap();
        Session::new(&inputs, assets, None, None, begin, Fixes::ALL, view)
    }

    fn new_game(view: ViewOpts) -> Session {
        open(Begin::Stage1(Start::NewGame { difficulty: None }), view)
    }

    /// 跑到玩家機可以控制而且畫面有東西。
    fn until_playable(s: &mut Session) {
        for _ in 0..400 {
            s.step(&[]);
            if s.playable(0) && !s.hold {
                // 再多幾步，讓開場的補間與進場結束。
                for _ in 0..3 {
                    s.step(&[]);
                }
                return;
            }
        }
        panic!("400 步內玩家機沒有可以控制");
    }

    fn pressed(code: u8) -> [bool; 128] {
        let mut k = [false; 128];
        k[code as usize] = true;
        k
    }

    /// 預測的前提：可以控制的每一步，玩家機照按鍵走滿一步。四關都要成立，否則 `lead` 會一直猜錯。
    #[test]
    #[ignore]
    fn d1_every_stage_moves_one_step_per_logic_step() {
        let mut s = open(Begin::Title, VIEW);
        let mut stats: Vec<(u32, u32, u32, u64)> = Vec::new(); // 可控制步數、照按鍵走滿的步數、不符的步數、ticks 累計
        let mut in_stage = 0u32;
        let (mut dir, mut held) = (RIGHT, false);
        let mut idle = 0u32;
        let mut release: Vec<u8> = Vec::new();
        for n in 0..60_000u32 {
            // 不死（`word_273A5` 的作弊旗標），才量得到整關。
            s.poke8(rs2_trace::mirror::a(0x273A5), b'Y');
            let before = s.game.player_status(0);
            let was_playable = s.playable(0);
            let mut sc: Vec<u8> = std::mem::take(&mut release);
            if was_playable {
                if stats.is_empty() || in_stage == 0 {
                    stats.push((0, 0, 0, 0));
                    in_stage = 1;
                }
                if !held {
                    sc.push(dir);
                    held = true;
                }
                // 走到牆邊就換方向，順便測到界限。
                if (dir == RIGHT && !before.can_right) || (dir == LEFT && !before.can_left) {
                    sc.push(dir | 0x80);
                    dir = if dir == RIGHT { LEFT } else { RIGHT };
                    sc.push(dir);
                }
                if stats.last().unwrap().0 >= 240 {
                    // 這關量夠了，用 Ctrl+C 過關。
                    sc.push(dir | 0x80);
                    held = false;
                    // 熱鍵看的是按鍵狀態，放開碼要留到下一步。
                    sc.extend_from_slice(&[0x1D, 0x2E]);
                    release.extend_from_slice(&[0xAE, 0x9D]);
                    in_stage = 0;
                }
                idle = 0;
            } else {
                in_stage = 0;
                // 換關時按鍵狀態表可能被清掉，進關後重按。
                if held {
                    sc.push(dir | 0x80);
                    held = false;
                }
                idle += 1;
                // 標題與過場等按鍵。
                if idle % 40 == 0 {
                    sc.push(0x39);
                    release.push(0xB9);
                }
            }
            let down = held;
            let Outcome::Continue = s.step(&sc) else {
                eprintln!("[d1] 第 {n} 步遊戲結束");
                break;
            };
            if stats.len() >= 4 && stats[3].0 >= 240 {
                break;
            }
            let after = s.game.player_status(0);
            if was_playable && s.playable(0) && !s.hold && down {
                let st = stats.last_mut().unwrap();
                st.0 += 1;
                st.3 += s.ticks() as u64;
                let moved = (after.x as i32 - before.x as i32) * 8;
                let can = if dir == RIGHT { before.can_right } else { before.can_left };
                let want = match (dir == RIGHT, can) {
                    (_, false) => 0,
                    (true, true) => 16,
                    (false, true) => -16,
                };
                if moved == want {
                    st.1 += 1;
                } else {
                    st.2 += 1;
                    if st.2 <= 5 {
                        eprintln!(
                            "[d1] 第 {n} 步不符：x {} -> {}，方向 {dir:#x}，預期 {want}，實際 {moved}，幀 {}",
                            before.x,
                            after.x,
                            s.ds().r16(0x422)
                        );
                    }
                }
            }
        }
        for (i, st) in stats.iter().enumerate() {
            eprintln!(
                "[d1] 第 {} 段：可控制 {} 步，走滿 {}，不符 {}，平均 {:.2} tick",
                i + 1,
                st.0,
                st.1,
                st.2,
                st.3 as f64 / st.0.max(1) as f64
            );
        }
        assert!(stats.len() >= 4, "沒有走完四關：{stats:?}");
        for st in &stats {
            assert!(st.0 > 0);
            assert!(st.2 * 50 <= st.0, "不符超過 2%：{stats:?}");
        }
    }

    /// 示範錄影在開車的時候不算可以控制：不按鍵從標題一路跑，玩家機動了就不能是 playable。
    #[test]
    #[ignore]
    fn d5_demo_is_not_playable() {
        let mut s = open(Begin::Title, VIEW);
        let (mut playable_steps, mut drawn_steps, mut moved_in_demo) = (0u32, 0u32, 0u32);
        let mut last = None;
        for _ in 0..6000 {
            if let Outcome::Ended(_) = s.step(&[]) {
                break;
            }
            let st = s.game.player_status(0);
            let flag = s.ds().r8(rs2_trace::mirror::a(0x1CFDC));
            if s.player_drawn[0] {
                drawn_steps += 1;
                if last.is_some_and(|l| l != (st.x, st.y)) {
                    moved_in_demo += 1;
                }
                last = Some((st.x, st.y));
            }
            if s.playable(0) {
                playable_steps += 1;
                if playable_steps <= 3 {
                    eprintln!("[d5] 可控制：位置 ({},{})，byte_1CFDC = {flag:#x}，步 {}", st.x, st.y, s.steps());
                }
            }
        }
        eprintln!("[d5] 畫了玩家機 {drawn_steps} 步，其中移動 {moved_in_demo} 次，判為可控制 {playable_steps} 步");
        assert!(drawn_steps > 100, "這段要涵蓋示範錄影");
        assert!(moved_in_demo > 0, "示範錄影要真的有移動");
        assert_eq!(playable_steps, 0, "沒有按任何鍵，不應該有可控制的步");
    }

    /// 真的開始遊戲，之後不按任何鍵：可以控制的期間玩家機不動。
    /// 原版放著不動約在幀 129 被打死，死亡動畫（`sub_14679`）每幀往下往右走，那段不算可以控制，預測不能跟著猜。
    #[test]
    #[ignore]
    fn d6_no_input_means_no_motion() {
        let mut s = open(Begin::Title, VIEW);
        let mut last: Option<(i16, i16)> = None;
        let (mut steps_in_play, mut moves) = (0u32, 0u32);
        let mut idle = 0u32;
        let mut hold_release: Vec<u8> = Vec::new();
        for _ in 0..4000 {
            let mut sc = std::mem::take(&mut hold_release);
            if !s.playable(0) && steps_in_play == 0 {
                idle += 1;
                if idle % 40 == 0 {
                    sc.push(0x39);
                    hold_release.push(0xB9);
                }
            }
            if let Outcome::Ended(_) = s.step(&sc) {
                break;
            }
            if s.playable(0) {
                steps_in_play += 1;
                let st = s.game.player_status(0);
                let pos = (st.x, st.y);
                if last.is_some_and(|l| l != pos) {
                    moves += 1;
                    if moves <= 4 {
                        let d = s.ds();
                        let k = |l: u32| d.r8(rs2_trace::mirror::a(l));
                        eprintln!(
                            "[d6] 可控制第 {steps_in_play} 步移動到 ({},{})：274DA {:#x} 274DB {:#x} 274DC {:#x} 1CFDC {:#x} 1CFDD {:#x} 1CFAD {:#x} 幀 {}",
                            st.x, st.y, k(0x274DA), k(0x274DB), k(0x274DC), k(0x1CFDC), k(0x1CFDD), k(0x1CFAD),
                            d.r16(0x422)
                        );
                    }
                }
                last = Some(pos);
            } else if steps_in_play > 0 {
                let st = s.game.player_status(0);
                eprintln!("[d6] 第 {steps_in_play} 步後不再可控制，位置 ({},{})", st.x, st.y);
                assert!(s.predictor.offset_hd(0).is_none(), "不能控制時要清掉預測");
                break;
            }
        }
        eprintln!("[d6] 可控制 {steps_in_play} 步，移動 {moves} 次");
        assert!(steps_in_play > 100);
        assert_eq!(moves, 0, "沒有按鍵，玩家機不該動");
    }

    /// 按鍵之後下一次繪製玩家機就動，只有玩家機周圍的像素改變。
    #[test]
    #[ignore]
    fn d2_key_press_shows_on_the_next_draw() {
        let mut s = new_game(VIEW);
        until_playable(&mut s);
        let st = s.game.player_status(0);
        assert!(st.can_right);
        let base = s.render(1.0).0.to_vec();
        let step_secs = s.ticks() as f32 * crate::pacer::TICK_SECS as f32;
        assert!(step_secs > 0.0);

        // 沒有按鍵：預測開著也和一般補間逐像素相同。
        s.predict(&[false; 128], 0.0, step_secs);
        assert_eq!(s.render(1.0).0, &base[..], "沒按鍵時不能改變畫面");

        // 按右：過半步的時間，玩家機往右 8 像素。
        s.predict(&pressed(RIGHT), step_secs / 2.0, step_secs);
        assert_eq!(s.predictor.offset_hd(0), Some((32, 0)));
        let (img, w, h) = s.render(1.0);
        assert_eq!((w, h), (320, 200));
        let px = st.x as i32 * 8;
        let py = st.y as i32;
        let (mut x0, mut x1, mut y0, mut y1, mut diff) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN, 0u32);
        for y in 0..200i32 {
            for x in 0..320i32 {
                let o = (y * 320 + x) as usize * 4;
                if img[o..o + 4] != base[o..o + 4] {
                    (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
                    diff += 1;
                }
            }
        }
        eprintln!("[d2] 玩家機 ({px},{py})，改變 {diff} 像素，範圍 x {x0}..{x1} y {y0}..{y1}");
        assert!(diff > 0, "按鍵後畫面要變");
        assert!(x0 >= px - 48 && x1 <= px + 112 && y0 >= py - 48 && y1 <= py + 64, "只有玩家機周圍會變");
        // 畫面和不按鍵的基準只差這個偏移：關掉預測後回到基準。
        s.set_view(ViewOpts { feel: false, ..VIEW });
        assert_eq!(s.render(1.0).0, &base[..], "關掉預測要回到基準");
    }

    /// 無敵：開著時旗標每步被寫成 `Y`，關掉寫回 `N`；從沒呼叫過的那一份完全不被碰。
    #[test]
    #[ignore]
    fn d5_invincible_writes_the_cheat_flag_only_when_asked() {
        let flag = |s: &Session| s.ds().r8(rs2_trace::mirror::a(0x273A5));
        let mut plain = new_game(VIEW);
        let mut god = new_game(VIEW);
        for _ in 0..20 {
            plain.step(&[]);
            god.step(&[]);
        }
        assert_eq!(plain.dseg_hash(), god.dseg_hash(), "沒呼叫過就和原版相同");
        let before = flag(&plain);
        for _ in 0..5 {
            god.set_invincible(true);
            god.step(&[]);
            assert_eq!(flag(&god), b'Y', "開著時每步都是 Y");
        }
        god.set_invincible(false);
        god.step(&[]);
        assert_eq!(flag(&god), b'N');
        for _ in 0..5 {
            plain.step(&[]);
        }
        assert_eq!(flag(&plain), before, "沒呼叫的那份旗標沒有被動");
    }

    /// 預測、繪製、切換開關都不碰模擬。
    #[test]
    #[ignore]
    fn d3_prediction_leaves_the_simulation_alone() {
        let script = |n: usize| -> Vec<u8> {
            match n % 40 {
                5 => vec![RIGHT],
                25 => vec![RIGHT | 0x80, LEFT],
                35 => vec![LEFT | 0x80],
                _ => Vec::new(),
            }
        };
        let mut a = new_game(ViewOpts { feel: false, tween: false, ..VIEW });
        let mut b = new_game(VIEW);
        let mut moved = false;
        for n in 0..600 {
            let sc = script(n);
            a.step(&sc);
            b.step(&sc);
            assert_eq!(a.dseg_hash(), b.dseg_hash(), "第 {n} 步");
            for f in 0..3 {
                b.predict(&pressed(RIGHT), 0.02, 0.055);
                moved |= b.predictor.offset_hd(0).is_some_and(|o| o != (0, 0));
                b.render(f as f32 / 3.0);
            }
            if n == 300 {
                b.set_view(ViewOpts { feel: false, ..VIEW });
            }
            if n == 400 {
                b.set_view(VIEW);
            }
        }
        assert!(moved, "這段輸入要真的用到預測");
    }

    /// 真實的節奏下顯示位置連續：邏輯步到來時不跳，按住時領先不超過一步。
    #[test]
    #[ignore]
    fn d4_shown_position_stays_continuous_while_held() {
        let mut s = new_game(VIEW);
        until_playable(&mut s);
        let mut sc = vec![RIGHT];
        let keys = pressed(RIGHT);
        let mut last: Option<f32> = None;
        let mut worst = 0f32;
        let mut moved_steps = 0;
        for _ in 0..12 {
            let before = s.game.player_status(0);
            let step_secs = s.ticks() as f32 * crate::pacer::TICK_SECS as f32;
            // 一步畫三次。
            for _ in 0..3 {
                s.predict(&keys, step_secs / 3.0, step_secs);
                let st = s.game.player_status(0);
                let shown = st.x as f32 * 8.0 + s.predictor.offset_hd(0).unwrap().0 as f32 / 4.0;
                if let Some(l) = last {
                    worst = worst.max((shown - l).abs());
                }
                last = Some(shown);
            }
            s.step(&sc);
            sc.clear();
            let st = s.game.player_status(0);
            let shown = st.x as f32 * 8.0 + s.predictor.offset_hd(0).unwrap().0 as f32 / 4.0;
            let l = last.unwrap();
            assert!((shown - l).abs() <= 0.25, "邏輯步到來時畫面跳了：{l} -> {shown}");
            if st.x != before.x {
                moved_steps += 1;
            }
        }
        eprintln!("[d4] 走了 {moved_steps} 步，相鄰兩次繪製最大位移 {worst} 像素");
        assert!(moved_steps >= 8);
        // 一步 16 像素分三次畫，每次約 5.3 像素；第一步要追上一步的差距，上限放到一步以內。
        assert!(worst <= 16.0, "{worst}");
    }

    // -----------------------------------------------------------------------
    // 規格 005「HD 介面」：HUD、打擊回饋、選單高亮。E7：不改模擬，原貌模式逐像素等於 `VgaSink`。
    // -----------------------------------------------------------------------

    fn hd_dir() -> Option<PathBuf> {
        std::env::var("RS2_HD").ok().map(PathBuf::from)
    }

    /// 有 HD 資料夾的 `Session`；沒有 `RS2_HD` 回傳 `None`。
    fn open_hd(begin: Begin, view: ViewOpts) -> Option<Session> {
        let hd_path = hd_dir()?;
        let dir = orig();
        let inputs = load_inputs(&dir).unwrap();
        let assets = Assets::load(&dir).unwrap();
        let mut hd = HdAssets::open(&hd_path).unwrap();
        hd.preload(&assets, 4);
        Some(Session::new(&inputs, assets, None, Some(hd), begin, Fixes::ALL, view))
    }

    const HD_VIEW: ViewOpts = ViewOpts { tween: true, counter: false, hd: true, feel: true, mode: DisplayMode::Vga };

    fn shots_dir() -> Option<PathBuf> {
        std::env::var("RS2_SHOTS").ok().map(PathBuf::from).inspect(|d| std::fs::create_dir_all(d).unwrap())
    }

    fn save_png(dir: &Option<PathBuf>, name: &str, px: &[u8], w: usize, h: usize) {
        if let Some(d) = dir {
            std::fs::write(d.join(name), rs2_gfx::png::encode_rgba(w, h, px)).unwrap();
        }
    }

    /// 玩家 1 的狀態：血量字組、武器字母、砲座數。
    fn set_p1(s: &mut Session, hp: u16, weapon: u8, pods: u8) {
        let yn = |n: u8, k: u8| if n > k { b'Y' } else { b'N' };
        s.poke16(rs2_trace::mirror::a(0x1CFDE), hp);
        s.poke16(rs2_trace::mirror::a(0x1DAA9), weapon as u16);
        s.poke8(rs2_trace::mirror::a(0x1DA8D), yn(pods, 0));
        s.poke8(rs2_trace::mirror::a(0x1DA8E), yn(pods, 1));
    }

    fn bottom_bar(img: &[u8]) -> Vec<u8> {
        img[(hud::Y0 as usize * 1280 * 4)..].to_vec()
    }

    /// 原貌模式（補間關、原版圖）逐像素等於 `VgaSink`；HUD、打擊回饋的開關與設定都不改它。
    #[test]
    #[ignore]
    fn e7_original_mode_is_pixel_identical_to_vga_sink_whatever_the_hd_ui_settings() {
        let off = ViewOpts { tween: false, counter: false, hd: false, feel: true, mode: DisplayMode::Vga };
        let mut plain = new_game(off);
        let mut ui = new_game(off);
        ui.set_hit_feedback(true);
        ui.set_hud_flags(HudFlags { rules: true, god: true, charge_bar: true, counter: Some(9) });
        let mut compared = 0;
        for n in 0..260 {
            let sc: Vec<u8> = if n == 40 { vec![0x39] } else { Vec::new() };
            plain.step(&sc);
            ui.step(&sc);
            ui.set_clock_ms(n * 110);
            let a = plain.render(1.0).0.to_vec();
            let (b, w, h) = {
                let (b, w, h) = ui.render(1.0);
                (b.to_vec(), w, h)
            };
            assert_eq!((w, h), (320, 200), "原貌模式輸出 320x200");
            assert_eq!(a, b, "第 {n} 步：HUD 與回饋的設定不能改變原貌畫面");
            // 正對照：直接問 VgaSink。
            let mut rgb = vec![0u8; 320 * 200 * 3];
            ui.vga.render_page(ui.layer.draw_page(), &mut rgb);
            let want: Vec<u8> = rgb.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
            assert_eq!(b, want, "第 {n} 步：不等於 VgaSink");
            compared += 1;
        }
        assert!(compared > 0);
        assert_eq!(plain.dseg_hash(), ui.dseg_hash(), "模擬沒有被改到");
    }

    /// HUD、打擊回饋與選單高亮不改每步雜湊：同樣的輸入，開與關兩份的 `Dseg` 完全相同。
    #[test]
    #[ignore]
    fn e7_hud_feedback_and_menu_highlight_never_change_the_simulation() {
        let Some(mut on) = open_hd(Begin::Title, HD_VIEW) else { return };
        let mut off = open_hd(Begin::Title, HD_VIEW).unwrap();
        off.set_hit_feedback(false);
        off.layer.set_menu_highlight(false);
        on.set_hit_feedback(true);
        let mut released: Vec<u8> = Vec::new();
        let mut shown = 0u32;
        for n in 0..900u64 {
            // 標題、選單與進關：偶爾按 C 與空白鍵。
            let mut sc: Vec<u8> = std::mem::take(&mut released);
            if n % 37 == 5 {
                sc.push(0x2E);
                released.push(0xAE);
            } else if n % 53 == 7 {
                sc.push(0x39);
                released.push(0xB9);
            }
            on.set_clock_ms(n * 110);
            off.set_clock_ms(n * 110);
            on.step(&sc);
            off.step(&sc);
            assert_eq!(on.dseg_hash(), off.dseg_hash(), "第 {n} 步");
            on.render(0.5);
            off.render(1.0);
            shown += 1;
        }
        assert!(shown > 0);
        // 進了關卡之後再跑一段，涵蓋 HUD 與事件掃描。
        let mut a = open_hd(Begin::Stage1(Start::NewGame { difficulty: None }), HD_VIEW).unwrap();
        let mut b = open_hd(Begin::Stage1(Start::NewGame { difficulty: None }), HD_VIEW).unwrap();
        b.set_hit_feedback(false);
        for n in 0..500u64 {
            let sc: Vec<u8> = if n == 3 { vec![0x39] } else { Vec::new() };
            a.set_clock_ms(n * 110);
            b.set_clock_ms(n * 110);
            a.step(&sc);
            b.step(&sc);
            assert_eq!(a.dseg_hash(), b.dseg_hash(), "關卡第 {n} 步");
            a.render(1.0);
            b.render(1.0);
        }
    }

    /// HUD 只蓋底部 40 列；上面 760 列和沒有 HUD 時相同。標題沒有玩家機，不畫 HUD。
    #[test]
    #[ignore]
    fn hud_covers_only_the_bottom_forty_rows_and_only_while_the_player_is_on_screen() {
        let Some(mut s) = open_hd(Begin::Title, HD_VIEW) else { return };
        for _ in 0..30 {
            s.step(&[]);
        }
        assert!(!s.player_drawn.iter().any(|d| *d), "標題沒有玩家機");
        let title = s.render(1.0).0.to_vec();
        let st = s.hud_status();
        let _ = st;
        // 標題的最下面 40 列和 `LayerSink` 原樣輸出相同（沒有 HUD）。
        let raw = s.layer.render(1.0, RenderOpts { interpolate: true, hd: true, show_counter: false, player: [None, None] }).0.to_vec();
        assert_eq!(bottom_bar(&title), bottom_bar(&raw), "標題不畫 HUD");

        let mut g = open_hd(Begin::Stage1(Start::NewGame { difficulty: None }), HD_VIEW).unwrap();
        until_playable(&mut g);
        let raw_opts = RenderOpts { interpolate: true, hd: true, show_counter: false, player: [None, None] };
        // `poke` 在下一步才寫進資料段，所以每次改狀態都要走一步。
        set_p1(&mut g, 0xFF, b'L', 1);
        g.step(&[]);
        let with = g.render(1.0).0.to_vec();
        let raw = g.layer.render(1.0, raw_opts).0.to_vec();
        assert_eq!(with[..hud::Y0 as usize * 1280 * 4], raw[..hud::Y0 as usize * 1280 * 4], "上面 760 列不動");
        assert_ne!(bottom_bar(&with), bottom_bar(&raw), "底部 40 列是 HUD");
        // HUD 讀的是玩家狀態：改血量與武器，底部跟著變，上面仍然不動。
        set_p1(&mut g, 0x03, b'K', 2);
        g.step(&[]);
        assert_eq!(hud::hp_cells(g.hud_status()[0].hp), 2);
        let after = g.render(1.0).0.to_vec();
        let raw2 = g.layer.render(1.0, raw_opts).0.to_vec();
        assert_eq!(after[..hud::Y0 as usize * 1280 * 4], raw2[..hud::Y0 as usize * 1280 * 4], "改血量之後上面仍不動");
        assert_ne!(bottom_bar(&with), bottom_bar(&after), "血量與武器變了，HUD 跟著變");
    }

    /// 存圖：各種 HUD 狀態（`RS2_SHOTS` 指定目錄才存），並驗證血條格數與玩家狀態一致。
    #[test]
    #[ignore]
    fn ui_shots_hud_states() {
        let Some(mut g) = open_hd(Begin::Stage1(Start::NewGame { difficulty: None }), HD_VIEW) else { return };
        let dir = shots_dir();
        until_playable(&mut g);
        let mut t = 100_000u64;
        let shot = |g: &mut Session, name: &str, clock: u64| {
            // poke 在下一步才寫進資料段。
            g.step(&[]);
            g.set_clock_ms(clock);
            let (px, w, h) = g.render(1.0);
            assert_eq!((w, h), (1280, 800));
            save_png(&dir, name, px, w, h);
        };
        set_p1(&mut g, 0xFF, b'K', 2);
        shot(&mut g, "hud_stage1_full.png", t);
        set_p1(&mut g, 0x07, b'B', 1);
        shot(&mut g, "hud_stage1_amber.png", t + 50);
        // 剩 1 格：呼吸最亮與最暗各一張。
        set_p1(&mut g, 0x01, b'L', 0);
        shot(&mut g, "hud_stage1_low_bright.png", 200_000);
        shot(&mut g, "hud_stage1_low_dim.png", 200_500);
        // 掉格淡出：從 7 格掉到 6 格，剛掉與 100 毫秒後。
        set_p1(&mut g, 0x7F, b'S', 2);
        shot(&mut g, "tmp.png", t + 1000);
        let _ = std::fs::remove_file(dir.as_ref().map(|d| d.join("tmp.png")).unwrap_or_default());
        set_p1(&mut g, 0x3F, b'S', 2);
        t += 1100;
        shot(&mut g, "hud_stage1_fade_0ms.png", t);
        shot(&mut g, "hud_stage1_fade_100ms.png", t + 100);
        // 雙人：P2 在場，兩邊武器不同，標籤與幀計數都開。
        g.poke8(rs2_trace::mirror::a(0x1CFDD), 0x11);
        g.poke16(rs2_trace::mirror::a(0x1CFE0), 0x1F);
        g.poke16(rs2_trace::mirror::a(0x1DB0B), b'B' as u16);
        g.poke8(rs2_trace::mirror::a(0x1DAEF), b'Y');
        g.poke8(rs2_trace::mirror::a(0x1DAF0), b'N');
        set_p1(&mut g, 0xFF, b'L', 2);
        g.set_hud_flags(HudFlags { rules: true, god: true, charge_bar: true, counter: None });
        g.set_view(ViewOpts { counter: true, ..HD_VIEW });
        shot(&mut g, "hud_two_players.png", t + 5000);
        let st = g.hud_status();
        assert!(st[0].active && st[1].active);
        assert_eq!(hud::hp_cells(st[0].hp), 8);
        assert_eq!(hud::hp_cells(st[1].hp), 5);
        assert_eq!((st[0].pods, st[1].pods), (2, 1));
    }

    /// 存圖與驗證：用 boss1 收據跑，抓命中光暈與震動。`RS2_DOSRUN` 要有 `boss1_kill/trace-s1.jsonl`。
    #[test]
    #[ignore]
    fn ui_shots_hit_feedback_from_the_boss_receipt() {
        let Some(mut s) = open_hd(Begin::Stage1(Start::NewGame { difficulty: None }), HD_VIEW) else { return };
        let dir = shots_dir();
        let receipt = PathBuf::from(std::env::var("RS2_DOSRUN").expect("缺 RS2_DOSRUN")).join("boss1_kill/trace-s1.jsonl");
        let text = std::fs::read_to_string(receipt).unwrap();
        let steps: Vec<Vec<u8>> = text.lines().filter(|l| !l.trim().is_empty()).map(rs2_trace::parse_sc).collect();
        let (mut glow_shot, mut shake_shot) = (false, false);
        let (mut hits, mut booms, mut fires, mut sfx) = (0u32, 0u32, 0u32, 0u32);
        let mut quiet = Session::new(&load_inputs(&orig()).unwrap(), Assets::load(&orig()).unwrap(), None, None, Begin::Stage1(Start::NewGame { difficulty: None }), Fixes::ALL, HD_VIEW);
        quiet.set_hit_feedback(false);
        for (n, sc) in steps.iter().enumerate() {
            let clock = 1000 + n as u64 * 110;
            s.set_clock_ms(clock);
            s.step(sc);
            quiet.step(sc);
            assert_eq!(s.dseg_hash(), quiet.dseg_hash(), "打擊回饋不改模擬（第 {n} 步）");
            for m in s.take_audio() {
                if let AudioMsg::Sfx(k) = m {
                    sfx += 1;
                    match k {
                        SfxKind::Fire => fires += 1,
                        SfxKind::Boom => booms += 1,
                    }
                }
            }
            let g = s.ui.feedback.glow_count(clock);
            hits += g as u32;
            if g > 0 && !glow_shot && n > 100 {
                let (px, w, h) = s.render(1.0);
                save_png(&dir, "hit_glow.png", px, w, h);
                glow_shot = true;
            }
            if s.shake_offset() != (0, 0) && !shake_shot {
                let off = s.shake_offset();
                eprintln!("[shots] 第 {n} 步開始震動，位移 {off:?}");
                let (px, w, h) = s.render(1.0);
                save_png(&dir, "boom_frame.png", px, w, h);
                shake_shot = true;
            }
        }
        eprintln!("[shots] boss1 收據 {} 步：光暈累計 {hits}，音效 {sfx}（開火 {fires}、爆炸 {booms}），光暈圖 {glow_shot}，震動圖 {shake_shot}", steps.len());
        assert!(glow_shot, "收據裡要有命中火花");
        assert!(shake_shot, "收據裡要有大爆炸");
        assert!(fires > 0 && booms > 0);
    }

    /// 標題選單：HD 選中列是圓角高亮條，不取反；存圖並確認有高亮色。
    #[test]
    #[ignore]
    fn ui_shots_title_menu_highlight() {
        let Some(mut s) = open_hd(Begin::Title, HD_VIEW) else { return };
        let dir = shots_dir();
        let mut released: Vec<u8> = Vec::new();
        let mut found = None;
        for n in 0..3000u64 {
            let mut sc: Vec<u8> = std::mem::take(&mut released);
            if n % 25 == 3 && found.is_none() {
                sc.push(0x2E);
                released.push(0xAE);
            }
            s.set_clock_ms(n * 110);
            s.step(&sc);
            if s.palette_name() == Some("menu") && found.is_none() {
                found = Some(n);
                // 走幾步讓選單畫完。
                for _ in 0..6 {
                    s.step(&[]);
                }
                break;
            }
        }
        let n = found.expect("3000 步內進不了選單");
        eprintln!("[shots] 第 {n} 步進入選單");
        let (px, w, h) = s.render(1.0);
        save_png(&dir, "title_menu_row1.png", px, w, h);
        // 第一列（LEVEL:）選中：圖的左上角附近（底色處）是高亮條的藍色；第二列（CTRL:）沒選中，仍是近黑。
        let at = |x: usize, y: usize| [px[(y * w + x) * 4], px[(y * w + x) * 4 + 1], px[(y * w + x) * 4 + 2]];
        let bar = at(650, 130);
        let idle = at(650, 170);
        assert!(bar[2] > 150 && bar[2] as i32 > bar[0] as i32 + 60, "選中列有圓角高亮條：{bar:?}");
        assert!(idle[2] < 30, "沒選中的列不變：{idle:?}");
        // 圓角：圖矩形的左上角那個像素在圓角之外，仍是黑底。
        assert!(at(640, 120)[2] < 60, "圓角把角落切掉：{:?}", at(640, 120));
        // 字不取反：矩形裡有亮字（銀灰加亮後接近白），不是黑字。
        let bright = (120..156).flat_map(|y| (640..864).map(move |x| (x, y))).filter(|&(x, y)| at(x, y)[0] > 200).count();
        assert!(bright > 200, "字是亮的：{bright}");
        // 按下鍵換到第二列。
        s.step(&[0x50]);
        s.step(&[0xD0]);
        for _ in 0..6 {
            s.step(&[]);
        }
        let (px, w, h) = s.render(1.0);
        save_png(&dir, "title_menu_row2.png", px, w, h);
    }
}
