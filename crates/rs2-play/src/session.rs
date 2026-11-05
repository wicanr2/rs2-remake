//! 一局遊戲：`Game` 加兩個繪製接收端，以及視窗模式與 `--headless-steps` 共用的步進與繪製路徑。
//! 不依賴 SDL，所以主迴圈以外的邏輯（模式選擇、排程）可以直接測。
//!
//! `Game::step` 回傳的 `draws` 同時餵給 `VgaSink`（補間關閉的原版圖）與 `LayerSink`（補間與 HD），
//! 兩邊都隨時是最新的，所以 F1、F3 切換不需要重播。畫面顯示的選擇只影響 `render`，不碰 `Game`。

use rs2_audio::SpkEv;
use rs2_core::{AudioCmd, DrawCmd, Entry, Exit, Fixes, Game, Input, MixFile, Start, boot_ds_with_dm1};
use rs2_formats::sha256_hex;
use rs2_gfx::speaker::speaker_events;
use rs2_gfx::{Assets, HdAssets, LayerSink, RenderOpts, VgaSink};
use rs2_trace::Inputs;

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
}

/// 顯示選項。`hd` 只有在 `Session::hd_available()` 時才生效。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewOpts {
    pub tween: bool,
    pub counter: bool,
    pub hd: bool,
}

/// `Session::step` 之後主迴圈要做的事。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Continue,
    /// 遊戲結束，主迴圈離開。
    Ended(Exit),
}

pub struct Session {
    game: Game,
    vga: VgaSink,
    layer: LayerSink,
    view: ViewOpts,
    hd_available: bool,
    steps: u64,
    /// 上一步回報的 `Frame.ticks`。
    ticks: u16,
    /// 上一步沒有繪製命令（輪詢、暫停），畫面停在最新一幀，不補間。
    hold: bool,
    /// 喇叭閘門函式沒有被改成 `retn`。
    speaker_on: bool,
    assets: Assets,
    audio: Vec<AudioMsg>,
    rgb: Vec<u8>,
    frame: Vec<u8>,
    logged_missing: usize,
}

impl Session {
    /// 建立並把進入關卡的初始化命令餵給接收端。
    pub fn new(inputs: &Inputs, assets: Assets, hd: Option<HdAssets>, begin: Begin, fixes: Fixes, view: ViewOpts) -> Session {
        let mut game = match begin {
            Begin::Title => {
                let ds = boot_ds_with_dm1(&inputs.image, &inputs.dm1);
                Game::from_snapshot(inputs.game_files(), ds.bytes(), Entry::Title, fixes)
            }
            Begin::Stage1(start) => Game::new(&inputs.image, &inputs.dm1, start, fixes),
        };
        let init = game.take_draws();
        let hd_available = hd.is_some();
        let mut s = Session {
            game,
            vga: VgaSink::new(assets.clone()),
            layer: LayerSink::new(assets.clone(), hd),
            view,
            hd_available,
            steps: 0,
            ticks: 0,
            hold: false,
            speaker_on: true,
            assets,
            audio: Vec::new(),
            rgb: vec![0; 320 * 200 * 3],
            frame: Vec::new(),
            logged_missing: 0,
        };
        s.vga.show_counter = view.counter;
        s.feed(&init);
        s
    }

    fn feed(&mut self, draws: &[DrawCmd]) {
        for c in draws {
            self.vga.apply(c);
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
        if self.speaker_on {
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
        match frame.exit {
            None => Outcome::Continue,
            Some(e) => Outcome::Ended(e),
        }
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

    pub fn set_view(&mut self, v: ViewOpts) {
        self.view = v;
        self.vga.show_counter = v.counter;
    }

    /// 實際用 HD 圖：選了 HD 而且有 HD 資料夾。
    pub fn hd_effective(&self) -> bool {
        self.view.hd && self.hd_available
    }

    /// `Game` 資料段的 SHA-256（十六進位）。
    pub fn dseg_hash(&self) -> String {
        sha256_hex(self.game.state_hash_input())
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

    pub fn palette_name(&self) -> Option<&'static str> {
        self.layer.palette_name()
    }

    /// 輸出目前畫面的 RGBA 與尺寸。
    ///
    /// - 補間關且原版圖：`VgaSink::render_page(取樣時畫的那一頁)`，320×200，逐像素等於原版。
    /// - 其餘：`LayerSink::render(alpha, ..)`，原版圖 320×200，HD 1280×800。
    pub fn render(&mut self, alpha: f32) -> (&[u8], usize, usize) {
        let hd = self.hd_effective();
        let alpha = if self.hold { 1.0 } else { alpha };
        let (w, h);
        if !self.view.tween && !hd {
            self.vga.render_page(self.layer.draw_page(), &mut self.rgb);
            self.frame.resize(320 * 200 * 4, 0);
            for (dst, src) in self.frame.chunks_exact_mut(4).zip(self.rgb.chunks_exact(3)) {
                dst[..3].copy_from_slice(src);
                dst[3] = 255;
            }
            (w, h) = (320, 200);
        } else {
            let opts = RenderOpts { interpolate: self.view.tween, hd, show_counter: self.view.counter };
            let (img, iw, ih) = self.layer.render(alpha, opts);
            self.frame.clear();
            self.frame.extend_from_slice(img);
            (w, h) = (iw, ih);
        }
        (&self.frame, w, h)
    }
}
