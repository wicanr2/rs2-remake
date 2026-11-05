//! 一局遊戲：`Game` 加兩個繪製接收端，以及視窗模式與 `--headless-steps` 共用的步進與繪製路徑。
//! 不依賴 SDL，所以主迴圈以外的邏輯（重新開始、過關畫面、模式選擇）可以直接測。
//!
//! `Game::step` 回傳的 `draws` 同時餵給 `VgaSink`（補間關閉的原版圖）與 `LayerSink`（補間與 HD），
//! 兩邊都隨時是最新的，所以 F1、F3 切換不需要重播。畫面顯示的選擇只影響 `render`，不碰 `Game`。

use crate::font;
use rs2_core::{DrawCmd, Exit, Fixes, Game, Input, Start};
use rs2_formats::sha256_hex;
use rs2_gfx::{Assets, HdAssets, LayerSink, RenderOpts, VgaSink};
use rs2_trace::Inputs;

/// 過關後顯示的提示（這個切片只有第一關）。
pub const CLEAR_TEXT: &str = "STAGE 1 CLEAR - PRESS ENTER";

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
    /// `Exit::End`：已經重新開始。
    Restarted,
    /// `Exit::Clear`：顯示過關畫面，等 Enter。
    Cleared,
    /// `Exit::Quit`：Ctrl+Q。
    Quit,
}

pub struct Session {
    game: Game,
    vga: VgaSink,
    layer: LayerSink,
    view: ViewOpts,
    hd_available: bool,
    steps: u64,
    cleared: bool,
    rgb: Vec<u8>,
    frame: Vec<u8>,
    logged_missing: usize,
}

impl Session {
    /// 建立並把進入關卡的初始化命令餵給接收端。
    pub fn new(inputs: &Inputs, assets: Assets, hd: Option<HdAssets>, start: Start, fixes: Fixes, view: ViewOpts) -> Session {
        let mut game = Game::new(&inputs.image, &inputs.dm1, start, fixes);
        let init = game.take_draws();
        let hd_available = hd.is_some();
        let mut s = Session {
            game,
            vga: VgaSink::new(assets.clone()),
            layer: LayerSink::new(assets, hd),
            view,
            hd_available,
            steps: 0,
            cleared: false,
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
        // 暫停步（frame.paused）沒有推進模擬，draws 是空的，LayerSink 會直接忽略。
        self.feed(&frame.draws);
        match frame.exit {
            None => Outcome::Continue,
            Some(Exit::Quit) => Outcome::Quit,
            Some(Exit::End) => {
                self.restart();
                Outcome::Restarted
            }
            Some(Exit::Clear) => {
                self.cleared = true;
                Outcome::Cleared
            }
        }
    }

    /// 重新開始新局。`Game`、`VgaSink`、`LayerSink` 都沿用，血條殘留等行為依規格維持。
    pub fn restart(&mut self) {
        self.game.restart(Start::NewGame { difficulty: None });
        let init = self.game.take_draws();
        self.feed(&init);
        self.cleared = false;
    }

    pub fn is_cleared(&self) -> bool {
        self.cleared
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
    ///
    /// 過關後疊上提示文字。
    pub fn render(&mut self, alpha: f32) -> (&[u8], usize, usize) {
        let hd = self.hd_effective();
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
        if self.cleared {
            overlay_clear(&mut self.frame, w, h);
        }
        (&self.frame, w, h)
    }
}

/// 在畫面中央疊上過關提示。字的大小隨寬度（320 寬 1 倍）。
pub fn overlay_clear(buf: &mut [u8], w: usize, h: usize) {
    let s = (w / 320).max(1);
    let tw = font::text_width(CLEAR_TEXT, s) as i32;
    let th = font::text_height(s) as i32;
    let (cx, cy) = (w as i32 / 2, h as i32 / 2);
    let pad = 4 * s as i32;
    font::fill_rect(buf, w, h, cx - tw / 2 - pad, cy - th / 2 - pad, tw + 2 * pad, th + 2 * pad, [0, 0, 0, 255]);
    font::draw_text(buf, w, h, cx - tw / 2, cy - th / 2, CLEAR_TEXT, s, [255, 255, 255, 255]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_draws_inside_the_frame_and_keeps_alpha_opaque() {
        for (w, h) in [(320usize, 200usize), (1280, 800)] {
            let mut buf = vec![7u8; w * h * 4];
            overlay_clear(&mut buf, w, h);
            let changed = buf.chunks_exact(4).filter(|p| p != &[7, 7, 7, 7]).count();
            assert!(changed > 0);
            // 提示框在中央，四個角不動。
            for &(x, y) in &[(0usize, 0usize), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
                let o = (y * w + x) * 4;
                assert_eq!(&buf[o..o + 4], &[7, 7, 7, 7]);
            }
            assert!(buf.chunks_exact(4).all(|p| p == [7, 7, 7, 7] || p[3] == 255));
        }
    }
}
