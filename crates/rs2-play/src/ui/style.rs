//! 現代化介面的繪圖部件：反鋸齒圓角矩形、外框、漸層、陰影、滑動開關、分段按鈕與簡單圖示。
//! 全部畫在 RGBA 緩衝區（`Canvas`），不依賴 SDL 子系統，版面與像素都能單元測試。規格 005「HD 介面」。
//!
//! 作法：每個圖形寫成有號距離函式，像素中心離邊界 0.5 像素內線性過渡，所以邊緣是反鋸齒的。
//! 疊色用非預乘的 over，目的像素有 alpha 時也正確。

use super::{Canvas, Rect, font};

pub type Rgba = [u8; 4];

pub const fn rgb(hex: u32) -> Rgba {
    [((hex >> 16) & 0xFF) as u8, ((hex >> 8) & 0xFF) as u8, (hex & 0xFF) as u8, 255]
}

pub const fn with_alpha(c: Rgba, a: u8) -> Rgba {
    [c[0], c[1], c[2], a]
}

/// 規格 005「工具列與設定畫面的風格」的色彩。
pub mod color {
    use super::{Rgba, rgb};
    /// 深藍底。
    pub const BG: Rgba = rgb(0x0E1422);
    /// 表面。
    pub const SURFACE: Rgba = rgb(0x1B2740);
    /// 主色。
    pub const PRIMARY: Rgba = rgb(0x4F9CFF);
    /// 文字。
    pub const TEXT: Rgba = rgb(0xEDF3FF);
    /// 次要文字。
    pub const TEXT2: Rgba = rgb(0x8FA3C2);
    /// 警示。
    pub const WARN: Rgba = rgb(0xFFB454);
    /// 開啟。
    pub const ON: Rgba = rgb(0x5CF2C2);
    /// 低血量與錯誤。
    pub const DANGER: Rgba = rgb(0xFF5C6C);
    /// 面板底，比深藍底亮一階。
    pub const PANEL: Rgba = rgb(0x121B2E);
    /// 欄位底，比表面亮一階。
    pub const FIELD: Rgba = rgb(0x24334F);
    /// 停用文字。
    pub const TEXT3: Rgba = rgb(0x5F7090);
}

pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    let f = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round().clamp(0.0, 255.0) as u8;
    [f(0), f(1), f(2), f(3)]
}

/// 往白色靠近 `t`，alpha 不變。
pub fn lighten(c: Rgba, t: f32) -> Rgba {
    mix(c, [255, 255, 255, c[3]], t)
}

/// 往黑色靠近 `t`，alpha 不變。
pub fn darken(c: Rgba, t: f32) -> Rgba {
    mix(c, [0, 0, 0, c[3]], t)
}

/// alpha 乘上 `f`。
pub fn fade(c: Rgba, f: f32) -> Rgba {
    [c[0], c[1], c[2], (c[3] as f32 * f.clamp(0.0, 1.0)).round() as u8]
}

/// 把 `src` 以覆蓋率 `cov` 疊到 `dst`（4 位元組，非預乘）。
#[inline]
pub fn over(dst: &mut [u8], src: Rgba, cov: f32) {
    let sa = src[3] as f32 / 255.0 * cov;
    if sa <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return;
    }
    for i in 0..3 {
        let v = (src[i] as f32 * sa + dst[i] as f32 * da * (1.0 - sa)) / oa;
        dst[i] = v.round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// 覆蓋率：離邊界 0.5 像素內線性過渡。`d` 是像素中心到邊界的有號距離（內部為負）。
#[inline]
pub fn cov(d: f32) -> f32 {
    (0.5 - d).clamp(0.0, 1.0)
}

/// 圓角矩形的有號距離。
pub fn sd_rrect(px: f32, py: f32, r: Rect, rad: f32) -> f32 {
    let (hw, hh) = (r.w as f32 / 2.0, r.h as f32 / 2.0);
    let (cx, cy) = (r.x as f32 + hw, r.y as f32 + hh);
    let rad = rad.min(hw).min(hh).max(0.0);
    let qx = (px - cx).abs() - (hw - rad);
    let qy = (py - cy).abs() - (hh - rad);
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - rad
}

/// 對 `bounds` 內每個像素呼叫 `f(中心x, 中心y)`，得到 (顏色, 覆蓋率) 後疊上去。夾在 `Canvas` 的夾制框內。
pub fn raster(c: &mut Canvas, bounds: Rect, f: impl Fn(f32, f32) -> (Rgba, f32)) {
    let b = bounds.intersect(&c.clip);
    for y in b.y..b.bottom() {
        let row = (y * c.w) as usize;
        for x in b.x..b.right() {
            let (col, cv) = f(x as f32 + 0.5, y as f32 + 0.5);
            if cv > 0.0 {
                let o = (row + x as usize) * 4;
                over(&mut c.px[o..o + 4], col, cv);
            }
        }
    }
}

/// 單色填滿一個有號距離函式描述的形狀。
pub fn fill_sdf(c: &mut Canvas, bounds: Rect, color: Rgba, sd: impl Fn(f32, f32) -> f32) {
    raster(c, bounds, |x, y| (color, cov(sd(x, y))));
}

pub fn fill_rrect(c: &mut Canvas, r: Rect, rad: f32, color: Rgba) {
    if r.w <= 0 || r.h <= 0 {
        return;
    }
    raster(c, r.inset(-1), |x, y| (color, cov(sd_rrect(x, y, r, rad))));
}

/// 垂直漸層的圓角矩形。
pub fn fill_rrect_vgrad(c: &mut Canvas, r: Rect, rad: f32, top: Rgba, bottom: Rgba) {
    if r.w <= 0 || r.h <= 0 {
        return;
    }
    let hh = r.h.max(1) as f32;
    raster(c, r.inset(-1), |x, y| (mix(top, bottom, (y - r.y as f32) / hh), cov(sd_rrect(x, y, r, rad))));
}

/// 圓角矩形的外框，粗 `th`，畫在矩形邊界內側。
pub fn stroke_rrect(c: &mut Canvas, r: Rect, rad: f32, th: f32, color: Rgba) {
    if r.w <= 0 || r.h <= 0 {
        return;
    }
    raster(c, r.inset(-1), |x, y| {
        let d = (sd_rrect(x, y, r, rad) + th / 2.0).abs() - th / 2.0;
        (color, cov(d))
    });
}

/// 圓角矩形下方的陰影：`blur` 是擴散寬度，`dy` 是往下的位移。矩形內部不畫（會被矩形蓋掉）。
pub fn shadow(c: &mut Canvas, r: Rect, rad: f32, blur: i32, dy: i32, color: Rgba) {
    if r.w <= 0 || r.h <= 0 || blur <= 0 {
        return;
    }
    let sh = r.offset(0, dy);
    let bounds = Rect { x: sh.x - blur, y: sh.y - blur, w: sh.w + 2 * blur, h: sh.h + 2 * blur };
    let b = blur as f32;
    raster(c, bounds, |x, y| {
        if sd_rrect(x, y, r, rad) < -1.0 {
            return (color, 0.0);
        }
        let t = (sd_rrect(x, y, sh, rad) / b).clamp(0.0, 1.0);
        (color, (1.0 - t) * (1.0 - t))
    });
}

pub fn fill_circle(c: &mut Canvas, cx: f32, cy: f32, rad: f32, color: Rgba) {
    let bounds = Rect { x: (cx - rad - 1.0).floor() as i32, y: (cy - rad - 1.0).floor() as i32, w: (rad * 2.0 + 3.0).ceil() as i32, h: (rad * 2.0 + 3.0).ceil() as i32 };
    raster(c, bounds, |x, y| (color, cov(((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - rad)));
}

// ---------------------------------------------------------------------------
// 控制項的狀態與按鈕
// ---------------------------------------------------------------------------

/// 控制項的視覺狀態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vis {
    pub hover: bool,
    pub down: bool,
    /// 鍵盤焦點。
    pub focus: bool,
    pub enabled: bool,
}

impl Default for Vis {
    fn default() -> Vis {
        Vis { hover: false, down: false, focus: false, enabled: true }
    }
}

/// 按鈕的底色隨狀態變：滑鼠移過變亮，按下變深，停用往背景靠。
pub fn state_fill(base: Rgba, v: Vis) -> Rgba {
    if !v.enabled {
        return mix(base, color::BG, 0.45);
    }
    if v.down {
        return darken(base, 0.22);
    }
    if v.hover {
        return lighten(base, 0.12);
    }
    base
}

/// 圓角按鈕：細微漸層、1 像素半透明邊線、鍵盤焦點的主色外框。`accent` 是主色調的啟用狀態（例如暫停中）。
pub fn button(c: &mut Canvas, r: Rect, rad: f32, v: Vis, accent: bool) {
    let base = if accent { mix(color::SURFACE, color::PRIMARY, 0.38) } else { color::SURFACE };
    let fill = state_fill(base, v);
    fill_rrect_vgrad(c, r, rad, lighten(fill, 0.05), fill);
    let edge = if accent { with_alpha(color::PRIMARY, if v.enabled { 150 } else { 60 }) } else { [255, 255, 255, if v.hover && v.enabled { 52 } else { 26 }] };
    stroke_rrect(c, r, rad, 1.0, edge);
    if v.focus {
        stroke_rrect(c, r.inset(-1), rad + 1.0, 2.0, color::PRIMARY);
    }
}

/// 欄位（輸入框、表格內的值）：比卡片亮一階的平面。
pub fn field(c: &mut Canvas, r: Rect, rad: f32, v: Vis, capturing: bool) {
    let base = if capturing { mix(color::FIELD, color::WARN, 0.28) } else { color::FIELD };
    let fill = state_fill(base, v);
    fill_rrect(c, r, rad, fill);
    let edge = if capturing { with_alpha(color::WARN, 190) } else { [255, 255, 255, if v.hover && v.enabled { 46 } else { 22 }] };
    stroke_rrect(c, r, rad, 1.0, edge);
    if v.focus {
        stroke_rrect(c, r.inset(-1), rad + 1.0, 2.0, color::PRIMARY);
    }
}

// ---------------------------------------------------------------------------
// 滑動開關
// ---------------------------------------------------------------------------

/// 高 `h` 的開關寬度。
pub fn switch_width(h: i32) -> i32 {
    h * 9 / 5
}

/// 開關。`r` 就是開關本身的矩形（寬高比 9:5）。
pub fn draw_switch(c: &mut Canvas, r: Rect, on: bool, v: Vis) {
    let rad = r.h as f32 / 2.0;
    let track = if on { mix(color::ON, color::BG, 0.25) } else { mix(color::FIELD, color::TEXT3, 0.35) };
    let track = state_fill(track, v);
    fill_rrect(c, r, rad, track);
    stroke_rrect(c, r, rad, 1.0, [255, 255, 255, if on { 40 } else { 28 }]);
    let kr = rad - 3.0;
    let kx = if on { r.right() as f32 - rad } else { r.x as f32 + rad };
    let ky = r.y as f32 + rad;
    // 旋鈕的陰影。
    fill_circle(c, kx, ky + 1.0, kr + 1.0, [0, 0, 0, 70]);
    let knob = if v.enabled { if on { rgb(0xFFFFFF) } else { mix(color::TEXT2, color::TEXT, 0.35) } } else { color::TEXT3 };
    fill_circle(c, kx, ky, kr, knob);
    if v.focus {
        stroke_rrect(c, r.inset(-2), rad + 2.0, 2.0, color::PRIMARY);
    }
}

// ---------------------------------------------------------------------------
// 分段按鈕
// ---------------------------------------------------------------------------

/// 每一段的寬度：字寬加左右留白。
pub fn seg_widths(labels: &[&str], fh: i32, pad: i32) -> Vec<i32> {
    labels.iter().map(|l| font::builtin().text_width(l, fh) + 2 * pad).collect()
}

/// 分段按鈕每一段的矩形，裡外都留 `inset` 的邊。總寬以 `r` 為準，多的寬度平均分給每段。
pub fn seg_rects(r: Rect, widths: &[i32], inset: i32) -> Vec<Rect> {
    let n = widths.len().max(1) as i32;
    let inner = r.inset(inset);
    let need: i32 = widths.iter().sum();
    let extra = (inner.w - need).max(0);
    let mut x = inner.x;
    let mut out = Vec::with_capacity(widths.len());
    for (i, w) in widths.iter().enumerate() {
        let add = extra / n + if (i as i32) < extra % n { 1 } else { 0 };
        out.push(Rect { x, y: inner.y, w: w + add, h: inner.h });
        x += w + add;
    }
    out
}

/// 分段按鈕。`sel` 是目前選中的段。
pub fn draw_segmented(c: &mut Canvas, r: Rect, labels: &[String], widths: &[i32], sel: Option<usize>, fh: i32, v: Vis) {
    let rad = (r.h as f32 / 2.0).min(r.h as f32 * 0.42);
    fill_rrect(c, r, rad, state_fill(color::BG, Vis { hover: false, ..v }));
    stroke_rrect(c, r, rad, 1.0, [255, 255, 255, if v.hover && v.enabled { 44 } else { 24 }]);
    let segs = seg_rects(r, widths, 2);
    for (i, s) in segs.iter().enumerate() {
        let on = sel == Some(i);
        if on {
            let top = if v.enabled { color::PRIMARY } else { mix(color::PRIMARY, color::BG, 0.6) };
            fill_rrect_vgrad(c, *s, rad - 2.0, lighten(top, 0.12), top);
        }
        let ink = if !v.enabled {
            color::TEXT3
        } else if on {
            color::BG
        } else {
            color::TEXT2
        };
        if let Some(l) = labels.get(i) {
            c.text_in(*s, l, fh, 0, super::Align::Center, ink);
        }
    }
    if v.focus {
        stroke_rrect(c, r.inset(-1), rad + 1.0, 2.0, color::PRIMARY);
    }
}

// ---------------------------------------------------------------------------
// 圖示
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Play,
    Pause,
    Gamepad,
    Gear,
    Fullscreen,
    Image,
    Monitor,
    Info,
    Power,
    ChevronUp,
    Close,
}

fn sd_circle(u: f32, v: f32, cx: f32, cy: f32, r: f32) -> f32 {
    ((u - cx).powi(2) + (v - cy).powi(2)).sqrt() - r
}

fn sd_box(u: f32, v: f32, cx: f32, cy: f32, hw: f32, hh: f32, rad: f32) -> f32 {
    let qx = (u - cx).abs() - (hw - rad);
    let qy = (v - cy).abs() - (hh - rad);
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - rad
}

/// 線段（兩端圓頭），`th` 是半粗。
fn sd_seg(u: f32, v: f32, a: (f32, f32), b: (f32, f32), th: f32) -> f32 {
    let (px, py) = (u - a.0, v - a.1);
    let (bx, by) = (b.0 - a.0, b.1 - a.1);
    let h = ((px * bx + py * by) / (bx * bx + by * by).max(1e-9)).clamp(0.0, 1.0);
    ((px - bx * h).powi(2) + (py - by * h).powi(2)).sqrt() - th
}

/// 凸多邊形（順時針或逆時針皆可）的近似有號距離。
fn sd_poly(u: f32, v: f32, pts: &[(f32, f32)]) -> f32 {
    let n = pts.len();
    let mut area = 0.0;
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        area += a.0 * b.1 - b.0 * a.1;
    }
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };
    let mut d = f32::MIN;
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let (ex, ey) = (b.0 - a.0, b.1 - a.1);
        let len = (ex * ex + ey * ey).sqrt().max(1e-9);
        // 逆時針（面積為正）時內部在邊的左側，叉積為正；外側為正的距離取負號。
        let cross = ex * (v - a.1) - ey * (u - a.0);
        d = d.max(-sign * cross / len);
    }
    d
}

fn sd_icon(kind: Icon, u: f32, v: f32) -> f32 {
    match kind {
        Icon::Play => sd_poly(u, v, &[(0.30, 0.18), (0.30, 0.82), (0.80, 0.50)]) - 0.02,
        Icon::Pause => sd_box(u, v, 0.34, 0.5, 0.09, 0.30, 0.04).min(sd_box(u, v, 0.66, 0.5, 0.09, 0.30, 0.04)),
        Icon::Gamepad => {
            let body = sd_box(u, v, 0.5, 0.52, 0.44, 0.24, 0.20);
            let dpad = sd_box(u, v, 0.30, 0.52, 0.11, 0.032, 0.01).min(sd_box(u, v, 0.30, 0.52, 0.032, 0.11, 0.01));
            let b1 = sd_circle(u, v, 0.66, 0.46, 0.05);
            let b2 = sd_circle(u, v, 0.76, 0.56, 0.05);
            body.max(-dpad).max(-b1).max(-b2)
        }
        Icon::Gear => {
            let (x, y) = (u - 0.5, v - 0.5);
            let mut d = sd_circle(u, v, 0.5, 0.5, 0.27);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                let (s, c) = a.sin_cos();
                // 旋轉到齒的座標：x 軸沿齒的方向。
                let (tx, ty) = (x * c + y * s, -x * s + y * c);
                d = d.min(sd_box(tx + 0.5, ty + 0.5, 0.5 + 0.30, 0.5, 0.085, 0.065, 0.02));
            }
            d.max(-sd_circle(u, v, 0.5, 0.5, 0.11))
        }
        Icon::Fullscreen => {
            let t = 0.035;
            let (a, b) = (0.18, 0.40);
            let (c, d) = (0.60, 0.82);
            let mut s = f32::MAX;
            for (cx, cy, sx, sy) in [(a, a, b, b), (d, a, c, b), (a, d, b, c), (d, d, c, c)] {
                s = s.min(sd_seg(u, v, (cx, cy), (sx, cy), t)).min(sd_seg(u, v, (cx, cy), (cx, sy), t));
            }
            s
        }
        Icon::Image => {
            let frame = (sd_box(u, v, 0.5, 0.5, 0.36, 0.30, 0.07)).abs() - 0.035;
            let sun = sd_circle(u, v, 0.37, 0.40, 0.065);
            let hill = sd_seg(u, v, (0.20, 0.66), (0.42, 0.48), 0.03)
                .min(sd_seg(u, v, (0.42, 0.48), (0.56, 0.60), 0.03))
                .min(sd_seg(u, v, (0.50, 0.55), (0.64, 0.46), 0.03))
                .min(sd_seg(u, v, (0.64, 0.46), (0.80, 0.66), 0.03));
            frame.min(sun).min(hill)
        }
        Icon::Monitor => {
            let frame = (sd_box(u, v, 0.5, 0.44, 0.36, 0.25, 0.06)).abs() - 0.035;
            frame.min(sd_seg(u, v, (0.5, 0.70), (0.5, 0.80), 0.035)).min(sd_seg(u, v, (0.34, 0.82), (0.66, 0.82), 0.035))
        }
        Icon::Info => {
            let ring = sd_circle(u, v, 0.5, 0.5, 0.36).abs() - 0.035;
            ring.min(sd_circle(u, v, 0.5, 0.33, 0.05)).min(sd_seg(u, v, (0.5, 0.46), (0.5, 0.68), 0.04))
        }
        Icon::Power => {
            let ring = sd_circle(u, v, 0.5, 0.54, 0.30).abs() - 0.04;
            // 圓環上方留缺口。
            let gap = sd_box(u, v, 0.5, 0.2, 0.09, 0.2, 0.0);
            ring.max(-gap).min(sd_seg(u, v, (0.5, 0.18), (0.5, 0.50), 0.04))
        }
        Icon::ChevronUp => sd_seg(u, v, (0.24, 0.64), (0.5, 0.36), 0.06).min(sd_seg(u, v, (0.5, 0.36), (0.76, 0.64), 0.06)),
        Icon::Close => sd_seg(u, v, (0.27, 0.27), (0.73, 0.73), 0.055).min(sd_seg(u, v, (0.73, 0.27), (0.27, 0.73), 0.055)),
    }
}

/// 在正方形 `r` 內畫圖示。
pub fn draw_icon(c: &mut Canvas, kind: Icon, r: Rect, color: Rgba) {
    if r.w <= 0 || r.h <= 0 {
        return;
    }
    let (w, h) = (r.w as f32, r.h as f32);
    let size = w.min(h);
    // 單位座標 0..1 對應正方形；非正方形置中。
    let ox = r.x as f32 + (w - size) / 2.0;
    let oy = r.y as f32 + (h - size) / 2.0;
    raster(c, r.inset(-1), |x, y| {
        let (u, v) = ((x - ox) / size, (y - oy) / size);
        (color, cov(sd_icon(kind, u, v) * size))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(c: &Canvas, x: i32, y: i32) -> Rgba {
        let o = ((y * c.w + x) * 4) as usize;
        [c.px[o], c.px[o + 1], c.px[o + 2], c.px[o + 3]]
    }

    #[test]
    fn palette_matches_spec_005() {
        assert_eq!(color::BG, [0x0E, 0x14, 0x22, 255]);
        assert_eq!(color::SURFACE, [0x1B, 0x27, 0x40, 255]);
        assert_eq!(color::PRIMARY, [0x4F, 0x9C, 0xFF, 255]);
        assert_eq!(color::TEXT, [0xED, 0xF3, 0xFF, 255]);
        assert_eq!(color::TEXT2, [0x8F, 0xA3, 0xC2, 255]);
        assert_eq!(color::WARN, [0xFF, 0xB4, 0x54, 255]);
        assert_eq!(color::ON, [0x5C, 0xF2, 0xC2, 255]);
    }

    #[test]
    fn rounded_rect_has_antialiased_corners_and_solid_inside() {
        let mut c = Canvas::new(40, 30, [0, 0, 0, 255]);
        let r = Rect::new(5, 5, 30, 20);
        fill_rrect(&mut c, r, 8.0, [255, 255, 255, 255]);
        assert_eq!(px(&c, 20, 15), [255, 255, 255, 255], "內部全實心");
        assert_eq!(px(&c, 5, 5), [0, 0, 0, 255], "圓角把角落切掉");
        assert_eq!(px(&c, 4, 15), [0, 0, 0, 255], "矩形外不動");
        // 圓角的弧線上要有中間灰階，不是階梯。
        let mid = (0..40).flat_map(|x| (0..30).map(move |y| (x, y))).filter(|&(x, y)| {
            let v = px(&c, x, y)[0];
            v > 20 && v < 235
        });
        assert!(mid.count() >= 8, "弧線上要有反鋸齒的過渡像素");
        // 上下左右邊中點是直線，邊緣整數對齊，不該有過渡。
        assert_eq!(px(&c, 20, 5), [255, 255, 255, 255]);
        assert_eq!(px(&c, 20, 4), [0, 0, 0, 255]);
    }

    #[test]
    fn stroke_draws_only_the_border_ring() {
        let mut c = Canvas::new(40, 30, [0, 0, 0, 255]);
        let r = Rect::new(4, 4, 32, 22);
        stroke_rrect(&mut c, r, 6.0, 2.0, [255, 0, 0, 255]);
        assert_eq!(px(&c, 20, 4)[0], 255, "上緣在邊線上");
        assert_eq!(px(&c, 20, 5)[0], 255);
        assert_eq!(px(&c, 20, 15)[0], 0, "內部不畫");
        assert_eq!(px(&c, 20, 3)[0], 0, "矩形外不畫");
    }

    #[test]
    fn gradient_and_clip_are_respected() {
        let mut c = Canvas::new(20, 40, [0, 0, 0, 255]);
        fill_rrect_vgrad(&mut c, Rect::new(0, 0, 20, 40), 0.0, [255, 0, 0, 255], [0, 0, 255, 255]);
        assert!(px(&c, 10, 1)[0] > 240 && px(&c, 10, 38)[2] > 240);
        assert!(px(&c, 10, 20)[0] > 100 && px(&c, 10, 20)[2] > 100, "中段是兩色混合");
        c.set_clip(Some(Rect::new(0, 0, 20, 10)));
        fill_rrect(&mut c, Rect::new(0, 0, 20, 40), 0.0, [0, 255, 0, 255]);
        assert_eq!(px(&c, 10, 5), [0, 255, 0, 255]);
        assert_ne!(px(&c, 10, 20), [0, 255, 0, 255], "夾制框外不動");
    }

    #[test]
    fn over_handles_transparent_destinations() {
        let mut d = [0u8, 0, 0, 0];
        over(&mut d, [200, 100, 50, 255], 0.5);
        assert_eq!(d, [200, 100, 50, 128], "透明底：顏色不被自己的 alpha 變暗");
        let mut o = [0u8, 0, 0, 255];
        over(&mut o, [200, 100, 50, 255], 0.5);
        assert_eq!(o[3], 255);
        assert!((o[0] as i32 - 100).abs() <= 1);
    }

    #[test]
    fn shadow_fades_away_from_the_rect_and_skips_the_inside() {
        let mut c = Canvas::new(80, 60, [0, 0, 0, 0]);
        let r = Rect::new(20, 10, 40, 30);
        shadow(&mut c, r, 8.0, 12, 4, [0, 0, 0, 200]);
        assert_eq!(px(&c, 40, 25)[3], 0, "內部不畫");
        let near = px(&c, 40, 43)[3];
        let far = px(&c, 40, 52)[3];
        assert!(near > far && far > 0 && near > 60, "越遠越淡：{near} {far}");
        assert_eq!(px(&c, 2, 2)[3], 0, "擴散範圍之外沒有");
    }

    #[test]
    fn switch_knob_moves_and_colors_follow_state() {
        let (w, h) = (switch_width(20), 20);
        let r = Rect::new(2, 2, w, h);
        let mut off = Canvas::new(w + 4, h + 4, color::BG);
        let mut on = Canvas::new(w + 4, h + 4, color::BG);
        draw_switch(&mut off, r, false, Vis::default());
        draw_switch(&mut on, r, true, Vis::default());
        assert_ne!(off.px, on.px);
        // 旋鈕在左或右：看哪一側有亮像素。
        let bright = |c: &Canvas, x: i32| (0..c.h).any(|y| px(c, x, y)[0] > 140);
        assert!(bright(&off, 2 + h / 2) && !bright(&off, 2 + w - h / 2));
        assert!(bright(&on, 2 + w - h / 2) && !bright(&on, 2 + h / 2 - 3));
        // 開啟的軌道是綠青色。
        assert!(px(&on, 2 + h / 2 + 2, 2 + 2)[1] > px(&on, 2 + h / 2 + 2, 2 + 2)[0]);
    }

    #[test]
    fn hover_brightens_and_down_darkens_a_button() {
        let draw = |v: Vis| {
            let mut c = Canvas::new(60, 30, color::BG);
            button(&mut c, Rect::new(5, 5, 50, 20), 6.0, v, false);
            px(&c, 30, 15)
        };
        let base = draw(Vis::default());
        let hover = draw(Vis { hover: true, ..Vis::default() });
        let down = draw(Vis { down: true, ..Vis::default() });
        let off = draw(Vis { enabled: false, ..Vis::default() });
        let sum = |p: Rgba| p[0] as u32 + p[1] as u32 + p[2] as u32;
        assert!(sum(hover) > sum(base) && sum(base) > sum(down) && sum(base) > sum(off), "{base:?} {hover:?} {down:?} {off:?}");
        // 鍵盤焦點畫主色外框。
        let mut c = Canvas::new(60, 30, color::BG);
        button(&mut c, Rect::new(5, 5, 50, 20), 6.0, Vis { focus: true, ..Vis::default() }, false);
        let p = px(&c, 30, 4);
        assert!(p[2] > p[0] + 60, "外框是主色：{p:?}");
    }

    #[test]
    fn segmented_rects_tile_the_control_and_selected_segment_is_primary() {
        let f = font::builtin();
        let labels = ["100%".to_string(), "120%".to_string(), "150%".to_string()];
        let widths = seg_widths(&["100%", "120%", "150%"], 16, 8);
        assert!(widths.iter().all(|w| *w > f.text_width("100%", 16)));
        let r = Rect::new(0, 0, widths.iter().sum::<i32>() + 4 + 30, 26);
        let segs = seg_rects(r, &widths, 2);
        assert_eq!(segs.len(), 3);
        for w in segs.windows(2) {
            assert_eq!(w[0].right(), w[1].x, "相鄰兩段無縫");
        }
        assert_eq!(segs[0].x, 2);
        assert_eq!(segs[2].right(), r.w - 2, "多的寬度分給每段，整排填滿");
        let mut c = Canvas::new(r.w, r.h, color::BG);
        draw_segmented(&mut c, r, &labels, &widths, Some(1), 16, Vis::default());
        let on = px(&c, segs[1].x + 4, segs[1].y + segs[1].h / 2);
        let off = px(&c, segs[0].x + 4, segs[0].y + segs[0].h / 2);
        assert!(on[2] > 200 && on[2] > off[2] + 80, "選中段是主色：{on:?} {off:?}");
        // 沒選中任何段也不 panic。
        draw_segmented(&mut c, r, &labels, &widths, None, 16, Vis { enabled: false, ..Vis::default() });
    }

    #[test]
    fn every_icon_draws_inside_its_square_and_has_ink() {
        for k in [Icon::Play, Icon::Pause, Icon::Gamepad, Icon::Gear, Icon::Fullscreen, Icon::Image, Icon::Monitor, Icon::Info, Icon::Power, Icon::ChevronUp, Icon::Close] {
            for size in [16, 24, 40] {
                let mut c = Canvas::new(size + 20, size + 20, [0, 0, 0, 0]);
                let r = Rect::new(10, 10, size, size);
                draw_icon(&mut c, k, r, [255, 255, 255, 255]);
                let mut ink = 0;
                for y in 0..c.h {
                    for x in 0..c.w {
                        let a = px(&c, x, y)[3];
                        if a > 0 {
                            ink += 1;
                            assert!(x >= 9 && x <= 10 + size && y >= 9 && y <= 10 + size, "{k:?} {size}：({x},{y}) 超出方框");
                        }
                    }
                }
                assert!(ink as f32 > (size * size) as f32 * 0.07, "{k:?} {size}：墨跡太少 {ink}");
                assert!((ink as f32) < (size * size) as f32 * 0.8, "{k:?} {size}：墨跡太多 {ink}");
            }
        }
    }

    #[test]
    fn pause_icon_is_two_bars_and_play_is_a_triangle() {
        let mut c = Canvas::new(40, 40, [0, 0, 0, 0]);
        draw_icon(&mut c, Icon::Pause, Rect::new(0, 0, 40, 40), [255, 255, 255, 255]);
        assert!(px(&c, 13, 20)[3] > 200 && px(&c, 27, 20)[3] > 200, "兩條");
        assert_eq!(px(&c, 20, 20)[3], 0, "中間是空的");
        let mut t = Canvas::new(40, 40, [0, 0, 0, 0]);
        draw_icon(&mut t, Icon::Play, Rect::new(0, 0, 40, 40), [255, 255, 255, 255]);
        assert!(px(&t, 16, 20)[3] > 200, "三角形左側實心");
        assert_eq!(px(&t, 16, 3)[3], 0);
        // 三角形越往右越窄：同一列右側比左側早結束。
        let width_at = |y: i32| (0..40).filter(|&x| px(&t, x, y)[3] > 128).count();
        assert!(width_at(20) > width_at(10) && width_at(10) > width_at(7));
    }
}
