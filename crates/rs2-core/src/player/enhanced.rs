//! 強化選項在玩家領域的實作（規格 005「射擊」「操控」）：波動砲充能、按住連射、砲座像 Force、細步移動。
//!
//! 全部屬於 remake-owned，沒有原版收據可以對拍，行為的依據是規格 005 與這個檔案的單元測試。
//! 強度（門檻、傷害、速度、射程）都是初值，寫成有名字的常數，規格「未決定」列的項目要由使用者試玩後調。
//!
//! 設計約束：
//!
//! - 狀態在 [`EnhancedPlayer`]，住在 `Core::enh`，不進 `Mirror`、不寫資料段，也不動 `DS_EXEMPT`。
//!   結構裡對應原版變數的欄位（位置、血量、砲座）沿用原版的欄位，只是換了算法。
//! - 每個入口都先看 [`Core::player_rules`]。選項全關（[`Rules::ORIGINAL`]）時這個檔案不寫任何東西，
//!   原版路徑的資料段寫入與繪製命令和沒有這個檔案時逐位元組相同。
//! - 展示錄影（標題的示範輪播）一律當作選項全關：錄影是照原版時序錄的，換了規則就不是同一場戰鬥。
//! - 不用時間、不用亂數，所以同樣的輸入與選項得到同樣的結果。
//!
//! | 選項 | 入口 | 行為 |
//! |---|---|---|
//! | 波動砲充能 | [`Core::fire_enhanced`] | 按住累積充能（邏輯步），放開依 [`WAVE_LEVEL_1`] 起的三級門檻射出 [`Core::fire_wave`] |
//! | 按住連射 | [`Core::fire_enhanced`] | 按住時每 [`AUTO_FIRE_INTERVAL`] 步射一發普通彈，砲座同步開火 |
//! | 砲座像 Force | [`Core::split_pods`]、[`Core::fly_free_pods`] | 分離鍵發射、收回砲座，砲座獨立飛行並碰撞 |
//! | 細步移動 | [`Core::steer_fine`] | 新按的方向第一步走半步，之後每步走整步 |

use super::draw::{POD_DX, POD_LEFT_SHIFT};
use super::{
    BOTTOM_ROW, BodyImages, COL_STEP, FACING_LEFT, FACING_RIGHT, InputBit, LEFT_COL, NO, PLAIN_WEAPON, PLAYER_COUNT, Player, RIGHT_COL, ROW_PIXELS,
    STYLES, YES,
};
use crate::game::{Core, Rules};
use crate::mirror::FireFlag;
use crate::pbullet::Facing;

// ---- 波動砲（初值，規格 005「未決定」）----

/// 第一、二、三級的充能門檻，單位是邏輯步（一秒約 9.1 步）。三個門檻等距，前端的充能條三等分刻度才對得上：
/// 4 步約 0.44 秒、8 步約 0.88 秒、12 步約 1.32 秒。充能從新按之後的第一個按住步起算。
pub const WAVE_LEVEL_1: u16 = 4;
pub const WAVE_LEVEL_2: u16 = 8;
pub const WAVE_LEVEL_3: u16 = 12;
/// 充能條滿格，也是計數的上限。
pub const WAVE_CHARGE_FULL: u16 = WAVE_LEVEL_3;
/// 子彈傷害見 `pbullet.rs` 的 `WAVE_DAMAGE`。

/// 充能對應的波動砲級數，0 表示不足第一級（放開不射）。
pub(crate) const fn wave_level(charge: u16) -> u8 {
    if charge >= WAVE_LEVEL_3 {
        3
    } else if charge >= WAVE_LEVEL_2 {
        2
    } else if charge >= WAVE_LEVEL_1 {
        1
    } else {
        0
    }
}

/// 砲座圖框動畫看的充能值（`POD_FRAMES` 以原版的計數為準，第二段 6、第三段 14）。
/// 波動砲的計數是 0 至 12，乘 7/6 對到 0 至 14：計數 6 起換第二段，滿格（12）換第三段。
const WAVE_TO_POD_NUM: u16 = 7;
const WAVE_TO_POD_DEN: u16 = 6;

// ---- 按住連射 ----

/// 按住開火鍵時，兩發普通彈之間隔的邏輯步數（規格 005：每 2 個邏輯步一發）。
pub(crate) const AUTO_FIRE_INTERVAL: u8 = 2;

// ---- 細步移動 ----

/// 新按的方向第一步走的距離：水平 1 欄（8 像素）、垂直 5 像素。之後每步走原版的整步（2 欄、10 像素）。
const HALF_COLS: u16 = COL_STEP / 2;
const HALF_PIXELS: u16 = ROW_PIXELS / 2;
/// 機身繪製位置 y 的最大值（列 16 起點）。
pub(crate) const BOTTOM_PIXELS: i16 = BOTTOM_ROW as i16 * ROW_PIXELS as i16;

// ---- 砲座像 Force（初值）----

/// 砲座分離後碰到實體時，每幀造成的傷害。原版附著時是 1（`POD_DAMAGE`）。
pub(super) const DETACHED_POD_DAMAGE: u8 = 2;
/// 發射後每步往右飛的欄數（和普通彈同速）。
const POD_LAUNCH_COLS: i16 = 4;
/// 發射後最多飛離起點幾欄，之後停在原地繼續開火。
const POD_RANGE_COLS: i16 = 16;
/// 砲座最右的欄。附著時砲座最右在欄 36（機身欄 33 加 3），飛行不超過它，才在碰撞格網（40 欄）之內。
const POD_MAX_COL: i16 = 36;
/// 收回時每步往機身靠近的欄數與像素列數。
const POD_RECALL_COLS: i16 = 4;
const POD_RECALL_PIXELS: i16 = 20;
/// 發射、收回之後幾步內不因接觸而接回，否則起飛當下就和機身重疊的砲座（玩家 2）會立刻接回。
const POD_REARM_STEPS: u8 = 3;
/// 接觸判定用的矩形：機身 5 欄 × 2 列（本體形狀 5 × 1 加上下一列的子彈判定列），砲座 2 × 2（`PlayerShape::Pod`）。
const BODY_COLS: i16 = 5;
const BODY_ROWS: i16 = 2;
const POD_COLS: i16 = 2;
const POD_ROWS: i16 = 2;

/// 砲座的狀態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PodMode {
    /// 附著在機身，位置和圖框由原版的 `advance_pods` 算。
    #[default]
    Docked,
    /// 發射中：往右飛到射程或右緣，停住。
    Launched,
    /// 收回中：往機身飛，接觸或到位就接回。
    Recalling,
}

/// 一位玩家的強化狀態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EnhancedPlayer {
    /// 波動砲充能，0 至 [`WAVE_CHARGE_FULL`]。選項關閉時恆為 0。
    pub charge: u16,
    /// 上一步開火鍵按著（強化開火路徑用）。
    pub holding: bool,
    /// 距離下一發連射還有幾步。
    pub auto_timer: u8,
    /// 上一步分離鍵按著，偵測按下的邊緣。
    pub split_prev: bool,
    pub pod: PodMode,
    /// 發射時砲座的欄，射程從這裡起算。
    pub launch_col: u16,
    /// 發射或收回之後經過的步數，到 [`POD_REARM_STEPS`] 才會因接觸而接回。
    pub pod_age: u8,
    /// 細步移動作用過：機身的位置可能是半步，離開細步時要對齊整格。
    pub fine: bool,
    /// 上一步實際採用的方向，水平（左 -1、右 +1）與垂直（上 -1、下 +1）。新按的方向和它不同。
    pub move_x: i8,
    pub move_y: i8,
}

/// 前端要顯示的強化狀態，每次交回控制權時公布（`Game::player_status`、`Game::hud_status`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ShootView {
    pub charge: u16,
    pub pods_detached: bool,
    pub last_move: (i8, i8),
}

/// 兩位玩家的強化狀態，加上發射子彈時的共用參數。
#[derive(Clone, Debug, Default)]
pub(crate) struct Enhanced {
    pub ship: [EnhancedPlayer; PLAYER_COUNT],
    /// 這一刻發射的子彈，繪製位置 y 比列起點多幾像素。細步移動的半列位置用，其他時候是 0。
    pub spawn_dy: u16,
}

impl Enhanced {
    /// 每關開始：砲座收回、細步與充能歸零。分離鍵的偵測狀態保留，否則關卡開始時還按著的分離鍵會被當成新按。
    pub(crate) fn reset_for_stage(&mut self) {
        for e in &mut self.ship {
            *e = EnhancedPlayer { split_prev: e.split_prev, ..EnhancedPlayer::default() };
        }
        self.spawn_dy = 0;
    }

    pub(crate) fn view(&self) -> [ShootView; PLAYER_COUNT] {
        std::array::from_fn(|i| {
            let e = &self.ship[i];
            ShootView { charge: e.charge, pods_detached: e.pod != PodMode::Docked, last_move: (e.move_x, e.move_y) }
        })
    }
}

impl Player {
    /// 設機身的繪製位置 y，列取上緣所在的列（`y / 10` 向下取整）。碰撞用整數列，所以半步位置（y 是 5 的奇數倍）
    /// 的碰撞格和它上方整步位置相同。
    fn set_y(&mut self, y: i16) {
        self.blit_y = y as u16;
        self.row = (y / ROW_PIXELS as i16) as u16;
    }

    /// 細步上移 `pixels` 像素。到頂不動，圖也不換；不足一步時停在頂端。
    fn fine_up(&mut self, images: &BodyImages, pixels: u16) {
        if (self.blit_y as i16) > 0 {
            self.set_y((self.blit_y as i16 - pixels as i16).max(0));
            self.image = images.up;
        }
    }

    fn fine_down(&mut self, images: &BodyImages, pixels: u16) {
        if (self.blit_y as i16) < BOTTOM_PIXELS {
            self.set_y((self.blit_y as i16 + pixels as i16).min(BOTTOM_PIXELS));
            self.image = images.down;
        }
    }

    /// 細步右移 `cols` 欄，停在欄 33（`RIGHT_COL`）。尾焰旗標設 `Y`。
    fn fine_right(&mut self, cols: u16) {
        if self.col < RIGHT_COL {
            self.col = (self.col + cols).min(RIGHT_COL);
            self.blit_x = self.col;
            self.flame = YES;
        }
    }

    /// 細步左移 `cols` 欄，停在欄 1。
    fn fine_left(&mut self, cols: u16) {
        if (self.col as i16) > LEFT_COL {
            self.col = (self.col as i16 - cols as i16).max(LEFT_COL) as u16;
            self.blit_x = self.col;
        }
    }
}

impl Core {
    /// 玩家領域看的強化選項。展示錄影一律當作全關。
    pub(crate) fn player_rules(&self) -> Rules {
        if self.demo_playing() { Rules::ORIGINAL } else { self.port.borrow().rules }
    }

    /// 選項被關掉時收尾：細步位置對齊整格、砲座接回、充能清掉。選項沒開過就什麼都不做。
    pub(super) fn settle_options(&mut self, who: usize, rules: Rules) {
        let e = self.enh.ship[who];
        if e.fine && !rules.fine_step {
            let p = &mut self.pl.ship[who];
            // 整步移動的欄是奇數（起點 5，每步 2），半步位置的欄可能是偶數。
            p.col = (p.col | 1).min(RIGHT_COL);
            p.blit_x = p.col;
            p.blit_y = p.row * ROW_PIXELS;
            let e = &mut self.enh.ship[who];
            (e.fine, e.move_x, e.move_y) = (false, 0, 0);
        }
        if e.pod != PodMode::Docked && !rules.force_pod {
            self.dock_pods(who);
        }
        if !rules.wave_charge {
            self.enh.ship[who].charge = 0;
        }
        if !rules.wave_charge && !rules.auto_fire {
            self.enh.ship[who].holding = false;
        }
    }

    // ---- 細步移動 ----

    /// 方向鍵的細步版（原版 `steer` 的替代）。優先順序與原版相同：上先於下，左先於右。
    ///
    /// 邏輯一步只在步上取樣按鍵，輕點與按住在一步內分不出來，所以：**方向和上一步不同（新按或換向）的那一步走半步，
    /// 方向和上一步相同的步走整步**。按住 N 步走 N − 0.5 個整步，和原版幾乎同速；輕點只走半步，可以微調。
    /// 到邊界時停在邊界（欄 1、33，y 0、160），不足一步也走完剩下的。
    pub(super) fn steer_fine(&mut self, who: usize) {
        let style = &STYLES[who];
        let up = self.any_held(style.keys.upward());
        let down = self.any_held(style.keys.downward());
        let left = self.any_held(style.keys.leftward());
        let right = self.any_held(style.keys.rightward());
        let dy: i8 = if up { -1 } else if down { 1 } else { 0 };
        let dx: i8 = if left { -1 } else if right { 1 } else { 0 };
        let e = &mut self.enh.ship[who];
        let (prev_x, prev_y) = (e.move_x, e.move_y);
        (e.move_x, e.move_y, e.fine) = (dx, dy, true);
        let p = &mut self.pl.ship[who];
        match dy {
            -1 => {
                p.fine_up(&style.images, if prev_y == dy { ROW_PIXELS } else { HALF_PIXELS });
                p.input_bits |= InputBit::Up as u8;
            }
            1 => {
                p.fine_down(&style.images, if prev_y == dy { ROW_PIXELS } else { HALF_PIXELS });
                p.input_bits |= InputBit::Down as u8;
            }
            _ => {}
        }
        match dx {
            -1 => {
                p.fine_left(if prev_x == dx { COL_STEP } else { HALF_COLS });
                p.input_bits |= InputBit::Left as u8;
            }
            1 => {
                p.fine_right(if prev_x == dx { COL_STEP } else { HALF_COLS });
                p.input_bits |= InputBit::Right as u8;
            }
            _ => {}
        }
    }

    /// 本體的髒格是否要多標一列：細步的半步位置讓機身橫跨三列。
    pub(super) fn body_dirty_rows(&self, who: usize) -> u16 {
        const ROWS: u16 = 2;
        if self.enh.ship[who].fine && self.pl.ship[who].blit_y % ROW_PIXELS != 0 { ROWS + 1 } else { ROWS }
    }

    // ---- 開火 ----

    /// 強化的開火路徑（波動砲充能或按住連射開著時，取代原版的 `release_charge` 與 `fire_main`）。
    ///
    /// 開火鍵的狀態直接讀按鍵表（`key_held`），以邏輯步計數，不看鍵盤事件，所以手把與鍵盤一樣。
    /// 「新按」的判定與原版相同（開火事件與新按旗標同時成立），兩個旗標照原版清除，所以輕到兩步之間按放的短按仍射一發。
    ///
    /// | 狀態 | 動作 |
    /// |---|---|
    /// | 新按 | 射一發普通彈，充能歸零，連射計時重設 |
    /// | 按住 | 波動砲開著：充能加 1。連射開著：計時減 1，到 0 射一發普通彈 |
    /// | 放開 | 波動砲開著：依充能級數射出波動砲。充能歸零 |
    ///
    /// 只開連射時，原版的大掃射仍在：放開時充能不小於 14 就從玩家的欄射出，充能在有第二座砲座時按住每步加 1
    /// （原版是每個鍵盤事件加 1）。
    pub(super) fn fire_enhanced(&mut self, who: usize, rules: Rules) {
        if !rules.wave_charge {
            self.release_charge(who);
        }
        let held = self.key_held(STYLES[who].keys.fire);
        let event = self.fire_flag(who, FireFlag::Event);
        let pressed = event && self.fire_flag(who, FireFlag::NewPress);
        if event {
            self.set_fire_flag(who, FireFlag::Event, false);
        }
        if pressed {
            self.set_fire_flag(who, FireFlag::NewPress, false);
        }
        let mut shot = false;
        if pressed {
            // 放開又在同一步內重按：先結算上一輪的充能。
            if rules.wave_charge && self.enh.ship[who].holding {
                self.release_wave(who);
            }
            shot = true;
            let e = &mut self.enh.ship[who];
            (e.charge, e.auto_timer, e.holding) = (0, AUTO_FIRE_INTERVAL, held);
            self.pl.ship[who].charge = 0;
        } else if held {
            let e = &mut self.enh.ship[who];
            e.holding = true;
            if rules.wave_charge {
                e.charge = (e.charge + 1).min(WAVE_CHARGE_FULL);
            } else if self.pl.ship[who].pods.second != NO {
                let p = &mut self.pl.ship[who];
                p.charge = p.charge.wrapping_add(1);
            }
            if rules.auto_fire {
                let e = &mut self.enh.ship[who];
                e.auto_timer = e.auto_timer.saturating_sub(1);
                if e.auto_timer == 0 {
                    e.auto_timer = AUTO_FIRE_INTERVAL;
                    shot = true;
                }
            }
        } else {
            if rules.wave_charge && self.enh.ship[who].holding {
                self.release_wave(who);
            }
            let e = &mut self.enh.ship[who];
            (e.holding, e.charge) = (false, 0);
        }
        let p = &mut self.pl.ship[who];
        if !shot {
            p.fired = NO;
            return;
        }
        p.input_bits |= InputBit::Fire as u8;
        p.fired = YES;
        let (col, row) = (p.col, p.row);
        let dy = self.main_gun_sub_row(who);
        self.enh.spawn_dy = dy;
        self.fire_player_bullets(PLAIN_WEAPON, Facing::Right, col, row);
        self.enh.spawn_dy = 0;
    }

    /// 放開開火鍵：依充能級數射出波動砲，充能歸零。不足第一級不射。
    fn release_wave(&mut self, who: usize) {
        let level = wave_level(self.enh.ship[who].charge);
        self.enh.ship[who].charge = 0;
        if level == 0 {
            return;
        }
        let p = &self.pl.ship[who];
        let (col, row) = (p.col, p.row);
        self.enh.spawn_dy = self.main_gun_sub_row(who);
        self.fire_wave(level, col, row);
        self.enh.spawn_dy = 0;
    }

    /// 主機發射的子彈，繪製 y 比列起點多幾像素：細步的半步位置是 5，其他是 0。
    pub(super) fn main_gun_sub_row(&self, who: usize) -> u16 {
        if self.enh.ship[who].fine { self.pl.ship[who].blit_y % ROW_PIXELS } else { 0 }
    }

    /// 砲座發射的子彈，繪製 y 比列起點多幾像素。砲座的繪製位置 y 可能是負值（機身在上緣附近），所以用有號的餘數。
    pub(super) fn pod_sub_row(&self, who: usize) -> u16 {
        if self.enh.ship[who].fine { (self.pl.ship[who].pods.blit_y as i16).rem_euclid(ROW_PIXELS as i16) as u16 } else { 0 }
    }

    /// 砲座圖框動畫看的充能值：波動砲充能中換算成原版的計數，否則是原版的充能計數。
    pub(super) fn pod_display_charge(&self, who: usize) -> u16 {
        let charge = self.enh.ship[who].charge;
        if charge == 0 { self.pl.ship[who].charge } else { charge * WAVE_TO_POD_NUM / WAVE_TO_POD_DEN }
    }

    // ---- 砲座像 Force ----

    /// 分離鍵（`Game::set_split_held`）：按下的邊緣在附著時發射砲座，在飛行時收回。沒有裝上砲座時沒有作用。
    pub(super) fn split_pods(&mut self, who: usize, rules: Rules) {
        let held = self.port.borrow().split_held[who];
        let e = &mut self.enh.ship[who];
        let pressed = held && !e.split_prev;
        e.split_prev = held;
        if !rules.force_pod || !pressed {
            return;
        }
        match e.pod {
            PodMode::Docked => {
                if self.pl.ship[who].pods.first_flying() {
                    self.launch_pods(who);
                }
            }
            PodMode::Launched => {
                e.pod = PodMode::Recalling;
                e.pod_age = 0;
            }
            PodMode::Recalling => {}
        }
    }

    /// 發射：砲座從目前位置脫離機身，朝右飛。朝向設成右，才朝前開火（轉向鍵之後仍可切換）。
    /// 位置夾在機身可以到的範圍裡（列 0 至 `16 - row_offset`），砲座在機身上緣附近時原版的列是負值，不帶進飛行。
    fn launch_pods(&mut self, who: usize) {
        let style = &STYLES[who].pod;
        let max_row = BOTTOM_ROW as i16 - style.row_offset as i16;
        let p = &mut self.pl.ship[who];
        let col = (p.pods.col as i16).clamp(0, POD_MAX_COL);
        let y = (p.pods.blit_y as i16).clamp(0, max_row * ROW_PIXELS as i16 + HALF_PIXELS as i16);
        p.pods.col = col as u16;
        p.pods.blit_x = col as u16;
        p.pods.blit_y = y as u16;
        p.pods.row = (y / ROW_PIXELS as i16) as u16;
        p.pods.facing = FACING_RIGHT;
        let e = &mut self.enh.ship[who];
        (e.pod, e.pod_age, e.launch_col) = (PodMode::Launched, 0, col as u16);
    }

    /// 接回機身。接回的方向決定朝向：砲座的中心在機身中心前面（右）是朝右，否則朝左，
    /// 位置由原版的 `advance_pods` 接手（朝右在機身右後方欄 +3，朝左再 -5）。
    fn dock_pods(&mut self, who: usize) {
        let p = &mut self.pl.ship[who];
        let front = p.pods.col as i16 > p.col as i16 + POD_FRONT_BIAS;
        p.pods.facing = if front { FACING_RIGHT } else { FACING_LEFT };
        let e = &mut self.enh.ship[who];
        (e.pod, e.pod_age) = (PodMode::Docked, 0);
    }

    /// 機身和砲座（任一座）是否重疊。
    fn ship_touches_pods(&self, who: usize) -> bool {
        let ship = &self.pl.ship[who];
        let pods = &ship.pods;
        let (sc, sr) = (ship.col as i16, ship.row as i16);
        let (pc, pr) = (pods.col as i16, pods.row as i16);
        let rows_hit = |r: i16| r + POD_ROWS > sr && r < sr + BODY_ROWS;
        let gap = STYLES[who].pod.gap_rows as i16;
        pc + POD_COLS > sc && pc < sc + BODY_COLS && (rows_hit(pr) || (pods.second_flying() && rows_hit(pr + gap)))
    }

    /// 分離中的砲座每幀的位置更新（取代原版的 `advance_pods` 位置算法）。畫與碰撞用的是更新前的位置，下一幀才換，
    /// 和附著時一樣。更新完仍是分離狀態就不再回到附著的位置算法；接回了就由原版的算法接手。
    ///
    /// - 發射中：每步往右 [`POD_LAUNCH_COLS`] 欄，飛離起點 [`POD_RANGE_COLS`] 欄或到 [`POD_MAX_COL`] 停住。
    /// - 收回中：每步往機身的接回位置靠近（[`POD_RECALL_COLS`] 欄、[`POD_RECALL_PIXELS`] 像素），到位就接回。
    /// - 機身碰到砲座就接回，發射或收回之後要先過 [`POD_REARM_STEPS`] 步。從前方碰到是朝右，從後方是朝左。
    pub(super) fn fly_free_pods(&mut self, who: usize) {
        let style = &STYLES[who].pod;
        let e = self.enh.ship[who];
        if e.pod_age >= POD_REARM_STEPS && self.ship_touches_pods(who) {
            self.dock_pods(who);
            return;
        }
        let max_y = (BOTTOM_ROW as i16 - style.row_offset as i16) * ROW_PIXELS as i16 + HALF_PIXELS as i16;
        let p = &mut self.pl.ship[who];
        let (sc, pc, py) = (p.col as i16, p.pods.col as i16, p.pods.blit_y as i16);
        let (col, y, arrived) = match e.pod {
            PodMode::Launched => {
                let limit = (e.launch_col as i16 + POD_RANGE_COLS).min(POD_MAX_COL);
                ((pc + POD_LAUNCH_COLS).min(limit).max(pc), py, false)
            }
            PodMode::Recalling => {
                // 目標是朝著砲座目前所在那一側的接回位置，和 `dock_pods` 判斷側邊的規則一致。
                let front = pc > sc + POD_FRONT_BIAS;
                let target_col = if front { sc + POD_DX as i16 } else { sc + POD_DX as i16 - POD_LEFT_SHIFT as i16 };
                let target_y = (p.blit_y as i16 - style.y_offset as i16).clamp(0, max_y);
                let col = pc + (target_col - pc).clamp(-POD_RECALL_COLS, POD_RECALL_COLS);
                let y = py + (target_y - py).clamp(-POD_RECALL_PIXELS, POD_RECALL_PIXELS);
                (col, y, col == target_col && y == target_y)
            }
            PodMode::Docked => return,
        };
        p.pods.col = col as u16;
        p.pods.blit_x = col as u16;
        p.pods.blit_y = y as u16;
        p.pods.row = (y / ROW_PIXELS as i16) as u16;
        let e = &mut self.enh.ship[who];
        e.pod_age = e.pod_age.saturating_add(1);
        if arrived {
            self.dock_pods(who);
        }
    }

    /// 砲座已分離（發射中或收回中）。
    pub(super) fn pods_detached(&self, who: usize) -> bool {
        self.enh.ship[who].pod != PodMode::Docked
    }
}

/// 砲座中心在機身中心前面的判斷：砲座寬 2 欄、機身寬 5 欄，中心差 = 砲座欄 + 1 - (機身欄 + 2)，
/// 大於 0 即砲座欄 > 機身欄 + 1。附著在右後方的砲座（欄 + 3）在前面，附著在左側的（欄 - 2）在後面。
const POD_FRONT_BIAS: i16 = 1;

#[cfg(test)]
mod tests {
    use super::super::test_support::{core, place, set_shapes};
    use super::*;
    use crate::game::Rules;

    /// 波動砲子彈記錄的 `owner`。
    const WAVE_OWNER: u16 = crate::pbullet::WAVE_LETTER;

    fn with_rules(c: &mut Core, rules: Rules) {
        c.port.borrow_mut().rules = rules;
    }

    fn fresh(rules: Rules) -> Core {
        let mut c = core();
        set_shapes(&mut c);
        with_rules(&mut c, rules);
        // 零映像沒有樣板表：補上大掃射樣板的行為字組 `S`（真實映像在 `dseg:2B35h`），波動砲第三級沿用它。
        c.ds.w16(crate::pbullet::Shot::Sweep.table_offset(Facing::Right), u16::from(b'S'));
        c
    }

    fn pressed(c: &mut Core, who: usize) {
        c.set_key(STYLES[who].keys.fire, true);
        c.set_fire_flag(who, FireFlag::NewPress, true);
        c.set_fire_flag(who, FireFlag::Event, true);
    }

    fn bullets(c: &Core) -> usize {
        c.pb.slots.iter().filter(|s| s.is_active()).count()
    }

    const WAVE: Rules = Rules { wave_charge: true, ..Rules::ORIGINAL };
    const AUTO: Rules = Rules { auto_fire: true, ..Rules::ORIGINAL };
    const FORCE: Rules = Rules { force_pod: true, ..Rules::ORIGINAL };
    const FINE: Rules = Rules { fine_step: true, ..Rules::ORIGINAL };

    #[test]
    fn the_wave_thresholds_are_evenly_spaced_for_the_three_part_charge_bar() {
        assert_eq!((WAVE_LEVEL_1, WAVE_LEVEL_2, WAVE_LEVEL_3), (WAVE_CHARGE_FULL / 3, WAVE_CHARGE_FULL * 2 / 3, WAVE_CHARGE_FULL));
        assert_eq!(WAVE_CHARGE_FULL % 3, 0);
        let levels: Vec<u8> = (0..=WAVE_CHARGE_FULL + 3).map(wave_level).collect();
        assert_eq!(levels[..4], [0, 0, 0, 0]);
        assert_eq!(levels[WAVE_LEVEL_1 as usize], 1);
        assert_eq!(levels[WAVE_LEVEL_2 as usize - 1], 1);
        assert_eq!(levels[WAVE_LEVEL_2 as usize], 2);
        assert_eq!(levels[WAVE_LEVEL_3 as usize - 1], 2);
        assert_eq!(levels[WAVE_LEVEL_3 as usize], 3);
        // 砲座圖框看的值：第二段（6）與第三段（14）的起點落在波動砲計數 6 與 12。
        let shown = |c: u16| c * WAVE_TO_POD_NUM / WAVE_TO_POD_DEN;
        assert!(shown(5) < 6 && shown(6) >= 6 && shown(11) < 14 && shown(12) >= 14);
    }

    // ---- 波動砲 ----

    /// 按住 `held_steps` 個按住步再放開，回傳放開那一步之後池裡的子彈。
    fn hold_and_release(rules: Rules, held_steps: usize) -> Core {
        let mut c = fresh(rules);
        place(&mut c, 0, 7, 8);
        pressed(&mut c, 0);
        c.update_player(0);
        for _ in 0..held_steps {
            c.update_player(0);
        }
        c.set_key(STYLES[0].keys.fire, false);
        c.update_player(0);
        c
    }

    #[test]
    fn wave_charge_counts_logic_steps_and_releases_by_level() {
        // 新按那一步射一發普通彈；按住步累積；放開那一步依級數射。
        let tap = hold_and_release(WAVE, 0);
        assert_eq!(bullets(&tap), 1, "輕點只有新按的普通彈");
        let short = hold_and_release(WAVE, WAVE_LEVEL_1 as usize - 1);
        assert_eq!(bullets(&short), 1, "不足第一級不射");
        let l1 = hold_and_release(WAVE, WAVE_LEVEL_1 as usize);
        assert_eq!(bullets(&l1), 2, "普通彈加一發波動砲");
        let big = l1.pb.slots.iter().filter(|s| s.is_active()).map(|s| s.bullet).find(|b| b.owner == WAVE_OWNER).expect("有波動砲");
        assert_eq!(big.behavior(), crate::pbullet::Behavior::WaveBolt, "第一級：消失的大彈");
        assert!(!big.behavior().pierces());
        assert_eq!((big.col, big.row), (7, 8));
        let l2 = hold_and_release(WAVE, WAVE_LEVEL_2 as usize);
        let lance = l2.pb.slots.iter().filter(|s| s.is_active()).map(|s| s.bullet).find(|b| b.owner == WAVE_OWNER).unwrap();
        assert_eq!(lance.behavior(), crate::pbullet::Behavior::WaveLance, "第二級：穿透");
        assert!(lance.behavior().pierces() && lance.behavior().damage() > big.behavior().damage());
        let l3 = hold_and_release(WAVE, WAVE_LEVEL_3 as usize + 5);
        let sweep: Vec<_> = l3.pb.slots.iter().filter(|s| s.is_active()).map(|s| s.bullet).filter(|b| b.owner == WAVE_OWNER).collect();
        assert_eq!(sweep.len(), 7, "第三級：七顆穿透彈");
        assert_eq!(sweep.iter().map(|b| b.row).collect::<Vec<_>>(), vec![0, 3, 6, 9, 12, 15, 17], "合起來涵蓋列 0 至 18");
        assert!(sweep.iter().all(|b| b.behavior().pierces() && b.behavior().damage() == 3 && b.col == 7));
        // 放開後充能歸零。
        assert_eq!(l3.enh.ship[0].charge, 0);
    }

    #[test]
    fn wave_charge_does_not_need_pods_or_keyboard_events() {
        // 沒有砲座、沒有任何鍵盤事件（Event 旗標只在新按那一步有）：原版不會累計充能，波動砲照樣累計。
        let c = hold_and_release(WAVE, WAVE_LEVEL_3 as usize);
        assert_eq!(c.pl.ship[0].pods.second, NO);
        assert!(bullets(&c) >= 8, "普通彈加七顆");
        // 同樣的按法，選項關閉時：原版不射波動砲，也沒有大掃射（沒有第二座砲座）。
        let off = hold_and_release(Rules::ORIGINAL, WAVE_LEVEL_3 as usize);
        assert_eq!(bullets(&off), 1);
    }

    #[test]
    fn the_wave_charge_caps_at_full_and_resets_when_the_option_is_switched_off() {
        let mut c = fresh(WAVE);
        place(&mut c, 0, 7, 8);
        pressed(&mut c, 0);
        c.update_player(0);
        for _ in 0..40 {
            c.update_player(0);
        }
        assert_eq!(c.enh.ship[0].charge, WAVE_CHARGE_FULL);
        assert_eq!(c.enh.view()[0].charge, WAVE_CHARGE_FULL);
        with_rules(&mut c, Rules::ORIGINAL);
        c.update_player(0);
        assert_eq!(c.enh.ship[0].charge, 0, "選項關掉，充能清掉");
        assert_eq!(c.pl.ship[0].charge, 0, "原版的充能計數沒被動過");
    }

    #[test]
    fn wave_charge_makes_the_pods_animate_like_the_original_charge() {
        let mut c = fresh(WAVE);
        place(&mut c, 0, 7, 8);
        c.enh.ship[0].charge = WAVE_CHARGE_FULL;
        assert_eq!(c.pod_display_charge(0), 14);
        c.enh.ship[0].charge = 0;
        c.pl.ship[0].charge = 9;
        assert_eq!(c.pod_display_charge(0), 9, "沒有波動砲充能時看原版的計數");
    }

    // ---- 按住連射 ----

    #[test]
    fn auto_fire_shoots_every_second_step_while_held_and_stops_on_release() {
        let mut c = fresh(AUTO);
        place(&mut c, 0, 7, 8);
        pressed(&mut c, 0);
        let mut shots = Vec::new();
        for step in 0..9 {
            let before = bullets(&c);
            c.update_player(0);
            // 子彈不移動（沒呼叫 update_player_bullets），池數的增加就是這一步的發射。
            shots.push(bullets(&c) - before);
            assert_eq!(c.pl.ship[0].fired == YES, shots[step] == 1, "fired 旗標跟著發射（砲座和閃光看它）");
        }
        assert_eq!(shots, vec![1, 0, 1, 0, 1, 0, 1, 0, 1], "每 2 步一發，第一發在新按那一步");
        c.set_key(STYLES[0].keys.fire, false);
        let before = bullets(&c);
        c.update_player(0);
        assert_eq!(bullets(&c), before);
        assert_eq!(c.pl.ship[0].fired, NO);
        // 關閉選項：按住就不連射（原版：按住只有第二座砲座才計數，不射）。
        let mut c = fresh(Rules::ORIGINAL);
        place(&mut c, 0, 7, 8);
        pressed(&mut c, 0);
        c.update_player(0);
        for _ in 0..8 {
            c.update_player(0);
        }
        assert_eq!(bullets(&c), 1, "原版：一次按放一發");
    }

    #[test]
    fn auto_fire_with_wave_charge_keeps_firing_and_settles_the_charge_on_release() {
        let both = Rules { wave_charge: true, auto_fire: true, ..Rules::ORIGINAL };
        let mut c = fresh(both);
        place(&mut c, 0, 7, 8);
        pressed(&mut c, 0);
        c.update_player(0);
        for _ in 0..WAVE_LEVEL_2 {
            c.update_player(0);
        }
        let plain = bullets(&c);
        assert!(plain >= 5, "連射照常：第 0、2、4、6、8 步各一發，{plain}");
        assert!(c.pb.slots.iter().filter(|s| s.is_active()).all(|s| s.bullet.owner != WAVE_OWNER), "按住時不射波動砲");
        c.set_key(STYLES[0].keys.fire, false);
        c.update_player(0);
        let waves = c.pb.slots.iter().filter(|s| s.is_active() && s.bullet.owner == WAVE_OWNER).count();
        assert_eq!(waves, 1, "放開才結算：第二級一發");
    }

    #[test]
    fn auto_fire_alone_keeps_the_original_sweep_with_two_pods_counted_per_step() {
        let mut c = fresh(AUTO);
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].pods.second = YES;
        pressed(&mut c, 0);
        c.update_player(0);
        for _ in 0..crate::player::SWEEP_CHARGE {
            c.update_player(0);
        }
        assert_eq!(c.pl.ship[0].charge, crate::player::SWEEP_CHARGE, "原版的充能計數改成每步加 1");
        c.set_key(STYLES[0].keys.fire, false);
        c.update_player(0);
        let sweep = c.pb.slots.iter().filter(|s| s.is_active() && s.bullet.owner == u16::from(b'F')).count();
        assert_eq!(sweep, 6, "放開時不小於 14 就射原版的大掃射");
    }

    #[test]
    fn a_tap_shorter_than_one_step_still_fires_once() {
        // 按下又放開都在兩步之間：開火鍵已放開，但開火事件與新按旗標都在（原版靠旗標保住這一發）。
        for rules in [WAVE, AUTO] {
            let mut c = fresh(rules);
            place(&mut c, 0, 7, 8);
            c.set_fire_flag(0, FireFlag::NewPress, true);
            c.set_fire_flag(0, FireFlag::Event, true);
            c.set_key(STYLES[0].keys.fire, false);
            c.update_player(0);
            assert_eq!(bullets(&c), 1, "{rules:?}");
            assert!(!c.fire_flag(0, FireFlag::Event) && !c.fire_flag(0, FireFlag::NewPress));
            c.update_player(0);
            assert_eq!(bullets(&c), 1, "之後沒有重複發射");
        }
    }

    #[test]
    fn the_player_two_gun_uses_its_own_keys_and_state() {
        let mut c = fresh(WAVE);
        place(&mut c, 0, 7, 8);
        place(&mut c, 1, 12, 5);
        pressed(&mut c, 1);
        c.update_player(1);
        for _ in 0..WAVE_LEVEL_1 {
            c.update_player(1);
        }
        assert_eq!((c.enh.ship[0].charge, c.enh.ship[1].charge), (0, WAVE_LEVEL_1));
        c.set_key(STYLES[1].keys.fire, false);
        c.update_player(1);
        let wave = c.pb.slots.iter().find(|s| s.is_active() && s.bullet.owner == WAVE_OWNER).expect("玩家 2 的波動砲");
        assert_eq!((wave.bullet.col, wave.bullet.row), (12, 5));
    }

    // ---- 細步移動 ----

    fn hold_dirs(c: &mut Core, who: usize, scans: &[u8]) {
        let k = STYLES[who].keys;
        for scan in [k.up, k.down, k.left, k.right, k.up_left, k.up_right, k.down_left, k.down_right] {
            c.set_key(scan, scans.contains(&scan));
        }
    }

    fn pos(c: &Core, who: usize) -> (u16, u16, u16, u16) {
        let p = &c.pl.ship[who];
        (p.col, p.row, p.blit_x, p.blit_y)
    }

    #[test]
    fn fine_step_moves_half_a_step_when_the_direction_is_new_and_a_whole_step_while_held() {
        let mut c = fresh(FINE);
        place(&mut c, 0, 7, 8);
        let k = STYLES[0].keys;
        hold_dirs(&mut c, 0, &[k.right, k.down]);
        c.update_player(0);
        assert_eq!(pos(&c, 0), (8, 8, 8, 85), "新按：右 1 欄（8 像素），下 5 像素；碰撞列取上緣所在的列（85 / 10 = 8）");
        c.update_player(0);
        assert_eq!(pos(&c, 0), (10, 9, 10, 95), "按住：右 2 欄，下 10 像素");
        c.update_player(0);
        assert_eq!(pos(&c, 0), (12, 10, 12, 105));
        // 輕點：一步之後放開，只走了半步。
        hold_dirs(&mut c, 0, &[]);
        c.update_player(0);
        assert_eq!(pos(&c, 0), (12, 10, 12, 105), "沒按鍵不動");
        hold_dirs(&mut c, 0, &[k.left]);
        c.update_player(0);
        assert_eq!(pos(&c, 0), (11, 10, 11, 105), "輕點一步只走半步，可以微調");
        hold_dirs(&mut c, 0, &[]);
        c.update_player(0);
        // 換向算新按。
        hold_dirs(&mut c, 0, &[k.right]);
        c.update_player(0);
        hold_dirs(&mut c, 0, &[k.left]);
        c.update_player(0);
        assert_eq!(pos(&c, 0).0, 12 - 1, "右轉左：換向那一步走半步");
        // 按住 N 步的總距離是 N - 0.5 個整步。
        let mut c = fresh(FINE);
        place(&mut c, 0, 3, 8);
        hold_dirs(&mut c, 0, &[k.right]);
        for _ in 0..6 {
            c.update_player(0);
        }
        assert_eq!(c.pl.ship[0].col, 3 + 1 + 5 * 2);
    }

    #[test]
    fn fine_step_holding_matches_the_original_speed_after_the_first_step() {
        let k = STYLES[0].keys;
        let mut fine = fresh(FINE);
        let mut orig = fresh(Rules::ORIGINAL);
        for c in [&mut fine, &mut orig] {
            place(c, 0, 3, 3);
            hold_dirs(c, 0, &[k.right, k.down]);
        }
        let mut gap = Vec::new();
        for _ in 0..6 {
            fine.update_player(0);
            orig.update_player(0);
            gap.push((orig.pl.ship[0].col as i16 - fine.pl.ship[0].col as i16, orig.pl.ship[0].blit_y as i16 - fine.pl.ship[0].blit_y as i16));
        }
        assert!(gap.iter().all(|&g| g == (1, 5)), "只差第一步少走的半步：{gap:?}");
    }

    #[test]
    fn fine_step_stops_at_the_same_borders_and_reaches_them_exactly() {
        let k = STYLES[0].keys;
        let mut c = fresh(FINE);
        place(&mut c, 0, 31, 15);
        hold_dirs(&mut c, 0, &[k.right, k.down]);
        for _ in 0..8 {
            c.update_player(0);
        }
        assert_eq!(pos(&c, 0), (RIGHT_COL, 16, RIGHT_COL, 160), "右緣欄 33、下緣列 16");
        hold_dirs(&mut c, 0, &[k.left, k.up]);
        for _ in 0..40 {
            c.update_player(0);
        }
        assert_eq!(pos(&c, 0), (1, 0, 1, 0), "左緣欄 1、上緣列 0");
        // 偶數欄也到得了邊界：欄 2 往左半步到欄 1、欄 32 往右整步停在欄 33。
        let mut c = fresh(FINE);
        place(&mut c, 0, 32, 8);
        hold_dirs(&mut c, 0, &[k.right]);
        c.update_player(0);
        c.update_player(0);
        assert_eq!(c.pl.ship[0].col, RIGHT_COL);
    }

    #[test]
    fn fine_step_keeps_the_pod_offsets_and_bullet_alignment_with_the_half_row_position() {
        let mut c = fresh(FINE);
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].pods.first = YES;
        c.pl.ship[0].pods.facing = FACING_RIGHT;
        let k = STYLES[0].keys;
        hold_dirs(&mut c, 0, &[k.down, k.right]);
        c.update_player(0);
        c.draw_player(0);
        hold_dirs(&mut c, 0, &[]);
        let p = c.pl.ship[0];
        // 砲座跟著機身：繪製 y 保留 5 像素，列取整數。
        assert_eq!((p.pods.col, p.pods.row, p.pods.blit_x, p.pods.blit_y), (8 + 3, 8 - 3, 11, 85 - 30));
        // 機身髒格多標一列（半步位置橫跨三列）。
        assert_eq!(c.body_dirty_rows(0), 3);
        // 子彈：碰撞列是整數，繪製 y 跟著機身多 5 像素。
        c.pl.ship[0].fired = NO;
        pressed(&mut c, 0);
        c.update_player(0);
        let b = c.pb.slots.iter().find(|s| s.is_active()).expect("射了一發").bullet;
        assert_eq!((b.row, b.blit_y), (p.row + 1, (p.row + 1) * 10 + 5));
        assert_eq!(c.enh.spawn_dy, 0, "發射完歸零");
    }

    #[test]
    fn leaving_fine_step_snaps_the_ship_back_to_the_whole_step_grid() {
        let mut c = fresh(FINE);
        place(&mut c, 0, 7, 8);
        let k = STYLES[0].keys;
        hold_dirs(&mut c, 0, &[k.right, k.down]);
        c.update_player(0);
        assert_eq!(pos(&c, 0), (8, 8, 8, 85));
        hold_dirs(&mut c, 0, &[]);
        with_rules(&mut c, Rules::ORIGINAL);
        c.update_player(0);
        assert_eq!(pos(&c, 0), (9, 8, 9, 80), "欄回到奇數，繪製 y 回到列起點");
        assert!(!c.enh.ship[0].fine);
        assert_eq!(c.body_dirty_rows(0), 2);
        // 之後走原版的整步。
        hold_dirs(&mut c, 0, &[k.right]);
        c.update_player(0);
        assert_eq!(c.pl.ship[0].col, 11);
    }

    #[test]
    fn fine_step_is_off_in_a_demo_and_when_the_option_is_off() {
        let k = STYLES[0].keys;
        let mut c = fresh(Rules::ORIGINAL);
        place(&mut c, 0, 7, 8);
        hold_dirs(&mut c, 0, &[k.right, k.down]);
        c.update_player(0);
        assert_eq!(pos(&c, 0), (9, 9, 9, 90), "關閉：原版整步");
        let mut c = fresh(FINE);
        c.set_demo_playing(true);
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].hp = 0xFF;
        // 展示錄影還原的輸入：右、下。
        c.record_demo_byte(c.stage_frame(), InputBit::Right as u8 | InputBit::Down as u8);
        c.update_player(0);
        assert_eq!(pos(&c, 0), (9, 9, 9, 90), "展示不套用強化規則");
    }

    // ---- 砲座像 Force ----

    fn armed(rules: Rules, who: usize, col: u16, row: u16) -> Core {
        let mut c = fresh(rules);
        place(&mut c, who, col, row);
        c.pl.ship[who].pods.first = YES;
        c.pl.ship[who].pods.second = YES;
        c.pl.ship[who].pods.facing = STYLES[who].start_facing;
        // 先畫一幀，砲座的位置才是機身的右後方。
        c.draw_player(who);
        c
    }

    fn press_split(c: &mut Core, who: usize, held: bool) {
        c.port.borrow_mut().split_held[who] = held;
    }

    #[test]
    fn the_split_key_launches_the_pods_forward_and_they_fly_free_of_the_ship() {
        let mut c = armed(FORCE, 0, 5, 8);
        let docked = c.pl.ship[0].pods;
        assert_eq!((docked.col, docked.row), (5 - 2, 5), "玩家 1 預設朝左：砲座在機身左後方");
        press_split(&mut c, 0, true);
        c.update_player(0);
        assert_eq!(c.enh.ship[0].pod, PodMode::Launched);
        assert_eq!(c.pl.ship[0].pods.facing, FACING_RIGHT, "分離後朝前開火");
        let mut cols = Vec::new();
        for _ in 0..8 {
            press_split(&mut c, 0, false);
            c.update_player(0);
            c.draw_player(0);
            cols.push(c.pl.ship[0].pods.col);
        }
        assert_eq!(cols[0], 3 + POD_LAUNCH_COLS as u16, "每步往右 4 欄");
        assert_eq!(*cols.last().unwrap(), 3 + POD_RANGE_COLS as u16, "飛到射程停住");
        assert_eq!(c.pl.ship[0].col, 5, "機身不動");
        assert!(c.pods_detached(0) && c.enh.view()[0].pods_detached);
        // 機身移動，砲座不跟著。
        let k = STYLES[0].keys;
        hold_dirs(&mut c, 0, &[k.down]);
        for _ in 0..3 {
            c.update_player(0);
            c.draw_player(0);
        }
        assert_eq!(c.pl.ship[0].pods.col, 3 + POD_RANGE_COLS as u16);
        assert_eq!(c.pl.ship[0].pods.row, 5, "列也不跟著");
    }

    #[test]
    fn detached_pods_draw_at_their_own_position_and_keep_their_pod_slots() {
        let mut c = armed(FORCE, 1, 5, 8);
        press_split(&mut c, 1, true);
        c.update_player(1);
        c.draw_player(1);
        c.draw_player(1);
        c.draws.clear();
        c.draw_player(1);
        let cmds: Vec<_> = std::mem::take(&mut c.draws)
            .into_iter()
            .filter_map(|d| match d {
                crate::draw::DrawCmd::Blit(b) => Some(b),
                _ => None,
            })
            .collect();
        let pod_cmds: Vec<_> = cmds.iter().filter(|b| b.obj.is_some_and(|o| o.pool == crate::draw::Pool::Pod)).collect();
        assert!(pod_cmds.len() >= 4, "兩座砲座各一組遮罩加四平面");
        assert!(pod_cmds.iter().all(|b| matches!(b.obj.unwrap().slot, 2 | 3)), "玩家 2 用砲座槽 2、3");
        let body = cmds.iter().find(|b| b.obj.is_some_and(|o| o.pool == crate::draw::Pool::Player)).unwrap();
        assert!(pod_cmds[0].x > body.x + 4, "砲座畫在機身前方很遠的地方：{} 對 {}", pod_cmds[0].x, body.x);
    }

    #[test]
    fn detached_pods_hurt_what_they_touch_and_shield_the_ship_from_bullets() {
        let mut c = armed(FORCE, 0, 5, 8);
        press_split(&mut c, 0, true);
        c.update_player(0);
        press_split(&mut c, 0, false);
        for _ in 0..6 {
            c.update_player(0);
            c.draw_player(0);
        }
        let pods = c.pl.ship[0].pods;
        // 實體：砲座 2×2，位置 (pods.col, pods.row)。
        c.en.slots[3].link = crate::mirror::enemy_record_link(3);
        crate::player::test_support::put_g1(&mut c, pods.col, pods.row, 4);
        c.pod_touches_entities(0, pods.col, pods.row);
        assert_eq!(c.en.slots[3].enemy.damage & 0xFF, u16::from(DETACHED_POD_DAMAGE), "分離中的傷害");
        // 附著時是原版的 1。
        let mut d = armed(FORCE, 0, 5, 8);
        d.en.slots[3].link = crate::mirror::enemy_record_link(3);
        crate::player::test_support::put_g1(&mut d, 5, 5, 4);
        d.pod_touches_entities(0, 5, 5);
        assert_eq!(d.en.slots[3].enemy.damage & 0xFF, 1);
        // 敵彈：砲座擋下，機身不受擊（砲座在機身前面，彈在砲座的格）。
        for s in c.eb.slots.iter_mut() {
            s.link = 0x1234;
        }
        crate::player::test_support::put_g2(&mut c, pods.col + 1, pods.row, 0x01);
        c.pod_shields_bullets(0, pods.col, pods.row);
        assert_eq!(c.eb.slots[0].link, 0, "砲座擋下敵彈");
    }

    #[test]
    fn pressing_split_again_recalls_the_pods_and_the_side_decides_the_facing() {
        // 從前方接回：砲座在機身前面，朝右。
        let mut c = armed(FORCE, 1, 5, 8);
        press_split(&mut c, 1, true);
        c.update_player(1);
        press_split(&mut c, 1, false);
        for _ in 0..8 {
            c.update_player(1);
            c.draw_player(1);
        }
        assert!(c.pl.ship[1].pods.col > 12);
        press_split(&mut c, 1, true);
        c.update_player(1);
        assert_eq!(c.enh.ship[1].pod, PodMode::Recalling);
        press_split(&mut c, 1, false);
        let mut steps = 0;
        while c.pods_detached(1) && steps < 40 {
            c.update_player(1);
            c.draw_player(1);
            steps += 1;
        }
        assert!(!c.pods_detached(1), "{steps} 步內接回");
        assert!(steps <= 8, "收回約 {steps} 步（每步 4 欄）");
        assert_eq!(c.pl.ship[1].pods.facing, FACING_RIGHT, "從前方接回朝右");
        c.draw_player(1);
        assert_eq!(c.pl.ship[1].pods.col, 5 + 3, "接回後回到機身的右後方");
        // 從後方接回：機身越過砲座，朝左。
        let mut c = armed(FORCE, 0, 3, 8);
        press_split(&mut c, 0, true);
        c.update_player(0);
        press_split(&mut c, 0, false);
        for _ in 0..8 {
            c.update_player(0);
            c.draw_player(0);
        }
        let stop = c.pl.ship[0].pods.col;
        // 把機身移到砲座右邊，列對到砲座。
        c.pl.ship[0].col = 33;
        c.pl.ship[0].blit_x = 33;
        press_split(&mut c, 0, true);
        c.update_player(0);
        press_split(&mut c, 0, false);
        for _ in 0..12 {
            c.update_player(0);
            c.draw_player(0);
        }
        assert!(stop < 33);
        assert!(!c.pods_detached(0));
        assert_eq!(c.pl.ship[0].pods.facing, FACING_LEFT, "砲座在機身後方（左）接回，朝左");
    }

    #[test]
    fn the_ship_catches_the_pods_by_flying_into_them_from_either_side() {
        // 從後方飛進砲座：砲座在機身前面，朝右。
        let mut c = armed(FORCE, 1, 5, 8);
        press_split(&mut c, 1, true);
        c.update_player(1);
        press_split(&mut c, 1, false);
        for _ in 0..8 {
            c.update_player(1);
            c.draw_player(1);
        }
        assert!(c.pods_detached(1));
        let k = STYLES[1].keys;
        hold_dirs(&mut c, 1, &[k.right]);
        let mut steps = 0;
        while c.pods_detached(1) && steps < 20 {
            c.update_player(1);
            c.draw_player(1);
            steps += 1;
        }
        assert!(!c.pods_detached(1), "機身飛到砲座就接回");
        assert_eq!(c.pl.ship[1].pods.facing, FACING_RIGHT);
        // 起飛後三步內不因接觸而接回。
        let mut c = armed(FORCE, 1, 5, 8);
        c.pl.ship[1].pods.col = 5;
        press_split(&mut c, 1, true);
        c.update_player(1);
        assert_eq!(c.enh.ship[1].pod_age, 0);
        c.fly_free_pods(1);
        assert!(c.pods_detached(1), "剛起飛還在重疊也不接回");
    }

    #[test]
    fn the_split_key_does_nothing_without_pods_or_with_the_option_off() {
        let mut c = fresh(FORCE);
        place(&mut c, 0, 5, 8);
        press_split(&mut c, 0, true);
        c.update_player(0);
        assert_eq!(c.enh.ship[0].pod, PodMode::Docked, "沒有砲座");
        let mut c = armed(Rules::ORIGINAL, 0, 5, 8);
        press_split(&mut c, 0, true);
        c.update_player(0);
        assert_eq!(c.enh.ship[0].pod, PodMode::Docked, "選項關閉");
        // 開著時飛出去，關掉就接回。
        let mut c = armed(FORCE, 0, 5, 8);
        press_split(&mut c, 0, true);
        c.update_player(0);
        press_split(&mut c, 0, false);
        c.draw_player(0);
        assert!(c.pods_detached(0));
        with_rules(&mut c, Rules::ORIGINAL);
        c.update_player(0);
        assert!(!c.pods_detached(0), "選項關掉，砲座接回");
    }

    #[test]
    fn the_turn_key_still_flips_the_facing_while_the_pods_are_free() {
        let mut c = armed(FORCE, 0, 5, 8);
        press_split(&mut c, 0, true);
        c.update_player(0);
        press_split(&mut c, 0, false);
        assert_eq!(c.pl.ship[0].pods.facing, FACING_RIGHT);
        c.set_fire_flag(0, FireFlag::Turn, true);
        c.update_player(0);
        assert_eq!(c.pl.ship[0].pods.facing, FACING_LEFT, "轉向鍵維持前後轉向");
    }

    #[test]
    fn the_stage_start_brings_the_pods_home_and_clears_the_state() {
        let mut c = armed(FORCE, 0, 5, 8);
        press_split(&mut c, 0, true);
        c.update_player(0);
        c.enh.ship[0].charge = 7;
        c.place_players_for_stage();
        assert_eq!(c.enh.ship[0], EnhancedPlayer { split_prev: true, ..EnhancedPlayer::default() });
    }

    // ---- 原版路徑 ----

    #[test]
    fn with_every_option_off_nothing_is_written_beyond_what_the_original_writes() {
        // 同一串輸入，選項全關與沒有強化程式的結果相同：這裡只能比兩個 Core 彼此，
        // 與原版收據逐位元組比對由 tools/rust.sh verify 做。
        let k = STYLES[0].keys;
        let run = |rules: Rules| {
            let mut c = fresh(rules);
            place(&mut c, 0, 7, 8);
            c.pl.ship[0].pods.first = YES;
            let mut trace = Vec::new();
            for step in 0..30 {
                let dirs = if step % 7 < 3 { [k.right, k.down] } else { [k.left, k.up] };
                hold_dirs(&mut c, 0, &dirs);
                if step % 5 == 0 {
                    pressed(&mut c, 0);
                } else if step % 5 == 3 {
                    c.set_key(k.fire, false);
                    c.set_fire_flag(0, FireFlag::NewPress, true);
                }
                if step % 4 == 1 {
                    c.set_fire_flag(0, FireFlag::Event, true);
                }
                press_split(&mut c, 0, step % 6 == 1);
                // 不呼叫 update_player_bullets：測試用的零映像沒有子彈形狀表，寬高 0 在原版的 `loop` 語意是 65536 次。
                c.update_player(0);
                c.draw_player(0);
                c.store_mirrors();
                trace.push((c.ds.bytes().to_vec(), std::mem::take(&mut c.draws).len()));
            }
            trace
        };
        assert_eq!(run(Rules::ORIGINAL), run(Rules { ai_aim: true, ai_fire: true, ai_react: true, ai_boss: true, ..Rules::ORIGINAL }), "AI 選項不影響玩家領域");
        let off = run(Rules::ORIGINAL);
        for rules in [WAVE, AUTO, FORCE, FINE] {
            assert_ne!(run(rules), off, "{rules:?} 開著時行為確實不同");
        }
    }

    #[test]
    fn the_same_input_and_options_give_the_same_result_twice() {
        let all = Rules { wave_charge: true, auto_fire: true, force_pod: true, fine_step: true, ..Rules::ORIGINAL };
        let k = STYLES[0].keys;
        let run = || {
            let mut c = fresh(all);
            place(&mut c, 0, 7, 8);
            c.pl.ship[0].pods.first = YES;
            c.pl.ship[0].pods.second = YES;
            let mut out = Vec::new();
            for step in 0..60 {
                let dirs = if step % 9 < 5 { [k.right, k.down] } else { [k.left, k.up] };
                hold_dirs(&mut c, 0, &dirs);
                if step % 11 == 0 {
                    pressed(&mut c, 0);
                } else if step % 11 == 8 {
                    c.set_key(k.fire, false);
                }
                press_split(&mut c, 0, step % 13 == 2 || step % 13 == 9);
                c.update_player(0);
                c.draw_player(0);
                c.store_mirrors();
                out.push(rs2_formats::sha256_hex(c.ds.bytes()));
            }
            out
        };
        assert_eq!(run(), run());
    }
}
