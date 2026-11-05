//! 玩家：輸入、移動、開火、砲座、命中與繪製。兩位玩家共用同一份程式。
//!
//! 原版把玩家 1（`sub_14458` 一族，變數在 `1DA8Ah` 至 `1DACBh`）與玩家 2（`sub_14F92` 一族，變數在 `1DAECh`
//! 至 `1DB39h`）各抄一遍，機器碼逐項平行。這裡只有一份 [`Player`] 結構與一組函式，兩位的差異都在 [`Style`]
//! 這張表：鍵位、本體圖、砲座偏移、發射閃光、展示錄影的讀法。表裡每一項都對照過兩邊的機器碼。
//!
//! 狀態與資料段位移的對應在 `mirror.rs`，這個檔案不出現原版位移。
//!
//! | 入口 | 原版 |
//! |---|---|
//! | [`Core::update_player`] | `sub_14458`、`sub_14F92`：展示輸入、輸入與移動、錄製 |
//! | [`Core::draw_player`] | `sub_14704`、`sub_15232`：砲座、本體、撿取光柱、碰撞、發射閃光、尾焰 |
//! | [`Core::place_players_for_stage`] | `sub_14F47`、`sub_158BA`：每關開始的位置，第一關或展示模式另重設裝備 |
//! | [`Core::player_alive`]、[`Core::player_cell`] | 敵人與敵彈讀玩家位置 |
//!
//! 檔案分工：本檔是狀態、常數、輸入與移動；`player/contact.rs` 是碰撞、撿取與升級；`player/draw.rs` 是砲座與繪製。
//!
//! 原版函式對照（玩家 1 / 玩家 2 / 新名）：
//!
//! | 原版 | 新名 |
//! |---|---|
//! | `sub_14497` / `sub_14FC5` | [`Core::replay_demo_input`] |
//! | `sub_14501` / `sub_1502F` | [`Core::control_player`]，拆成 [`Core::steer`]、[`Core::turn_pods`]、[`Core::release_charge`]、[`Core::fire_main`] |
//! | `sub_14474` / `sub_14FAE`（空） | [`Core::record_demo_input`] |
//! | `sub_14679` / `sub_151A7` | [`Core::die_step`] |
//! | `sub_146AC`、`146C3`、`146DA`、`146F2` / `151DA`、`151F1`、`15208`、`15220` | [`Player::move_up`]、`move_down`、`move_right`、`move_left` |
//! | `sub_14F47` / `sub_158BA` | [`Core::place_players_for_stage`] |
//! | `sub_14704` / `sub_15232` | [`Core::draw_player`] |
//! | `sub_1480A` / `sub_15338` | [`Core::fly_pods`] |
//! | `sub_14908`、`1496D`、`149D2` / `15436`、`1549B`、`15500` | [`Core::advance_pods`] |
//! | `sub_147BC` / `sub_152EA` | [`Core::draw_pickup_beam`] |
//! | `sub_1478B` / `sub_152B9` | [`Core::draw_muzzle_flash`] |
//! | `sub_147E1` / `sub_1530F` | [`Core::collide_player`] |
//! | `sub_14A2C` / `sub_1555A` | [`Core::mark_body_cells`] |
//! | `sub_14A59` / `sub_15587` | [`Core::mark_flame_cells`]（走 [`Core::stamp_shape`]） |
//! | `sub_14A90` / `sub_155BE` | [`Core::body_touches_terrain`] |
//! | `sub_14B1E` / `sub_1564C` | [`Core::body_touches_entities`] |
//! | `sub_14B63`、`sub_14C18` / `sub_15691`、`sub_15746` | [`Core::pod_touches_entities`]（走 [`Core::damage_entity_at_cell`]） |
//! | `sub_14BAE` / `sub_156DC` | [`Core::pod_shields_bullets`] |
//! | `sub_14C85`、`14CD4`、`14D04` / `sub_157B3`、`15802`、`15832` | [`Core::body_touches_bullets`]、[`Core::touch_bullet_cell`]、[`Core::upgrade`] |
//! | `sub_14AD7`、`14AFC`、`14BF4`、`14C3D`、`14C61`、`14D68` 與玩家 2 的同型函式 | 算掃描位址，併入 [`Core::scan_footprint`] |
//!
//! 頭目段背景（原版 `sub_15906` 一族）原本和玩家擠在同一個檔案，不屬於這個領域，搬到 `boss_scroll.rs`。
//!
//! 強化模式（規格 005）的四個玩家選項（波動砲充能、按住連射、砲座像 Force、細步移動）在 `player/enhanced.rs`，
//! 狀態在 `Core::enh`，不進 `Mirror`。這個檔案的原版函式只在入口多一個 `Rules` 判斷，選項全關時不做任何事：
//! [`Core::control_player`] 的 [`Core::settle_options`]、[`Core::split_pods`]，[`Core::steer`] 的細步分支，
//! 開火的 [`Core::fire_enhanced`] 分支，[`Core::advance_pods`] 的分離分支。

mod contact;
mod draw;
mod enhanced;

#[cfg(test)]
pub(crate) use contact::Pickup;
pub use enhanced::{WAVE_CHARGE_FULL, WAVE_LEVEL_1, WAVE_LEVEL_2, WAVE_LEVEL_3};
pub(crate) use enhanced::{BOTTOM_PIXELS, Enhanced, ShootView};

use crate::fx::EffectKind;
use crate::game::Core;
use crate::mirror::FireFlag;
use crate::pbullet::Facing;

/// 玩家數。
pub(crate) const PLAYER_COUNT: usize = 2;

/// 旗標位元組：`Y` 成立，`N` 不成立。
const YES: u8 = b'Y';
const NO: u8 = b'N';

/// 砲座與主機的朝向字組：`>` 朝右，`<` 朝左（判斷只看是不是 `>`）。
const FACING_RIGHT: u16 = b'>' as u16;
const FACING_LEFT: u16 = b'<' as u16;

// ---- 移動 ----

/// 一步移動 1 列，本體的繪製位置跟著動 10 像素列。
pub(crate) const ROW_STEP: u16 = 1;
pub(crate) const ROW_PIXELS: u16 = 10;
/// 一步移動 2 欄，繪製位置的 x（單位是位元組）也動 2。
pub(crate) const COL_STEP: u16 = 2;
/// 上移要求列大於這個值（有號比較）。
const TOP_ROW: i16 = 0;
/// 下移要求列小於這個值（無號比較）。
const BOTTOM_ROW: u16 = 0x10;
/// 左移要求欄大於這個值（有號比較）。
const LEFT_COL: i16 = 1;
/// 右移要求欄小於這個值（無號比較）。
const RIGHT_COL: u16 = 0x21;
/// 每關開始的位置。
const START_COL: u16 = 5;
const START_ROW: u16 = 10;

// ---- 命中與死亡 ----

/// 受擊後的無敵幀數。
const HIT_INVINCIBILITY: u8 = 6;
/// 活著時的死亡倒數初值，血量歸零後每幀減 1。
const DYING_FRAMES: u8 = 0x0F;
/// 死亡動畫結束、清掉存活位元之後，倒數重設成的值。
const DYING_RESET: u8 = 6;
/// 血量初值：8 個位元全亮，每次受擊右移一位。
const FULL_HEALTH: u16 = 0xFF;

// ---- 撿取光柱 ----

/// 撿到道具時光柱的起點 y（FF42h，有號 -190）。
const BEAM_START: u16 = 0xFF42;
/// 光柱每幀往下的像素列數；y 大於 0（有號）時結束，所以撿到後約 5 幀。
const BEAM_STEP: u16 = 0x28;

// ---- 開火 ----

/// 放開開火鍵時，充能計數不小於這個值才射出大掃射。
pub(crate) const SWEEP_CHARGE: u16 = 0x0E;
/// 主機的普通彈（武器字母 0）。
const PLAIN_WEAPON: u16 = 0;

// ---- 展示錄影 ----

/// 非展示時，幀計數超過這個值就不再錄（原版 `cmp ..., 5DBh; ja`，緩衝區前 1500 個位元組）。
const LAST_RECORDED_FRAME: u16 = 0x5DB;

/// 展示錄影每個位元組的內容，也是 `input_bits` 的位元。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InputBit {
    Up = 0x01,
    Down = 0x02,
    Right = 0x04,
    Left = 0x08,
    Fire = 0x10,
    Turn = 0x20,
}

impl InputBit {
    fn is_set(self, bits: u8) -> bool {
        bits & self as u8 != 0
    }
}

/// 一位玩家的方向鍵與開火鍵，掃描碼（set 1）。上下左右各有三個鍵，斜向的鍵同時算兩個方向。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Keys {
    pub up_left: u8,
    pub up: u8,
    pub up_right: u8,
    pub left: u8,
    pub right: u8,
    pub down_left: u8,
    pub down: u8,
    pub down_right: u8,
    /// 開火鍵（Shift）。
    pub fire: u8,
}

impl Keys {
    fn upward(&self) -> [u8; 3] {
        [self.up_left, self.up, self.up_right]
    }

    fn downward(&self) -> [u8; 3] {
        [self.down_left, self.down, self.down_right]
    }

    fn leftward(&self) -> [u8; 3] {
        [self.down_left, self.left, self.up_left]
    }

    fn rightward(&self) -> [u8; 3] {
        [self.down_right, self.right, self.up_right]
    }
}

/// 本體的三張圖（`MY.GO` 圖庫索引）：平飛、上移、下移。
#[derive(Clone, Copy, Debug)]
pub(crate) struct BodyImages {
    pub level: u16,
    pub up: u16,
    pub down: u16,
}

/// 發射閃光：圖庫索引與相對本體的位移（x 單位是位元組，y 是像素列）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Muzzle {
    pub image: u16,
    pub dx: u16,
    pub dy: u16,
}

/// 砲座相對本體的偏移。第二座砲座在第一座下方 `gap_rows` 列。
#[derive(Clone, Copy, Debug)]
pub(crate) struct PodStyle {
    /// 砲座的列 = 本體的列減這個值。
    pub row_offset: u16,
    /// 砲座繪製位置的 y = 本體的 y 減這個值。
    pub y_offset: u16,
    /// 畫砲座時 y 再加這個值。
    pub draw_dy: u16,
    /// 第二座砲座在第一座下方幾列（繪製位置差 `gap_rows × ROW_PIXELS`）。
    pub gap_rows: u16,
}

/// 兩位玩家的差異。每個欄位都對照過玩家 1 與玩家 2 的機器碼。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Style {
    pub keys: Keys,
    pub images: BodyImages,
    pub muzzle: Muzzle,
    pub pod: PodStyle,
    /// 展示錄影的讀取位移：玩家 1 讀本幀的位元組，玩家 2 讀下一幀的（`0AAE3h`），而錄影只有玩家 1 的輸入。
    pub demo_offset: u16,
    /// 展示還原輸入時，轉向旗標的預設值：玩家 1 先清成 `N`，玩家 2 先設成 `Y`（`14FEFh` 起），
    /// 所以玩家 2 在展示中每幀都有轉向事件。玩家 2 在展示裡不在場，旗標就一直是 `Y`。
    pub demo_turn_default: bool,
    /// 非展示時有沒有錄本幀輸入：只有玩家 1 錄（玩家 2 的 `sub_14FAE` 是空的）。
    pub records_demo: bool,
    /// 每關開始的砲座朝向：玩家 1 朝左（`<`），玩家 2 朝右（`>`）。
    pub start_facing: u16,
    /// 死亡動畫結束時，存活位元組與這個遮罩相與。原版玩家 1 是 `F0h`、玩家 2 是 `0Fh`，
    /// 各自清掉自己那個半位元組（只用到位元 0 與位元 4）。
    pub survivors_mask: u8,
    /// 存活位元組裡這位玩家的位元。
    pub alive_bit: u8,
}

pub(crate) static STYLES: [Style; PLAYER_COUNT] = [
    Style {
        // 玩家 1：方向鍵與數字鍵盤。
        keys: Keys {
            up_left: 0x47,    // Home（數字鍵盤 7）
            up: 0x48,         // ↑
            up_right: 0x49,   // PgUp
            left: 0x4B,       // ←
            right: 0x4D,      // →
            down_left: 0x4F,  // End
            down: 0x50,       // ↓
            down_right: 0x51, // PgDn
            fire: 0x36,       // 右 Shift
        },
        images: BodyImages { level: 9, up: 0x0B, down: 7 },
        muzzle: Muzzle { image: 0x11, dx: 3, dy: 3 },
        pod: PodStyle { row_offset: 3, y_offset: 0x1E, draw_dy: 5, gap_rows: 5 },
        demo_offset: 0,
        demo_turn_default: false,
        records_demo: true,
        start_facing: FACING_LEFT,
        survivors_mask: 0xF0,
        alive_bit: 0x01,
    },
    Style {
        // 玩家 2：R T Y / F H / V B N。
        keys: Keys {
            up_left: 0x13,    // R
            up: 0x14,         // T
            up_right: 0x15,   // Y
            left: 0x21,       // F
            right: 0x23,      // H
            down_left: 0x2F,  // V
            down: 0x30,       // B
            down_right: 0x31, // N
            fire: 0x2A,       // 左 Shift
        },
        images: BodyImages { level: 0x10, up: 0x0E, down: 0x12 },
        muzzle: Muzzle { image: 0x0A, dx: 3, dy: 6 },
        pod: PodStyle { row_offset: 1, y_offset: 0x0A, draw_dy: 2, gap_rows: 2 },
        demo_offset: 1,
        demo_turn_default: true,
        records_demo: false,
        start_facing: FACING_RIGHT,
        survivors_mask: 0x0F,
        alive_bit: 0x10,
    },
];

/// 砲座的狀態。位置與圖在每幀畫完之後才更新（[`Core::advance_pods`]），所以畫的與碰撞用的是上一幀算好的值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Pods {
    /// 第一座砲座的旗標位元組：`Y` 已裝上，`N` 沒有。畫、碰撞、發射都只認 `Y`。
    pub first: u8,
    /// 第二座砲座的旗標位元組：`N` 沒有，其他值（`Y`）已裝上。
    pub second: u8,
    /// 朝向字組：`>` 朝右，`<` 朝左。砲座的子彈與位置、圖都看它，主機彈不受影響。
    pub facing: u16,
    /// 砲座的圖庫索引。
    pub image: u16,
    /// 圖框動畫的計數。
    pub anim: u16,
    /// 砲座在格網上的位置：欄、列。第二座砲座在第一座下方 [`PodStyle::gap_rows`] 列。
    pub col: u16,
    pub row: u16,
    /// 砲座的繪製位置：x（位元組）、y（像素列）。
    pub blit_x: u16,
    pub blit_y: u16,
}

impl Pods {
    /// 第一座砲座在場（會畫、會碰撞、會發射）。
    pub(crate) fn first_flying(&self) -> bool {
        self.first == YES
    }

    /// 第二座砲座在場。
    pub(crate) fn second_flying(&self) -> bool {
        self.second != NO
    }

    /// 已裝上的砲座數，0 到 2（給介面顯示）。
    pub(crate) fn installed(&self) -> u8 {
        u8::from(self.first != NO) + u8::from(self.second != NO)
    }
}

/// 一位玩家的狀態。欄位用原版的字組寬度，順序與位移對應見 `mirror.rs`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Player {
    /// 血量：初值 FFh，每次受擊右移一位，亮的位元數就是剩下的格數，0 是死亡。跨關保留，只在第一關或展示模式重設。
    pub hp: u16,
    /// 格網位置：欄、列。欄 1 至 33 的奇數，列 0 至 16。
    pub col: u16,
    pub row: u16,
    /// 繪製位置：x（位元組）、y（像素列），和欄、列同步（x = 欄，y = 列 × 10）。
    pub blit_x: u16,
    pub blit_y: u16,
    /// 本體的圖庫索引（[`BodyImages`]）。
    pub image: u16,
    /// 本幀受擊旗標：`Y` 受擊。下一幀的 [`Core::control_player`] 結算並清掉；死亡動畫中不清。
    pub hit: u8,
    /// 受擊後的無敵倒數，大於 0 的幀只畫遮罩、跳過所有碰撞。
    pub invincible: u8,
    /// 死亡倒數。活著時每幀設成 [`DYING_FRAMES`]，血量歸零後每幀減 1，到 0 清掉存活位元。
    pub dying: u8,
    /// 尾焰旗標：本幀往右移動過。
    pub flame: u8,
    /// 武器字母：0、`L`、`B`、`K`、`S`。跨關保留。
    pub weapon: u16,
    /// 充能計數：開火鍵按住時（第二座砲座在場）每個事件加 1，放開時不小於 [`SWEEP_CHARGE`] 就射大掃射，然後歸零。
    pub charge: u16,
    /// 本幀開火旗標：新按那一幀設 `Y`，沒有開火事件的幀清成 `N`，按住的幀不動（所以新按那幀的 `Y` 會延續，砲座多發）。
    pub fired: u8,
    /// 本幀的輸入位元（[`InputBit`]），也是展示錄影的內容。
    pub input_bits: u8,
    /// 撿取光柱的 y：撿到道具時設成負值，每幀加 [`BEAM_STEP`] 並畫一道光柱，大於 0（有號）結束。
    pub beam_y: u16,
    pub pods: Pods,
}

impl Player {
    /// 上移的條件：列大於 0（有號）。
    pub(crate) fn can_move_up(&self) -> bool {
        (self.row as i16) > TOP_ROW
    }

    /// 下移的條件：列小於 10h（無號）。
    pub(crate) fn can_move_down(&self) -> bool {
        self.row < BOTTOM_ROW
    }

    /// 左移的條件：欄大於 1（有號）。
    pub(crate) fn can_move_left(&self) -> bool {
        (self.col as i16) > LEFT_COL
    }

    /// 右移的條件：欄小於 21h（無號）。
    pub(crate) fn can_move_right(&self) -> bool {
        self.col < RIGHT_COL
    }

    /// 上移一步，圖換成上移圖。到邊界不動，圖也不換。
    fn move_up(&mut self, images: &BodyImages) {
        if self.can_move_up() {
            self.row = self.row.wrapping_sub(ROW_STEP);
            self.blit_y = self.blit_y.wrapping_sub(ROW_STEP * ROW_PIXELS);
            self.image = images.up;
        }
    }

    /// 下移一步，圖換成下移圖。
    fn move_down(&mut self, images: &BodyImages) {
        if self.can_move_down() {
            self.row = self.row.wrapping_add(ROW_STEP);
            self.blit_y = self.blit_y.wrapping_add(ROW_STEP * ROW_PIXELS);
            self.image = images.down;
        }
    }

    /// 右移一步，尾焰旗標設 `Y`。
    fn move_right(&mut self) {
        if self.can_move_right() {
            self.col = self.col.wrapping_add(COL_STEP);
            self.blit_x = self.blit_x.wrapping_add(COL_STEP);
            self.flame = YES;
        }
    }

    /// 左移一步。
    fn move_left(&mut self) {
        if self.can_move_left() {
            self.col = self.col.wrapping_sub(COL_STEP);
            self.blit_x = self.blit_x.wrapping_sub(COL_STEP);
        }
    }

    /// 每關開始的位置（欄 5、列 10，繪製位置同步）。
    fn move_to_start(&mut self) {
        self.col = START_COL;
        self.row = START_ROW;
        self.blit_x = START_COL;
        self.blit_y = START_ROW * ROW_PIXELS;
    }

    /// 第一關或展示模式重設的裝備：武器、血量、砲座、砲座朝向。
    fn reset_loadout(&mut self, style: &Style) {
        self.weapon = 0;
        self.hp = FULL_HEALTH;
        self.pods.first = NO;
        self.pods.second = NO;
        self.pods.facing = style.start_facing;
    }
}

/// 兩位玩家與共用的存活位元組。
#[derive(Clone, Debug, Default)]
pub(crate) struct Players {
    /// 存活位元組（原版 `byte_1CFDD`）：玩家 1 是位元 0，玩家 2 是位元 4。標題選角色時設，死亡動畫結束時清，
    /// 兩位都是 0 時回標題。
    pub alive: u8,
    pub ship: [Player; PLAYER_COUNT],
}

impl Players {
    pub(crate) fn is_alive(&self, who: usize) -> bool {
        self.alive & STYLES[who].alive_bit != 0
    }
}

impl Core {
    /// 玩家（0 是玩家 1，1 是玩家 2）是否在場。
    pub(crate) fn player_alive(&self, who: usize) -> bool {
        self.pl.is_alive(who)
    }

    /// 玩家在格網上的位置：(欄, 列)。
    pub(crate) fn player_cell(&self, who: usize) -> (u16, u16) {
        let p = &self.pl.ship[who];
        (p.col, p.row)
    }

    /// 還有沒有玩家在場。都不在時主迴圈回標題。
    pub(crate) fn any_player_alive(&self) -> bool {
        self.pl.alive != 0
    }

    /// 標題選角色：設存活位元組（01h 玩家 1、10h 玩家 2、11h 雙人）。
    pub(crate) fn set_players_present(&mut self, alive: u8) {
        self.pl.alive = alive;
    }

    /// 開發者秘技（INT 9 的 `sub_1A8F4`）：兩位的血量補滿。
    pub(crate) fn refill_health(&mut self) {
        for p in &mut self.pl.ship {
            p.hp = FULL_HEALTH;
        }
    }

    /// 每關開始的位置（原版 `sub_14F47`、`sub_158BA`）。第一關或展示模式另重設武器、血量、砲座與朝向，
    /// 其他關保留，所以裝備跨關。
    pub(crate) fn place_players_for_stage(&mut self) {
        let reset = self.demo_playing() || self.on_first_stage();
        for (p, style) in self.pl.ship.iter_mut().zip(&STYLES) {
            p.move_to_start();
            if reset {
                p.reset_loadout(style);
            }
        }
        // 強化狀態（砲座分離、細步、充能）不跨關。
        self.enh.reset_for_stage();
    }

    /// 玩家每幀的輸入與移動（原版 `sub_14458`、`sub_14F92`）。過關後不處理。
    pub(crate) fn update_player(&mut self, who: usize) {
        if self.stage_cleared() {
            return;
        }
        self.pl.ship[who].input_bits = 0;
        self.replay_demo_input(who);
        self.control_player(who);
        self.record_demo_input(who);
    }

    /// 展示模式：由展示錄影還原這位玩家本幀的方向鍵與開火旗標（原版 `sub_14497`、`sub_14FC5`）。
    ///
    /// 直接寫按鍵狀態表與開火旗標，後面的 [`Core::control_player`] 照一般輸入處理。不在場的玩家也照做，
    /// 所以玩家 2 在單人展示裡仍會留下 `Y` 的轉向旗標（[`Style::demo_turn_default`]）。
    /// 錄影位元組的位址 16 位元環繞，沒有檢查幀數上限。
    fn replay_demo_input(&mut self, who: usize) {
        if !self.demo_playing() {
            return;
        }
        let style = &STYLES[who];
        let keys = &style.keys;
        for scan in [keys.up, keys.down, keys.right, keys.left, keys.fire] {
            self.set_key(scan, false);
        }
        self.set_fire_flag(who, FireFlag::NewPress, true);
        self.set_fire_flag(who, FireFlag::Turn, style.demo_turn_default);
        let bits = self.demo_byte(self.stage_frame().wrapping_add(style.demo_offset));
        for (bit, scan) in [(InputBit::Up, keys.up), (InputBit::Down, keys.down), (InputBit::Right, keys.right), (InputBit::Left, keys.left)] {
            if bit.is_set(bits) {
                self.set_key(scan, true);
            }
        }
        if InputBit::Fire.is_set(bits) {
            self.set_fire_flag(who, FireFlag::Event, true);
        }
        if InputBit::Turn.is_set(bits) {
            self.set_fire_flag(who, FireFlag::Turn, true);
        }
    }

    /// 非展示時，把本幀的輸入位元錄進展示錄影（原版 `sub_14474`）。只有玩家 1 錄，幀數不超過 [`LAST_RECORDED_FRAME`]。
    fn record_demo_input(&mut self, who: usize) {
        if !STYLES[who].records_demo || self.demo_playing() {
            return;
        }
        let frame = self.stage_frame();
        if frame > LAST_RECORDED_FRAME {
            return;
        }
        self.record_demo_byte(frame, self.pl.ship[who].input_bits);
    }

    /// 在場玩家的一幀：結算上一幀的命中、方向鍵、轉向、充能與開火（原版 `sub_14501`、`sub_1502F`）。
    fn control_player(&mut self, who: usize) {
        if !self.pl.is_alive(who) {
            return;
        }
        let style = &STYLES[who];
        let p = &mut self.pl.ship[who];
        p.image = style.images.level;
        p.flame = NO;
        if p.hit != NO {
            self.take_hit(who);
            if self.pl.ship[who].hp == 0 {
                self.die_step(who);
                return;
            }
        }
        let p = &mut self.pl.ship[who];
        p.dying = DYING_FRAMES;
        p.hit = NO;
        // 強化選項（規格 005）。全關時以下三個呼叫都不做事，原版路徑沒有任何資料段寫入。
        let rules = self.rules();
        self.settle_options(who, rules);
        self.steer(who, rules.fine_step);
        self.turn_pods(who);
        self.split_pods(who, rules);
        if rules.wave_charge || rules.auto_fire {
            self.fire_enhanced(who, rules);
        } else {
            self.release_charge(who);
            self.fire_main(who);
        }
    }

    /// 結算受擊：血量右移一位（無敵秘技開著時不移），重畫血條，進入無敵倒數。
    fn take_hit(&mut self, who: usize) {
        if !self.invincible_cheat() {
            self.pl.ship[who].hp >>= 1;
        }
        self.draw_health_bar(who);
        self.pl.ship[who].invincible = HIT_INVINCIBILITY;
    }

    /// 血量歸零後的死亡動畫（原版 `sub_14679`、`sub_151A7`）：倒數減 1，到 0 清掉存活位元；否則不接受輸入，
    /// 往下再往右走一步，並在位置播特效。受擊旗標沒有清，所以每幀都回到這裡。
    fn die_step(&mut self, who: usize) {
        let style = &STYLES[who];
        let p = &mut self.pl.ship[who];
        p.dying = p.dying.wrapping_sub(1);
        if p.dying == 0 {
            p.dying = DYING_RESET;
            self.pl.alive &= style.survivors_mask;
            return;
        }
        p.move_down(&style.images);
        p.move_right();
        let (col, row) = (p.col, p.row);
        self.spawn_effect(EffectKind::PlayerDeath, col, row);
    }

    /// 方向鍵（原版 `sub_14501` 的 `1454Bh` 至 `145C6h`）。上優先於下，左優先於右；斜向鍵同時算兩個方向。
    /// 輸入位元不管有沒有真的移動（到邊界）都記。`fine` 是細步移動選項（規格 005），走 [`Core::steer_fine`]。
    fn steer(&mut self, who: usize, fine: bool) {
        if fine {
            return self.steer_fine(who);
        }
        let style = &STYLES[who];
        let up = self.any_held(style.keys.upward());
        let down = self.any_held(style.keys.downward());
        let left = self.any_held(style.keys.leftward());
        let right = self.any_held(style.keys.rightward());
        let p = &mut self.pl.ship[who];
        if up {
            p.move_up(&style.images);
            p.input_bits |= InputBit::Up as u8;
        } else if down {
            p.move_down(&style.images);
            p.input_bits |= InputBit::Down as u8;
        }
        if left {
            p.move_left();
            p.input_bits |= InputBit::Left as u8;
        } else if right {
            p.move_right();
            p.input_bits |= InputBit::Right as u8;
        }
    }

    fn any_held(&self, scans: [u8; 3]) -> bool {
        scans.into_iter().any(|scan| self.key_held(scan))
    }

    /// 轉向鍵：砲座的朝向在左右之間切換（原版 `145D1h`）。
    fn turn_pods(&mut self, who: usize) {
        if !self.fire_flag(who, FireFlag::Turn) {
            return;
        }
        self.set_fire_flag(who, FireFlag::Turn, false);
        let p = &mut self.pl.ship[who];
        p.input_bits |= InputBit::Turn as u8;
        p.pods.facing = if p.pods.facing != FACING_RIGHT { FACING_RIGHT } else { FACING_LEFT };
    }

    /// 開火鍵不在按住時結算充能（原版 `145F8h`）：累積到 [`SWEEP_CHARGE`] 以上就從玩家所在的欄射大掃射，
    /// 無論有沒有射出，充能歸零。
    fn release_charge(&mut self, who: usize) {
        if self.key_held(STYLES[who].keys.fire) {
            return;
        }
        let p = self.pl.ship[who];
        if p.charge >= SWEEP_CHARGE {
            self.fire_sweep(p.col);
        }
        self.pl.ship[who].charge = 0;
    }

    /// 主機開火（原版 `14621h` 起）：有開火事件才處理。新按射一發普通彈（朝右）並重新充能；
    /// 按住不放只在第二座砲座在場時累計充能，不連射。
    fn fire_main(&mut self, who: usize) {
        if !self.fire_flag(who, FireFlag::Event) {
            self.pl.ship[who].fired = NO;
            return;
        }
        self.set_fire_flag(who, FireFlag::Event, false);
        if !self.fire_flag(who, FireFlag::NewPress) {
            let p = &mut self.pl.ship[who];
            if p.pods.second != NO {
                p.charge = p.charge.wrapping_add(1);
            }
            return;
        }
        self.set_fire_flag(who, FireFlag::NewPress, false);
        let p = &mut self.pl.ship[who];
        p.charge = 0;
        p.input_bits |= InputBit::Fire as u8;
        p.fired = YES;
        let (col, row) = (p.col, p.row);
        // 細步的半列位置：子彈的繪製 y 跟著機身。原版恆為 0。
        self.enh.spawn_dy = self.main_gun_sub_row(who);
        self.fire_player_bullets(PLAIN_WEAPON, Facing::Right, col, row);
        self.enh.spawn_dy = 0;
    }
}

/// 測試用的設定與讀取。
#[cfg(test)]
impl Core {
    /// 設玩家的存活與位置。`None` 表示不存活，位置不動。
    pub(crate) fn set_players(&mut self, players: [Option<(u16, u16)>; PLAYER_COUNT]) {
        let mut alive = 0u8;
        for (who, p) in players.into_iter().enumerate() {
            if let Some((col, row)) = p {
                alive |= STYLES[who].alive_bit;
                self.set_player_cell(who, col, row);
            }
        }
        self.pl.alive = alive;
    }

    pub(crate) fn set_player_cell(&mut self, who: usize, col: u16, row: u16) {
        self.pl.ship[who].col = col;
        self.pl.ship[who].row = row;
    }
}

/// 三個檔案的單元測試共用的設定。
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::game::{Fixes, Start};
    use crate::mirror::{GRID1_START, GRID2_START};

    /// 全 0 的映像開出來的核心，列表已擺好，玩家狀態是原版映像的初值（旗標 `N`、光柱 y 為 2）。
    pub(crate) fn core() -> Core {
        let image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
        let mut c = Core::new(&image, &[], Start::NewGame { difficulty: None }, Fixes::NONE);
        c.fill_row_tables();
        for p in &mut c.pl.ship {
            p.hit = NO;
            p.fired = NO;
            p.flame = NO;
            p.beam_y = 2;
        }
        c.forget_stride();
        c.set_draw_page(0);
        c
    }

    /// 擺三張形狀表（本體 5 × 1、砲座 2 × 2、尾焰髒格 3 × 3，格值全 1），兩位玩家各一份。
    pub(crate) fn set_shapes(c: &mut Core) {
        use crate::mirror::PlayerShape;
        for who in 0..PLAYER_COUNT {
            for (shape, w, h) in [(PlayerShape::Body, 5u16, 1u16), (PlayerShape::Pod, 2, 2), (PlayerShape::Flame, 3, 3)] {
                let at = shape.table_offset(who);
                c.ds.w16(at, w);
                c.ds.w16(at + 2, h);
                for i in 0..w * h {
                    c.ds.w8(at + 4 + i, 1);
                }
            }
        }
    }

    /// 一位在 (`col`, `row`) 的活玩家，位置與繪製位置同步。
    pub(crate) fn place(c: &mut Core, who: usize, col: u16, row: u16) {
        c.pl.alive |= STYLES[who].alive_bit;
        let p = &mut c.pl.ship[who];
        (p.col, p.row, p.blit_x, p.blit_y) = (col, row, col, row * ROW_PIXELS);
    }

    pub(crate) fn put_g1(c: &mut Core, col: u16, row: u16, cell: u8) {
        c.ds.w8(GRID1_START + row * 0x28 + col, cell);
    }

    pub(crate) fn put_g2(c: &mut Core, col: u16, row: u16, cell: u8) {
        c.ds.w8(GRID2_START + row * 0x28 + col, cell);
    }

    /// 邏輯寫過、還沒被繪製命令帶走的形狀寬；沒有則 `None`。
    pub(crate) fn stride(c: &Core) -> Option<u16> {
        c.pending_stride()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{core, place};
    use super::*;
    use crate::dseg::Dseg;
    use crate::mirror::Mirror;

    fn hold(c: &mut Core, scan: u8) {
        c.set_key(scan, true);
    }

    #[test]
    fn the_two_players_differ_only_in_the_style_table() {
        // 同一個函式，同樣的按鍵（各自的鍵位）和同樣的起點，走出對稱的結果。
        let mut c = core();
        c.set_stage_frame(10);
        for who in 0..PLAYER_COUNT {
            place(&mut c, who, 7, 8);
            let k = STYLES[who].keys;
            hold(&mut c, k.up);
            hold(&mut c, k.right);
        }
        c.update_player(0);
        c.update_player(1);
        for who in 0..PLAYER_COUNT {
            let p = c.pl.ship[who];
            assert_eq!((p.col, p.row, p.blit_x, p.blit_y), (9, 7, 9, 70), "玩家 {} 往右上走一步", who + 1);
            assert_eq!(p.image, STYLES[who].images.up, "上移圖各自不同");
            assert_eq!((p.flame, p.input_bits), (YES, 0x05), "尾焰旗標與輸入位元（上 1 + 右 4）");
        }
        assert_eq!((c.pl.ship[0].image, c.pl.ship[1].image), (0x0B, 0x0E));
    }

    #[test]
    fn up_wins_over_down_and_left_over_right_and_diagonals_count_twice() {
        let mut c = core();
        place(&mut c, 0, 7, 8);
        let k = STYLES[0].keys;
        hold(&mut c, k.up);
        hold(&mut c, k.down);
        hold(&mut c, k.left);
        hold(&mut c, k.right);
        c.update_player(0);
        let p = c.pl.ship[0];
        assert_eq!((p.col, p.row), (5, 7), "上優先於下，左優先於右");
        assert_eq!(p.input_bits, 0x09);
        assert_eq!(p.flame, NO);
        // 斜向鍵：End（左下）= 下 + 左。
        let mut c = core();
        place(&mut c, 0, 7, 8);
        hold(&mut c, k.down_left);
        c.update_player(0);
        assert_eq!((c.pl.ship[0].col, c.pl.ship[0].row, c.pl.ship[0].input_bits), (5, 9, 0x0A));
        // 玩家 2 的 V（左下）同理。
        let mut c = core();
        place(&mut c, 1, 7, 8);
        hold(&mut c, STYLES[1].keys.down_left);
        c.update_player(1);
        assert_eq!((c.pl.ship[1].col, c.pl.ship[1].row, c.pl.ship[1].input_bits), (5, 9, 0x0A));
    }

    #[test]
    fn movement_stops_at_the_limits_but_still_records_the_input_bit() {
        let mut p = Player { col: 1, row: 0, blit_x: 1, blit_y: 0, ..Player::default() };
        let images = STYLES[0].images;
        p.move_up(&images);
        p.move_left();
        assert_eq!((p.col, p.row, p.blit_y, p.image), (1, 0, 0, 0), "欄 1 不能再左，列 0 不能再上，圖也不換");
        assert!(!p.can_move_up() && !p.can_move_left() && p.can_move_down() && p.can_move_right());
        let mut p = Player { col: 0x21, row: 0x10, blit_x: 0x21, blit_y: 0x100, ..Player::default() };
        p.move_right();
        p.move_down(&images);
        assert_eq!((p.col, p.row, p.flame, p.image), (0x21, 0x10, 0, 0), "欄 21h 不能再右（無號），列 10h 不能再下");
        assert!(!p.can_move_right() && !p.can_move_down());
        // 負列、負欄是有號比較：不能上、不能左；無號比較：下與右的條件反而成立。
        let p = Player { col: 0xFFFF, row: 0xFFFF, ..Player::default() };
        assert!(!p.can_move_up() && !p.can_move_left());
        assert!(!p.can_move_down() && !p.can_move_right(), "負值當無號是很大的數");
        // 經 update_player：被擋住的方向仍記輸入位元。
        let mut c = core();
        place(&mut c, 0, 1, 0);
        hold(&mut c, STYLES[0].keys.up);
        hold(&mut c, STYLES[0].keys.left);
        c.update_player(0);
        assert_eq!((c.pl.ship[0].col, c.pl.ship[0].row, c.pl.ship[0].input_bits), (1, 0, 0x09));
    }

    #[test]
    fn a_hit_halves_the_health_unless_the_cheat_is_on_and_starts_the_invincibility() {
        for (cheat, want) in [(false, 0x7F), (true, 0xFF)] {
            let mut c = core();
            place(&mut c, 0, 7, 8);
            c.pl.ship[0].hp = 0xFF;
            c.pl.ship[0].hit = YES;
            if cheat {
                c.set_invincible_cheat(true);
            }
            c.update_player(0);
            let p = c.pl.ship[0];
            assert_eq!((p.hp, p.invincible, p.hit, p.dying), (want, HIT_INVINCIBILITY, NO, DYING_FRAMES), "秘技 {cheat}");
        }
    }

    #[test]
    fn health_reaching_zero_runs_the_death_animation_then_clears_only_that_players_alive_bit() {
        let mut c = core();
        place(&mut c, 0, 7, 8);
        place(&mut c, 1, 7, 8);
        for p in &mut c.pl.ship {
            p.dying = DYING_FRAMES;
        }
        c.pl.ship[0].hp = 1;
        c.pl.ship[0].hit = YES;
        c.update_player(0);
        let p = c.pl.ship[0];
        assert_eq!(p.hp, 0);
        assert_eq!((p.dying, p.hit), (DYING_FRAMES - 1, YES), "倒數減 1，受擊旗標不清所以每幀都回到這裡");
        assert_eq!((p.col, p.row), (9, 9), "每幀往下再往右走一步");
        assert_eq!((p.image, p.flame), (STYLES[0].images.down, YES));
        assert_eq!(c.fx.slots.iter().filter(|s| s.is_active()).count(), 1, "在位置播特效");
        // 再 13 幀倒數到 0：清存活位元。玩家 2 的位元不受影響。
        for _ in 0..DYING_FRAMES - 2 {
            c.update_player(0);
        }
        assert_eq!(c.pl.ship[0].dying, 1);
        c.update_player(0);
        assert_eq!((c.pl.ship[0].dying, c.pl.alive), (DYING_RESET, 0x10), "倒數 0 時清玩家 1 的位元，倒數重設成 6");
        let before = c.pl.ship[0];
        c.update_player(0);
        assert_eq!(c.pl.ship[0], before, "不在場的玩家不處理");
        // 玩家 2 死亡清的是自己那邊。
        c.pl.ship[1].hp = 1;
        c.pl.ship[1].hit = YES;
        for _ in 0..DYING_FRAMES {
            c.update_player(1);
        }
        assert_eq!(c.pl.alive, 0);
        assert!(!c.any_player_alive());
    }

    #[test]
    fn no_player_logic_runs_after_the_stage_is_cleared() {
        let mut c = core();
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].input_bits = 0x55;
        hold(&mut c, STYLES[0].keys.up);
        c.mark_stage_cleared();
        c.update_player(0);
        assert_eq!((c.pl.ship[0].row, c.pl.ship[0].input_bits), (8, 0x55), "連輸入位元都不清");
    }

    #[test]
    fn firing_is_one_bullet_per_press_and_holding_only_charges_with_a_second_pod() {
        let mut c = core();
        place(&mut c, 0, 7, 8);
        c.set_fire_flag(0, FireFlag::NewPress, true);
        c.set_fire_flag(0, FireFlag::Event, true);
        c.update_player(0);
        let p = c.pl.ship[0];
        assert_eq!((p.fired, p.charge, p.input_bits), (YES, 0, 0x10));
        assert_eq!(c.pb.slots.iter().filter(|s| s.is_active()).count(), 1, "主機射一發");
        assert!(!c.fire_flag(0, FireFlag::NewPress) && !c.fire_flag(0, FireFlag::Event), "旗標處理後清掉");
        // 按住：有事件、不是新按。沒有第二座砲座不累計，也不射。
        c.set_fire_flag(0, FireFlag::Event, true);
        hold(&mut c, STYLES[0].keys.fire);
        c.update_player(0);
        assert_eq!((c.pl.ship[0].charge, c.pl.ship[0].fired), (0, YES), "按住的幀不動 fired，新按那幀的 Y 延續");
        assert_eq!(c.pb.slots.iter().filter(|s| s.is_active()).count(), 1);
        // 有第二座砲座就累計。
        c.pl.ship[0].pods.second = YES;
        for _ in 0..3 {
            c.set_fire_flag(0, FireFlag::Event, true);
            c.update_player(0);
        }
        assert_eq!(c.pl.ship[0].charge, 3);
        // 沒有事件的幀把 fired 清成 N。
        c.update_player(0);
        assert_eq!(c.pl.ship[0].fired, NO);
    }

    #[test]
    fn releasing_the_fire_key_with_enough_charge_sweeps_and_always_clears_the_charge() {
        let mut c = core();
        place(&mut c, 1, 9, 8);
        c.pl.ship[1].charge = SWEEP_CHARGE - 1;
        c.update_player(1);
        assert_eq!((c.pl.ship[1].charge, c.pb.slots.iter().filter(|s| s.is_active()).count()), (0, 0), "不夠就只歸零");
        c.pl.ship[1].charge = SWEEP_CHARGE;
        c.update_player(1);
        assert_eq!(c.pb.slots.iter().filter(|s| s.is_active()).count(), 6, "大掃射六顆");
        assert!(c.pb.slots.iter().filter(|s| s.is_active()).all(|s| s.bullet.col == 9), "從玩家所在的欄射出");
        // 開火鍵還按著就不結算。
        let mut c = core();
        place(&mut c, 1, 9, 8);
        c.pl.ship[1].charge = 20;
        hold(&mut c, STYLES[1].keys.fire);
        c.update_player(1);
        assert_eq!(c.pl.ship[1].charge, 20);
    }

    #[test]
    fn the_turn_key_toggles_the_pod_facing_and_records_its_bit() {
        let mut c = core();
        place(&mut c, 0, 7, 8);
        c.pl.ship[0].pods.facing = FACING_LEFT;
        c.set_fire_flag(0, FireFlag::Turn, true);
        c.update_player(0);
        assert_eq!((c.pl.ship[0].pods.facing, c.pl.ship[0].input_bits), (FACING_RIGHT, 0x20));
        assert!(!c.fire_flag(0, FireFlag::Turn));
        c.set_fire_flag(0, FireFlag::Turn, true);
        c.update_player(0);
        assert_eq!(c.pl.ship[0].pods.facing, FACING_LEFT);
        // 任何不是 `>` 的值再轉一次都變成 `>`。
        c.pl.ship[0].pods.facing = 0;
        c.set_fire_flag(0, FireFlag::Turn, true);
        c.update_player(0);
        assert_eq!(c.pl.ship[0].pods.facing, FACING_RIGHT);
    }

    #[test]
    fn demo_replay_restores_keys_and_flags_and_player_two_reads_the_next_byte() {
        let mut c = core();
        c.set_demo_playing(true);
        c.set_stage_frame(3);
        c.record_demo_byte(3, 0b0010_0101); // 上、右、轉向
        c.record_demo_byte(4, 0b0001_1010); // 下、左、開火
        // 先擺滿髒值，確認還原會清掉。
        for who in 0..PLAYER_COUNT {
            let k = STYLES[who].keys;
            for scan in [k.up, k.down, k.right, k.left, k.fire] {
                c.set_key(scan, true);
            }
        }
        c.replay_demo_input(0);
        c.replay_demo_input(1);
        let k1 = STYLES[0].keys;
        assert_eq!([k1.up, k1.down, k1.right, k1.left, k1.fire].map(|s| c.key_held(s)), [true, false, true, false, false], "玩家 1 讀本幀位元組");
        assert!(c.fire_flag(0, FireFlag::NewPress) && !c.fire_flag(0, FireFlag::Event) && c.fire_flag(0, FireFlag::Turn));
        let k2 = STYLES[1].keys;
        assert_eq!([k2.up, k2.down, k2.right, k2.left, k2.fire].map(|s| c.key_held(s)), [false, true, false, true, false], "玩家 2 讀下一個位元組");
        assert!(c.fire_flag(1, FireFlag::NewPress) && c.fire_flag(1, FireFlag::Event));
        assert!(c.fire_flag(1, FireFlag::Turn), "玩家 2 的轉向旗標預設就是 Y");
        // 沒有轉向位元：玩家 1 清成 N，玩家 2 仍是 Y。
        c.set_stage_frame(0);
        c.replay_demo_input(0);
        c.replay_demo_input(1);
        assert!(!c.fire_flag(0, FireFlag::Turn) && c.fire_flag(1, FireFlag::Turn), "原版玩家 1 與玩家 2 的不對稱");
        // 非展示時完全不動。
        let mut c = core();
        c.set_demo_playing(false);
        c.set_key(k1.up, true);
        c.replay_demo_input(0);
        assert!(c.key_held(k1.up));
    }

    #[test]
    fn only_player_one_records_the_input_bits_and_never_during_a_demo_or_past_the_limit() {
        let mut c = core();
        c.set_demo_playing(false);
        c.pl.ship[0].input_bits = 0x2A;
        c.pl.ship[1].input_bits = 0x15;
        c.set_stage_frame(7);
        c.record_demo_input(0);
        c.record_demo_input(1);
        assert_eq!((c.demo_byte(7), c.demo_byte(8)), (0x2A, 0), "玩家 2 沒有錄製碼");
        c.set_stage_frame(LAST_RECORDED_FRAME);
        c.record_demo_input(0);
        assert_eq!(c.demo_byte(LAST_RECORDED_FRAME), 0x2A, "5DBh 還錄");
        c.pl.ship[0].input_bits = 0x01;
        c.set_stage_frame(LAST_RECORDED_FRAME + 1);
        c.record_demo_input(0);
        assert_eq!(c.demo_byte(LAST_RECORDED_FRAME + 1), 0, "超過 5DBh 不錄");
        c.set_demo_playing(true);
        c.set_stage_frame(9);
        c.record_demo_input(0);
        assert_eq!(c.demo_byte(9), 0, "展示中不錄");
    }

    #[test]
    fn stage_start_places_both_players_and_resets_the_loadout_only_on_the_first_stage_or_in_a_demo() {
        let loadout = |p: &Player| (p.weapon, p.hp, p.pods.first, p.pods.second, p.pods.facing);
        for (stage, demo, reset) in [(b'A', b'N', true), (b'B', b'N', false), (b'B', b'Y', true), (b'A', b'Y', true)] {
            let mut c = core();
            c.set_stage_letter(stage);
            c.set_demo_playing(demo == b'Y');
            for p in &mut c.pl.ship {
                *p = Player { weapon: 0x4C, hp: 0x0F, col: 20, row: 3, blit_x: 20, blit_y: 30, ..Player::default() };
                p.pods.first = YES;
                p.pods.second = YES;
                p.pods.facing = 0x77;
            }
            c.place_players_for_stage();
            for (who, p) in c.pl.ship.iter().enumerate() {
                assert_eq!((p.col, p.row, p.blit_x, p.blit_y), (5, 10, 5, 100), "玩家 {} 的起點", who + 1);
                if reset {
                    assert_eq!(loadout(p), (0, 0xFF, NO, NO, STYLES[who].start_facing), "關 {} 展示 {}", stage as char, demo as char);
                } else {
                    assert_eq!(loadout(p), (0x4C, 0x0F, YES, YES, 0x77), "裝備跨關保留");
                }
            }
        }
        assert_eq!((STYLES[0].start_facing, STYLES[1].start_facing), (FACING_LEFT, FACING_RIGHT), "玩家 1 朝左，玩家 2 朝右");
    }

    #[test]
    fn a_new_core_starts_with_only_player_one_present() {
        // Core::new 為了直接進第一關設存活位元組；它在結構裡，不能只寫資料段（資料段的值會在下一次 store 被蓋掉）。
        let c = core();
        assert_eq!(c.pl.alive, 0x01);
        assert!(c.player_alive(0) && !c.player_alive(1));
        assert!(c.any_player_alive());
    }

    #[test]
    fn accessors_read_the_player_variables() {
        let mut c = core();
        c.set_players([Some((3, 4)), Some((5, 6))]);
        assert_eq!((c.player_alive(0), c.player_alive(1)), (true, true));
        assert_eq!((c.player_cell(0), c.player_cell(1)), ((3, 4), (5, 6)));
        c.set_players([None, Some((9, 8))]);
        assert_eq!((c.player_alive(0), c.player_alive(1)), (false, true));
        assert_eq!(c.player_cell(1), (9, 8));
        c.set_players_present(0x11);
        assert_eq!(c.pl.alive, 0x11);
        c.refill_health();
        assert_eq!((c.pl.ship[0].hp, c.pl.ship[1].hp), (0xFF, 0xFF));
    }

    #[test]
    fn the_public_status_views_read_the_mirrored_state() {
        use crate::game::{Fixes, Game, Start};
        // 全 0 的映像開一局：玩家 1 在場，起點欄 5 列 10，血量滿，砲座沒有。
        let image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
        let game = Game::new(&image, &[], Start::NewGame { difficulty: None }, Fixes::NONE);
        let status = game.player_status(0);
        assert_eq!((status.x, status.y), (5, 100));
        assert!(status.controllable && status.can_up && status.can_down && status.can_left && status.can_right);
        let p2 = game.player_status(1);
        assert!(!p2.controllable, "玩家 2 不在場");
        let hud = game.hud_status(0);
        assert_eq!((hud.active, hud.hp, hud.weapon, hud.pods, hud.charge), (true, 0xFF, 0, 0, 0));
        assert!(!game.hud_status(1).active);
    }

    #[test]
    fn the_status_views_follow_the_documented_conditions() {
        // Game 只有資料段複本，它的兩個視圖從 Mirror 讀回結構，條件和移動函式用的是同一組。
        let mut players = Players { alive: 0x11, ..Players::default() };
        players.ship[0] = Player { hp: 0, col: 0x21, row: 0, blit_x: 0x21, blit_y: 0, charge: 9, weapon: 0x4C, ..Player::default() };
        players.ship[0].pods.first = YES;
        players.ship[0].pods.second = YES;
        players.ship[1] = Player { hp: 0x0F, col: 1, row: 0x10, blit_x: 1, blit_y: 0x100, ..Player::default() };
        let mut ds = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        players.store(&mut ds);
        let back = Players::from_dseg(&ds);
        assert!(back.is_alive(0) && back.is_alive(1));
        let p1 = back.ship[0];
        assert!(!p1.can_move_right() && !p1.can_move_up() && p1.can_move_down() && p1.can_move_left());
        assert_eq!((p1.hp, p1.pods.installed(), p1.charge, p1.weapon), (0, 2, 9, 0x4C));
        let p2 = back.ship[1];
        assert!(!p2.can_move_left() && !p2.can_move_down() && p2.can_move_up() && p2.can_move_right());
    }

    #[test]
    fn installed_pods_count_every_flag_that_is_not_n() {
        let mut pods = Pods { first: NO, second: NO, ..Pods::default() };
        assert_eq!(pods.installed(), 0);
        pods.first = YES;
        assert_eq!((pods.installed(), pods.first_flying(), pods.second_flying()), (1, true, false));
        pods.second = YES;
        assert_eq!((pods.installed(), pods.second_flying()), (2, true));
        // 兩個旗標的判斷形式不同：第一座只認 `Y` 才飛，第二座只要不是 `N` 就飛，兩者都是原版的比較。
        pods.first = 0;
        pods.second = 0;
        assert_eq!((pods.first_flying(), pods.second_flying(), pods.installed()), (false, true, 2));
    }

    #[test]
    fn the_style_table_matches_the_documented_machine_code() {
        // docs/re/11-entities-combat.md 第 9.4、9.6 節與機器碼：鍵位掃描碼由按鍵狀態表位址（2748Bh + 掃描碼）換算。
        let [one, two] = STYLES;
        assert_eq!(
            [one.keys.up_left, one.keys.up, one.keys.up_right, one.keys.left, one.keys.right, one.keys.down_left, one.keys.down, one.keys.down_right, one.keys.fire]
                .map(|s| 0x2748B + u32::from(s)),
            [0x274D2, 0x274D3, 0x274D4, 0x274D6, 0x274D8, 0x274DA, 0x274DB, 0x274DC, 0x274C1]
        );
        assert_eq!(
            [two.keys.up_left, two.keys.up, two.keys.up_right, two.keys.left, two.keys.right, two.keys.down_left, two.keys.down, two.keys.down_right, two.keys.fire]
                .map(|s| 0x2748B + u32::from(s)),
            [0x2749E, 0x2749F, 0x274A0, 0x274AC, 0x274AE, 0x274BA, 0x274BB, 0x274BC, 0x274B5]
        );
        assert_eq!([one.images.level, one.images.up, one.images.down], [9, 0x0B, 7]);
        assert_eq!([two.images.level, two.images.up, two.images.down], [0x10, 0x0E, 0x12]);
        assert_eq!((one.pod.row_offset, one.pod.gap_rows, two.pod.row_offset, two.pod.gap_rows), (3, 5, 1, 2));
        assert_eq!((one.pod.y_offset, one.pod.draw_dy, two.pod.y_offset, two.pod.draw_dy), (0x1E, 5, 0x0A, 2));
        assert_eq!(((one.muzzle.image, one.muzzle.dx, one.muzzle.dy), (two.muzzle.image, two.muzzle.dx, two.muzzle.dy)), ((0x11, 3, 3), (0x0A, 3, 6)));
        assert_eq!((one.survivors_mask, two.survivors_mask, one.alive_bit, two.alive_bit), (0xF0, 0x0F, 0x01, 0x10));
    }

}
