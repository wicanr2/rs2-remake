//! 視窗版面與畫面輸出：視窗大小、遊戲畫面的位置、串流貼圖與 PNG 擷取。
//! 版面計算（`window_size`、`fit_rect`）不碰 SDL，可單元測試；`Presenter` 才用 SDL。

use crate::toolbar::{BarState, Rect as BarRect, Toolbar};
use crate::touch::{self, Control};
use crate::poster::{self, PosterArt, PosterMode};
use rs2_gfx::png;
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::Rect;
use sdl2::render::{BlendMode, ScaleMode, Texture, TextureCreator, WindowCanvas};
use sdl2::video::WindowContext;
use std::path::Path;
use std::sync::Arc;

/// 遊戲畫面的邏輯尺寸。
pub const GAME_W: u32 = 320;
pub const GAME_H: u32 = 200;

/// 倍率 `scale` 的視窗大小（工具列顯示時高度加上工具列與狀態列）。
pub fn window_size(scale: u32, toolbar_visible: bool) -> (u32, u32) {
    let w = GAME_W * scale;
    let bar = if toolbar_visible { crate::toolbar::desktop_height(w) } else { 0 };
    (w, GAME_H * scale + bar)
}

pub fn window_size_with_poster(scale: u32, toolbar_visible: bool, mode: PosterMode) -> (u32, u32) {
    if mode == PosterMode::Off {
        return window_size(scale, toolbar_visible);
    }
    let w = poster::WIDTH * scale;
    let bar = if toolbar_visible { crate::toolbar::desktop_height(w) } else { 0 };
    (w, poster::HEIGHT * scale + bar)
}

/// 海報模式使用定案的中央安全區，原貌與 HD 都保持各自的顯示比例。
pub fn game_rect_with_poster(w: u32, h: u32, bar: u32, fit: Fit, mobile: bool, on: bool) -> BarRect {
    if !on {
        return fit_rect_for(w, h, bar, fit, mobile);
    }
    let center = poster::layout(w, h, bar).center;
    let mut r = fit_rect_for(center.w as u32, center.h as u32, 0, Fit { integer: false, ..fit }, mobile);
    r.x += center.x;
    r.y += center.y;
    r
}

/// 遊戲畫面怎麼放進視窗：顯示比例、要不要整數倍、縮放濾鏡。由 `Session::fit` 依顯示模式給。
///
/// 視窗大小固定是 `window_size`（倍率 × 320×200 加工具列），換顯示模式不改視窗，只改畫面在視窗裡的矩形：
/// 比例不同的模式在視窗內置中，其餘填黑，不拉伸。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fit {
    /// 顯示比例（寬, 高）。
    pub aspect: (u32, u32),
    /// 1 倍的邊長單位：比例乘它就是 1 倍的大小。8:5 的單位是 40，1 倍是 320×200。
    pub unit: u32,
    /// 放得下就放整數倍，放不下 1 倍才縮小（以比例乘 `unit` 的矩形為 1 倍）。為假則填滿可用區域。
    pub integer: bool,
    /// 線性縮放。為假是最近鄰（原版圖的像素不糊）。
    pub smooth: bool,
}

impl Fit {
    /// VGA、EGA 的原版圖，320×200：8:5，整數倍，最近鄰。
    pub const SD: Fit = Fit { aspect: (8, 5), unit: 40, integer: true, smooth: false };
    /// HD 圖，1280×800：8:5，整數倍，線性。
    pub const HD: Fit = Fit { aspect: (8, 5), unit: 40, integer: true, smooth: true };
    /// CGA 640×200、MGA 640×300：原版在 4:3 螢幕上都是 4:3。填滿可用區域，線性縮放
    /// （水平不是整數倍，最近鄰會讓 1 位元的點陣花紋忽寬忽窄）。
    pub const MONO: Fit = Fit { aspect: (4, 3), unit: 40, integer: false, smooth: true };

    /// 戰場加大時（規格 005）的 VGA、EGA 畫面：`cols` 欄寬 × 200 列（384×200、480×200），1 倍是 `cols × 8` 乘 200。
    /// 欄數 40 就是 [`Fit::SD`]（`hd` 為真是 [`Fit::HD`]）。視窗比例不變，所以加寬的畫面上下留邊。
    pub const fn frame(cols: u16, hd: bool) -> Fit {
        if cols == 40 {
            return if hd { Fit::HD } else { Fit::SD };
        }
        Fit { aspect: (cols as u32, 25), unit: 8, integer: true, smooth: hd }
    }
}

/// 遊戲畫面在視窗中的位置：可用區域是工具列以下，依 `fit` 放（見 `Fit`）。置中，剩下的邊填黑。
/// `mobile`（手機版）一律填滿可用區域，不限整數倍：HD 是線性縮放，原版圖的像素不等寬也比留大黑邊好。
pub fn fit_rect_for(win_w: u32, win_h: u32, bar_h: u32, fit: Fit, mobile: bool) -> BarRect {
    let avail_h = win_h.saturating_sub(bar_h);
    let (aw, ah) = (win_w as f64, avail_h as f64);
    let (uw, uh) = (fit.aspect.0 as f64 * fit.unit as f64, fit.aspect.1 as f64 * fit.unit as f64);
    let k = (aw / uw).min(ah / uh);
    let k = if fit.integer && !mobile && k >= 1.0 { k.floor() } else { k };
    let w = ((uw * k).round() as i32).max(1);
    let h = ((uh * k).round() as i32).max(1);
    BarRect { x: (win_w as i32 - w) / 2, y: bar_h as i32 + (avail_h as i32 - h) / 2, w, h }
}

/// VGA、EGA 原版圖（8:5）在視窗中的位置：能放整數倍就放整數倍（最近鄰不糊），放不下 1 倍時才縮小。
pub fn fit_rect(win_w: u32, win_h: u32, bar_h: u32) -> BarRect {
    fit_rect_for(win_w, win_h, bar_h, Fit::SD, false)
}

/// 手機版：填滿可用區域，不限整數倍。
pub fn fill_rect(win_w: u32, win_h: u32, bar_h: u32) -> BarRect {
    fit_rect_for(win_w, win_h, bar_h, Fit::SD, true)
}

/// 震動位移換成視窗像素：只有 HD 畫面（寬 1280，戰場加大時 1536、1920）震動，`frame_w` 是畫面寬，
/// `shown_w` 是貼到視窗的寬。原版圖與單色畫面（最寬 640）不震動。
pub fn shake_px(shake: (i32, i32), frame_w: usize, shown_w: i32) -> (i32, i32) {
    if frame_w < 1280 || shake == (0, 0) {
        return (0, 0);
    }
    let k = shown_w as f32 / frame_w as f32;
    ((shake.0 as f32 * k).round() as i32, (shake.1 as f32 * k).round() as i32)
}

/// 觸控控制鈕的疊加層。
#[derive(Clone, Copy, Debug)]
pub struct Overlay<'o> {
    pub controls: &'o [Control],
    /// 玩家機在畫面上，右手兩顆鈕是開火與轉向。
    pub playable: bool,
}

/// 設定畫面或暫停標示：一張 RGBA 圖貼在視窗的 `(x, y)`。`key` 不變就不重傳貼圖，內容變了要換 `key`。
#[derive(Clone, Copy, Debug)]
pub struct UiLayer<'l> {
    pub px: &'l [u8],
    pub w: u32,
    pub h: u32,
    pub x: i32,
    pub y: i32,
    pub key: u64,
}

/// 存圖的結果，給日誌用。
#[derive(Debug, Clone, Copy)]
pub struct ShotInfo {
    pub w: usize,
    pub h: usize,
    /// RGB 任一分量非 0 的像素數。
    pub nonblack: usize,
}

impl ShotInfo {
    pub fn ratio(&self) -> f64 {
        self.nonblack as f64 / (self.w * self.h).max(1) as f64
    }
}

/// 非黑像素數（RGBA，RGB 任一分量非 0）。
pub fn count_nonblack(rgba: &[u8]) -> usize {
    rgba.chunks_exact(4).filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0).count()
}

pub struct Presenter<'a> {
    creator: &'a TextureCreator<WindowContext>,
    /// 遊戲畫面的貼圖與它的尺寸、是否線性縮放。畫面尺寸或濾鏡變了（換顯示模式、開關 HD）才重建。
    game: (Texture<'a>, u32, u32, bool),
    bar: Option<(Texture<'a>, u32, u32)>,
    bar_key: Option<(BarState, Toolbar, u32)>,
    overlay: Option<(Texture<'a>, u32, u32)>,
    overlay_key: Option<(bool, u32, u32)>,
    layer: Option<(Texture<'a>, u32, u32)>,
    layer_key: Option<(u64, u32, u32)>,
    /// 畫面震動的位移（HD 像素，規格 005「打擊回饋」）。只改遊戲畫面貼到視窗的位置，內容不動，不蓋到工具列。
    pub shake: (i32, i32),
    pub poster: PosterMode,
    poster_art: Arc<PosterArt>,
    poster_textures: [Option<[Texture<'a>; 2]>; 2],
    poster_cache: Option<PosterCache<'a>>,
    poster_cache_supported: Option<bool>,
}

struct PosterCache<'a> {
    mode: usize,
    sizes: [(u32, u32); 2],
    textures: [Texture<'a>; 2],
}

fn make_tex<'a>(c: &'a TextureCreator<WindowContext>, w: u32, h: u32, mode: ScaleMode) -> Result<Texture<'a>, String> {
    let mut t = c.create_texture_streaming(PixelFormatEnum::ABGR8888, w, h).map_err(|e| format!("建立 {w}x{h} 貼圖失敗：{e}"))?;
    t.set_scale_mode(mode);
    Ok(t)
}

fn scale_mode(smooth: bool) -> ScaleMode {
    if smooth { ScaleMode::Linear } else { ScaleMode::Nearest }
}

impl<'a> Presenter<'a> {
    pub fn new(creator: &'a TextureCreator<WindowContext>) -> Result<Presenter<'a>, String> {
        Ok(Presenter {
            creator,
            game: (make_tex(creator, GAME_W, GAME_H, ScaleMode::Nearest)?, GAME_W, GAME_H, false),
            bar: None,
            bar_key: None,
            overlay: None,
            overlay_key: None,
            layer: None,
            layer_key: None,
            shake: (0, 0),
            poster: PosterMode::Off,
            poster_art: Arc::new(PosterArt::default()),
            poster_textures: [None, None],
            poster_cache: None,
            poster_cache_supported: None,
        })
    }

    pub fn set_posters(&mut self, art: Arc<PosterArt>) {
        self.poster_art = art;
        self.poster_textures = [None, None];
        self.poster_cache = None;
        self.poster_cache_supported = None;
    }

    fn poster_active(&self) -> bool {
        self.poster.index().is_some() && self.poster.available(self.poster_art.available())
    }

    fn draw_posters(&mut self, canvas: &mut WindowCanvas, w: u32, h: u32, bar: u32) -> Result<(), String> {
        if !self.poster_active() {
            return Ok(());
        }
        let i = self.poster.index().unwrap();
        if self.poster_textures[i].is_none() {
            let pair = self.poster_art.pairs[i].as_ref().unwrap();
            let upload = |im: &png::Image| -> Result<Texture<'a>, String> {
                let mut t = make_tex(self.creator, im.w as u32, im.h as u32, ScaleMode::Linear)?;
                t.update(None, &im.rgba, im.w * 4).map_err(|e| e.to_string())?;
                Ok(t)
            };
            // 兩張均上傳成功才提交整組。
            self.poster_textures[i] = Some([upload(&pair[0])?, upload(&pair[1])?]);
        }
        canvas.set_draw_color(Color::RGB(8, 14, 25));
        canvas.fill_rect(Rect::new(0, bar as i32, w, h.saturating_sub(bar).max(1)))?;
        let l = poster::layout(w, h, bar);
        let rects = [l.left, l.right];
        let sizes = rects.map(|r| (r.w as u32, r.h as u32));
        let supported = *self.poster_cache_supported.get_or_insert_with(|| canvas.render_target_supported());
        let fresh = self.poster_cache.as_ref().is_some_and(|p| p.mode == i && p.sizes == sizes);
        if supported && !fresh {
            let build = (|| {
                let pair = self.poster_textures[i].as_ref().unwrap();
                let mut targets = Vec::new();
                for (src, (w, h)) in pair.iter().zip(sizes) {
                    let mut t = self.creator.create_texture_target(PixelFormatEnum::ABGR8888, w, h).map_err(|e| e.to_string())?;
                    t.set_scale_mode(ScaleMode::Nearest);
                    let mut result = Ok(());
                    canvas.with_texture_canvas(&mut t, |c| {
                        result = c.copy(src, None, None);
                    }).map_err(|e| e.to_string())?;
                    result?;
                    targets.push(t);
                }
                let textures = targets.try_into().map_err(|_| "poster cache pair".to_string())?;
                Ok::<_, String>(PosterCache { mode: i, sizes, textures })
            })();
            match build {
                Ok(c) => self.poster_cache = Some(c),
                Err(e) => {
                    eprintln!("[海報] 縮放快取不可用，沿用原畫法：{e}");
                    self.poster_cache = None;
                    self.poster_cache_supported = Some(false);
                }
            }
        }
        if let Some(cache) = self.poster_cache.as_ref().filter(|p| p.mode == i && p.sizes == sizes) {
            for (t, r) in cache.textures.iter().zip(rects) {
                canvas.copy(t, None, Rect::new(r.x, r.y, r.w as u32, r.h as u32))?;
            }
            return Ok(());
        }
        let pair = self.poster_textures[i].as_ref().unwrap();
        for (t, r) in pair.iter().zip([l.left, l.right]) {
            canvas.copy(t, None, Rect::new(r.x, r.y, r.w as u32, r.h as u32))?;
        }
        Ok(())
    }

    /// 繪圖裝置重置（Android 切回前景時 GL context 可能重建）：貼圖都要重建。內容每次繪製都會重新上傳。
    pub fn invalidate(&mut self) -> Result<(), String> {
        self.game = (make_tex(self.creator, GAME_W, GAME_H, ScaleMode::Nearest)?, GAME_W, GAME_H, false);
        self.bar = None;
        self.bar_key = None;
        self.overlay = None;
        self.overlay_key = None;
        self.layer = None;
        self.layer_key = None;
        self.poster_textures = [None, None];
        self.poster_cache = None;
        self.poster_cache_supported = None;
        Ok(())
    }

    /// 畫一幀：工具列在上，遊戲畫面在下；`shot` 有值時在 `present` 之前擷取視窗內容存成 PNG。
    /// `frame` 是 RGBA，`fw × fh` 隨顯示模式：320×200（VGA、EGA 原版圖）、1280×800（HD）、640×200（CGA）、640×300（MGA）。
    /// `fit` 決定畫面在視窗內的比例與縮放濾鏡（`Session::fit`），畫面尺寸或濾鏡變了就重建貼圖。
    /// `layer` 是設定畫面或暫停標示，蓋在遊戲畫面與觸控鈕之上、工具列之下。
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        canvas: &mut WindowCanvas,
        frame: &[u8],
        fw: usize,
        fh: usize,
        fit: Fit,
        toolbar: &Toolbar,
        bar_state: &BarState,
        overlay: Option<Overlay<'_>>,
        layer: Option<UiLayer<'_>>,
        shot: Option<&Path>,
    ) -> Result<Option<ShotInfo>, String> {
        let (ww, wh) = canvas.output_size()?;
        let bar_h = toolbar.height(ww);
        canvas.set_draw_color(Color::RGB(0, 0, 0));
        canvas.clear();
        if bar_h > 0 {
            let need_new = !matches!(&self.bar, Some((_, w, h)) if *w == ww && *h == bar_h);
            if need_new {
                self.bar = Some((make_tex(self.creator, ww, bar_h, ScaleMode::Nearest)?, ww, bar_h));
                self.bar_key = None;
            }
            let fresh = matches!(&self.bar_key, Some((s, t, w)) if s == bar_state && t == toolbar && *w == ww);
            if !fresh {
                let px = toolbar.render(ww, bar_state);
                if let Some((t, _, _)) = self.bar.as_mut() {
                    t.update(None, &px, ww as usize * 4).map_err(|e| format!("更新工具列貼圖失敗：{e}"))?;
                }
                self.bar_key = Some((bar_state.clone(), *toolbar, ww));
            }
        }
        if frame.len() != fw * fh * 4 {
            return Err(format!("畫面緩衝區長度 {} 與 {fw}x{fh} 不符", frame.len()));
        }
        let want = (fw as u32, fh as u32, fit.smooth);
        if (self.game.1, self.game.2, self.game.3) != want {
            self.game = (make_tex(self.creator, want.0, want.1, scale_mode(want.2))?, want.0, want.1, want.2);
        }
        self.draw_posters(canvas, ww, wh, bar_h)?;
        let on = self.poster_active();
        let tex = &mut self.game.0;
        tex.update(None, frame, fw * 4).map_err(|e| format!("更新畫面貼圖失敗：{e}"))?;
        let r = game_rect_with_poster(ww, wh, bar_h, fit, toolbar.mobile, on);
        // 震動：只有 HD 畫面，位移從 HD 像素換成視窗像素；夾在遊戲區內，不蓋到工具列。
        let (sx, sy) = shake_px(self.shake, fw, r.w);
        if (sx, sy) != (0, 0) {
            let clip = if on { poster::layout(ww, wh, bar_h).center } else { BarRect { x: 0, y: bar_h as i32, w: ww as i32, h: wh.saturating_sub(bar_h).max(1) as i32 } };
            canvas.set_clip_rect(Rect::new(clip.x, clip.y, clip.w as u32, clip.h as u32));
        }
        canvas.copy(tex, None, Rect::new(r.x + sx, r.y + sy, r.w as u32, r.h as u32))?;
        if (sx, sy) != (0, 0) {
            canvas.set_clip_rect(sdl2::render::ClippingRect::None);
        }
        if let Some(o) = overlay {
            let key = (o.playable, ww, wh);
            if self.overlay_key != Some(key) {
                if !matches!(&self.overlay, Some((_, w, h)) if *w == ww && *h == wh) {
                    let mut t = make_tex(self.creator, ww, wh, ScaleMode::Nearest)?;
                    t.set_blend_mode(BlendMode::Blend);
                    self.overlay = Some((t, ww, wh));
                }
                let px = touch::render_overlay(o.controls, o.playable, ww as usize, wh as usize);
                if let Some((t, _, _)) = self.overlay.as_mut() {
                    t.update(None, &px, ww as usize * 4).map_err(|e| format!("更新觸控層貼圖失敗：{e}"))?;
                }
                self.overlay_key = Some(key);
            }
            if let Some((t, _, _)) = self.overlay.as_ref() {
                canvas.copy(t, None, None)?;
            }
        }
        // 設定畫面或暫停標示：蓋在遊戲畫面與觸控鈕上面。
        if let Some(l) = layer {
            if l.px.len() != l.w as usize * l.h as usize * 4 {
                return Err(format!("介面層緩衝區長度 {} 與 {}x{} 不符", l.px.len(), l.w, l.h));
            }
            if l.w > 0 && l.h > 0 {
                let need_new = !matches!(&self.layer, Some((_, w, h)) if *w == l.w && *h == l.h);
                if need_new {
                    let mut t = make_tex(self.creator, l.w, l.h, ScaleMode::Nearest)?;
                    t.set_blend_mode(BlendMode::Blend);
                    self.layer = Some((t, l.w, l.h));
                    self.layer_key = None;
                }
                if self.layer_key != Some((l.key, l.w, l.h)) {
                    if let Some((t, _, _)) = self.layer.as_mut() {
                        t.update(None, l.px, l.w as usize * 4).map_err(|e| format!("更新介面層貼圖失敗：{e}"))?;
                    }
                    self.layer_key = Some((l.key, l.w, l.h));
                }
                if let Some((t, _, _)) = self.layer.as_ref() {
                    canvas.copy(t, None, Rect::new(l.x, l.y, l.w, l.h))?;
                }
            }
        }
        // 工具列最後畫，蓋在觸控層上面。
        if bar_h > 0 {
            if let Some((t, _, _)) = self.bar.as_ref() {
                canvas.copy(t, None, Rect::new(0, 0, ww, bar_h))?;
            }
        }
        let info = match shot {
            Some(path) => {
                let px = canvas.read_pixels(None, PixelFormatEnum::ABGR8888)?;
                let (w, h) = (ww as usize, wh as usize);
                if px.len() != w * h * 4 {
                    return Err(format!("read_pixels 回傳 {} 位元組，預期 {}", px.len(), w * h * 4));
                }
                let png = png::encode_rgba(w, h, &px);
                std::fs::write(path, &png).map_err(|e| format!("寫不了 {}：{e}", path.display()))?;
                Some(ShotInfo { w, h, nonblack: count_nonblack(&px) })
            }
            None => None,
        };
        canvas.present();
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "需要已定案的本機海報與 SDL"]
    fn poster_cache_is_pixel_identical_after_resize_switch_and_device_reset() {
        let root = std::env::var("RS2_HD").unwrap();
        let (art, notes) = PosterArt::load(Some(Path::new(&root)));
        assert!(notes.is_empty());
        let art = Arc::new(art);
        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        let win = video.window("poster-cache-check", 1536, 972).hidden().build().unwrap();
        let mut canvas = win.into_canvas().software().build().unwrap();
        assert!(canvas.render_target_supported());
        let creator = canvas.texture_creator();
        let mut old = Presenter::new(&creator).unwrap();
        old.set_posters(art.clone());
        old.poster_cache_supported = Some(false);
        let mut cached = Presenter::new(&creator).unwrap();
        cached.set_posters(art);
        for (mode, w, h) in [(PosterMode::Classic, 1536, 972), (PosterMode::Modern, 1536, 972), (PosterMode::Modern, 960, 670)] {
            canvas.window_mut().set_size(w, h).unwrap();
            old.poster = mode;
            cached.poster = mode;
            let bar = crate::toolbar::desktop_height(w);
            canvas.set_draw_color(Color::RGB(0, 0, 0));
            canvas.clear();
            old.draw_posters(&mut canvas, w, h, bar).unwrap();
            let direct = canvas.read_pixels(None, PixelFormatEnum::ABGR8888).unwrap();
            for reset in [false, true] {
                if reset { cached.invalidate().unwrap(); }
                canvas.set_draw_color(Color::RGB(0, 0, 0));
                canvas.clear();
                cached.draw_posters(&mut canvas, w, h, bar).unwrap();
                assert!(cached.poster_cache.is_some());
                let actual = canvas.read_pixels(None, PixelFormatEnum::ABGR8888).unwrap();
                assert_eq!(actual, direct, "cache must preserve approved artwork pixels");
            }
        }
    }

    #[test]
    fn poster_center_preserves_each_mode_and_matches_the_approved_frame() {
        let r = game_rect_with_poster(1920, 1080, 0, Fit::HD, false, true);
        assert_eq!((r.x, r.y, r.w, r.h), (360, 165, 1200, 750));
        for fit in [Fit::SD, Fit::HD, Fit::MONO, Fit::frame(48, true), Fit::frame(60, true)] {
            let r = game_rect_with_poster(1536, 904, 40, fit, false, true);
            let c = poster::layout(1536, 904, 40).center;
            assert!(r.x >= c.x && r.y >= c.y && r.x + r.w <= c.x + c.w && r.y + r.h <= c.y + c.h);
            assert!((r.w * fit.aspect.1 as i32 - r.h * fit.aspect.0 as i32).abs() <= fit.aspect.0 as i32);
            assert_eq!(game_rect_with_poster(960, 626, 26, fit, false, false), fit_rect_for(960, 626, 26, fit, false));
        }
        assert_eq!(window_size_with_poster(3, true, PosterMode::Off), window_size(3, true));
        let (w, h) = window_size_with_poster(3, true, PosterMode::Modern);
        assert_eq!((w, h - crate::toolbar::desktop_height(w)), (1536, 864));
    }

    #[test]
    fn window_size_adds_the_toolbar_on_top() {
        assert_eq!(window_size(2, false), (640, 400));
        assert_eq!(window_size(5, false), (1600, 1000));
        for scale in 2..=5u32 {
            let bar = crate::toolbar::desktop_height(320 * scale);
            assert!(bar > 40, "工具列加狀態列至少兩列字高");
            assert_eq!(window_size(scale, true), (320 * scale, 200 * scale + bar));
        }
        // 預設倍率 3：工具列（按鈕列）加狀態列。
        assert_eq!(window_size(3, true).1, 600 + crate::toolbar::desktop_height(960));
    }

    #[test]
    fn game_rect_sits_below_the_bar_without_overlap() {
        for scale in 1..=8 {
            for vis in [false, true] {
                let (w, h) = window_size(scale, vis);
                let bar = if vis { crate::toolbar::desktop_height(w) } else { 0 };
                let r = fit_rect(w, h, bar);
                assert_eq!((r.x, r.y, r.w, r.h), (0, bar as i32, (320 * scale) as i32, (200 * scale) as i32), "scale {scale} bar {bar}");
            }
        }
    }

    #[test]
    fn fullscreen_uses_integer_scale_and_centres() {
        let r = fit_rect(1920, 1080, 26);
        // 可用 1920×1054：min(6, 5.27) -> 5 倍 = 1600×1000，置中。
        assert_eq!((r.w, r.h), (1600, 1000));
        assert_eq!(r.x, 160);
        assert_eq!(r.y, 26 + (1054 - 1000) / 2);
        assert!(r.x + r.w <= 1920 && r.y + r.h <= 1080);
    }

    #[test]
    fn tiny_window_scales_down_but_stays_in_bounds() {
        let r = fit_rect(200, 150, 0);
        assert!(r.w <= 200 && r.h <= 150 && r.w > 0 && r.h > 0);
        assert_eq!(r.w * 200, r.h * 320, "維持 8:5");
        let r = fit_rect(100, 10, 20);
        assert!(r.w >= 1 && r.h >= 1);
    }

    #[test]
    fn phone_fill_uses_the_whole_height() {
        let r = fill_rect(2400, 1080, 0);
        assert_eq!((r.w, r.h, r.x, r.y), (1728, 1080, 336, 0));
        // 工具列展開時往下縮。
        let r = fill_rect(2400, 1080, 150);
        assert_eq!(r.h, 930);
        assert_eq!(r.y, 150);
        // 16:10 平板填滿。
        let r = fill_rect(1280, 800, 0);
        assert_eq!((r.x, r.y, r.w, r.h), (0, 0, 1280, 800));
    }

    #[test]
    fn sd_and_hd_fit_are_exactly_the_old_eight_by_five_layout() {
        for (w, h, bar) in [(960u32, 600 + 26, 26u32), (1920, 1080, 26), (200, 150, 0), (640, 400, 0), (2400, 1080, 0), (100, 10, 20)] {
            assert_eq!(fit_rect_for(w, h, bar, Fit::SD, false), fit_rect(w, h, bar));
            assert_eq!(fit_rect_for(w, h, bar, Fit::HD, false), fit_rect(w, h, bar), "HD 的矩形和原版圖相同，只有濾鏡不同");
            assert_eq!(fit_rect_for(w, h, bar, Fit::SD, true), fill_rect(w, h, bar));
        }
        assert!(!Fit::SD.smooth && Fit::HD.smooth && Fit::MONO.smooth);
    }

    #[test]
    fn a_wide_field_keeps_the_window_and_letterboxes_top_and_bottom() {
        assert_eq!(Fit::frame(40, false), Fit::SD);
        assert_eq!(Fit::frame(40, true), Fit::HD);
        assert!(!Fit::frame(48, false).smooth && Fit::frame(60, true).smooth, "原版圖最近鄰，HD 線性");
        // 預設視窗倍率 3：960×600 的遊戲區加 26 的工具列。
        let (w, h, bar) = (960u32, 600 + 26, 26u32);
        let r = fit_rect_for(w, h, bar, Fit::frame(60, false), false);
        assert_eq!((r.w, r.h), (960, 400), "150%：480×200 放 2 倍，寬剛好填滿");
        assert_eq!((r.x, r.y), (0, bar as i32 + 100), "上下各留 100");
        let r = fit_rect_for(w, h, bar, Fit::frame(48, false), false);
        assert_eq!((r.w, r.h), (768, 400), "120%：384×200 放 2 倍，左右上下留邊");
        assert_eq!((r.x, r.y), (96, bar as i32 + 100));
        // 全螢幕 1920×1080（工具列 26）：可用 1920×1054，150% 放 4 倍 = 1920×800。
        let r = fit_rect_for(1920, 1080, 26, Fit::frame(60, false), false);
        assert_eq!((r.w, r.h), (1920, 800));
        // 畫面不超出視窗，比例是欄數比 25。
        for cols in [48u16, 60] {
            for (ww, wh) in [(640u32, 426u32), (1280, 826), (2400, 1080), (300, 100)] {
                let r = fit_rect_for(ww, wh, 26, Fit::frame(cols, true), false);
                assert!(r.x >= 0 && r.y >= 26 && r.x + r.w <= ww as i32 && r.y + r.h <= wh as i32, "{cols} 欄 {ww}×{wh}：{r:?}");
            }
        }
    }

    #[test]
    fn only_hd_sized_frames_shake_including_the_wide_ones() {
        let shake = (10, -6);
        assert_eq!(shake_px(shake, 320, 960), (0, 0));
        assert_eq!(shake_px(shake, 480, 1440), (0, 0), "原版圖的寬戰場不震動");
        assert_eq!(shake_px(shake, 640, 960), (0, 0), "單色不震動");
        for (frame_w, shown) in [(1280usize, 1280i32), (1536, 1536), (1920, 960)] {
            let want = (shake.0 * shown / frame_w as i32, shake.1 * shown / frame_w as i32);
            let got = shake_px(shake, frame_w, shown);
            assert!((got.0 - want.0).abs() <= 1 && (got.1 - want.1).abs() <= 1, "{frame_w}：{got:?} 約 {want:?}");
            assert_ne!(got, (0, 0), "{frame_w} 寬的 HD 畫面要震");
        }
    }

    #[test]
    fn mono_fit_is_four_by_three_centred_and_never_stretched() {
        // 預設倍率 3 的視窗：可用區域 960×600，4:3 的矩形取滿高度，左右各留 80。
        let r = fit_rect_for(960, 626, 26, Fit::MONO, false);
        assert_eq!((r.x, r.y, r.w, r.h), (80, 26, 800, 600));
        // 倍率 2 到 5 與全螢幕，比例都是 4:3（取整誤差一個像素內），整個矩形在視窗內。
        for (w, h, bar) in [(640u32, 400 + 26, 26u32), (960, 626, 26), (1280, 800 + 40, 40), (1600, 1000 + 40, 40), (1920, 1080, 26), (2400, 1080, 0)] {
            let r = fit_rect_for(w, h, bar, Fit::MONO, false);
            assert!((r.w * 3 - r.h * 4).abs() <= 4, "{w}x{h}：{}x{} 不是 4:3", r.w, r.h);
            assert!(r.x >= 0 && r.y >= bar as i32 && r.x + r.w <= w as i32 && r.y + r.h <= h as i32, "{w}x{h}：{r:?} 超出視窗");
            // 取滿可用區域的一個方向，另一個方向置中。
            let avail_h = (h - bar) as i32;
            assert!(r.h == avail_h || r.w == w as i32, "{w}x{h}：沒有取滿 {r:?}");
            assert_eq!(r.x, (w as i32 - r.w) / 2);
            assert_eq!(r.y, bar as i32 + (avail_h - r.h) / 2);
            // 手機版也是填滿同一個矩形。
            assert_eq!(fit_rect_for(w, h, bar, Fit::MONO, true), r);
        }
        // 很小的視窗也不會變成零大小。
        let r = fit_rect_for(10, 5, 4, Fit::MONO, false);
        assert!(r.w >= 1 && r.h >= 1);
    }

    #[test]
    fn mono_frames_are_not_distorted_relative_to_the_vga_window() {
        // 同一個視窗（倍率 3）裡，VGA 的 8:5 矩形取滿寬度，單色的 4:3 矩形取滿高度，兩者高度相同。
        let (w, h) = window_size(3, true);
        let bar = crate::toolbar::desktop_height(w);
        let vga = fit_rect_for(w, h, bar, Fit::SD, false);
        let mono = fit_rect_for(w, h, bar, Fit::MONO, false);
        assert_eq!((vga.w, vga.h), (960, 600));
        assert_eq!((mono.w, mono.h), (800, 600));
        assert_eq!(vga.y, mono.y);
    }

    #[test]
    fn shake_scales_with_the_shown_size_and_only_applies_to_hd() {
        assert_eq!(shake_px((6, -3), 1280, 1280), (6, -3));
        assert_eq!(shake_px((6, -3), 1280, 640), (3, -2), "半尺寸顯示，位移跟著減半（四捨五入）");
        assert_eq!(shake_px((6, 6), 320, 960), (0, 0), "原版圖不震動");
        assert_eq!(shake_px((0, 0), 1280, 960), (0, 0));
    }

    #[test]
    fn nonblack_counts_rgb_only() {
        let px = [0, 0, 0, 255, 1, 0, 0, 255, 0, 0, 0, 0, 0, 0, 9, 255];
        assert_eq!(count_nonblack(&px), 2);
        let info = ShotInfo { w: 2, h: 2, nonblack: 2 };
        assert!((info.ratio() - 0.5).abs() < 1e-9);
    }
}
