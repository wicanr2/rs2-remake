//! 打擊回饋（規格 005「HD 介面」「打擊回饋」）：畫面震動、命中光暈與疊在原版聲音上的音效。
//! 全部只看繪製命令與玩家狀態，只改輸出位置、畫面像素與音訊，不改核心、不碰模擬。不依賴 SDL 子系統，時間由呼叫端以毫秒傳入。
//!
//! 事件怎麼判斷：特效池（`Pool::Effect`）的槽被生成函式配置時，世代加 1（`ObjId::generation`）。
//! 一個世代第一次出現在繪製命令裡就是一個新特效，看它第一格畫的圖號（`MY.GO`，`docs/re/11-entities-combat.md` 8.1 節）：
//!
//! | 圖號 | 腳本 | 意義 | 回饋 |
//! |---|---|---|---|
//! | 2 | 火花（`S`、`>`、`<`）的第一格 | 玩家子彈命中、實體被打爆、玩家被命中 | 命中點 80 毫秒光暈 |
//! | 27 | 大爆炸（`B`）的第一格 | 大爆炸 | 震動 180 毫秒、爆炸音效 |
//! | 3 | 兩團爆炸（`V`）的第一格 | 大爆炸、頭目爆炸 | 同上 |
//!
//! 玩家死亡的爆炸（`G`）與尾跡（`E`）第一格同為圖 24，單看圖號分不出來，所以玩家死亡改看狀態：血量的亮格數從正數變成 0。
//! 玩家開火：`Pool::PlayerBullet` 的新世代。

use rs2_audio::SfxKind;
use rs2_game::{DrawCmd, Pool};
use std::collections::HashSet;

/// 火花（右、左）的第一格。
pub const IMG_SPARK: u16 = 2;
/// `V` 兩團爆炸的第一格。
pub const IMG_BLAST: u16 = 3;
/// `B` 大爆炸的第一格。
pub const IMG_BIG: u16 = 27;

pub const SHAKE_MS: u64 = 180;
/// 震動的最大振幅（HD 像素）。
pub const SHAKE_AMP: f32 = 6.0;
pub const GLOW_MS: u64 = 80;
/// 光暈半徑（HD 像素）。
pub const GLOW_RADIUS: i32 = 44;
pub const MAX_GLOWS: usize = 24;
/// 同一種音效的最短間隔，連射和連爆不要糊成一片。
pub const SFX_GAP_MS: u64 = 55;

/// 從繪製命令或玩家狀態判斷出的事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxEvent {
    /// 命中火花，座標是 HD 像素（圖的中心）。
    Hit { x: i32, y: i32 },
    /// 大爆炸或頭目爆炸。
    Boom,
    /// 玩家死亡（血量亮格數變成 0）。
    PlayerDown,
    /// 玩家開火。
    Fire,
}

/// 掃繪製命令找新生成的特效與玩家子彈。
#[derive(Clone, Debug, Default)]
pub struct Scanner {
    prev: HashSet<(Pool, u8, u32)>,
    primed: bool,
}

impl Scanner {
    pub fn new() -> Scanner {
        Scanner::default()
    }

    /// 掃一個邏輯幀的命令。`size_of(圖號)` 回傳 `MY` 圖的邏輯像素寬高，用來求命中點的中心。
    /// 空的命令串（暫停中）不是一幀，不改狀態。第一次掃只建立基準，不回報事件（載入時場上已有的物件不算新的）。
    pub fn scan(&mut self, draws: &[DrawCmd], size_of: &dyn Fn(u16) -> (i32, i32)) -> Vec<FxEvent> {
        if draws.is_empty() {
            return Vec::new();
        }
        let mut cur: HashSet<(Pool, u8, u32)> = HashSet::new();
        let mut out = Vec::new();
        for d in draws {
            let DrawCmd::Blit(b) = d else { continue };
            let Some(o) = b.obj else { continue };
            if !matches!(o.pool, Pool::Effect | Pool::PlayerBullet) {
                continue;
            }
            let key = (o.pool, o.slot, o.generation);
            if !cur.insert(key) || self.prev.contains(&key) {
                continue;
            }
            match o.pool {
                Pool::PlayerBullet => out.push(FxEvent::Fire),
                _ => match b.index {
                    IMG_SPARK => {
                        let (w, h) = size_of(b.index);
                        let (x, y) = (b.x as i16 as i32 * 8 + w / 2, b.y as i16 as i32 + h / 2);
                        out.push(FxEvent::Hit { x: x * 4, y: y * 4 });
                    }
                    IMG_BLAST | IMG_BIG => out.push(FxEvent::Boom),
                    _ => {}
                },
            }
        }
        self.prev = cur;
        if !std::mem::replace(&mut self.primed, true) {
            out.clear();
        }
        out
    }
}

/// 一個光暈。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Glow {
    x: i32,
    y: i32,
    t0: u64,
}

/// 回饋的執行狀態：還在進行的震動與光暈，以及等著送去音訊端的音效。
#[derive(Clone, Debug, Default)]
pub struct Feedback {
    shake_t0: Option<u64>,
    shake_seed: u32,
    glows: Vec<Glow>,
    sfx: Vec<SfxKind>,
    last_sfx: [Option<u64>; 2],
}

fn sfx_slot(k: SfxKind) -> usize {
    match k {
        SfxKind::Fire => 0,
        SfxKind::Boom => 1,
    }
}

impl Feedback {
    pub fn new() -> Feedback {
        Feedback::default()
    }

    /// 清掉進行中的回饋（關掉選項、離開 HD）。
    pub fn clear(&mut self) {
        *self = Feedback::default();
    }

    fn push_sfx(&mut self, now_ms: u64, k: SfxKind) {
        let slot = sfx_slot(k);
        if self.last_sfx[slot].is_some_and(|t| now_ms.saturating_sub(t) < SFX_GAP_MS) {
            return;
        }
        self.last_sfx[slot] = Some(now_ms);
        self.sfx.push(k);
    }

    /// 收新事件。`now_ms` 是事件發生的時間。
    pub fn ingest(&mut self, now_ms: u64, events: &[FxEvent]) {
        for e in events {
            match *e {
                FxEvent::Hit { x, y } => {
                    if self.glows.len() >= MAX_GLOWS {
                        self.glows.remove(0);
                    }
                    self.glows.push(Glow { x, y, t0: now_ms });
                }
                FxEvent::Boom | FxEvent::PlayerDown => {
                    self.shake_t0 = Some(now_ms);
                    self.shake_seed = self.shake_seed.wrapping_add(0x9E37_79B1);
                    self.push_sfx(now_ms, SfxKind::Boom);
                }
                FxEvent::Fire => self.push_sfx(now_ms, SfxKind::Fire),
            }
        }
    }

    /// 現在的畫面位移（HD 像素）。沒有震動是 (0, 0)。
    pub fn shake_offset(&self, now_ms: u64) -> (i32, i32) {
        match self.shake_t0 {
            Some(t0) => shake_at(now_ms.saturating_sub(t0), self.shake_seed),
            None => (0, 0),
        }
    }

    /// 取走等著播的音效。
    pub fn take_sfx(&mut self) -> Vec<SfxKind> {
        std::mem::take(&mut self.sfx)
    }

    /// 還在進行的光暈數。
    pub fn glow_count(&self, now_ms: u64) -> usize {
        self.glows.iter().filter(|g| now_ms.saturating_sub(g.t0) < GLOW_MS).count()
    }

    /// 把光暈加亮疊到 `w × h` 的 RGBA 畫面上，並丟掉過期的。
    pub fn draw_glows(&mut self, frame: &mut [u8], w: usize, h: usize, now_ms: u64) {
        self.glows.retain(|g| now_ms.saturating_sub(g.t0) < GLOW_MS);
        if frame.len() < w * h * 4 {
            return;
        }
        for g in &self.glows {
            let life = 1.0 - now_ms.saturating_sub(g.t0) as f32 / GLOW_MS as f32;
            let r = GLOW_RADIUS;
            for y in (g.y - r).max(0)..(g.y + r + 1).min(h as i32) {
                for x in (g.x - r).max(0)..(g.x + r + 1).min(w as i32) {
                    let d = (((x - g.x).pow(2) + (y - g.y).pow(2)) as f32).sqrt() / r as f32;
                    if d >= 1.0 {
                        continue;
                    }
                    let a = (1.0 - d) * (1.0 - d) * life * 0.9;
                    let o = (y as usize * w + x as usize) * 4;
                    for (k, tint) in [255.0f32, 238.0, 196.0].into_iter().enumerate() {
                        frame[o + k] = (frame[o + k] as f32 + tint * a).min(255.0) as u8;
                    }
                }
            }
        }
    }
}

/// 震動 `dt_ms` 毫秒後的位移（HD 像素）：振幅從 `SHAKE_AMP` 平方衰減到 0，兩軸頻率不同。`seed` 決定相位。
pub fn shake_at(dt_ms: u64, seed: u32) -> (i32, i32) {
    if dt_ms >= SHAKE_MS {
        return (0, 0);
    }
    let t = dt_ms as f32 / 1000.0;
    let k = 1.0 - dt_ms as f32 / SHAKE_MS as f32;
    let amp = SHAKE_AMP * k * k;
    let ph = (seed >> 8) as f32 / (1u32 << 24) as f32 * std::f32::consts::TAU;
    let x = amp * (std::f32::consts::TAU * 24.0 * t + ph).cos();
    let y = amp * 0.75 * (std::f32::consts::TAU * 31.0 * t + ph * 1.7 + 1.1).sin();
    (x.round() as i32, y.round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rs2_game::{BegGroup, BlitCmd, ObjId, Op, ShtGroup};

    fn blit(pool: Pool, slot: u8, generation: u32, index: u16, x: u16, y: u16) -> DrawCmd {
        DrawCmd::Blit(BlitCmd {
            op: Op::MyOverlayEffect,
            x,
            y,
            index,
            page: 0,
            scratch: 0,
            src_x: 0,
            src_y: 0,
            shift: 0,
            cx: 0,
            dx: 0,
            text_x: 0,
            text_y: 0,
            sht_group: ShtGroup::Sht1,
            mask_group: ShtGroup::Sht1,
            beg_group: BegGroup::Beg1,
            stride_reg: None,
            obj: Some(ObjId { pool, slot, generation }),
        })
    }

    fn size(_: u16) -> (i32, i32) {
        (16, 16)
    }

    #[test]
    fn first_scan_only_primes_the_baseline() {
        let mut s = Scanner::new();
        let f = vec![blit(Pool::Effect, 0, 1, IMG_BIG, 10, 10), blit(Pool::PlayerBullet, 0, 1, 5, 5, 5)];
        assert!(s.scan(&f, &size).is_empty(), "載入時場上已有的物件不算新的");
        assert!(s.scan(&f, &size).is_empty(), "同一個世代續畫不算新的");
    }

    #[test]
    fn a_new_effect_generation_is_classified_by_its_first_image() {
        let mut s = Scanner::new();
        s.scan(&[blit(Pool::Entity, 0, 1, 9, 0, 0)], &size);
        let f = vec![
            blit(Pool::Effect, 0, 1, IMG_SPARK, 10, 100),
            blit(Pool::Effect, 0, 1, IMG_SPARK, 10, 100), // 同一物件的第二個 blit（遮罩加四平面）
            blit(Pool::Effect, 1, 1, IMG_BIG, 3, 3),
            blit(Pool::Effect, 2, 1, IMG_BLAST, 4, 4),
            blit(Pool::Effect, 3, 1, 24, 5, 5),
            blit(Pool::PlayerBullet, 0, 1, 7, 20, 20),
            blit(Pool::Entity, 0, 1, 9, 0, 0),
        ];
        let ev = s.scan(&f, &size);
        assert_eq!(ev, vec![FxEvent::Hit { x: (10 * 8 + 8) * 4, y: (100 + 8) * 4 }, FxEvent::Boom, FxEvent::Boom, FxEvent::Fire], "尾跡（圖 24）與實體不產生事件");
        // 下一幀同樣的物件續畫，沒有新事件；槽重用（世代加 1）才算新的。
        assert!(s.scan(&f, &size).is_empty());
        let f2 = vec![blit(Pool::Effect, 0, 2, IMG_SPARK, 50, 60)];
        assert_eq!(s.scan(&f2, &size), vec![FxEvent::Hit { x: (50 * 8 + 8) * 4, y: (60 + 8) * 4 }], "同槽新世代");
    }

    #[test]
    fn an_empty_step_does_not_reset_the_baseline() {
        let mut s = Scanner::new();
        let f = vec![blit(Pool::Effect, 0, 1, IMG_BIG, 1, 1)];
        s.scan(&[], &size);
        s.scan(&f, &size);
        assert!(s.scan(&[], &size).is_empty());
        assert!(s.scan(&f, &size).is_empty(), "暫停步之後同一個物件仍不是新的");
    }

    #[test]
    fn negative_coordinates_wrap_as_signed_words() {
        let mut s = Scanner::new();
        s.scan(&[blit(Pool::Entity, 0, 0, 1, 0, 0)], &size);
        let ev = s.scan(&[blit(Pool::Effect, 0, 5, IMG_SPARK, 0xFFFF, 0xFFF0)], &size);
        assert_eq!(ev, vec![FxEvent::Hit { x: (-8 + 8) * 4, y: (-16 + 8) * 4 }]);
    }

    #[test]
    fn shake_lasts_180_ms_never_exceeds_6_px_and_decays() {
        for seed in [0u32, 1, 0x9E37_79B1, 0xFFFF_FFFF] {
            let mut peak_first = 0;
            let mut peak_last = 0;
            for dt in 0..SHAKE_MS {
                let (x, y) = shake_at(dt, seed);
                assert!(x.abs() <= 6 && y.abs() <= 6, "{dt} ms：({x},{y})");
                if dt < 40 {
                    peak_first = peak_first.max(x.abs()).max(y.abs());
                }
                if dt >= SHAKE_MS - 40 {
                    peak_last = peak_last.max(x.abs()).max(y.abs());
                }
            }
            assert!(peak_first >= 3, "開頭要有明顯的震動 seed {seed}：{peak_first}");
            assert!(peak_last < peak_first, "越震越小");
            assert_eq!(shake_at(SHAKE_MS, seed), (0, 0));
            assert_eq!(shake_at(10_000, seed), (0, 0));
        }
        assert_eq!(shake_at(37, 5), shake_at(37, 5), "同樣輸入同樣位移");
    }

    #[test]
    fn events_start_shake_glow_and_sound_but_nothing_else() {
        let mut f = Feedback::new();
        assert_eq!(f.shake_offset(1000), (0, 0));
        f.ingest(1000, &[FxEvent::Hit { x: 100, y: 100 }, FxEvent::Boom, FxEvent::Fire]);
        assert_eq!(f.glow_count(1000), 1);
        assert!((0..SHAKE_MS).any(|d| f.shake_offset(1000 + d) != (0, 0)));
        assert_eq!(f.shake_offset(1000 + SHAKE_MS), (0, 0));
        assert_eq!(f.take_sfx(), vec![SfxKind::Boom, SfxKind::Fire]);
        assert!(f.take_sfx().is_empty());
        assert_eq!(f.glow_count(1000 + GLOW_MS), 0, "光暈 80 毫秒");
        f.ingest(2000, &[FxEvent::PlayerDown]);
        assert_eq!(f.take_sfx(), vec![SfxKind::Boom]);
        assert!((0..SHAKE_MS).any(|d| f.shake_offset(2000 + d) != (0, 0)));
    }

    #[test]
    fn sounds_of_one_kind_are_spaced_apart_and_glows_are_capped() {
        let mut f = Feedback::new();
        for i in 0..10 {
            f.ingest(1000 + i * 10, &[FxEvent::Fire]);
        }
        assert_eq!(f.take_sfx().len(), 2, "100 毫秒內最多兩聲（間隔 55 毫秒）");
        f.ingest(5000, &[FxEvent::Fire]);
        assert_eq!(f.take_sfx().len(), 1);
        for i in 0..100 {
            f.ingest(7000, &[FxEvent::Hit { x: i, y: i }]);
        }
        assert_eq!(f.glow_count(7000), MAX_GLOWS);
    }

    #[test]
    fn glow_brightens_around_the_hit_point_fades_and_stays_in_bounds() {
        let (w, h) = (200usize, 120usize);
        let base = vec![40u8; w * h * 4];
        let mut f = Feedback::new();
        f.ingest(0, &[FxEvent::Hit { x: 100, y: 60 }]);
        let draw = |f: &mut Feedback, t: u64| {
            let mut b = base.clone();
            f.draw_glows(&mut b, w, h, t);
            b
        };
        let c0 = draw(&mut f.clone(), 0);
        let c40 = draw(&mut f.clone(), 40);
        let px = |b: &[u8], x: usize, y: usize| b[(y * w + x) * 4] as i32;
        assert!(px(&c0, 100, 60) > 200, "中心最亮：{}", px(&c0, 100, 60));
        assert!(px(&c0, 100, 60) > px(&c0, 120, 60) && px(&c0, 120, 60) > px(&c0, 140, 60), "往外變暗");
        assert_eq!(px(&c0, 100 + GLOW_RADIUS as usize + 2, 60), 40, "半徑外不動");
        assert!(px(&c40, 100, 60) < px(&c0, 100, 60), "隨時間變暗");
        let gone = draw(&mut f, GLOW_MS);
        assert_eq!(gone, base, "80 毫秒後完全消失");
        // 光暈靠近邊緣不會寫出緩衝區。
        let mut e = Feedback::new();
        e.ingest(0, &[FxEvent::Hit { x: 2, y: 2 }, FxEvent::Hit { x: 199, y: 119 }, FxEvent::Hit { x: -30, y: 500 }]);
        let mut b = base.clone();
        e.draw_glows(&mut b, w, h, 0);
        assert_eq!(b.len(), base.len());
        // 只加亮 RGB，alpha 不動。
        assert!(b.chunks_exact(4).all(|p| p[3] == 40));
    }

    #[test]
    fn clearing_drops_everything() {
        let mut f = Feedback::new();
        f.ingest(0, &[FxEvent::Boom, FxEvent::Hit { x: 1, y: 1 }, FxEvent::Fire]);
        f.clear();
        assert_eq!(f.glow_count(0), 0);
        assert_eq!(f.shake_offset(10), (0, 0));
        assert!(f.take_sfx().is_empty());
    }
}
