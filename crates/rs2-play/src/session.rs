//! 一局遊戲：`Game` 加兩個繪製接收端，以及視窗模式與 `--headless-steps` 共用的步進與繪製路徑。
//! 不依賴 SDL，所以主迴圈以外的邏輯（模式選擇、排程）可以直接測。
//!
//! `Game::step` 回傳的 `draws` 同時餵給 `VgaSink`（補間關閉的原版圖）與 `LayerSink`（補間與 HD），
//! 兩邊都隨時是最新的，所以 F1、F3 切換不需要重播。畫面顯示的選擇只影響 `render`，不碰 `Game`。

use rs2_audio::SpkEv;
use rs2_core::{AudioCmd, DrawCmd, Entry, Exit, Fixes, Game, Input, MixFile, Pool, Start, boot_ds_with_dm1};
use rs2_formats::sha256_hex;
use rs2_gfx::speaker::speaker_events;
use rs2_gfx::{Assets, HdAssets, LayerSink, RenderOpts, VgaSink};
use rs2_trace::Inputs;

use crate::predict::{Dir, Predictor, Reach};

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
    /// 玩家機預測。只在補間開著時有作用，補間關著的畫面要逐像素等於原版。
    pub feel: bool,
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
    predictor: Predictor,
    /// 目前有真人輸入（`predict` 呼叫過）。回放與 `--headless-steps` 沒有，照一般補間畫。
    live_input: bool,
    /// 最新一幀畫了玩家機。標題與過場沒有畫，那裡按鍵不會移動任何東西。
    player_drawn: [bool; 2],
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
            predictor: Predictor::new(),
            live_input: false,
            player_drawn: [false; 2],
        };
        s.vga.show_counter = view.counter;
        s.feed(&init);
        s
    }

    /// 這一步玩家的按鍵會不會移動玩家機。
    fn playable(&self, p: usize) -> bool {
        self.player_drawn[p] && self.game.player_status(p).controllable
    }

    /// 把玩家機的邏輯位置交給預測器。空的命令串（暫停中）不改變 `player_drawn`。
    fn sync_predictor(&mut self, draws: &[DrawCmd]) {
        if !draws.is_empty() {
            self.player_drawn = [false; 2];
            for d in draws {
                if let DrawCmd::Blit(b) = d {
                    if let Some(o) = b.obj {
                        if o.pool == Pool::Player {
                            self.player_drawn[(o.slot as usize).min(1)] = true;
                        }
                    }
                }
            }
        }
        for p in 0..2 {
            let st = self.game.player_status(p);
            let pos = self.playable(p).then_some((st.x as i32 * 8, st.y as i32));
            self.predictor.on_step(p, pos);
        }
    }

    /// 過了 `dt` 秒，推進玩家機預測。`state` 是下一步時每個 set 1 碼是否按著，`step_secs` 是這一步到下一步的時間。
    /// 只影響 `render`，不碰 `Game`。
    pub fn predict(&mut self, state: &[bool; 128], dt: f32, step_secs: f32) {
        self.live_input = true;
        for p in 0..2 {
            let st = self.game.player_status(p);
            let reach = Reach { up: st.can_up, down: st.can_down, left: st.can_left, right: st.can_right };
            // 暫停中邏輯不會動，不累積。
            let dir = if self.hold { Dir::default() } else { crate::predict::dir_of(state, p) };
            self.predictor.advance(p, dir, reach, dt, step_secs);
        }
    }

    /// 玩家 1 的邏輯位置（邏輯像素）。不在畫面上時是 `None`。
    pub fn player_pos(&self) -> Option<(i32, i32)> {
        let st = self.game.player_status(0);
        self.playable(0).then_some((st.x as i32 * 8, st.y as i32))
    }

    /// 有玩家機在畫面上而且可以控制，觸控層用它決定右手兩顆鈕的作用。
    pub fn in_play(&self) -> bool {
        self.playable(0) || self.playable(1)
    }

    /// 這一幀要給繪製層的玩家機偏移。
    fn player_offsets(&self) -> [Option<(i32, i32)>; 2] {
        if self.live_input && self.view.feel && self.view.tween {
            [self.predictor.offset_hd(0), self.predictor.offset_hd(1)]
        } else {
            [None, None]
        }
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
        self.sync_predictor(&frame.draws);
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
            let player = self.player_offsets();
            let opts = RenderOpts { interpolate: self.view.tween, hd, show_counter: self.view.counter, player };
            let (img, iw, ih) = self.layer.render(alpha, opts);
            self.frame.clear();
            self.frame.extend_from_slice(img);
            (w, h) = (iw, ih);
        }
        (&self.frame, w, h)
    }
}

#[cfg(test)]
mod tests {
    //! 需要原版檔，標 `#[ignore]`，由 `tools/rust.sh play-test` 帶 `--include-ignored` 跑。

    use super::*;
    use rs2_trace::load_inputs;
    use std::path::PathBuf;

    const VIEW: ViewOpts = ViewOpts { tween: true, counter: false, hd: false, feel: true };
    const RIGHT: u8 = 0x4D;
    const LEFT: u8 = 0x4B;

    fn orig() -> PathBuf {
        PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh play-test 跑"))
    }

    fn open(begin: Begin, view: ViewOpts) -> Session {
        let dir = orig();
        let inputs = load_inputs(&dir).unwrap();
        let assets = Assets::load(&dir).unwrap();
        Session::new(&inputs, assets, None, begin, Fixes::ALL, view)
    }

    fn new_game(view: ViewOpts) -> Session {
        open(Begin::Stage1(Start::NewGame { difficulty: None }), view)
    }

    /// 跑到玩家機可以控制而且畫面有東西。
    fn until_playable(s: &mut Session) {
        for _ in 0..400 {
            s.step(&[]);
            if s.playable(0) && !s.hold {
                // 再多幾步，讓開場的補間與進場結束。
                for _ in 0..3 {
                    s.step(&[]);
                }
                return;
            }
        }
        panic!("400 步內玩家機沒有可以控制");
    }

    fn pressed(code: u8) -> [bool; 128] {
        let mut k = [false; 128];
        k[code as usize] = true;
        k
    }

    /// 預測的前提：可以控制的每一步，玩家機照按鍵走滿一步。四關都要成立，否則 `lead` 會一直猜錯。
    #[test]
    #[ignore]
    fn d1_every_stage_moves_one_step_per_logic_step() {
        let mut s = open(Begin::Title, VIEW);
        let mut stats: Vec<(u32, u32, u32, u64)> = Vec::new(); // 可控制步數、照按鍵走滿的步數、不符的步數、ticks 累計
        let mut in_stage = 0u32;
        let (mut dir, mut held) = (RIGHT, false);
        let mut idle = 0u32;
        let mut release: Vec<u8> = Vec::new();
        for n in 0..60_000u32 {
            // 不死（`word_273A5` 的作弊旗標），才量得到整關。
            s.game.poke8(rs2_core::a(0x273A5), b'Y');
            let before = s.game.player_status(0);
            let was_playable = s.playable(0);
            let mut sc: Vec<u8> = std::mem::take(&mut release);
            if was_playable {
                if stats.is_empty() || in_stage == 0 {
                    stats.push((0, 0, 0, 0));
                    in_stage = 1;
                }
                if !held {
                    sc.push(dir);
                    held = true;
                }
                // 走到牆邊就換方向，順便測到界限。
                if (dir == RIGHT && !before.can_right) || (dir == LEFT && !before.can_left) {
                    sc.push(dir | 0x80);
                    dir = if dir == RIGHT { LEFT } else { RIGHT };
                    sc.push(dir);
                }
                if stats.last().unwrap().0 >= 240 {
                    // 這關量夠了，用 Ctrl+C 過關。
                    sc.push(dir | 0x80);
                    held = false;
                    // 熱鍵看的是按鍵狀態，放開碼要留到下一步。
                    sc.extend_from_slice(&[0x1D, 0x2E]);
                    release.extend_from_slice(&[0xAE, 0x9D]);
                    in_stage = 0;
                }
                idle = 0;
            } else {
                in_stage = 0;
                // 換關時按鍵狀態表可能被清掉，進關後重按。
                if held {
                    sc.push(dir | 0x80);
                    held = false;
                }
                idle += 1;
                // 標題與過場等按鍵。
                if idle % 40 == 0 {
                    sc.push(0x39);
                    release.push(0xB9);
                }
            }
            let down = held;
            let Outcome::Continue = s.step(&sc) else {
                eprintln!("[d1] 第 {n} 步遊戲結束");
                break;
            };
            if stats.len() >= 4 && stats[3].0 >= 240 {
                break;
            }
            let after = s.game.player_status(0);
            if was_playable && s.playable(0) && !s.hold && down {
                let st = stats.last_mut().unwrap();
                st.0 += 1;
                st.3 += s.ticks() as u64;
                let moved = (after.x as i32 - before.x as i32) * 8;
                let can = if dir == RIGHT { before.can_right } else { before.can_left };
                let want = match (dir == RIGHT, can) {
                    (_, false) => 0,
                    (true, true) => 16,
                    (false, true) => -16,
                };
                if moved == want {
                    st.1 += 1;
                } else {
                    st.2 += 1;
                    if st.2 <= 5 {
                        eprintln!(
                            "[d1] 第 {n} 步不符：x {} -> {}，方向 {dir:#x}，預期 {want}，實際 {moved}，幀 {}",
                            before.x,
                            after.x,
                            u16::from_le_bytes([s.game.state_hash_input()[0x422], s.game.state_hash_input()[0x423]])
                        );
                    }
                }
            }
        }
        for (i, st) in stats.iter().enumerate() {
            eprintln!(
                "[d1] 第 {} 段：可控制 {} 步，走滿 {}，不符 {}，平均 {:.2} tick",
                i + 1,
                st.0,
                st.1,
                st.2,
                st.3 as f64 / st.0.max(1) as f64
            );
        }
        assert!(stats.len() >= 4, "沒有走完四關：{stats:?}");
        for st in &stats {
            assert!(st.0 > 0);
            assert!(st.2 * 50 <= st.0, "不符超過 2%：{stats:?}");
        }
    }

    /// 示範錄影在開車的時候不算可以控制：不按鍵從標題一路跑，玩家機動了就不能是 playable。
    #[test]
    #[ignore]
    fn d5_demo_is_not_playable() {
        let mut s = open(Begin::Title, VIEW);
        let (mut playable_steps, mut drawn_steps, mut moved_in_demo) = (0u32, 0u32, 0u32);
        let mut last = None;
        for _ in 0..6000 {
            if let Outcome::Ended(_) = s.step(&[]) {
                break;
            }
            let st = s.game.player_status(0);
            let flag = s.game.state_hash_input()[rs2_core::a(0x1CFDC) as usize];
            if s.player_drawn[0] {
                drawn_steps += 1;
                if last.is_some_and(|l| l != (st.x, st.y)) {
                    moved_in_demo += 1;
                }
                last = Some((st.x, st.y));
            }
            if s.playable(0) {
                playable_steps += 1;
                if playable_steps <= 3 {
                    eprintln!("[d5] 可控制：位置 ({},{})，byte_1CFDC = {flag:#x}，步 {}", st.x, st.y, s.steps());
                }
            }
        }
        eprintln!("[d5] 畫了玩家機 {drawn_steps} 步，其中移動 {moved_in_demo} 次，判為可控制 {playable_steps} 步");
        assert!(drawn_steps > 100, "這段要涵蓋示範錄影");
        assert!(moved_in_demo > 0, "示範錄影要真的有移動");
        assert_eq!(playable_steps, 0, "沒有按任何鍵，不應該有可控制的步");
    }

    /// 真的開始遊戲，之後不按任何鍵：可以控制的期間玩家機不動。
    /// 原版放著不動約在幀 129 被打死，死亡動畫（`sub_14679`）每幀往下往右走，那段不算可以控制，預測不能跟著猜。
    #[test]
    #[ignore]
    fn d6_no_input_means_no_motion() {
        let mut s = open(Begin::Title, VIEW);
        let mut last: Option<(i16, i16)> = None;
        let (mut steps_in_play, mut moves) = (0u32, 0u32);
        let mut idle = 0u32;
        let mut hold_release: Vec<u8> = Vec::new();
        for _ in 0..4000 {
            let mut sc = std::mem::take(&mut hold_release);
            if !s.playable(0) && steps_in_play == 0 {
                idle += 1;
                if idle % 40 == 0 {
                    sc.push(0x39);
                    hold_release.push(0xB9);
                }
            }
            if let Outcome::Ended(_) = s.step(&sc) {
                break;
            }
            if s.playable(0) {
                steps_in_play += 1;
                let st = s.game.player_status(0);
                let pos = (st.x, st.y);
                if last.is_some_and(|l| l != pos) {
                    moves += 1;
                    if moves <= 4 {
                        let d = s.game.state_hash_input();
                        let k = |l: u32| d[rs2_core::a(l) as usize];
                        eprintln!(
                            "[d6] 可控制第 {steps_in_play} 步移動到 ({},{})：274DA {:#x} 274DB {:#x} 274DC {:#x} 1CFDC {:#x} 1CFDD {:#x} 1CFAD {:#x} 幀 {}",
                            st.x, st.y, k(0x274DA), k(0x274DB), k(0x274DC), k(0x1CFDC), k(0x1CFDD), k(0x1CFAD),
                            u16::from_le_bytes([d[0x422], d[0x423]])
                        );
                    }
                }
                last = Some(pos);
            } else if steps_in_play > 0 {
                let st = s.game.player_status(0);
                eprintln!("[d6] 第 {steps_in_play} 步後不再可控制，位置 ({},{})", st.x, st.y);
                assert!(s.predictor.offset_hd(0).is_none(), "不能控制時要清掉預測");
                break;
            }
        }
        eprintln!("[d6] 可控制 {steps_in_play} 步，移動 {moves} 次");
        assert!(steps_in_play > 100);
        assert_eq!(moves, 0, "沒有按鍵，玩家機不該動");
    }

    /// 按鍵之後下一次繪製玩家機就動，只有玩家機周圍的像素改變。
    #[test]
    #[ignore]
    fn d2_key_press_shows_on_the_next_draw() {
        let mut s = new_game(VIEW);
        until_playable(&mut s);
        let st = s.game.player_status(0);
        assert!(st.can_right);
        let base = s.render(1.0).0.to_vec();
        let step_secs = s.ticks() as f32 * crate::pacer::TICK_SECS as f32;
        assert!(step_secs > 0.0);

        // 沒有按鍵：預測開著也和一般補間逐像素相同。
        s.predict(&[false; 128], 0.0, step_secs);
        assert_eq!(s.render(1.0).0, &base[..], "沒按鍵時不能改變畫面");

        // 按右：過半步的時間，玩家機往右 8 像素。
        s.predict(&pressed(RIGHT), step_secs / 2.0, step_secs);
        assert_eq!(s.predictor.offset_hd(0), Some((32, 0)));
        let (img, w, h) = s.render(1.0);
        assert_eq!((w, h), (320, 200));
        let px = st.x as i32 * 8;
        let py = st.y as i32;
        let (mut x0, mut x1, mut y0, mut y1, mut diff) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN, 0u32);
        for y in 0..200i32 {
            for x in 0..320i32 {
                let o = (y * 320 + x) as usize * 4;
                if img[o..o + 4] != base[o..o + 4] {
                    (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
                    diff += 1;
                }
            }
        }
        eprintln!("[d2] 玩家機 ({px},{py})，改變 {diff} 像素，範圍 x {x0}..{x1} y {y0}..{y1}");
        assert!(diff > 0, "按鍵後畫面要變");
        assert!(x0 >= px - 48 && x1 <= px + 112 && y0 >= py - 48 && y1 <= py + 64, "只有玩家機周圍會變");
        // 畫面和不按鍵的基準只差這個偏移：關掉預測後回到基準。
        s.set_view(ViewOpts { feel: false, ..VIEW });
        assert_eq!(s.render(1.0).0, &base[..], "關掉預測要回到基準");
    }

    /// 預測、繪製、切換開關都不碰模擬。
    #[test]
    #[ignore]
    fn d3_prediction_leaves_the_simulation_alone() {
        let script = |n: usize| -> Vec<u8> {
            match n % 40 {
                5 => vec![RIGHT],
                25 => vec![RIGHT | 0x80, LEFT],
                35 => vec![LEFT | 0x80],
                _ => Vec::new(),
            }
        };
        let mut a = new_game(ViewOpts { feel: false, tween: false, ..VIEW });
        let mut b = new_game(VIEW);
        let mut moved = false;
        for n in 0..600 {
            let sc = script(n);
            a.step(&sc);
            b.step(&sc);
            assert_eq!(a.dseg_hash(), b.dseg_hash(), "第 {n} 步");
            for f in 0..3 {
                b.predict(&pressed(RIGHT), 0.02, 0.055);
                moved |= b.predictor.offset_hd(0).is_some_and(|o| o != (0, 0));
                b.render(f as f32 / 3.0);
            }
            if n == 300 {
                b.set_view(ViewOpts { feel: false, ..VIEW });
            }
            if n == 400 {
                b.set_view(VIEW);
            }
        }
        assert!(moved, "這段輸入要真的用到預測");
    }

    /// 真實的節奏下顯示位置連續：邏輯步到來時不跳，按住時領先不超過一步。
    #[test]
    #[ignore]
    fn d4_shown_position_stays_continuous_while_held() {
        let mut s = new_game(VIEW);
        until_playable(&mut s);
        let mut sc = vec![RIGHT];
        let keys = pressed(RIGHT);
        let mut last: Option<f32> = None;
        let mut worst = 0f32;
        let mut moved_steps = 0;
        for _ in 0..12 {
            let before = s.game.player_status(0);
            let step_secs = s.ticks() as f32 * crate::pacer::TICK_SECS as f32;
            // 一步畫三次。
            for _ in 0..3 {
                s.predict(&keys, step_secs / 3.0, step_secs);
                let st = s.game.player_status(0);
                let shown = st.x as f32 * 8.0 + s.predictor.offset_hd(0).unwrap().0 as f32 / 4.0;
                if let Some(l) = last {
                    worst = worst.max((shown - l).abs());
                }
                last = Some(shown);
            }
            s.step(&sc);
            sc.clear();
            let st = s.game.player_status(0);
            let shown = st.x as f32 * 8.0 + s.predictor.offset_hd(0).unwrap().0 as f32 / 4.0;
            let l = last.unwrap();
            assert!((shown - l).abs() <= 0.25, "邏輯步到來時畫面跳了：{l} -> {shown}");
            if st.x != before.x {
                moved_steps += 1;
            }
        }
        eprintln!("[d4] 走了 {moved_steps} 步，相鄰兩次繪製最大位移 {worst} 像素");
        assert!(moved_steps >= 8);
        // 一步 16 像素分三次畫，每次約 5.3 像素；第一步要追上一步的差距，上限放到一步以內。
        assert!(worst <= 16.0, "{worst}");
    }
}
