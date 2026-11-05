//! 載入畫面（規格 005「載入畫面」）：中文字樣加圓角進度條，第一次啟動解資料與 HD 預載都用它。
//! 畫成 1280×800 的 RGBA，和 HD 遊戲畫面同尺寸，所以走同一張貼圖。不依賴 SDL 子系統。

use super::style::{self, color};
use super::{Align, Canvas, Rect, text};

pub const W: i32 = 1280;
pub const H: i32 = 800;
/// 進度條的寬高。
pub const BAR_W: i32 = 520;
pub const BAR_H: i32 = 12;

/// 進度條的狀態。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    /// 已知進度，0.0 到 1.0。
    Done(f32),
    /// 不知道還要多久（HD 預載）：一段光帶在條裡來回，`phase` 是 0.0 到 1.0 的循環相位。
    Sweep(f32),
}

/// 進度條的矩形，置中偏下。
pub fn bar_rect() -> Rect {
    Rect::new((W - BAR_W) / 2, H / 2 + 56, BAR_W, BAR_H)
}

/// 進度條裡填色的範圍（條的座標）。
pub fn fill_range(p: Progress) -> (i32, i32) {
    let inner = BAR_W;
    match p {
        Progress::Done(f) => (0, (inner as f32 * f.clamp(0.0, 1.0)).round() as i32),
        Progress::Sweep(ph) => {
            let seg = inner * 28 / 100;
            // 三角波：0 到 1 再回到 0。
            let t = 1.0 - (ph.rem_euclid(1.0) * 2.0 - 1.0).abs();
            let x = ((inner - seg) as f32 * t).round() as i32;
            (x, x + seg)
        }
    }
}

/// 畫一張載入畫面。`title` 是大字，`note` 是下面的說明。
pub fn frame(title: &str, note: &str, p: Progress) -> Vec<u8> {
    let mut c = Canvas::new(W, H, color::BG);
    // 背景：上亮下暗的垂直漸層。
    for y in 0..H {
        let t = y as f32 / (H - 1) as f32;
        let col = style::mix(style::rgb(0x16203A), color::BG, t);
        c.rect(Rect::new(0, y, W, 1), col);
    }
    let cy = H / 2;
    c.text_in(Rect::new(0, cy - 90, W, 56), title, 48, 0, Align::Center, color::TEXT);
    c.text_in(Rect::new(0, cy - 24, W, 36), note, 24, 0, Align::Center, color::TEXT2);
    let bar = bar_rect();
    style::fill_rrect(&mut c, bar, BAR_H as f32 / 2.0, [255, 255, 255, 22]);
    style::stroke_rrect(&mut c, bar, BAR_H as f32 / 2.0, 1.0, [255, 255, 255, 40]);
    let (a, b) = fill_range(p);
    if b > a {
        let r = Rect::new(bar.x + a, bar.y, b - a, bar.h);
        style::fill_rrect_vgrad(&mut c, r, BAR_H as f32 / 2.0, style::lighten(color::PRIMARY, 0.2), style::darken(color::PRIMARY, 0.1));
    }
    if let Progress::Done(f) = p {
        let pct = format!("{}%", (f.clamp(0.0, 1.0) * 100.0).round() as i32);
        c.text_in(Rect::new(0, bar.bottom() + 14, W, 28), &pct, 20, 0, Align::Center, color::TEXT2);
    }
    c.px
}

/// 第一次啟動解資料時的畫面。
pub fn install_frame(f: f32) -> Vec<u8> {
    frame(text::LOAD_INSTALL_TITLE, text::LOAD_INSTALL, Progress::Done(f))
}

/// HD 預載時的畫面。
pub fn hd_frame(phase: f32) -> Vec<u8> {
    frame(text::LOAD_TITLE, text::LOAD_HD, Progress::Sweep(phase))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(b: &[u8], x: i32, y: i32) -> [u8; 4] {
        let o = ((y * W + x) * 4) as usize;
        [b[o], b[o + 1], b[o + 2], b[o + 3]]
    }

    #[test]
    fn frame_is_hd_sized_opaque_and_has_chinese_text() {
        let f = hd_frame(0.0);
        assert_eq!(f.len(), (W * H * 4) as usize);
        assert!(f.chunks_exact(4).all(|p| p[3] == 255));
        // 大字落在畫面中央偏上，有亮像素。
        let bright = (H / 2 - 90..H / 2 - 34).flat_map(|y| (400..880).map(move |x| (x, y))).filter(|&(x, y)| px(&f, x, y)[0] > 180).count();
        assert!(bright > 300, "標題有字：{bright}");
        assert!(super::super::font::builtin().missing(&format!("{}{}{}{}", text::LOAD_TITLE, text::LOAD_HD, text::LOAD_INSTALL_TITLE, text::LOAD_INSTALL)).is_empty());
    }

    #[test]
    fn progress_bar_fills_in_proportion_and_has_rounded_ends() {
        let bar = bar_rect();
        let mid = |f: f32| {
            let b = install_frame(f);
            (0..BAR_W).filter(|&dx| px(&b, bar.x + dx, bar.y + BAR_H / 2)[2] > 150).count() as i32
        };
        let (a, b, c) = (mid(0.0), mid(0.5), mid(1.0));
        assert!(a < 8 && (b - BAR_W / 2).abs() <= 6 && c >= BAR_W - 6, "{a} {b} {c}");
        // 圓角：滿格時條的左上角那個像素不是填色。
        let full = install_frame(1.0);
        let corner = px(&full, bar.x, bar.y);
        let inside = px(&full, bar.x + BAR_H, bar.y + BAR_H / 2);
        assert!(corner[2] < inside[2] - 60, "{corner:?} {inside:?}");
        assert_eq!(fill_range(Progress::Done(2.0)), (0, BAR_W));
        assert_eq!(fill_range(Progress::Done(-1.0)), (0, 0));
    }

    #[test]
    fn sweep_moves_back_and_forth_inside_the_bar() {
        let mut seen = Vec::new();
        for i in 0..=20 {
            let (a, b) = fill_range(Progress::Sweep(i as f32 / 20.0));
            assert!(a >= 0 && b <= BAR_W && b > a, "{a}..{b}");
            seen.push(a);
        }
        assert_eq!(seen[0], 0);
        assert!(seen[10] > BAR_W / 2 - 10 - 140 && seen[10] == *seen.iter().max().unwrap(), "中間相位在最右邊");
        assert_eq!(seen[20], 0, "一輪回到起點");
        assert_ne!(hd_frame(0.0), hd_frame(0.5));
    }

    /// 存圖（`RS2_SHOTS` 指定目錄才存）：兩種載入畫面。
    #[test]
    fn ui_shots_loading_screens() {
        let Some(dir) = std::env::var("RS2_SHOTS").ok().map(std::path::PathBuf::from) else { return };
        std::fs::create_dir_all(&dir).unwrap();
        let save = |name: &str, px: Vec<u8>| std::fs::write(dir.join(name), rs2_gfx::png::encode_rgba(W as usize, H as usize, &px)).unwrap();
        save("loading_install_40pct.png", install_frame(0.4));
        save("loading_hd_preload.png", hd_frame(0.35));
    }
}
