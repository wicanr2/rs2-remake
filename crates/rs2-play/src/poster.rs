//! 定案的左右紀念海報，規格 007。只在前端使用，不參與模擬與遊戲存檔。

use crate::toolbar::Rect;
use crate::ui::text;
use rs2_formats::sha256_hex;
use rs2_gfx::png::{self, Image};
use std::path::Path;

pub const WIDTH: u32 = 512;
pub const HEIGHT: u32 = 288;
pub const PANEL_W: usize = 512;
pub const PANEL_H: usize = 1536;
const MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PosterMode {
    #[default]
    Off,
    Classic,
    Modern,
}

impl PosterMode {
    pub const ALL: [Self; 3] = [Self::Off, Self::Classic, Self::Modern];

    pub fn code(self) -> &'static str {
        match self { Self::Off => "off", Self::Classic => "classic", Self::Modern => "modern" }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.code() == code)
    }

    pub fn label(self) -> &'static str {
        match self { Self::Off => text::POSTER_OFF, Self::Classic => text::POSTER_CLASSIC, Self::Modern => text::POSTER_MODERN }
    }

    pub fn index(self) -> Option<usize> {
        match self { Self::Off => None, Self::Classic => Some(0), Self::Modern => Some(1) }
    }

    pub fn available(self, pairs: [bool; 2]) -> bool {
        self.index().is_none_or(|i| pairs[i])
    }

    pub fn next(self, pairs: [bool; 2], delta: i32) -> Self {
        let i = Self::ALL.iter().position(|m| *m == self).unwrap();
        let direction = if delta < 0 { -1 } else { 1 };
        (1..=3).map(|n| Self::ALL[(i as i32 + direction * n).rem_euclid(3) as usize])
            .find(|m| m.available(pairs)).unwrap_or(Self::Off)
    }
}

const FILES: [[(&str, &str); 2]; 2] = [
    [
        ("classic-left.png", "05239e1b53226d7d7f3d681840fac07daab332081887945cb326e3ccc896bf91"),
        ("classic-right.png", "e15a016a891f4f5c59af1a636e20564b12aada5d60744cc2709d4bf49cfa9699"),
    ],
    [
        ("modern-left.png", "363e3f8f6daab22833c18cfd534eb0b46686df952f23af6fad1f6758f7746482"),
        ("modern-right.png", "163add451f5779ea468681492d1dace5001a3da4a42c41c1b31781848052a2b4"),
    ],
];

fn decode_panel(data: &[u8], digest: &str) -> Result<Image, String> {
    if data.len() as u64 > MAX_BYTES || sha256_hex(data) != digest {
        return Err("海報大小或 SHA-256 不符".into());
    }
    let im = png::decode(data)?;
    if (im.w, im.h) != (PANEL_W, PANEL_H) || im.rgba.chunks_exact(4).any(|p| p[3] != 255) {
        return Err("海報必須為不透明的 512×1536 圖片".into());
    }
    Ok(im)
}

fn read_panel(root: &Path, name: &str, digest: &str) -> Result<Image, String> {
    let path = root.join("posters").join(name);
    let meta = std::fs::metadata(&path).map_err(|e| format!("{name}: {e}"))?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return Err(format!("{name}: 海報檔案大小無效"));
    }
    decode_panel(&std::fs::read(&path).map_err(|e| e.to_string())?, digest)
}

#[derive(Default, Debug)]
pub struct PosterArt {
    pub pairs: [Option<[Image; 2]>; 2],
}

impl PosterArt {
    pub fn load(root: Option<&Path>) -> (Self, Vec<String>) {
        let mut art = Self::default();
        let mut notes = Vec::new();
        let Some(root) = root else { return (art, notes) };
        if !root.join("posters").exists() {
            return (art, notes);
        }
        for (i, files) in FILES.iter().enumerate() {
            let pair = (|| Ok::<_, String>([
                read_panel(root, files[0].0, files[0].1)?,
                read_panel(root, files[1].0, files[1].1)?,
            ]))();
            match pair {
                Ok(p) => art.pairs[i] = Some(p),
                Err(e) => notes.push(format!("略過海報組 {}：{e}", i + 1)),
            }
        }
        (art, notes)
    }

    pub fn available(&self) -> [bool; 2] {
        self.pairs.each_ref().map(Option::is_some)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub left: Rect,
    pub right: Rect,
    pub center: Rect,
}

/// 512×288 定案畫布等比置中。相鄰邊由同一組錨點計算，避免取整產生縫隙。
pub fn layout(w: u32, h: u32, bar: u32) -> Layout {
    let ah = h.saturating_sub(bar) as f64;
    let k = (w as f64 / WIDTH as f64).min(ah / HEIGHT as f64);
    let fw = WIDTH as f64 * k;
    let fh = HEIGHT as f64 * k;
    let x = (w as f64 - fw) / 2.0;
    let y = bar as f64 + (ah - fh) / 2.0;
    let at_x = |v: f64| (x + v * k).round() as i32;
    let at_y = |v: f64| (y + v * k).round() as i32;
    let rect = |a, b, c, d| Rect { x: at_x(a), y: at_y(b), w: (at_x(c) - at_x(a)).max(1), h: (at_y(d) - at_y(b)).max(1) };
    Layout { left: rect(0.0, 0.0, 96.0, 288.0), right: rect(416.0, 0.0, 512.0, 288.0), center: rect(96.0, 44.0, 416.0, 244.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_preview_geometry_and_small_windows() {
        let l = layout(1920, 1080, 0);
        assert_eq!((l.left.x, l.left.y, l.left.w, l.left.h), (0, 0, 360, 1080));
        assert_eq!((l.right.x, l.right.y, l.right.w, l.right.h), (1560, 0, 360, 1080));
        assert_eq!((l.center.x, l.center.y, l.center.w, l.center.h), (360, 165, 1200, 750));
        for (w, h, b) in [(1536, 904, 40), (960, 626, 26), (640, 400, 0), (1280, 720, 0), (100, 100, 10)] {
            let l = layout(w, h, b);
            for r in [l.left, l.right, l.center] {
                assert!(r.x >= 0 && r.y >= b as i32 && r.x + r.w <= w as i32 && r.y + r.h <= h as i32, "{w}×{h}: {r:?}");
            }
            assert_eq!(l.left.x + l.left.w, l.center.x);
            assert_eq!(l.center.x + l.center.w, l.right.x);
            assert!((l.left.h - l.left.w * 3).abs() <= 2);
        }
    }

    #[test]
    fn missing_or_invalid_assets_do_not_make_half_pairs() {
        assert_eq!(PosterArt::load(None).0.available(), [false, false]);
        assert_eq!(PosterArt::load(Some(Path::new("/nonexistent-rs2-posters"))).0.available(), [false, false]);
        assert!(decode_panel(b"bad", FILES[0][0].1).is_err());
        let wrong = png::encode_rgba(1, 1, &[0, 0, 0, 255]);
        assert!(decode_panel(&wrong, &sha256_hex(&wrong)).unwrap_err().contains("512"));
    }

    #[test]
    #[ignore = "需要已定案的本機海報"]
    fn real_posters_load_and_a_missing_side_disables_only_its_pair() {
        let hd = std::env::var("RS2_HD").unwrap();
        let (art, notes) = PosterArt::load(Some(Path::new(&hd)));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(art.available(), [true, true]);
        let out = std::env::temp_dir().join(format!("rs2-poster-pair-{}", std::process::id()));
        std::fs::create_dir_all(out.join("posters")).unwrap();
        for (name, _) in FILES.iter().flatten().filter(|(n, _)| *n != "classic-right.png") {
            std::fs::copy(Path::new(&hd).join("posters").join(name), out.join("posters").join(name)).unwrap();
        }
        assert_eq!(PosterArt::load(Some(&out)).0.available(), [false, true]);
        std::fs::write(out.join("posters/modern-right.png"), b"corrupt").unwrap();
        assert_eq!(PosterArt::load(Some(&out)).0.available(), [false, false]);
        std::fs::remove_dir_all(out).unwrap();
    }
}
