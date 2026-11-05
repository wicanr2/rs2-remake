//! HD 模式的遊戲內 HUD（規格 005「HD 介面」「HUD」）。畫在 1280×40 的 RGBA 緩衝區，蓋住 HD 畫面底部 40 像素
//! （對應原版第 190 到 199 列）。不依賴 SDL 子系統，時間由呼叫端以毫秒傳入，所以動畫可以單元測試。
//!
//! 資料全來自 `rs2_core::HudStatus`（`Game::hud_status`）。血條 8 格對應原版 `hp` 字組的 8 個位元：
//! 開局是 `FFh`，每次命中右移一位，所以亮的位元數就是剩下的格數。
//!
//! 充能條的刻度（`CHARGE_MAX` 與三級門檻）是顯示用的初值，規格 005「未決定」列的波動砲門檻定案後再對。

use super::style::{self, Rgba, color, fade, lighten, mix, with_alpha};
use super::{Align, Canvas, Rect, font, text};
use rs2_core::HudStatus;

/// HUD 緩衝區的寬高，也是 HD 畫面的寬度與 HUD 的高度。
pub const W: i32 = 1280;
pub const H: i32 = 40;
/// HUD 蓋在 HD 畫面的起始列（HD 畫面高 800）。
pub const Y0: i32 = 800 - H;
/// 掉格淡出的時間。
pub const FADE_MS: u64 = 200;
/// 剩 1 格時的呼吸週期。
pub const BREATH_MS: u64 = 1000;
pub const HP_CELLS: usize = 8;
/// 充能條滿格的計數（顯示用的初值）。
pub const CHARGE_MAX: u16 = 48;

const M: i32 = 16;
const CELL_W: i32 = 20;
const CELL_H: i32 = 12;
const CELL_GAP: i32 = 4;
const BAR_W: i32 = HP_CELLS as i32 * CELL_W + (HP_CELLS as i32 - 1) * CELL_GAP;
const TAG_W: i32 = 36;
const TAG_H: i32 = 22;
const ICON: i32 = 28;
const POD_R: i32 = 5;
const POD_W: i32 = 2 * POD_R * 2 + 6;
const CHARGE_W: i32 = 160;
const CHARGE_H: i32 = 12;
const DIGIT_W: i32 = 12;
const DIGITS: i32 = 5;
const FONT_H: i32 = 16;

/// 一側玩家區塊的寬度：標籤、血條、武器、砲座。
const BLOCK_W: i32 = TAG_W + 10 + BAR_W + 14 + ICON + 10 + POD_W;

/// HUD 要顯示的前端狀態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HudFlags {
    /// 改變規則的選項開著，右端標「強化」。
    pub rules: bool,
    /// 無敵開著，右端標「無敵」。
    pub god: bool,
    /// 波動砲充能選項開著，中央畫充能條。
    pub charge_bar: bool,
    /// 幀計數：`Some` 時畫在中央偏右。
    pub counter: Option<u16>,
}

/// 武器字母。
pub const WEAPONS: [(u16, char); 4] = [(b'L' as u16, 'L'), (b'B' as u16, 'B'), (b'K' as u16, 'K'), (b'S' as u16, 'S')];

/// 武器字母在 `MY` 圖庫的項號（L、B、K、S 依序是第 42 到 45 項）。
pub const WEAPON_MY_INDEX: [u16; 4] = [42, 43, 44, 45];

/// 武器值的編號 0 到 3，沒有武器或不認得是 `None`。
pub fn weapon_slot(w: u16) -> Option<usize> {
    WEAPONS.iter().position(|(v, _)| *v == w)
}

/// 血量字組亮幾格。
pub fn hp_cells(hp: u16) -> usize {
    (hp & 0xFF).count_ones() as usize
}

/// 小圖示：RGBA，已知寬高。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconImg {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// 四種武器的 HD 道具球。沒有 HD 資料時是 `None`，HUD 畫字母圓形代替。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HudIcons {
    pub balls: [Option<IconImg>; 4],
}

/// 掉格的紀錄：熄滅的時間與熄滅前的亮格數（決定淡出時用的顏色）。
type Lost = Option<(u64, usize)>;

/// 掉格淡出的狀態。呼叫端每步（或每次繪製）用最新的 `HudStatus` 更新一次。
#[derive(Clone, Debug, Default)]
pub struct HudState {
    cells: [usize; 2],
    seen: [bool; 2],
    lost: [[Lost; HP_CELLS]; 2],
}

impl HudState {
    pub fn new() -> HudState {
        HudState::default()
    }

    /// 以最新狀態更新。亮格數變少時，熄滅的那幾格開始淡出；變多（新局、復活）或玩家離場就清掉。
    pub fn update(&mut self, now_ms: u64, st: &[HudStatus; 2]) {
        for p in 0..2 {
            if !st[p].active {
                self.seen[p] = false;
                self.lost[p] = [None; HP_CELLS];
                self.cells[p] = 0;
                continue;
            }
            let n = hp_cells(st[p].hp);
            if self.seen[p] {
                let prev = self.cells[p];
                if n < prev {
                    for i in n..prev {
                        self.lost[p][i] = Some((now_ms, prev));
                    }
                } else if n > prev {
                    for i in prev..n.min(HP_CELLS) {
                        self.lost[p][i] = None;
                    }
                }
            }
            self.seen[p] = true;
            self.cells[p] = n;
        }
    }

    /// 第 `i` 格的淡出程度：1.0 剛熄滅，0.0 已淡完或沒有掉格。回傳 (程度, 熄滅前的亮格數)。
    fn fading(&self, p: usize, i: usize, now_ms: u64) -> Option<(f32, usize)> {
        let (t0, prev) = self.lost[p][i]?;
        let dt = now_ms.saturating_sub(t0);
        (dt < FADE_MS).then(|| (1.0 - dt as f32 / FADE_MS as f32, prev))
    }
}

/// 血條顏色分級。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HpTone {
    Healthy,
    Amber,
    Red,
}

/// 滿血青綠，剩 3 格以下琥珀，剩 1 格紅色。
pub fn hp_tone(cells: usize) -> HpTone {
    match cells {
        0 | 1 => HpTone::Red,
        2 | 3 => HpTone::Amber,
        _ => HpTone::Healthy,
    }
}

fn tone_colors(t: HpTone) -> (Rgba, Rgba) {
    let base = match t {
        HpTone::Healthy => color::ON,
        HpTone::Amber => color::WARN,
        HpTone::Red => color::DANGER,
    };
    (lighten(base, 0.28), mix(base, color::BG, 0.30))
}

/// 剩 1 格時的呼吸：每 `BREATH_MS` 一輪，亮度在 0.45 到 1.0 之間。
pub fn breath(now_ms: u64) -> f32 {
    let ph = (now_ms % BREATH_MS) as f32 / BREATH_MS as f32;
    0.725 + 0.275 * (ph * std::f32::consts::TAU).cos()
}

/// HUD 上的元素。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elem {
    Tag(usize),
    Bar(usize),
    Weapon(usize),
    Pods(usize),
    Charge,
    Counter,
    Rules,
    God,
}

fn tag_w(s: &str) -> i32 {
    font::builtin().text_width(s, FONT_H) + 16
}

/// 各元素的矩形（HUD 緩衝區座標）。P1 靠左，P2 靠右並鏡射排列，沒有的玩家不排。
pub fn layout(active: [bool; 2], flags: &HudFlags) -> Vec<(Elem, Rect)> {
    let mut v = Vec::new();
    let y_bar = (H - CELL_H) / 2;
    for p in 0..2 {
        if !active[p] {
            continue;
        }
        // 由外向內：標籤、血條、武器、砲座。P2 往左排。
        let mut x = if p == 0 { M } else { W - M };
        let step = |x: &mut i32, w: i32| -> i32 {
            if p == 0 {
                let at = *x;
                *x += w;
                at
            } else {
                *x -= w;
                *x
            }
        };
        let tx = step(&mut x, TAG_W);
        v.push((Elem::Tag(p), Rect::new(tx, (H - TAG_H) / 2, TAG_W, TAG_H)));
        step(&mut x, 10);
        let bx = step(&mut x, BAR_W);
        v.push((Elem::Bar(p), Rect::new(bx, y_bar, BAR_W, CELL_H)));
        step(&mut x, 14);
        let wx = step(&mut x, ICON);
        v.push((Elem::Weapon(p), Rect::new(wx, (H - ICON) / 2, ICON, ICON)));
        step(&mut x, 10);
        let px = step(&mut x, POD_W);
        v.push((Elem::Pods(p), Rect::new(px, (H - POD_R * 2) / 2, POD_W, POD_R * 2)));
    }
    if flags.charge_bar {
        v.push((Elem::Charge, Rect::new((W - CHARGE_W) / 2, (H - CHARGE_H) / 2, CHARGE_W, CHARGE_H)));
    }
    if flags.counter.is_some() {
        let w = DIGIT_W * DIGITS;
        v.push((Elem::Counter, Rect::new(W / 2 + 104, (H - FONT_H) / 2, w, FONT_H)));
    }
    // 右端的標籤：有 P2 時排在 P2 區塊左邊。
    let mut right = if active[1] { W - M - BLOCK_W - 16 } else { W - M };
    for (on, e, s) in [(flags.god, Elem::God, text::HUD_GOD), (flags.rules, Elem::Rules, text::HUD_RULES)] {
        if on {
            let w = tag_w(s);
            right -= w;
            v.push((e, Rect::new(right, (H - TAG_H) / 2, w, TAG_H)));
            right -= 8;
        }
    }
    v
}

/// 縮小或放大 `src` 到 `fit × fit` 的框內置中，回傳 RGBA 疊在 `c` 的 `r` 上。區域平均，保持長寬比。
fn draw_scaled(c: &mut Canvas, img: &IconImg, r: Rect) {
    if img.w == 0 || img.h == 0 || img.rgba.len() < img.w * img.h * 4 {
        return;
    }
    let s = (r.w as f32 / img.w as f32).min(r.h as f32 / img.h as f32);
    let (dw, dh) = (((img.w as f32 * s).round() as i32).max(1), ((img.h as f32 * s).round() as i32).max(1));
    let (ox, oy) = (r.x + (r.w - dw) / 2, r.y + (r.h - dh) / 2);
    for dy in 0..dh {
        for dx in 0..dw {
            let (sx0, sx1) = (dx as f32 / s, (dx + 1) as f32 / s);
            let (sy0, sy1) = (dy as f32 / s, (dy + 1) as f32 / s);
            let (mut acc, mut wsum) = ([0f32; 4], 0f32);
            let (ix0, ix1) = (sx0.floor() as usize, (sx1.ceil() as usize).min(img.w));
            let (iy0, iy1) = (sy0.floor() as usize, (sy1.ceil() as usize).min(img.h));
            for iy in iy0..iy1 {
                let wy = sy1.min((iy + 1) as f32) - sy0.max(iy as f32);
                for ix in ix0..ix1 {
                    let wx = sx1.min((ix + 1) as f32) - sx0.max(ix as f32);
                    let w = wx.max(0.0) * wy.max(0.0);
                    let o = (iy * img.w + ix) * 4;
                    let a = img.rgba[o + 3] as f32 / 255.0;
                    // 預乘平均，避免透明處的顏色滲進邊緣。
                    for k in 0..3 {
                        acc[k] += img.rgba[o + k] as f32 * a * w;
                    }
                    acc[3] += a * w;
                    wsum += w;
                }
            }
            if wsum <= 0.0 || acc[3] <= 0.0 {
                continue;
            }
            let a = acc[3] / wsum;
            let px = [(acc[0] / acc[3]).round() as u8, (acc[1] / acc[3]).round() as u8, (acc[2] / acc[3]).round() as u8, (a * 255.0).round() as u8];
            let (x, y) = (ox + dx, oy + dy);
            if x >= 0 && y >= 0 && x < c.w && y < c.h {
                let o = ((y * c.w + x) * 4) as usize;
                style::over(&mut c.px[o..o + 4], px, 1.0);
            }
        }
    }
}

fn weapon_tint(slot: usize) -> Rgba {
    match slot {
        0 => color::ON,
        1 => color::PRIMARY,
        2 => color::WARN,
        _ => style::rgb(0xFF7AB6),
    }
}

fn player_tint(p: usize) -> Rgba {
    if p == 0 { color::PRIMARY } else { style::rgb(0xFF8A5C) }
}

fn draw_plate(c: &mut Canvas) {
    let (top, bot) = (style::rgb(0x111B33), style::rgb(0x070B14));
    for y in 0..H {
        let col = mix(top, bot, y as f32 / (H - 1) as f32);
        c.rect(Rect::new(0, y, W, 1), col);
    }
    // 上緣 1 像素的半透明亮線。
    c.rect(Rect::new(0, 0, W, 1), [255, 255, 255, 40]);
}

fn draw_tag_chip(c: &mut Canvas, r: Rect, label: &str, tint: Rgba) {
    style::fill_rrect(c, r, 6.0, with_alpha(tint, 46));
    style::stroke_rrect(c, r, 6.0, 1.0, with_alpha(tint, 170));
    c.text_in(r, label, FONT_H, 0, Align::Center, tint);
}

fn draw_hp(c: &mut Canvas, r: Rect, p: usize, st: &HudStatus, state: &HudState, now_ms: u64) {
    let n = hp_cells(st.hp).min(HP_CELLS);
    let tone = hp_tone(n);
    let (top, bot) = tone_colors(tone);
    let k = if tone == HpTone::Red && n > 0 { breath(now_ms) } else { 1.0 };
    for i in 0..HP_CELLS {
        let cell = Rect::new(r.x + i as i32 * (CELL_W + CELL_GAP), r.y, CELL_W, CELL_H);
        // P2 鏡射：血條由外向內遞減，所以第 0 格在最外側（靠右）。
        let cell = if p == 1 { Rect { x: r.right() - (cell.x - r.x) - CELL_W, ..cell } } else { cell };
        style::fill_rrect(c, cell, 3.0, [255, 255, 255, 16]);
        style::stroke_rrect(c, cell, 3.0, 1.0, [255, 255, 255, 34]);
        if i < n {
            let dim = |col: Rgba| mix(color::BG, col, k);
            style::fill_rrect_vgrad(c, cell, 3.0, dim(top), dim(bot));
            // 上緣高光。
            style::fill_rrect(c, Rect::new(cell.x + 2, cell.y + 1, cell.w - 4, 2), 1.0, [255, 255, 255, (70.0 * k) as u8]);
        } else if let Some((f, prev)) = state.fading(p, i, now_ms) {
            let (t, b) = tone_colors(hp_tone(prev));
            style::fill_rrect_vgrad(c, cell, 3.0, fade(t, f), fade(b, f));
        }
    }
}

fn draw_weapon(c: &mut Canvas, r: Rect, weapon: u16, icons: &HudIcons) {
    match weapon_slot(weapon) {
        None => {
            style::fill_rrect(c, r, 7.0, [255, 255, 255, 14]);
            style::stroke_rrect(c, r, 7.0, 1.0, [143, 163, 194, 70]);
            // 灰色空槽中央一條短橫線。
            style::fill_rrect(c, Rect::new(r.x + r.w / 2 - 5, r.y + r.h / 2 - 1, 10, 2), 1.0, [143, 163, 194, 90]);
        }
        Some(slot) => {
            let tint = weapon_tint(slot);
            style::fill_rrect(c, r, 7.0, with_alpha(mix(color::SURFACE, tint, 0.18), 255));
            style::stroke_rrect(c, r, 7.0, 1.0, with_alpha(tint, 150));
            match &icons.balls[slot] {
                Some(img) => draw_scaled(c, img, r.inset(2)),
                None => {
                    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
                    style::fill_circle(c, cx, cy, r.w as f32 / 2.0 - 4.0, tint);
                    let letter = WEAPONS[slot].1.to_string();
                    c.text_in(r, &letter, FONT_H, 0, Align::Center, color::BG);
                }
            }
        }
    }
}

fn draw_pods(c: &mut Canvas, r: Rect, pods: u8) {
    for i in 0..2i32 {
        let cx = (r.x + POD_R + i * (2 * POD_R + 6)) as f32;
        let cy = (r.y + POD_R) as f32;
        if (i as u8) < pods {
            style::fill_circle(c, cx, cy, POD_R as f32 + 3.0, with_alpha(color::ON, 40));
            style::fill_circle(c, cx, cy, POD_R as f32, lighten(color::ON, 0.1));
        } else {
            style::fill_circle(c, cx, cy, POD_R as f32, [255, 255, 255, 14]);
            style::stroke_rrect(c, Rect::new(cx as i32 - POD_R, cy as i32 - POD_R, POD_R * 2, POD_R * 2), POD_R as f32, 1.0, [143, 163, 194, 80]);
        }
    }
}

fn draw_charge(c: &mut Canvas, r: Rect, charge: u16) {
    style::fill_rrect(c, r, 6.0, [255, 255, 255, 18]);
    style::stroke_rrect(c, r, 6.0, 1.0, [255, 255, 255, 40]);
    let f = (charge.min(CHARGE_MAX) as f32 / CHARGE_MAX as f32).clamp(0.0, 1.0);
    let w = ((r.w - 2) as f32 * f).round() as i32;
    if w > 0 {
        let tone = if f >= 1.0 { color::WARN } else if f >= 2.0 / 3.0 { color::ON } else { color::PRIMARY };
        style::fill_rrect_vgrad(c, Rect::new(r.x + 1, r.y + 1, w, r.h - 2), 5.0, lighten(tone, 0.25), mix(tone, color::BG, 0.25));
    }
    // 三級門檻的刻度：三分之一與三分之二。
    for k in 1..3 {
        let x = r.x + r.w * k / 3;
        c.rect(Rect::new(x, r.y + 2, 1, r.h - 4), [255, 255, 255, 120]);
    }
}

fn draw_counter(c: &mut Canvas, r: Rect, v: u16) {
    // 等寬數字：每位數放在固定寬的格子正中央。
    let s = format!("{:05}", v);
    let f = font::builtin();
    for (i, ch) in s.chars().enumerate() {
        let cell = Rect::new(r.x + i as i32 * DIGIT_W, r.y, DIGIT_W, r.h);
        let t = ch.to_string();
        let tw = f.text_width(&t, FONT_H);
        c.text(cell.x + (cell.w - tw) / 2, cell.y, &t, FONT_H, color::TEXT2);
    }
}

/// 畫整條 HUD，回傳 1280×40 的 RGBA（不透明）。
pub fn render(state: &HudState, players: &[HudStatus; 2], flags: &HudFlags, icons: &HudIcons, now_ms: u64) -> Vec<u8> {
    let mut c = Canvas::new(W, H, color::BG);
    draw_plate(&mut c);
    let active = [players[0].active, players[1].active];
    for (e, r) in layout(active, flags) {
        match e {
            Elem::Tag(p) => draw_tag_chip(&mut c, r, if p == 0 { text::HUD_P1 } else { text::HUD_P2 }, player_tint(p)),
            Elem::Bar(p) => draw_hp(&mut c, r, p, &players[p], state, now_ms),
            Elem::Weapon(p) => draw_weapon(&mut c, r, players[p].weapon, icons),
            Elem::Pods(p) => draw_pods(&mut c, r, players[p].pods),
            Elem::Charge => {
                // 充能條顯示有玩家在場時最高的計數。
                let ch = players.iter().filter(|s| s.active).map(|s| s.charge).max().unwrap_or(0);
                draw_charge(&mut c, r, ch);
            }
            Elem::Counter => draw_counter(&mut c, r, flags.counter.unwrap_or(0)),
            Elem::Rules => draw_tag_chip(&mut c, r, text::HUD_RULES, color::WARN),
            Elem::God => draw_tag_chip(&mut c, r, text::HUD_GOD, color::PRIMARY),
        }
    }
    c.px
}

/// 把 HUD 蓋到 HD 畫面（1280×800 RGBA）的底部 40 列。
pub fn overlay(frame: &mut [u8], hud: &[u8]) {
    let off = (Y0 * W * 4) as usize;
    if frame.len() >= off + hud.len() && hud.len() == (W * H * 4) as usize {
        frame[off..off + hud.len()].copy_from_slice(hud);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(active: bool, hp: u16) -> HudStatus {
        HudStatus { active, hp, weapon: 0, pods: 0, charge: 0 }
    }

    fn px(buf: &[u8], x: i32, y: i32) -> Rgba {
        let o = ((y * W + x) * 4) as usize;
        [buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]
    }

    fn solo(hp: u16) -> [HudStatus; 2] {
        [st(true, hp), st(false, 0xFF)]
    }

    fn cell_rect(p: usize, i: usize, active: [bool; 2]) -> Rect {
        let bar = layout(active, &HudFlags::default()).into_iter().find(|(e, _)| *e == Elem::Bar(p)).unwrap().1;
        let x = if p == 0 { bar.x + i as i32 * (CELL_W + CELL_GAP) } else { bar.right() - i as i32 * (CELL_W + CELL_GAP) - CELL_W };
        Rect::new(x, bar.y, CELL_W, CELL_H)
    }

    fn center(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    #[test]
    fn hp_word_bits_map_to_cells() {
        let words = [0xFFu16, 0x7F, 0x3F, 0x1F, 0x0F, 0x07, 0x03, 0x01, 0x00];
        for (i, w) in words.iter().enumerate() {
            assert_eq!(hp_cells(*w), 8 - i, "hp={w:#x}");
        }
        assert_eq!(hp_cells(0xFFFF), 8, "高位元組不算");
        assert_eq!(hp_tone(8), HpTone::Healthy);
        assert_eq!(hp_tone(4), HpTone::Healthy);
        assert_eq!(hp_tone(3), HpTone::Amber);
        assert_eq!(hp_tone(2), HpTone::Amber);
        assert_eq!(hp_tone(1), HpTone::Red);
    }

    #[test]
    fn weapon_letters_map_to_my_items_42_to_45() {
        assert_eq!(weapon_slot(b'L' as u16), Some(0));
        assert_eq!(weapon_slot(b'B' as u16), Some(1));
        assert_eq!(weapon_slot(b'K' as u16), Some(2));
        assert_eq!(weapon_slot(b'S' as u16), Some(3));
        assert_eq!(weapon_slot(0), None);
        assert_eq!(WEAPON_MY_INDEX, [42, 43, 44, 45]);
    }

    #[test]
    fn layout_stays_inside_the_bar_and_never_overlaps() {
        for mask in 0..64u32 {
            let active = [mask & 1 != 0 || mask & 2 == 0, mask & 2 != 0];
            let flags = HudFlags { rules: mask & 4 != 0, god: mask & 8 != 0, charge_bar: mask & 16 != 0, counter: (mask & 32 != 0).then_some(12345) };
            let l = layout(active, &flags);
            for (i, (e, r)) in l.iter().enumerate() {
                assert!(r.x >= 0 && r.right() <= W && r.y >= 0 && r.bottom() <= H, "{e:?} {r:?} 超出 HUD（mask {mask}）");
                for (o, orc) in &l[..i] {
                    let x = r.intersect(orc);
                    assert!(x.w == 0 || x.h == 0, "{e:?} {r:?} 與 {o:?} {orc:?} 重疊（mask {mask}）");
                }
            }
        }
    }

    #[test]
    fn player_two_is_mirrored_on_the_right_and_absent_players_are_not_laid_out() {
        let both = layout([true, true], &HudFlags::default());
        let get = |e: Elem| both.iter().find(|(x, _)| *x == e).unwrap().1;
        assert!(get(Elem::Tag(0)).x < get(Elem::Bar(0)).x && get(Elem::Bar(0)).x < get(Elem::Weapon(0)).x && get(Elem::Weapon(0)).x < get(Elem::Pods(0)).x, "P1 由左往右");
        assert!(get(Elem::Tag(1)).x > get(Elem::Bar(1)).x && get(Elem::Bar(1)).x > get(Elem::Weapon(1)).x && get(Elem::Weapon(1)).x > get(Elem::Pods(1)).x, "P2 鏡射由右往左");
        assert_eq!(get(Elem::Tag(0)).x, M);
        assert_eq!(get(Elem::Tag(1)).right(), W - M);
        let solo_l = layout([true, false], &HudFlags::default());
        assert!(!solo_l.iter().any(|(e, _)| matches!(e, Elem::Tag(1) | Elem::Bar(1) | Elem::Weapon(1) | Elem::Pods(1))));
        // 標籤在右端；有 P2 時讓出 P2 的區塊。
        let flags = HudFlags { rules: true, god: true, ..HudFlags::default() };
        let t1 = layout([true, false], &flags);
        let t2 = layout([true, true], &flags);
        let right = |l: &[(Elem, Rect)]| l.iter().find(|(e, _)| *e == Elem::God).unwrap().1.right();
        assert_eq!(right(&t1), W - M);
        assert!(right(&t2) < W - M - BLOCK_W);
    }

    #[test]
    fn hud_is_one_opaque_1280_by_40_plate_with_a_highlight_line() {
        let r = render(&HudState::new(), &solo(0xFF), &HudFlags::default(), &HudIcons::default(), 0);
        assert_eq!(r.len(), (W * H * 4) as usize);
        assert!(r.chunks_exact(4).all(|p| p[3] == 255));
        // 漸層：上方比下方亮；上緣亮線比第二列亮。
        assert!(px(&r, 900, 20)[2] > px(&r, 900, 39)[2]);
        assert!(px(&r, 900, 0)[0] > px(&r, 900, 1)[0] + 20);
    }

    #[test]
    fn full_health_is_green_three_cells_amber_one_cell_red() {
        let hue = |hp: u16| {
            let r = render(&HudState::new(), &solo(hp), &HudFlags::default(), &HudIcons::default(), 0);
            let (x, y) = center(cell_rect(0, 0, [true, false]));
            px(&r, x, y)
        };
        let g = hue(0xFF);
        assert!(g[1] > g[0] + 40 && g[1] > 150, "滿血青綠：{g:?}");
        let a = hue(0x07);
        assert!(a[0] > 200 && a[0] > a[2] + 80 && a[1] > a[2], "剩 3 格琥珀：{a:?}");
        let r = hue(0x01);
        assert!(r[0] > r[1] + 60 && r[0] > r[2] + 40, "剩 1 格紅：{r:?}");
    }

    #[test]
    fn lit_cells_equal_the_lit_bits_and_the_rest_are_dark() {
        for hp in [0xFFu16, 0x7F, 0x1F, 0x03, 0x01, 0x00] {
            let r = render(&HudState::new(), &solo(hp), &HudFlags::default(), &HudIcons::default(), 0);
            let lit = (0..HP_CELLS)
                .filter(|&i| {
                    let (x, y) = center(cell_rect(0, i, [true, false]));
                    let p = px(&r, x, y);
                    p[0] as u32 + p[1] as u32 + p[2] as u32 > 250
                })
                .count();
            assert_eq!(lit, hp_cells(hp), "hp={hp:#x}");
        }
    }

    #[test]
    fn player_two_bar_fills_from_the_outside_in() {
        let players = [st(true, 0xFF), st(true, 0x07)];
        let r = render(&HudState::new(), &players, &HudFlags::default(), &HudIcons::default(), 0);
        let active = [true, true];
        let lit: Vec<bool> = (0..HP_CELLS)
            .map(|i| {
                let (x, y) = center(cell_rect(1, i, active));
                let p = px(&r, x, y);
                p[0] as u32 + p[1] as u32 + p[2] as u32 > 250
            })
            .collect();
        assert_eq!(lit, vec![true, true, true, false, false, false, false, false]);
    }

    #[test]
    fn a_lost_cell_fades_out_over_200_ms() {
        let mut s = HudState::new();
        let flags = HudFlags::default();
        s.update(1000, &solo(0xFF));
        s.update(2000, &solo(0x7F));
        let (x, y) = center(cell_rect(0, 7, [true, false]));
        let at = |t: u64| {
            let r = render(&s, &solo(0x7F), &flags, &HudIcons::default(), t);
            let p = px(&r, x, y);
            p[0] as u32 + p[1] as u32 + p[2] as u32
        };
        let (t0, t100, t199, t200, t400) = (at(2000), at(2100), at(2199), at(2200), at(2400));
        assert!(t0 > 250, "剛熄滅還很亮：{t0}");
        assert!(t0 > t100 && t100 > t199, "逐漸變暗：{t0} {t100} {t199}");
        assert!(t200 < 150 && t400 == t200, "200 毫秒後只剩空格：{t200} {t400}");
        // 補血（新局）時清掉淡出。
        s.update(2050, &solo(0xFF));
        let r = render(&s, &solo(0xFF), &flags, &HudIcons::default(), 2100);
        assert!(px(&r, x, y).iter().take(3).map(|v| *v as u32).sum::<u32>() > 250, "亮格回來");
    }

    #[test]
    fn last_cell_breathes_once_per_second() {
        let flags = HudFlags::default();
        let (x, y) = center(cell_rect(0, 0, [true, false]));
        let at = |t: u64| {
            let r = render(&HudState::new(), &solo(0x01), &flags, &HudIcons::default(), t);
            let p = px(&r, x, y);
            p[0] as u32 + p[1] as u32 + p[2] as u32
        };
        assert_eq!(at(0), at(1000), "週期 1 秒");
        assert_eq!(at(130), at(1130));
        assert!(at(0) > at(500) + 40, "半週期最暗：{} {}", at(0), at(500));
        assert!((breath(0) - 1.0).abs() < 1e-6 && breath(500) < 0.5);
        // 多於 1 格不呼吸。
        let full = |t: u64| render(&HudState::new(), &solo(0xFF), &flags, &HudIcons::default(), t);
        assert_eq!(full(0), full(500));
    }

    #[test]
    fn empty_weapon_slot_is_grey_and_each_weapon_changes_it() {
        let flags = HudFlags::default();
        let slot = layout([true, false], &flags).into_iter().find(|(e, _)| *e == Elem::Weapon(0)).unwrap().1;
        let draw = |w: u16, icons: &HudIcons| {
            let mut s = solo(0xFF);
            s[0].weapon = w;
            render(&HudState::new(), &s, &flags, icons, 0)
        };
        let empty = draw(0, &HudIcons::default());
        let region = |buf: &[u8]| -> Vec<u8> {
            let mut v = Vec::new();
            for y in slot.y..slot.bottom() {
                for x in slot.x..slot.right() {
                    v.extend_from_slice(&px(buf, x, y));
                }
            }
            v
        };
        let (cx, cy) = center(slot);
        let e = px(&empty, cx - 9, cy - 9);
        assert!(e[0].abs_diff(e[1]) < 20 && e[1].abs_diff(e[2]) < 30, "空槽是灰藍色：{e:?}");
        let mut seen = vec![region(&empty)];
        for (w, _) in WEAPONS {
            let r = draw(w, &HudIcons::default());
            let reg = region(&r);
            assert!(!seen.contains(&reg), "武器 {w} 的圖和別的一樣");
            seen.push(reg);
        }
        // 有 HD 圖示時用圖示：整塊紅色的球。
        let ball = IconImg { w: 96, h: 68, rgba: [250, 20, 20, 255].repeat(96 * 68) };
        let icons = HudIcons { balls: [Some(ball.clone()), Some(ball.clone()), Some(ball.clone()), Some(ball)] };
        let r = draw(b'B' as u16, &icons);
        let p = px(&r, cx, cy);
        assert!(p[0] > 200 && p[1] < 60, "中央是圖示的紅色：{p:?}");
        // 圖示依長寬比縮進槽內，槽的邊角不被蓋掉。
        let corner = px(&r, slot.x + 1, slot.y + 1);
        assert!(corner[0] < 200, "{corner:?}");
    }

    #[test]
    fn pod_lamps_light_from_the_inside() {
        let flags = HudFlags::default();
        let pods = layout([true, false], &flags).into_iter().find(|(e, _)| *e == Elem::Pods(0)).unwrap().1;
        let lamp = |n: u8, i: i32| {
            let mut s = solo(0xFF);
            s[0].pods = n;
            let r = render(&HudState::new(), &s, &flags, &HudIcons::default(), 0);
            let p = px(&r, pods.x + POD_R + i * (2 * POD_R + 6), pods.y + POD_R);
            p[1] as u32
        };
        assert!(lamp(0, 0) < 120 && lamp(0, 1) < 120);
        assert!(lamp(1, 0) > 200 && lamp(1, 1) < 120);
        assert!(lamp(2, 0) > 200 && lamp(2, 1) > 200);
    }

    #[test]
    fn charge_bar_and_labels_only_when_asked() {
        let base = render(&HudState::new(), &solo(0xFF), &HudFlags::default(), &HudIcons::default(), 0);
        let mut s = solo(0xFF);
        s[0].charge = CHARGE_MAX / 2;
        let with_bar = render(&HudState::new(), &s, &HudFlags { charge_bar: true, ..HudFlags::default() }, &HudIcons::default(), 0);
        assert_ne!(base, with_bar);
        let bar = layout([true, false], &HudFlags { charge_bar: true, ..HudFlags::default() }).into_iter().find(|(e, _)| *e == Elem::Charge).unwrap().1;
        // 半滿：左半有填色，右半沒有；刻度在三分之一處。
        let l = px(&with_bar, bar.x + bar.w / 4, bar.y + bar.h / 2);
        let rr = px(&with_bar, bar.x + bar.w * 3 / 4, bar.y + bar.h / 2);
        assert!(l[2] > rr[2] + 60, "{l:?} {rr:?}");
        let tick = px(&with_bar, bar.x + bar.w / 3, bar.y + 4);
        assert!(tick[0] > 120, "刻度：{tick:?}");
        // 沒開選項時中央什麼都沒有。
        assert_eq!(px(&base, 640, 20), px(&base, 700, 20));
        let tags = render(&HudState::new(), &solo(0xFF), &HudFlags { rules: true, god: true, ..HudFlags::default() }, &HudIcons::default(), 0);
        assert_ne!(base, tags);
        let counter = render(&HudState::new(), &solo(0xFF), &HudFlags { counter: Some(123), ..HudFlags::default() }, &HudIcons::default(), 0);
        assert_ne!(base, counter);
        let other = render(&HudState::new(), &solo(0xFF), &HudFlags { counter: Some(124), ..HudFlags::default() }, &HudIcons::default(), 0);
        assert_ne!(counter, other);
    }

    #[test]
    fn counter_digits_sit_in_fixed_width_cells() {
        // 等寬：1 與 8 佔的格寬相同，墨跡落在各自的格內。
        for v in [11111u16, 58888] {
            let r = render(&HudState::new(), &solo(0xFF), &HudFlags { counter: Some(v), ..HudFlags::default() }, &HudIcons::default(), 0);
            let c = layout([true, false], &HudFlags { counter: Some(v), ..HudFlags::default() }).into_iter().find(|(e, _)| *e == Elem::Counter).unwrap().1;
            for i in 0..DIGITS {
                let cell = Rect::new(c.x + i * DIGIT_W, c.y, DIGIT_W, c.h);
                let ink = (cell.y..cell.bottom()).flat_map(|y| (cell.x..cell.right()).map(move |x| (x, y))).filter(|&(x, y)| px(&r, x, y)[0] > 60).count();
                assert!(ink > 6, "{v} 第 {i} 位沒有字");
                // 格與格之間不貼字：每格最左一欄與最右一欄不該有亮點（字寬小於格寬）。
            }
        }
    }

    #[test]
    fn overlay_replaces_only_the_bottom_forty_rows() {
        let mut frame = vec![7u8; (W * 800 * 4) as usize];
        let hud = render(&HudState::new(), &solo(0xFF), &HudFlags::default(), &HudIcons::default(), 0);
        overlay(&mut frame, &hud);
        let o = (Y0 * W * 4) as usize;
        assert!(frame[..o].iter().all(|b| *b == 7), "上面 760 列不動");
        assert_eq!(&frame[o..], &hud[..]);
        // 尺寸不對就不動。
        let mut small = vec![1u8; 100];
        overlay(&mut small, &hud);
        assert!(small.iter().all(|b| *b == 1));
    }

    #[test]
    fn every_hud_string_has_a_bitmap() {
        let f = font::builtin();
        for s in [text::HUD_P1, text::HUD_P2, text::HUD_RULES, text::HUD_GOD] {
            assert!(f.missing(s).is_empty(), "{s}");
        }
    }
}
