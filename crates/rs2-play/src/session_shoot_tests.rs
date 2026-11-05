//! 射擊與操控選項（規格 005：波動砲充能、按住連射、砲座像 Force、細步移動）在真實資料上的測試。
//!
//! 需要原版檔，標 `#[ignore]`，由 `tools/rust.sh play-test shoot_` 帶 `--include-ignored` 跑。單元層的行為測試
//! （門檻、傷害、移動規則、砲座飛行）在 `rs2-core` 的 `player/enhanced.rs`，這裡驗：
//!
//! | 測試 | 內容 |
//! |---|---|
//! | `shoot_e2_*` | 每個選項開啟時，同一份輸入與選項跑兩次，每步 `Dseg` 雜湊相同；開與關的雜湊不同 |
//! | `shoot_e3_*` | 每個選項在它擁有的狀態上留下痕跡：充能到滿格、砲座分離過、出現過半步位置 |
//! | `shoot_e1_*` | 選項全關：經過強化介面（開了又關、分離鍵、規則設定）的一局，每步雜湊與畫面和完全沒碰過的一局相同 |
//! | `shoot_combo_*` | 四個選項全開、兩位玩家、第一關幾百步不當掉，而且四個選項都真的動過 |
//! | `shoot_predict_*` | 細步移動時，玩家機預測的步長和實際位移一致，邏輯步到來時畫面不跳 |
//! | `shoot_shots_*` | 存波動砲與砲座分離的畫面（`RS2_OUT/enhance-shots`），給人看 |
//!
//! 這個檔是 `session` 的子模組，可以讀 `Session` 的私有欄位（`game`、`playable`）。

use super::*;
use crate::ui::hud::HudFlags;
use rs2_core::{Rules, a as dseg};
use rs2_trace::load_inputs;
use std::path::PathBuf;
use std::sync::OnceLock;

const FIRE_P1: u8 = 0x36;
const DOWN_P1: u8 = 0x50;
const RIGHT_P1: u8 = 0x4D;
/// 玩家 1 的上、下、左、右與右 Shift（set 1 碼）。
const P1: [u8; 5] = [0x48, DOWN_P1, 0x4B, RIGHT_P1, FIRE_P1];
/// 玩家 2 的 T、B、F、H 與左 Shift。
const P2: [u8; 5] = [0x14, 0x30, 0x21, 0x23, 0x2A];

fn orig() -> PathBuf {
    PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh play-test 跑"))
}

fn inputs() -> &'static Inputs {
    static I: OnceLock<Inputs> = OnceLock::new();
    I.get_or_init(|| load_inputs(&orig()).unwrap())
}

const SD_VIEW: ViewOpts = ViewOpts { tween: true, counter: false, hd: false, feel: true, mode: DisplayMode::Vga };
const HD_VIEW: ViewOpts = ViewOpts { tween: true, counter: false, hd: true, feel: true, mode: DisplayMode::Vga };

fn open(view: ViewOpts) -> Session {
    let assets = Assets::load(&orig()).unwrap();
    Session::new(inputs(), assets, None, None, Begin::Stage1(Start::NewGame { difficulty: None }), Fixes::ALL, view)
}

/// 有 `RS2_HD` 就開 HD 畫面，沒有就退回原版圖。
fn open_shot() -> Session {
    let Some(hd_path) = std::env::var("RS2_HD").ok().map(PathBuf::from) else { return open(SD_VIEW) };
    let assets = Assets::load(&orig()).unwrap();
    let mut hd = HdAssets::open(&hd_path).unwrap();
    hd.preload(&assets, 4);
    Session::new(inputs(), assets, None, Some(hd), Begin::Stage1(Start::NewGame { difficulty: None }), Fixes::ALL, HD_VIEW)
}

/// 跑到玩家機可以控制而且畫面有東西。
fn until_playable(s: &mut Session) {
    for _ in 0..400 {
        s.step(&[]);
        if s.playable(0) && !s.hold {
            for _ in 0..3 {
                s.step(&[]);
            }
            return;
        }
    }
    panic!("400 步內玩家機沒有可以控制");
}

/// 玩家 1 的裝備：兩座砲座與武器字母。
fn arm(s: &mut Session, weapon: u8) {
    s.game.poke16(dseg(0x1DAA9), u16::from(weapon));
    s.game.poke8(dseg(0x1DA8D), b'Y');
    s.game.poke8(dseg(0x1DA8E), b'Y');
}

/// 玩家 2 在場，帶兩座砲座與散彈。
fn bring_player_two(s: &mut Session) {
    s.game.poke8(dseg(0x1CFDD), 0x11);
    s.game.poke16(dseg(0x1DB0B), u16::from(b'S'));
    s.game.poke8(dseg(0x1DAEF), b'Y');
    s.game.poke8(dseg(0x1DAF0), b'Y');
}

/// 腳本輸入：只看步數 `n`，所以兩次執行吃到的輸入完全相同。每 17 步換一組方向，每 40 步一輪開火
/// （第 3 到 22 步按住，第 30 步輕點），分離鍵見 [`split_at`]。
fn keys_at(n: u32, codes: [u8; 5]) -> Vec<u8> {
    let [up, down, left, right, fire] = codes;
    let dirs: [&[u8]; 6] = [&[right], &[right, down], &[left], &[up], &[left, up], &[down]];
    let mut k = dirs[(n / 17 % 6) as usize].to_vec();
    let r = n % 40;
    if (3..23).contains(&r) || r == 30 {
        k.push(fire);
    }
    k
}

/// 每 48 步按三次分離鍵（第 12、26、40 步，各一步）。
fn split_at(n: u32) -> bool {
    matches!(n % 48, 12 | 26 | 40)
}

/// 腳本把「現在該按著的鍵」換成這一步的掃描碼：新按的送按下碼，放開的送放開碼。
struct Keyboard {
    down: Vec<u8>,
}

impl Keyboard {
    fn new() -> Keyboard {
        Keyboard { down: Vec::new() }
    }

    fn step(&mut self, want: &[u8]) -> Vec<u8> {
        let mut sc: Vec<u8> = self.down.iter().filter(|c| !want.contains(c)).map(|c| c | 0x80).collect();
        sc.extend(want.iter().filter(|c| !self.down.contains(c)).copied());
        self.down = want.to_vec();
        sc
    }
}

/// 一局的記錄：每步的雜湊，加上跑的過程中有沒有看到各選項的作用。
#[derive(Default)]
struct Run {
    hashes: Vec<String>,
    max_charge: u16,
    detached_steps: u32,
    half_row_steps: u32,
    even_col_steps: u32,
}

/// 跑 `steps` 步腳本。`two` 為真時玩家 2 也在場並吃另一份腳本。
fn run(rules: Rules, steps: u32, two: bool) -> Run {
    let mut s = open(SD_VIEW);
    s.set_rules(rules);
    until_playable(&mut s);
    arm(&mut s, b'K');
    if two {
        bring_player_two(&mut s);
    }
    let (mut k1, mut k2) = (Keyboard::new(), Keyboard::new());
    let mut out = Run::default();
    for n in 0..steps {
        // 不死，才跑得完整段。
        s.set_invincible(true);
        let mut sc = k1.step(&keys_at(n, P1));
        if two {
            sc.extend(k2.step(&keys_at(n + 7, P2)));
        }
        s.set_split_held([split_at(n), two && split_at(n + 5)]);
        s.step(&sc);
        out.hashes.push(s.dseg_hash());
        let st = s.game.player_status(0);
        out.max_charge = out.max_charge.max(s.game.hud_status(0).charge);
        out.detached_steps += u32::from(st.pods_detached);
        out.half_row_steps += u32::from(st.y % 10 != 0);
        out.even_col_steps += u32::from(st.x % 2 == 0);
    }
    out
}

const WAVE: Rules = Rules { wave_charge: true, ..Rules::ORIGINAL };
const AUTO: Rules = Rules { auto_fire: true, ..Rules::ORIGINAL };
const FORCE: Rules = Rules { force_pod: true, ..Rules::ORIGINAL };
const FINE: Rules = Rules { fine_step: true, ..Rules::ORIGINAL };
const ALL: Rules = Rules { wave_charge: true, auto_fire: true, force_pod: true, fine_step: true, ..Rules::ORIGINAL };

/// E2：每個選項開啟時，同一份輸入與選項跑兩次，每步雜湊相同。E3：開與關的雜湊不同（行為確實改變）。
#[test]
#[ignore]
fn shoot_e2_each_option_is_deterministic_and_changes_the_run() {
    const STEPS: u32 = 240;
    let original = run(Rules::ORIGINAL, STEPS, false);
    assert_eq!(original.hashes.len(), STEPS as usize);
    for (name, rules) in [("波動砲充能", WAVE), ("按住連射", AUTO), ("砲座像 Force", FORCE), ("細步移動", FINE), ("全開", ALL)] {
        let a = run(rules, STEPS, false);
        let b = run(rules, STEPS, false);
        assert_eq!(a.hashes, b.hashes, "{name}：同樣的輸入與選項，兩次的雜湊要相同");
        let first_diff = a.hashes.iter().zip(&original.hashes).position(|(x, y)| x != y);
        assert!(first_diff.is_some(), "{name}：開啟後行為要和原版不同");
        eprintln!("[shoot] {name}：第 {} 步起和原版不同", first_diff.unwrap());
    }
}

/// 選項分別證明自己動過：充能到過滿、砲座分離過、出現過半步位置。
#[test]
#[ignore]
fn shoot_e3_each_option_leaves_its_mark_in_the_state_it_owns() {
    let wave = run(WAVE, 200, false);
    assert_eq!(wave.max_charge, rs2_core::WAVE_CHARGE_FULL, "按住 20 步，充能到滿格");
    let orig = run(Rules::ORIGINAL, 200, false);
    assert_eq!((orig.detached_steps, orig.half_row_steps, orig.even_col_steps), (0, 0, 0), "原版：砲座不分離、位置永遠是整格和奇數欄");
    let force = run(FORCE, 200, false);
    assert!(force.detached_steps > 10, "分離鍵讓砲座分離過：{}", force.detached_steps);
    let fine = run(FINE, 200, false);
    assert!(fine.half_row_steps > 5 && fine.even_col_steps > 5, "細步出現過半列、偶數欄：{} {}", fine.half_row_steps, fine.even_col_steps);
}

/// E1：選項全關時，經過強化介面的一局和完全沒碰過的一局，每步雜湊與畫面相同。
/// （與原版收據逐位元組比對由 `tools/rust.sh verify` 的對拍做，那裡不經過 `Session`。）
#[test]
#[ignore]
fn shoot_e1_original_rules_leave_the_run_untouched() {
    let mut quiet = open(SD_VIEW);
    let mut touched = open(SD_VIEW);
    // 先開全部再關掉：核心記住的是最後一次的規則。
    touched.set_rules(ALL);
    touched.set_rules(Rules::ORIGINAL);
    until_playable(&mut quiet);
    until_playable(&mut touched);
    assert_eq!(quiet.dseg_hash(), touched.dseg_hash());
    arm(&mut quiet, b'K');
    arm(&mut touched, b'K');
    let (mut ka, mut kb) = (Keyboard::new(), Keyboard::new());
    for n in 0..260 {
        quiet.set_invincible(true);
        touched.set_invincible(true);
        let want = keys_at(n, P1);
        let (a, b) = (ka.step(&want), kb.step(&want));
        // 分離鍵照按，選項關著核心不看。
        touched.set_split_held([split_at(n), split_at(n + 3)]);
        quiet.step(&a);
        touched.step(&b);
        assert_eq!(quiet.dseg_hash(), touched.dseg_hash(), "第 {n} 步的雜湊");
        let fa = quiet.render(1.0).0.to_vec();
        let fb = touched.render(1.0).0.to_vec();
        assert_eq!(fa, fb, "第 {n} 步的畫面");
    }
    assert!(!quiet.game.player_status(0).pods_detached);
    assert!(!touched.game.player_status(0).pods_detached);
}

/// 組合：四個選項全開、兩位玩家、第一關 600 步不當掉，而且每個選項都動過。
#[test]
#[ignore]
fn shoot_combo_all_four_options_with_two_players_run_the_first_stage() {
    let r = run(ALL, 600, true);
    assert_eq!(r.hashes.len(), 600);
    assert_eq!(r.max_charge, rs2_core::WAVE_CHARGE_FULL);
    assert!(r.detached_steps > 20, "砲座分離過 {} 步", r.detached_steps);
    assert!(r.half_row_steps > 10 && r.even_col_steps > 10);
    let distinct: std::collections::HashSet<&String> = r.hashes.iter().collect();
    assert!(distinct.len() > 300, "局面一直在變：{}", distinct.len());
    // 同樣的輸入再跑一次，雜湊相同（兩位玩家、四個選項）。
    assert_eq!(r.hashes, run(ALL, 600, true).hashes);
}

/// 四種顯示模式（VGA、EGA、CGA、MGA）都能玩四個選項，而且顯示模式不影響模擬：同一份輸入，每步雜湊相同；
/// 每步都畫（單色的 `MonoSink` 與 `LayerSink` 都走過）不當掉。第一關不讀地形，所以模式不同也不會分歧。
#[test]
#[ignore]
fn shoot_modes_all_options_play_in_every_display_mode() {
    let mono = Arc::new(MonoAssets::load(&orig()).expect("單色圖庫載入失敗"));
    let mut runs: Vec<(DisplayMode, Vec<String>)> = Vec::new();
    for mode in [DisplayMode::Vga, DisplayMode::Ega, DisplayMode::Cga, DisplayMode::Mga] {
        let assets = Assets::load(&orig()).unwrap();
        let view = ViewOpts { tween: false, counter: false, hd: false, feel: true, mode };
        let mut s = Session::new(inputs(), assets, Some(Arc::clone(&mono)), None, Begin::Stage1(Start::NewGame { difficulty: None }), Fixes::ALL, view);
        s.set_rules(ALL);
        until_playable(&mut s);
        arm(&mut s, b'K');
        let mut k = Keyboard::new();
        let mut hashes = Vec::new();
        for n in 0..160 {
            s.set_invincible(true);
            s.set_split_held([split_at(n), false]);
            s.step(&k.step(&keys_at(n, P1)));
            hashes.push(s.dseg_hash());
            let (px, w, h) = s.render(1.0);
            assert_eq!(px.len(), w * h * 4);
        }
        assert_eq!(s.mode(), mode, "{mode:?} 沒被退回");
        runs.push((mode, hashes));
    }
    for (mode, hashes) in &runs[1..] {
        assert_eq!(hashes, &runs[0].1, "{mode:?} 的雜湊要和 VGA 相同");
    }
}

/// 細步移動時的玩家機預測：按住右，每個邏輯步開始前，預測用的步長（`next_step`）要等於這一步實際走的距離，
/// 而且預測領先量在邏輯步到來時剛好被實際位移吃掉，畫面不跳。第一步半步（8 像素），之後整步（16 像素）。
#[test]
#[ignore]
fn shoot_predict_fine_step_matches_the_logic_position_at_each_step() {
    let mut s = open(SD_VIEW);
    s.set_rules(FINE);
    until_playable(&mut s);
    s.live_input = true;
    let step_secs = 1.0 / 9.1;
    let mut state = [false; 128];
    state[RIGHT_P1 as usize] = true;
    let mut sizes = Vec::new();
    for n in 0..10 {
        let before = s.game.player_status(0);
        assert!(before.fine_step && before.can_right);
        let want = crate::predict::next_step(before.fine_step, before.last_move, crate::predict::dir_of(&state, 0));
        // 一整步的時間，分成幾次繪製。
        for _ in 0..6 {
            s.predict(&state, step_secs / 6.0, step_secs);
        }
        let lead = s.predictor.offset_hd(0).expect("可以控制時有預測").0 as f32 / 4.0;
        s.step(if n == 0 { &[RIGHT_P1] } else { &[] });
        let after = s.game.player_status(0);
        let moved = ((after.x - before.x) * 8) as f32;
        assert_eq!(moved, want.x, "第 {n} 步：實際走 {moved}，預測的步長 {}", want.x);
        assert!((lead - moved).abs() <= 0.5, "第 {n} 步：預測領先 {lead}，實際走 {moved}");
        let now = s.predictor.offset_hd(0).expect("有預測").0 as f32 / 4.0;
        assert!(now.abs() <= 0.5, "第 {n} 步到來時畫面跳了 {now} 像素");
        sizes.push(moved);
    }
    assert_eq!(sizes[..3], [8.0, 16.0, 16.0], "新按半步，之後整步");
}

// ---------------------------------------------------------------------------
// 截圖：RS2_OUT/enhance-shots，給人看（手感、強度不在這裡驗）。
// ---------------------------------------------------------------------------

fn shots_dir() -> Option<PathBuf> {
    std::env::var("RS2_OUT").ok().map(|d| PathBuf::from(d).join("enhance-shots")).inspect(|d| std::fs::create_dir_all(d).unwrap())
}

fn save(s: &mut Session, name: &str) {
    let Some(dir) = shots_dir() else { return };
    let (px, w, h) = s.render(1.0);
    std::fs::write(dir.join(name), rs2_gfx::png::encode_rgba(w, h, px)).unwrap();
}

/// 波動砲三級與充能條，砲座分離與收回，細步的半列位置。
#[test]
#[ignore]
fn shoot_shots_wave_charge_and_force_pods() {
    // 波動砲：新按後按住 N 個按住步放開，放開那一步畫面就有波動砲。
    for (name, held) in [("wave_l1", 5u64), ("wave_l2", 9), ("wave_l3", 14)] {
        let mut s = open_shot();
        s.set_rules(WAVE);
        s.set_hud_flags(HudFlags { rules: true, god: false, charge_bar: true, counter: None });
        until_playable(&mut s);
        s.set_clock_ms(1000);
        s.set_invincible(true);
        s.step(&[FIRE_P1]);
        for i in 0..held {
            s.set_clock_ms(1000 + 110 * i);
            s.step(&[]);
        }
        save(&mut s, &format!("{name}_charged.png"));
        s.step(&[FIRE_P1 | 0x80]);
        save(&mut s, &format!("{name}_fired.png"));
        s.step(&[]);
        save(&mut s, &format!("{name}_fired_next.png"));
    }
    // 砲座分離：兩座砲座與大彈，機身往下走，砲座留在前面。
    let mut s = open_shot();
    s.set_rules(FORCE);
    until_playable(&mut s);
    arm(&mut s, b'K');
    s.set_invincible(true);
    s.step(&[]);
    save(&mut s, "force_docked.png");
    s.set_split_held([true, false]);
    s.step(&[]);
    s.set_split_held([false, false]);
    for _ in 0..3 {
        s.step(&[DOWN_P1]);
    }
    s.step(&[FIRE_P1]);
    save(&mut s, "force_detached.png");
    assert!(s.game.player_status(0).pods_detached);
    s.step(&[FIRE_P1 | 0x80, DOWN_P1 | 0x80]);
    s.set_split_held([true, false]);
    s.step(&[]);
    s.set_split_held([false, false]);
    for _ in 0..2 {
        s.step(&[]);
    }
    save(&mut s, "force_recalling.png");
    for _ in 0..6 {
        s.step(&[]);
    }
    save(&mut s, "force_redocked.png");
    // 細步：半列位置的機身。
    let mut s = open_shot();
    s.set_rules(FINE);
    until_playable(&mut s);
    s.step(&[DOWN_P1]);
    s.step(&[DOWN_P1 | 0x80]);
    save(&mut s, "fine_half_row.png");
    assert_eq!(s.game.player_status(0).y % 10, 5);
    // 原貌（VgaSink，補間關）：細步與分離的砲座在逐像素的原版畫面裡不留殘影。
    let mut s = open(ViewOpts { tween: false, counter: false, hd: false, feel: true, mode: DisplayMode::Vga });
    s.set_rules(Rules { force_pod: true, fine_step: true, ..Rules::ORIGINAL });
    until_playable(&mut s);
    arm(&mut s, b'K');
    s.set_invincible(true);
    s.set_split_held([true, false]);
    s.step(&[]);
    s.set_split_held([false, false]);
    for n in 0..9 {
        s.step(&if n == 0 { vec![DOWN_P1, RIGHT_P1] } else { Vec::new() });
    }
    s.step(&[DOWN_P1 | 0x80]);
    s.step(&[]);
    save(&mut s, "vga_fine_force.png");
}
