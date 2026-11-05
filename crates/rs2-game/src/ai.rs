//! 敵人 AI 強化（規格 005「選項」的 AI 四項）。
//!
//! 內容全部是 remake-owned，沒有原版收據可以對拍，行為的依據是規格與單元測試。原版規則永遠保留：
//! `Rules::ORIGINAL`（四個選項全關）時這裡的決策點一個都不執行，原版路徑沒有多出任何資料段寫入、繪製命令，
//! 也沒有多走一步原版的亂數表索引。強化狀態放在 [`AiState`]，不進 `State`。
//!
//! 四個選項各是一個獨立的決策點，彼此可以單獨開關：
//!
//! | 選項 | 決策點 | 呼叫端 |
//! |---|---|---|
//! | `ai_aim` 瞄準更準 | 瞄準彈生成後，把八向腳本換成逐幀的移動碼（[`AimPath`]），朝玩家當下位置（加一點預判）飛 | `ebullet.rs` 的 `fire_enemy_bullets`、`step_bullet` |
//! | `ai_fire` 火力加強 | 原版這一幀沒開火的普通敵機另有一次機會（AI 自己的亂數）；原本從不開火的瞄準彈敵機也偶爾開火 | `entity.rs` 的 `step_enemy` |
//! | `ai_react` 敵人會反應 | 腳本停著或在走的普通敵機，閃避迎面的玩家彈、玩家靠近時對齊玩家的列 | `entity.rs` 的 `step_enemy` |
//! | `ai_boss` 頭目新招式 | 頭目核心依自己的時鐘額外發射既有彈種的組合 | `entity.rs` 的 `step_enemy` |
//!
//! 共同原則：
//!
//! - 不寫新物理。移動只用腳本的八個方向碼，子彈只用既有樣板。
//! - 要新亂數就用 [`AiState`] 自己的狀態，不碰原版的亂數表索引，所以開一個選項不會改變其他選項看到的原版亂數序列。
//! - 不用時間。同樣的輸入與選項，兩次結果相同。
//! - 強度初值寫成有名字的常數，沒有真人驗證，使用者試玩後調。強度隨難度（四段）調整，難度由原版兩個加成字組反推
//!   （[`Core::ai_level`]）。
//! - 只有生成表生成的敵人才有角色（[`Role`]，分類表在 `tables.rs` 的 `enemy_role`）。從快照載入的敵人與片尾的字樣是
//!   [`Role::Inert`]，AI 不碰。

use crate::draw::Pool;
use crate::ebullet::{Aiming, FirePattern, Heading, POOL_SIZE as BULLET_SLOTS};
use crate::entity::{Enemy, Kind, Mode, Motion, NEVER_FIRES, POOL_SIZE as ENEMY_SLOTS};
use crate::game::Core;
use crate::tables::enemy_role;
use crate::pbullet::Behavior;

// ---- 強度初值（沒有真人驗證）----

/// AI 自己的亂數種子（xorshift32，不可為 0）。
const RNG_SEED: u32 = 0x2F6E_2B1D;

/// 預判：用玩家最近一幀的移動量乘上這個幀數當提前量，上限如下（欄、列）。
const AIM_LEAD_FRAMES: i16 = 3;
const AIM_LEAD_MAX_COLS: i16 = 4;
const AIM_LEAD_MAX_ROWS: i16 = 3;
/// 玩家碰撞形狀是 5×1，位置在 (欄, 列 + 1)（文件 11 第 5.1 節）。瞄準它的中心。
const PLAYER_CENTER_COLS: i16 = 2;
const PLAYER_CENTER_ROWS: i16 = 1;

/// 敵人在畫面內的範圍（欄、列）。敵人從欄 40 進場，欄 37 以後還在邊緣，不開火也不反應。
const FIELD_COL_MIN: i16 = 1;
const FIELD_COL_MAX: i16 = 36;
const FIELD_ROW_MIN: i16 = 0;
/// 敵人底緣能到的最低一列（足跡寫到列 12h，文件 11 第 1.3 節）。
const FIELD_ROW_BOTTOM: i16 = 17;

/// 火力加強：原版會開火的普通敵機，這一幀原版沒開火時，額外開火的百分比（依難度 EASY、NORMAL、HARD、HARDEST）。
/// 對照原版：門檻 16 的敵機 EASY 每幀 0%、NORMAL 6.7%、HARDEST 12%；門檻 14 的 6.7%、17%、23%（文件 11 第 8.2 節）。
const ARMED_EXTRA_PERCENT: [u32; 4] = [4, 5, 6, 8];
/// 火力加強：原版從不開火（門檻 63h）的普通敵機，每幀開火的百分比。9.1 幀一秒，3% 約三秒一發。
const SILENT_FIRE_PERCENT: [u32; 4] = [3, 4, 5, 6];
/// 強化規則加的子彈，場上敵彈已有這麼多顆就不加。
const LIVE_BULLET_CAP: usize = 10;
/// 沒有修正 `bullet_slot_carry` 時，強化規則只用槽 0 至 14。槽號 15 起 G2 格值低四位進位（文件 11 第 5.4 節）。
pub(crate) const SAFE_BULLET_SLOTS: usize = 15;

/// 敵人會反應：閃避的偵測範圍（子彈在敵人左緣左邊幾欄內）、每個敵人一生最多閃避幾步。
const DODGE_RANGE_COLS: i16 = 7;
const DODGE_BUDGET: u8 = 6;
/// 敵人會反應：玩家在水平這麼近、腳本停著時對齊玩家的列；列差至少這麼多才動；每個敵人一生最多對齊幾步。
const ALIGN_NEAR_COLS: i16 = 12;
const ALIGN_MIN_ROWS: i16 = 2;
const ALIGN_BUDGET: u8 = 5;

/// 頭目新招式：每一招的週期隨難度縮短這麼多幀，不短於 [`BOSS_PERIOD_MIN`]。
const BOSS_PERIOD_STEP: u16 = 4;
const BOSS_PERIOD_MIN: u16 = 24;

/// 道具載體的 `carrier` 字組。
const CARRIER: u16 = b'P' as u16;
/// 瞄準彈的字母。
const AIMED: u8 = b'F';

// ---- 角色 ----

/// 四個頭目，依關卡。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Boss {
    Stage1,
    Stage2,
    Stage3,
    Stage4,
}

/// 敵人在 AI 強化裡的角色，生成時依樣板位址分類一次。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Role {
    /// 不是生成表生成的（快照載入、片尾字樣）：AI 不碰。
    #[default]
    Inert,
    /// 普通敵人：會被 `ai_fire`、`ai_react` 影響。
    Grunt,
    /// 頭目的控制器、損壞貼片、分件與中頭目：跟著合成圖的腳本走，AI 不碰。
    BossPart,
    /// 頭目核心：`ai_boss` 的新招式從這裡發射。
    BossCore(Boss),
}

/// 一個敵人槽的強化狀態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AiEnemy {
    pub(crate) role: Role,
    /// 生成後經過的幀數。
    pub(crate) clock: u16,
    /// 還能閃避幾步、還能對齊幾步。
    pub(crate) dodges_left: u8,
    pub(crate) aligns_left: u8,
    /// 連發中：還要發幾發、距下一發幾幀。
    pub(crate) burst_left: u8,
    pub(crate) burst_wait: u8,
}

/// 玩家最近一幀的位置與移動量，給預判用。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PlayerTrack {
    prev: Option<(i16, i16)>,
    vel: (i16, i16),
}

impl PlayerTrack {
    fn update(&mut self, cell: Option<(u16, u16)>) {
        match cell {
            Some((col, row)) => {
                let now = (col as i16, row as i16);
                self.vel = self.prev.map_or((0, 0), |p| (now.0.wrapping_sub(p.0), now.1.wrapping_sub(p.1)));
                self.prev = Some(now);
            }
            None => *self = PlayerTrack::default(),
        }
    }
}

// ---- 逐幀路徑 ----

/// 八個方向的移動碼（同 [`Heading`] 的編號）：(水平, 垂直) 各為 -1、0、1，垂直向下為正。
fn move_code(mx: i8, my: i8) -> u8 {
    match (mx, my) {
        (0, -1) => 1,
        (1, -1) => 2,
        (1, 0) => 3,
        (1, 1) => 4,
        (0, 1) => 5,
        (-1, 1) => 6,
        (-1, 0) => 7,
        (-1, -1) => 8,
        _ => 0,
    }
}

/// 四捨五入的整數除法，`d` 為正。
fn div_round(n: i32, d: i32) -> i32 {
    if n >= 0 { (n + d / 2) / d } else { -((-n + d / 2) / d) }
}

/// 一顆敵彈的逐幀路徑：每幀吐一個移動碼，累積起來是從發射點到瞄準點的直線（Bresenham）。
///
/// 主軸每幀走一格，副軸依比例走。單位是子彈自己的單步（記錄的 `dcol`、`drow`），所以彈速和原版同一個等級：
/// 瞄準彈每幀 1 欄或 1 列，寬彈每幀 4 欄。不寫新物理，吐出的只是腳本裡本來就有的八個碼。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AimPath {
    /// 配路徑時這個槽的世代（`Core::bump_gen`）。槽被別顆子彈重用後世代不同，路徑自動失效。
    generation: u32,
    step_x: i8,
    step_y: i8,
    major: u32,
    minor: u32,
    x_major: bool,
    step: u32,
    taken: u32,
}

impl AimPath {
    /// `dx`、`dy` 是從發射點到瞄準點的單步數（向右、向下為正）。`west_only` 的寬彈只往西，目標在東邊或同欄就直走。
    pub(crate) fn new(dx: i32, dy: i32, west_only: bool, generation: u32) -> AimPath {
        let (mut ax, mut ay) = (dx.unsigned_abs(), dy.unsigned_abs());
        let (mut step_x, step_y) = (dx.signum() as i8, dy.signum() as i8);
        if west_only {
            step_x = -1;
            if dx >= 0 {
                (ax, ay) = (1, 0);
            } else {
                ay = ay.min(ax);
            }
        }
        if ax == 0 && ay == 0 {
            // 目標就在發射點上：和原版同一格的瞄準一樣取西。
            (step_x, ax) = (-1, 1);
        }
        let x_major = ax >= ay;
        let (major, minor) = if x_major { (ax, ay) } else { (ay, ax) };
        AimPath { generation, step_x, step_y, major, minor, x_major, step: 0, taken: 0 }
    }

    /// 這一幀的移動碼（1 至 8）。
    pub(crate) fn next(&mut self) -> u8 {
        self.step = self.step.saturating_add(1);
        // 走了 step 個主軸格之後，副軸應該累積 round(step × minor / major) 格。
        let want = (2 * u64::from(self.step) * u64::from(self.minor) + u64::from(self.major)) / (2 * u64::from(self.major));
        let minor_step = want > u64::from(self.taken);
        if minor_step {
            self.taken += 1;
        }
        let (mx, my) = if self.x_major {
            (self.step_x, if minor_step { self.step_y } else { 0 })
        } else {
            (if minor_step { self.step_x } else { 0 }, self.step_y)
        };
        move_code(mx, my)
    }
}

// ---- 狀態 ----

/// 敵人 AI 強化的全部狀態。
#[derive(Clone, Debug)]
pub(crate) struct AiState {
    rng: u32,
    pub(crate) enemies: [AiEnemy; ENEMY_SLOTS],
    paths: [Option<AimPath>; BULLET_SLOTS],
    track: [PlayerTrack; 2],
}

impl Default for AiState {
    fn default() -> AiState {
        AiState { rng: RNG_SEED, enemies: [AiEnemy::default(); ENEMY_SLOTS], paths: [None; BULLET_SLOTS], track: [PlayerTrack::default(); 2] }
    }
}

impl AiState {
    /// xorshift32。
    fn next_u32(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// 0 至 `n` - 1。
    fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n
    }

    /// 生成表生成了一個敵人（`Core::spawn_enemy`）：分類並重設這一槽的狀態。
    pub(crate) fn on_spawn(&mut self, slot: usize, template: u16) {
        self.enemies[slot] = AiEnemy { role: enemy_role(template), dodges_left: DODGE_BUDGET, aligns_left: ALIGN_BUDGET, ..AiEnemy::default() };
    }

    /// 這個敵人過了一幀。
    pub(crate) fn tick_enemy(&mut self, slot: usize) {
        let e = &mut self.enemies[slot];
        e.clock = e.clock.saturating_add(1);
    }
}

/// 敵人是不是在畫面內（可以開火或反應的範圍）。`extra` 是戰場多出來的欄數（規格 005「戰場大小」），右界跟著加。
fn on_field(enemy: &Enemy, extra: i16) -> bool {
    let (col, row) = (enemy.col as i16, enemy.row as i16);
    (FIELD_COL_MIN..=FIELD_COL_MAX + extra).contains(&col) && (FIELD_ROW_MIN..=FIELD_ROW_BOTTOM).contains(&row)
}

impl Core {
    /// 難度四段（0 EASY、1 NORMAL、2 HARD、3 HARDEST），由原版的兩個加成字組反推（文件 11 第 9.1 節）。
    /// 沒進選單直接開始時兩個字都是 0，等於 EASY。
    pub(crate) fn ai_level(&self) -> usize {
        match (self.hp_bonus(), self.fire_bonus()) {
            (0, 0) => 0,
            (0, _) => 1,
            (_, 0..=2) => 2,
            _ => 3,
        }
    }

    /// 每幀一次（`update_enemies` 開頭）：記下玩家的位置，給預判用。不碰資料段，選項開不開都做。
    pub(crate) fn ai_begin_frame(&mut self) {
        for who in 0..2 {
            let cell = self.player_alive(who).then(|| self.player_cell(who));
            self.ai.track[who].update(cell);
        }
    }

    /// 敵彈能用的槽數。原版是整個池；強化規則會多放子彈（`ai_fire`、`ai_boss`），沒有修正 `bullet_slot_carry` 時
    /// 把全部敵彈（含原版的圖案）限制在槽 0 至 14，不讓更多子彈在飛而更常撞上槽號 15 起的格值進位缺陷。
    pub(crate) fn enemy_bullet_slot_limit(&self) -> usize {
        let rules = self.rules();
        if (rules.ai_fire || rules.ai_boss) && !self.fixes.bullet_slot_carry { SAFE_BULLET_SLOTS } else { BULLET_SLOTS }
    }

    // ---- ai_aim ----

    /// 瞄準點：最近玩家的碰撞形狀中心，加一點預判，夾在畫面內。兩位都不在場時回傳 `None`，沿用原版的瞄準。
    fn aim_point(&self, from_col: u16) -> Option<(i16, i16)> {
        if !self.player_alive(0) && !self.player_alive(1) {
            return None;
        }
        let who = self.aim_target_player(from_col);
        let (col, row) = self.player_cell(who);
        let (vc, vr) = self.ai.track[who].vel;
        let lead_c = vc.saturating_mul(AIM_LEAD_FRAMES).clamp(-AIM_LEAD_MAX_COLS, AIM_LEAD_MAX_COLS);
        let lead_r = vr.saturating_mul(AIM_LEAD_FRAMES).clamp(-AIM_LEAD_MAX_ROWS, AIM_LEAD_MAX_ROWS);
        let c = (col as i16).wrapping_add(PLAYER_CENTER_COLS).wrapping_add(lead_c).clamp(FIELD_COL_MIN, FIELD_COL_MAX + 2 + self.field_extra() as i16);
        let r = (row as i16).wrapping_add(PLAYER_CENTER_ROWS).wrapping_add(lead_r).clamp(FIELD_ROW_MIN, FIELD_ROW_BOTTOM);
        Some((c, r))
    }

    /// 瞄準更準（`ai_aim`）：`slot` 剛從原版的八向樣板生成，改成朝玩家當下位置飛的逐幀路徑。
    /// 只處理敵人的瞄準彈與寬彈；道具球不動。選項沒開、沒有玩家或樣板的單步為 0 時什麼都不做。
    pub(crate) fn refine_aim(&mut self, slot: usize, aiming: Aiming, col: u16, row: u16) {
        if !self.rules().ai_aim {
            return;
        }
        let west_only = match aiming {
            Aiming::Shot => false,
            Aiming::Wide => true,
            Aiming::Capsule(_) => return,
        };
        let Some((target_col, target_row)) = self.aim_point(col) else { return };
        let bullet = &self.eb.slots[slot].bullet;
        let (unit_col, unit_row) = (i32::from(bullet.dcol as i16), i32::from(bullet.drow as i16));
        if unit_col <= 0 || unit_row <= 0 {
            return;
        }
        let dx = i32::from(target_col) - i32::from(col as i16);
        let dy = i32::from(target_row) - i32::from(row as i16);
        let generation = self.gens[Pool::EnemyBullet as usize][slot];
        self.ai.paths[slot] = Some(AimPath::new(div_round(dx, unit_col), div_round(dy, unit_row), west_only, generation));
    }

    /// 這一顆敵彈有逐幀路徑就回傳這一幀的移動碼，沒有（含世代不符）回傳 `None`，由呼叫端讀腳本。
    pub(crate) fn ai_path_code(&mut self, slot: u8) -> Option<u8> {
        let generation = self.gens[Pool::EnemyBullet as usize][slot as usize];
        let path = self.ai.paths[slot as usize].as_mut()?;
        (path.generation == generation).then(|| path.next())
    }

    // ---- 共用 ----

    /// 敵人形狀的寬與高。原版最大的形狀是 59×19，夾在 0 至 64，免得壞資料讓後面的 i16 運算溢位。
    fn footprint(&self, enemy: &Enemy) -> (i16, i16) {
        let size = |w: u16| (w as i16).clamp(0, 64);
        let shape = self.tables.shape(enemy.shape);
        (size(shape.width), size(shape.height))
    }

    /// 普通的、會移動也會被打的敵機：`ai_fire` 與 `ai_react` 的對象。
    fn is_plain_grunt(&self, enemy: &Enemy, slot: usize) -> bool {
        self.ai.enemies[slot].role == Role::Grunt
            && enemy.mode() == Mode::Script
            && enemy.kind() == Kind::Normal
            && enemy.hp < 999
            && enemy.carrier != CARRIER
    }

    // ---- ai_fire ----

    /// 火力加強（`ai_fire`）：原版這一幀沒開火的普通敵機另有一次機會，用 AI 自己的亂數擲，
    /// 原版的亂數表索引已由 `rolls_to_fire` 推進，這裡不碰。
    ///
    /// - 原版會開火的（門檻小於 63h）：每幀額外 [`ARMED_EXTRA_PERCENT`] 的機率，原版 EASY 永遠不開火的門檻 16 也開得了。
    /// - 原版從不開火的瞄準彈敵機：每幀 [`SILENT_FIRE_PERCENT`] 的機率。
    ///
    /// 只在敵人進到畫面內時；場上敵彈已有 [`LIVE_BULLET_CAP`] 顆就不加。
    pub(crate) fn ai_extra_fire(&mut self, enemy: &Enemy, slot: usize) {
        if !self.is_plain_grunt(enemy, slot) || !on_field(enemy, self.field_extra() as i16) || self.eb.active_count() >= LIVE_BULLET_CAP {
            return;
        }
        let level = self.ai_level();
        let fires = if enemy.fire_threshold == NEVER_FIRES {
            enemy.bullet == u16::from(AIMED) && self.ai.below(100) < SILENT_FIRE_PERCENT[level]
        } else {
            self.ai.below(100) < ARMED_EXTRA_PERCENT[level]
        };
        if fires {
            self.fire_enemy_bullets(enemy.bullet, enemy.col, enemy.row);
        }
    }

    // ---- ai_react ----

    /// 敵人會反應（`ai_react`）：回傳這一幀要執行的移動碼。腳本照常前進，只換這一幀的動作。
    ///
    /// 1. 閃避：玩家彈在敵人左邊 [`DODGE_RANGE_COLS`] 欄內、列重疊，就垂直離開子彈的列。保留腳本原本的水平分量
    ///    （西加上下是西北、西南，都是腳本裡本來就有的碼），腳本原本的垂直分量被取代。
    /// 2. 對齊：腳本停著（碼 0）、玩家在水平 [`ALIGN_NEAR_COLS`] 欄內，朝玩家的列走一步。
    ///
    /// 兩種都有每個敵人一生的步數上限，離開畫面上下緣的動作一律不做。腳本走移動模式以外的碼（追蹤、右跳、發射、結束）時不插手。
    pub(crate) fn ai_react(&mut self, enemy: &Enemy, slot: usize, code: u8) -> u8 {
        if !self.is_plain_grunt(enemy, slot) || !on_field(enemy, self.field_extra() as i16) {
            return code;
        }
        let motion = Motion::from_code(code);
        if !matches!(motion, Motion::Hold | Motion::Go(_)) {
            return code;
        }
        let (width, height) = self.footprint(enemy);
        let (row, top_limit, bottom_limit) = (enemy.row as i16, FIELD_ROW_MIN, FIELD_ROW_BOTTOM - (height - 1).max(0));
        let can_go = |up: bool| if up { row > top_limit } else { row < bottom_limit };
        let state = self.ai.enemies[slot];
        if state.dodges_left > 0 {
            if let Some(bullet_row) = self.incoming_shot_row(enemy, width, height) {
                let center2 = 2 * row + height - 1;
                // 子彈的列在敵人中心以上就往下躲，以下就往上；那個方向沒有空間就反方向。
                let prefer_up = 2 * bullet_row > center2;
                let up = if can_go(prefer_up) { Some(prefer_up) } else if can_go(!prefer_up) { Some(!prefer_up) } else { None };
                if let Some(up) = up {
                    self.ai.enemies[slot].dodges_left -= 1;
                    return with_vertical(motion, up);
                }
            }
        }
        if matches!(motion, Motion::Hold) && state.aligns_left > 0 {
            if let Some(up) = self.align_direction(enemy, height) {
                if can_go(up) {
                    self.ai.enemies[slot].aligns_left -= 1;
                    return with_vertical(motion, up);
                }
            }
        }
        code
    }

    /// 迎面的玩家彈：回傳最近那顆的列。雷射是生成當幀就結算的光束，不算。
    fn incoming_shot_row(&self, enemy: &Enemy, width: i16, height: i16) -> Option<i16> {
        let (col, row) = (enemy.col as i16, enemy.row as i16);
        let mut best: Option<(i16, i16)> = None;
        for s in self.pb.slots.iter().filter(|s| s.is_active()) {
            if matches!(s.bullet.behavior(), Behavior::LaserRight | Behavior::LaserLeft) {
                continue;
            }
            let (bc, br) = (s.bullet.col as i16, s.bullet.row as i16);
            let gap = col.wrapping_sub(bc);
            // 子彈在敵人身體裡（gap 介於 -寬 與 0）也算：下一幀就命中。
            if !(-width..=DODGE_RANGE_COLS).contains(&gap) || br < row - 1 || br > row + height {
                continue;
            }
            if best.is_none_or(|(g, _)| gap.abs() < g) {
                best = Some((gap.abs(), br));
            }
        }
        best.map(|(_, br)| br)
    }

    /// 對齊玩家的列要往哪走：玩家在水平範圍內、列差夠大才動。`Some(true)` 是往上。
    fn align_direction(&self, enemy: &Enemy, height: i16) -> Option<bool> {
        if !self.player_alive(0) && !self.player_alive(1) {
            return None;
        }
        let who = self.aim_target_player(enemy.col);
        let (pc, pr) = self.player_cell(who);
        // 玩家的座標在死亡動畫裡可以很離譜，全部用 i32 算，不會溢位。
        let (ec, er, pc, pr) = (i32::from(enemy.col as i16), i32::from(enemy.row as i16), i32::from(pc as i16), i32::from(pr as i16));
        if (ec - pc).abs() > i32::from(ALIGN_NEAR_COLS) {
            return None;
        }
        let diff = (pr + i32::from(PLAYER_CENTER_ROWS)) - (er + i32::from((height - 1) / 2));
        (diff.abs() >= i32::from(ALIGN_MIN_ROWS)).then_some(diff < 0)
    }

    // ---- ai_boss ----

    /// 頭目新招式（`ai_boss`）：頭目核心每幀看自己的時鐘，到了就從核心中心發射既有彈種的組合。
    ///
    /// - 過關旗標設起後不再發射。
    /// - 發射點（核心中心）不在畫面內時不發射：第一關核心從畫面下方升起，第二、三關頭目有一半在畫面外。
    /// - 發射點離玩家太近（[`BOSS_SAFE_RANGE`]）時略過，免得沒有反應時間。
    pub(crate) fn ai_boss_step(&mut self, enemy: &Enemy, slot: usize) {
        let Role::BossCore(boss) = self.ai.enemies[slot].role else { return };
        if self.stage_cleared() {
            return;
        }
        let (width, height) = self.footprint(enemy);
        let (col, row) = (enemy.col.wrapping_add((width / 2) as u16), enemy.row.wrapping_add((height / 2) as u16));
        if !self.boss_may_fire(col, row) {
            return;
        }
        // 進行中的連發。
        let state = self.ai.enemies[slot];
        if state.burst_left > 0 {
            if state.burst_wait == 0 {
                self.boss_shot(AIMED, col, row);
                let s = &mut self.ai.enemies[slot];
                s.burst_left -= 1;
                s.burst_wait = BURST_GAP - 1;
            } else {
                self.ai.enemies[slot].burst_wait -= 1;
            }
        }
        let level = self.ai_level();
        let clock = self.ai.enemies[slot].clock;
        for attack in boss_attacks(boss) {
            let period = attack.period.saturating_sub(BOSS_PERIOD_STEP * level as u16).max(BOSS_PERIOD_MIN);
            if clock < attack.first || (clock - attack.first) % period != 0 {
                continue;
            }
            match attack.action {
                Action::Volley(letter) => self.boss_shot(letter, col, row),
                Action::Burst(count) => {
                    self.boss_shot(AIMED, col, row);
                    let s = &mut self.ai.enemies[slot];
                    s.burst_left = count.saturating_sub(1);
                    s.burst_wait = BURST_GAP - 1;
                }
            }
        }
    }

    /// 發射點 (`col`, `row`) 在畫面內，而且離每一位在場玩家都不近於 [`BOSS_SAFE_RANGE`]（欄差加列差）。
    fn boss_may_fire(&self, col: u16, row: u16) -> bool {
        let (c, r) = (col as i16, row as i16);
        if !(FIELD_COL_MIN..=BOSS_COL_MAX + self.field_extra() as i16).contains(&c) || !(FIELD_ROW_MIN..=FIELD_ROW_BOTTOM).contains(&r) {
            return false;
        }
        (0..2).filter(|&who| self.player_alive(who)).all(|who| {
            let (pc, pr) = self.player_cell(who);
            (i32::from(c) - i32::from(pc as i16)).abs() + (i32::from(r) - i32::from(pr as i16)).abs() >= i32::from(BOSS_SAFE_RANGE)
        })
    }

    /// 頭目新招式的一次開火。頭目戰的敵彈池常常是滿的（第三關的彈幕、第四關永不消失的反彈球），
    /// 新招式要先騰出空間（`EnemyBullets::make_room`），整個圖案才會完整出現。
    /// 騰出空間只會讓池回到原版也會有的滿載，不會比原版更密。
    fn boss_shot(&mut self, letter: u8, col: u16, row: u16) {
        let needed = FirePattern::from_letter(u16::from(letter)).bullet_count();
        let limit = self.enemy_bullet_slot_limit();
        if self.eb.make_room(limit, needed) {
            self.fire_enemy_bullets(u16::from(letter), col, row);
        }
    }
}

/// 垂直分量換成 `up`（往上或往下），保留腳本原本的水平分量。
fn with_vertical(motion: Motion, up: bool) -> u8 {
    let horizontal = match motion {
        Motion::Go(Heading::NE | Heading::E | Heading::SE) => 1,
        Motion::Go(Heading::NW | Heading::W | Heading::SW) => -1,
        _ => 0,
    };
    move_code(horizontal, if up { -1 } else { 1 })
}

// ---- 頭目新招式表 ----

/// 連發每發相隔的幀數。
const BURST_GAP: u8 = 3;
/// 發射點的欄上限（欄 38 以後的子彈第一步就出界）。
const BOSS_COL_MAX: i16 = 37;
/// 發射點離玩家（欄差加列差）至少這麼遠才發射。
const BOSS_SAFE_RANGE: i16 = 7;

/// 一招做什麼。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    /// 連續幾發瞄準彈（字母 `F`），每發重新瞄準，所以會跟著玩家移動。
    Burst(u8),
    /// 一次噴發一個既有開火字母的圖案（`ebullet.rs` 的 `FirePattern`）。
    Volley(u8),
}

/// 一招：核心生成後第 `first` 幀起，每 `period` 幀做一次。時鐘是這個敵人自己的幀數，頭目戰的關卡幀計數會釘住，不能用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Attack {
    first: u16,
    period: u16,
    action: Action,
}

const fn attack(first: u16, period: u16, action: Action) -> Attack {
    Attack { first, period, action }
}

/// 第一關頭目（核心原版從不開火，在畫面裡四處遊走）：三連瞄準彈、環形彈。
const STAGE1_ATTACKS: &[Attack] = &[attack(24, 30, Action::Burst(3)), attack(48, 72, Action::Volley(b'<'))];
/// 第二關頭目（核心是 K 型，不畫出來，跟合成圖一起從右邊進場）：瞄準寬彈、五連扇。
const STAGE2_ATTACKS: &[Attack] = &[attack(30, 40, Action::Volley(b'B')), attack(60, 64, Action::Volley(b'L'))];
/// 第三關頭目（噴嘴原版就會發射 `E` 彈幕，池常常是滿的）：四連瞄準彈、北／西／西北三向。
const STAGE3_ATTACKS: &[Attack] = &[attack(40, 48, Action::Burst(4)), attack(70, 80, Action::Volley(b'X'))];
/// 第四關頭目（圓盤原版發射永不消失的反彈球）：三連瞄準彈、環形彈。
const STAGE4_ATTACKS: &[Attack] = &[attack(30, 36, Action::Burst(3)), attack(54, 72, Action::Volley(b'<'))];

fn boss_attacks(boss: Boss) -> &'static [Attack] {
    match boss {
        Boss::Stage1 => STAGE1_ATTACKS,
        Boss::Stage2 => STAGE2_ATTACKS,
        Boss::Stage3 => STAGE3_ATTACKS,
        Boss::Stage4 => STAGE4_ATTACKS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebullet::{Bullet, Template, Tilt};
    use crate::entity::FIRE_ROLLS;
    use crate::game::{Fixes, Rules, Start};
    use crate::ebullet::record_link;
    use crate::entity::enemy_record_link;
    use crate::pbullet::pb_record_link;
    use crate::pbullet::{PlayerBullet, Slot as PlayerSlot};

    const ALL: Rules = Rules { ai_aim: true, ai_fire: true, ai_react: true, ai_boss: true, ..Rules::ORIGINAL };
    const AIM: Rules = Rules { ai_aim: true, ..Rules::ORIGINAL };
    const FIRE: Rules = Rules { ai_fire: true, ..Rules::ORIGINAL };
    const REACT: Rules = Rules { ai_react: true, ..Rules::ORIGINAL };
    const BOSS: Rules = Rules { ai_boss: true, ..Rules::ORIGINAL };

    fn core_with(rules: Rules, fixes: Fixes) -> Core {
        let mut c = Core::for_test(Start::Demo, fixes);
        // 展示錄影播放中一律原版規則（`Core::rules`）；這些測試要的是正式遊戲。
        c.set_demo_playing(false);
        c.fill_row_tables();
        c.draws.clear();
        c.port.borrow_mut().rules = rules;
        c
    }

    fn core(rules: Rules) -> Core {
        core_with(rules, Fixes::NONE)
    }

    fn put_shape(c: &mut Core, at: u16, w: u16, h: u16, cell: u8) {
        c.tables_mut().put_shape(at, w, h, cell);
    }

    /// 擺一筆敵彈樣板。
    fn put_bullet_template(c: &mut Core, template: Template, step: u16, frames: u16, script: u16, shape: u16) {
        let words = [0u16, frames, 0, 0, step, 10, 0, 0, 0, 0, 0, step, 1, script, shape];
        c.tables_mut().put_bullet(template, Bullet::from_words(words));
    }

    /// 擺一種 1×1 的朝向 `h` 的瞄準彈樣板：每步 1 欄 1 列，腳本全是移動碼 `code`，結尾 `F7` 循環。
    fn install_shot(c: &mut Core, h: Heading, code: u8) {
        put_shape(c, 0x7100, 1, 1, 1);
        c.tables_mut().put_words(0x7200, &(0..9u16).map(|i| 40 + i).collect::<Vec<_>>());
        let script = 0x7300 + 0x10 * h.index();
        let mut bytes = [code; 8];
        bytes[7] = 0xF7;
        c.tables_mut().put_bytes(script, &bytes);
        put_bullet_template(c, Template::Aimed(h), 1, 0x7200, script, 0x7100);
    }

    /// 擺 5×1 的寬彈樣板（三種傾斜）：每步 4 欄 1 列，腳本全是同一個碼（西北 8、西 7、西南 6）。
    fn install_wide(c: &mut Core) {
        put_shape(c, 0x7110, 5, 1, 1);
        c.tables_mut().put_words(0x7220, &[13; 9]);
        for (n, (tilt, code)) in [(Tilt::NorthWest, 8u8), (Tilt::West, 7), (Tilt::SouthWest, 6)].into_iter().enumerate() {
            let script = 0x7400 + 0x10 * n as u16;
            let mut bytes = [code; 8];
            bytes[7] = 0xF7;
            c.tables_mut().put_bytes(script, &bytes);
            put_bullet_template(c, Template::Wide(tilt), 4, 0x7220, script, 0x7110);
        }
    }

    /// 全表亂數都是 `value`。
    fn fill_rolls(c: &mut Core, value: u16) {
        for i in 0..FIRE_ROLLS {
            c.tables_mut().put_fire_roll(usize::from(i), value);
        }
    }

    /// 擺一個 2×2 的普通敵機在槽 `slot`，腳本是 `script`（結尾補 0F）。角色是普通敵人。
    fn grunt(c: &mut Core, slot: usize, col: u16, row: u16, threshold: u16, script: &[u8]) {
        put_shape(c, 0x7000, 2, 2, 1);
        c.tables_mut().put_words(0x7100, &(0..9u16).map(|i| 20 + i).collect::<Vec<_>>());
        let at = 0x7800 + 0x100 * slot as u16;
        let mut bytes = script.to_vec();
        bytes.push(0x0F);
        c.tables_mut().put_bytes(at, &bytes);
        let enemy = Enemy {
            hp: 3,
            frames: 0x7100,
            blit_x: col,
            blit_y: row * 10,
            blit_dx: 1,
            blit_dy: 10,
            fire_threshold: threshold,
            bullet: u16::from(b'F'),
            mode: u16::from(b'N'),
            col,
            row,
            dcol: 1,
            drow: 1,
            script: at,
            shape: 0x7000,
            explosion: u16::from(b'B'),
            kind: u16::from(b'O'),
            carrier: u16::from(b'0'),
            ..Enemy::default()
        };
        c.en.slots[slot].link = enemy_record_link(slot);
        c.en.slots[slot].enemy = enemy;
        c.ai.on_spawn(slot, 0x42FE);
    }

    fn spawned(c: &Core) -> u32 {
        c.gens[Pool::EnemyBullet as usize].iter().sum()
    }

    fn player_shot(c: &mut Core, slot: usize, col: u16, row: u16) {
        c.pb.slots[slot] = PlayerSlot { link: pb_record_link(slot), bullet: PlayerBullet { col, row, ..PlayerBullet::default() } };
    }

    // ---- 逐幀路徑 ----

    fn delta(code: u8) -> (i32, i32) {
        match code {
            1 => (0, -1),
            2 => (1, -1),
            3 => (1, 0),
            4 => (1, 1),
            5 => (0, 1),
            6 => (-1, 1),
            7 => (-1, 0),
            8 => (-1, -1),
            _ => panic!("移動碼 {code} 不在 1 至 8"),
        }
    }

    #[test]
    fn aim_path_is_a_straight_line_with_one_major_cell_per_frame() {
        for dx in -15..=15i32 {
            for dy in -9..=9i32 {
                let mut path = AimPath::new(dx, dy, false, 0);
                let (mut x, mut y) = (0i32, 0i32);
                let major = dx.abs().max(dy.abs()).max(1);
                for k in 1..=major {
                    let (mx, my) = delta(path.next());
                    x += mx;
                    y += my;
                    let along = if dx.abs() >= dy.abs() { x.abs() } else { y.abs() };
                    assert_eq!(along, k, "({dx}, {dy}) 第 {k} 幀主軸前進 1 格");
                }
                let want = if (dx, dy) == (0, 0) { (-1, 0) } else { (dx, dy) };
                assert_eq!((x, y), want, "({dx}, {dy}) 走到瞄準點");
            }
        }
    }

    #[test]
    fn aim_path_codes_are_the_alternations_the_original_scripts_use() {
        // 原版 ENE 腳本是 02 03 交替：欄 2、列 1。
        let mut path = AimPath::new(2, -1, false, 0);
        let codes: Vec<u8> = (0..4).map(|_| path.next()).collect();
        assert!(codes.iter().all(|c| matches!(c, 2 | 3)), "{codes:?}");
        assert_eq!(codes.iter().filter(|&&c| c == 2).count(), 2);
        // 正北、正東、正南、正西各只有一個碼。
        for (dx, dy, code) in [(0, -5, 1u8), (5, 0, 3), (0, 5, 5), (-5, 0, 7)] {
            let mut p = AimPath::new(dx, dy, false, 0);
            assert!((0..5).all(|_| p.next() == code), "({dx}, {dy})");
        }
    }

    #[test]
    fn west_only_paths_never_turn_east_and_clamp_the_slope_to_one_row_per_step() {
        let mut p = AimPath::new(3, 2, true, 0);
        assert!((0..4).all(|_| p.next() == 7), "目標在東邊：直走西");
        let mut p = AimPath::new(-6, 5, true, 0);
        let steps: Vec<(i32, i32)> = (0..6).map(|_| delta(p.next())).collect();
        assert!(steps.iter().all(|&(mx, _)| mx == -1));
        assert_eq!(steps.iter().map(|&(_, my)| my).sum::<i32>(), 5);
        // 列差比欄差大：每步最多一列，所以最多 6 列。
        let mut p = AimPath::new(-2, 9, true, 0);
        let rows: i32 = (0..2).map(|_| delta(p.next()).1).sum();
        assert_eq!(rows, 2);
    }

    // ---- ai_aim ----

    #[test]
    fn aim_flies_to_the_player_hit_box_instead_of_the_nearest_band() {
        // 發射點 (5, 7)，玩家 (20, 10)：原版三個頻帶判成「同一列」，正東直飛，從玩家上方 4 列掠過。
        for (rules, want) in [(Rules::ORIGINAL, (22u16, 7u16)), (AIM, (22, 11))] {
            let mut c = core(rules);
            install_shot(&mut c, Heading::E, 3);
            c.set_players([Some((20, 10)), None]);
            c.fire_enemy_bullets(u16::from(b'F'), 5, 7);
            assert!(c.eb.slots[0].is_active());
            for _ in 0..17 {
                c.update_enemy_bullets();
            }
            let b = c.eb.slots[0].bullet;
            assert_eq!((b.col, b.row), want, "ai_aim = {}", rules.ai_aim);
        }
    }

    #[test]
    fn original_rules_never_make_a_path() {
        let mut c = core(Rules::ORIGINAL);
        install_shot(&mut c, Heading::E, 3);
        c.set_players([Some((20, 10)), None]);
        c.ai.track[0].vel = (2, 1);
        c.fire_enemy_bullets(u16::from(b'F'), 5, 7);
        assert!(c.ai.paths.iter().all(Option::is_none));
        assert_eq!(c.ai_path_code(0), None);
        assert_eq!(c.enemy_bullet_slot_limit(), BULLET_SLOTS);
    }

    #[test]
    fn aim_leads_the_target_a_little_and_clamps_the_lead() {
        let mut c = core(AIM);
        c.set_players([Some((20, 10)), None]);
        assert_eq!(c.aim_point(5), Some((22, 11)), "不動的玩家：碰撞形狀中心");
        c.ai.track[0].vel = (1, 0);
        assert_eq!(c.aim_point(5), Some((25, 11)), "每幀 1 欄，提前 3 幀");
        c.ai.track[0].vel = (2, -1);
        assert_eq!(c.aim_point(5), Some((26, 8)), "欄的提前量夾在 4，列的夾在 3");
        c.set_players([None, None]);
        assert_eq!(c.aim_point(5), None, "兩位都不在場：沿用原版");
    }

    #[test]
    fn player_track_measures_the_last_move_and_forgets_a_dead_player() {
        let mut c = core(AIM);
        c.set_players([Some((10, 5)), None]);
        c.ai_begin_frame();
        assert_eq!(c.ai.track[0].vel, (0, 0), "第一幀沒有前一幀");
        c.set_player_cell(0, 12, 4);
        c.ai_begin_frame();
        assert_eq!(c.ai.track[0].vel, (2, -1));
        c.set_players([None, None]);
        c.ai_begin_frame();
        assert_eq!(c.ai.track[0], PlayerTrack::default());
    }

    #[test]
    fn wide_shots_get_a_path_that_reaches_the_players_row() {
        // 原版的微偏西南樣板這裡是每步往南一列，6 步到列 14；路徑讓寬彈在同樣 6 步落在玩家的列 13。
        for (rules, want) in [(Rules::ORIGINAL, (6u16, 14u16)), (AIM, (6, 13))] {
            let mut c = core(rules);
            install_wide(&mut c);
            c.set_players([Some((5, 12)), None]);
            c.fire_enemy_bullets(u16::from(b'B'), 30, 7);
            let slot = (0..BULLET_SLOTS).find(|&i| c.eb.slots[i].is_active()).expect("寬彈");
            for _ in 0..6 {
                c.update_enemy_bullets();
            }
            let b = c.eb.slots[slot].bullet;
            assert_eq!((b.col, b.row), want, "ai_aim = {}", rules.ai_aim);
        }
        // 玩家在東邊：寬彈只往西，路徑直走西。
        let mut c = core(AIM);
        install_wide(&mut c);
        c.set_players([Some((38, 12)), None]);
        c.fire_enemy_bullets(u16::from(b'B'), 30, 7);
        let slot = (0..BULLET_SLOTS).find(|&i| c.eb.slots[i].is_active()).expect("寬彈");
        let path = c.ai.paths[slot].expect("有路徑");
        let mut p = path;
        assert!((0..3).all(|_| p.next() == 7));
    }

    #[test]
    fn the_title_demo_always_plays_by_the_original_rules() {
        // 展示錄影是原版錄下的輸入，AI 強化會讓它對不上：選項全開也一樣是原版。
        let mut c = core(ALL);
        install_shot(&mut c, Heading::E, 3);
        c.set_players([Some((20, 10)), None]);
        assert_eq!(c.rules(), ALL);
        c.set_demo_playing(true);
        assert_eq!(c.rules(), Rules::ORIGINAL);
        c.fire_enemy_bullets(u16::from(b'F'), 5, 7);
        assert!(c.ai.paths.iter().all(Option::is_none));
        assert_eq!(c.enemy_bullet_slot_limit(), BULLET_SLOTS);
    }

    #[test]
    fn capsules_and_stale_paths_are_left_alone() {
        let mut c = core(AIM);
        install_shot(&mut c, Heading::E, 3);
        c.set_players([Some((20, 10)), None]);
        c.fire_enemy_bullets(u16::from(b'l'), 5, 7);
        assert!(c.ai.paths.iter().all(Option::is_none), "道具球不換路徑");
        // 路徑的世代和槽不符就失效：槽被別顆子彈重用。
        c.fire_enemy_bullets(u16::from(b'F'), 5, 7);
        let slot = (0..BULLET_SLOTS).find(|&i| c.ai.paths[i].is_some()).expect("瞄準彈有路徑");
        assert!(c.ai_path_code(slot as u8).is_some());
        c.bump_gen(Pool::EnemyBullet, slot as u8);
        assert_eq!(c.ai_path_code(slot as u8), None);
    }

    // ---- ai_fire ----

    #[test]
    fn fire_boost_keeps_every_bullet_out_of_the_carry_slots_unless_the_fix_is_on() {
        let fill = |c: &mut Core| {
            for i in 0..20u16 {
                c.fire_enemy_bullets(u16::from(b'Z'), i, 0);
            }
        };
        let mut c = core(FIRE);
        assert_eq!(c.enemy_bullet_slot_limit(), SAFE_BULLET_SLOTS);
        fill(&mut c);
        assert_eq!(c.eb.active_count(), SAFE_BULLET_SLOTS);
        assert!(c.eb.slots[SAFE_BULLET_SLOTS..].iter().all(|s| !s.is_active()), "槽 15 起格值低四位會進位");
        // 頭目新招式同理。
        let mut c = core(BOSS);
        fill(&mut c);
        assert_eq!(c.eb.active_count(), SAFE_BULLET_SLOTS);
        // 原版規則：整個池。
        let mut c = core(Rules::ORIGINAL);
        fill(&mut c);
        assert_eq!(c.eb.active_count(), BULLET_SLOTS);
        // 修正開著：格值另存槽號，整個池都安全。
        let mut c = core_with(FIRE, Fixes::ALL);
        assert_eq!(c.enemy_bullet_slot_limit(), BULLET_SLOTS);
        fill(&mut c);
        assert_eq!(c.eb.active_count(), BULLET_SLOTS);
    }

    #[test]
    fn fire_boost_gives_an_armed_enemy_the_original_roll_missed_another_chance() {
        // 門檻 16、亂數表全是 0、難度加成 0：原版永遠不開火（EASY 的普通敵機）。
        let (off, _) = shots_of(Rules::ORIGINAL, (0, 0), 16, 600);
        let (easy, rolls_easy) = shots_of(FIRE, (0, 0), 16, 600);
        let (hardest, rolls_hardest) = shots_of(FIRE, (1, 3), 16, 600);
        assert_eq!(off, 0);
        assert!((10..=45).contains(&easy), "EASY 每幀 4%，600 幀約 24 發，實際 {easy}");
        assert!(hardest > easy, "難度越高越常開火：{easy} 對 {hardest}");
        assert_eq!(rolls_easy, 600 % 75, "亂數表索引和原版一樣每幀走一步，開不開選項都一樣");
        assert_eq!(rolls_hardest, 600 % 75);
        assert_eq!(shots_of(FIRE, (0, 0), 16, 600), shots_of(FIRE, (0, 0), 16, 600), "同樣的輸入與選項，結果相同");
        // 原版已經開火的那一幀不再加碼：亂數表全是 15、門檻 4 的敵人每幀原版就開火，選項開不開都是每幀一槍。
        let always = |rules| {
            let mut c = core(rules);
            fill_rolls(&mut c, 15);
            c.set_players([Some((20, 12)), None]);
            grunt(&mut c, 0, 20, 5, 4, &[0; 8]);
            c.update_enemies();
            spawned(&c)
        };
        assert_eq!((always(Rules::ORIGINAL), always(FIRE)), (1, 1));
    }

    /// 擺一個門檻 `threshold` 的普通敵機（亂數表全是 0，所以原版只有加成夠大的門檻才會開火），跑 `frames` 幀，
    /// 每幀把敵彈清掉（免得場上的彈數影響上限）。回傳 (開了幾槍, 敵人的亂數表索引)。
    fn shots_of(rules: Rules, level: (u16, u16), threshold: u16, frames: usize) -> (u32, u16) {
        let mut c = core(rules);
        c.set_difficulty(level.0, level.1);
        fill_rolls(&mut c, 0);
        c.set_players([Some((20, 12)), None]);
        grunt(&mut c, 0, 25, 5, threshold, &[0; 8]);
        for _ in 0..frames {
            // 腳本永遠停在碼 0，不走到結束碼。
            c.en.slots[0].enemy.script = 0x7800;
            c.update_enemies();
            c.eb.release_all();
        }
        (spawned(&c), c.en.slots[0].enemy.rand_index)
    }

    #[test]
    fn fire_boost_makes_silent_aimed_shooters_fire_now_and_then_by_its_own_dice() {
        assert_eq!(shots_of(Rules::ORIGINAL, (0, 0), NEVER_FIRES, 600).0, 0, "原版從不開火");
        let (easy, rolls) = shots_of(FIRE, (0, 0), NEVER_FIRES, 600);
        let (hardest, _) = shots_of(FIRE, (1, 3), NEVER_FIRES, 600);
        assert!((5..=45).contains(&easy), "EASY 每幀 3%，600 幀約 18 發，實際 {easy}");
        assert!(hardest > easy, "難度越高越常開火：{easy} 對 {hardest}");
        assert_eq!(rolls, 600 % 75, "原版的亂數表索引照樣每幀走一步");
        assert_eq!(shots_of(FIRE, (0, 0), NEVER_FIRES, 600), shots_of(FIRE, (0, 0), NEVER_FIRES, 600), "同樣的輸入與選項，結果相同");
    }

    #[test]
    fn fire_boost_skips_enemies_it_should_not_touch() {
        let mut c = core(FIRE);
        fill_rolls(&mut c, 15);
        c.set_players([Some((20, 12)), None]);
        for (slot, edit) in [
            (0usize, (|e: &mut Enemy| e.carrier = u16::from(b'P')) as fn(&mut Enemy)),
            (1, |e: &mut Enemy| e.kind = u16::from(b'N')),
            (2, |e: &mut Enemy| e.mode = u16::from(b'Y')),
            (3, |e: &mut Enemy| e.hp = 999),
            (4, |e: &mut Enemy| e.col = 38),
            (5, |e: &mut Enemy| e.col = 40),
        ] {
            grunt(&mut c, slot, 20, 5, 16, &[0; 8]);
            edit(&mut c.en.slots[slot].enemy);
        }
        // 角色不是普通敵人。
        grunt(&mut c, 6, 20, 5, 16, &[0; 8]);
        c.ai.enemies[6].role = Role::BossPart;
        grunt(&mut c, 7, 20, 5, 16, &[0; 8]);
        c.ai.enemies[7].role = Role::Inert;
        c.update_enemies();
        assert_eq!(spawned(&c), 0);
    }

    #[test]
    fn fire_boost_adds_nothing_while_the_screen_already_has_ten_bullets() {
        let mut c = core(FIRE);
        c.set_players([Some((20, 12)), None]);
        fill_rolls(&mut c, 15);
        for i in 0..LIVE_BULLET_CAP {
            c.eb.slots[i].link = record_link(i);
            c.eb.slots[i].bullet = Bullet { script: 0x7000, ..Bullet::default() };
        }
        grunt(&mut c, 0, 20, 5, 16, &[0; 8]);
        let before = spawned(&c);
        c.update_enemies();
        assert_eq!(spawned(&c), before);
    }

    // ---- ai_react ----

    const W: u8 = 7;

    #[test]
    fn a_grunt_sidesteps_a_player_bullet_coming_at_its_row_and_keeps_going_west() {
        // 子彈在列 8、敵人在列 8 至 9：子彈在中心以上，往下躲，西加南是碼 6。
        for (rules, want_row) in [(Rules::ORIGINAL, 8u16), (REACT, 9)] {
            let mut c = core(rules);
            grunt(&mut c, 0, 20, 8, NEVER_FIRES, &[W, W, W]);
            player_shot(&mut c, 0, 15, 8);
            c.update_enemies();
            let e = c.en.slots[0].enemy;
            assert_eq!((e.col, e.row), (19, want_row), "ai_react = {}", rules.ai_react);
        }
        // 子彈在中心以下就往上躲，西加北是碼 8。
        let mut c = core(REACT);
        grunt(&mut c, 0, 20, 8, NEVER_FIRES, &[W]);
        player_shot(&mut c, 0, 15, 9);
        c.update_enemies();
        assert_eq!((c.en.slots[0].enemy.col, c.en.slots[0].enemy.row), (19, 7));
    }

    #[test]
    fn dodging_needs_a_close_bullet_on_an_overlapping_row_and_spends_the_budget() {
        let still = |c: &Core| (c.en.slots[0].enemy.col, c.en.slots[0].enemy.row);
        for (col, row) in [(10u16, 8u16), (15, 5), (15, 11), (30, 8)] {
            let mut c = core(REACT);
            grunt(&mut c, 0, 20, 8, NEVER_FIRES, &[W]);
            player_shot(&mut c, 0, col, row);
            c.update_enemies();
            assert_eq!(still(&c), (19, 8), "子彈 ({col}, {row}) 太遠、在後面或列不重疊");
        }
        // 閃避有每個敵人一生的步數上限。
        let mut c = core(REACT);
        grunt(&mut c, 0, 20, 8, NEVER_FIRES, &[W; 12]);
        for _ in 0..DODGE_BUDGET + 2 {
            // 子彈永遠對著敵人目前的列。
            let (col, row) = (c.en.slots[0].enemy.col - 3, c.en.slots[0].enemy.row);
            player_shot(&mut c, 0, col, row);
            c.update_enemies();
        }
        assert_eq!(c.ai.enemies[0].dodges_left, 0);
        let (col, row) = (c.en.slots[0].enemy.col - 3, c.en.slots[0].enemy.row);
        player_shot(&mut c, 0, col, row);
        let before = c.en.slots[0].enemy;
        c.update_enemies();
        assert_eq!(c.en.slots[0].enemy.row, before.row, "步數用完就照腳本走");
    }

    #[test]
    fn dodging_never_leaves_the_field_and_picks_the_other_way_at_an_edge() {
        // 最上面一列：子彈在中心以下本來要往上躲，上面沒有空間，改往下。
        let mut c = core(REACT);
        grunt(&mut c, 0, 20, 0, NEVER_FIRES, &[W]);
        player_shot(&mut c, 0, 15, 1);
        c.update_enemies();
        assert_eq!(c.en.slots[0].enemy.row, 1);
        // 最下面能到的一列（底緣在 17）：往下沒有空間。
        let mut c = core(REACT);
        grunt(&mut c, 0, 20, 16, NEVER_FIRES, &[W]);
        player_shot(&mut c, 0, 15, 16);
        c.update_enemies();
        assert_eq!(c.en.slots[0].enemy.row, 15);
        // 長時間來回：列始終在畫面內。
        let mut c = core(REACT);
        grunt(&mut c, 0, 30, 8, NEVER_FIRES, &[0; 60]);
        for k in 0..60u16 {
            let col = c.en.slots[0].enemy.col.saturating_sub(2);
            player_shot(&mut c, 0, col, (k % 3) * 8);
            c.ai.enemies[0].dodges_left = DODGE_BUDGET;
            c.update_enemies();
            let row = c.en.slots[0].enemy.row as i16;
            assert!((0..=FIELD_ROW_BOTTOM - 1).contains(&row), "第 {k} 幀列 {row}");
        }
    }

    #[test]
    fn a_waiting_grunt_lines_up_with_a_nearby_player_a_few_steps() {
        // 腳本停著（碼 0）：玩家在水平 12 欄內、列差 2 以上，朝玩家的列走。
        let mut c = core(REACT);
        c.set_players([Some((14, 14)), None]);
        grunt(&mut c, 0, 20, 5, NEVER_FIRES, &[0; 12]);
        c.update_enemies();
        assert_eq!((c.en.slots[0].enemy.col, c.en.slots[0].enemy.row), (20, 6));
        for _ in 0..20 {
            c.update_enemies();
        }
        assert_eq!(c.en.slots[0].enemy.row, 5 + u16::from(ALIGN_BUDGET), "對齊有步數上限");
        // 太遠、列差太小、原版：不動。
        for (rules, player) in [(REACT, (4u16, 14u16)), (REACT, (14, 5)), (Rules::ORIGINAL, (14, 14))] {
            let mut c = core(rules);
            c.set_players([Some(player), None]);
            grunt(&mut c, 0, 20, 5, NEVER_FIRES, &[0; 12]);
            c.update_enemies();
            assert_eq!(c.en.slots[0].enemy.row, 5);
        }
        // 腳本在走（不是停著）：不插手。
        let mut c = core(REACT);
        c.set_players([Some((14, 14)), None]);
        grunt(&mut c, 0, 20, 5, NEVER_FIRES, &[W]);
        c.update_enemies();
        assert_eq!((c.en.slots[0].enemy.col, c.en.slots[0].enemy.row), (19, 5));
    }

    #[test]
    fn react_leaves_bosses_carriers_scenery_and_special_codes_alone() {
        let mut c = core(REACT);
        c.set_players([Some((14, 14)), None]);
        let cases: [(&str, fn(&mut Core, usize)); 7] = [
            ("頭目分件", |c, s| c.ai.enemies[s].role = Role::BossPart),
            ("頭目核心", |c, s| c.ai.enemies[s].role = Role::BossCore(Boss::Stage1)),
            ("沒有角色", |c, s| c.ai.enemies[s].role = Role::Inert),
            ("載體", |c, s| c.en.slots[s].enemy.carrier = u16::from(b'P')),
            ("隨景物", |c, s| c.en.slots[s].enemy.mode = u16::from(b'Y')),
            ("不可摧毀", |c, s| c.en.slots[s].enemy.hp = 999),
            ("不畫的", |c, s| c.en.slots[s].enemy.kind = u16::from(b'N')),
        ];
        for (name, edit) in cases {
            grunt(&mut c, 0, 20, 5, NEVER_FIRES, &[0; 4]);
            edit(&mut c, 0);
            player_shot(&mut c, 0, 17, 5);
            c.update_enemies();
            assert_eq!(c.en.slots[0].enemy.row, 5, "{name}");
        }
        // 腳本的特殊碼（0B 發射、0D 找列、0A 右跳）照原樣執行。
        for code in [0x0Bu8, 0x0D, 0x0A] {
            let mut on = core(REACT);
            let mut off = core(Rules::ORIGINAL);
            for c in [&mut on, &mut off] {
                c.set_players([Some((14, 14)), None]);
                grunt(c, 0, 20, 5, NEVER_FIRES, &[code]);
            }
            player_shot(&mut on, 0, 17, 5);
            on.update_enemies();
            off.update_enemies();
            assert_eq!(on.en.slots[0].enemy, off.en.slots[0].enemy, "碼 {code:02X}");
        }
    }

    // ---- ai_boss ----

    /// 擺第 `boss` 關的頭目核心（形狀 4×3），位置 (12, 6)，發射點 (14, 7)。
    fn boss_core(c: &mut Core, boss: Boss, clock: u16) {
        grunt(c, 0, 12, 6, NEVER_FIRES, &[0; 200]);
        put_shape(c, 0x7000, 4, 3, 1);
        c.ai.enemies[0].role = Role::BossCore(boss);
        c.ai.enemies[0].clock = clock;
        c.en.slots[0].enemy.bullet = u16::from(b'E');
    }

    #[test]
    fn the_boss_core_fires_its_moves_on_its_own_clock_and_not_before() {
        // 第一關：生成後第 24 幀起每 30 幀一次三連瞄準彈；第 48 幀起每 72 幀一次環形彈。
        let mut c = core(BOSS);
        c.set_players([Some((30, 14)), None]);
        boss_core(&mut c, Boss::Stage1, 22);
        c.update_enemies();
        assert_eq!(c.eb.active_count(), 0, "時鐘 23");
        c.update_enemies();
        assert_eq!(c.eb.active_count(), 1, "時鐘 24：三連發的第一發");
        for _ in 0..BURST_GAP - 1 {
            c.update_enemies();
        }
        assert_eq!(c.eb.active_count(), 1, "還沒到 3 幀");
        c.update_enemies();
        assert_eq!(c.eb.active_count(), 2, "3 幀之後第二發");
        for _ in 0..BURST_GAP {
            c.update_enemies();
        }
        assert_eq!(c.eb.active_count(), 3);
        for _ in 0..BURST_GAP + 5 {
            c.update_enemies();
        }
        assert_eq!(c.eb.active_count(), 3, "只有三發");
        // 環形彈：時鐘 48 一次噴出八顆，從核心中心 (14, 7) 起，列 +2。
        let mut c = core(BOSS);
        c.set_players([Some((30, 14)), None]);
        boss_core(&mut c, Boss::Stage1, 47);
        c.update_enemies();
        assert_eq!(c.eb.active_count(), 8);
        assert!(c.eb.slots.iter().filter(|s| s.is_active()).all(|s| (s.bullet.col, s.bullet.row) == (14, 9)));
    }

    #[test]
    fn the_boss_does_nothing_with_the_option_off_or_off_screen_or_up_close_or_after_the_clear() {
        let fired = |rules: Rules, edit: &dyn Fn(&mut Core)| {
            let mut c = core(rules);
            c.set_players([Some((30, 14)), None]);
            boss_core(&mut c, Boss::Stage1, 47);
            edit(&mut c);
            c.update_enemies();
            c.eb.active_count()
        };
        assert_eq!(fired(BOSS, &|_| {}), 8);
        assert_eq!(fired(Rules::ORIGINAL, &|_| {}), 0, "原版");
        assert_eq!(fired(Rules { ai_boss: false, ..ALL }, &|_| {}), 0, "別的選項不會讓頭目出招");
        assert_eq!(fired(BOSS, &|c| c.en.slots[0].enemy.row = 25), 0, "核心還在畫面下方");
        assert_eq!(fired(BOSS, &|c| c.en.slots[0].enemy.col = 60), 0, "核心在畫面右外");
        assert_eq!(fired(BOSS, &|c| c.set_player_cell(0, 15, 8)), 0, "發射點貼著玩家");
        assert_eq!(fired(BOSS, &|c| c.mark_stage_cleared()), 0, "過關之後");
        assert_eq!(fired(BOSS, &|c| c.ai.enemies[0].role = Role::BossPart), 0, "分件不出招");
    }

    #[test]
    fn boss_moves_speed_up_with_difficulty_but_not_below_the_floor() {
        let at = |level: (u16, u16), clock: u16| {
            let mut c = core(BOSS);
            c.set_difficulty(level.0, level.1);
            c.set_players([Some((30, 14)), None]);
            boss_core(&mut c, Boss::Stage1, clock - 1);
            c.update_enemies();
            c.eb.active_count()
        };
        // 三連發第一發的時鐘：24、24 + 30、24 + 26、24 + 24 + 22 → 下限 24。
        assert_eq!(at((0, 0), 54), 1);
        assert_eq!(at((0, 0), 48 + 0), 8, "EASY 環形彈 48");
        assert_eq!(at((0, 2), 50), 1, "NORMAL 週期 26");
        assert_eq!(at((0, 0), 50), 0);
        assert_eq!(at((1, 3), 48), 8 + 1, "HARDEST 週期降到下限 24，48 同時是三連發與環形彈");
        assert_eq!(at((1, 3), 54), 0);
        assert!(STAGE1_ATTACKS.iter().chain(STAGE2_ATTACKS).chain(STAGE3_ATTACKS).chain(STAGE4_ATTACKS).all(|a| a.period > BOSS_PERIOD_MIN));
    }

    #[test]
    fn a_full_pool_makes_room_for_the_new_move_bouncers_first() {
        let mut c = core(BOSS);
        c.set_players([Some((30, 14)), None]);
        for i in 0..SAFE_BULLET_SLOTS {
            c.eb.slots[i].link = record_link(i);
            // 偶數槽是反彈球，奇數槽是一般彈。
            c.eb.slots[i].bullet = Bullet { heading: if i % 2 == 0 { 4 } else { 0 }, ..Bullet::default() };
        }
        boss_core(&mut c, Boss::Stage1, 47);
        c.update_enemies();
        // 環形彈 8 顆：先放 8 顆反彈球（偶數槽 0 至 14 共 8 個），一般彈都留著，新彈進了空出的槽。
        let ring = c.eb.slots.iter().take(SAFE_BULLET_SLOTS).filter(|s| s.is_active() && (s.bullet.col, s.bullet.row) == (14, 9)).count();
        assert_eq!(ring, 8);
        assert_eq!(c.eb.slots.iter().take(SAFE_BULLET_SLOTS).filter(|s| s.is_active()).count(), 15);
        let odd_kept = (0..SAFE_BULLET_SLOTS).filter(|i| i % 2 == 1).all(|i| c.eb.slots[i].is_active() && c.eb.slots[i].bullet.heading == 0);
        assert!(odd_kept, "一般彈沒被放掉");
        // 原版池的槽號 15 起不在騰出的範圍：沒有修正時不會因此多放。
        assert!(c.eb.slots[SAFE_BULLET_SLOTS..].iter().all(|s| !s.is_active()));
    }

    #[test]
    fn every_boss_has_one_or_two_moves_with_gaps_in_each_pattern() {
        for boss in [Boss::Stage1, Boss::Stage2, Boss::Stage3, Boss::Stage4] {
            let moves = boss_attacks(boss);
            assert!((1..=2).contains(&moves.len()), "{boss:?}");
            for m in moves {
                let count = match m.action {
                    Action::Burst(n) => {
                        assert!((2..=4).contains(&n), "連發 {n} 發，每發相隔 {BURST_GAP} 幀");
                        1
                    }
                    Action::Volley(letter) => FirePattern::from_letter(u16::from(letter)).bullet_count(),
                };
                assert!((1..=8).contains(&count), "{boss:?} 一招最多 8 顆，實際 {count}");
                assert!(m.first >= 20, "進場後才出招");
            }
        }
        // 環形彈是八個方向各一顆，相鄰夾角 45 度。
        let ring = FirePattern::from_letter(u16::from(b'<'));
        assert_eq!(ring.bullet_count(), 8);
    }

    #[test]
    fn roles_follow_the_template_addresses() {
        use crate::tables::enemy_role;
        assert_eq!(enemy_role(0x42FE), Role::Grunt);
        assert_eq!(enemy_role(0x47AA), Role::BossCore(Boss::Stage1));
        assert_eq!(enemy_role(0x5BFC), Role::BossCore(Boss::Stage2));
        assert_eq!(enemy_role(0x724E), Role::BossCore(Boss::Stage3));
        assert_eq!(enemy_role(0x8441), Role::BossCore(Boss::Stage4));
        for part in [0x43E4u16, 0x45DE, 0x46C4, 0x59D4, 0x5BCE, 0x5A30, 0x5B72, 0x7168, 0x7220, 0x83E5] {
            assert_eq!(enemy_role(part), Role::BossPart, "{part:04X}");
        }
        for ending in [0x8D2Fu16, 0x8D5D, 0x8E15] {
            assert_eq!(enemy_role(ending), Role::Inert, "片尾 {ending:04X}");
        }
        // 生成時分類，槽重用時重設。
        let mut ai = AiState::default();
        ai.on_spawn(3, 0x47AA);
        ai.enemies[3].clock = 99;
        ai.on_spawn(3, 0x42FE);
        assert_eq!(ai.enemies[3], AiEnemy { role: Role::Grunt, dodges_left: DODGE_BUDGET, aligns_left: ALIGN_BUDGET, ..AiEnemy::default() });
        assert_eq!(AiEnemy::default().role, Role::Inert, "沒生成過的槽（快照載入）不碰");
    }

    #[test]
    fn difficulty_levels_follow_the_two_original_bonus_words() {
        let mut c = core(FIRE);
        for (hp, fire, level) in [(0u16, 0u16, 0usize), (0, 2, 1), (1, 2, 2), (1, 3, 3)] {
            c.set_difficulty(hp, fire);
            assert_eq!(c.ai_level(), level, "({hp}, {fire})");
        }
    }

    // ---- 選項之間互不干擾、決定性 ----

    /// 一個擺滿各種角色的場景，跑 `frames` 幀，回傳資料段全部位元組與 AI 以外的狀態雜湊。
    fn scene(rules: Rules, frames: usize, roles: bool) -> crate::game::State {
        let mut c = core(rules);
        install_shot(&mut c, Heading::E, 3);
        install_shot(&mut c, Heading::S, 5);
        install_shot(&mut c, Heading::W, 7);
        install_wide(&mut c);
        fill_rolls(&mut c, 14);
        c.set_difficulty(1, 2);
        c.set_players([Some((6, 8)), Some((9, 12))]);
        grunt(&mut c, 0, 18, 4, 16, &[0, 0, 0, 0, 7, 0, 0, 0, 7, 0, 0, 0, 7, 7, 7, 7]);
        grunt(&mut c, 1, 24, 10, NEVER_FIRES, &[7, 0, 0, 0, 7, 7, 7, 7, 7, 7, 7, 7]);
        grunt(&mut c, 2, 16, 12, 14, &[7; 14]);
        grunt(&mut c, 3, 12, 6, NEVER_FIRES, &[0; 200]);
        if roles {
            c.ai.enemies[3].role = Role::BossCore(Boss::Stage3);
            c.ai.enemies[3].clock = 38;
        } else {
            c.ai.enemies[3].role = Role::Inert;
        }
        for k in 0..frames {
            if k % 5 == 0 {
                player_shot(&mut c, k / 5 % 20, 12, 3 + (k as u16 / 5) % 11);
            }
            if k % 9 == 4 {
                c.set_player_cell(0, 6, 5 + (k as u16 / 9) % 10);
            }
            c.update_enemies();
            patch_blank_bullets(&mut c);
            c.update_enemy_bullets();
        }
        c.snapshot()
    }

    /// 沒擺樣板的圖案會生出全 0 的記錄；形狀寬高 0 在原版是 65536 次迴圈（`loop_count`），所以測試場景給它們
    /// 一個 1×1 的形狀與向西的腳本。
    fn patch_blank_bullets(c: &mut Core) {
        for s in c.eb.slots.iter_mut().filter(|s| s.is_active() && s.bullet.shape == 0) {
            s.bullet = Bullet { frames: 0x7200, blit_dx: 1, blit_dy: 10, dcol: 1, drow: 1, script: 0x7300 + 0x10 * Heading::W.index(), shape: 0x7100, ..s.bullet };
        }
    }

    #[test]
    fn with_every_option_off_roles_and_ai_state_do_not_matter() {
        assert!(scene(Rules::ORIGINAL, 120, true) == scene(Rules::ORIGINAL, 120, false), "選項全關時角色分類不影響模擬");
    }

    #[test]
    fn same_input_and_options_give_the_same_result_and_each_option_changes_it() {
        let off = scene(Rules::ORIGINAL, 150, true);
        for (name, rules) in [("ai_aim", AIM), ("ai_fire", FIRE), ("ai_react", REACT), ("ai_boss", BOSS), ("全開", ALL)] {
            let a = scene(rules, 150, true);
            let b = scene(rules, 150, true);
            assert!(a == b, "{name}：兩次結果相同");
            assert!(a != off, "{name}：開啟後行為確實改變");
            assert!(scene(Rules::ORIGINAL, 150, true) == off, "{name}：關閉後回到原狀");
        }
    }

    #[test]
    fn options_do_not_leak_into_each_other() {
        // 只開頭目新招式：瞄準彈仍然走原版腳本，沒有任何路徑。
        let mut c = core(BOSS);
        install_shot(&mut c, Heading::E, 3);
        c.set_players([Some((20, 10)), None]);
        c.fire_enemy_bullets(u16::from(b'F'), 5, 7);
        assert!(c.ai.paths.iter().all(Option::is_none));
        // 只開瞄準更準：頭目核心不出招，普通敵機不多開火，也不閃避。
        let mut c = core(AIM);
        install_shot(&mut c, Heading::S, 5);
        fill_rolls(&mut c, 15);
        c.set_players([Some((20, 12)), None]);
        grunt(&mut c, 0, 20, 5, 16, &[0; 8]);
        grunt(&mut c, 1, 30, 8, NEVER_FIRES, &[W]);
        boss_core_at(&mut c, 2);
        player_shot(&mut c, 0, 27, 8);
        c.update_enemies();
        assert_eq!(spawned(&c), 0);
        assert_eq!(c.en.slots[1].enemy.row, 8);
    }

    fn boss_core_at(c: &mut Core, slot: usize) {
        grunt(c, slot, 12, 6, NEVER_FIRES, &[0; 200]);
        c.ai.enemies[slot].role = Role::BossCore(Boss::Stage1);
        c.ai.enemies[slot].clock = 47;
    }
}

/// 用原版檔跑整關的測試。需要 `RS2_ORIG`，標 `#[ignore]`：`tools/rust.sh test -p rs2-game ai::real -- --ignored --nocapture`。
#[cfg(test)]
mod real {
    use crate::ebullet::EnemyBullets;
    use crate::entity::Enemies;
    use crate::game::{Entry, Fixes, Game, GameFiles, Input, Rules, Start};

    fn files() -> GameFiles {
        let dir = std::path::PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh 跑"));
        let image = rs2_formats::unpack_s_exe(&std::fs::read(dir.join("S.EXE")).unwrap()).unwrap();
        let scn = std::fs::read(dir.join("SHT.SCN")).unwrap();
        GameFiles { image, scn, ..GameFiles::default() }
    }

    /// 以第 `stage` 關（1 至 4）的入口開一局。第二關要 `Fixes::no_hit_hang`（玩家血量被補滿，從沒被打中，原版會卡住）。
    fn game(stage: u8, rules: Rules, difficulty: u8, fixes: Fixes) -> Game {
        let files = files();
        let start = Start::NewGame { difficulty: Some(difficulty) };
        let mut g = if stage == 1 {
            Game::new(&files.image, &[], start, fixes)
        } else {
            let entry = [Entry::Stage1, Entry::Stage2, Entry::Stage3, Entry::Stage4][stage as usize - 1];
            // 第一關初始化後的資料段（玩家 1 存活、難度已寫入）當快照，再補第二關入口檢查的密碼旗標（byte_1D3A2 = Y）。
            let boot = Game::new(&files.image, &[], start, fixes);
            let mut state = boot.state().clone();
            state.flow.password_accepted = b'Y';
            Game::from_state(files, state, entry, fixes)
        };
        g.set_rules(rules);
        g
    }

    /// 第 `stage` 關要用的修正：第二關需要 `no_hit_hang`，其他關維持原版（驗證 `bullet_slot_carry` 關閉時的行為）。
    fn fixes_for(stage: u8) -> Fixes {
        Fixes { no_hit_hang: stage == 2, ..Fixes::NONE }
    }

    /// 玩家 1 的輸入腳本：上下來回、不停開火（右 Shift 是玩家 1 的開火鍵）。
    fn keys(step: u32) -> Vec<u8> {
        let mut k = Vec::new();
        match step % 48 {
            0 => k.push(0x48),
            12 => k.push(0xC8),
            24 => k.push(0x50),
            36 => k.push(0xD0),
            _ => {}
        }
        match step % 6 {
            0 => k.push(0x36),
            3 => k.push(0xB6),
            _ => {}
        }
        k
    }

    /// FNV-1a 64 位元。
    fn fnv(h: &mut u64, bytes: &[u8]) {
        for &b in bytes {
            *h = (*h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }

    /// 一局的結果：每一步的資料段與繪製命令雜湊，加上跑了幾步、怎麼離開。
    struct Outcome {
        hashes: Vec<u64>,
        exit: Option<crate::game::Exit>,
        /// 每一步場上敵彈數的總和，量「彈幕有多密」。
        bullets: u64,
        peak: usize,
    }

    /// 玩家怎麼輸入。
    #[derive(Clone, Copy)]
    enum Control {
        /// 不輸入。
        Idle,
        /// 上下來回、不停開火（[`keys`]）。
        Roam,
        /// 追著頭目核心的列、不停開火（[`Chaser`]）。
        Chase,
    }

    /// 追著頭目打的玩家：對準頭目核心（子彈字母 `E` 或 `Q`）的列，不停開火。只在按鍵狀態改變時送碼。
    #[derive(Default)]
    struct Chaser {
        up: bool,
        down: bool,
        fire: bool,
    }

    impl Chaser {
        fn keys(&mut self, g: &Game) -> Vec<u8> {
            let (en, _) = pools(g);
            let me = g.state().pl.ship[0];
            let target = en
                .slots
                .iter()
                .filter(|s| s.is_active())
                .map(|s| &s.enemy)
                .find(|e| matches!(e.bullet as u8, b'E' | b'Q') && (0..=17).contains(&(e.row as i16)));
            let want = target.map(|e| e.row as i16 + 1);
            let row = me.row as i16;
            let (up, down) = (want.is_some_and(|w| row > w + 1), want.is_some_and(|w| row < w - 1));
            let mut k = Vec::new();
            if up != self.up {
                k.push(if up { 0x48 } else { 0xC8 });
            }
            if down != self.down {
                k.push(if down { 0x50 } else { 0xD0 });
            }
            // 開火鍵每步按下、放開交替。
            k.push(if self.fire { 0xB6 } else { 0x36 });
            (self.up, self.down, self.fire) = (up, down, !self.fire);
            k
        }
    }

    /// 跑一關到離開或 `max` 步。玩家血量補滿，照 `control` 輸入。每一步之後檢查池沒有溢位、敵彈只用 `limit` 以下的槽。
    fn play(g: &mut Game, max: u32, control: Control, limit: usize) -> Outcome {
        let mut out = Outcome { hashes: Vec::new(), exit: None, bullets: 0, peak: 0 };
        let mut chaser = Chaser::default();
        for step in 0..max {
            g.state_mut().pl.ship[0].hp = 0xFF;
            let k = match control {
                Control::Idle => Vec::new(),
                Control::Roam => keys(step),
                Control::Chase => chaser.keys(g),
            };
            let f = g.step(&Input { scancodes: &k }, &mut |_: &Game, _| {});
            let mut h = 0xCBF2_9CE4_8422_2325u64;
            fnv(&mut h, format!("{:?}", g.state()).as_bytes());
            fnv(&mut h, format!("{:?}", f.draws).as_bytes());
            out.hashes.push(h);
            let (en, eb) = pools(g);
            let live = eb.active_count();
            out.bullets += live as u64;
            out.peak = out.peak.max(live);
            // 池沒有溢位；敵彈只用 `limit` 以下的槽；活著的敵彈在界內（出界的那一幀就釋放了）。
            assert!(eb.slots[limit..].iter().all(|s| !s.is_active()), "第 {step} 步：槽 {limit} 起有敵彈");
            // 敵彈與敵人的座標不查：原版就有畫面外還活著的敵彈（畫面下方生成、只橫向移動的彈不檢查列，只直走的不檢查欄），
            // 頭目的分件也會跑到畫面外很遠（選項全關也到欄 -301）。AI 對座標的限制在 `ai::tests`。
            let _ = en;
            if f.exit.is_some() {
                out.exit = f.exit;
                break;
            }
        }
        out
    }

    fn pools(g: &Game) -> (Enemies, EnemyBullets) {
        (g.state().en.clone(), g.state().eb.clone())
    }

    const AIM: Rules = Rules { ai_aim: true, ..Rules::ORIGINAL };
    const FIRE: Rules = Rules { ai_fire: true, ..Rules::ORIGINAL };
    const REACT: Rules = Rules { ai_react: true, ..Rules::ORIGINAL };
    const BOSS: Rules = Rules { ai_boss: true, ..Rules::ORIGINAL };
    const ALL: Rules = Rules { ai_aim: true, ai_fire: true, ai_react: true, ai_boss: true, ..Rules::ORIGINAL };
    const OPTIONS: [(&str, Rules); 5] = [("ai_aim", AIM), ("ai_fire", FIRE), ("ai_react", REACT), ("ai_boss", BOSS), ("全開", ALL)];

    /// 一關跑完的最多步數（頭目戰的幀計數會釘住，所以用步數）。
    const MAX_STEPS: u32 = 4000;

    /// 選項全關跑一次和沒設過 `set_rules` 的預設跑一次，結果逐步相同（E1 的前提：關著的選項不改變模擬）。
    /// 逐位元組對原版收據的比對在 `rs2-trace`（`tools/dos/wait_test.sh`）。
    #[test]
    #[ignore]
    fn rules_original_equals_never_calling_set_rules() {
        for stage in 1..=4u8 {
            let mut a = game(stage, Rules::ORIGINAL, 1, fixes_for(stage));
            let mut b = game(stage, Rules::default(), 1, fixes_for(stage));
            let limit = 20;
            let (oa, ob) = (play(&mut a, 1500, Control::Roam, limit), play(&mut b, 1500, Control::Roam, limit));
            assert!(oa.hashes == ob.hashes, "第 {stage} 關");
        }
    }

    /// 四個選項各自與全開，四關、四種難度，跑到離開或 4000 步：不 panic、池不溢位、位置在範圍內。
    /// 沒有修正 `bullet_slot_carry` 時（第一、三、四關）敵彈不得用到槽 15 起，才不會引進格值進位的新錯誤。
    #[test]
    #[ignore]
    fn every_option_runs_every_stage_at_every_difficulty() {
        for stage in 1..=4u8 {
            for difficulty in 0..4u8 {
                for (name, rules) in OPTIONS {
                    let mut g = game(stage, rules, difficulty, fixes_for(stage));
                    let limit = if rules.ai_fire || rules.ai_boss { 15 } else { 20 };
                    let out = play(&mut g, MAX_STEPS, Control::Roam, limit);
                    println!("第 {stage} 關 難度 {difficulty} {name}：{} 步，離開 {:?}，彈數合計 {}，尖峰 {}", out.hashes.len(), out.exit, out.bullets, out.peak);
                    assert!(out.hashes.len() > 200, "第 {stage} 關 {name} 只跑了 {} 步", out.hashes.len());
                }
            }
        }
    }

    /// 打得贏：玩家追著頭目核心的列、不停開火，四個選項全開，四關的頭目戰照樣可以打到離開關卡。
    /// 選項不改頭目的耐久與移動，所以和選項全關比，只看離開的步數。
    #[test]
    #[ignore]
    fn a_player_who_chases_the_boss_still_wins_with_every_option_on() {
        for stage in 1..=4u8 {
            let off = play(&mut game(stage, Rules::ORIGINAL, 1, fixes_for(stage)), MAX_STEPS, Control::Chase, 20);
            let on = play(&mut game(stage, ALL, 1, fixes_for(stage)), MAX_STEPS, Control::Chase, 15);
            println!("第 {stage} 關追擊：全關 {} 步離開 {:?}，全開 {} 步離開 {:?}", off.hashes.len(), off.exit, on.hashes.len(), on.exit);
            if off.exit.is_some() {
                assert!(on.exit.is_some(), "第 {stage} 關全關打得贏，全開要打得贏");
            }
        }
    }

    /// 修正全開（遊戲的預設）：敵彈格值另存槽號，整個池 20 槽都能用。四個選項全開，四關各跑完。
    #[test]
    #[ignore]
    fn all_options_with_every_fix_on_may_use_the_whole_pool() {
        let mut peaks = Vec::new();
        for stage in 1..=4u8 {
            let mut g = game(stage, ALL, 2, Fixes::ALL);
            let out = play(&mut g, MAX_STEPS, Control::Roam, 20);
            assert!(out.hashes.len() > 200, "第 {stage} 關只跑了 {} 步", out.hashes.len());
            peaks.push(out.peak);
        }
        println!("修正全開、選項全開，四關敵彈尖峰 {peaks:?}");
        assert!(peaks.iter().any(|&p| p > 15), "至少有一關用到槽 15 起：{peaks:?}");
    }

    /// 第一關從頭跑到頭目，四個選項全開（規格 005 的驗收）。玩家不輸入，會被地形或敵人逼到邊緣，但血量補滿。
    #[test]
    #[ignore]
    fn all_options_on_stage_one_from_the_start_to_the_boss() {
        let mut g = game(1, ALL, 1, Fixes::NONE);
        let out = play(&mut g, MAX_STEPS, Control::Idle, 15);
        assert!(out.hashes.len() > 1000, "第一關只跑了 {} 步", out.hashes.len());
        assert_eq!(out.exit, Some(crate::game::Exit::Clear), "頭目的合成圖腳本走完過關");
    }

    /// 決定性（E2）與開關有效（E3）：同樣的輸入與選項兩次雜湊相同；每個選項開啟後的雜湊序列和選項全關不同。
    #[test]
    #[ignore]
    fn every_option_is_deterministic_and_changes_the_run() {
        for stage in 1..=4u8 {
            let off = play(&mut game(stage, Rules::ORIGINAL, 1, fixes_for(stage)), 2500, Control::Roam, 20);
            for (name, rules) in OPTIONS {
                let limit = if rules.ai_fire || rules.ai_boss { 15 } else { 20 };
                let a = play(&mut game(stage, rules, 1, fixes_for(stage)), 2500, Control::Roam, limit);
                let b = play(&mut game(stage, rules, 1, fixes_for(stage)), 2500, Control::Roam, limit);
                assert!(a.hashes == b.hashes, "第 {stage} 關 {name}：兩次結果不同");
                let differs = a.hashes != off.hashes;
                println!("第 {stage} 關 {name}：與全關{}，彈數合計 {} 對 {}", if differs { "不同" } else { "相同" }, a.bullets, off.bullets);
                if stage == 1 || name == "全開" || name == "ai_boss" {
                    assert!(differs, "第 {stage} 關 {name} 開啟後行為沒有改變");
                }
            }
        }
    }

    /// 頭目新招式在四關都看得到：頭目戰的彈數合計比原版多（第三、四關原版的彈幕已經塞滿池，新招式是換掉一部分，所以只要求不同）。
    #[test]
    #[ignore]
    fn boss_moves_show_up_in_every_boss_fight() {
        for stage in 1..=4u8 {
            let off = play(&mut game(stage, Rules::ORIGINAL, 1, fixes_for(stage)), MAX_STEPS, Control::Idle, 20);
            let on = play(&mut game(stage, BOSS, 1, fixes_for(stage)), MAX_STEPS, Control::Idle, 15);
            println!("第 {stage} 關頭目：彈數合計 原版 {} 對 新招式 {}，尖峰 {} 對 {}，離開 {:?} 對 {:?}", off.bullets, on.bullets, off.peak, on.peak, off.exit, on.exit);
            assert!(on.hashes != off.hashes, "第 {stage} 關");
            if stage <= 2 {
                assert!(on.bullets > off.bullets, "第 {stage} 關原版頭目核心不開火，新招式要讓彈變多");
            }
        }
    }
}
