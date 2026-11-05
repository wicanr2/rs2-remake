//! HD 引擎：同一批繪製命令合成 RGBA，座標乘 4。
//!
//! 記憶體是四塊畫布：顯示頁 0、1（1280×800）與暫存區 A、B（2560 寬、310 邏輯列）。
//! 顯示頁永遠不透明（alpha 255）；暫存區保留 HD 圖的 alpha，因為原版的「色號 0」在 OR 時等於不動，
//! 換成 RGBA 就是透明，覆寫時才變成黑色。
//!
//! 語意（規格 001「繪製模型」與 `docs/re/12-stages-presentation.md` 第 2 節）：
//! - AND 遮罩：目的乘上 HD 遮罩（255 保留，0 清掉）。
//! - OR：HD 圖以 alpha 疊上。覆寫：先當成不透明黑底，再以 alpha 疊上（原版覆寫連黑色都蓋掉底下）。
//! - 閃白：依遮罩反相的比例，朝色號 15 的顏色填。
//! - 暫存區複製、位移、OR、AND、填色都乘 4。調色盤顏色在畫的當下取，已畫上的像素不會隨後來的調色盤改變。
//!
//! HD 靠人判讀，不要求和 `VgaSink` 逐像素相同：目的欄超出一列時裁掉，不像原版接到下一列。

use super::hd_assets::{HdAssets, HdImg, HdMask, Lib, SCALE};
use super::pal::Pal;
use super::recolor::Colors;
use super::Off;
use crate::assets::Assets;
use crate::font::DIGITS;
use crate::geom::Geom;
use rs2_game::{BlitCmd, Op};

pub const S: i32 = SCALE as i32;
/// 原版版面（40 欄）的顯示頁寬、高（HD 像素）。畫布自己記寬（[`Canvas::w`]），戰場加大時（規格 005）頁更寬，高不變。
pub const PW: i32 = 320 * S;
pub const PH: i32 = 200 * S;
/// 原版版面的暫存區寬（640 邏輯像素）與列數（310 列，涵蓋 `A19Ah` 表的 300 多列）。戰場加大時暫存區寬 960 邏輯像素。
#[cfg(test)]
pub const SW: i32 = 640 * S;
pub const SH: i32 = 310 * S;
/// 遊戲區只有 190 列（`y > 189` 的列不畫）。
const GAME_ROWS: i32 = 190 * S;

#[derive(Clone)]
pub struct Canvas {
    pub px: Vec<u8>,
    /// 畫布寬（HD 像素）。空畫布是 0。
    pub w: i32,
}

impl Canvas {
    pub fn new(w: usize, h: usize, fill: [u8; 4]) -> Canvas {
        let mut px = vec![0u8; w * h * 4];
        if fill != [0; 4] {
            for p in px.chunks_exact_mut(4) {
                p.copy_from_slice(&fill);
            }
        }
        Canvas { px, w: w as i32 }
    }

    /// 已有像素資料的畫布，`w` 是寬（HD 像素）。
    pub fn from_px(px: Vec<u8>, w: i32) -> Canvas {
        Canvas { px, w }
    }

    pub fn empty() -> Canvas {
        Canvas { px: Vec::new(), w: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.px.is_empty()
    }
}

/// 暫存區內容的色彩空間。背景通常在載入用的調色盤下寫進暫存區，之後才換成關卡調色盤，
/// 所以寫入時保留圖原本的調色盤，第一次被複製到顯示頁才換色，避免來回轉換失真。
#[derive(Clone)]
pub struct ScrTag {
    pub colors: Colors,
    pub pal: Pal,
}

#[derive(Clone)]
pub struct HdMem {
    pub pages: [Canvas; 2],
    pub scratch: [Canvas; 2],
    pub scr_tag: [Option<ScrTag>; 2],
    /// 序列器 map mask。HD 畫布沒有平面，只有 `0Fh`（全平面）的填色與清除有意義，
    /// 其他值下的 `VramFill`、`ClearScratchA`、`ClearScratchB` 略過（`VgaSink` 與索引色引擎才有逐平面的效果）。
    pub map_mask: u8,
    /// 版面：顯示頁與暫存區的欄數（`Geom`），決定畫布寬與位址換算。
    pub g: Geom,
}

impl HdMem {
    pub fn new(g: Geom) -> HdMem {
        let pw = g.px_width() * SCALE;
        let sw = g.scols() as usize * 8 * SCALE;
        let page = || Canvas::new(pw, PH as usize, [0, 0, 0, 255]);
        let scr = || Canvas::new(sw, SH as usize, [0, 0, 0, 0]);
        HdMem { pages: [page(), page()], scratch: [scr(), scr()], scr_tag: [None, None], map_mask: 0x0F, g }
    }

    pub fn from_parts(g: Geom, pages: [Canvas; 2], scratch: [Canvas; 2]) -> HdMem {
        HdMem { pages, scratch, scr_tag: [None, None], map_mask: 0x0F, g }
    }
}

impl Default for HdMem {
    fn default() -> Self {
        HdMem::new(Geom::ORIGINAL)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Or,
    Mov,
}

#[inline]
fn over(d: &mut [u8], s: &[u8]) {
    let a = s[3] as u32;
    if a == 0 {
        return;
    }
    if a == 255 {
        d.copy_from_slice(s);
        return;
    }
    let ia = 255 - a;
    for i in 0..3 {
        d[i] = ((s[i] as u32 * a + d[i] as u32 * ia + 127) / 255) as u8;
    }
    d[3] = 255;
}

#[inline]
fn over_black(d: &mut [u8], s: &[u8]) {
    let a = s[3] as u32;
    if a == 255 {
        d.copy_from_slice(s);
        return;
    }
    if a == 0 {
        d.copy_from_slice(&[0, 0, 0, 255]);
        return;
    }
    for i in 0..3 {
        d[i] = ((s[i] as u32 * a + 127) / 255) as u8;
    }
    d[3] = 255;
}

#[inline]
fn lum(s: &[u8]) -> u32 {
    let l = (s[0] as u32 * 77 + s[1] as u32 * 150 + s[2] as u32 * 29) >> 8;
    (l * s[3] as u32 + 127) / 255
}

#[inline]
fn scale_rgb(d: &mut [u8], f: u32) {
    for v in d.iter_mut().take(3) {
        *v = ((*v as u32 * f + 127) / 255) as u8;
    }
}

#[inline]
fn pg(c: &BlitCmd) -> usize {
    ((c.page >> 15) & 1) as usize
}

#[inline]
fn sx(v: u16) -> i32 {
    v as i16 as i32
}

/// 圖畫到顯示頁。裁切：欄 `[0, 頁寬)`（原版 1280）、列 `[0, 760)`，負座標的列欄跳過。
fn blit_img(page: &mut Canvas, img: &HdImg, x0: i32, y0: i32, mode: Mode) {
    if page.is_empty() {
        return;
    }
    let pw = page.w;
    let xs = (-x0).max(0);
    let xe = (img.w as i32).min(pw - x0);
    if xs >= xe {
        return;
    }
    for iy in 0..img.h as i32 {
        let y = y0 + iy;
        if y < 0 {
            continue;
        }
        if y >= GAME_ROWS {
            break;
        }
        let so = iy as usize * img.w;
        let srow = &img.rgba[(so + xs as usize) * 4..(so + xe as usize) * 4];
        let drow = &mut page.px[((y * pw + x0 + xs) as usize) * 4..((y * pw + x0 + xe) as usize) * 4];
        for (d, s) in drow.chunks_exact_mut(4).zip(srow.chunks_exact(4)) {
            match mode {
                Mode::Or => over(d, s),
                Mode::Mov => over_black(d, s),
            }
        }
    }
}

fn mask_apply(page: &mut Canvas, m: &HdMask, x0: i32, y0: i32, white: Option<[u8; 3]>) {
    if page.is_empty() {
        return;
    }
    let pw = page.w;
    let xs = (-x0).max(0);
    let xe = (m.w as i32).min(pw - x0);
    if xs >= xe {
        return;
    }
    for iy in 0..m.h as i32 {
        let y = y0 + iy;
        if y < 0 {
            continue;
        }
        if y >= GAME_ROWS {
            break;
        }
        let so = iy as usize * m.w;
        let srow = &m.v[so + xs as usize..so + xe as usize];
        let drow = &mut page.px[((y * pw + x0 + xs) as usize) * 4..((y * pw + x0 + xe) as usize) * 4];
        for (d, &v) in drow.chunks_exact_mut(4).zip(srow) {
            match white {
                None => {
                    if v != 255 {
                        scale_rgb(d, v as u32);
                    }
                }
                Some(w) => {
                    let t = 255 - v as u32;
                    if t != 0 {
                        for i in 0..3 {
                            d[i] = ((d[i] as u32 * (255 - t) + w[i] as u32 * t + 127) / 255) as u8;
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Reg {
    /// 落在哪一頁。複製的來源指到顯示頁時不處理，編號目前只有測試讀。
    #[allow(dead_code)]
    Page(usize),
    Scratch(usize),
}

/// 位址空間落在哪一塊：頁 0（`cols` 位元組一列）、暫存區 A（`scols` 位元組一列）、頁 1、暫存區 B。
/// 原版是 64 KB 的位址空間（頁 0 在 0，暫存區 A 在 1F40h，頁 1 在 8000h，暫存區 B 在 9F40h）。
fn addr_region(g: Geom, a: u32) -> (Reg, i32, i32) {
    let a = a as i32;
    let (cols, scols) = (i32::from(g.cols()), i32::from(g.scols()));
    let (page, base, set1) = (g.page_bytes() as i32, g.scratch_base() as i32, g.set1() as i32);
    if a < page {
        (Reg::Page(0), a / cols, a % cols)
    } else if a < set1 {
        (Reg::Scratch(0), (a - base) / scols, (a - base) % scols)
    } else if a < set1 + page {
        (Reg::Page(1), (a - set1) / cols, (a - set1) % cols)
    } else {
        (Reg::Scratch(1), (a - set1 - base) / scols, (a - set1 - base) % scols)
    }
}

/// `sub_19ADC`：HD 圖原樣（含 alpha）寫進暫存區。奇數寬度丟掉最後一個位元組，不裁切（只受畫布邊界限制）。
fn scratch_put(sc: &mut Canvas, img: &HdImg, stride: u16, c: &BlitCmd) {
    if sc.is_empty() {
        return;
    }
    let sw = sc.w;
    let w_eff = ((stride & !1) as usize * 8 * SCALE).min(img.w);
    let x0 = sx(c.x) * 8 * S;
    let y0 = sx(c.y) * S;
    let xs = (-x0).max(0) as usize;
    let xe = (w_eff as i32).min(sw - x0).max(0) as usize;
    if xs >= xe {
        return;
    }
    for iy in 0..img.h as i32 {
        let y = y0 + iy;
        if !(0..SH).contains(&y) {
            continue;
        }
        let so = iy as usize * img.w;
        let (d0, d1) = ((y * sw + x0 + xs as i32) as usize, (y * sw + x0 + xe as i32) as usize);
        sc.px[d0 * 4..d1 * 4].copy_from_slice(&img.rgba[(so + xs) * 4..(so + xe) * 4]);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CopyMode {
    /// 覆寫：透明處變黑。
    Over,
    Or,
    And,
}

/// 暫存區列複製的形狀。
#[derive(Clone, Copy)]
struct Shape {
    /// `sub_19D08`：水平位移，位移加欄落在 `[0, 80)` 才寫。
    shifted: bool,
    /// 垂直鏡射（`sub_19C4F`、`19F3B`、`1A1E6`）：來源第 `i` 列畫在目的第 `y - i` 列。
    mirror: bool,
    /// 每列最多寫幾個邏輯位元組（整列 40，`sub_19E66`、`19FF9`、`1A144` 只有 2）。
    bytes: i32,
    /// 目的列（HD 列，不含）上限。整列版是遊戲區 190 列，`sub_19C4F` 沒有上限（顯示頁 200 列）。
    ylim: i32,
    /// 鏡射版的 `bx` 遞增（`sub_19F3B`、`1A1E6`）：來源列 `i` 只畫到 `y + i <= 189`。
    count_up: bool,
}

impl Shape {
    /// 整列版：每列寫顯示頁的全部 `cols` 個位元組。
    const fn full(cols: u16) -> Shape {
        Shape { shifted: false, mirror: false, bytes: cols as i32, ylim: GAME_ROWS, count_up: false }
    }
}

/// 暫存區列複製到顯示頁：`sub_19BA4`、`19D08`（`shifted`）、`19DA8`（OR）、`1A09F`（AND）。
fn copy_hd(mem: &mut HdMem, c: &BlitCmd, off: Off, mode: CopyMode, shifted: bool) {
    let shape = Shape { shifted, ..Shape::full(mem.g.cols()) };
    copy_hd_ex(mem, c, off, mode, shape);
}

/// 暫存區列複製到顯示頁，含鏡射與 2 位元組寬的變體。
fn copy_hd_ex(mem: &mut HdMem, c: &BlitCmd, off: Off, mode: CopyMode, shape: Shape) {
    let (shifted, mirror) = (shape.shifted, shape.mirror);
    let g = mem.g;
    let HdMem { pages, scratch, .. } = mem;
    let dst = &mut pages[pg(c)];
    if dst.is_empty() {
        return;
    }
    let pw = dst.w;
    // 位址不含 shift：負的 shift（頭目合成圖）會讓含 shift 的位址落到暫存區之外的頁，欄位移在下面另算。
    let (reg, srow, scol) = addr_region(g, g.scratch_src(c).wrapping_sub(Geom::rel(c.shift)) & g.mask());
    let src = match reg {
        Reg::Scratch(i) => &scratch[i],
        Reg::Page(_) => return,
    };
    if src.is_empty() {
        return;
    }
    let y0 = sx(c.y) * S + off.oy;
    let mut rows = sx(c.cx) * S;
    if rows <= 0 {
        return;
    }
    if shape.count_up {
        // 列號遞增、目的遞減：來源第 i 列只畫到 y + i <= 189。
        rows = rows.min((190 - sx(c.y)).max(0) * S);
    }
    let x0 = sx(c.x) * 8 * S + off.ox;
    let col0 = (scol + sx(c.shift)) * 8 * S + off.sox;
    // 位移版每個位元組的計數 `shift + i` 要落在 [0, 暫存區列寬)（原版 80），換成 HD 像素。
    let shift_dev = sx(c.shift) * 8 * S + off.sox;
    let sw = src.w;
    let mut klo = (-x0).max(0);
    let mut khi = pw.min(pw - x0).min(shape.bytes * 8 * S);
    if shifted {
        klo = klo.max(-shift_dev);
        khi = khi.min(sw - shift_dev);
    }
    for t in 0..rows {
        // 鏡射：來源第 t 個 HD 列畫在目的 y0 + S - 1 - t，列號往上走。
        let y = if mirror { y0 + (S - 1) - t } else { y0 + t };
        if y < 0 {
            if mirror {
                break;
            }
            continue;
        }
        if y >= shape.ylim {
            if mirror {
                continue;
            }
            break;
        }
        let sr0 = srow * S + t;
        let mut k = klo;
        while k < khi {
            let sc = col0 + k;
            let sr = sr0 + sc.div_euclid(sw);
            let sc = sc.rem_euclid(sw);
            let seg = (khi - k).min(sw - sc);
            let d0 = ((y * pw + x0 + k) as usize) * 4;
            let drow = &mut dst.px[d0..d0 + seg as usize * 4];
            if (0..SH).contains(&sr) {
                let s0 = ((sr * sw + sc) as usize) * 4;
                let srow_px = &src.px[s0..s0 + seg as usize * 4];
                match mode {
                    CopyMode::Over => {
                        // 來源整段不透明（背景圖的常態）就整段複製。
                        if srow_px.chunks_exact(4).all(|p| p[3] == 255) {
                            drow.copy_from_slice(srow_px);
                        } else {
                            for (d, s) in drow.chunks_exact_mut(4).zip(srow_px.chunks_exact(4)) {
                                over_black(d, s);
                            }
                        }
                    }
                    CopyMode::Or => {
                        for (d, s) in drow.chunks_exact_mut(4).zip(srow_px.chunks_exact(4)) {
                            over(d, s);
                        }
                    }
                    CopyMode::And => {
                        for (d, s) in drow.chunks_exact_mut(4).zip(srow_px.chunks_exact(4)) {
                            // 黑白遮罩（不透明的灰階）的亮度就是灰階值，省掉加權。
                            let f = if s[3] == 255 && s[0] == s[1] && s[1] == s[2] { s[0] as u32 } else { lum(s) };
                            if f != 255 {
                                scale_rgb(d, f);
                            }
                        }
                    }
                }
            } else {
                // 暫存區以外：視為全透明黑。
                for d in drow.chunks_exact_mut(4) {
                    match mode {
                        CopyMode::Over => over_black(d, &[0, 0, 0, 0]),
                        CopyMode::Or => {}
                        CopyMode::And => scale_rgb(d, 0),
                    }
                }
            }
            k += seg;
        }
    }
}

fn fill_rect(page: &mut Canvas, x0: i32, y0: i32, x1: i32, y1: i32, rgb: [u8; 3], row_limit: i32) {
    if page.is_empty() {
        return;
    }
    let pw = page.w;
    for y in y0.max(0)..y1.min(row_limit) {
        for x in x0.max(0)..x1.min(pw) {
            let o = ((y * pw + x) as usize) * 4;
            page.px[o..o + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
}

/// `sub_1A287`：填實色，每列一整列寬。
fn fill_hd(mem: &mut HdMem, vis: &[[u8; 3]; 16], c: &BlitCmd, off: Off) {
    let rows = sx(c.cx) * S + off.ocx;
    if rows <= 0 {
        return;
    }
    let y0 = sx(c.y) * S + off.oy;
    let x0 = sx(c.x) * 8 * S + off.ox;
    let pw = mem.pages[pg(c)].w;
    fill_rect(&mut mem.pages[pg(c)], x0, y0, x0 + pw, y0 + rows, vis[(c.dx & 15) as usize], GAME_ROWS);
}

/// `sub_1929F`：血條的一格，7 列各 8 個像素，兩頁同寫，不裁 y 上限。
fn bar_hd(mem: &mut HdMem, vis: &[[u8; 3]; 16], c: &BlitCmd) {
    let x0 = sx(c.x) * 8 * S;
    let y0 = sx(c.y) * S;
    for p in mem.pages.iter_mut() {
        fill_rect(p, x0, y0, x0 + 8 * S, y0 + 7 * S, vis[(c.dx & 15) as usize], PH);
    }
}

/// `sub_19CF9`：清 190 列成色號 0 的顏色。
fn clear_page_hd(mem: &mut HdMem, vis: &[[u8; 3]; 16], c: &BlitCmd) {
    let page = &mut mem.pages[pg(c)];
    let pw = page.w;
    fill_rect(page, 0, 0, pw, GAME_ROWS, vis[0], GAME_ROWS);
}

/// `sub_1152D`（暫存區 B，`which` = 1）與 `sub_11504`（暫存區 A，`which` = 0）：
/// 前 `417Eh` 位元組（209 列又 46 位元組，戰場加大時同樣是 209 列又 46 位元組）清成透明。
/// map mask 不是 0Fh 時只清部分平面，HD 畫布表達不了，略過。
fn clear_scratch_hd(mem: &mut HdMem, which: usize) {
    if mem.map_mask != 0x0F {
        return;
    }
    let sc = &mut mem.scratch[which];
    if sc.is_empty() {
        return;
    }
    let sw = sc.w;
    let bytes = mem.g.scratch_clear_bytes() as i32;
    let scols = i32::from(mem.g.scols());
    let full = (bytes / scols) * S;
    let part = (bytes % scols) * 8 * S;
    for y in 0..(full + S).min(SH) {
        let w = if y < full { sw } else { part };
        let o = (y * sw) as usize * 4;
        sc.px[o..o + w as usize * 4].fill(0);
    }
}

/// 數字畫在平面 3，這裡改成把字形的亮點畫成色號 8 的顏色。`both` 為真時兩頁同寫。
pub fn counter(pages: &mut [Canvas; 2], vis: &[[u8; 3]; 16], col: u16, row: u16, value: u16, both: bool) {
    let mut v = value;
    let mut digits = [0u8; 5];
    for (i, d) in [10000u16, 1000, 100, 10, 1].into_iter().enumerate() {
        digits[i] = (v / d) as u8;
        v %= d;
    }
    let rgb = vis[8];
    for (i, &dg) in digits.iter().enumerate() {
        let glyph = DIGITS[(dg % 10) as usize];
        for (r, &bits) in glyph.iter().enumerate() {
            let y = row as i32 + r as i32;
            if y >= 200 {
                continue;
            }
            for bit in 0..8i32 {
                if (bits >> (7 - bit)) & 1 == 0 {
                    continue;
                }
                let x = (col as i32 + i as i32) * 8 + bit;
                for (pi, page) in pages.iter_mut().enumerate() {
                    // 頁寬之外（原版 320 像素）不畫。
                    if (!both && pi != 0) || page.is_empty() || x * S >= page.w {
                        continue;
                    }
                    fill_rect(page, x * S, y * S, (x + 1) * S, (y + 1) * S, rgb, PH);
                }
            }
        }
    }
}

/// 快照入口：兩個顯示頁用索引色記憶體（`idx_px`，每像素一個位元組）依 `vis` 上色後最近鄰放大 4 倍帶入，
/// 暫存區留空（透明）。入口之前畫上去的 HD 圖已經不可考，所以這是原版圖的放大，不是 HD 圖。
pub fn load_pages(mem: &mut HdMem, idx_px: &[u8], vis: &[[u8; 3]; 16]) {
    assert!(!mem.g.wide(), "快照只有原版的 64 KB 版面");
    for (p, base) in [(0usize, 0usize), (1, 0x8000)] {
        let page = &mut mem.pages[p];
        if page.is_empty() {
            continue;
        }
        for y in 0..200usize {
            for x in 0..320usize {
                let c = vis[(idx_px[(base * 8 + y * 320 + x) % idx_px.len()] & 15) as usize];
                for dy in 0..SCALE_USIZE {
                    let row = (y * SCALE_USIZE + dy) * PW as usize + x * SCALE_USIZE;
                    for dx in 0..SCALE_USIZE {
                        page.px[(row + dx) * 4..(row + dx) * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                    }
                }
            }
        }
    }
}

const SCALE_USIZE: usize = SCALE;

/// `loc_19534` 資料取反後寫入（`sub_18C6E`、`sub_190CC`）：整個圖矩形都被寫到，原版色號 `c` 變成 `15 - c`。
/// HD 圖沒有色號，做法是矩形先填色號 15 的顏色（原版色號 0 取反的結果），再把 HD 圖的 RGB 取反後以 alpha 疊上。
/// 對白底黑字、黑底白字的選單字樣與原版的 `15 - c` 一致，其他顏色只是近似（HD 靠人判讀，不求逐像素）。
fn blit_inv(page: &mut Canvas, img: &HdImg, x0: i32, y0: i32, bg: [u8; 3]) {
    if page.is_empty() {
        return;
    }
    let pw = page.w;
    let xs = (-x0).max(0);
    let xe = (img.w as i32).min(pw - x0);
    if xs >= xe {
        return;
    }
    for iy in 0..img.h as i32 {
        let y = y0 + iy;
        if y < 0 {
            continue;
        }
        if y >= GAME_ROWS {
            break;
        }
        let so = iy as usize * img.w;
        let srow = &img.rgba[(so + xs as usize) * 4..(so + xe as usize) * 4];
        let drow = &mut page.px[((y * pw + x0 + xs) as usize) * 4..((y * pw + x0 + xe) as usize) * 4];
        for (d, s) in drow.chunks_exact_mut(4).zip(srow.chunks_exact(4)) {
            d.copy_from_slice(&[bg[0], bg[1], bg[2], 255]);
            over(d, &[255 - s[0], 255 - s[1], 255 - s[2], s[3]]);
        }
    }
}

/// 圓角矩形（垂直漸層，邊緣反鋸齒）疊到顯示頁。範圍夾在遊戲區 `[0, 1280) x [0, 760)` 內。
fn fill_round_grad(page: &mut Canvas, x0: i32, y0: i32, w: i32, h: i32, rad: f32, top: [u8; 3], bot: [u8; 3], edge: [u8; 3]) {
    if page.is_empty() || w <= 0 || h <= 0 {
        return;
    }
    let (hw, hh) = (w as f32 / 2.0, h as f32 / 2.0);
    let rad = rad.min(hw).min(hh);
    let pw = page.w;
    for iy in 0..h {
        let y = y0 + iy;
        if y < 0 || y >= GAME_ROWS {
            continue;
        }
        for ix in 0..w {
            let x = x0 + ix;
            if x < 0 || x >= pw {
                continue;
            }
            let (px, py) = (ix as f32 + 0.5 - hw, iy as f32 + 0.5 - hh);
            let (qx, qy) = (px.abs() - (hw - rad), py.abs() - (hh - rad));
            let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - rad;
            let cov = (0.5 - d).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let t = iy as f32 / (h - 1).max(1) as f32;
            let mut c = [0u8; 3];
            for k in 0..3 {
                c[k] = (top[k] as f32 + (bot[k] as f32 - top[k] as f32) * t).round() as u8;
            }
            // 內側 1.5 像素內漸變成邊線色。
            let e = (1.0 - (-d / 1.5).clamp(0.0, 1.0)) * 0.55;
            for k in 0..3 {
                c[k] = (c[k] as f32 + (edge[k] as f32 - c[k] as f32) * e).round() as u8;
            }
            let o = ((y * pw + x) as usize) * 4;
            over(&mut page.px[o..o + 4], &[c[0], c[1], c[2], (cov * 255.0).round() as u8]);
        }
    }
}

/// 把圖「相對於它的底色」加亮疊在頁面上：選單字樣是不透明的圖，底色是左上角那個像素（近黑），字是銀灰色。
/// 每個像素減掉底色後（負值當 0）加到頁面上，所以底色處露出後面的高亮條，字照樣是亮的，斜角的明暗也保留。
fn add_over_bg(page: &mut Canvas, img: &HdImg, x0: i32, y0: i32) {
    if page.is_empty() || img.w == 0 || img.h == 0 {
        return;
    }
    let bg = [img.rgba[0] as i32, img.rgba[1] as i32, img.rgba[2] as i32];
    let pw = page.w;
    let xs = (-x0).max(0);
    let xe = (img.w as i32).min(pw - x0);
    if xs >= xe {
        return;
    }
    for iy in 0..img.h as i32 {
        let y = y0 + iy;
        if y < 0 {
            continue;
        }
        if y >= GAME_ROWS {
            break;
        }
        let so = iy as usize * img.w;
        let srow = &img.rgba[(so + xs as usize) * 4..(so + xe as usize) * 4];
        let drow = &mut page.px[((y * pw + x0 + xs) as usize) * 4..((y * pw + x0 + xe) as usize) * 4];
        for (d, s) in drow.chunks_exact_mut(4).zip(srow.chunks_exact(4)) {
            let a = s[3] as i32;
            for k in 0..3 {
                let delta = (s[k] as i32 - bg[k]).max(0) * a / 255;
                d[k] = (d[k] as i32 + delta).min(255) as u8;
            }
            d[3] = 255;
        }
    }
}

/// 標題選單選中列（`MyOverwriteInverted`）在 HD 介面的畫法（規格 005「HD 介面」）：不取反，字樣照常亮著（文字不反相），
/// 後面墊一條圓角高亮條。高亮條正好是圖的矩形：選單不清畫面，沒選中的列用 `MyOverwrite` 覆寫自己的矩形，
/// 條若超出矩形，選取移到別列時會留下殘影。
pub fn menu_highlight(mem: &mut HdMem, hd: &mut HdAssets, assets: &Assets, pal: &Pal, c: &BlitCmd) {
    let x0 = sx(c.x) * 8 * S;
    let y0 = sx(c.y) * S;
    if mem.pages[pg(c)].is_empty() {
        return;
    }
    if let Some(img) = hd.image(Lib::My, c.index, pal, assets) {
        let page = &mut mem.pages[pg(c)];
        // 先清成黑，再墊高亮條，最後疊字：和 `MyOverwrite` 一樣整個矩形都被寫到。
        fill_round_grad(page, x0, y0, img.w as i32, img.h as i32, 0.0, [0, 0, 0], [0, 0, 0], [0, 0, 0]);
        fill_round_grad(page, x0, y0, img.w as i32, img.h as i32, 12.0 * S as f32 / 4.0, [66, 140, 236], [40, 98, 190], [150, 200, 255]);
        add_over_bg(page, &img, x0, y0);
    }
    if restores_map_mask(c.op) {
        mem.map_mask = 0x0F;
    }
}

/// `sub_18ACE`（`sub_196E5`）：MAP 圖塊寫進暫存區。原版固定 10 列、每列 1 位元組，也就是 8x10 邏輯像素，
/// HD 圖塊是 32x40，原樣（含 alpha）寫入，不經過 `scratch_put` 的偶數位元組規則（那條規則會把寬度 1 的圖丟掉）。
fn scratch_put_tile(sc: &mut Canvas, img: &HdImg, c: &BlitCmd) {
    if sc.is_empty() {
        return;
    }
    let (w, h) = (img.w.min(8 * SCALE) as i32, img.h.min(10 * SCALE) as i32);
    let sw = sc.w;
    let x0 = sx(c.x) * 8 * S;
    let y0 = sx(c.y) * S;
    let xs = (-x0).max(0);
    let xe = w.min(sw - x0);
    if xs >= xe {
        return;
    }
    for iy in 0..h {
        let y = y0 + iy;
        if !(0..SH).contains(&y) {
            continue;
        }
        let so = iy as usize * img.w;
        let (d0, d1) = ((y * sw + x0 + xs) as usize, (y * sw + x0 + xe) as usize);
        sc.px[d0 * 4..d1 * 4].copy_from_slice(&img.rgba[(so + xs as usize) * 4..(so + xe as usize) * 4]);
    }
}

/// `sub_18894`：兩頁的 200 列清成色號 0 的顏色。
fn clear_both_pages_hd(mem: &mut HdMem, vis: &[[u8; 3]; 16]) {
    for p in mem.pages.iter_mut() {
        let pw = p.w;
        fill_rect(p, 0, 0, pw, PH, vis[0], PH);
    }
}

/// `DrawCmd::SetMapMask`。
pub fn set_map_mask(mem: &mut HdMem, mask: u8) {
    mem.map_mask = mask & 0x0F;
}

/// `DrawCmd::VramFill`：`rep stosw`。HD 畫布沒有平面，所以只在 map mask 為 0Fh 時處理：
/// 每個位元組的 8 個位元各是一個像素，1 填色號 15 的顏色，0 填色號 0 的顏色（暫存區的 0 是透明）。
/// 位址落在哪一塊由 `addr_region` 決定，顯示頁正好 200 列，沒有空隙。
pub fn vram_fill(mem: &mut HdMem, vis: &[[u8; 3]; 16], off: u16, words: u16, value: u16) {
    if mem.map_mask != 0x0F {
        return;
    }
    let (lo, hi) = (value as u8, (value >> 8) as u8);
    let g = mem.g;
    let mut a = if g.wide() { g.seg(off) } else { u32::from(off) };
    for i in 0..words as usize * 2 {
        let b = if i % 2 == 0 { lo } else { hi };
        let (reg, row, col) = addr_region(g, a);
        a = g.wrap(a.wrapping_add(1));
        let (canvas, is_page) = match reg {
            Reg::Page(p) => (&mut mem.pages[p], true),
            Reg::Scratch(p) => (&mut mem.scratch[p], false),
        };
        if canvas.is_empty() {
            continue;
        }
        let (cw, ch) = (canvas.w, if is_page { PH } else { SH });
        for bit in 0..8i32 {
            let on = (b >> (7 - bit)) & 1 != 0;
            let px: [u8; 4] = if on {
                [vis[15][0], vis[15][1], vis[15][2], 255]
            } else if is_page {
                [vis[0][0], vis[0][1], vis[0][2], 255]
            } else {
                [0, 0, 0, 0]
            };
            let x = (col * 8 + bit) * S;
            let y = row * S;
            for dy in 0..S {
                if y + dy >= ch {
                    break;
                }
                for dx in 0..S {
                    if x + dx >= cw {
                        break;
                    }
                    let o = (((y + dy) * cw + x + dx) as usize) * 4;
                    canvas.px[o..o + 4].copy_from_slice(&px);
                }
            }
        }
    }
}

/// 暫存區位元組串流位置（每列 `scols` 位元組，原版 80）換成（邏輯列，位元組欄）。
#[inline]
fn spos(scols: i32, pos: i32) -> (i32, i32) {
    (pos.div_euclid(scols), pos.rem_euclid(scols))
}

/// 暫存區 `s` 的（HD 列 `sy`，HD 欄 `sx`）起 `len` 個 HD 像素，複製到暫存區 `d` 的（`dy`，`dx`）。超出畫布略過。
fn move_px(sc: &mut [Canvas; 2], s: usize, sy: i32, sx: i32, d: usize, dy: i32, dx: i32, len: i32) {
    if len <= 0 || sc[s].is_empty() || sc[d].is_empty() {
        return;
    }
    let sw = sc[s].w;
    if !(0..SH).contains(&sy) || !(0..SH).contains(&dy) || sx < 0 || dx < 0 || sx + len > sw || dx + len > sw {
        return;
    }
    let (so, dof, n) = (((sy * sw + sx) as usize) * 4, ((dy * sw + dx) as usize) * 4, len as usize * 4);
    if s == d {
        sc[s].px.copy_within(so..so + n, dof);
    } else {
        let (a, b) = sc.split_at_mut(1);
        let (from, to) = if s == 0 { (&a[0], &mut b[0]) } else { (&b[0], &mut a[0]) };
        to.px[dof..dof + n].copy_from_slice(&from.px[so..so + n]);
    }
}

/// `sub_17EBA`、`sub_17F26` 的一個位元組：高低半位元組對調複製（A 到 B）。
fn hd_swap_byte(sc: &mut [Canvas; 2], scols: i32, s: usize, sp: i32, d: usize, dp: i32) {
    let ((sr, scol), (dr, dcol)) = (spos(scols, sp), spos(scols, dp));
    for line in 0..S {
        let (sy, dy) = (sr * S + line, dr * S + line);
        move_px(sc, s, sy, (scol * 8 + 4) * S, d, dy, dcol * 8 * S, 4 * S);
        move_px(sc, s, sy, scol * 8 * S, d, dy, (dcol * 8 + 4) * S, 4 * S);
    }
}

/// `sub_17EEA`、`sub_17F5D` 的一個位元組：暫存區 `d` 內位元組 `to` 的像素 4 至 7 取自位元組 `from` 的像素 4 至 7。
fn hd_low_nibble(sc: &mut [Canvas; 2], scols: i32, d: usize, from: i32, to: i32) {
    let ((fr, fc), (tr, tc)) = (spos(scols, from), spos(scols, to));
    for line in 0..S {
        move_px(sc, d, fr * S + line, (fc * 8 + 4) * S, d, tr * S + line, (tc * 8 + 4) * S, 4 * S);
    }
}

/// 第三關捲動的 `ScrollRotateCopy`、`ScrollShiftLeft4`、`ScrollRotateColumn`、`ScrollMergeSeam`。原版對四個平面各做一次（`sub_1882E` 選平面 3、2、1、0），
/// HD 畫布沒有平面，只在選到平面 3（map mask 8）那一次做，其餘三次略過。
/// 來源暫存區由 `word_2685C` 選（0 是 A），目的是另一個。
fn hd_scroll_pair(mem: &mut HdMem, c: &BlitCmd) {
    if mem.map_mask != 8 {
        return;
    }
    let s = ((c.scratch >> 15) & 1) as usize;
    let d = s ^ 1;
    let x = sx(c.x);
    let scols = i32::from(mem.g.scols());
    let sc = &mut mem.scratch;
    match c.op {
        // 200 列（原版 3E80h 位元組）。
        Op::ScrollRotateCopy => {
            for i in 0..200 * scols {
                hd_swap_byte(sc, scols, s, x + i, d, x + i);
            }
        }
        Op::ScrollShiftLeft4 => {
            for i in 0..200 * scols {
                hd_low_nibble(sc, scols, d, x + i + 1, x + i);
            }
        }
        Op::ScrollRotateColumn => {
            for r in 0..200 {
                hd_swap_byte(sc, scols, s, x + r * scols, d, x + r * scols);
            }
        }
        Op::ScrollMergeSeam => {
            for r in 0..200 {
                hd_low_nibble(sc, scols, d, x + r * scols, x + r * scols - 1);
            }
        }
        _ => {}
    }
}

/// `sub_17FA0`、`sub_18031`：暫存區（`word_2685C`）第 `word_25BC4` 欄起的一個顯示頁寬（原版 40 欄）x 190 列，寫到顯示頁。
/// 覆寫，透明處變黑。視窗左緣 `word_25BC4` 在 0 至 40 之間，是背景圖的週期，與版面無關。
fn hd_scroll_view(mem: &mut HdMem, c: &BlitCmd) {
    let HdMem { pages, scratch, .. } = mem;
    let dst = &mut pages[pg(c)];
    let src = &scratch[((c.scratch >> 15) & 1) as usize];
    if dst.is_empty() || src.is_empty() {
        return;
    }
    let (pw, sw) = (dst.w, src.w);
    let x0 = sx(c.x).clamp(0, 40) * 8 * S;
    let w = pw.min(sw - x0);
    for y in 0..190 * S {
        let so = ((y * sw + x0) as usize) * 4;
        let d0 = ((y * pw) as usize) * 4;
        let srow = &src.px[so..so + w as usize * 4];
        let drow = &mut dst.px[d0..d0 + w as usize * 4];
        if srow.chunks_exact(4).all(|p| p[3] == 255) {
            drow.copy_from_slice(srow);
        } else {
            for (d, s) in drow.chunks_exact_mut(4).zip(srow.chunks_exact(4)) {
                over_black(d, s);
            }
        }
    }
}

/// `sub_180C6` 進位時的搬移：暫存區 A、B 的位元組串流整體左移 28h 位元組（背景圖的一個週期），共 190 列。
/// 一列分成 40 位元組一段（原版兩段，戰場加大三段）：每段取自右鄰的一段，最後一段取自下一列的第一段。
fn hd_scroll_left(mem: &mut HdMem) {
    let half = (40 * 8 * S) as usize;
    let segs = mem.g.scols() as usize / 40;
    for sc in mem.scratch.iter_mut() {
        if sc.is_empty() {
            continue;
        }
        let sw = sc.w;
        for r in 0..190 {
            for line in 0..S {
                let y = r * S + line;
                let row = (y * sw) as usize;
                for k in 0..segs - 1 {
                    sc.px.copy_within((row + (k + 1) * half) * 4..(row + (k + 2) * half) * 4, (row + k * half) * 4);
                }
                let next = ((y + S) * sw) as usize;
                sc.px.copy_within(next * 4..(next + half) * 4, (row + (segs - 1) * half) * 4);
            }
        }
    }
}

/// 這個命令結束時原版會呼叫 `sub_18878`（map mask 回 0Fh）。不呼叫的：`ClearScratchA`、`ClearScratchB`（清暫存區）、
/// `ScratchReplace`、`ScratchReplaceShifted`、`ScratchReplaceMirrored`、`ScratchLatchPairs`（閂鎖複製）、`FillRows`、`BarCell`（填色），以及第三關捲動的七個命令
/// （`sub_17E73` 等呼叫端自己在最後呼叫 `sub_18878`）。
fn restores_map_mask(op: Op) -> bool {
    !matches!(
        op,
        Op::ClearScratchA
            | Op::ClearScratchB
            | Op::ScratchReplace
            | Op::ScratchReplaceShifted
            | Op::ScratchReplaceMirrored
            | Op::ScratchLatchPairs
            | Op::FillRows
            | Op::BarCell
            | Op::ScrollRotateCopy
            | Op::ScrollShiftLeft4
            | Op::ScrollRotateColumn
            | Op::ScrollMergeSeam
            | Op::ScrollShowA
            | Op::ScrollShowB
            | Op::ScrollShiftBothLeft
    )
}

/// 把圖寫進暫存區的命令用哪個圖庫。
pub fn scratch_image_lib(c: &BlitCmd) -> Option<Lib> {
    match c.op {
        Op::StageToScratch => Some(Lib::from_sht(c.sht_group)),
        Op::BegToScratch => Some(Lib::from_beg(c.beg_group)),
        Op::MapToScratch => Some(Lib::Map),
        _ => None,
    }
}

/// 套用一個 blit 命令。`Number` 的幀計數位置不在這裡處理。
pub fn apply(mem: &mut HdMem, hd: &mut HdAssets, assets: &Assets, pal: &Pal, c: &BlitCmd, off: Off) {
    let x0 = sx(c.x) * 8 * S + off.ox;
    let y0 = sx(c.y) * S + off.oy;
    let sht = Lib::from_sht(c.sht_group);
    let mask_lib = Lib::from_sht(c.mask_group);
    let beg = Lib::from_beg(c.beg_group);
    let img_op = |mem: &mut HdMem, hd: &mut HdAssets, lib: Lib, mode: Mode| {
        if mem.pages[pg(c)].is_empty() {
            return;
        }
        if let Some(img) = hd.image(lib, c.index, pal, assets) {
            blit_img(&mut mem.pages[pg(c)], &img, x0, y0, mode);
        }
    };
    let inv_op = |mem: &mut HdMem, hd: &mut HdAssets, lib: Lib| {
        if mem.pages[pg(c)].is_empty() {
            return;
        }
        if let Some(img) = hd.image(lib, c.index, pal, assets) {
            blit_inv(&mut mem.pages[pg(c)], &img, x0, y0, pal.visible8()[15]);
        }
    };
    let mask_op = |mem: &mut HdMem, hd: &mut HdAssets, lib: Lib, flash: bool| {
        if mem.pages[pg(c)].is_empty() {
            return;
        }
        if let Some(m) = hd.mask(lib, c.index, assets) {
            let white = flash.then(|| pal.visible8()[15]);
            mask_apply(&mut mem.pages[pg(c)], &m, x0, y0, white);
        }
    };
    let scratch_op = |mem: &mut HdMem, hd: &mut HdAssets, lib: Lib| {
        let Some(it) = lib.go(assets).planes[0].get(c.index as usize) else { return };
        let stride = it.stride;
        if let Some(img) = hd.image(lib, c.index, pal, assets) {
            scratch_put(&mut mem.scratch[((c.scratch >> 15) & 1) as usize], &img, stride, c);
        }
    };
    // 2 位元組寬（16 像素）的變體。
    let full = Shape::full(mem.g.cols());
    let narrow = Shape { bytes: 2, ..full };
    // 第四關通道上壁等鏡射版。`ScratchReplaceMirrored` 沒有 189 列上限，`bx` 遞減；其餘兩個 `bx` 遞增。
    let mirror = Shape { mirror: true, ylim: PH, ..full };
    let mirror_up = Shape { mirror: true, count_up: true, ..full };
    match c.op {
        Op::MyOverwrite => img_op(mem, hd, Lib::My, Mode::Mov),
        Op::MyOverlay | Op::MyOverlayPlayerShot | Op::MyOverlayEffect => img_op(mem, hd, Lib::My, Mode::Or),
        Op::MyOverwriteInverted => inv_op(mem, hd, Lib::My),
        Op::MyCut | Op::MyCutEffect | Op::MyCutPlayerShot => mask_op(mem, hd, Lib::My, false),
        Op::MyFlash => mask_op(mem, hd, Lib::My, true),
        Op::StageCut => mask_op(mem, hd, mask_lib, false),
        Op::StageFlash => mask_op(mem, hd, mask_lib, true),
        Op::StageOverlay => img_op(mem, hd, sht, Mode::Or),
        Op::StageOverwrite => img_op(mem, hd, sht, Mode::Mov),
        Op::BegOverwrite => img_op(mem, hd, beg, Mode::Mov),
        Op::BegInverted => inv_op(mem, hd, beg),
        Op::StageToScratch => scratch_op(mem, hd, sht),
        Op::BegToScratch => scratch_op(mem, hd, beg),
        Op::MapToScratch => {
            if let Some(img) = hd.image(Lib::Map, c.index, pal, assets) {
                scratch_put_tile(&mut mem.scratch[((c.scratch >> 15) & 1) as usize], &img, c);
            }
        }
        Op::ScratchReplace => copy_hd(mem, c, off, CopyMode::Over, false),
        Op::ScratchReplaceShifted => copy_hd(mem, c, off, CopyMode::Over, true),
        Op::ScratchReplaceMirrored => copy_hd_ex(mem, c, off, CopyMode::Over, mirror),
        Op::ScratchOr => copy_hd(mem, c, off, CopyMode::Or, false),
        Op::ScratchOrMirrored => copy_hd_ex(mem, c, off, CopyMode::Or, mirror_up),
        Op::ScratchOrPairs => copy_hd_ex(mem, c, off, CopyMode::Or, narrow),
        Op::ScratchLatchPairs => copy_hd_ex(mem, c, off, CopyMode::Over, narrow),
        Op::ScratchAnd => copy_hd(mem, c, off, CopyMode::And, false),
        Op::ScratchAndMirrored => copy_hd_ex(mem, c, off, CopyMode::And, mirror_up),
        Op::ScratchAndPairs => copy_hd_ex(mem, c, off, CopyMode::And, narrow),
        Op::ClearDrawPage => clear_page_hd(mem, &pal.visible8(), c),
        Op::FillRows => fill_hd(mem, &pal.visible8(), c, off),
        Op::BarCell => bar_hd(mem, &pal.visible8(), c),
        Op::ClearScratchA => clear_scratch_hd(mem, 0),
        Op::ClearScratchB => clear_scratch_hd(mem, 1),
        Op::ClearBothPages => clear_both_pages_hd(mem, &pal.visible8()),
        Op::ScrollRotateCopy | Op::ScrollShiftLeft4 | Op::ScrollRotateColumn | Op::ScrollMergeSeam => hd_scroll_pair(mem, c),
        Op::ScrollShowA | Op::ScrollShowB => hd_scroll_view(mem, c),
        Op::ScrollShiftBothLeft => hd_scroll_left(mem),
        // MGA 專用（`sub_1117F`），E 直接 `retn`。
        Op::EndOfFrameCopy => {}
        Op::Number => counter(&mut mem.pages, &pal.visible8(), c.text_x, c.text_y, c.cx, true),
    }
    if restores_map_mask(c.op) {
        mem.map_mask = 0x0F;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rs2_game::ShtGroup;

    fn img(w: usize, h: usize, px: &[[u8; 4]]) -> HdImg {
        assert_eq!(px.len(), w * h);
        HdImg { w, h, rgba: px.iter().flatten().copied().collect() }
    }

    fn page() -> Canvas {
        Canvas::new(PW as usize, PH as usize, [0, 0, 0, 255])
    }

    fn at(p: &Canvas, x: i32, y: i32) -> [u8; 4] {
        let o = ((y * PW + x) as usize) * 4;
        [p.px[o], p.px[o + 1], p.px[o + 2], p.px[o + 3]]
    }

    #[test]
    fn region_mapping_covers_the_address_space() {
        let g = Geom::ORIGINAL;
        assert!(matches!(addr_region(g, 0), (Reg::Page(0), 0, 0)));
        assert!(matches!(addr_region(g, 0x1F3F), (Reg::Page(0), 199, 39)));
        assert!(matches!(addr_region(g, 0x1F40), (Reg::Scratch(0), 0, 0)));
        assert!(matches!(addr_region(g, 0x1F40 + 80 * 5 + 7), (Reg::Scratch(0), 5, 7)));
        assert!(matches!(addr_region(g, 0x8000), (Reg::Page(1), 0, 0)));
        assert!(matches!(addr_region(g, 0x9F40), (Reg::Scratch(1), 0, 0)));
        assert!(matches!(addr_region(g, 0xFFFF), (Reg::Scratch(1), 309, 47)));
    }

    #[test]
    fn region_mapping_follows_the_wide_geometry() {
        for cols in [48u16, 60] {
            let g = Geom::new(cols);
            let page = 200 * u32::from(cols);
            assert!(matches!(addr_region(g, page - 1), (Reg::Page(0), 199, c) if c == i32::from(cols) - 1));
            assert!(matches!(addr_region(g, page), (Reg::Scratch(0), 0, 0)));
            assert!(matches!(addr_region(g, page + 120 * 5 + 7), (Reg::Scratch(0), 5, 7)));
            assert!(matches!(addr_region(g, 0x10000), (Reg::Page(1), 0, 0)));
            assert!(matches!(addr_region(g, 0x10000 + page), (Reg::Scratch(1), 0, 0)));
        }
    }

    #[test]
    fn or_blends_and_mov_covers_with_black() {
        let mut p = page();
        p.px[..4].copy_from_slice(&[100, 100, 100, 255]);
        let im = img(2, 1, &[[200, 0, 0, 128], [0, 200, 0, 0]]);
        blit_img(&mut p, &im, 0, 0, Mode::Or);
        let a = at(&p, 0, 0);
        assert_eq!(a[3], 255);
        assert_eq!(a[0], ((200u32 * 128 + 100 * 127 + 127) / 255) as u8);
        assert_eq!(at(&p, 1, 0), [0, 0, 0, 255], "alpha 0 不動，底色是黑");
        let mut p = page();
        p.px[4..8].copy_from_slice(&[9, 9, 9, 255]);
        blit_img(&mut p, &im, 0, 0, Mode::Mov);
        assert_eq!(at(&p, 1, 0), [0, 0, 0, 255], "覆寫時透明處變黑，蓋掉底下");
    }

    #[test]
    fn clipping_rules() {
        let mut p = page();
        let im = img(2, 2, &[[255, 255, 255, 255]; 4]);
        blit_img(&mut p, &im, -1, -1, Mode::Or);
        assert_eq!(at(&p, 0, 0), [255, 255, 255, 255]);
        assert_eq!(at(&p, 1, 0), [0, 0, 0, 255]);
        blit_img(&mut p, &im, PW - 1, 0, Mode::Or);
        assert_eq!(at(&p, PW - 1, 0), [255, 255, 255, 255]);
        // 第 190 邏輯列起不畫
        blit_img(&mut p, &im, 5, GAME_ROWS - 1, Mode::Or);
        assert_eq!(at(&p, 5, GAME_ROWS - 1), [255, 255, 255, 255]);
        assert_eq!(at(&p, 5, GAME_ROWS), [0, 0, 0, 255]);
    }

    #[test]
    fn mask_and_flash() {
        let mut p = page();
        fill_rect(&mut p, 0, 0, 4, 1, [200, 100, 50], PH);
        let m = HdMask { w: 4, h: 1, v: vec![255, 128, 0, 255] };
        mask_apply(&mut p, &m, 0, 0, None);
        assert_eq!(at(&p, 0, 0)[..3], [200, 100, 50]);
        assert_eq!(at(&p, 1, 0)[0], ((200u32 * 128 + 127) / 255) as u8);
        assert_eq!(at(&p, 2, 0)[..3], [0, 0, 0]);
        let mut p = page();
        fill_rect(&mut p, 0, 0, 4, 1, [200, 100, 50], PH);
        mask_apply(&mut p, &m, 0, 0, Some([255, 255, 255]));
        assert_eq!(at(&p, 0, 0)[..3], [200, 100, 50], "255 保留背景，不閃白");
        assert_eq!(at(&p, 2, 0)[..3], [255, 255, 255], "0 填成白");
    }

    #[test]
    fn scratch_copy_roundtrip_with_wrap_and_shift() {
        let mut mem = HdMem::from_parts(Geom::ORIGINAL, [page(), Canvas::empty()], [Canvas::new(SW as usize, SH as usize, [0, 0, 0, 0]), Canvas::empty()]);
        // 暫存區 A 第 0 列：每個 HD 像素的 R 等於欄號低位元組，用來辨認來源欄。
        for y in 0..S {
            for x in 0..SW {
                let o = ((y * SW + x) as usize) * 4;
                mem.scratch[0].px[o..o + 4].copy_from_slice(&[(x & 0xFF) as u8, (x >> 8) as u8, 0, 255]);
            }
        }
        let mut c = BlitCmd {
            op: Op::ScratchReplace,
            x: 0,
            y: 0,
            index: 0,
            page: 0,
            scratch: 0,
            src_x: 0,
            src_y: 0,
            shift: 0,
            cx: 1,
            dx: 0,
            text_x: 0,
            text_y: 0,
            sht_group: ShtGroup::Bh1,
            mask_group: ShtGroup::Sht1,
            beg_group: rs2_game::BegGroup::Beg1,
            stride_reg: None,
            obj: None,
        };
        copy_hd(&mut mem, &c, Off::default(), CopyMode::Over, false);
        assert_eq!(at(&mem.pages[0], 0, 0)[..2], [0, 0]);
        assert_eq!(at(&mem.pages[0], 300, 3)[..2], [(300 & 0xFF) as u8, 1]);
        // 來源欄位移 80 HD 像素（20 邏輯像素的 4 倍）：目的第 0 欄取來源第 80 欄。
        copy_hd(&mut mem, &c, Off { sox: 80, ..Off::default() }, CopyMode::Over, false);
        assert_eq!(at(&mem.pages[0], 0, 0)[0], 80);
        // 負的來源位移環繞到前一列（第 -1 列在畫布外，視為黑）。
        copy_hd(&mut mem, &c, Off { sox: -4, ..Off::default() }, CopyMode::Over, false);
        assert_eq!(at(&mem.pages[0], 0, 0), [0, 0, 0, 255]);
        assert_eq!(at(&mem.pages[0], 4, 0)[0], 0);
        // 位移版：shift = -1 位元組（-32 HD 像素）時，目的前 32 欄不寫。
        c.op = Op::ScratchReplaceShifted;
        c.shift = 0xFFFF;
        let mut m2 = mem.clone();
        m2.pages[0] = Canvas::new(PW as usize, PH as usize, [7, 7, 7, 255]);
        copy_hd(&mut m2, &c, Off::default(), CopyMode::Over, true);
        assert_eq!(at(&m2.pages[0], 31, 0)[..3], [7, 7, 7]);
        assert_eq!(at(&m2.pages[0], 32, 0)[0], 0, "寫入的第一欄來自來源第 0 欄");
        assert_eq!(at(&m2.pages[0], 33, 0)[0], 1, "之後依序是來源第 1 欄");
    }

    #[test]
    fn and_uses_source_luminance_and_alpha() {
        let mut mem = HdMem::from_parts(Geom::ORIGINAL, [page(), Canvas::empty()], [Canvas::new(SW as usize, SH as usize, [0, 0, 0, 0]), Canvas::empty()]);
        fill_rect(&mut mem.pages[0], 0, 0, 8, 1, [100, 100, 100], PH);
        // 來源：欄 0 白不透明、欄 1 黑不透明、欄 2 白透明（等於沒寫）
        mem.scratch[0].px[..12].copy_from_slice(&[255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 0]);
        let c = BlitCmd {
            op: Op::ScratchAnd,
            x: 0,
            y: 0,
            index: 0,
            page: 0,
            scratch: 0,
            src_x: 0,
            src_y: 0,
            shift: 0,
            cx: 1,
            dx: 0,
            text_x: 0,
            text_y: 0,
            sht_group: ShtGroup::Bh1,
            mask_group: ShtGroup::Sht1,
            beg_group: rs2_game::BegGroup::Beg1,
            stride_reg: None,
            obj: None,
        };
        copy_hd(&mut mem, &c, Off::default(), CopyMode::And, false);
        assert_eq!(at(&mem.pages[0], 0, 0)[..3], [100, 100, 100]);
        assert_eq!(at(&mem.pages[0], 1, 0)[..3], [0, 0, 0]);
        assert_eq!(at(&mem.pages[0], 2, 0)[..3], [0, 0, 0]);
    }

    #[test]
    fn clear_scratch_b_clears_exactly_the_original_range() {
        let mut mem = HdMem::from_parts(Geom::ORIGINAL, [Canvas::empty(), Canvas::empty()], [Canvas::empty(), Canvas::new(SW as usize, SH as usize, [9, 9, 9, 9])]);
        clear_scratch_hd(&mut mem, 1);
        let sc = &mem.scratch[1];
        let px = |x: i32, y: i32| sc.px[((y * SW + x) as usize) * 4];
        assert_eq!(px(0, 0), 0);
        assert_eq!(px(SW - 1, 208 * S + 3), 0, "第 208 邏輯列整列清掉");
        assert_eq!(px(0, 209 * S), 0, "第 209 列前 46 位元組清掉");
        assert_eq!(px(46 * 8 * S - 1, 209 * S + 3), 0);
        assert_eq!(px(46 * 8 * S, 209 * S), 9, "第 46 位元組之後保留");
        assert_eq!(px(0, 210 * S), 9);
    }

    #[test]
    fn menu_highlight_bar_is_rounded_gradient_and_stays_inside_its_rect() {
        let mut p = page();
        fill_round_grad(&mut p, 100, 50, 80, 24, 8.0, [200, 0, 0], [0, 0, 200], [255, 255, 255]);
        let top = at(&p, 140, 53);
        let bottom = at(&p, 140, 70);
        assert!(top[0] > top[2] && bottom[2] > bottom[0], "上紅下藍的漸層：{top:?} {bottom:?}");
        assert_eq!(top[3], 255);
        assert_eq!(at(&p, 100, 50), [0, 0, 0, 255], "圓角把角落切掉，矩形外與角落不動");
        assert_eq!(at(&p, 99, 60), [0, 0, 0, 255]);
        assert_eq!(at(&p, 180, 60), [0, 0, 0, 255]);
        assert_eq!(at(&p, 140, 74), [0, 0, 0, 255]);
        // 邊緣反鋸齒：圓角弧線上有中間色。
        let mid = (100..110).flat_map(|x| (50..58).map(move |y| (x, y))).filter(|&(x, y)| {
            let c = at(&p, x, y);
            c[0] > 20 && c[0] < 180
        });
        assert!(mid.count() >= 4);
        // 超出遊戲區（第 760 列起是 HUD）的部分不畫。
        let mut q = page();
        fill_round_grad(&mut q, 0, GAME_ROWS - 4, 40, 20, 4.0, [255, 255, 255], [255, 255, 255], [255, 255, 255]);
        assert_eq!(at(&q, 20, GAME_ROWS - 1)[0], 255);
        assert_eq!(at(&q, 20, GAME_ROWS), [0, 0, 0, 255]);
    }

    #[test]
    fn menu_text_is_added_over_the_bar_instead_of_inverted() {
        let mut p = page();
        fill_round_grad(&mut p, 10, 10, 8, 4, 0.0, [66, 140, 236], [66, 140, 236], [66, 140, 236]);
        // 8x4 的不透明圖：底色 (1,1,1)，兩個銀灰色的字點。
        let mut px = [[1u8, 1, 1, 255]; 32];
        px[9] = [158, 166, 173, 255];
        px[10] = [255, 255, 255, 255];
        let im = img(8, 4, &px);
        add_over_bg(&mut p, &im, 10, 10);
        assert_eq!(at(&p, 10, 10), [66, 140, 236, 255], "底色處露出高亮條");
        assert_eq!(at(&p, 11, 11), [223, 255, 255, 255], "字加亮並夾在 255，不取反");
        assert_eq!(at(&p, 12, 11), [255, 255, 255, 255]);
        assert_eq!(at(&p, 9, 10), [0, 0, 0, 255], "圖的矩形外不動");
        // 部分透明的像素只加一部分。
        let mut q = page();
        let mut with_bg = vec![[0u8, 0, 0, 255]];
        with_bg.push([201, 201, 201, 128]);
        let two = img(2, 1, &with_bg);
        add_over_bg(&mut q, &two, 0, 0);
        assert_eq!(at(&q, 1, 0)[0], (201u32 * 128 / 255) as u8);
    }
}
