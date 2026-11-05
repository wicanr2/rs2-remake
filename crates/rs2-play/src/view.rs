//! 視窗版面與畫面輸出：視窗大小、遊戲畫面的位置、串流貼圖與 PNG 擷取。
//! 版面計算（`window_size`、`fit_rect`）不碰 SDL，可單元測試；`Presenter` 才用 SDL。

use crate::toolbar::{BarState, Rect as BarRect, Toolbar};
use rs2_gfx::png;
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::Rect;
use sdl2::render::{ScaleMode, Texture, TextureCreator, WindowCanvas};
use sdl2::video::WindowContext;
use std::path::Path;

/// 遊戲畫面的邏輯尺寸。
pub const GAME_W: u32 = 320;
pub const GAME_H: u32 = 200;

/// 倍率 `scale` 的視窗大小（工具列顯示時高度加上工具列）。
pub fn window_size(scale: u32, toolbar_visible: bool) -> (u32, u32) {
    let w = GAME_W * scale;
    let bar = if toolbar_visible { crate::toolbar::bar_height(w) } else { 0 };
    (w, GAME_H * scale + bar)
}

/// 遊戲畫面在視窗中的位置：可用區域是工具列以下，能放整數倍就放整數倍（最近鄰不糊），
/// 放不下 1 倍時才縮小。置中，剩下的邊填黑。
pub fn fit_rect(win_w: u32, win_h: u32, bar_h: u32) -> BarRect {
    let avail_h = win_h.saturating_sub(bar_h);
    let (aw, ah) = (win_w as f64, avail_h as f64);
    let k = (aw / GAME_W as f64).min(ah / GAME_H as f64);
    let k = if k >= 1.0 { k.floor() } else { k };
    let w = ((GAME_W as f64 * k).round() as i32).max(1);
    let h = ((GAME_H as f64 * k).round() as i32).max(1);
    BarRect { x: (win_w as i32 - w) / 2, y: bar_h as i32 + (avail_h as i32 - h) / 2, w, h }
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
    small: Texture<'a>,
    big: Option<Texture<'a>>,
    bar: Option<(Texture<'a>, u32, u32)>,
    bar_key: Option<(BarState, u32)>,
}

fn make_tex<'a>(c: &'a TextureCreator<WindowContext>, w: u32, h: u32, mode: ScaleMode) -> Result<Texture<'a>, String> {
    let mut t = c.create_texture_streaming(PixelFormatEnum::ABGR8888, w, h).map_err(|e| format!("建立 {w}x{h} 貼圖失敗：{e}"))?;
    t.set_scale_mode(mode);
    Ok(t)
}

impl<'a> Presenter<'a> {
    pub fn new(creator: &'a TextureCreator<WindowContext>) -> Result<Presenter<'a>, String> {
        Ok(Presenter { creator, small: make_tex(creator, GAME_W, GAME_H, ScaleMode::Nearest)?, big: None, bar: None, bar_key: None })
    }

    /// 畫一幀：工具列在上，遊戲畫面在下；`shot` 有值時在 `present` 之前擷取視窗內容存成 PNG。
    /// `frame` 是 RGBA，`fw × fh` 是 320×200（原版圖，最近鄰放大）或 1280×800（HD，線性縮放）。
    pub fn draw(
        &mut self,
        canvas: &mut WindowCanvas,
        frame: &[u8],
        fw: usize,
        fh: usize,
        toolbar: &Toolbar,
        bar_state: &BarState,
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
            if self.bar_key != Some((*bar_state, ww)) {
                let px = toolbar.render(ww, bar_state);
                if let Some((t, _, _)) = self.bar.as_mut() {
                    t.update(None, &px, ww as usize * 4).map_err(|e| format!("更新工具列貼圖失敗：{e}"))?;
                }
                self.bar_key = Some((*bar_state, ww));
            }
            if let Some((t, _, _)) = self.bar.as_ref() {
                canvas.copy(t, None, Rect::new(0, 0, ww, bar_h))?;
            }
        }
        if frame.len() != fw * fh * 4 {
            return Err(format!("畫面緩衝區長度 {} 與 {fw}x{fh} 不符", frame.len()));
        }
        let tex = if fw == GAME_W as usize && fh == GAME_H as usize {
            &mut self.small
        } else {
            // 建立一次，尺寸固定 1280×800（LayerSink 的 HD 輸出）。
            if self.big.is_none() {
                self.big = Some(make_tex(self.creator, fw as u32, fh as u32, ScaleMode::Linear)?);
            }
            self.big.as_mut().ok_or("HD 貼圖不存在")?
        };
        tex.update(None, frame, fw * 4).map_err(|e| format!("更新畫面貼圖失敗：{e}"))?;
        let r = fit_rect(ww, wh, bar_h);
        canvas.copy(tex, None, Rect::new(r.x, r.y, r.w as u32, r.h as u32))?;
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
    fn window_size_adds_the_toolbar_on_top() {
        assert_eq!(window_size(2, false), (640, 400));
        assert_eq!(window_size(2, true), (640, 413));
        assert_eq!(window_size(3, true), (960, 626));
        assert_eq!(window_size(4, true), (1280, 826));
        assert_eq!(window_size(5, false), (1600, 1000));
    }

    #[test]
    fn game_rect_sits_below_the_bar_without_overlap() {
        for scale in 1..=8 {
            for vis in [false, true] {
                let (w, h) = window_size(scale, vis);
                let bar = if vis { crate::toolbar::bar_height(w) } else { 0 };
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
    fn nonblack_counts_rgb_only() {
        let px = [0, 0, 0, 255, 1, 0, 0, 255, 0, 0, 0, 0, 0, 0, 9, 255];
        assert_eq!(count_nonblack(&px), 2);
        let info = ShotInfo { w: 2, h: 2, nonblack: 2 };
        assert!((info.ratio() - 0.5).abs() < 1e-9);
    }
}
