//! 敵方子彈：開火、瞄準、移動、繪製與格網。
//!
//! 池有 20 槽。每槽一筆記錄（[`Bullet`]，原版 15 個字組）加一個池指標；指標為 0 表示空槽，
//! 使用中時等於這一槽記錄的位址。開火（原版 `sub_16D3B`）依字母查 [`FirePattern`] 的步驟表，
//! 每一步從樣板表複製一筆記錄、填入起點（原版 `sub_16CEA`）。每幀（原版 `sub_17361`）對每個使用中的槽
//! 讀一個腳本位元組、移動、畫圖、寫格網 G3 與 G2、必要時播尾跡，出界或腳本結束就釋放。
//! 語意見 `docs/re/11-entities-combat.md` 第 7 節。
//!
//! 狀態與資料段位移的對應在 `mirror.rs`，這個檔案不出現原版位移。
//!
//! 對外的入口：
//!
//! | 入口 | 原版 |
//! |---|---|
//! | [`Core::fire_enemy_bullets`] | `sub_16D3B`（呼叫端：實體隨機開火 `sub_1617A`、移動碼 0Bh、道具載體死亡 `sub_15DBE`） |
//! | [`Core::aim_heading`]、[`turn_toward`] | `sub_16F9F` 加 `sub_172DE`、`sub_173FE` 加 `sub_175EA`（呼叫端：實體 `sub_1606A`、追蹤路徑 `15E9Ah`） |
//! | [`Core::update_enemy_bullets`] | `sub_17361`（各關每幀） |
//! | [`Core::remove_hit_bullet`] | `sub_14D54`、`sub_15882`（玩家 1、玩家 2 碰到 G2） |
//! | [`EnemyBullets::release_all`] | `sub_10E33` 內清池指標表的那一段 |
//!
//! 瞄準（[`Core::aim_heading`]、[`Core::aim_target`]）與派發表（[`FirePattern`]、[`Step`]、[`Template`]）
//! 各自獨立，強化模式（規格 005，`ai.rs`）只換這兩處：`ai_aim` 在瞄準彈生成後把八向腳本換成逐幀的移動碼
//! （[`Core::refine_aim`]，`step_bullet` 讀 [`Core::ai_path_code`]），`ai_fire` 與 `ai_boss` 會多放子彈，
//! 沒有修正 `bullet_slot_carry` 時 [`Core::spawn_bullet`] 只用槽 0 至 14（[`Core::enemy_bullet_slot_limit`]）。
//! 選項全關時這些分支都不執行。

use crate::draw::Pool;
use crate::fx::{EffectKind, loop_count};
use crate::game::Core;
use crate::video::MyCopy;

/// 池的槽數。
pub(crate) const POOL_SIZE: usize = 20;

/// 記錄裡的 `Y` 字組。
const YES: u16 = b'Y' as u16;
/// `Bullet::library` 為 `M`：用 `MY.GO` 圖庫畫（遮罩、y 加 1、四平面）；其他值用舞台圖庫。
const MY_LIBRARY: u16 = b'M' as u16;

/// 出界門檻（無符號）：往上列 >= 13h、往下列 >= 12h、往左右欄 >= 27h 算出界。
const TOP_LIMIT: u16 = 0x13;
const BOTTOM_LIMIT: u16 = 0x12;
const SIDE_LIMIT: u16 = 0x27;

/// 寫 G3 時，敵彈只畫列 0 至 11h；列 >= 12h 的剩餘列都不畫。
const G3_ROW_LIMIT: u16 = 0x12;

/// 瞄準：發射點與目標的欄距在這個範圍內算正對（欄差 10 以內），列距在 ±3 以內算同一列。
const AIM_NEAR_COLS: u16 = 10;
const AIM_NEAR_ROWS: u16 = 3;
/// 發射點上移 3 列後仍在螢幕內的上限，瞄向上方時才成立。
const AIM_TOP_BAND: u16 = 20;

/// 八個方向，編號同移動碼 1 至 8。腳本的斜向會交錯斜走與橫走（例如 [`Heading::NE`] 是 02 03 交替），
/// 所以實際軌跡是偏平的，細節見文件第 7.2 節。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Heading {
    N = 1,
    NE,
    E,
    SE,
    S,
    SW,
    W,
    NW,
}

impl Heading {
    /// 原版的朝向編號 1 至 8。
    pub(crate) const fn code(self) -> u16 {
        self as u16
    }

    /// 從 0 起算，樣板表依它排列。
    pub(crate) const fn index(self) -> u16 {
        self as u16 - 1
    }
}

/// 道具球的種類。生成時格值高四位為 80h、40h、C0h、20h，玩家撿到換成對應武器。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Capsule {
    L,
    B,
    K,
    S,
}

/// 寬彈（5×1）的三種腳本：往西，微偏西北，微偏西南。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tilt {
    /// 腳本 07 07 07 08 07 07 08：微偏西北。
    NorthWest = 0,
    /// 全部往西。
    West = 1,
    /// 腳本 07 07 07 06 07 07 06：微偏西南。
    SouthWest = 2,
}

/// 五連彈的五個方向，依生成順序。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Spread {
    W = 0,
    NW,
    SW,
    N,
    S,
}

/// 敵彈樣板：資料段裡一筆唯讀的記錄，生成時複製進池。位址換算在 `mirror.rs`（[`Template::table_offset`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Template {
    /// 八向瞄準彈，1×1。依瞄準朝向選。
    Aimed(Heading),
    /// 道具球，2×3，`MY.GO` 圖庫。依瞄準朝向選。
    Capsule(Capsule, Heading),
    /// 字母 `A` 的樣板：腳本指標是 0007h（資料段開頭），沒有實體樣板使用 `A`，視為未使用（文件第 7.2 節，等級 I）。
    Unused,
    /// 寬彈 5×1，每步 4 欄，有尾跡。
    Wide(Tilt),
    /// 五連彈 3×2，每步 2 欄。
    Spread(Spread),
    /// 寬 12 的一幀閃光：腳本第一碼是 0，生成當幀就結束。
    WideFlash,
    /// 寬 5 的一幀閃光。
    Flash,
    /// 八向直線彈 2×2。
    Straight(Heading),
    /// 慢彈，每步 2 欄：西南、西、西北各一。
    SlowSw,
    SlowW,
    SlowNw,
    /// 邊界反彈球，每步 2 欄。腳本是 9，每兩幀才走一步，方向取記錄的 `heading`（西北 8 或東北 2）。
    BouncerNw,
    BouncerNe,
}

/// 開火字母對應的圖案。字母是實體樣板的 `w7`，載體死亡掉落的道具球也走這裡。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FirePattern {
    /// `F`：一顆瞄準彈。
    AimedShot,
    /// `l`、`b`、`k`、`s`：瞄準後射出對應的道具球。
    Capsule(Capsule),
    /// `A`：沒有意義的樣板。
    Unused,
    /// `B`：起點列 +1，瞄準後射寬彈。
    AimedWide,
    /// `L`：起點欄 +8，五連彈。
    FiveWay,
    /// `V`：起點欄 -12，寬 12 的一幀閃光。
    WideFlash,
    /// `1`：欄 +4、列 -2，一幀閃光。
    FlashNear,
    /// `2`：欄 +3、列 -2，一幀閃光。
    FlashFar,
    /// `X`：北、西、西北。
    UpLeft,
    /// `C`：東北、東、東南。
    Right,
    /// `<`：列 +2，八向放射。
    Ring,
    /// `?`：列 +6，八向放射（先 +4 再接 `<` 的 +2）。
    RingLow,
    /// `Q`：欄 +4 一顆西北反彈球，再 +4 一顆東北反彈球。
    Bouncers,
    /// `U`：西北、東北。
    UpDiagonals,
    /// `D`：東南、西南。
    DownDiagonals,
    /// `E`：三顆慢彈，接六發瞄準彈。
    Barrage,
    /// 其他字母：一顆向上的 `F` 彈，不瞄準。
    Upward,
}

/// 開火圖案的一步。起點（欄、列）從呼叫端給的位置起算，`Shift` 累積往後的步驟。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// 起點平移。
    Shift { cols: i16, rows: i16 },
    /// 固定樣板。
    Fire(Template),
    /// 先瞄準，再依朝向選樣板。
    Aim(Aiming),
}

/// 瞄準後選哪一族樣板。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aiming {
    Shot,
    Capsule(Capsule),
    Wide,
}

impl Aiming {
    /// 依瞄準朝向選樣板。寬彈只分西北、西南與其他（原版對朝向 8、6 取微偏腳本，其餘取西）。
    pub(crate) fn template(self, heading: Heading) -> Template {
        match self {
            Aiming::Shot => Template::Aimed(heading),
            Aiming::Capsule(c) => Template::Capsule(c, heading),
            Aiming::Wide => Template::Wide(match heading {
                Heading::NW => Tilt::NorthWest,
                Heading::SW => Tilt::SouthWest,
                _ => Tilt::West,
            }),
        }
    }
}

const fn shift(cols: i16, rows: i16) -> Step {
    Step::Shift { cols, rows }
}

const fn fire(t: Template) -> Step {
    Step::Fire(t)
}

const fn straight(h: Heading) -> Step {
    Step::Fire(Template::Straight(h))
}

const fn aim(a: Aiming) -> Step {
    Step::Aim(a)
}

const AIMED_SHOT: &[Step] = &[aim(Aiming::Shot)];
const CAPSULE_L: &[Step] = &[aim(Aiming::Capsule(Capsule::L))];
const CAPSULE_B: &[Step] = &[aim(Aiming::Capsule(Capsule::B))];
const CAPSULE_K: &[Step] = &[aim(Aiming::Capsule(Capsule::K))];
const CAPSULE_S: &[Step] = &[aim(Aiming::Capsule(Capsule::S))];
const UNUSED: &[Step] = &[fire(Template::Unused)];
const AIMED_WIDE: &[Step] = &[shift(0, 1), aim(Aiming::Wide)];
const FIVE_WAY: &[Step] = &[
    shift(8, 0),
    fire(Template::Spread(Spread::W)),
    fire(Template::Spread(Spread::NW)),
    fire(Template::Spread(Spread::SW)),
    fire(Template::Spread(Spread::N)),
    fire(Template::Spread(Spread::S)),
];
const WIDE_FLASH: &[Step] = &[shift(-12, 0), fire(Template::WideFlash)];
const FLASH_NEAR: &[Step] = &[shift(4, -2), fire(Template::Flash)];
const FLASH_FAR: &[Step] = &[shift(3, -2), fire(Template::Flash)];
const UP_LEFT: &[Step] = &[straight(Heading::N), straight(Heading::W), straight(Heading::NW)];
const RIGHT: &[Step] = &[straight(Heading::NE), straight(Heading::E), straight(Heading::SE)];
const RING: &[Step] = &[
    shift(0, 2),
    straight(Heading::N),
    straight(Heading::NE),
    straight(Heading::E),
    straight(Heading::SE),
    straight(Heading::S),
    straight(Heading::SW),
    straight(Heading::W),
    straight(Heading::NW),
];
const RING_LOW: &[Step] = &[
    shift(0, 4),
    shift(0, 2),
    straight(Heading::N),
    straight(Heading::NE),
    straight(Heading::E),
    straight(Heading::SE),
    straight(Heading::S),
    straight(Heading::SW),
    straight(Heading::W),
    straight(Heading::NW),
];
const BOUNCERS: &[Step] = &[shift(4, 0), fire(Template::BouncerNw), shift(4, 0), fire(Template::BouncerNe)];
const UP_DIAGONALS: &[Step] = &[straight(Heading::NW), straight(Heading::NE)];
const DOWN_DIAGONALS: &[Step] = &[straight(Heading::SE), straight(Heading::SW)];
/// 三顆慢彈，然後六發瞄準彈：第一發在原點，第二發在 (x+4, y-8)，其後每發列 +3，共五發在 x+4。
const BARRAGE: &[Step] = &[
    fire(Template::SlowNw),
    fire(Template::SlowW),
    fire(Template::SlowSw),
    aim(Aiming::Shot),
    shift(4, -8),
    aim(Aiming::Shot),
    shift(0, 3),
    aim(Aiming::Shot),
    shift(0, 3),
    aim(Aiming::Shot),
    shift(0, 3),
    aim(Aiming::Shot),
    shift(0, 3),
    aim(Aiming::Shot),
];
const UPWARD: &[Step] = &[fire(Template::Aimed(Heading::N))];

impl FirePattern {
    /// 開火字母的派發表（原版 `sub_16D3B` 的比較順序）。字母是 ASCII 的字組。
    pub(crate) fn from_letter(letter: u16) -> FirePattern {
        match letter {
            0x46 => FirePattern::AimedShot,
            0x6C => FirePattern::Capsule(Capsule::L),
            0x62 => FirePattern::Capsule(Capsule::B),
            0x6B => FirePattern::Capsule(Capsule::K),
            0x73 => FirePattern::Capsule(Capsule::S),
            0x41 => FirePattern::Unused,
            0x42 => FirePattern::AimedWide,
            0x4C => FirePattern::FiveWay,
            0x56 => FirePattern::WideFlash,
            0x31 => FirePattern::FlashNear,
            0x32 => FirePattern::FlashFar,
            0x58 => FirePattern::UpLeft,
            0x43 => FirePattern::Right,
            0x3C => FirePattern::Ring,
            0x3F => FirePattern::RingLow,
            0x51 => FirePattern::Bouncers,
            0x55 => FirePattern::UpDiagonals,
            0x44 => FirePattern::DownDiagonals,
            0x45 => FirePattern::Barrage,
            _ => FirePattern::Upward,
        }
    }

    /// 這個圖案一次最多生成幾顆子彈。
    pub(crate) fn bullet_count(self) -> usize {
        self.steps().iter().filter(|s| matches!(s, Step::Fire(_) | Step::Aim(_))).count()
    }

    /// 這個圖案依序要做的事。
    pub(crate) fn steps(self) -> &'static [Step] {
        match self {
            FirePattern::AimedShot => AIMED_SHOT,
            FirePattern::Capsule(Capsule::L) => CAPSULE_L,
            FirePattern::Capsule(Capsule::B) => CAPSULE_B,
            FirePattern::Capsule(Capsule::K) => CAPSULE_K,
            FirePattern::Capsule(Capsule::S) => CAPSULE_S,
            FirePattern::Unused => UNUSED,
            FirePattern::AimedWide => AIMED_WIDE,
            FirePattern::FiveWay => FIVE_WAY,
            FirePattern::WideFlash => WIDE_FLASH,
            FirePattern::FlashNear => FLASH_NEAR,
            FirePattern::FlashFar => FLASH_FAR,
            FirePattern::UpLeft => UP_LEFT,
            FirePattern::Right => RIGHT,
            FirePattern::Ring => RING,
            FirePattern::RingLow => RING_LOW,
            FirePattern::Bouncers => BOUNCERS,
            FirePattern::UpDiagonals => UP_DIAGONALS,
            FirePattern::DownDiagonals => DOWN_DIAGONALS,
            FirePattern::Barrage => BARRAGE,
            FirePattern::Upward => UPWARD,
        }
    }
}

/// 一顆敵彈的記錄。欄位是原版的 15 個字組，順序與位移對應見 `mirror.rs`，數值沿用原版字組寬度。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Bullet {
    /// 反彈球的移動朝向（2、4、6、8）；0 表示不反彈。腳本碼 9 在繪製頁不為 0（8000h）時依它移動，出界時依它反彈。
    pub heading: u16,
    /// 圖框表（9 個字組）在資料段的指標，以移動碼索引，碼大於 8 取第 0 項。
    pub frames: u16,
    /// 繪製位置，單位是位元組（8 像素）。
    pub blit_x: u16,
    /// 繪製位置，單位是像素列。
    pub blit_y: u16,
    /// 每步的繪製位移：x（位元組）、y（像素列）。
    pub blit_dx: u16,
    pub blit_dy: u16,
    /// 沒有程式碼讀寫的字組（所有樣板都是 0），只在複製記錄時帶著走。
    pub spare: u16,
    /// 為 `Y` 時，每幀在 (`col`, `row`) 播尾跡特效。
    pub trail: u16,
    /// 為 `M` 時用 `MY.GO` 圖庫畫。
    pub library: u16,
    /// 格網位置：欄、列。
    pub col: u16,
    pub row: u16,
    /// 每步的格網位移：欄、列。
    pub dcol: u16,
    pub drow: u16,
    /// 腳本讀取位置，原版為資料段指標。
    pub script: u16,
    /// 形狀指標，指向 `寬, 高, 寬 × 高 位元組` 的格值。
    pub shape: u16,
}

/// 離開哪一邊界。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Top,
    Bottom,
    Right,
    Left,
}

impl Edge {
    /// 反彈球出這個邊界時的朝向轉換：`(原朝向, 新朝向)`，只有這兩組會反彈。
    fn bounces(self) -> [(u16, u16); 2] {
        match self {
            Edge::Top => [(8, 6), (2, 4)],
            Edge::Bottom => [(4, 2), (6, 8)],
            Edge::Right => [(2, 8), (4, 6)],
            Edge::Left => [(6, 4), (8, 2)],
        }
    }
}

impl Bullet {
    /// 出界（原版 `sub_17955`、`sub_1798E`、`sub_179C7`、`sub_17A00`）：預設釋放；反彈球（`heading` 非 0）
    /// 朝向在轉換表內就改朝向並留下，不在表內仍釋放。原版這四個函式開頭還檢查旗標是否為 `Y`，
    /// 呼叫端剛設成 `Y`，所以那個檢查恆成立，不移植。
    /// `release` 是整次移動共用的旗標（原版 `word_1FE71`），每次出界都重新指定，兩段移動的碼，後一次出界會蓋掉前一次的結果。
    fn leave(&mut self, edge: Edge, release: &mut bool) {
        *release = true;
        if self.heading == 0 {
            return;
        }
        *release = false;
        match edge.bounces().iter().find(|&&(from, _)| from == self.heading) {
            Some(&(_, to)) => self.heading = to,
            None => *release = true,
        }
    }

    /// 往上（原版 `sub_176D0`）：繪製 y 減 `blit_dy`、列減 `drow`，列 >= 13h（無號）出上界。
    fn move_up(&mut self, release: &mut bool) {
        self.blit_y = self.blit_y.wrapping_sub(self.blit_dy);
        self.row = self.row.wrapping_sub(self.drow);
        if self.row >= TOP_LIMIT {
            self.leave(Edge::Top, release);
        }
    }

    /// 往下（原版 `sub_176F6`）：列 >= 12h（無號）出下界。
    fn move_down(&mut self, release: &mut bool) {
        self.blit_y = self.blit_y.wrapping_add(self.blit_dy);
        self.row = self.row.wrapping_add(self.drow);
        if self.row >= BOTTOM_LIMIT {
            self.leave(Edge::Bottom, release);
        }
    }

    /// 往左（原版 `sub_1771C`）：繪製 x 減 `blit_dx`、欄減 `dcol`，欄 >= 27h（無號）出左界。
    fn move_left(&mut self, release: &mut bool) {
        self.blit_x = self.blit_x.wrapping_sub(self.blit_dx);
        self.col = self.col.wrapping_sub(self.dcol);
        if self.col >= SIDE_LIMIT {
            self.leave(Edge::Left, release);
        }
    }

    /// 往右（原版 `sub_17742`）：欄 >= 27h（無號）出右界。
    fn move_right(&mut self, release: &mut bool) {
        self.blit_x = self.blit_x.wrapping_add(self.blit_dx);
        self.col = self.col.wrapping_add(self.dcol);
        if self.col >= SIDE_LIMIT {
            self.leave(Edge::Right, release);
        }
    }

    fn leaves_trail(&self) -> bool {
        self.trail == YES
    }
}

/// 池的一槽。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Slot {
    /// 池指標表的項：0 表示空槽，使用中等於這一槽記錄的資料段位址（只有生成會寫，見 `mirror.rs` 的 `record_link`）。
    /// 載入傾印時保留原值，只為了寫回時位元組相同；邏輯只看是否為 0。
    pub link: u16,
    /// 記錄。空槽也保留舊內容：原版釋放只清指標，記錄原封不動，資料段比對會看到它。
    pub bullet: Bullet,
}

impl Slot {
    pub(crate) fn is_active(&self) -> bool {
        self.link != 0
    }
}

/// 敵方子彈領域的全部狀態。
#[derive(Clone, Debug, Default)]
pub(crate) struct EnemyBullets {
    pub(crate) slots: [Slot; POOL_SIZE],
}

impl EnemyBullets {
    /// 第一個空槽，只找槽號小於 `limit` 的。原版 `limit` 是 [`POOL_SIZE`]；強化規則（`ai.rs`）為了
    /// 不讓格值低四位進位（槽號 15 起，見 [`Core::stamp_grid2`]），在沒有修正時把它壓到 15。
    fn first_free_below(&self, limit: usize) -> Option<usize> {
        self.slots.iter().take(limit).position(|s| !s.is_active())
    }

    /// 使用中的槽數。
    pub(crate) fn active_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_active()).count()
    }

    /// 槽號小於 `limit` 的空槽數。
    pub(crate) fn free_below(&self, limit: usize) -> usize {
        self.slots.iter().take(limit).filter(|s| !s.is_active()).count()
    }

    /// 為新的一批子彈騰出空間（頭目新招式用）：槽號小於 `limit` 的空槽不足 `needed` 時，釋放使用中的子彈補足。
    /// 先放反彈球（第四關頭目的反彈球永不消失，會把池塞滿），再依槽號由小到大放（槽號小的通常飛得比較久）。
    /// 回傳是否騰出足夠空間。
    pub(crate) fn make_room(&mut self, limit: usize, needed: usize) -> bool {
        let mut free = self.free_below(limit);
        for bouncers_first in [true, false] {
            for slot in self.slots.iter_mut().take(limit) {
                if free >= needed {
                    return true;
                }
                if slot.is_active() && (slot.bullet.heading != 0) == bouncers_first {
                    slot.link = 0;
                    free += 1;
                }
            }
        }
        free >= needed
    }

    /// 關卡初始化：全部槽釋放（只清指標，記錄不動）。
    pub(crate) fn release_all(&mut self) {
        for s in &mut self.slots {
            s.link = 0;
        }
    }
}

/// 追蹤路徑的轉向（原版 `sub_173FE` 與 `sub_175EA`）：從目前朝向 `current` 往目標朝向 `target` 轉一步。
/// 每個目標有一張 8 項的表（[`TURN_TABLES`]）：目前朝向在前 4 項就減 1，在第 5、6 項就加 1，
/// 等於第 7 項就取第 8 項，都不是取 1（北）。已經相同則不動。
/// `current` 是實體記錄裡的字組，不保證在 1 至 8。
/// `target` 只來自 [`Core::aim_heading`]，恆為 1 至 8；原版對其他值回傳 7，不可達，不移植。
pub(crate) fn turn_toward(target: Heading, current: u16) -> u16 {
    if current == target.code() {
        return current;
    }
    let t = &TURN_TABLES[target.index() as usize];
    if t[..4].contains(&current) {
        current.wrapping_sub(1)
    } else if t[4..6].contains(&current) {
        current.wrapping_add(1)
    } else if current == t[6] {
        t[7]
    } else {
        Heading::N.code()
    }
}

/// 轉向表，依目標朝向 1 至 8 排列（原版 `sub_1744A`、`1747E`、`174B2`、`174E6`、`1751A`、`1754E`、`17582`、`175B6` 各寫一張）。
const TURN_TABLES: [[u16; 8]; 8] = [
    [2, 3, 4, 5, 6, 7, 8, 1],
    [3, 4, 5, 6, 7, 1, 8, 1],
    [4, 5, 6, 7, 1, 2, 8, 1],
    [5, 6, 7, 8, 1, 2, 3, 4],
    [6, 7, 8, 2, 3, 4, 1, 8],
    [7, 8, 2, 3, 4, 5, 1, 8],
    [8, 3, 2, 4, 5, 6, 1, 8],
    [2, 3, 4, 5, 6, 7, 1, 8],
];

impl Core {
    /// 在 (`x`, `y`)（欄、列）依字母開火（原版 `sub_16D3B`）。池滿了該顆放棄，後面的步驟照常做。
    pub(crate) fn fire_enemy_bullets(&mut self, letter: u16, x: u16, y: u16) {
        let (mut col, mut row) = (x, y);
        for step in FirePattern::from_letter(letter).steps() {
            match *step {
                Step::Shift { cols, rows } => {
                    col = col.wrapping_add(cols as u16);
                    row = row.wrapping_add(rows as u16);
                }
                Step::Fire(template) => {
                    self.spawn_bullet(template, col, row);
                }
                Step::Aim(aiming) => {
                    let heading = self.aim_heading(col, row);
                    if let Some(slot) = self.spawn_bullet(aiming.template(heading), col, row) {
                        // 瞄準更準（`ai_aim`）：已選好最接近的既有彈道，這裡再依實際向量換成逐幀的移動碼。
                        self.refine_aim(slot, aiming, col, row);
                    }
                }
            }
        }
    }

    /// 把樣板複製進第一個空槽，起點取 (`col`, `row`)，繪製位置跟著設（原版 `sub_16CEA`）。池滿了就放棄。
    /// 回傳配到的槽號。
    fn spawn_bullet(&mut self, template: Template, col: u16, row: u16) -> Option<usize> {
        let slot = self.eb.first_free_below(self.enemy_bullet_slot_limit())?;
        let mut bullet = self.bullet_template(template);
        bullet.col = col;
        bullet.blit_x = col;
        bullet.row = row;
        bullet.blit_y = row.wrapping_mul(10);
        self.eb.slots[slot] = Slot { link: crate::mirror::record_link(slot), bullet };
        self.bump_gen(Pool::EnemyBullet, slot as u8);
        Some(slot)
    }

    /// 瞄準：從 (`col`, `row`) 看最近玩家的方向（原版 `sub_16F9F`，所有比較為無符號）。
    ///
    /// 發射點上移 3 列後在 0 至 20 列且大於目標列，目標在上方：欄差超過 10 偏左右，否則正上。
    /// 發射點下移 3 列仍小於目標列，目標在下方，同樣分左、右、正下。其餘算同一列，只分左右。
    /// 欄小於 10 時 `col - 10` 繞回大值，永遠判成「目標在左」，這是原版行為。
    pub(crate) fn aim_heading(&self, col: u16, row: u16) -> Heading {
        let (target_col, target_row) = self.aim_target(col);
        let far_left = col.wrapping_sub(AIM_NEAR_COLS) > target_col;
        let far_right = col.wrapping_add(AIM_NEAR_COLS) < target_col;
        let above = row.wrapping_sub(AIM_NEAR_ROWS);
        if above <= AIM_TOP_BAND && above > target_row {
            if far_left {
                Heading::NW
            } else if far_right {
                Heading::NE
            } else {
                Heading::N
            }
        } else if row.wrapping_add(AIM_NEAR_ROWS) < target_row {
            if far_left {
                Heading::SW
            } else if far_right {
                Heading::SE
            } else {
                Heading::S
            }
        } else if col > target_col {
            Heading::W
        } else {
            Heading::E
        }
    }

    /// 選瞄準目標（原版 `sub_172DE`），回傳目標玩家的 (欄, 列)。
    ///
    /// 只有一位存活取那一位；兩位都不在取玩家 2。兩位都在時取欄距較小者，平手規則見各分支
    /// （`docs/re/11-entities-combat.md` 第 7.3 節）：發射點在兩位之間且等距取左邊那位，
    /// 兩位同欄時發射點在其右取玩家 2、在其左取玩家 1。
    pub(crate) fn aim_target(&self, col: u16) -> (u16, u16) {
        self.player_cell(self.aim_target_player(col))
    }

    /// [`Core::aim_target`] 選中的玩家編號（0 或 1）。
    pub(crate) fn aim_target_player(&self, col: u16) -> usize {
        let (first, second) = (self.player_cell(0), self.player_cell(1));
        let (c1, c2) = (first.0, second.0);
        let pick_first = if !self.player_alive(0) {
            false
        } else if !self.player_alive(1) {
            true
        } else if col > c1 {
            if col > c2 {
                // 兩位都在左邊：取欄較大者（較近）；相等取玩家 2。
                c1 > c2
            } else {
                // 在兩位之間（玩家 1 在左）：到玩家 1 的距離不大於到玩家 2 才取玩家 1。
                col - c1 <= c2 - col
            }
        } else if col > c2 {
            // 在兩位之間（玩家 2 在左）：到玩家 2 的距離不大於到玩家 1 才取玩家 2，所以距離較大時取玩家 1。
            col - c2 > c1 - col
        } else {
            // 兩位都在右邊：取欄較小者；相等取玩家 1。
            c1 <= c2
        };
        if pick_first { 0 } else { 1 }
    }

    /// 每幀更新 20 顆敵彈（原版 `sub_17361`）：每個使用中的槽走一步，出界或腳本結束就釋放。
    pub(crate) fn update_enemy_bullets(&mut self) {
        for slot in 0..POOL_SIZE {
            if !self.eb.slots[slot].is_active() {
                continue;
            }
            self.set_obj(Pool::EnemyBullet, slot as u8);
            let mut bullet = self.eb.slots[slot].bullet;
            let release = self.step_bullet(&mut bullet, slot as u8);
            self.eb.slots[slot].bullet = bullet;
            if release {
                self.eb.slots[slot].link = 0;
            }
        }
        self.clear_obj();
    }

    /// 一顆敵彈的一幀（原版 `sub_173C5`）。回傳是否要釋放。出界的那一幀仍會畫、寫格網、播尾跡，釋放在最後。
    fn step_bullet(&mut self, bullet: &mut Bullet, slot: u8) -> bool {
        // `ai_aim` 配過逐幀路徑的敵彈不讀腳本；其他敵彈（含選項全關）照原版。
        let code = match self.ai_path_code(slot) {
            Some(code) => code,
            None => self.read_script_code(bullet),
        };
        let release = self.move_bullet(bullet, code);
        self.draw_bullet(bullet, code);
        self.stamp_grids(bullet, slot);
        if bullet.leaves_trail() {
            self.spawn_effect(EffectKind::Trail, bullet.col, bullet.row);
        }
        release
    }

    /// 讀這一幀的移動碼（腳本位元組的低 4 位元），指標前進一格（原版 `sub_173C5` 前半與 `sub_176B5`）。
    /// 高 4 位元為 Fh 時，低 4 位元是往回跳的位元組數：指標（已前進）減去它，改以該處位元組的低 4 位元為碼，
    /// 指標不再前進，所以腳本尾端的 `F7` 會讓最後 7 格循環。
    fn read_script_code(&self, bullet: &mut Bullet) -> u8 {
        let byte = self.table_byte(bullet.script);
        bullet.script = bullet.script.wrapping_add(1);
        let code = byte & 0x0F;
        if byte & 0xF0 != 0xF0 {
            return code;
        }
        bullet.script = bullet.script.wrapping_sub(code as u16);
        self.table_byte(bullet.script) & 0x0F
    }

    /// 依移動碼移動（原版 `sub_17631`）。回傳要不要釋放。
    ///
    /// 碼 1、3、5、7 是上、右、下、左，2、4、6、8 是上右、下右、下左、左上，0 結束。
    /// 碼 9 在繪製頁為 0 時不動，非 0 時改依記錄的 `heading` 當碼重新派發（每兩幀走一步）。
    /// 其他碼不動。原版對 `heading` 的低位元組是 9 的記錄會無限迴圈，樣板不會出現。
    fn move_bullet(&self, bullet: &mut Bullet, code: u8) -> bool {
        let mut release = false;
        let mut code = code;
        loop {
            match code {
                1 => bullet.move_up(&mut release),
                3 => bullet.move_right(&mut release),
                5 => bullet.move_down(&mut release),
                7 => bullet.move_left(&mut release),
                2 => {
                    bullet.move_up(&mut release);
                    bullet.move_right(&mut release);
                }
                4 => {
                    bullet.move_down(&mut release);
                    bullet.move_right(&mut release);
                }
                6 => {
                    bullet.move_down(&mut release);
                    bullet.move_left(&mut release);
                }
                8 => {
                    bullet.move_left(&mut release);
                    bullet.move_up(&mut release);
                }
                9 => {
                    if self.draw_page() != 0 {
                        code = bullet.heading as u8;
                        continue;
                    }
                }
                0 => release = true,
                _ => {}
            }
            return release;
        }
    }

    /// 畫敵彈（原版 `sub_17768` 前半）。圖框取自 `frames` 表，以移動碼索引，碼大於 8 取第 0 項。
    /// `MY.GO` 圖庫先畫遮罩再畫四平面，四平面的 y 加 1；舞台圖庫先遮罩再四平面，y 不變。
    fn draw_bullet(&mut self, bullet: &Bullet, code: u8) {
        let frame = if code > 8 { 0 } else { code as u16 };
        let index = self.table_word(bullet.frames.wrapping_add(2 * frame));
        if bullet.library == MY_LIBRARY {
            self.set_blit_params(bullet.blit_x, bullet.blit_y, index);
            self.cut_my(MyCopy::General);
            self.set_blit_params(bullet.blit_x, bullet.blit_y.wrapping_add(1), index);
            self.overlay_my(MyCopy::General);
        } else {
            self.set_blit_params(bullet.blit_x, bullet.blit_y, index);
            self.cut_stage();
            self.overlay_stage();
        }
    }

    /// 把形狀寫進 G3 與 G2。形狀寬同時寫進繪製的每列位元組殘值（`Core::set_shape_width`），
    /// 下一個繪製命令會記下它；寫兩次的值相同，所以只寫一次。
    fn stamp_grids(&mut self, bullet: &Bullet, slot: u8) {
        let width = self.table_word(bullet.shape);
        let height = self.table_word(bullet.shape.wrapping_add(2));
        self.set_shape_width(width);
        self.stamp_grid3(bullet, width, height);
        self.stamp_grid2(bullet, width, height, slot);
    }

    /// 形狀以 `or` 寫進 G3 髒格（原版 `sub_177D2`、`sub_17821`）。
    ///
    /// 列為負的敵彈什麼都不寫：`sub_17821` 會跳過形狀上方的列（調整來源指標與剩餘高度），
    /// 但回來時 `bx` 還原成原本的負列，下面 `bx >= 12h` 的無號比較立刻離開。
    /// 高度（有號）不大於 0 的形狀同樣不寫。其餘逐列寫，列 >= 12h 的剩餘列整個放棄。
    /// 每列只准寫在該列的範圍內（欄 0 至 39），超出的格略過，不像特效（`fx.rs`）可以溢進相鄰的列。
    /// 列距 50h 是從第一列累加，不是每列重查列表。
    fn stamp_grid3(&mut self, bullet: &Bullet, width: u16, height: u16) {
        let mut row = bullet.row;
        if (row as i16) < 0 || (height as i16) <= 0 {
            return;
        }
        let mut src = bullet.shape.wrapping_add(4);
        let mut cell = self.grid3_cell_addr(row, bullet.col);
        let mut first = self.grid3_cell_addr(row, 0);
        for _ in 0..height {
            if row >= G3_ROW_LIMIT {
                return;
            }
            let mut at = cell;
            for _ in 0..loop_count(width) {
                let v = self.table_byte(src);
                self.grid3_or_in_row(first, at, v);
                at = at.wrapping_add(2);
                src = src.wrapping_add(1);
            }
            row = row.wrapping_add(1);
            cell = cell.wrapping_add(crate::mirror::GRID3_ROW_BYTES);
            first = first.wrapping_add(crate::mirror::GRID3_ROW_BYTES);
        }
    }

    /// 形狀的非 0 格值加上槽號寫進 G2（原版 `sub_17855`、`sub_17898`）。
    ///
    /// 格值低四位是槽號 + 1（形狀的格值低四位為 1），玩家撿到或被打到時靠它找回敵彈（[`Core::remove_hit_bullet`]）。
    /// 8 位元相加，不處理進位：槽號 15 以上低四位繞回並進位到高四位，道具判定與刪除都會錯。
    /// `Fixes::bullet_slot_carry` 開啟時格值只留形狀，槽號另存在 `g2_slots`。
    /// 列基底、可寫範圍與列 >= 14h（含負列）別名到 G1 的行為在 `Core::grid2_row_base`、`Core::grid2_writable`。
    pub(crate) fn stamp_grid2(&mut self, bullet: &Bullet, width: u16, height: u16, slot: u8) {
        let mut src = bullet.shape.wrapping_add(4);
        let mut row = bullet.row;
        for _ in 0..loop_count(height) {
            let base = self.grid2_row_base(row);
            let mut at = base.wrapping_add(bullet.col);
            for _ in 0..loop_count(width) {
                if self.grid2_writable(base, at) {
                    let shape_cell = self.table_byte(src);
                    if shape_cell != 0 {
                        let cell = if self.fixes.bullet_slot_carry {
                            let index = self.grid2_index(at);
                            self.g2_slots[index] = slot.wrapping_add(1);
                            shape_cell
                        } else {
                            shape_cell.wrapping_add(slot)
                        };
                        self.grid2_write(at, cell);
                    }
                }
                at = at.wrapping_add(1);
                src = src.wrapping_add(1);
            }
            row = row.wrapping_add(1);
        }
    }

    /// 玩家碰到 G2 在 `addr`、格值為 `cell` 的格：刪除那一顆敵彈（原版 `sub_14D54`、`sub_15882`）。
    ///
    /// 槽號是格值低四位減 1。槽號 15 以上的敵彈低四位繞回（見 [`Core::stamp_grid2`]），原版因此刪錯槽；
    /// 低四位為 0 時槽號減 1 繞回 FFh，原版寫的是「池指標表第 255 項」，落在槽 15 記錄的列欄
    /// （[`EnemyBullets::clear_link_word`]）。`Fixes::bullet_slot_carry` 開啟時改用另存的槽號，刪對槽。
    ///
    /// 回傳原版這時留在 `bx` 的值（槽號 × 2）。呼叫端的掃描迴圈拿 `bx` 當列計數，被它蓋掉，
    /// 之後的列沿用這個殘值，所以要回傳。`None` 表示 `bx` 沒被改（修正模式找不到槽）。
    pub(crate) fn remove_hit_bullet(&mut self, cell: u8, addr: u16) -> Option<u16> {
        if self.fixes.bullet_slot_carry {
            let stored = *self.g2_slots.get(self.grid2_index(addr))?;
            if stored == 0 {
                return None;
            }
            let slot = (stored - 1) as usize;
            self.eb.clear_link_word(slot);
            return Some((slot as u16) << 1);
        }
        let slot = (cell & 0x0F).wrapping_sub(1);
        self.eb.clear_link_word(slot as usize);
        Some((slot as u16) << 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Fixes, Start};

    fn core() -> Core {
        let image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
        Core::new(&image, &[], Start::Demo, Fixes::NONE)
    }

    /// 在 `at` 擺一個 `w` × `h` 的形狀，格值全為 `cell`。
    fn put_shape(c: &mut Core, at: u16, w: u16, h: u16, cell: u8) {
        c.ds.w16(at, w);
        c.ds.w16(at + 2, h);
        for i in 0..w * h {
            c.ds.w8(at + 4 + i, cell);
        }
    }

    impl Heading {
        const ALL: [Heading; 8] =
            [Heading::N, Heading::NE, Heading::E, Heading::SE, Heading::S, Heading::SW, Heading::W, Heading::NW];
    }

    #[test]
    fn letters_pick_the_documented_patterns() {
        use FirePattern as P;
        for (letter, want) in [
            (b'F', P::AimedShot),
            (b'l', P::Capsule(Capsule::L)),
            (b'b', P::Capsule(Capsule::B)),
            (b'k', P::Capsule(Capsule::K)),
            (b's', P::Capsule(Capsule::S)),
            (b'A', P::Unused),
            (b'B', P::AimedWide),
            (b'L', P::FiveWay),
            (b'V', P::WideFlash),
            (b'1', P::FlashNear),
            (b'2', P::FlashFar),
            (b'X', P::UpLeft),
            (b'C', P::Right),
            (b'<', P::Ring),
            (b'?', P::RingLow),
            (b'Q', P::Bouncers),
            (b'U', P::UpDiagonals),
            (b'D', P::DownDiagonals),
            (b'E', P::Barrage),
            (b'Z', P::Upward),
            (b'a', P::Upward),
        ] {
            assert_eq!(FirePattern::from_letter(letter as u16), want, "字母 {}", letter as char);
        }
    }

    #[test]
    fn patterns_spawn_the_documented_templates_in_order() {
        // 每個字母第一個與最後一個樣板，加起點平移的總和。
        let expect = |pattern: FirePattern, templates: &[Template], dx: i16, dy: i16| {
            let mut c = (0u16, 0u16);
            let mut got = Vec::new();
            for step in pattern.steps() {
                match *step {
                    Step::Shift { cols, rows } => {
                        c = (c.0.wrapping_add(cols as u16), c.1.wrapping_add(rows as u16));
                    }
                    Step::Fire(t) => got.push(t),
                    Step::Aim(_) => {}
                }
            }
            assert_eq!(got, templates, "{pattern:?}");
            assert_eq!((c.0, c.1), (dx as u16, dy as u16), "{pattern:?} 的平移");
        };
        expect(FirePattern::FiveWay, &Spread::ALL_FIVE.map(Template::Spread), 8, 0);
        expect(FirePattern::WideFlash, &[Template::WideFlash], -12, 0);
        expect(FirePattern::FlashNear, &[Template::Flash], 4, -2);
        expect(FirePattern::FlashFar, &[Template::Flash], 3, -2);
        expect(FirePattern::Ring, &Heading::ALL.map(Template::Straight), 0, 2);
        expect(FirePattern::RingLow, &Heading::ALL.map(Template::Straight), 0, 6);
        expect(FirePattern::Bouncers, &[Template::BouncerNw, Template::BouncerNe], 8, 0);
        expect(
            FirePattern::UpLeft,
            &[Template::Straight(Heading::N), Template::Straight(Heading::W), Template::Straight(Heading::NW)],
            0,
            0,
        );
        expect(
            FirePattern::UpDiagonals,
            &[Template::Straight(Heading::NW), Template::Straight(Heading::NE)],
            0,
            0,
        );
        expect(
            FirePattern::DownDiagonals,
            &[Template::Straight(Heading::SE), Template::Straight(Heading::SW)],
            0,
            0,
        );
        expect(FirePattern::Barrage, &[Template::SlowNw, Template::SlowW, Template::SlowSw], 4, -8 + 12);
        let aims = FirePattern::Barrage.steps().iter().filter(|s| matches!(s, Step::Aim(_))).count();
        assert_eq!(aims, 6, "E 的瞄準彈六發");
    }

    impl Spread {
        const ALL_FIVE: [Spread; 5] = [Spread::W, Spread::NW, Spread::SW, Spread::N, Spread::S];
    }

    #[test]
    fn wide_aiming_picks_tilt_by_heading() {
        for (h, want) in [
            (Heading::NW, Tilt::NorthWest),
            (Heading::W, Tilt::West),
            (Heading::SW, Tilt::SouthWest),
            (Heading::N, Tilt::West),
            (Heading::E, Tilt::West),
            (Heading::S, Tilt::West),
        ] {
            assert_eq!(Aiming::Wide.template(h), Template::Wide(want), "{h:?}");
        }
    }

    #[test]
    fn aim_heading_follows_the_three_bands() {
        let mut c = core();
        c.set_players([Some((20, 10)), None]);
        // 目標在上方：發射點 (20, 15)，上移 3 列 = 12 > 10。
        assert_eq!(c.aim_heading(20, 15), Heading::N);
        assert_eq!(c.aim_heading(31, 15), Heading::NW, "欄差 11：偏左");
        assert_eq!(c.aim_heading(30, 15), Heading::N, "欄差剛好 10 仍算正上");
        // 目標在下方：發射點 (20, 2)，下移 3 列 = 5 < 10。
        assert_eq!(c.aim_heading(20, 2), Heading::S);
        assert_eq!(c.aim_heading(31, 2), Heading::SW);
        // 同一列（±3）：只分左右。
        assert_eq!(c.aim_heading(21, 10), Heading::W);
        assert_eq!(c.aim_heading(20, 10), Heading::E, "同欄取東");
        assert_eq!(c.aim_heading(19, 12), Heading::E);
        assert_eq!(c.aim_heading(19, 13), Heading::E, "上移 3 列恰等於目標列，不算上方");
        // 目標在右邊 11 欄以上才偏右。
        c.set_players([Some((35, 10)), None]);
        assert_eq!(c.aim_heading(20, 15), Heading::NE);
        assert_eq!(c.aim_heading(20, 2), Heading::SE);
        assert_eq!(c.aim_heading(25, 15), Heading::N, "欄差剛好 10 仍算正上");
    }

    #[test]
    fn aim_near_the_left_edge_wraps_to_far_left() {
        // 欄 < 10 時 col - 10 繞回大值，永遠判成「目標在左」，所以左邊緣的發射點不會瞄向右上、右下。
        let mut c = core();
        c.set_players([Some((30, 10)), None]);
        assert_eq!(c.aim_heading(5, 15), Heading::NW);
        assert_eq!(c.aim_heading(5, 2), Heading::SW);
    }

    #[test]
    fn aim_above_needs_the_launch_point_inside_the_screen_band() {
        let mut c = core();
        c.set_players([Some((20, 1)), None]);
        // 上移 3 列後超過 20（無號）：不算上方，落到同列分支。
        assert_eq!(c.aim_heading(20, 24), Heading::E);
        assert_eq!(c.aim_heading(20, 23), Heading::N, "上移 3 列 = 20，仍在範圍內");
        // 列 < 3 時 row - 3 繞回大值，同樣落到其他分支。
        assert_eq!(c.aim_heading(20, 2), Heading::E);
    }

    #[test]
    fn target_is_the_only_living_player() {
        let mut c = core();
        c.set_players([Some((5, 6)), None]);
        assert_eq!(c.aim_target(30), (5, 6));
        c.set_players([None, Some((7, 8))]);
        assert_eq!(c.aim_target(0), (7, 8));
        c.set_players([None, None]);
        c.set_player_cell(1, 3, 4);
        assert_eq!(c.aim_target(0), (3, 4), "兩位都不在取玩家 2");
    }

    #[test]
    fn target_ties_follow_the_documented_rules() {
        let mut c = core();
        // 兩位在發射點左邊：取欄較大者，同欄取玩家 2。
        c.set_players([Some((10, 1)), Some((14, 2))]);
        assert_eq!(c.aim_target(20), (14, 2));
        c.set_players([Some((14, 1)), Some((10, 2))]);
        assert_eq!(c.aim_target(20), (14, 1));
        c.set_players([Some((10, 1)), Some((10, 2))]);
        assert_eq!(c.aim_target(20), (10, 2), "同欄在右取玩家 2");
        // 兩位在右邊：取欄較小者，同欄取玩家 1。
        c.set_players([Some((30, 1)), Some((25, 2))]);
        assert_eq!(c.aim_target(20), (25, 2));
        c.set_players([Some((25, 1)), Some((30, 2))]);
        assert_eq!(c.aim_target(20), (25, 1));
        c.set_players([Some((25, 1)), Some((25, 2))]);
        assert_eq!(c.aim_target(20), (25, 1), "同欄在左取玩家 1");
        // 在兩位之間：取較近者，等距取左邊那位。
        c.set_players([Some((10, 1)), Some((20, 2))]);
        assert_eq!(c.aim_target(14), (10, 1));
        assert_eq!(c.aim_target(16), (20, 2));
        assert_eq!(c.aim_target(15), (10, 1), "等距取左邊的玩家 1");
        c.set_players([Some((20, 1)), Some((10, 2))]);
        assert_eq!(c.aim_target(15), (10, 2), "等距取左邊的玩家 2");
        assert_eq!(c.aim_target(14), (10, 2));
        assert_eq!(c.aim_target(16), (20, 1));
    }

    #[test]
    fn turn_toward_steps_one_heading_at_a_time() {
        for target in Heading::ALL {
            for current in 0..=9u16 {
                let new = turn_toward(target, current);
                if current == target.code() {
                    assert_eq!(new, current);
                } else {
                    assert!((1..=8).contains(&new), "{target:?} {current} -> {new}");
                }
            }
        }
        // 目標朝向 1：朝向 3 減 1，朝向 7 加 1，朝向 8 取第 8 項，朝向 0 取 1。
        assert_eq!(turn_toward(Heading::N, 3), 2);
        assert_eq!(turn_toward(Heading::N, 7), 8);
        assert_eq!(turn_toward(Heading::N, 8), 1);
        assert_eq!(turn_toward(Heading::N, 0), 1);
        // 目標朝向 5（南）：朝向 1 取第 7 項後的第 8 項，朝向 3 加 1 靠向 4。
        assert_eq!(turn_toward(Heading::S, 1), 8);
        assert_eq!(turn_toward(Heading::S, 3), 4);
        assert_eq!(turn_toward(Heading::S, 7), 6);
        // 現在的朝向不在 1 至 8（腳本指標的殘值）：取 1。
        assert_eq!(turn_toward(Heading::E, 0x3EB9), 1);
    }

    #[test]
    fn spawn_takes_first_free_slot_and_places_it() {
        let mut c = core();
        c.fire_enemy_bullets(b'F' as u16, 12, 7);
        let s0 = c.eb.slots[0];
        assert!(s0.is_active());
        assert_eq!(s0.link, crate::mirror::record_link(0));
        assert_eq!((s0.bullet.col, s0.bullet.blit_x, s0.bullet.row, s0.bullet.blit_y), (12, 12, 7, 70));
        c.fire_enemy_bullets(b'Z' as u16, 1, 2);
        assert_eq!(c.eb.slots[1].bullet.col, 1);
        c.eb.slots[0].link = 0;
        c.fire_enemy_bullets(b'Z' as u16, 3, 4);
        assert_eq!(c.eb.slots[0].bullet.col, 3, "釋放的槽優先重用");
    }

    #[test]
    fn full_pool_drops_the_new_bullets_but_later_steps_still_run() {
        let mut c = core();
        for i in 0..POOL_SIZE as u16 {
            c.fire_enemy_bullets(b'Z' as u16, i, 0);
        }
        let before = c.eb.clone().slots;
        c.fire_enemy_bullets(b'<' as u16, 9, 9);
        assert_eq!(c.eb.slots, before);
        // 空出一槽：八向放射只剩第一顆有地方放。
        c.eb.slots[7].link = 0;
        c.fire_enemy_bullets(b'<' as u16, 9, 9);
        assert_eq!(c.eb.slots[7].bullet.col, 9);
        assert_eq!(c.eb.slots[7].bullet.row, 11, "列 +2");
    }

    #[test]
    fn y_offsets_use_16_bit_wraparound() {
        let mut c = core();
        c.fire_enemy_bullets(b'1' as u16, 0, 1);
        let b = c.eb.slots[0].bullet;
        assert_eq!((b.col, b.row), (4, 0xFFFF), "列 1 - 2 繞回");
        assert_eq!(b.blit_y, 0xFFFFu16.wrapping_mul(10));
    }

    #[test]
    fn script_f7_loops_the_last_seven_codes() {
        let mut c = core();
        let script = 0x7000u16;
        for (i, b) in [1u8, 2, 3, 4, 5, 6, 7, 0xF7].into_iter().enumerate() {
            c.ds.w8(script + i as u16, b);
        }
        let mut b = Bullet { script, ..Bullet::default() };
        let codes: Vec<u8> = (0..10).map(|_| c.read_script_code(&mut b)).collect();
        // 第 8 次讀到 F7：指標（已前進到 8）減 7 回到第二個位元組，以它的碼 2 當本幀，指標不再前進；
        // 第 9 次再讀同一個位元組，所以碼 2 連出現兩次，之後接第三個位元組。
        assert_eq!(codes, [1, 2, 3, 4, 5, 6, 7, 2, 2, 3]);
        assert_eq!(b.script, script + 3);
    }

    #[test]
    fn move_codes_and_boundaries() {
        let c = core();
        let mut b = Bullet { col: 20, row: 10, blit_x: 20, blit_y: 100, blit_dx: 2, blit_dy: 10, dcol: 2, drow: 1, ..Bullet::default() };
        assert!(!c.move_bullet(&mut b, 3));
        assert_eq!((b.col, b.blit_x), (22, 22));
        assert!(!c.move_bullet(&mut b, 8));
        assert_eq!((b.col, b.blit_x, b.row, b.blit_y), (20, 20, 9, 90), "左上");
        assert!(!c.move_bullet(&mut b, 0x0A), "其他碼不動");
        assert_eq!((b.col, b.row), (20, 9));
        assert!(c.move_bullet(&mut b, 0), "碼 0 結束");
        // 出界：往右到欄 27h（含）。
        let mut e = Bullet { col: 0x25, dcol: 2, ..Bullet::default() };
        assert!(c.move_bullet(&mut e, 3));
        // 往上到列 13h 以上（無號，所以負列也算）。
        let mut e = Bullet { row: 0, drow: 1, ..Bullet::default() };
        assert!(c.move_bullet(&mut e, 1));
        // 往下到列 12h。
        let mut e = Bullet { row: 0x11, drow: 1, ..Bullet::default() };
        assert!(c.move_bullet(&mut e, 5));
        let mut e = Bullet { row: 0x10, drow: 1, ..Bullet::default() };
        assert!(!c.move_bullet(&mut e, 5));
    }

    #[test]
    fn bouncing_balls_turn_at_the_edges_and_others_leave() {
        let c = core();
        let ball = |heading: u16, col: u16, row: u16| Bullet { heading, col, row, dcol: 2, drow: 1, blit_dx: 2, blit_dy: 10, ..Bullet::default() };
        // 朝向 8（西北）往上出上界變 6；往左不出界時只有上界事件。
        let mut b = ball(8, 20, 0);
        assert!(!c.move_bullet(&mut b, 8));
        assert_eq!(b.heading, 6);
        // 朝向 2 出上界變 4，朝向 4 出下界變 2，6 出下界變 8，2 出右界變 8，4 出右界變 6，6 出左界變 4，8 出左界變 2。
        for (heading, code, col, row, want) in [
            (2u16, 1u8, 20u16, 0u16, 4u16),
            (4, 5, 20, 0x11, 2),
            (6, 5, 20, 0x11, 8),
            (2, 3, 0x25, 5, 8),
            (4, 3, 0x25, 5, 6),
            (6, 7, 0, 5, 4),
            (8, 7, 0, 5, 2),
        ] {
            let mut b = ball(heading, col, row);
            assert!(!c.move_bullet(&mut b, code), "朝向 {heading} 碼 {code}");
            assert_eq!(b.heading, want, "朝向 {heading} 碼 {code}");
        }
        // 朝向不在該邊界的表內：照樣釋放，朝向不變。
        let mut b = ball(4, 20, 0);
        assert!(c.move_bullet(&mut b, 1));
        assert_eq!(b.heading, 4);
        // 朝向為 0 的一般彈出界直接釋放。
        let mut b = ball(0, 20, 0);
        assert!(c.move_bullet(&mut b, 1));
    }

    #[test]
    fn a_later_in_bounds_move_keeps_an_earlier_release() {
        // 碼 2（上右）：上出界（不反彈）後右邊沒出界，仍要釋放。
        let c = core();
        let mut b = Bullet { row: 0, drow: 1, col: 5, dcol: 1, ..Bullet::default() };
        assert!(c.move_bullet(&mut b, 2));
    }

    #[test]
    fn code_9_moves_by_heading_only_on_odd_pages() {
        let mut c = core();
        let ball = Bullet { heading: 3, col: 5, dcol: 2, blit_dx: 2, ..Bullet::default() };
        let mut b = ball;
        c.set_draw_page(0);
        assert!(!c.move_bullet(&mut b, 9));
        assert_eq!(b, ball, "頁 0 不動");
        c.set_draw_page(0x8000);
        assert!(!c.move_bullet(&mut b, 9));
        assert_eq!((b.col, b.blit_x), (7, 2), "頁 8000h 依朝向 3 往右");
        // 朝向 0 的記錄走到碼 9：重新派發成碼 0，結束。
        let mut z = Bullet::default();
        assert!(c.move_bullet(&mut z, 9));
    }

    #[test]
    fn update_moves_draws_stamps_and_releases() {
        let mut c = core();
        c.fill_row_tables();
        // 腳本：往右兩格後結束。形狀 1 × 1，格值 1。
        let (script, shape, frames) = (0x7000u16, 0x7100u16, 0x7200u16);
        c.ds.w8(script, 3);
        c.ds.w8(script + 1, 3);
        c.ds.w8(script + 2, 0);
        put_shape(&mut c, shape, 1, 1, 1);
        for i in 0..9u16 {
            c.ds.w16(frames + 2 * i, 40 + i);
        }
        c.eb.slots[2] = Slot {
            link: crate::mirror::record_link(2),
            bullet: Bullet {
                frames,
                col: 5,
                row: 4,
                blit_x: 5,
                blit_y: 40,
                blit_dx: 1,
                blit_dy: 10,
                dcol: 1,
                drow: 1,
                script,
                shape,
                ..Bullet::default()
            },
        };
        c.draws.clear();
        c.update_enemy_bullets();
        let b = c.eb.slots[2].bullet;
        assert_eq!((b.col, b.blit_x, b.script), (6, 6, script + 1));
        assert!(c.eb.slots[2].is_active());
        let blits: Vec<_> = c
            .draws
            .iter()
            .filter_map(|d| if let crate::draw::DrawCmd::Blit(b) = d { Some(*b) } else { None })
            .collect();
        assert_eq!(blits.len(), 2, "遮罩與四平面各一次");
        assert!(blits.iter().all(|x| x.x == 6 && x.y == 40 && x.index == 43), "圖框取移動碼 3 的項");
        assert!(blits.iter().all(|x| x.obj.map(|o| (o.pool, o.slot)) == Some((Pool::EnemyBullet, 2))));
        assert_eq!(c.cur_obj, None, "更新完清掉目前物件");
        assert_eq!(c.ds.r8(c.grid3_cell_addr(4, 6)), 1, "形狀寫進 G3");
        assert_eq!(c.ds.r8(c.grid2_row_base(4) + 6), 1 + 2, "形狀格值加槽號寫進 G2");

        c.update_enemy_bullets();
        assert_eq!(c.eb.slots[2].bullet.col, 7);
        c.update_enemy_bullets();
        assert!(!c.eb.slots[2].is_active(), "碼 0 釋放槽");
        assert_eq!(c.eb.slots[2].bullet.col, 7, "釋放的槽保留記錄");
    }

    #[test]
    fn my_library_draws_mask_then_planes_one_row_lower() {
        let mut c = core();
        let (shape, frames) = (0x7100u16, 0x7200u16);
        put_shape(&mut c, shape, 1, 1, 1);
        for i in 0..9u16 {
            c.ds.w16(frames + 2 * i, 42);
        }
        let mut b = Bullet { library: MY_LIBRARY, frames, shape, blit_x: 3, blit_y: 50, col: 3, row: 5, ..Bullet::default() };
        c.draws.clear();
        c.draw_bullet(&b, 0);
        let ops: Vec<_> = c
            .draws
            .iter()
            .filter_map(|d| if let crate::draw::DrawCmd::Blit(x) = d { Some((x.op, x.y)) } else { None })
            .collect();
        assert_eq!(ops, [(crate::draw::Op::S18D46, 50), (crate::draw::Op::S18CE2, 51)]);
        b.library = 0;
        c.draws.clear();
        c.draw_bullet(&b, 0);
        let ops: Vec<_> = c
            .draws
            .iter()
            .filter_map(|d| if let crate::draw::DrawCmd::Blit(x) = d { Some((x.op, x.y)) } else { None })
            .collect();
        assert_eq!(ops, [(crate::draw::Op::S18F1A, 50), (crate::draw::Op::S18F5D, 50)]);
    }

    #[test]
    fn frame_index_falls_back_to_the_first_entry_above_8() {
        let mut c = core();
        let frames = 0x7200u16;
        put_shape(&mut c, 0x7100, 1, 1, 1);
        for i in 0..9u16 {
            c.ds.w16(frames + 2 * i, 100 + i);
        }
        let b = Bullet { frames, shape: 0x7100, ..Bullet::default() };
        for (code, want) in [(0u8, 100u16), (3, 103), (8, 108), (9, 100), (15, 100)] {
            c.draws.clear();
            c.draw_bullet(&b, code);
            let idx = c.draws.iter().find_map(|d| if let crate::draw::DrawCmd::Blit(x) = d { Some(x.index) } else { None });
            assert_eq!(idx, Some(want), "碼 {code}");
        }
    }

    #[test]
    fn trail_bullets_play_the_effect_at_their_cell() {
        let mut c = core();
        put_shape(&mut c, 0x7100, 1, 1, 0);
        c.ds.w8(0x7000, 0x0A);
        let b = Bullet { trail: YES, script: 0x7000, shape: 0x7100, col: 8, row: 3, ..Bullet::default() };
        c.eb.slots[0] = Slot { link: crate::mirror::record_link(0), bullet: b };
        c.update_enemy_bullets();
        assert_eq!((c.fx.slots[0].x, c.fx.slots[0].y), (8, 3));
        assert!(c.fx.slots[0].is_active());
    }

    #[test]
    fn g3_stamp_clips_to_the_row_and_writes_nothing_for_negative_rows() {
        let mut c = core();
        c.fill_row_tables();
        let shape = 0x7100u16;
        c.ds.w16(shape, 2);
        c.ds.w16(shape + 2, 3);
        for (i, v) in [1u8, 2, 3, 4, 5, 6].into_iter().enumerate() {
            c.ds.w8(shape + 4 + i as u16, v);
        }
        // 列 -1：原版什麼都不寫（見 stamp_grid3 的說明），不是只跳過上方那一列。
        let before = c.ds.bytes().to_vec();
        let b = Bullet { shape, col: 10, row: 0xFFFF, ..Bullet::default() };
        c.stamp_grid3(&b, 2, 3);
        assert_eq!(c.ds.bytes(), &before[..]);
        // 列 0 起正常寫三列。
        let b = Bullet { shape, col: 10, row: 0, ..Bullet::default() };
        c.stamp_grid3(&b, 2, 3);
        assert_eq!(c.ds.r8(c.grid3_cell_addr(0, 10)), 1);
        assert_eq!(c.ds.r8(c.grid3_cell_addr(0, 11)), 2);
        assert_eq!(c.ds.r8(c.grid3_cell_addr(2, 10)), 5);
        assert_eq!(c.ds.r8(c.grid3_cell_addr(2, 11)), 6);
        // 欄 39 的下一格在該列之外：略過，不溢進下一列的欄 0。
        let mut c = core();
        c.fill_row_tables();
        c.ds.w16(shape, 2);
        c.ds.w16(shape + 2, 1);
        c.ds.w8(shape + 4, 7);
        c.ds.w8(shape + 5, 9);
        let b = Bullet { shape, col: 39, row: 4, ..Bullet::default() };
        c.stamp_grid3(&b, 2, 1);
        assert_eq!(c.ds.r8(c.grid3_cell_addr(4, 39)), 7);
        assert_eq!(c.ds.r8(c.grid3_cell_addr(5, 0)), 0, "不溢進下一列");
        // 列 12h 以後整列放棄。
        let mut c = core();
        c.fill_row_tables();
        c.ds.w16(shape, 1);
        c.ds.w16(shape + 2, 3);
        for i in 0..3 {
            c.ds.w8(shape + 4 + i, 1);
        }
        let b = Bullet { shape, col: 3, row: 0x10, ..Bullet::default() };
        c.stamp_grid3(&b, 1, 3);
        assert_eq!(c.ds.r8(c.grid3_cell_addr(0x10, 3)), 1);
        assert_eq!(c.ds.r8(c.grid3_cell_addr(0x11, 3)), 1);
        assert_eq!(c.ds.r8(c.grid3_cell_addr(0x12, 3)), 0);
    }

    #[test]
    fn g2_stamp_adds_the_slot_and_stays_inside_columns_1_to_38() {
        let mut c = core();
        let shape = 0x7100u16;
        c.ds.w16(shape, 3);
        c.ds.w16(shape + 2, 1);
        for (i, v) in [1u8, 0, 1].into_iter().enumerate() {
            c.ds.w8(shape + 4 + i as u16, v);
        }
        let b = Bullet { shape, col: 37, row: 4, ..Bullet::default() };
        c.stamp_grid2(&b, 3, 1, 5);
        let base = c.grid2_row_base(4);
        assert_eq!(c.ds.r8(base + 37), 1 + 5);
        assert_eq!(c.ds.r8(base + 38), 0, "格值 0 不寫");
        assert_eq!(c.ds.r8(base + 39), 0, "欄 39 不可寫");
        // 欄 0 不可寫。
        let b = Bullet { shape, col: 0, row: 4, ..Bullet::default() };
        c.stamp_grid2(&b, 3, 1, 1);
        assert_eq!(c.ds.r8(base), 0);
    }

    #[test]
    fn g2_stamp_adds_without_carry_so_high_nibble_changes() {
        let mut c = core();
        let shape = 0x7100u16;
        put_shape(&mut c, shape, 1, 1, 0x81);
        let b = Bullet { shape, col: 5, row: 4, ..Bullet::default() };
        c.stamp_grid2(&b, 1, 1, 16);
        assert_eq!(c.ds.r8(c.grid2_row_base(4) + 5), 0x91, "81h + 16 進位到高四位");
        let mut c = core();
        c.ds.w16(shape, 1);
        c.ds.w16(shape + 2, 1);
        c.ds.w8(shape + 4, 0xF0);
        c.stamp_grid2(&b, 1, 1, 0x20);
        assert_eq!(c.ds.r8(c.grid2_row_base(4) + 5), 0x10, "8 位元相加，溢位丟掉");
    }

    #[test]
    fn g2_rows_at_or_past_20_alias_the_row_table_end_into_g1() {
        // 列 >= 14h（含負列）的列基底取自 G1 的前兩格。
        let mut c = core();
        c.ds.w16(crate::mirror::GRID1_START, 0x0003);
        let base = c.grid2_row_base(20);
        assert_eq!(base, crate::mirror::GRID2_START + 3);
        assert_eq!(c.grid2_row_base(0xFFFF), base);
        assert_eq!(c.grid2_row_base(1234), base);
    }

    #[test]
    fn removing_by_cell_value_matches_the_original_slot_arithmetic() {
        let mut c = core();
        for s in c.eb.slots.iter_mut() {
            s.link = 0x1234;
        }
        // 槽 16 的格值 01 + 16 = 11h，低四位 1 減 1 得槽 0：原版刪錯槽。
        assert_eq!(c.remove_hit_bullet(0x11, 0), Some(0));
        assert_eq!(c.eb.slots[0].link, 0);
        assert_eq!(c.eb.slots[16].link, 0x1234);
        // 槽 3 的格值 01 + 3 = 04h：槽 3，bx = 6。
        assert_eq!(c.remove_hit_bullet(0x04, 0), Some(6));
        assert_eq!(c.eb.slots[3].link, 0);
        // 只看低四位：高四位是道具標記。
        assert_eq!(c.remove_hit_bullet(0x85, 0), Some(8));
        assert_eq!(c.eb.slots[4].link, 0);
    }

    #[test]
    fn removing_with_low_nibble_zero_writes_slot_15s_row() {
        // 槽 15 的格值 01 + 15 = 10h，低四位 0 減 1 繞回 FFh，寫到池指標表第 255 項，落在槽 15 記錄的列（w10）。
        let mut c = core();
        c.eb.slots[15].bullet.row = 0x0A;
        c.eb.slots[15].link = 0x1234;
        assert_eq!(c.remove_hit_bullet(0x10, 0), Some(0x1FE));
        assert_eq!(c.eb.slots[15].bullet.row, 0);
        assert_eq!(c.eb.slots[15].link, 0x1234, "指標不動");
        assert_eq!(c.eb.slots[14].bullet.row, 0, "其他槽不受影響");
    }
}
