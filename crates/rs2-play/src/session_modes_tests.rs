//! 顯示模式（VGA、EGA、CGA、MGA）的測試：切換不影響模擬，切換當下的畫面正確，每個模式的輸出尺寸與內容。
//!
//! 需要原版檔，標 `#[ignore]`，由 `tools/rust.sh play-test` 帶 `--include-ignored` 跑。比對筆數為 0 視為失敗。
//! 這個檔是 `session` 的子模組，可以讀 `Session` 的私有欄位（`game`、`playable`）。
//!
//! | 測試 | 內容 |
//! |---|---|
//! | `m1_*` | 同一份輸入，一個全程 VGA，一個一路循環切換四種模式，第一到第四關與片尾每步的 `Dseg` 雜湊相同 |
//! | `m2_*` | 第三關關內切換：進場模式固定（VGA、CGA 各一次），關內循環切換，每步雜湊與不切換的相同 |
//! | `m3_*` | 進場模式不同則第三關地形不同（已決定的行為）：靜態比兩份地圖陣列，動態比雜湊分歧點 |
//! | `m4_*` | 在第 N 步切到模式 M，之後每一步的 `render` 與一開始就用 M 的 Session 逐位元組相同（四種模式兩兩） |
//! | `m5_*` | 每個模式的輸出尺寸、內容（單色只有黑白，EGA 只有 EGA 色表的顏色）、不是全黑 |
//! | `m6_*` | CGA、MGA 沒有喇叭附帶音，VGA、EGA 照舊；中途切到單色，喇叭音就停 |
//! | `m7_*` | 單色圖庫缺檔時 CGA、MGA 被拒絕，VGA 不受影響 |
//! | `m8_*` | 有 HD 資料夾才跑：EGA 加 HD 量化到 EGA 色表，單色模式 HD 用 `MonoHdSink`（1280×800、只有黑白） |

use super::*;
use rs2_core::a as dseg;
use rs2_formats::unrle;
use rs2_trace::load_inputs;
use std::path::PathBuf;
use std::sync::OnceLock;

const RIGHT: u8 = 0x4D;
const LEFT: u8 = 0x4B;
const FIRE_P1: u8 = 0x36;
/// 每一關按 Ctrl+C 之前可以控制的步數。夠長，打得到每一關中段的敵人與地形（頭目在更後面）。
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

fn view(mode: DisplayMode) -> ViewOpts {
    ViewOpts { tween: false, counter: false, hd: false, feel: false, mode }
}

fn open(begin: Begin, v: ViewOpts) -> Session {
    let assets = Assets::load(&orig()).unwrap();
    Session::new(inputs(), assets, Some(mono_assets()), None, begin, Fixes::ALL, v)
}

fn open_without_mono(begin: Begin, v: ViewOpts) -> Session {
    let assets = Assets::load(&orig()).unwrap();
    Session::new(inputs(), assets, None, None, begin, Fixes::ALL, v)
}

fn new_game() -> Begin {
    Begin::Stage1(Start::NewGame { difficulty: None })
}

/// 目前的關卡字母（`byte_1CFDB`）：`A` 到 `D`。
fn letter(s: &Session) -> u8 {
    s.game.state_hash_input()[dseg(0x1CFDB) as usize]
}

/// 這個模式與補間設定下 `render` 該有的尺寸（沒有 HD）。
fn size_of(mode: DisplayMode) -> (usize, usize) {
    match mode {
        DisplayMode::Vga | DisplayMode::Ega => (320, 200),
        DisplayMode::Cga => (640, 200),
        DisplayMode::Mga => (640, 300),
    }
}

/// 不死（`word_273A5` 的作弊旗標），整個流程才走得完。兩個 Session 都要寫。
fn invincible(s: &mut Session) {
    s.game.poke8(dseg(0x273A5), b'Y');
}

/// 從標題走完四關與片尾的輸入：可以控制時左右走並按住開火，走滿 `PLAY_STEPS` 步就用 Ctrl+C 過關，其餘時間隔一段時間按空白鍵。
/// 只看基準 Session 的狀態，所以兩個 Session 吃的輸入完全相同。
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

fn is_ended(o: Outcome) -> bool {
    matches!(o, Outcome::Ended(_))
}

/// 跑到玩家機可以控制而且畫面有東西（開場的淡入結束），再多幾步。
fn warm_up(s: &mut Session) {
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

// ---------------------------------------------------------------------------
// m1：切換不影響模擬（四關與片尾）
// ---------------------------------------------------------------------------

/// 一個 Session 全程 VGA，另一個每 7 步換下一個模式（補間與幀計數也輪流開關），每步 `Dseg` 雜湊相同。
///
/// 第三關的地形來源在進場時決定（已決定的取捨），所以從第二關按 Ctrl+C 到第三關進場那一步為止，
/// 切換的那一份固定用 EGA（和基準同樣讀 `SHT.SCN`）；進場之後（第三關關內、第四關、片尾）照樣循環四種模式。
#[test]
#[ignore]
fn m1_switching_modes_never_changes_the_simulation_in_all_four_stages() {
    let mut base = open(Begin::Title, view(DisplayMode::Vga));
    let mut sw = open(Begin::Title, view(DisplayMode::Vga));
    let mut drv = Driver::new();
    let mut seen = [false; 4];
    let mut leaving_stage2 = false;
    let mut tail = 0u32;
    let mut per_mode = [0usize; 4];
    let mut compared = 0usize;
    let mut stage_steps = [0usize; 4];
    let (mut mono_anom, mut anomalies_by_stage) = (0u64, [0u64; 4]);
    for n in 0..60_000u32 {
        let (sc, left) = drv.next(&base);
        let before = letter(&base);
        if left && before == b'B' {
            leaving_stage2 = true;
        }
        if left && before == b'D' {
            tail = 2500;
        }
        let mode = if leaving_stage2 && before != b'C' { DisplayMode::Ega } else { DisplayMode::ALL[(n as usize / 7) % 4] };
        sw.set_view(ViewOpts { tween: (n / 11) % 2 == 0, counter: (n / 13) % 2 == 0, hd: false, feel: true, mode });
        assert_eq!(sw.mode(), mode);
        invincible(&mut base);
        invincible(&mut sw);
        let oa = base.step(&sc);
        let ob = sw.step(&sc);
        assert_eq!(base.dseg_hash(), sw.dseg_hash(), "第 {n} 步（模式 {mode:?}，關卡 {}）的 Dseg 雜湊不同", letter(&base) as char);
        compared += 1;
        let after = letter(&base);
        if after == b'C' {
            leaving_stage2 = false;
        }
        if let Some(i) = (after as usize).checked_sub(b'A' as usize).filter(|i| *i < 4) {
            seen[i] = true;
            stage_steps[i] += 1;
        }
        // 單色接收端「無法照機器碼重現」的次數增加了，記下是在哪一關（診斷用，不影響通過與否）。
        let now = sw.mono_anomalies().map(|(c, m)| c + m).unwrap_or(0);
        if now > mono_anom {
            let i = (after as usize).saturating_sub(b'A' as usize).min(3);
            anomalies_by_stage[i] += now - mono_anom;
            mono_anom = now;
        }
        let (px, w, h) = sw.render(1.0);
        assert_eq!((w, h), size_of(mode), "第 {n} 步 {mode:?} 的輸出尺寸");
        assert_eq!(px.len(), w * h * 4);
        per_mode[DisplayMode::ALL.iter().position(|m| *m == mode).unwrap()] += 1;
        assert_eq!(is_ended(oa), is_ended(ob), "第 {n} 步兩邊的結束狀態不同");
        if is_ended(oa) {
            break;
        }
        if tail > 0 {
            tail -= 1;
            if tail == 0 {
                break;
            }
        }
    }
    eprintln!("[m1] 比對 {compared} 步，各關步數 {stage_steps:?}，各模式 render 次數 {per_mode:?}");
    assert!(compared > 1000, "比對筆數 {compared} 太少");
    assert_eq!(seen, [true; 4], "四關都要走到");
    assert!(per_mode.iter().all(|c| *c > 100), "每個模式都要真的畫過：{per_mode:?}");
    assert!(stage_steps.iter().all(|c| *c > 100), "每一關都要有足夠的步數：{stage_steps:?}");
    if let Some((c, m)) = sw.mono_anomalies() {
        eprintln!("[m1] 單色接收端異常計數 CGA {c}、MGA {m}，依關卡（A 到 D）增加的次數 {anomalies_by_stage:?}");
        if let Some(mono) = &sw.mono {
            for line in mono.cga.anomaly_log().iter().take(6) {
                eprintln!("[m1] CGA 異常：{line}");
            }
        }
    }
    let (va, la) = sw.anomalies();
    eprintln!("[m1] VgaSink 異常 {va}、LayerSink 異常 {la}");
}

// ---------------------------------------------------------------------------
// m2：第三關關內切換
// ---------------------------------------------------------------------------

/// 進第三關之前兩邊都固定在 `entry`，進場之後切換的那一份在關內每 5 步換下一個模式，每步雜湊與不切換的相同。
fn stage3_in_stage_switching(entry: DisplayMode) {
    let mut base = open(Begin::Title, view(entry));
    let mut sw = open(Begin::Title, view(entry));
    let mut drv = Driver::new();
    let mut in_stage3 = 0u32;
    let mut per_mode = [0usize; 4];
    let mut compared = 0usize;
    let mut done = false;
    for n in 0..40_000u32 {
        let (sc, _) = drv.next(&base);
        // 字母在進場那一步才變成 C，所以進場那一步仍是固定的 entry。
        let in3 = letter(&base) == b'C';
        let mode = if in3 { DisplayMode::ALL[(in_stage3 as usize / 5) % 4] } else { entry };
        sw.set_view(view(mode));
        invincible(&mut base);
        invincible(&mut sw);
        let oa = base.step(&sc);
        sw.step(&sc);
        assert_eq!(base.dseg_hash(), sw.dseg_hash(), "進場 {entry:?}：第 {n} 步（第三關第 {in_stage3} 步，模式 {mode:?}）雜湊不同");
        compared += 1;
        if letter(&base) == b'C' {
            in_stage3 += 1;
            per_mode[DisplayMode::ALL.iter().position(|m| *m == mode).unwrap()] += 1;
            let (_, w, h) = sw.render(1.0);
            assert_eq!((w, h), size_of(mode));
        } else if in3 {
            // 第三關結束（字母變成 D）。
            done = true;
            break;
        }
        if is_ended(oa) {
            break;
        }
    }
    eprintln!("[m2] 進場 {entry:?}：比對 {compared} 步，第三關 {in_stage3} 步，各模式 {per_mode:?}");
    assert!(done, "第三關沒有走完");
    assert!(in_stage3 >= 150, "第三關只跑了 {in_stage3} 步");
    assert!(compared > 0);
    // 關內有切換到的模式至少三種（進場那一種之外的，加上它自己）。
    assert!(per_mode.iter().filter(|c| **c > 0).count() == 4, "關內要四種模式都用過：{per_mode:?}");
}

#[test]
#[ignore]
fn m2_stage3_in_stage_switching_after_a_vga_entry() {
    stage3_in_stage_switching(DisplayMode::Vga);
}

#[test]
#[ignore]
fn m2_stage3_in_stage_switching_after_a_cga_entry() {
    stage3_in_stage_switching(DisplayMode::Cga);
}

// ---------------------------------------------------------------------------
// m3：進場模式決定第三關地形
// ---------------------------------------------------------------------------

/// 地圖陣列是 600 欄 × 20 列的字組（24,005 位元組解壓後的前 24,000 位元組）。
fn map_words(raw: &[u8]) -> Vec<u16> {
    let bytes = unrle(raw, 0);
    bytes.chunks_exact(2).take(12_000).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
}

#[test]
#[ignore]
fn m3_the_mode_at_stage3_entry_picks_the_terrain() {
    // 靜態：兩份地圖陣列在零與非零上有差異（docs/re/14-display-modes.md 5.3 記為 514 格）。
    let scn = map_words(&inputs().files["SHT.SCN"]);
    let scr = map_words(&inputs().files["SHT.SCR"]);
    assert_eq!((scn.len(), scr.len()), (12_000, 12_000));
    let differing = scn.iter().zip(&scr).filter(|(x, y)| (**x == 0) != (**y == 0)).count();
    eprintln!("[m3] SHT.SCN 與 SHT.SCR 零與非零不同的格數：{differing}");
    assert!(differing > 0, "兩份地圖陣列的地形沒有差異，這個測試沒有鑑別力");

    // 動態：同一份輸入，進場時 VGA 與進場時 CGA 的第三關。進場之前每步雜湊相同，進場之後某一步起分歧。
    let mut vga = open(Begin::Title, view(DisplayMode::Vga));
    let mut cga = open(Begin::Title, view(DisplayMode::Cga));
    let mut drv = Driver::new();
    let (mut entry, mut first_div, mut stage3_steps) = (None, None, 0u32);
    let mut equal_before = 0usize;
    for n in 0..40_000u32 {
        let (sc, _) = drv.next(&vga);
        invincible(&mut vga);
        invincible(&mut cga);
        vga.step(&sc);
        cga.step(&sc);
        let same = vga.dseg_hash() == cga.dseg_hash();
        if letter(&vga) == b'C' {
            if entry.is_none() {
                entry = Some(n);
            }
            stage3_steps += 1;
        }
        if same {
            if first_div.is_none() {
                equal_before += 1;
            }
        } else if first_div.is_none() {
            first_div = Some(n);
            // 分歧的位元組要落在 G4（dseg:900Ch 起 1,600 位元組）。
            let (x, y) = (vga.game.state_hash_input(), cga.game.state_hash_input());
            let diff: Vec<usize> = (0..x.len()).filter(|&i| x[i] != y[i]).collect();
            let in_g4 = diff.iter().filter(|&&i| (0x900C..0x900C + 1600).contains(&i)).count();
            eprintln!("[m3] 第 {n} 步首次分歧（進場第 {} 步）：{} 個位元組不同，其中 {in_g4} 個在 G4", stage3_steps, diff.len());
            assert!(in_g4 > 0, "首次分歧的位元組沒有落在地形格 G4");
        }
        // 第三關結束（字母離開 C 進到 D）或已經跑夠。
        if stage3_steps >= 400 || (entry.is_some() && letter(&vga) == b'D') {
            break;
        }
    }
    let entry = entry.expect("沒有走到第三關");
    let div = first_div.expect("進場模式不同，第三關的雜湊卻完全沒有分歧：地形來源沒有隨進場模式");
    assert!(div >= entry, "分歧（第 {div} 步）不能早於進場（第 {entry} 步）：進場之前模擬與模式無關");
    assert!(equal_before > 500, "進場之前只比了 {equal_before} 步");
    eprintln!("[m3] 進場在第 {entry} 步，首次分歧在第 {div} 步，分歧之前 {equal_before} 步雜湊相同");
}

// ---------------------------------------------------------------------------
// m4：切換當下顯示正確
// ---------------------------------------------------------------------------

/// 兩份 Session 吃同一份輸入。固定的腳本：往右走，開火，再往左。
fn script(n: u32) -> Vec<u8> {
    match n {
        10 => vec![RIGHT],
        15 => vec![FIRE_P1],
        50 => vec![RIGHT | 0x80, LEFT],
        90 => vec![LEFT | 0x80],
        110 => vec![FIRE_P1 | 0x80],
        _ => Vec::new(),
    }
}

#[test]
#[ignore]
fn m4_switching_to_a_mode_looks_exactly_like_starting_in_it() {
    let mut pairs = 0usize;
    let mut compared = 0usize;
    for (name, begin, at, extra) in [("第一關", new_game(), 140u32, 40u32), ("標題", Begin::Title, 250, 40)] {
        for from in DisplayMode::ALL {
            for to in DisplayMode::ALL {
                if from == to {
                    continue;
                }
                for tween in [false, true] {
                    let v = |mode| ViewOpts { tween, ..view(mode) };
                    let mut s = open(begin, v(from));
                    let mut r = open(begin, v(to));
                    let mut n_cmp = 0usize;
                    for n in 0..at + extra {
                        if n == at {
                            s.set_view(v(to));
                            assert_eq!(s.mode(), to);
                            // 切換的當下（還沒跑下一步）就要等於一開始就用這個模式的畫面。
                            for alpha in [0.5f32, 1.0] {
                                let (x, y) = (s.render(alpha).0.to_vec(), r.render(alpha).0.to_vec());
                                assert!(x == y, "{name} {from:?}→{to:?} tween {tween}：切換當下 alpha {alpha} 的畫面不同");
                                n_cmp += 1;
                            }
                            assert_eq!(s.fit(), r.fit());
                        }
                        let sc = script(n);
                        s.step(&sc);
                        r.step(&sc);
                        assert_eq!(s.dseg_hash(), r.dseg_hash(), "{name} {from:?}→{to:?}：模擬不能受切換影響");
                        if n >= at {
                            for alpha in [0.5f32, 1.0] {
                                let (sa, sw, sh) = {
                                    let (p, w, h) = s.render(alpha);
                                    (p.to_vec(), w, h)
                                };
                                let (ra, rw, rh) = {
                                    let (p, w, h) = r.render(alpha);
                                    (p.to_vec(), w, h)
                                };
                                assert_eq!((sw, sh), (rw, rh), "{name} {from:?}→{to:?} 第 {n} 步尺寸");
                                assert!(sa == ra, "{name} {from:?}→{to:?} tween {tween} 第 {n} 步 alpha {alpha} 的畫面不同");
                                n_cmp += 1;
                            }
                        }
                    }
                    assert!(n_cmp > 0, "{name} {from:?}→{to:?} 沒有比對");
                    compared += n_cmp;
                    pairs += 1;
                }
            }
        }
    }
    eprintln!("[m4] {pairs} 組切換，逐位元組比對 {compared} 張畫面，全部相同");
    assert_eq!(pairs, 2 * 12 * 2);
    assert!(compared > 0);
}

// ---------------------------------------------------------------------------
// m5：每個模式的輸出
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn m5_every_mode_renders_the_right_size_and_content() {
    let mut frames = 0usize;
    for mode in DisplayMode::ALL {
        for tween in [false, true] {
            let mut s = open(new_game(), ViewOpts { tween, ..view(mode) });
            warm_up(&mut s);
            for n in 0..40 {
                s.step(&fire_script(n));
            }
            let pal = s.vga.ega_colors();
            let want = size_of(mode);
            for alpha in [0.5f32, 1.0] {
                let fit = s.fit();
                let (px, w, h) = s.render(alpha);
                assert_eq!((w, h), want, "{mode:?} tween {tween}");
                assert_eq!(px.len(), w * h * 4);
                assert!(px.chunks_exact(4).all(|p| p[3] == 255), "{mode:?}：alpha 通道要全是 255");
                let lit = px.chunks_exact(4).filter(|p| p[0] != 0 || p[1] != 0 || p[2] != 0).count();
                assert!(lit > w * h / 100, "{mode:?} tween {tween}：幾乎全黑（{lit} / {}）", w * h);
                assert!(lit < w * h, "{mode:?}：不能全白或全亮");
                match mode {
                    DisplayMode::Cga | DisplayMode::Mga => {
                        assert!(px.chunks_exact(4).all(|p| (p[0] == 0 || p[0] == 255) && p[0] == p[1] && p[1] == p[2]), "{mode:?}：單色只有黑與白");
                        assert_eq!(fit, Fit::MONO, "{mode:?}");
                    }
                    DisplayMode::Ega => {
                        let in_pal = px.chunks_exact(4).all(|p| pal.iter().any(|c| c[..] == p[..3]));
                        assert!(in_pal, "EGA tween {tween}：有像素不在 EGA 色表內");
                        assert_eq!(fit, Fit::SD);
                    }
                    DisplayMode::Vga => assert_eq!(fit, Fit::SD),
                }
                frames += 1;
            }
        }
    }
    assert_eq!(frames, 16);
}

/// VGA 路徑沒有被動到：補間關時，輸出等於 `VgaSink::render_page` 的 RGB 加上不透明 alpha。
#[test]
#[ignore]
fn m5_vga_output_is_unchanged() {
    let mut s = open(new_game(), view(DisplayMode::Vga));
    warm_up(&mut s);
    for n in 0..40 {
        s.step(&fire_script(n));
    }
    let mut rgb = vec![0u8; 320 * 200 * 3];
    s.vga.render_page(s.layer.draw_page(), &mut rgb);
    let (px, w, h) = s.render(1.0);
    assert_eq!((w, h), (320, 200));
    let mut n = 0;
    for (d, src) in px.chunks_exact(4).zip(rgb.chunks_exact(3)) {
        assert!(d[..3] == src[..] && d[3] == 255);
        n += 1;
    }
    assert_eq!(n, 320 * 200);
}

/// EGA 補間關時是逐像素的 `render_page_ega`；補間開時，每個像素是 EGA 色表中離 LayerSink 輸出最近的一色。
#[test]
#[ignore]
fn m5_ega_tween_output_is_the_nearest_ega_colour_of_the_layer_output() {
    let mut s = open(new_game(), ViewOpts { tween: true, ..view(DisplayMode::Ega) });
    warm_up(&mut s);
    for n in 0..40 {
        s.step(&fire_script(n));
    }
    let pal = s.vga.ega_colors();
    // 用 VGA 的同一份補間輸出當輸入，自己算最近色。
    s.set_view(ViewOpts { tween: true, ..view(DisplayMode::Vga) });
    let raw = s.render(1.0).0.to_vec();
    s.set_view(ViewOpts { tween: true, ..view(DisplayMode::Ega) });
    let got = s.render(1.0).0.to_vec();
    assert_eq!(raw.len(), got.len());
    let mut checked = 0usize;
    let mut distinct = std::collections::HashSet::new();
    for (r, g) in raw.chunks_exact(4).zip(got.chunks_exact(4)) {
        let d = |c: &[u8; 3]| (0..3).map(|i| (r[i] as i32 - c[i] as i32).pow(2)).sum::<i32>();
        let best = pal.iter().map(d).min().unwrap();
        let chosen = d(&[g[0], g[1], g[2]]);
        assert_eq!(chosen, best, "像素 {r:?} 被量化成 {g:?}，不是最近的一色");
        assert_eq!(g[3], 255);
        distinct.insert([r[0], r[1], r[2]]);
        checked += 1;
    }
    assert_eq!(checked, 320 * 200);
    assert!(distinct.len() > 3, "輸入只有 {} 種顏色，這個測試沒有鑑別力", distinct.len());
}

// ---------------------------------------------------------------------------
// m6：喇叭附帶音
// ---------------------------------------------------------------------------

/// 一直開火的輸入：隔幾步按放一次，左右走動。
fn fire_script(n: u32) -> Vec<u8> {
    match n % 12 {
        0 => vec![FIRE_P1],
        3 => vec![FIRE_P1 | 0x80],
        _ => match n {
            10 => vec![RIGHT],
            80 => vec![RIGHT | 0x80, LEFT],
            _ => Vec::new(),
        },
    }
}

fn speaker_msgs(s: &mut Session) -> usize {
    s.take_audio().iter().filter(|m| matches!(m, AudioMsg::Speaker(_))).count()
}

#[test]
#[ignore]
fn m6_mono_modes_make_no_speaker_sound() {
    const STEPS: u32 = 700;
    let mut counts = Vec::new();
    for mode in DisplayMode::ALL {
        let mut s = open(new_game(), view(mode));
        let mut total = 0usize;
        for n in 0..STEPS {
            invincible(&mut s);
            let o = s.step(&fire_script(n));
            assert!(!is_ended(o), "{mode:?}：第 {n} 步第一關就結束了，測試要跑滿 {STEPS} 步");
            total += speaker_msgs(&mut s);
        }
        eprintln!("[m6] {mode:?}：{STEPS} 步，喇叭訊息 {total} 則");
        counts.push(total);
    }
    assert!(counts[0] > 0, "VGA 開火 {STEPS} 步卻沒有喇叭音，這個測試沒有鑑別力");
    assert_eq!(counts[1], counts[0], "EGA 的喇叭音和 VGA 相同");
    assert_eq!(counts[2], 0, "CGA 不該有喇叭音");
    assert_eq!(counts[3], 0, "MGA 不該有喇叭音");
}

/// 中途從 VGA 切到 CGA，喇叭音就停（基準一直 VGA，這段還有）；切回 VGA 之後兩邊又相同。
#[test]
#[ignore]
fn m6_switching_to_mono_mid_run_stops_the_speaker_and_back_restores_it() {
    const PH: u32 = 350;
    let mut base = open(new_game(), view(DisplayMode::Vga));
    let mut sw = open(new_game(), view(DisplayMode::Vga));
    // 三段：VGA、CGA、VGA 各自的（基準, 切換者）喇叭訊息數。
    let mut phase = [(0usize, 0usize); 3];
    for n in 0..3 * PH {
        let p = (n / PH) as usize;
        sw.set_view(view(if p == 1 { DisplayMode::Cga } else { DisplayMode::Vga }));
        invincible(&mut base);
        invincible(&mut sw);
        let (oa, ob) = (base.step(&fire_script(n)), sw.step(&fire_script(n)));
        assert!(!is_ended(oa) && !is_ended(ob), "第 {n} 步第一關就結束了，測試要跑滿 {} 步", 3 * PH);
        assert_eq!(base.dseg_hash(), sw.dseg_hash());
        let (b, w) = (speaker_msgs(&mut base), speaker_msgs(&mut sw));
        phase[p].0 += b;
        phase[p].1 += w;
        if p != 1 {
            assert_eq!(b, w, "VGA 期間兩邊的喇叭音相同（第 {n} 步）");
        } else {
            assert_eq!(w, 0, "單色期間不該有喇叭音（第 {n} 步）");
        }
    }
    eprintln!("[m6] 三段（基準, 切換者）：{phase:?}");
    assert!(phase[0].1 > 0, "切換之前要有喇叭音");
    assert!(phase[1].0 > 0, "基準在單色期間應該還有喇叭音，否則沒有鑑別力");
    assert_eq!(phase[1].1, 0);
}

// ---------------------------------------------------------------------------
// m7：單色圖庫缺檔
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn m7_without_the_mono_bank_cga_and_mga_are_refused_and_vga_still_works() {
    let mut s = open_without_mono(new_game(), view(DisplayMode::Cga));
    assert!(!s.mono_available());
    assert_eq!(s.mode(), DisplayMode::Vga, "要求 CGA 但沒有單色圖庫：退回 VGA");
    assert!(!s.set_mode(DisplayMode::Mga) && !s.set_mode(DisplayMode::Cga));
    assert_eq!(s.mode(), DisplayMode::Vga);
    assert!(s.mono_anomalies().is_none());
    for n in 0..60 {
        s.step(&script(n));
    }
    let (px, w, h) = s.render(1.0);
    assert_eq!((w, h), (320, 200));
    assert!(px.chunks_exact(4).any(|p| p[0] != 0 || p[1] != 0 || p[2] != 0));
    assert!(s.set_mode(DisplayMode::Ega) && s.mode() == DisplayMode::Ega, "EGA 不需要單色圖庫");
    let (_, w, h) = s.render(1.0);
    assert_eq!((w, h), (320, 200));
    // set_view 要求單色時也不 panic，維持原來的模式。
    s.set_view(view(DisplayMode::Mga));
    assert_eq!(s.mode(), DisplayMode::Ega);
    s.step(&[]);
    assert_eq!(s.render(1.0).1, 320);
}

/// 有單色圖庫時 `set_mode` 回傳真，並且是唯一影響第三關地形旗標的入口：
/// 這裡只確認模式本身；旗標對模擬的影響由 m2、m3 量。
#[test]
#[ignore]
fn m7_set_mode_reports_success_and_the_effective_mode() {
    let mut s = open(new_game(), view(DisplayMode::Vga));
    assert!(s.mono_available());
    for m in DisplayMode::ALL {
        assert!(s.set_mode(m));
        assert_eq!(s.mode(), m);
        assert_eq!(s.fit(), if m.is_mono() { Fit::MONO } else { Fit::SD });
    }
}

// ---------------------------------------------------------------------------
// m8：HD
// ---------------------------------------------------------------------------

/// 有 `RS2_HD` 才跑：EGA 加 HD 時輸出 1280×800 且只有 EGA 色表的顏色；單色模式 HD 用 `MonoHdSink`，
/// 輸出 1280×800，遊戲畫面只有黑白兩色，比例同單色的 4:3。HUD 蓋底部 40 列，不屬於遊戲畫面：EGA 不量化，
/// 單色轉灰階。打擊回饋關掉，光暈才不會混進遊戲畫面。
#[test]
#[ignore]
fn m8_hd_with_ega_is_quantized_and_mono_uses_the_mono_hd_sink() {
    let Ok(dir) = std::env::var("RS2_HD") else {
        eprintln!("[m8] 沒有 RS2_HD，略過");
        return;
    };
    let assets = Assets::load(&orig()).unwrap();
    let mut hd = HdAssets::open(&PathBuf::from(dir)).expect("開不了 HD 資料夾");
    hd.preload(&assets, 1);
    let v = |mode| ViewOpts { tween: true, hd: true, feel: false, counter: false, mode };
    let mut s = Session::new(inputs(), assets, Some(mono_assets()), Some(hd), new_game(), Fixes::ALL, v(DisplayMode::Vga));
    s.set_hit_feedback(false);
    for n in 0..120 {
        s.step(&script(n));
    }
    let pal = s.vga.ega_colors();
    for mode in DisplayMode::ALL {
        s.set_view(v(mode));
        let fit = s.fit();
        let (px, w, h) = s.render(1.0);
        let game = &px[..1280 * 760 * 4];
        match mode {
            DisplayMode::Vga => assert_eq!(((w, h), fit), ((1280, 800), Fit::HD)),
            DisplayMode::Ega => {
                assert_eq!(((w, h), fit), ((1280, 800), Fit::HD));
                assert!(game.chunks_exact(4).all(|p| pal.iter().any(|c| c[..] == p[..3])), "EGA 加 HD：有像素不在 EGA 色表內");
            }
            DisplayMode::Cga | DisplayMode::Mga => {
                assert_eq!(((w, h), fit), ((1280, 800), Fit::MONO), "{mode:?}：單色 HD");
                assert!(game.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2] && (p[0] == 0 || p[0] == 255)), "{mode:?}：單色 HD 只有黑白");
                assert!(px.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2]), "{mode:?}：HUD 要是灰階");
            }
        }
        assert!(px.chunks_exact(4).any(|p| p[0] != 0 || p[1] != 0 || p[2] != 0), "{mode:?}：全黑");
    }
    s.set_view(v(DisplayMode::Cga));
    assert!(s.hd_effective(), "單色圖庫與 HD 資料夾都有，單色模式用 HD 圖");
    s.set_view(ViewOpts { hd: false, ..v(DisplayMode::Cga) });
    assert!(!s.hd_effective());
    assert_eq!({ let (_, w, h) = s.render(1.0); (w, h) }, size_of(DisplayMode::Cga), "關掉 HD 回到 640×200");
    s.set_view(v(DisplayMode::Ega));
    assert!(s.hd_effective());
}
