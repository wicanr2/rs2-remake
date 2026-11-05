//! 戰場大小（規格 005「戰場大小」：100%、120%、150%）在真實資料上的測試。
//!
//! 需要原版檔，標 `#[ignore]`，由 `tools/rust.sh play-test field_` 帶 `--include-ignored` 跑。單元層的行為測試
//! （格網版面、生成欄、子彈與玩家界限、幾何、HUD 寬度）在各 crate 自己的 `tests`，這裡驗整條路：
//!
//! | 測試 | 內容 |
//! |---|---|
//! | `field_e4_*` | 120%、150% 從標題一路玩到片尾不當掉：每關欄數、玩家碰到新右緣、敵人從新右緣進場、子彈在新邊界消失 |
//! | `field_e2_*` | 每個寬度同一份輸入跑兩次，每步 `Dseg` 雜湊相同；三個寬度的雜湊兩兩不同 |
//! | `field_e1_*` | 選項開著，展示錄影（`Start::Demo`）一律 100%，每步雜湊與畫面和選項關著的相同 |
//! | `field_fixed_*` | 關內改選項不生效，下一關進場才生效；離開關卡回到 40 欄 |
//! | `field_mono_*` | 寬戰場拒絕切到 CGA、MGA；單色進場的關卡不加大 |
//! | `field_sizes_*` | 輸出尺寸是欄數 × 8（HD 再乘 4）、每個寬度的畫面不是全黑、左側 320 像素內的內容與 100% 同風格 |
//! | `field_shots_*` | 存第一關與第三關的 120%、150% 畫面（原版圖與 HD），`RS2_OUT/field-shots`，給人看 |
//!
//! 這個檔是 `session` 的子模組，可以讀 `Session` 的私有欄位（`game`、`playable`）。
//! 寬戰場的格網不在原版資料段裡，所以這裡一律用 `set_invincible`（寫狀態），不用 `poke8`（會被 `Mirror` 拒絕）。

use super::*;
use rs2_game::{FieldSize, Rules};
use rs2_trace::load_inputs;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

const RIGHT: u8 = 0x4D;
const LEFT: u8 = 0x4B;
const FIRE_P1: u8 = 0x36;
/// 每一關按 Ctrl+C 之前可以控制的步數。
const PLAY_STEPS: u32 = 400;

fn orig() -> PathBuf {
    PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh play-test 跑"))
}

fn inputs() -> &'static Inputs {
    static I: OnceLock<Inputs> = OnceLock::new();
    I.get_or_init(|| load_inputs(&orig()).unwrap())
}

fn mono_assets() -> Arc<MonoAssets> {
    static M: OnceLock<Arc<MonoAssets>> = OnceLock::new();
    Arc::clone(M.get_or_init(|| Arc::new(MonoAssets::load(&orig()).expect("單色圖庫載入失敗"))))
}

fn rules(f: FieldSize) -> Rules {
    Rules { field: f, ..Rules::ORIGINAL }
}

fn view(mode: DisplayMode) -> ViewOpts {
    ViewOpts { tween: false, counter: false, hd: false, feel: false, mode }
}

fn open(begin: Begin, f: FieldSize, v: ViewOpts) -> Session {
    let assets = Assets::load(&orig()).unwrap();
    Session::new_with_rules(inputs(), assets, Some(mono_assets()), None, begin, Fixes::ALL, v, rules(f))
}

fn new_game() -> Begin {
    Begin::Stage1(Start::NewGame { difficulty: None })
}

/// 目前的關卡字母（`byte_1CFDB`）：`A` 到 `D`。
fn letter(s: &Session) -> u8 {
    s.ds().r8(rs2_trace::mirror::a(0x1CFDB))
}

fn until_playable(s: &mut Session) {
    for _ in 0..400 {
        s.step(&[]);
        if s.playable(0) && !s.hold {
            for _ in 0..5 {
                s.step(&[]);
            }
            return;
        }
    }
    panic!("400 步內玩家機沒有可以控制");
}

/// 從標題走完四關與片尾的輸入：可以控制時左右走並按住開火，走滿 `PLAY_STEPS` 步就用 Ctrl+C 過關，
/// 其餘時間隔一段時間按空白鍵。只看 Session 自己的狀態，所以同樣的選項輸入完全相同。
struct Driver {
    dir: u8,
    held: bool,
    idle: u32,
    in_play: u32,
    release: Vec<u8>,
}

impl Driver {
    fn new() -> Driver {
        Driver { dir: RIGHT, held: false, idle: 0, in_play: 0, release: Vec::new() }
    }

    /// 這一步的掃描碼，以及這一步有沒有按 Ctrl+C（要過關）。
    fn next(&mut self, s: &Session) -> (Vec<u8>, bool) {
        let mut sc = std::mem::take(&mut self.release);
        let mut left_stage = false;
        if s.playable(0) {
            let st = s.game.player_status(0);
            if !self.held {
                sc.push(self.dir);
                sc.push(FIRE_P1);
                self.held = true;
            }
            // 走到牆邊就換方向。
            if (self.dir == RIGHT && !st.can_right) || (self.dir == LEFT && !st.can_left) {
                sc.push(self.dir | 0x80);
                self.dir = if self.dir == RIGHT { LEFT } else { RIGHT };
                sc.push(self.dir);
            }
            self.in_play += 1;
            if self.in_play >= PLAY_STEPS {
                sc.push(self.dir | 0x80);
                sc.push(FIRE_P1 | 0x80);
                self.held = false;
                // 熱鍵看的是按鍵狀態，放開碼要留到下一步。
                sc.extend_from_slice(&[0x1D, 0x2E]);
                self.release.extend_from_slice(&[0xAE, 0x9D]);
                self.in_play = 0;
                left_stage = true;
            }
            self.idle = 0;
        } else {
            self.in_play = 0;
            if self.held {
                sc.push(self.dir | 0x80);
                sc.push(FIRE_P1 | 0x80);
                self.held = false;
            }
            self.idle += 1;
            // 標題與過場等按鍵。
            if self.idle % 40 == 0 {
                sc.push(0x39);
                self.release.push(0xB9);
            }
        }
        (sc, left_stage)
    }
}

/// 一關（依字母）累積的觀察。
#[derive(Clone, Debug, Default)]
struct StageStats {
    /// 這一關出現過的欄數。
    cols: std::collections::BTreeSet<u16>,
    /// 可以控制的步數。
    play_steps: u32,
    /// 玩家機左緣（位元組）的最小與最大值。
    min_x: i16,
    max_x: i16,
    /// 在場敵人（邏輯欄）的最大值，以及在場子彈（玩家彈、敵彈）欄的最大值。
    max_enemy_col: i16,
    max_pbullet_col: i16,
    max_ebullet_col: i16,
    /// 玩家在最右緣（`can_right` 為假）的步數。
    at_right_edge: u32,
}

/// 一整段流程的記錄。
struct Playthrough {
    hashes: Vec<String>,
    stages: BTreeMap<u8, StageStats>,
    first_cols: u16,
    last_cols: u16,
    /// 第四關過關後片尾跑滿 2500 步，或遊戲自己回報結束。
    ended: bool,
    sizes: std::collections::BTreeSet<(usize, usize)>,
}

/// 從標題玩到片尾：每步不死、每 4 步畫一次（`LayerSink` 路徑），記下每關的觀察。
fn playthrough(f: FieldSize, hd: Option<&PathBuf>) -> Playthrough {
    let v = ViewOpts { tween: false, counter: false, hd: hd.is_some(), feel: false, mode: DisplayMode::Vga };
    let mut s = match hd {
        Some(dir) => {
            let assets = Assets::load(&orig()).unwrap();
            let mut h = HdAssets::open(dir).unwrap();
            h.preload(&assets, 4);
            Session::new_with_rules(inputs(), assets, None, Some(h), Begin::Title, Fixes::ALL, v, rules(f))
        }
        None => open(Begin::Title, f, v),
    };
    let mut drv = Driver::new();
    let mut out = Playthrough {
        hashes: Vec::new(),
        stages: BTreeMap::new(),
        first_cols: s.field_cols(),
        last_cols: 0,
        ended: false,
        sizes: Default::default(),
    };
    let mut tail = 0u32;
    for n in 0..60_000u32 {
        let (sc, left) = drv.next(&s);
        let before = letter(&s);
        if left && before == b'D' {
            tail = 2500;
        }
        s.set_invincible(true);
        let o = s.step(&sc);
        out.hashes.push(s.dseg_hash());
        if n % 4 == 0 {
            let (px, w, h) = s.render(1.0);
            assert_eq!(px.len(), w * h * 4);
            out.sizes.insert((w, h));
        }
        let st = out.stages.entry(letter(&s)).or_insert_with(|| StageStats { min_x: i16::MAX, max_x: i16::MIN, max_enemy_col: i16::MIN, max_pbullet_col: i16::MIN, max_ebullet_col: i16::MIN, ..Default::default() });
        st.cols.insert(s.field_cols());
        if s.playable(0) {
            let p = s.game.player_status(0);
            st.play_steps += 1;
            st.min_x = st.min_x.min(p.x);
            st.max_x = st.max_x.max(p.x);
            st.at_right_edge += u32::from(!p.can_right);
            let state = s.game.state();
            for e in state.en.slots.iter().filter(|e| e.link != 0) {
                st.max_enemy_col = st.max_enemy_col.max(e.enemy.col as i16);
            }
            for b in state.pb.slots.iter().filter(|b| b.link != 0) {
                st.max_pbullet_col = st.max_pbullet_col.max(b.bullet.col as i16);
            }
            for b in state.eb.slots.iter().filter(|b| b.link != 0) {
                st.max_ebullet_col = st.max_ebullet_col.max(b.bullet.col as i16);
            }
        }
        if matches!(o, Outcome::Ended(_)) {
            out.ended = true;
            break;
        }
        if tail > 0 {
            tail -= 1;
            if tail == 0 {
                // 第四關過關後片尾跑滿 2500 步（和 `session_modes_tests` 的 m1 同樣的收尾）。
                out.ended = true;
                break;
            }
        }
    }
    out.last_cols = s.field_cols();
    out
}

fn extra_of(f: FieldSize) -> i16 {
    f.cols() as i16 - 40
}

fn describe(f: FieldSize, p: &Playthrough) {
    eprintln!("[field] {} 欄：{} 步，尺寸 {:?}，開頭欄數 {}，結尾欄數 {}，走到片尾 {}", f.cols(), p.hashes.len(), p.sizes, p.first_cols, p.last_cols, p.ended);
    for (l, st) in &p.stages {
        eprintln!(
            "[field]   關 {}：欄數 {:?}，可控制 {} 步，玩家 x {}..{}（最右 {} 步），敵人欄最大 {}，玩家彈欄最大 {}，敵彈欄最大 {}",
            *l as char, st.cols, st.play_steps, st.min_x, st.max_x, st.at_right_edge, st.max_enemy_col, st.max_pbullet_col, st.max_ebullet_col
        );
    }
}

/// 一個寬度從標題玩到片尾，每一關檢查：欄數、玩家碰到新右緣、敵人從新右緣進場、子彈在新邊界消失。
fn check_playthrough(f: FieldSize, p: &Playthrough, hd: bool) {
    let extra = extra_of(f);
    let cols = f.cols() as i16;
    assert!(p.ended, "{} 欄：四關過完，片尾要跑完", f.cols());
    assert_eq!(p.first_cols, 40, "標題是 40 欄");
    assert_eq!(p.last_cols, 40, "片尾回到 40 欄");
    for letter in [b'A', b'B', b'C', b'D'] {
        let st = p.stages.get(&letter).unwrap_or_else(|| panic!("{} 欄：沒走到第 {} 關", f.cols(), letter as char));
        let name = format!("{} 欄第 {} 關", f.cols(), letter as char);
        assert!(st.play_steps > 100, "{name}：可控制的步數 {} 太少", st.play_steps);
        assert!(st.cols.contains(&(cols as u16)), "{name}：欄數 {:?} 要含 {cols}", st.cols);
        // 玩家碰到新右緣：右緣是 33 欄加上多出來的欄數。
        assert_eq!(st.max_x, 33 + extra, "{name}：玩家最右欄");
        assert!(st.at_right_edge > 0, "{name}：玩家要真的停在右緣過");
        assert!(st.min_x >= 0, "{name}：左緣不變");
        // 敵人從新右緣進場：生成欄是 40 加多出來的欄數（或更右），第一次被取樣前最多已經走了幾欄。
        assert!(st.max_enemy_col >= 40 + extra - 4, "{name}：敵人欄最大 {} 要接近新右緣 {}", st.max_enemy_col, 40 + extra);
        // 子彈在新邊界消失：飛過原版的邊界，不超過新邊界。
        if extra > 0 {
            assert!(st.max_pbullet_col > 39, "{name}：玩家彈飛過原版邊界，最大 {}", st.max_pbullet_col);
        }
        assert!(st.max_pbullet_col <= cols - 1, "{name}：玩家彈最大欄 {} 不超過新邊界 {}", st.max_pbullet_col, cols - 1);
        assert!(st.max_ebullet_col <= cols - 1, "{name}：敵彈最大欄 {} 不超過新邊界 {}", st.max_ebullet_col, cols - 1);
    }
    // 輸出尺寸：欄數 × 8 像素寬，HD 再乘 4；標題與過場是原本的 320×200。
    let k = if hd { 4 } else { 1 };
    let (w, h) = ((f.cols() as usize) * 8 * k, 200 * k);
    assert!(p.sizes.contains(&(w, h)), "{} 欄：render 尺寸 {:?} 要含 {w}×{h}", f.cols(), p.sizes);
    assert!(p.sizes.contains(&(320 * k, h)), "標題與過場是 {}×{h}：{:?}", 320 * k, p.sizes);
}

fn hd_dir() -> Option<PathBuf> {
    std::env::var("RS2_HD").ok().map(PathBuf::from)
}

/// E4：120% 與 150% 從標題一路玩到片尾不當掉（原版圖；有 HD 資料夾時 150% 再用 HD 圖走一次）。
#[test]
#[ignore]
fn field_e4_both_widths_play_from_the_title_to_the_ending() {
    // 100% 當對照：同一套檢查（多出來的欄數是 0）也要過，數字拿來和寬戰場比。
    for f in [FieldSize::Normal, FieldSize::Wide, FieldSize::Wider] {
        let p = playthrough(f, None);
        describe(f, &p);
        check_playthrough(f, &p, false);
    }
    if let Some(dir) = hd_dir() {
        let p = playthrough(FieldSize::Wider, Some(&dir));
        describe(FieldSize::Wider, &p);
        check_playthrough(FieldSize::Wider, &p, true);
    } else {
        eprintln!("[field] 沒有 RS2_HD，略過 HD 全程");
    }
}

/// E2：每個寬度同一份輸入跑兩次，每步雜湊相同。三個寬度的雜湊不同（行為確實改變）。
#[test]
#[ignore]
fn field_e2_each_width_is_deterministic_and_differs_from_the_others() {
    let mut runs: Vec<(FieldSize, Vec<String>)> = Vec::new();
    for f in [FieldSize::Normal, FieldSize::Wide, FieldSize::Wider] {
        let a = stage_run(f, 900);
        let b = stage_run(f, 900);
        assert_eq!(a, b, "{} 欄：同樣的輸入，兩次的雜湊要相同", f.cols());
        runs.push((f, a.1));
    }
    for (i, (fa, ha)) in runs.iter().enumerate() {
        for (fb, hb) in &runs[i + 1..] {
            let first = ha.iter().zip(hb).position(|(x, y)| x != y);
            assert!(first.is_some(), "{} 欄與 {} 欄的雜湊要不同", fa.cols(), fb.cols());
            eprintln!("[field] {} 欄與 {} 欄從第 {} 步起不同", fa.cols(), fb.cols(), first.unwrap());
        }
    }
    // 完整流程（標題到片尾）每步雜湊也相同。
    for f in [FieldSize::Wide, FieldSize::Wider] {
        let a = playthrough(f, None);
        let b = playthrough(f, None);
        assert_eq!(a.hashes.len(), b.hashes.len());
        assert_eq!(a.hashes, b.hashes, "{} 欄：標題到片尾的每步雜湊要相同", f.cols());
    }
}

/// 第一關 `steps` 步，吃固定的腳本輸入，回傳每步雜湊（`dseg_hash`，寬戰場含格網）與
/// 每步敵人池的雜湊（比對兩個寬度的行為從哪一步開始分歧用：格網的內容本來就隨寬度不同，不能拿來比）。
fn stage_run(f: FieldSize, steps: u32) -> (Vec<String>, Vec<String>) {
    let mut s = open(new_game(), f, view(DisplayMode::Vga));
    until_playable(&mut s);
    let mut drv = Driver::new();
    let (mut full, mut enemies) = (Vec::new(), Vec::new());
    for _ in 0..steps {
        let (sc, _) = drv.next(&s);
        s.set_invincible(true);
        s.step(&sc);
        full.push(s.dseg_hash());
        enemies.push(sha256_hex(format!("{:?}", s.game.state().en).as_bytes()));
    }
    assert_eq!(s.field_cols(), f.cols());
    (full, enemies)
}

/// E1：選項開著，展示錄影一律 100%，每步雜湊與畫面和選項關著的相同。
#[test]
#[ignore]
fn field_e1_demo_recordings_stay_at_100_percent() {
    for f in [FieldSize::Wide, FieldSize::Wider] {
        let mut on = open(Begin::Stage1(Start::Demo), f, view(DisplayMode::Vga));
        let mut off = open(Begin::Stage1(Start::Demo), FieldSize::Normal, view(DisplayMode::Vga));
        for n in 0..700 {
            on.step(&[]);
            off.step(&[]);
            assert_eq!(on.field_cols(), 40, "展示錄影第 {n} 步：欄數是 40");
            assert_eq!(on.dseg_hash(), off.dseg_hash(), "{} 欄：展示錄影第 {n} 步的雜湊", f.cols());
            if n % 5 == 0 {
                let a = on.render(1.0).0.to_vec();
                let b = off.render(1.0).0.to_vec();
                assert_eq!(a, b, "{} 欄：展示錄影第 {n} 步的畫面", f.cols());
            }
        }
    }
}

/// 選項開著、全程選項關著的兩個 Session：從第一關進場起，100% 的每一步雜湊與畫面相同
/// （選項值是 100% 時，不論有沒有碰過選項，行為都等於原版）。
#[test]
#[ignore]
fn field_e1_normal_size_after_touching_the_option_equals_untouched() {
    let mut quiet = open(new_game(), FieldSize::Normal, view(DisplayMode::Vga));
    let mut touched = open(new_game(), FieldSize::Normal, view(DisplayMode::Vga));
    touched.set_rules(rules(FieldSize::Wider));
    touched.set_rules(rules(FieldSize::Normal));
    let (mut da, mut db) = (Driver::new(), Driver::new());
    for n in 0..500 {
        let (a, _) = da.next(&quiet);
        let (b, _) = db.next(&touched);
        assert_eq!(a, b);
        quiet.set_invincible(true);
        touched.set_invincible(true);
        quiet.step(&a);
        touched.step(&b);
        assert_eq!(quiet.dseg_hash(), touched.dseg_hash(), "第 {n} 步的雜湊");
        if n % 3 == 0 {
            assert_eq!(quiet.render(1.0).0.to_vec(), touched.render(1.0).0.to_vec(), "第 {n} 步的畫面");
        }
    }
    assert_eq!(touched.field_cols(), 40);
}

/// 關內改選項不生效，下一次進關才生效；離開關卡（標題、過場）回到 40 欄。
#[test]
#[ignore]
fn field_fixed_in_stage_and_applied_at_the_next_entry() {
    // 從標題開始，過關才會接到下一關（`Begin::Stage1` 第一關結束就離開）。
    let mut s = open(Begin::Title, FieldSize::Wide, view(DisplayMode::Vga));
    play_to(&mut s, b'A', 5);
    assert_eq!(s.field_cols(), 48);
    // 關內改成 150%、再改成 100%：這一關還是 48。
    for f in [FieldSize::Wider, FieldSize::Normal] {
        s.set_rules(rules(f));
        for _ in 0..30 {
            s.set_invincible(true);
            s.step(&[]);
        }
        assert_eq!(s.field_cols(), 48, "關內改成 {} 欄，這一關不變", f.cols());
    }
    // 此時選項是 100%。Ctrl+C 過關，下一關進場讀新值。
    s.set_rules(rules(FieldSize::Wider));
    let mut drv = Driver::new();
    drv.in_play = PLAY_STEPS - 1;
    let mut seen_second = false;
    let mut between = Vec::new();
    for _ in 0..3000 {
        let (sc, _) = drv.next(&s);
        s.set_invincible(true);
        s.step(&sc);
        let l = letter(&s);
        if l == b'B' && s.playable(0) {
            seen_second = true;
            break;
        }
        between.push(s.field_cols());
    }
    assert!(seen_second, "要走到第二關：目前第 {} 關，欄數 {}", letter(&s) as char, s.field_cols());
    assert_eq!(s.field_cols(), 60, "第二關進場讀到新的選項值 150%");
    assert!(between.contains(&40), "過關到第二關之間（過場）是 40 欄：{between:?}");
}

/// 單色模式：寬戰場拒絕切換；單色進場的關卡不加大（`Settings::rules` 在單色模式把選項壓成 100%，這裡驗 Session 端）。
#[test]
#[ignore]
fn field_mono_modes_are_refused_in_a_wide_stage() {
    let mut s = open(new_game(), FieldSize::Wide, view(DisplayMode::Vga));
    until_playable(&mut s);
    assert!(s.field_wide() && s.mono_available());
    assert!(!s.set_mode(DisplayMode::Cga), "寬戰場拒絕 CGA");
    assert!(!s.set_mode(DisplayMode::Mga), "寬戰場拒絕 MGA");
    assert_eq!(s.mode(), DisplayMode::Vga);
    assert!(s.set_mode(DisplayMode::Ega), "EGA 照常");
    assert_eq!(s.mode(), DisplayMode::Ega);
    s.step(&[]);
    let (_, w, h) = s.render(1.0);
    assert_eq!((w, h), (384, 200));
    // 100% 的關卡仍能切單色。
    let mut n = open(new_game(), FieldSize::Normal, view(DisplayMode::Vga));
    until_playable(&mut n);
    assert!(n.set_mode(DisplayMode::Cga) && n.set_mode(DisplayMode::Mga));
}

/// 輸出尺寸與畫面內容：每個寬度的 `render` 是欄數 × 8 寬、200 高，EGA 與 VGA 都一樣；畫面有內容，
/// 玩家機所在的位置（右緣附近）有像素，不是全黑，右側新增的區域有背景（不是整塊空白）。
#[test]
#[ignore]
fn field_sizes_output_matches_the_column_count_in_vga_and_ega() {
    for f in [FieldSize::Normal, FieldSize::Wide, FieldSize::Wider] {
        for mode in [DisplayMode::Vga, DisplayMode::Ega] {
            let mut s = open(new_game(), f, view(mode));
            until_playable(&mut s);
            for _ in 0..60 {
                s.set_invincible(true);
                s.step(&[]);
            }
            let (px, w, h) = s.render(1.0);
            assert_eq!((w, h), (f.cols() as usize * 8, 200), "{} 欄 {mode:?}", f.cols());
            let lit = px.chunks_exact(4).filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0).count();
            assert!(lit > w * h / 4, "{} 欄 {mode:?}：畫面有內容（亮點 {lit}）", f.cols());
            // 右側新增的區域（原版右緣之外）也要有背景：第 40 欄之後、遊戲區（190 列）內不是整塊黑。
            if f != FieldSize::Normal {
                let right_lit = (0..190).flat_map(|y| (320..w).map(move |x| (x, y))).filter(|&(x, y)| {
                    let i = (y * w + x) * 4;
                    px[i] != 0 || px[i + 1] != 0 || px[i + 2] != 0
                }).count();
                let area = (w - 320) * 190;
                assert!(right_lit > area / 2, "{} 欄 {mode:?}：新增區域的亮點 {right_lit} / {area}", f.cols());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 截圖：RS2_OUT/field-shots，給人看。
// ---------------------------------------------------------------------------

fn shots_dir() -> Option<PathBuf> {
    std::env::var("RS2_OUT").ok().map(|d| PathBuf::from(d).join("field-shots")).inspect(|d| std::fs::create_dir_all(d).unwrap())
}

fn save(s: &mut Session, name: &str) {
    let Some(dir) = shots_dir() else { return };
    let (px, w, h) = s.render(1.0);
    std::fs::write(dir.join(name), rs2_gfx::png::encode_rgba(w, h, px)).unwrap();
}

/// 開 Session，`hd` 為真時載入 HD 資料夾（沒有 `RS2_HD` 就回傳 `None`）。
fn open_shot(begin: Begin, f: FieldSize, hd: bool) -> Option<Session> {
    if !hd {
        return Some(open(begin, f, ViewOpts { tween: true, counter: false, hd: false, feel: true, mode: DisplayMode::Vga }));
    }
    let dir = std::env::var("RS2_HD").ok().map(PathBuf::from)?;
    let assets = Assets::load(&orig()).unwrap();
    let mut h = HdAssets::open(&dir).unwrap();
    h.preload(&assets, 4);
    let v = ViewOpts { tween: true, counter: false, hd: true, feel: true, mode: DisplayMode::Vga };
    Some(Session::new_with_rules(inputs(), assets, None, Some(h), begin, Fixes::ALL, v, rules(f)))
}

/// 玩到第 `target` 關（`A` 到 `D`）可以控制，再多走 `extra_steps` 步（不死、左右走、按住開火）。
fn play_to(s: &mut Session, target: u8, extra_steps: u32) {
    let mut drv = reach(s, target);
    advance(s, &mut drv, extra_steps);
}

/// 多走 `n` 步，不過關。
fn advance(s: &mut Session, drv: &mut Driver, n: u32) {
    for _ in 0..n {
        let (sc, _) = drv.next(s);
        drv.in_play = 0;
        s.set_invincible(true);
        s.step(&sc);
    }
}

/// 從標題玩到第 `target` 關可以控制，回傳接著用的腳本輸入。
fn reach(s: &mut Session, target: u8) -> Driver {
    let mut drv = Driver::new();
    for _ in 0..40_000 {
        let (sc, _) = drv.next(s);
        s.set_invincible(true);
        s.step(&sc);
        if letter(s) == target && s.playable(0) && !s.hold {
            break;
        }
        // 還沒到目標關：用最短的路過關（可控制就按 Ctrl+C）。
        if s.playable(0) && letter(s) != target {
            drv.in_play = PLAY_STEPS - 1;
        }
    }
    assert_eq!(letter(s), target, "走不到第 {} 關", target as char);
    drv
}

/// 四關在 100%、120%、150% 的畫面，原版圖與 HD 各一組。每關三張：關卡前段、中段、頭目附近。
/// 100% 當對照；第一關與第三關是規格要求的，第二關（視差背景）與第四關（地形與頭目）一併存。
#[test]
#[ignore]
fn field_shots_every_stage_in_all_widths() {
    let marks: [(u8, &[u32]); 4] = [(b'A', &[200, 500, 700]), (b'B', &[150, 350, 650]), (b'C', &[120, 450, 475, 500, 1150]), (b'D', &[150, 350, 1080])];
    for hd in [false, true] {
        for f in [FieldSize::Normal, FieldSize::Wide, FieldSize::Wider] {
            let tag = format!("{}{}", f.cols() * 5 / 2, if hd { "hd" } else { "sd" });
            for (stage, steps) in marks {
                let Some(mut s) = open_shot(Begin::Title, f, hd) else {
                    eprintln!("[field] 沒有 RS2_HD，略過 HD 截圖");
                    break;
                };
                let mut drv = reach(&mut s, stage);
                let mut done = 0;
                for &at in steps {
                    advance(&mut s, &mut drv, at - done);
                    done = at;
                    save(&mut s, &format!("stage{}_{}_{:04}.png", (stage - b'A' + 1), tag, at));
                }
            }
        }
    }
}

/// 不開火、不死，每一關前 1000 步裡每個敵人被釋放前最後一次在場的欄：`(左緣外, 畫面內, 右緣外)` 各幾個，
/// 以及畫面內釋放的欄。沒被打死，所以釋放全是腳本結束。
fn release_spots(f: FieldSize, stage: u8) -> (usize, usize, usize, Vec<i16>) {
    let mut s = open(Begin::Title, f, view(DisplayMode::Vga));
    let _drv = reach(&mut s, stage);
    let n_slots = s.game.state().en.slots.len();
    let mut was = vec![false; n_slots];
    let mut last = vec![0i16; n_slots];
    let mut rel: Vec<i16> = Vec::new();
    for _ in 0..1000 {
        s.set_invincible(true);
        s.step(&[]);
        for (i, e) in s.game.state().en.slots.iter().enumerate() {
            let on = e.link != 0;
            if was[i] && !on {
                rel.push(last[i]);
            }
            if on {
                last[i] = e.enemy.col as i16;
            }
            was[i] = on;
        }
    }
    let c = f.cols() as i16;
    let mut inside: Vec<i16> = rel.iter().copied().filter(|&x| x >= 0 && x < c).collect();
    inside.sort();
    (rel.iter().filter(|&&x| x < 0).count(), inside.len(), rel.iter().filter(|&&x| x >= c).count(), inside)
}

/// 腳本是原版的長度，敵人從新右緣進場後走同樣長的路：沒有處理的話，腳本結束時還在畫面裡，會憑空消失
/// （150% 第四關 109 個裡 108 個在畫面內消失）。寬戰場的敵人腳本結束後繼續飛出畫面（`entity::Linger`），
/// 所以從左緣離開的敵人數量要和 100% 相近（不少於 100% 的四分之三）。
#[test]
#[ignore]
fn field_exits_enemies_leave_through_the_left_edge_instead_of_vanishing_in_view() {
    for stage in [b'A', b'B', b'C', b'D'] {
        let (base_left, base_in, base_right, _) = release_spots(FieldSize::Normal, stage);
        eprintln!("[field] 關 {} 100%：左緣外 {base_left}、畫面內 {base_in}、右緣外 {base_right}", stage as char);
        for f in [FieldSize::Wide, FieldSize::Wider] {
            let (left, inside, right, spots) = release_spots(f, stage);
            eprintln!("[field] 關 {} {} 欄：左緣外 {left}、畫面內 {inside}、右緣外 {right}，畫面內的欄 {spots:?}", stage as char, f.cols());
            assert!(left * 4 >= base_left * 3, "關 {} {} 欄：從左緣離開 {left} 個，100% 是 {base_left}", stage as char, f.cols());
        }
    }
}

/// 進關後的開場畫面（150%，原版圖）：每次欄數從 40 變成 60，存進場後第 4、12、24、40、60 步的畫面，
/// 確認右下角的大宇資訊橫幅靠右對齊、玩家機與背景在寬畫面裡的位置。關卡字卡在 40 欄的過場裡，不在這些畫面中。
#[test]
#[ignore]
fn field_shots_stage_cards_in_the_wide_field() {
    let mut s = open(Begin::Title, FieldSize::Wider, view(DisplayMode::Vga));
    let mut drv = Driver::new();
    let (mut prev_cols, mut entered, mut stage_no) = (40u16, None::<u32>, 0u32);
    for n in 0..30_000u32 {
        let (sc, _) = drv.next(&s);
        s.set_invincible(true);
        s.step(&sc);
        let cols = s.field_cols();
        if cols != 40 && prev_cols == 40 {
            entered = Some(n);
            stage_no += 1;
        }
        prev_cols = cols;
        if let Some(at) = entered {
            let since = n - at;
            if [4, 12, 24, 40, 60].contains(&since) {
                save(&mut s, &format!("card{stage_no}_150sd_{since:03}.png"));
            }
        }
        if stage_no == 4 && entered.is_some_and(|at| n - at > 60) {
            break;
        }
    }
    assert_eq!(stage_no, 4, "四關都進場過");
}
