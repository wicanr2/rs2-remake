//! 敵人（原版稱實體）：生成、移動腳本、命中結算、足跡與繪製。
//!
//! 池有 15 槽。每槽一筆記錄（[`Enemy`]，原版 24 個字組）加一個池指標；指標為 0 表示空槽，
//! 使用中時等於這一槽記錄的位址。生成（原版 `sub_15D1C`）每幀掃一張時間表，幀計數相符的列從樣板複製一筆記錄
//! （原版 `sub_15D41`）。每幀（原版 `sub_15D8C`）對每個使用中的槽：結算上一幀的傷害、讀一個腳本位元組（或朝玩家轉一步）、
//! 移動、挑圖框、把足跡寫進 G3 與 G1、擲亂數決定開火；腳本結束或被打死的槽釋放（原版 `sub_15DBE`）。
//! 語意見 `docs/re/11-entities-combat.md` 第 2 至 5 節與 8.2 節。
//!
//! 樣板、腳本、圖框與形狀是唯讀表格，在 `tables.rs`。
//!
//! 對外的入口：
//!
//! | 入口 | 原版 |
//! |---|---|
//! | [`Core::spawn_scheduled`] | `sub_17D82`、`sub_17D8E`、`sub_17D9A`、`sub_17DA6`、`sub_17DB2`（各關每幀）加 `sub_15D1C` |
//! | [`Core::update_enemies`] | `sub_15D8C`（各關每幀） |
//! | [`Core::draw_enemies`]、[`Core::draw_enemy_overlays`] | `sub_164BB`、`sub_164D7` |
//! | [`Core::entity_link`]、[`Core::add_entity_damage`] | 玩家彈 `sub_16946` 與砲座 `sub_14C18`、`sub_15746` 命中敵人時存取池指標表與傷害字組 |
//! | [`Enemies::release_all`] | `sub_10E33` 內清池指標表的那一段 |
//! | [`Core::clear_grid1`]（`mirror.rs`） | `sub_162B1`（各關每幀末） |
//!
//! 強化模式（規格 005）的位置：移動腳本的直譯器是 [`Core::read_command`]、
//! [`Motion`] 與 [`Core::apply_motion`]；開火判斷是 [`Core::rolls_to_fire`]；生成表是 [`Timetable`] 與 [`Core::spawn_enemy`]；
//! 追蹤與反應走 [`Core::seek_row`]、[`Core::chase_row`]。戰場的欄數是 `mirror.rs` 的 `GRID_COLS`。
//!
//! 敵人 AI 強化（`ai.rs`）在 [`Core::step_enemy`] 有三個決策點，選項全關時一個都不執行：腳本讀出移動碼之後
//! （`ai_react` 換這一幀的碼）、原版的開火擲骰之後（`ai_fire` 加一次機會）、一幀的最後（`ai_boss` 頭目核心的新招式）。
//! `spawn_enemy` 另外記下敵人的角色，`update_enemies` 開頭記下玩家的位置，這兩步不碰資料段，也不碰原版的亂數表索引。
//!
//! 原版有幾處靠暫存器殘值或表外讀寫成立的行為，逐項寫在對應的函式文件裡：
//! 被打死的敵人沒有產生繪製值（[`Enemies::last_sprite`]）、裝甲格索引讀到池之後的記錄區
//! （[`Enemies::link_at`]）、傷害寫到指標 0 加 20h（[`Core::add_entity_damage`]）、
//! 形狀寬同時是繪製的每列位元組殘值（[`Core::for_each_footprint_cell`]）、
//! 死亡旗標的位址同時是 G3、G4 列起點表的越界列（`Core::set_enemy_death_flag`，在 `grid.rs`）。

use crate::draw::Pool;
use crate::ebullet::{Heading, turn_toward};
use crate::fx::{EffectKind, loop_count};
use crate::game::Core;
use crate::grid::GRID_COLS;
use crate::video::SCRATCH_B;

/// 池的槽數。
pub const POOL_SIZE: usize = 15;

/// 記錄的字組數。
const RECORD_WORDS: usize = 24;

/// y 的單位是格列，乘 10 為像素列。
const ROW_PIXELS: u16 = 10;

/// 旗標字組的值。
const YES: u16 = b'Y' as u16;
const NO: u16 = b'N' as u16;

/// 傷害字組的高位元組：1 表示本幀被打中，等著結算。
const HIT_PENDING: u8 = 1;

/// 開火門檻：這個值永遠不開火（載體、碰撞體、損壞貼片、核心）。
pub(crate) const NEVER_FIRES: u16 = 0x63;
/// 固定亂數表的項數（值域 0 至 15）。
pub(crate) const FIRE_ROLLS: u16 = 75;

// ---- 腳本 ----

/// 腳本控制碼：後接位址，跳到該處繼續。
const SCRIPT_JUMP: u8 = 0xF0;
/// 腳本控制碼：後接樣板位址，換成那個樣板，保留位置與死亡特效。
const SCRIPT_MORPH: u8 = 0xE0;
/// 圖框表與動畫上限表只有 9 項，以移動碼 0 至 8 索引。
const LAST_FRAMED_CODE: u16 = 8;

/// 移動碼 09、0A、0B、0D、0F。1 至 8 是八個方向，其餘不動。
const CODE_CHASE_ROW: u8 = 0x09;
const CODE_JUMP_RIGHT: u8 = 0x0A;
const CODE_FIRE_BURST: u8 = 0x0B;
const CODE_SEEK_ROW: u8 = 0x0D;
const CODE_DIE: u8 = 0x0F;

/// 碼 0A 右跳的欄數。
const JUMP_COLS: u16 = 6;
/// 碼 0B 與載體死亡掉道具的起點在敵人下方的列數。
const MUZZLE_ROWS: u16 = 2;
/// 碼 09 追玩家 1 的列：敵人列大於這個值就往下。
const CHASE_ROW_LIMIT: u16 = 0x18;
/// 碼 0D 的目標點相對敵人的位移，與判斷上下的兩個門檻（有號，目標列在 0 以下取下、超過 14h 取上）。
const SEEK_OFFSET_COLS: u16 = 3;
const SEEK_OFFSET_ROWS: u16 = 4;
const SEEK_ROW_BOTTOM: i16 = 0;
const SEEK_ROW_TOP: i16 = 0x14;

// ---- 足跡 ----

/// 足跡只寫到列 12h（有號比較，列 13h 起不寫）。G1 的列 13h 因此沒有敵人。
const FOOTPRINT_ROW_LIMIT: i16 = 0x13;
/// G1 格值：裝甲，不加槽號。玩家彈碰到不記錄為傷害目標，但子彈仍會消失。
pub(crate) const ARMOR_CELL: u8 = 0xFE;
/// 裝甲格的實體索引（格值減 1）。雷射與砲座沒有裝甲檢查，會以這個索引讀實體指標表（文件第 5.3 節）。
const ARMOR_INDEX: u8 = ARMOR_CELL - 1;
/// G1 格值：存在但不造成接觸，不加槽號。所有樣板的形狀都沒有用到，照原版保留。
const INERT_CELL: u8 = 0xFF;

/// 敵人的移動模式（記錄的 `mode`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// `N` 與不認得的字母：照腳本走。
    Script,
    /// `A`：每幀朝玩家轉一步，用在道具載體。
    Chase,
    /// `Y`：照腳本走，另外每幀隨景物往左 1 欄。
    Scroll,
}

impl Mode {
    pub(crate) fn from_word(word: u16) -> Mode {
        match word {
            0x41 => Mode::Chase,
            0x59 => Mode::Scroll,
            _ => Mode::Script,
        }
    }
}

/// 敵人怎麼畫（記錄的 `kind`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `O` 與不認得的字母：遮罩加平面。
    Normal,
    /// `N`：不畫，形狀仍寫進格網（隱形地形）。
    Hidden,
    /// `K`：不畫，死亡時把損壞貼片畫進暫存區 B（地面砲台）。
    Embedded,
    /// `S`：第二輪畫在玩家之上，不用遮罩，直接覆寫平面。為什麼要這樣畫，視覺效果未驗證（文件第 2.4 節）。
    Overlay,
}

impl Kind {
    pub(crate) fn from_word(word: u16) -> Kind {
        match word {
            0x4E => Kind::Hidden,
            0x4B => Kind::Embedded,
            0x53 => Kind::Overlay,
            _ => Kind::Normal,
        }
    }
}

/// 一個敵人的記錄，欄位照原版 24 個字組的順序（對照表在 `mirror.rs`）。
/// 字母、指標與旗標都留原版的字組寬度，轉換成列舉等到表格階段。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Enemy {
    /// 耐久門檻，999 是打不壞。累計傷害大於它加難度加成就死。
    pub hp: u16,
    /// 圖框表指標：9 個字組，以移動碼 0 至 8 索引。`K` 型指向 `(圖, x, y)` 三個字組。
    pub frames: u16,
    /// 繪製的 x，單位是位元組（8 像素），恆等於 `col`。
    pub blit_x: u16,
    /// 繪製的 y，像素列，恆等於 `row` 乘 10（除了幾個樣板的單步不是 10 的倍數，文件第 2.1 節）。
    pub blit_y: u16,
    /// 水平單步（繪製）。
    pub blit_dx: u16,
    /// 垂直單步（繪製，像素）。
    pub blit_dy: u16,
    /// 開火門檻：亂數值加難度加成大於它才開火，[`NEVER_FIRES`] 永不開火。
    pub fire_threshold: u16,
    /// 子彈字母，`E` 兼任過關標記。
    pub bullet: u16,
    /// 移動模式字母，見 [`Mode`]。
    pub mode: u16,
    /// 邏輯的欄，可為負。
    pub col: u16,
    /// 邏輯的列，可為負。
    pub row: u16,
    /// 水平單步（邏輯，欄）。
    pub dcol: u16,
    /// 垂直單步（邏輯，列）。
    pub drow: u16,
    /// 腳本讀取位置，原版為資料段指標。追蹤模式下存目前朝向 1 至 8。
    pub script: u16,
    /// 碰撞形狀指標，指向 `寬, 高, 寬 × 高 位元組` 的格值。
    pub shape: u16,
    /// 所有樣板都是 1，沒有任何指令讀。
    pub spare: u16,
    /// 傷害字組：低位元組是累計傷害，高位元組為 1 表示本幀被打中（外部寫入，下一幀結算）。
    pub damage: u16,
    /// 死亡特效字母，交給 [`EffectKind::from_letter`]。
    pub explosion: u16,
    /// 畫法字母，見 [`Kind`]。
    pub kind: u16,
    /// `P`：死亡時掉出 `bullet` 字母的道具。
    pub carrier: u16,
    /// 動畫上限表指標：9 個字組，0 表示沒有動畫。
    pub anim_limits: u16,
    /// 動畫計數，樣板為 0。
    pub anim: u16,
    /// 這個敵人自己的亂數表索引，樣板為 0。
    pub rand_index: u16,
    /// 樣板只有 23 個字組，複製 24 個字組多帶進來的下一個樣板的 `hp`，沒有任何指令讀。
    pub spill: u16,
}

/// 池指標：第 `slot` 槽記錄的記號，0 是空槽。記號只在遊戲裡用，不是原版的記錄位址。
pub const fn enemy_record_link(slot: usize) -> u16 {
    slot as u16 + 1
}

impl Enemy {
    /// 記錄的欄位順序（原版 24 個字組）：`Enemy` 的欄位依序對應 `w0` 至 `w23`。樣板表與工作區同一版面。
    pub fn from_words(w: [u16; 24]) -> Enemy {
        Enemy {
            hp: w[0],
            frames: w[1],
            blit_x: w[2],
            blit_y: w[3],
            blit_dx: w[4],
            blit_dy: w[5],
            fire_threshold: w[6],
            bullet: w[7],
            mode: w[8],
            col: w[9],
            row: w[10],
            dcol: w[11],
            drow: w[12],
            script: w[13],
            shape: w[14],
            spare: w[15],
            damage: w[16],
            explosion: w[17],
            kind: w[18],
            carrier: w[19],
            anim_limits: w[20],
            anim: w[21],
            rand_index: w[22],
            spill: w[23],
        }
    }

    pub fn to_words(&self) -> [u16; 24] {
        [
            self.hp,
            self.frames,
            self.blit_x,
            self.blit_y,
            self.blit_dx,
            self.blit_dy,
            self.fire_threshold,
            self.bullet,
            self.mode,
            self.col,
            self.row,
            self.dcol,
            self.drow,
            self.script,
            self.shape,
            self.spare,
            self.damage,
            self.explosion,
            self.kind,
            self.carrier,
            self.anim_limits,
            self.anim,
            self.rand_index,
            self.spill,
        ]
    }

    pub(crate) fn mode(&self) -> Mode {
        Mode::from_word(self.mode)
    }

    pub(crate) fn kind(&self) -> Kind {
        Kind::from_word(self.kind)
    }

    /// 往上：y 與列各減一步（原版 `sub_160B4`；碼 09 用的 `sub_160DE` 是同一個運算）。
    fn move_up(&mut self) {
        self.blit_y = self.blit_y.wrapping_sub(self.blit_dy);
        self.row = self.row.wrapping_sub(self.drow);
    }

    /// 往下（原版 `sub_160C9`、`sub_160F3`）。
    fn move_down(&mut self) {
        self.blit_y = self.blit_y.wrapping_add(self.blit_dy);
        self.row = self.row.wrapping_add(self.drow);
    }

    /// 往左（原版 `sub_16108`）。
    fn move_left(&mut self) {
        self.blit_x = self.blit_x.wrapping_sub(self.blit_dx);
        self.col = self.col.wrapping_sub(self.dcol);
    }

    /// 往右（原版 `sub_1611D`）。
    fn move_right(&mut self) {
        self.blit_x = self.blit_x.wrapping_add(self.blit_dx);
        self.col = self.col.wrapping_add(self.dcol);
    }

    /// 走一步。斜向同一幀做兩個軸（原版 `sub_15FD9` 的分支）。
    fn step(&mut self, heading: Heading) {
        match heading {
            Heading::N => self.move_up(),
            Heading::NE => {
                self.move_up();
                self.move_right();
            }
            Heading::E => self.move_right(),
            Heading::SE => {
                self.move_down();
                self.move_right();
            }
            Heading::S => self.move_down(),
            Heading::SW => {
                self.move_down();
                self.move_left();
            }
            Heading::W => self.move_left(),
            Heading::NW => {
                self.move_left();
                self.move_up();
            }
        }
    }

    /// 碼 0A：不管單步多大，x 與欄各加 6（原版 `sub_16132`）。
    fn jump_right(&mut self) {
        self.blit_x = self.blit_x.wrapping_add(JUMP_COLS);
        self.col = self.col.wrapping_add(JUMP_COLS);
    }

    /// 隨景物左移 1 欄（原版 `sub_15F3D`）。
    fn drift_with_scenery(&mut self) {
        self.blit_x = self.blit_x.wrapping_sub(1);
        self.col = self.col.wrapping_sub(1);
    }
}

/// 一幀要畫的內容：圖庫索引、位置與畫法字母。每個敵人更新時產生，畫的時候照著畫。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sprite {
    /// 圖庫索引。
    pub image: u16,
    /// 欄，位元組。
    pub x: u16,
    /// 像素列。
    pub y: u16,
    /// 畫法字母，見 [`Kind`]。
    pub kind: u16,
}

/// 戰場加大（規格 005「戰場大小」）時，腳本結束的敵人「飛出畫面」的狀態。
///
/// 腳本的長度是原版的：敵人從新右緣進場，走同樣長的路，腳本結束時還在畫面裡，會憑空消失。
/// 所以寬戰場裡，腳本結束時還有一部分在左右界之內、而且最後一個移動碼有水平分量的敵人（或隨景物左移的敵人），
/// 沿著最後的方向繼續飛，整個形狀離開左右界才釋放。不屬於原版狀態：欄數 40 時完全不用，原版照舊在腳本結束那一幀釋放。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Linger {
    /// 每槽最後一次執行的方向碼（1 至 8），沒有是 0。生成時清掉。
    last: [u8; POOL_SIZE],
    /// 每槽還能飛的幀數，0 是沒有在飛。上限防止水平步長是 0 的敵人永遠不釋放。
    left: [u16; POOL_SIZE],
}

/// 飛出畫面的幀數上限：橫越一次戰場再加這麼多。
const LINGER_SLACK: u16 = 16;

/// 跟著景物捲動的敵人（規格 005「戰場大小」）。它們的位置是對著景物設計的：在原版生成幀出現在畫面右緣（欄 40），
/// 也就是地圖的第 `左緣欄 + 40` 欄，之後的腳本一路和景物同步。戰場加大時如果把它平移到新右緣，就偏離地圖多出來的欄數，
/// 地面敵人浮在缺口上方，所以改成提前生成、先隨景物移動到原版的位置。
///
/// 兩種速度是從程式碼查到的：
///
/// | 種類 | 景物速度 | 怎麼認 |
/// |---|---|---|
/// | [`Pace::Drift`] | 每幀 1 欄 | 模式 `Y`（`drift_with_scenery`）。第二關的柱（`draw_stage2_pillars`）也是每幀 1 欄 |
/// | [`Pace::Map`] | 每兩幀 1 欄 | 第三關時間表裡腳本開頭是「往左一步、停一步」四組以上的列（地圖每兩個頁面捲 1 欄，見 `stage3/map.rs`） |
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pace {
    Drift,
    Map,
}

impl Pace {
    /// 景物左移 1 欄要幾幀。
    const fn frames_per_col(self) -> u16 {
        match self {
            Pace::Drift => 1,
            Pace::Map => 2,
        }
    }
}

/// 提前生成的敵人在原版生成幀之前的狀態。不屬於原版狀態：欄數 40 時完全不用。
///
/// 欄數大於 40 時，景物綁定、x 在 40 以上的列在原版生成幀的 `lead` 幀之前（`lead` = 多出來的欄數 × 景物走 1 欄的幀數，
/// 到第 0 幀為止）生成在新右緣。這段時間只隨景物移動：不讀腳本、不擲亂數、不開火、不吃 AI 計時，但有形狀、可以被打中、
/// 也可以撞玩家（寬戰場裡這一段在畫面裡）。到原版生成幀它剛好在樣板的欄，從那一幀起照原版的腳本走，之後和 100% 完全相同。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Approach {
    /// 距離原版生成幀還有幾幀（包含這一幀），0 是沒有在提前移動。
    left: [u16; POOL_SIZE],
    /// 樣板的欄，也就是原版生成幀那一幀的欄。
    home: [u16; POOL_SIZE],
    /// 地圖速度的敵人釘住的地圖欄：樣板的欄加上原版生成幀前一幀的地圖左緣欄。每幀的欄是它減地圖左緣欄。
    /// 速度是 [`Pace::Drift`] 的敵人是 `None`。
    anchor: [Option<u16>; POOL_SIZE],
    /// 這一槽是景物綁定的敵人（含畫面內生成、不提前的）。它們的軌跡是對著景物設計的，腳本結束照原版釋放，
    /// 不套用 [`Linger`]（那是給自由飛行的敵人補上多出來的橫向距離）。
    bound: [bool; POOL_SIZE],
}

/// 生成時敵人放在哪裡。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Placement {
    /// 自由飛行：樣板的 x 依 [`Core::spawn_col`] 平移或放大。
    Free { boss: bool },
    /// 景物綁定、畫面內生成（x 小於 40）：x 不動，景物在各寬度的同一欄是同一個位置。
    Keep,
    /// 景物綁定、x 在 40 以上：提前 `left` 幀生成，`frame` 是現在的幀計數。
    Approach { pace: Pace, left: u16, frame: u16 },
}

/// 池的一槽。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    /// 池指標：0 是空槽，使用中是這一槽記錄的記號（[`enemy_record_link`]）。
    pub link: u16,
    pub enemy: Enemy,
    /// 這一槽上次更新產生的繪製值。釋放的槽保留舊值，沒有人讀。
    pub sprite: Sprite,
    /// 本幀被打中閃白：`Y` 閃，`N` 不閃。
    pub flash: u16,
}

impl Slot {
    pub(crate) fn is_active(&self) -> bool {
        self.link != 0
    }

    fn is_flashing(&self) -> bool {
        self.flash == YES
    }
}

/// 敵人領域的全部狀態。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Enemies {
    pub slots: [Slot; POOL_SIZE],
    /// 池指標表的第 16 個字組，只被關卡初始化清零，沒有人讀。
    pub spare_link: u16,
    /// 最近一個更新的敵人本幀要不要釋放（原版 `word_1EDAA`）：`Y`（0059h）釋放，`N`（004Eh）不釋放。
    /// 列起點表（G3、G4 共用）的越界列讀的就是這個字組，見 `Core::set_enemy_death_flag`。
    pub death_flag: u16,
    /// 傷害寫到指標 0 加 20h 的那個字組（原版 `dseg:0020h`，啟動時文字畫面的色彩屬性，關卡內不再讀），
    /// 見 [`Core::add_entity_damage`]。
    pub stray_damage: u16,
    /// 最近一個做完整幀的敵人產生的繪製值（原版 `word_1EE4C` 起 4 個字組）。
    ///
    /// 更新一個敵人時，被打死的那一幀在產生繪製值之前就結束，但原版仍把這 4 個字組複製進它的槽，
    /// 所以死掉的槽拿到的是前一個敵人（可能是前一幀）的繪製值。那個槽立刻釋放，沒有人畫它，
    /// 但資料段比對看得到，所以保留。
    pub last_sprite: Sprite,
}

/// 池指標表一項的內容（見 [`Enemies::link_at`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntityLink {
    /// 使用中的槽。
    Slot(usize),
    /// 不是記錄位址的字組值：空槽的 0、備用字組、記錄區的字組。
    Stray(u16),
}

impl Enemies {
    fn first_free(&self) -> Option<usize> {
        self.slots.iter().position(|s| !s.is_active())
    }

    /// 關卡初始化：全部槽釋放（只清指標，記錄不動），備用的第 16 個字組一併清掉。
    pub(crate) fn release_all(&mut self) {
        for s in &mut self.slots {
            s.link = 0;
        }
        self.spare_link = 0;
    }

    /// 池指標表第 `index` 項。原版沒有範圍檢查，表有 16 個字組（15 槽加備用），之後緊接記錄區，
    /// 所以索引 16 起讀到的是槽 0 記錄的 `w0`，每 24 項換一槽。`u8` 的索引最多到槽 9 的 `w23`。
    /// 玩家彈碰到裝甲格（格值 FEh）時索引是 FDh，讀到槽 9 的 `anim`。
    ///
    /// 使用中的槽是 [`EntityLink::Slot`]；其他都是原版讀到的字組值（空槽的 0、備用字組、記錄區的字組），
    /// 沒有一個是記錄位址，傷害字組寫在它加 20h 的地方，見 [`Core::add_entity_damage`]。
    pub(crate) fn link_at(&self, index: u8) -> EntityLink {
        let i = usize::from(index);
        if i < POOL_SIZE {
            if self.slots[i].is_active() { EntityLink::Slot(i) } else { EntityLink::Stray(0) }
        } else if i == POOL_SIZE {
            EntityLink::Stray(self.spare_link)
        } else {
            let word = i - POOL_SIZE - 1;
            EntityLink::Stray(self.slots[word / RECORD_WORDS].enemy.to_words()[word % RECORD_WORDS])
        }
    }

    /// 原版 [`Enemies::link_at`] 讀到的字組值：使用中的槽是記錄的記號。測試用。
    #[cfg(test)]
    pub(crate) fn word_at(&self, index: u8) -> u16 {
        match self.link_at(index) {
            EntityLink::Slot(slot) => enemy_record_link(slot),
            EntityLink::Stray(word) => word,
        }
    }
}

/// 生成用的時間表：每列 `(幀, 樣板)`，幀計數相符就生成一個敵人，幀為 0 結束。表的內容在 `tables.rs`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Timetable {
    /// 第一關，56 列。
    Stage1,
    /// 第二關，90 列。
    Stage2,
    /// 第三關，85 列。
    Stage3,
    /// 第四關，111 列。
    Stage4,
    /// 片尾，3 列：END 字樣。
    Ending,
}

impl Timetable {
    /// 這一列是頭目的分件嗎（戰場加大時靠右對齊，見 [`Core::spawn_col`]）。位置是 `entities.json` 的時間表逐列對過的：
    ///
    /// | 關 | 頭目列 |
    /// |---|---|
    /// | 第一關 | 幀 623（計數超過 26Ch 的列：合成圖 59 欄的隱形本體、六個嵌在合成圖上的分件、核心） |
    /// | 第二關 | 幀 263、264（中頭目第一層）、347、348（第二層）、630（頭目） |
    /// | 第三關 | 幀 1133（計數超過 469h 的列：六個砲台與核心） |
    /// | 第四關 | 幀 1060（計數超過 3E8h 的列：圓盤） |
    pub(crate) fn is_boss_row(self, frame: u16) -> bool {
        match self {
            Timetable::Stage1 => frame > 0x26C,
            Timetable::Stage2 => matches!(frame, 263 | 264 | 347 | 348 | 630),
            Timetable::Stage3 => frame > 0x469,
            Timetable::Stage4 => frame > 0x3E8,
            Timetable::Ending => false,
        }
    }
}

/// 腳本一幀的內容。1 至 8 是八個方向（編號同 [`Heading`]），其餘照原版的派發表（`sub_15FD9`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Motion {
    /// 00 與不在派發表內的碼（含 0C、0E、10 以上，E0 與 F0 讀出控制碼後也算）：不動。
    Hold,
    /// 01 至 08：往某個方向走一步。
    Go(Heading),
    /// 09：追玩家 1 的列。
    ChaseRow,
    /// 0A：右跳 6 欄。
    JumpRight,
    /// 0B：朝玩家發射 `B`。
    FireBurst,
    /// 0D：依最近玩家的方向決定上或下。
    SeekRow,
    /// 0F：腳本結束，本幀走完後釋放。
    Die,
}

impl Motion {
    pub(crate) fn from_code(code: u8) -> Motion {
        match code {
            CODE_CHASE_ROW => Motion::ChaseRow,
            CODE_JUMP_RIGHT => Motion::JumpRight,
            CODE_FIRE_BURST => Motion::FireBurst,
            CODE_SEEK_ROW => Motion::SeekRow,
            CODE_DIE => Motion::Die,
            _ => heading_from_code(code).map_or(Motion::Hold, Motion::Go),
        }
    }
}

/// 移動碼 1 至 8 對應的方向。
fn heading_from_code(code: u8) -> Option<Heading> {
    Some(match code {
        1 => Heading::N,
        2 => Heading::NE,
        3 => Heading::E,
        4 => Heading::SE,
        5 => Heading::S,
        6 => Heading::SW,
        7 => Heading::W,
        8 => Heading::NW,
        _ => return None,
    })
}

/// 上一幀的傷害結算結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Damage {
    /// 沒有被打中。
    None,
    /// 被打中但還沒死，本幀閃白。
    Flash,
    /// 累計傷害超過門檻，已播爆炸特效。
    Killed,
}

/// 一個敵人一幀的結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Step {
    flashing: bool,
    /// 本幀結束後要釋放：被打死，或腳本走到結束碼。
    dying: bool,
}

impl Core {
    // ---- 生成 ----

    /// 掃一張時間表，幀計數相符的每一列生成一個敵人（原版 `sub_15D1C`）。掃描不依賴排序，也不跳過；
    /// 池滿了該列放棄，後面的列照常處理。
    ///
    /// 戰場加大時，景物綁定的列（[`Core::scenery_pace`]）另外處理：x 在 40 以上的提前生成（[`Approach`]），
    /// 畫面內生成的 x 不放大。
    pub(crate) fn spawn_scheduled(&mut self, table: Timetable) {
        let tables = self.tables();
        let frame = self.stage_frame();
        let extra = self.field_extra();
        for &(row_frame, template) in tables.timetable(table) {
            let boss = table.is_boss_row(row_frame);
            // 景物綁定的列最多提前 2 × 多出來的欄數幀（地圖速度）生成。
            if extra > 0 && !boss && row_frame >= frame && row_frame - frame <= 2 * extra {
                if let Some(pace) = self.scenery_pace(table, template) {
                    self.spawn_bound(template, pace, row_frame, frame);
                    continue;
                }
            }
            if row_frame == frame && !self.spawn_row(template, boss) && !boss && self.scenery_pace(table, template).is_some() {
                // 量測用：景物綁定的列因為池滿放棄（100% 也算，才能和寬戰場比）。
                self.port.borrow_mut().bound_drops += 1;
            }
        }
    }

    /// 這個樣板是不是跟著景物捲動，是的話景物的速度（[`Pace`]）。頭目列不算，由呼叫端先排除。
    fn scenery_pace(&self, table: Timetable, template: u16) -> Option<Pace> {
        let enemy = self.enemy_template(template);
        if enemy.mode() == Mode::Scroll {
            return Some(Pace::Drift);
        }
        (table == Timetable::Stage3 && self.script_tracks_map(enemy.script)).then_some(Pace::Map)
    }

    /// 腳本開頭是「往左一步、停一步」的交替（任一相位）四組：每兩幀左移 1 欄，和第三關的地圖捲速相同。
    fn script_tracks_map(&self, at: u16) -> bool {
        const W: u8 = 7;
        const HOLD: u8 = 0;
        let bytes: Vec<Option<u8>> = (0..8).map(|k| self.tables.script_byte_opt(at.wrapping_add(k))).collect();
        [(W, HOLD), (HOLD, W)].iter().any(|&(a, b)| bytes.iter().enumerate().all(|(k, &v)| v == Some(if k % 2 == 0 { a } else { b })))
    }

    /// 景物綁定的列在現在這一幀要不要生成（`row_frame` 是原版的生成幀，`frame` 是現在的幀）。
    ///
    /// - x 在 40 以下（畫面內生成）：原版生成幀照原樣生成，x 不放大。
    /// - x 在 40 以上：提前 `lead` 幀生成在新右緣，`lead` 是景物走完多出來的欄數的幀數，不夠就從第 0 幀起。
    fn spawn_bound(&mut self, template: u16, pace: Pace, row_frame: u16, frame: u16) {
        let home = self.enemy_template(template).col;
        let placement = if (home as i16) < GRID_COLS as i16 {
            if row_frame != frame {
                return;
            }
            Placement::Keep
        } else {
            let start = row_frame.saturating_sub(self.field_extra() * pace.frames_per_col());
            if start != frame {
                return;
            }
            let left = row_frame - start;
            if left == 0 { Placement::Free { boss: false } } else { Placement::Approach { pace, left, frame } }
        };
        if !self.spawn_with(template, placement) {
            self.port.borrow_mut().bound_drops += 1;
        }
    }

    /// 戰場加大時（規格 005「戰場大小」）生成列的 x 平移，欄數 40 時原樣回傳：
    ///
    /// | 樣板的 x（欄） | 戰場加大時 |
    /// |---|---|
    /// | 40 以上（畫面右緣，或右緣之外交錯進場） | 加上多出來的欄數，整體平移到新右緣 |
    /// | 頭目列（見 [`Timetable::is_boss_row`]），含 x 為負的合成圖分件 | 同上，和頭目合成圖（`restart_boss_layer`）一起靠右對齊 |
    /// | 0 至 39 與負的其他列（畫面內、上下方與左緣進場） | x 乘 `cols / 40`（整數除法，往零取整） |
    ///
    /// 這是自由飛行的敵人的規則。跟著景物捲動的列（[`Core::scenery_pace`]）不走這裡：x 在 40 以上的提前生成
    /// （[`Approach`]），畫面內生成的 x 不放大，它們在各寬度站在同一個地圖欄上。
    ///
    /// 時間表的幀與腳本內的相對位移不改，所以自由飛行的敵人進場距離變遠，到達玩家面前的時間會晚。
    pub(crate) fn spawn_col(&self, col: u16, boss: bool) -> u16 {
        let extra = self.field_extra();
        if extra == 0 {
            return col;
        }
        let x = col as i16;
        if boss || x >= GRID_COLS as i16 {
            x.wrapping_add(extra as i16) as u16
        } else {
            (i32::from(x) * i32::from(self.field_cols()) / i32::from(GRID_COLS)) as u16
        }
    }

    /// 把樣板複製進第一個空槽（原版 `sub_15D41`），繪製位置跟著邏輯位置設：x 取欄，y 取列乘 10。
    /// 沒有空槽就放棄。傷害字組、動畫計數與亂數索引都是樣板的值，所以每個敵人的亂數序列從頭開始。
    #[cfg(test)]
    pub(crate) fn spawn_enemy(&mut self, template: u16) {
        self.spawn_row(template, false);
    }

    /// [`Core::spawn_enemy`]，`boss` 是時間表的這一列屬於頭目（戰場加大時 x 的平移規則見 [`Core::spawn_col`]）。
    pub(crate) fn spawn_row(&mut self, template: u16, boss: bool) -> bool {
        self.spawn_with(template, Placement::Free { boss })
    }

    /// 生成的共同部分：`placement` 決定欄，其餘照樣板。回傳有沒有放進池裡（沒有空槽就放棄，回傳 `false`）。
    fn spawn_with(&mut self, template: u16, placement: Placement) -> bool {
        let Some(slot) = self.en.first_free() else {
            self.port.borrow_mut().spawn_drops += 1;
            return false;
        };
        let mut enemy = self.enemy_template(template);
        let home = enemy.col;
        self.approach.left[slot] = 0;
        self.approach.bound[slot] = !matches!(placement, Placement::Free { .. });
        enemy.col = match placement {
            Placement::Free { boss } => self.spawn_col(home, boss),
            Placement::Keep => home,
            Placement::Approach { pace, left, frame } => {
                // 這一幀（動作之後）的欄：原版生成幀的前一幀剛好在樣板的欄，往前推到現在。
                let col = match pace {
                    Pace::Drift => home + left - 1,
                    // 地圖左緣每兩幀加 1，偶數幀加（第二頁）：從現在到生成幀的前一幀加了幾欄。
                    Pace::Map => home + (frame + left - 1) / 2 - frame / 2,
                };
                self.approach.left[slot] = left;
                self.approach.home[slot] = home;
                self.approach.anchor[slot] = (pace == Pace::Map).then(|| self.scenes.map.left_col.wrapping_add(col));
                col
            }
        };
        enemy.blit_x = enemy.col;
        enemy.blit_y = enemy.row.wrapping_mul(ROW_PIXELS);
        let s = &mut self.en.slots[slot];
        s.link = enemy_record_link(slot);
        s.enemy = enemy;
        self.linger.last[slot] = 0;
        self.linger.left[slot] = 0;
        self.bump_gen(Pool::Entity, slot as u8);
        // AI 強化只認得生成表生成的敵人（`ai.rs`）；這一步不碰資料段。
        self.ai.on_spawn(slot, template);
        true
    }

    // ---- 每幀更新 ----

    /// 每幀更新 15 個敵人（原版 `sub_15D8C`）：每個使用中的槽走一步，存繪製值，必要時釋放。
    ///
    /// 原版先把記錄複製進工作區、更新、複製回去；這裡複製成區域變數，結果相同。
    /// 池指標指向的就是這一槽自己的記錄（生成時寫的），不會指到別處。
    pub(crate) fn update_enemies(&mut self) {
        self.ai_begin_frame();
        for slot in 0..POOL_SIZE {
            if !self.en.slots[slot].is_active() {
                continue;
            }
            let mut enemy = self.en.slots[slot].enemy;
            let step = self.step_enemy(&mut enemy, slot);
            self.set_enemy_death_flag(step.dying);
            let sprite = self.en.last_sprite;
            let s = &mut self.en.slots[slot];
            s.enemy = enemy;
            s.sprite = sprite;
            s.flash = if step.flashing { YES } else { NO };
            if step.dying {
                self.bury_enemy(slot, &enemy);
            }
        }
    }

    /// 一個敵人的一幀（原版 `sub_15E68`）。
    ///
    /// 順序：結算傷害（死了就到此為止）、取移動碼、移動、隨景物左移、挑圖框、產生繪製值、寫足跡、擲亂數開火。
    /// 腳本結束碼（0Fh）那一幀不同：只標記要釋放，其餘步驟照常做，所以那一幀仍寫足跡、仍可開火、仍推進亂數。
    fn step_enemy(&mut self, enemy: &mut Enemy, slot: usize) -> Step {
        // 戰場加大：景物綁定的敵人在原版生成幀之前只隨景物移動（[`Approach`]）。
        let approaching = self.approach.left[slot];
        if approaching > 0 {
            self.approach.left[slot] = approaching - 1;
            return self.approach_frame(enemy, slot, approaching);
        }
        let flashing = match self.resolve_damage(enemy) {
            Damage::Killed => return Step { flashing: false, dying: true },
            Damage::Flash => true,
            Damage::None => false,
        };
        let rules = self.rules();
        self.ai.tick_enemy(slot);
        let lingering = self.linger.left[slot] > 0;
        let code = match enemy.mode() {
            Mode::Chase => {
                let target = self.aim_heading(enemy.col, enemy.row);
                enemy.script = turn_toward(target, enemy.script);
                enemy.script as u8
            }
            // 戰場加大：腳本結束後飛出畫面的敵人不再讀腳本，沿最後的方向走。
            Mode::Script | Mode::Scroll if lingering => self.linger.last[slot],
            Mode::Script | Mode::Scroll => {
                let code = self.read_command(enemy);
                // 敵人會反應（`ai_react`）：只換這一幀要執行的移動碼，腳本照常前進。
                if rules.ai_react { self.ai_react(enemy, slot, code) } else { code }
            }
        };
        let mut dying = self.apply_motion(enemy, code);
        if self.field_extra() > 0 {
            dying = self.linger_after_motion(enemy, slot, code, dying, lingering);
        }
        if enemy.mode() == Mode::Scroll {
            enemy.drift_with_scenery();
        }
        let image = self.select_frame(enemy, code);
        self.en.last_sprite = Sprite { image, x: enemy.blit_x, y: enemy.blit_y, kind: enemy.kind };
        self.stamp_footprint_g3(enemy);
        self.stamp_footprint_g1(enemy, slot);
        if self.rolls_to_fire(enemy) {
            self.fire_enemy_bullets(enemy.bullet, enemy.col, enemy.row);
        } else if rules.ai_fire {
            // 火力加強（`ai_fire`）：原版這一幀沒開火的敵人另有一次機會，用 AI 自己的亂數，不碰原版的亂數表索引。
            self.ai_extra_fire(enemy, slot);
        }
        if rules.ai_boss {
            self.ai_boss_step(enemy, slot);
        }
        Step { flashing, dying }
    }

    /// 提前生成的敵人的一幀（[`Approach`]）：結算傷害、隨景物移到這一幀的欄、挑圖框、寫足跡。
    /// 不讀腳本、不推動畫計數、不擲亂數、不開火、不走 AI。`left` 是距離原版生成幀的幀數（包含這一幀），
    /// 這一幀動作之後的欄是原版生成幀前一幀剛好在樣板的欄往前推 `left - 1` 幀的景物位移。
    fn approach_frame(&mut self, enemy: &mut Enemy, slot: usize, left: u16) -> Step {
        let flashing = match self.resolve_damage(enemy) {
            Damage::Killed => return Step { flashing: false, dying: true },
            Damage::Flash => true,
            Damage::None => false,
        };
        let home = self.approach.home[slot];
        let col = match self.approach.anchor[slot] {
            Some(anchor) => anchor.wrapping_sub(self.scenes.map.left_col),
            None => home + left - 1,
        };
        enemy.col = col;
        enemy.blit_x = col;
        // 圖框用腳本第一個碼的那一張（原版生成幀那一幀會畫的），動畫計數不動。
        let code = self.tables.script_byte_opt(enemy.script).unwrap_or(0);
        let framed = u16::from(code) <= LAST_FRAMED_CODE;
        let image = if framed {
            self.tables.table_word(enemy.frames, u16::from(code) << 1).wrapping_add(enemy.anim)
        } else {
            self.tables.table_word(enemy.frames, 0)
        };
        self.en.last_sprite = Sprite { image, x: enemy.blit_x, y: enemy.blit_y, kind: enemy.kind };
        self.stamp_footprint_g3(enemy);
        self.stamp_footprint_g1(enemy, slot);
        Step { flashing, dying: false }
    }

    /// 戰場加大時敵人這一幀動作之後的處理（[`Linger`]），回傳這一幀要不要釋放。
    ///
    /// - 飛出畫面中：幀數減一，整個形狀離開左右界或幀數用完才釋放。
    /// - 記下最後一次的方向碼。腳本結束（`ended`）時，如果還有一部分在左右界之內，而且最後的方向有水平分量
    ///   （或是隨景物左移的敵人），改成飛出畫面，這一幀不釋放。
    /// - 不套用在 `K` 型（死後要把損壞貼片畫進背景）與子彈字母 `E`（死亡就是過關旗標）的敵人。
    fn linger_after_motion(&mut self, enemy: &Enemy, slot: usize, code: u8, ended: bool, lingering: bool) -> bool {
        if self.approach.bound[slot] {
            return ended;
        }
        if lingering {
            self.linger.left[slot] -= 1;
            return self.linger.left[slot] == 0 || !self.overlaps_field_columns(enemy);
        }
        if heading_from_code(code).is_some() {
            self.linger.last[slot] = code;
        }
        if !ended || enemy.kind() == Kind::Embedded || enemy.bullet == u16::from(b'E') || !self.overlaps_field_columns(enemy) {
            return ended;
        }
        let horizontal = matches!(heading_from_code(self.linger.last[slot]), Some(Heading::NE | Heading::E | Heading::SE | Heading::SW | Heading::W | Heading::NW));
        if horizontal || enemy.mode() == Mode::Scroll {
            self.linger.left[slot] = self.field_cols() + LINGER_SLACK;
            return false;
        }
        true
    }

    /// 敵人的形狀還有一部分在戰場的左右界之內（欄 0 至 `field_cols() - 1`，不看上下）。
    fn overlaps_field_columns(&self, enemy: &Enemy) -> bool {
        let width = self.tables().shape(enemy.shape).width as i16;
        let col = enemy.col as i16;
        col.wrapping_add(width.max(1)) > 0 && col < self.field_cols() as i16
    }

    /// 結算上一幀被打中的傷害（原版 `sub_15F8D`）。
    ///
    /// 傷害字組的高位元組是 1 才結算：先把它清成只剩低位元組（累計傷害，整個生命週期不歸零），
    /// 累計傷害不大於 `hp` 加難度加成就只閃白，否則死亡並在敵人位置播 `explosion` 特效。
    fn resolve_damage(&mut self, enemy: &mut Enemy) -> Damage {
        if (enemy.damage >> 8) as u8 != HIT_PENDING {
            return Damage::None;
        }
        enemy.damage &= 0x00FF;
        if enemy.damage <= enemy.hp.wrapping_add(self.hp_bonus()) {
            return Damage::Flash;
        }
        self.spawn_effect(EffectKind::from_letter(enemy.explosion), enemy.col, enemy.row);
        Damage::Killed
    }

    /// 取這一幀的移動碼（原版 `sub_15E79` 起的取位元組加 `sub_15ECD`）。讀腳本目前的位元組，讀取位置前進；
    /// 控制碼不進移動表，這裡處理完回傳實際要執行的碼：
    ///
    /// - `F0` 跳轉：後接位址，讀取位置改成位址加 1，碼取該處的位元組。
    /// - `E0` 變身：後接樣板位址，見 [`Core::morph_enemy`]。
    ///
    /// 跳轉或變身後讀到的碼就算又是 `F0` 或 `E0` 也不再處理，當作不在移動表內（不動）。
    pub(crate) fn read_command(&self, enemy: &mut Enemy) -> u8 {
        let at = enemy.script;
        enemy.script = at.wrapping_add(1);
        let code = self.tables.script_byte(at);
        match code {
            SCRIPT_JUMP => {
                let target = self.tables.script_word(at.wrapping_add(1));
                enemy.script = target.wrapping_add(1);
                self.tables.script_byte(target)
            }
            SCRIPT_MORPH => self.morph_enemy(enemy, self.tables.script_word(at.wrapping_add(1))),
            _ => code,
        }
    }

    /// 變身（腳本 `E0`，原版 `15EE8h`）：整筆記錄換成新樣板的 24 個字組，只保留目前的位置（欄、列）與死亡特效字母；
    /// 繪製位置照新位置重算。耐久、單步、動畫計數、亂數索引都是新樣板的值，傷害字組也被洗掉。
    /// 回傳新腳本的第一個移動碼。
    fn morph_enemy(&self, enemy: &mut Enemy, template: u16) -> u8 {
        let (col, row, explosion) = (enemy.col, enemy.row, enemy.explosion);
        *enemy = self.enemy_template(template);
        enemy.col = col;
        enemy.row = row;
        enemy.explosion = explosion;
        enemy.blit_x = col;
        enemy.blit_y = row.wrapping_mul(ROW_PIXELS);
        let at = enemy.script;
        enemy.script = at.wrapping_add(1);
        self.tables.script_byte(at)
    }

    /// 執行一個移動碼（原版 `sub_15FD9`）。回傳 `true` 表示腳本結束（碼 0Fh）。
    pub(crate) fn apply_motion(&mut self, enemy: &mut Enemy, code: u8) -> bool {
        match Motion::from_code(code) {
            Motion::Hold => {}
            Motion::Go(heading) => enemy.step(heading),
            Motion::ChaseRow => {
                let heading = self.chase_row(enemy);
                enemy.step(heading);
            }
            Motion::SeekRow => {
                let heading = self.seek_row(enemy);
                enemy.step(heading);
            }
            Motion::JumpRight => enemy.jump_right(),
            Motion::FireBurst => {
                self.fire_enemy_bullets(u16::from(b'B'), enemy.col, enemy.row.wrapping_add(MUZZLE_ROWS));
            }
            Motion::Die => return true,
        }
        false
    }

    /// 碼 0D（原版 `sub_1606A`）：求 (欄 + 3, 列 + 4) 到最近玩家的朝向，朝向 4 至 6（下方）取下，其餘取上。
    /// 目標列在螢幕最上方或之外（列 + 4 小於等於 0）一律取下，超過 14h 一律取上，這兩種不呼叫瞄準。
    pub(crate) fn seek_row(&self, enemy: &Enemy) -> Heading {
        let col = enemy.col.wrapping_add(SEEK_OFFSET_COLS);
        let row = enemy.row.wrapping_add(SEEK_OFFSET_ROWS);
        if (row as i16) <= SEEK_ROW_BOTTOM {
            return Heading::S;
        }
        if (row as i16) > SEEK_ROW_TOP {
            return Heading::N;
        }
        match self.aim_heading(col, row) {
            Heading::SE | Heading::S | Heading::SW => Heading::S,
            _ => Heading::N,
        }
    }

    /// 碼 09（原版 `sub_1609C`）：列大於 18h 或小於玩家 1 的列就往下，否則往上。不看玩家 1 在不在。
    pub(crate) fn chase_row(&self, enemy: &Enemy) -> Heading {
        let (_, player_row) = self.player_cell(0);
        if enemy.row > CHASE_ROW_LIMIT || enemy.row < player_row { Heading::S } else { Heading::N }
    }

    /// 挑這一幀的圖框，並推進動畫計數（原版 `sub_15F4C`）。
    ///
    /// 碼 0 至 8 取圖框表第 `碼` 項加動畫計數，其餘碼取第 0 項。有動畫上限表時，圖框小於上限計數加 1，否則歸零。
    /// 上限表的索引在碼大於 8 時不乘 2，以位元組偏移 `碼` 讀字組，位置不對齊，照原版。
    fn select_frame(&self, enemy: &mut Enemy, code: u8) -> u16 {
        let code = u16::from(code);
        let framed = code <= LAST_FRAMED_CODE;
        let offset = if framed { code << 1 } else { code };
        let image = if framed {
            self.tables.table_word(enemy.frames, offset).wrapping_add(enemy.anim)
        } else {
            self.tables.table_word(enemy.frames, 0)
        };
        if enemy.anim_limits != 0 {
            let limit = self.tables.table_word(enemy.anim_limits, offset);
            enemy.anim = if image < limit { enemy.anim.wrapping_add(1) } else { 0 };
        }
        image
    }

    /// 擲亂數決定這一幀要不要開火（原版 `sub_1617A`）。
    ///
    /// 每個敵人用自己的亂數索引，先推進再判斷，所以門檻是 [`NEVER_FIRES`] 的敵人索引每幀照樣前進。
    /// 亂數值加難度加成（16 位元）大於門檻才開火。
    pub(crate) fn rolls_to_fire(&self, enemy: &mut Enemy) -> bool {
        let mut next = enemy.rand_index.wrapping_add(1);
        if next >= FIRE_ROLLS {
            next = 0;
        }
        enemy.rand_index = next;
        if enemy.fire_threshold == NEVER_FIRES {
            return false;
        }
        self.fire_roll_at(next).wrapping_add(self.fire_bonus()) > enemy.fire_threshold
    }

    // ---- 足跡 ----

    /// 逐格走訪敵人形狀的足跡（原版 `sub_161B0`、`sub_16233` 共同的走法），對每格呼叫 `visit(核心, 列, 欄, 格值)`。
    ///
    /// 形狀是 `寬, 高, 寬 × 高 位元組`，列優先，左上角在 (`col`, `row`)。列為負的部分整列跳過（讀取位置跟著前進），
    /// 列 13h 起不走（有號比較，所以整個形狀在 13h 以下也是直接結束）。欄不在這裡裁，由各網格的寫入函式處理。
    ///
    /// 原版先把形狀寬寫進繪製的每列位元組殘值（`word_2687E`），下一個繪製命令會記下它，所以不論有沒有格可寫都要設。
    fn for_each_footprint_cell(&mut self, enemy: &Enemy, mut visit: impl FnMut(&mut Core, u16, u16, u8)) {
        let tables = self.tables();
        let shape = tables.shape(enemy.shape);
        let width = shape.width;
        let mut height = shape.height;
        self.set_shape_width(width);
        let mut src = 0u16;
        let mut row = enemy.row;
        while (row as i16) < 0 {
            row = row.wrapping_add(1);
            height = height.wrapping_sub(1);
            src = src.wrapping_add(width);
        }
        if (height as i16) <= 0 {
            return;
        }
        for _ in 0..height {
            if (row as i16) >= FOOTPRINT_ROW_LIMIT {
                return;
            }
            for k in 0..loop_count(width) {
                let v = shape.cell(src);
                visit(self, row, enemy.col.wrapping_add(k as u16), v);
                src = src.wrapping_add(1);
            }
            row = row.wrapping_add(1);
        }
    }

    /// 把足跡以 `or` 寫進 G3 髒格（原版 `sub_161B0`）。每格限定在自己那一列內，超出列右緣的略過，不溢進下一列。
    fn stamp_footprint_g3(&mut self, enemy: &Enemy) {
        self.for_each_footprint_cell(enemy, |core, row, col, v| {
            let first = core.grid3_cell_addr(row, 0);
            let at = core.grid3_cell_addr(row, col);
            core.grid3_or_in_row(first, at, v);
        });
    }

    /// 把足跡寫進 G1 實體格（原版 `sub_16233`）：格值 0 略過，裝甲與惰性格照寫，其餘加上槽號，覆蓋而不是 `or`。
    /// 玩家彈與砲座用「格值減 1」回推槽號。
    fn stamp_footprint_g1(&mut self, enemy: &Enemy, slot: usize) {
        self.for_each_footprint_cell(enemy, |core, row, col, v| {
            if v == 0 {
                return;
            }
            let cell = if v == ARMOR_CELL || v == INERT_CELL { v } else { v.wrapping_add(slot as u8) };
            core.grid1_put(row, col, cell);
        });
    }

    // ---- 死亡 ----

    /// 釋放本幀死亡的敵人（原版 `sub_15DBE`）：
    /// 子彈字母 `E` 的設過關旗標；釋放槽；`K` 型把損壞貼片畫進暫存區 B；否則載體 `P` 掉出 `bullet` 字母的道具。
    fn bury_enemy(&mut self, slot: usize, enemy: &Enemy) {
        if enemy.bullet == u16::from(b'E') {
            self.mark_stage_cleared();
        }
        self.en.slots[slot].link = 0;
        if enemy.kind() == Kind::Embedded {
            self.bake_wreck(enemy);
            return;
        }
        if enemy.carrier == u16::from(b'P') {
            self.fire_enemy_bullets(enemy.bullet, enemy.col, enemy.row.wrapping_add(MUZZLE_ROWS));
        }
    }

    /// `K` 型敵人死後留下的損壞貼片（原版 `15DFEh`）：圖框表指向 `(圖, x, y)` 三個字組，
    /// 以那張圖無遮罩畫進暫存區 B。畫進背景的緩衝，之後每幀的背景複製都帶著它，不屬於任何補間物件。
    /// 暫存區選擇留在 B，之後的繪製命令都帶這個值，直到有人再設（原版行為）。
    fn bake_wreck(&mut self, enemy: &Enemy) {
        let image = self.tables.table_word(enemy.frames, 0);
        let x = self.tables.table_word(enemy.frames, 2);
        let y = self.tables.table_word(enemy.frames, 4);
        self.set_blit_params(x, y, image);
        self.select_scratch(SCRATCH_B);
        self.clear_obj();
        self.draw_current_tile_to_scratch();
    }

    // ---- 繪製 ----

    /// 畫第一輪（原版 `sub_164BB`）：每個使用中的槽依序畫本體。
    pub(crate) fn draw_enemies(&mut self) {
        for slot in 0..POOL_SIZE {
            if self.en.slots[slot].is_active() {
                self.set_obj(Pool::Entity, slot as u8);
                self.draw_enemy_body(slot);
                self.clear_obj();
            }
        }
    }

    /// 畫一個敵人的本體（原版 `sub_164F3`）。`Hidden`、`Embedded`、`Overlay` 不在這一輪畫。
    /// 先以舞台圖庫畫遮罩；閃白幀接著只畫閃白遮罩，不畫平面；一般幀 y 加 1 再畫平面。
    fn draw_enemy_body(&mut self, slot: usize) {
        let Slot { sprite, .. } = self.en.slots[slot];
        self.set_blit_params(sprite.x, sprite.y, sprite.image);
        if Kind::from_word(sprite.kind) != Kind::Normal {
            return;
        }
        self.cut_stage();
        if self.en.slots[slot].is_flashing() {
            self.flash_stage();
            return;
        }
        self.set_blit_params(sprite.x, sprite.y.wrapping_add(1), sprite.image);
        self.overlay_stage();
    }

    /// 畫第二輪（原版 `sub_164D7`）：只有 `Overlay` 型會畫，在玩家之上。
    pub(crate) fn draw_enemy_overlays(&mut self) {
        for slot in 0..POOL_SIZE {
            if self.en.slots[slot].is_active() {
                self.set_obj(Pool::Entity, slot as u8);
                self.draw_enemy_overlay(slot);
                self.clear_obj();
            }
        }
    }

    /// 畫一個敵人的第二輪（原版 `sub_16541`）：`Overlay` 型以舞台圖庫無遮罩畫平面，覆寫。
    fn draw_enemy_overlay(&mut self, slot: usize) {
        let sprite = self.en.slots[slot].sprite;
        self.set_blit_params(sprite.x, sprite.y, sprite.image);
        if Kind::from_word(sprite.kind) == Kind::Overlay {
            self.overwrite_stage();
        }
    }

    // ---- 玩家彈與砲座命中 ----

    /// 池指標表第 `index` 項（槽號，G1 格值減 1）。0 表示該槽已釋放。見 [`Enemies::link_at`]。
    pub(crate) fn entity_link(&self, index: u8) -> EntityLink {
        self.en.link_at(index)
    }

    /// 玩家的攻擊打到 G1 格值為 `cell` 的格：對那個敵人累計 `damage` 點傷害（玩家彈命中 `16946h`，砲座接觸 `14C18h`、`15746h`）。
    /// 格值減 1 是槽號。
    ///
    /// 裝甲格 FEh 取到索引 FDh，讀實體指標表之後的 `dseg:248Eh`（實體槽 9 的 w21，實測全為 0），
    /// 於是把傷害字組寫到 `dseg:0020h`（啟動時文字畫面的色彩屬性，關卡內不再讀取）。指向已釋放槽的格值走同一路徑。
    /// `Fixes::armor_cell_index` 開啟時這兩種都不寫。玩家彈與砲座共用這一份判斷。
    pub(crate) fn damage_entity_at_cell(&mut self, cell: u8, damage: u8) {
        let index = cell.wrapping_sub(1);
        let link = self.entity_link(index);
        if self.fixes.armor_cell_index && (index == ARMOR_INDEX || link == EntityLink::Stray(0)) {
            return;
        }
        self.add_entity_damage(link, damage);
    }

    /// 對指標為 `link` 的敵人累計 `damage` 點傷害（玩家彈命中，原版 `16946h`；砲座接觸，`14C18h`、`15746h`）：
    /// 低位元組 8 位元相加不進位，高位元組設 1，下一幀更新時結算。回傳寫入的字組。
    ///
    /// 傷害字組在記錄 +20h。`link` 不是任何一槽的記錄位址時，原版照樣把字組寫到 `link` 加 20h：
    /// 指標 0（已釋放的槽，或裝甲格 FEh 的索引 FDh 讀到槽 9 的動畫計數 0）寫到 `dseg:0020h`，
    /// 那是啟動時文字畫面的色彩屬性，關卡內不再讀，所以玩家看不出來。`Fixes::armor_cell_index` 由呼叫端處理。
    ///
    /// 重製只留下指標 0 的那個字組（[`Enemies::stray_damage`]，進狀態、對拍比對）。其他非記錄的指標（備用字組、
    /// 記錄區的字組當作指標，例如槽 9 的動畫計數不是 0）原版會寫到 `dseg:0021h` 起的別處；實測這個值恆為 0，
    /// 收據走不到，寫入丟掉、讀回 0。
    pub(crate) fn add_entity_damage(&mut self, link: EntityLink, damage: u8) -> u16 {
        let old = self.damage_word(link);
        let new = (u16::from(HIT_PENDING) << 8) | u16::from((old as u8).wrapping_add(damage));
        self.set_damage_word(link, new);
        new
    }

    fn damage_word(&self, link: EntityLink) -> u16 {
        match link {
            EntityLink::Slot(slot) => self.en.slots[slot].enemy.damage,
            EntityLink::Stray(0) => self.en.stray_damage,
            EntityLink::Stray(_) => 0,
        }
    }

    fn set_damage_word(&mut self, link: EntityLink, v: u16) {
        match link {
            EntityLink::Slot(slot) => self.en.slots[slot].enemy.damage = v,
            EntityLink::Stray(0) => self.en.stray_damage = v,
            EntityLink::Stray(_) => {}
        }
    }

    /// 測試用：擺一個使用中的敵人在槽 `slot`，傷害字組設成 `damage`。
    #[cfg(test)]
    pub(crate) fn set_entity(&mut self, slot: usize, damage: u16) {
        self.en.slots[slot].link = enemy_record_link(slot);
        self.en.slots[slot].enemy.damage = damage;
    }

    /// 測試用：指標為 `link` 的敵人的傷害字組。`Stray(0)` 時是 `dseg:0020h`。
    #[cfg(test)]
    pub(crate) fn entity_damage(&self, link: EntityLink) -> u16 {
        self.damage_word(link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{DrawCmd, Op};
    use crate::game::{Fixes, Start};

    fn core() -> Core {
        let mut c = Core::for_test(Start::Demo, Fixes::NONE);
        c.fill_row_tables();
        c.draws.clear();
        c
    }

    /// 擺一個 `w x h` 的形狀在 `at`，每格都是 `cell`。
    fn put_shape(c: &mut Core, at: u16, w: u16, h: u16, cell: u8) {
        c.tables_mut().put_shape(at, w, h, cell);
    }

    /// 在 `at` 擺一個樣板。
    fn put_template(c: &mut Core, at: u16, e: &Enemy) {
        c.tables_mut().put_enemy(at, *e);
    }

    /// 擺一個一般敵人，腳本在 `0x7800`：碼 `script`。形狀 1x1 的惰性空格，圖框表全是 `frame`。
    fn walker(c: &mut Core, script: &[u8], frame: u16) -> Enemy {
        if !script.is_empty() {
            // 腳本後面接的位元組讀成 0（原版測試用的零映像就是這樣）。
            let mut bytes = script.to_vec();
            bytes.extend([0u8; 8]);
            c.tables_mut().put_bytes(0x7800, &bytes);
        }
        put_shape(c, 0x7000, 1, 1, 0);
        c.tables_mut().put_words(0x7100, &(0..9u16).map(|i| frame + i).collect::<Vec<_>>());
        Enemy {
            hp: 3,
            frames: 0x7100,
            blit_x: 20,
            blit_y: 50,
            blit_dx: 8,
            blit_dy: 10,
            fire_threshold: NEVER_FIRES,
            bullet: u16::from(b'F'),
            mode: u16::from(b'N'),
            col: 20,
            row: 5,
            dcol: 1,
            drow: 1,
            script: 0x7800,
            shape: 0x7000,
            explosion: u16::from(b'B'),
            kind: u16::from(b'O'),
            ..Enemy::default()
        }
    }

    fn place(c: &mut Core, slot: usize, e: Enemy) {
        c.en.slots[slot].link = enemy_record_link(slot);
        c.en.slots[slot].enemy = e;
    }

    #[test]
    fn mode_and_kind_decode_the_letters() {
        assert_eq!(
            [b'A', b'Y', b'N', b'?'].map(|c| Mode::from_word(u16::from(c))),
            [Mode::Chase, Mode::Scroll, Mode::Script, Mode::Script]
        );
        assert_eq!(
            [b'N', b'K', b'S', b'O', b'?'].map(|c| Kind::from_word(u16::from(c))),
            [Kind::Hidden, Kind::Embedded, Kind::Overlay, Kind::Normal, Kind::Normal]
        );
    }

    #[test]
    fn motion_codes_follow_the_dispatch_table() {
        assert_eq!(Motion::from_code(1), Motion::Go(Heading::N));
        assert_eq!(Motion::from_code(8), Motion::Go(Heading::NW));
        assert_eq!(Motion::from_code(9), Motion::ChaseRow);
        assert_eq!(Motion::from_code(0x0A), Motion::JumpRight);
        assert_eq!(Motion::from_code(0x0B), Motion::FireBurst);
        assert_eq!(Motion::from_code(0x0D), Motion::SeekRow);
        assert_eq!(Motion::from_code(0x0F), Motion::Die);
        for code in [0, 0x0C, 0x0E, 0x10, 0x1F, 0xE0, 0xF0, 0xFF] {
            assert_eq!(Motion::from_code(code), Motion::Hold, "碼 {code:#x} 不動");
        }
    }

    #[test]
    fn eight_directions_move_the_two_axes() {
        let mut c = core();
        let base = walker(&mut c, &[], 0);
        let moved = |h: Heading| {
            let mut e = Enemy { blit_dx: 8, blit_dy: 10, dcol: 1, drow: 1, ..base };
            e.step(h);
            (e.col as i16 - 20, e.row as i16 - 5, e.blit_x as i16 - 20, e.blit_y as i16 - 50)
        };
        assert_eq!(moved(Heading::N), (0, -1, 0, -10));
        assert_eq!(moved(Heading::NE), (1, -1, 8, -10));
        assert_eq!(moved(Heading::E), (1, 0, 8, 0));
        assert_eq!(moved(Heading::SE), (1, 1, 8, 10));
        assert_eq!(moved(Heading::S), (0, 1, 0, 10));
        assert_eq!(moved(Heading::SW), (-1, 1, -8, 10));
        assert_eq!(moved(Heading::W), (-1, 0, -8, 0));
        assert_eq!(moved(Heading::NW), (-1, -1, -8, -10));
    }

    #[test]
    fn logical_and_blit_steps_are_independent() {
        // 少數樣板的繪製單步和邏輯單步不是 10 倍關係，兩邊各用各的（文件第 2.1 節）。
        let mut c = core();
        let mut e = walker(&mut c, &[], 0);
        e.blit_dy = 11;
        e.drow = 1;
        e.move_down();
        assert_eq!((e.row, e.blit_y), (6, 61));
    }

    #[test]
    fn jump_right_ignores_the_step_size() {
        let mut c = core();
        let mut e = walker(&mut c, &[], 0);
        e.jump_right();
        assert_eq!((e.col, e.blit_x), (26, 26));
    }

    #[test]
    fn script_reads_one_byte_per_frame_and_advances() {
        let mut c = core();
        let mut e = walker(&mut c, &[3, 5, 0], 2);
        assert_eq!(c.read_command(&mut e), 3);
        assert_eq!(c.read_command(&mut e), 5);
        assert_eq!(e.script, 0x7802);
    }

    #[test]
    fn jump_control_code_reads_the_target_and_continues_after_it() {
        let mut c = core();
        // 0x7800: F0 → 0x7810；0x7810 的碼是 7。
        let mut e = walker(&mut c, &[0xF0, 0x10, 0x78], 0);
        c.tables_mut().put_bytes(0x7810, &[7]);
        assert_eq!(c.read_command(&mut e), 7, "跳轉後讀目標處的位元組");
        assert_eq!(e.script, 0x7811, "讀取位置在目標之後");
        // 目標處又是 F0：不再處理，視為不在移動表內。
        c.tables_mut().put_bytes(0x7810, &[0xF0]);
        e.script = 0x7800;
        assert_eq!(c.read_command(&mut e), 0xF0);
        assert_eq!(Motion::from_code(0xF0), Motion::Hold);
    }

    #[test]
    fn morph_swaps_the_template_but_keeps_position_and_explosion() {
        let mut c = core();
        let mut e = walker(&mut c, &[0xE0, 0x00, 0x72], 0);
        e.damage = 0x0003;
        e.anim = 4;
        e.rand_index = 30;
        e.col = 33;
        e.row = 7;
        e.explosion = u16::from(b'V');
        let new = Enemy {
            hp: 99,
            col: 1,
            row: 2,
            explosion: u16::from(b'S'),
            blit_dx: 3,
            dcol: 3,
            script: 0x7900,
            kind: u16::from(b'S'),
            spill: 0xABCD,
            ..Enemy::default()
        };
        put_template(&mut c, 0x7200, &new);
        c.tables_mut().put_bytes(0x7900, &[5]);
        assert_eq!(c.read_command(&mut e), 5, "變身後讀新腳本的第一個碼");
        assert_eq!(e.script, 0x7901);
        assert_eq!((e.col, e.row, e.blit_x, e.blit_y), (33, 7, 33, 70), "位置保留，繪製位置重算");
        assert_eq!(e.explosion, u16::from(b'V'), "死亡特效字母保留");
        assert_eq!((e.hp, e.blit_dx, e.dcol, e.kind, e.spill), (99, 3, 3, u16::from(b'S'), 0xABCD), "其餘照新樣板，含多帶進來的第 24 個字組");
        assert_eq!((e.damage, e.anim, e.rand_index), (0, 0, 0), "傷害、動畫計數、亂數索引都被洗掉");
    }

    #[test]
    fn chase_mode_turns_toward_the_player_one_step_per_frame() {
        let mut c = core();
        c.set_players([Some((20, 10)), None]);
        let mut e = Enemy { mode: u16::from(b'A'), script: 7, col: 30, row: 10, blit_x: 30, blit_y: 100, blit_dx: 8, blit_dy: 10, dcol: 1, drow: 1, ..walker(&mut c, &[], 0) };
        // 目標在左（朝向 7），目前朝向 7 不動，走一步往左。
        let step = c.step_enemy(&mut e, 0);
        assert!(!step.dying);
        assert_eq!(e.script, 7);
        assert_eq!((e.col, e.row), (29, 10));
        // 目標在右（朝向 3）：目前朝向 7 在轉向表前 4 項，減 1 得 6，一步只轉一格。
        c.set_players([Some((38, 10)), None]);
        c.step_enemy(&mut e, 0);
        assert_eq!(e.script, 6, "轉一步，不是直接轉到目標");
    }

    #[test]
    fn code_0d_seeks_up_or_down_by_the_nearest_player() {
        let mut c = core();
        c.set_players([Some((20, 10)), None]);
        let base = walker(&mut c, &[], 0);
        let e = |row| Enemy { col: 20, row, ..base };
        // 玩家在 (20, 10)：敵人 (20, 2)，目標點 (23, 6)，玩家在下方 → 朝向 4 至 6 → 下。
        assert_eq!(c.seek_row(&e(2)), Heading::S);
        // 敵人 (20, 14)，目標點 (23, 18)，玩家在上方 → 上。
        assert_eq!(c.seek_row(&e(14)), Heading::N);
        // 列加 4 小於等於 0：不問瞄準，直接下。
        assert_eq!(c.seek_row(&e((-4i16) as u16)), Heading::S);
        assert_eq!(c.seek_row(&e((-9i16) as u16)), Heading::S);
        // 列加 4 超過 14h：直接上。
        assert_eq!(c.seek_row(&e(0x11)), Heading::N);
    }

    #[test]
    fn code_09_chases_player_one_row() {
        let mut c = core();
        c.set_players([Some((20, 8)), Some((20, 2))]);
        let base = walker(&mut c, &[], 0);
        let at = |row| Enemy { row, ..base };
        assert_eq!(c.chase_row(&at(5)), Heading::S, "列小於玩家 1：往下");
        assert_eq!(c.chase_row(&at(8)), Heading::N, "同列：往上");
        assert_eq!(c.chase_row(&at(0x18)), Heading::N, "18h 以內且不小於玩家 1：往上");
        assert_eq!(c.chase_row(&at(0x19)), Heading::S, "大於 18h：往下");
        c.set_players([None, Some((20, 2))]);
        assert_eq!(c.chase_row(&at(5)), Heading::S, "玩家 1 不在場，仍看他留下的列");
    }

    #[test]
    fn code_0b_fires_a_burst_two_rows_below() {
        let mut c = core();
        c.set_players([Some((20, 10)), None]);
        let mut e = walker(&mut c, &[], 0);
        let dies = c.apply_motion(&mut e, CODE_FIRE_BURST);
        assert!(!dies);
        assert_eq!((e.col, e.row), (20, 5), "不移動");
        assert!(c.eb.slots.iter().any(|s| s.is_active()), "發射了敵彈");
    }

    #[test]
    fn code_0f_marks_the_script_end_but_the_frame_still_runs() {
        let mut c = core();
        let e = walker(&mut c, &[CODE_DIE], 6);
        place(&mut c, 0, e);
        put_shape(&mut c, 0x7000, 1, 1, 1);
        c.update_enemies();
        assert!(!c.en.slots[0].is_active(), "腳本結束的槽釋放");
        assert_eq!(c.en.slots[0].sprite.image, 6, "那一幀仍產生繪製值");
        assert_eq!(c.grids.peek8(c.grid1_row_start(5) + 20), 1, "也寫了足跡");
        assert_eq!(c.en.slots[0].enemy.rand_index, 1, "亂數索引照常前進");
    }

    /// 一個欄數為 `cols` 的核心，槽 0 擺 `walker`（腳本 `script`、起點欄 `col`、模式 `mode`、水平步長 `dcol`），
    /// 一幀一幀更新到槽 0 釋放（最多 `limit` 幀）。回傳釋放時的幀數與最後的欄；沒釋放時幀數是 `limit`。
    fn run_until_released(cols: u16, script: &[u8], col: u16, mode: u8, dcol: u16, limit: u32) -> (u32, i16) {
        let mut c = core();
        if cols != 40 {
            c.enter_field(cols);
        }
        let mut e = walker(&mut c, script, 0);
        (e.col, e.blit_x, e.mode, e.dcol, e.blit_dx) = (col, col, u16::from(mode), dcol, dcol);
        place(&mut c, 0, e);
        for n in 1..=limit {
            c.update_enemies();
            if !c.en.slots[0].is_active() {
                return (n, c.en.slots[0].enemy.col as i16);
            }
        }
        (limit, c.en.slots[0].enemy.col as i16)
    }

    #[test]
    fn at_100_percent_a_finished_script_releases_the_enemy_in_view_as_in_the_original() {
        // 欄 20 往左走三步，第四幀腳本結束就釋放，不管還在不在畫面裡。
        assert_eq!(run_until_released(40, &[7, 7, 7, CODE_DIE], 20, b'N', 1, 100), (4, 17));
    }

    #[test]
    fn a_wide_field_lets_a_left_flying_enemy_leave_through_the_left_edge() {
        // 同一個腳本：寬戰場裡不在左緣外就繼續往左飛，形狀（寬 1）整個離開欄 0 的那一幀釋放，欄是 -1。
        for cols in [48u16, 60] {
            let (frames, col) = run_until_released(cols, &[7, 7, 7, CODE_DIE], 20, b'N', 1, 100);
            assert_eq!((frames, col), (4 + 18, -1), "{cols} 欄");
        }
        // 步長 2：走三步到欄 20，之後每幀兩欄，欄 -2 的那一幀形狀（寬 1）整個在左緣外，釋放。
        assert_eq!(run_until_released(48, &[7, 7, 7, CODE_DIE], 26, b'N', 2, 100), (4 + 11, -2));
    }

    #[test]
    fn a_wide_field_lets_a_right_flying_enemy_leave_through_the_right_edge() {
        // 往右走兩步到欄 42，腳本結束，繼續飛到欄 48（整個形狀在右界之外）才釋放。
        assert_eq!(run_until_released(48, &[3, 3, CODE_DIE], 40, b'N', 1, 100), (3 + 6, 48));
        // 100% 在腳本結束那一幀釋放。
        assert_eq!(run_until_released(40, &[3, 3, CODE_DIE], 30, b'N', 1, 100), (3, 32));
    }

    #[test]
    fn a_wide_field_does_not_extend_vertical_exits_or_enemies_already_outside() {
        // 最後的方向是上或下：腳本結束就釋放（垂直方向的戰場沒有變）。
        for code in [1u8, 5] {
            assert_eq!(run_until_released(60, &[code, CODE_DIE], 30, b'N', 1, 100), (2, 30), "方向碼 {code}");
        }
        // 腳本結束時已經整個在左緣外或右界外：釋放。
        assert_eq!(run_until_released(60, &[7, CODE_DIE], 0u16.wrapping_sub(1), b'N', 1, 100).0, 2);
        assert_eq!(run_until_released(60, &[3, CODE_DIE], 59, b'N', 1, 100).0, 2);
        // 沒有移動過（最後方向碼是 0）：沒有方向可飛，釋放。
        assert_eq!(run_until_released(48, &[0, CODE_DIE], 20, b'N', 1, 100), (2, 20));
    }

    #[test]
    fn a_wide_field_keeps_the_stage_clear_marker_and_wreck_enemies_out_of_the_flight() {
        // 子彈字母 E（死亡就是過關旗標）：腳本結束照原版釋放。
        let mut c = core();
        c.enter_field(48);
        let mut e = walker(&mut c, &[7, CODE_DIE], 0);
        e.bullet = u16::from(b'E');
        place(&mut c, 0, e);
        c.update_enemies();
        c.update_enemies();
        assert!(!c.en.slots[0].is_active(), "E 敵人照原版在腳本結束時釋放");
        assert!(c.stage_cleared(), "過關旗標照常設");
    }

    #[test]
    fn a_wide_field_caps_the_flight_when_the_enemy_cannot_leave() {
        // 水平步長 0：永遠飛不出去，用上限（欄數加 16 幀）釋放。移動 1 幀、腳本結束 1 幀、再飛 cols + 16 幀。
        assert_eq!(run_until_released(48, &[7, CODE_DIE], 20, b'N', 0, 200), (2 + 48 + 16, 20));
        assert_eq!(run_until_released(60, &[7, CODE_DIE], 20, b'N', 0, 200), (2 + 60 + 16, 20));
    }

    #[test]
    fn a_wide_field_lets_scenery_drifting_enemies_drift_out_after_the_script_ends() {
        // 模式 Y：每幀隨景物左移一欄。腳本結束時還在畫面裡就繼續漂到左緣外。100% 在腳本結束那一幀釋放。
        assert_eq!(run_until_released(40, &[CODE_DIE], 10, b'Y', 1, 100), (1, 9));
        // 每幀先檢查再左移：欄 -1 時檢查到形狀整個在左緣外，那一幀仍左移一欄，存下來的欄是 -2。
        assert_eq!(run_until_released(48, &[CODE_DIE], 10, b'Y', 1, 100), (12, -2));
    }

    // ---- 景物綁定的敵人（Approach） ----

    /// 欄數 `cols` 的核心，時間表 `table` 只有一列 `(t0, 0x7400)`，樣板：模式 `mode`、x `x`、腳本 `script`、耐久 2。
    fn bound_core(cols: u16, table: Timetable, mode: u8, x: u16, script: &[u8], t0: u16) -> Core {
        let mut c = core();
        if cols != 40 {
            c.enter_field(cols);
        }
        let mut e = walker(&mut c, script, 0);
        e.hp = 2;
        e.mode = u16::from(mode);
        e.col = x;
        e.blit_x = x;
        put_template(&mut c, 0x7400, &e);
        c.tables_mut().put_timetable(table, &[(t0, 0x7400)]);
        c
    }

    /// 照第三關的順序跑一段幀：幀計數、地圖左緣欄（每兩幀加 1，偶數幀加，幀 0 是 1）、生成、更新。
    /// 每幀之後呼叫 `each`。
    fn run_frames(c: &mut Core, table: Timetable, frames: std::ops::RangeInclusive<u16>, mut each: impl FnMut(&Core, u16)) {
        for f in frames {
            c.set_stage_frame(f);
            c.scenes.map.left_col = 1 + f / 2;
            c.spawn_scheduled(table);
            c.update_enemies();
            each(c, f);
        }
    }

    /// 槽 0 在每一幀之後的欄（沒有就是 `None`）。
    fn cols_of_slot0(c: &mut Core, table: Timetable, frames: std::ops::RangeInclusive<u16>) -> Vec<(u16, Option<i16>)> {
        let mut out = Vec::new();
        run_frames(c, table, frames, |c, f| {
            let s = &c.en.slots[0];
            out.push((f, s.is_active().then_some(s.enemy.col as i16)));
        });
        out
    }

    /// 往左一步、停一步的腳本，接結束碼。
    fn map_script(pairs: usize) -> Vec<u8> {
        let mut v: Vec<u8> = (0..pairs).flat_map(|_| [7u8, 0]).collect();
        v.push(CODE_DIE);
        v
    }

    #[test]
    fn scenery_bound_rows_are_recognised_by_mode_and_by_the_map_speed_script() {
        let mut c = core();
        let y = walker(&mut c, &[7, 7, 7, 7, 7, 7, 7, 7], 0);
        put_template(&mut c, 0x7400, &Enemy { mode: u16::from(b'Y'), ..y });
        assert_eq!(c.scenery_pace(Timetable::Stage2, 0x7400), Some(Pace::Drift));
        assert_eq!(c.scenery_pace(Timetable::Stage1, 0x7400), Some(Pace::Drift), "模式 Y 的敵人在哪一關都是每幀 1 欄");
        // 第三關：往左一步、停一步四組以上（兩種相位）是地圖速度。
        for script in [vec![7u8, 0, 7, 0, 7, 0, 7, 0, 7, 0], vec![0, 7, 0, 7, 0, 7, 0, 7, 0]] {
            let mut c = core();
            let e = walker(&mut c, &script, 0);
            put_template(&mut c, 0x7400, &e);
            assert_eq!(c.scenery_pace(Timetable::Stage3, 0x7400), Some(Pace::Map), "{script:?}");
            assert_eq!(c.scenery_pace(Timetable::Stage2, 0x7400), None, "別的關的同樣腳本不算（只有第三關有地圖）");
        }
        // 自由飛行：每幀一步、只有三組、中途改方向、腳本比 8 個位元組短（不 panic）。
        for script in [vec![7u8; 10], vec![7, 0, 7, 0, 7, 0, 3, 0], vec![7, 0, 7, 0, 7, 0, 0x0F]] {
            let mut c = core();
            let e = walker(&mut c, &script, 0);
            put_template(&mut c, 0x7400, &e);
            assert_eq!(c.scenery_pace(Timetable::Stage3, 0x7400), None, "{script:?}");
        }
    }

    #[test]
    fn a_wide_field_spawns_a_map_speed_row_early_and_it_arrives_on_the_original_column() {
        // 第三關 t0 = 100 的列，x 40，腳本是往左一步停一步。100% 的軌跡當標準。
        let script = map_script(30);
        let mut base = bound_core(40, Timetable::Stage3, b'N', 40, &script, 100);
        let want = cols_of_slot0(&mut base, Timetable::Stage3, 90..=160);
        assert_eq!(want[10], (100, Some(39)), "100%：原版生成幀走第一步，欄 39");
        for (cols, extra) in [(48u16, 8u16), (60, 20)] {
            let mut c = bound_core(cols, Timetable::Stage3, b'N', 40, &script, 100);
            let got = cols_of_slot0(&mut c, Timetable::Stage3, 0..=160);
            let lead = 2 * extra;
            // 提前 2 × 多出來的欄數幀生成，之前沒有。
            let first = got.iter().find(|(_, col)| col.is_some()).unwrap().0;
            assert_eq!(first, 100 - lead, "{cols} 欄：提前 {lead} 幀");
            // 從原版生成幀起，每幀的欄和 100% 完全相同（同一個腳本位置、同一個地圖左緣）。
            for (f, col) in got.iter().filter(|(f, _)| *f >= 90) {
                let base_col = want.iter().find(|(g, _)| g == f).map(|(_, c)| *c);
                if *f >= 100 {
                    assert_eq!(Some(*col), base_col, "{cols} 欄第 {f} 幀");
                }
            }
            // 原版生成幀的前一幀剛好在樣板的欄 40；提前的這段隨地圖每兩幀左移 1 欄，所在地圖欄（左緣欄加欄）不超過 1 欄的誤差。
            assert_eq!(got.iter().find(|(f, _)| *f == 99).unwrap().1, Some(40));
            let spawn_col = got.iter().find(|(f, _)| *f == first).unwrap().1.unwrap();
            assert!((i32::from(spawn_col) - i32::from(40 + extra)).abs() <= 1, "{cols} 欄：新右緣進場的欄 {spawn_col}");
            let mut maps = Vec::new();
            for f in first..100 {
                let col = got.iter().find(|(g, _)| *g == f).unwrap().1.unwrap();
                maps.push(i32::from(col) + 1 + i32::from(f / 2));
            }
            assert!(maps.iter().all(|m| *m == maps[0]), "{cols} 欄：提前這段釘在同一個地圖欄：{maps:?}");
        }
    }

    #[test]
    fn a_wide_field_spawns_a_drift_row_early_and_it_arrives_on_the_original_column() {
        let script = vec![7u8, 7, 7, 3, 3, 3, 7, 7, 7, CODE_DIE];
        let mut base = bound_core(40, Timetable::Stage2, b'Y', 40, &script, 50);
        let want = cols_of_slot0(&mut base, Timetable::Stage2, 40..=70);
        for (cols, extra) in [(48u16, 8u16), (60, 20)] {
            let mut c = bound_core(cols, Timetable::Stage2, b'Y', 40, &script, 50);
            let got = cols_of_slot0(&mut c, Timetable::Stage2, 0..=70);
            let first = got.iter().find(|(_, col)| col.is_some()).unwrap();
            assert_eq!(first.0, 50 - extra, "{cols} 欄：模式 Y 每幀 1 欄，提前多出來的欄數那麼多幀");
            assert_eq!(first.1, Some(39 + extra as i16), "新右緣進場：動作之後在新右緣的最後一欄");
            for f in 40..=70u16 {
                let g = got.iter().find(|(g, _)| *g == f).unwrap().1;
                let b = want.iter().find(|(b, _)| *b == f).unwrap().1;
                if f >= 50 {
                    assert_eq!(g, b, "{cols} 欄第 {f} 幀");
                }
            }
            assert_eq!(got.iter().find(|(f, _)| *f == 49).unwrap().1, Some(40), "原版生成幀的前一幀在樣板的欄");
        }
    }

    #[test]
    fn a_bound_row_inside_the_screen_keeps_its_column_and_free_rows_still_shift() {
        // 模式 Y、x 20：畫面內生成，欄不放大（景物同欄在各寬度是同一個位置），也不提前。
        let script = vec![7u8, 7, 7, CODE_DIE];
        for cols in [40u16, 48, 60] {
            let mut c = bound_core(cols, Timetable::Stage2, b'Y', 20, &script, 50);
            let got = cols_of_slot0(&mut c, Timetable::Stage2, 0..=52);
            assert_eq!(got.iter().find(|(_, col)| col.is_some()).unwrap().0, 50, "{cols} 欄不提前");
            assert_eq!(got.iter().find(|(f, _)| *f == 50).unwrap().1, Some(18), "{cols} 欄：欄 20 走一步加隨景物左移一欄");
        }
        // 自由飛行（模式 N、腳本每幀一步）：照舊在原版生成幀生成在新右緣，不提前。
        let free = vec![7u8; 6];
        let mut c = bound_core(60, Timetable::Stage3, b'N', 40, &free, 50);
        let got = cols_of_slot0(&mut c, Timetable::Stage3, 0..=51);
        assert_eq!(got.iter().find(|(_, col)| col.is_some()).unwrap(), &(50, Some(59)));
        // 頭目列不提前（即使模式是 Y）：它靠合成圖對齊，不靠景物。
        let mut c = bound_core(60, Timetable::Stage2, b'Y', 40, &free, 263);
        let got = cols_of_slot0(&mut c, Timetable::Stage2, 200..=264);
        assert_eq!(got.iter().find(|(_, col)| col.is_some()).unwrap().0, 263);
    }

    #[test]
    fn an_approaching_enemy_does_not_read_its_script_roll_or_fire_but_can_be_hit() {
        let script = map_script(30);
        let mut c = bound_core(60, Timetable::Stage3, b'N', 40, &script, 100);
        run_frames(&mut c, Timetable::Stage3, 0..=70, |_, _| {});
        let e = c.en.slots[0].enemy;
        assert_eq!((e.script, e.rand_index, e.anim), (0x7800, 0, 0), "提前這段沒有讀腳本、沒有擲亂數、沒有動畫");
        assert_eq!(c.approach.left[0], 100 - 70 - 1, "第 70 幀之後，距離原版生成幀（第 100 幀）還有 29 幀");
        assert!(c.eb.slots.iter().all(|s| !s.is_active()), "沒有開火");
        // 有形狀：寫了足跡，玩家彈打得到。
        put_shape(&mut c, 0x7000, 1, 1, 1);
        run_frames(&mut c, Timetable::Stage3, 71..=71, |_, _| {});
        assert_eq!(c.grids.peek8(c.grid1_row_start(e.row) + c.en.slots[0].enemy.col), 1, "足跡寫進 G1");
        // 被打中（傷害 3 大於耐久 2）就死，和在原版生成幀之後被打中一樣。
        c.en.slots[0].enemy.damage = (u16::from(HIT_PENDING) << 8) | 3;
        run_frames(&mut c, Timetable::Stage3, 72..=72, |_, _| {});
        assert!(!c.en.slots[0].is_active(), "提前這段被打死");
    }

    #[test]
    fn a_full_pool_drops_the_bound_row_and_counts_it() {
        let script = map_script(10);
        let mut c = bound_core(48, Timetable::Stage3, b'N', 40, &script, 100);
        for slot in 0..POOL_SIZE {
            c.en.slots[slot].link = enemy_record_link(slot);
        }
        c.set_stage_frame(100 - 16);
        c.scenes.map.left_col = 1 + (100 - 16) / 2;
        c.spawn_scheduled(Timetable::Stage3);
        let port = c.port.borrow();
        assert_eq!((port.spawn_drops, port.bound_drops), (1, 1), "提前生成沒有空槽：放棄，算景物綁定的列");
        drop(port);
        assert!(c.approach.left.iter().all(|&l| l == 0));
    }

    #[test]
    fn an_early_start_before_frame_zero_begins_at_frame_zero() {
        // 第三關有列在幀 21：150% 要提前 40 幀，不夠就從第 0 幀起，欄是從樣板往前推的位置。
        let script = map_script(30);
        let mut base = bound_core(40, Timetable::Stage3, b'N', 40, &script, 21);
        let want = cols_of_slot0(&mut base, Timetable::Stage3, 0..=60);
        let mut c = bound_core(60, Timetable::Stage3, b'N', 40, &script, 21);
        let got = cols_of_slot0(&mut c, Timetable::Stage3, 0..=60);
        let first = got.iter().find(|(_, col)| col.is_some()).unwrap();
        assert_eq!(first.0, 0, "從第 0 幀起");
        assert!(first.1.unwrap() >= 40 && first.1.unwrap() <= 60, "在畫面右半：{first:?}");
        for f in 21..=60u16 {
            assert_eq!(got.iter().find(|(g, _)| *g == f).unwrap().1, want.iter().find(|(g, _)| *g == f).unwrap().1, "第 {f} 幀");
        }
    }

    #[test]
    fn a_respawn_in_a_slot_forgets_the_flight_of_the_previous_enemy() {
        let mut c = core();
        c.enter_field(48);
        let e = walker(&mut c, &[7, CODE_DIE], 0);
        place(&mut c, 0, e);
        c.update_enemies();
        c.update_enemies();
        assert!(c.en.slots[0].is_active(), "飛出畫面中");
        assert!(c.linger.left[0] > 0);
        // 同一槽重新生成：飛行狀態清掉，照新敵人的腳本走。
        c.en.slots[0].link = 0;
        put_template(&mut c, 0x7400, &Enemy { col: 30, row: 3, ..Enemy::default() });
        c.spawn_row(0x7400, false);
        assert_eq!((c.linger.left[0], c.linger.last[0]), (0, 0));
    }

    #[test]
    fn scroll_mode_drifts_one_column_left_after_moving() {
        let mut c = core();
        let mut e = walker(&mut c, &[3, 0], 0);
        e.mode = u16::from(b'Y');
        e.blit_dx = 1;
        place(&mut c, 0, e);
        c.update_enemies();
        let got = c.en.slots[0].enemy;
        assert_eq!((got.col, got.blit_x), (20, 20), "右走一步再左移一欄");
        assert_eq!(got.script, 0x7801);
    }

    #[test]
    fn frame_is_the_table_entry_plus_the_animation_counter() {
        let mut c = core();
        let mut e = walker(&mut c, &[], 10);
        e.anim = 2;
        assert_eq!(c.select_frame(&mut e, 3), 10 + 3 + 2);
        assert_eq!(c.select_frame(&mut e, 0), 10 + 2);
        assert_eq!(c.select_frame(&mut e, 8), 10 + 8 + 2);
        assert_eq!(e.anim, 2, "沒有上限表就不推進");
        // 碼大於 8：取第 0 項，不加動畫計數。
        assert_eq!(c.select_frame(&mut e, 9), 10);
        assert_eq!(c.select_frame(&mut e, 0xFF), 10);
    }

    #[test]
    fn animation_counter_advances_below_the_limit_and_wraps_to_zero() {
        let mut c = core();
        let mut e = walker(&mut c, &[], 2);
        e.anim_limits = 0x7300;
        c.tables_mut().put_words(0x7300, &[5; 9]);
        e.anim = 0;
        let seen: Vec<u16> = (0..6).map(|_| c.select_frame(&mut e, 0)).collect();
        assert_eq!(seen, [2, 3, 4, 5, 2, 3], "圖框 2 至 5 循環");
    }

    #[test]
    fn animation_limit_for_codes_above_8_is_read_at_the_unaligned_byte_offset() {
        let mut c = core();
        let mut e = walker(&mut c, &[], 2);
        e.anim_limits = 0x7300;
        // 碼 0Bh 讀 0x730B 起的字組（不乘 2）：低位元組在 0x730B，高位元組在 0x730C。
        c.tables_mut().put_words(0x7300, &[0, 0, 0, 0, 0, 0x0300, 0, 0, 0]);
        assert_eq!(c.select_frame(&mut e, 0x0B), 2, "碼 > 8 取第 0 項");
        assert_eq!(e.anim, 1, "圖框 2 小於上限 3，計數加 1");
        e.frames = 0x7100;
        c.tables_mut().put_words(0x7300, &[0, 0, 0, 0, 0, 0x0200, 0, 0, 0]);
        c.select_frame(&mut e, 0x0B);
        assert_eq!(e.anim, 0, "圖框不小於上限，歸零");
    }

    #[test]
    fn damage_below_the_threshold_flashes_and_keeps_only_the_low_byte() {
        let mut c = core();
        let mut e = Enemy { hp: 3, damage: 0x0103, ..walker(&mut c, &[], 0) };
        assert_eq!(c.resolve_damage(&mut e), Damage::Flash);
        assert_eq!(e.damage, 0x0003, "高位元組清掉，累計傷害留著");
        assert_eq!(c.resolve_damage(&mut e), Damage::None, "沒有新的命中就不結算");
        assert!(c.fx.slots.iter().all(|s| !s.is_active()));
    }

    #[test]
    fn damage_above_the_threshold_kills_and_plays_the_explosion() {
        let mut c = core();
        let mut e = Enemy { hp: 3, damage: 0x0104, explosion: u16::from(b'B'), col: 9, row: 4, ..walker(&mut c, &[], 0) };
        assert_eq!(c.resolve_damage(&mut e), Damage::Killed);
        assert_eq!(c.fx.slots[0].script_pos, c.tables.fx_start(crate::fx::Script::BigExplosion));
        assert_eq!((c.fx.slots[0].x, c.fx.slots[0].y), (9, 4));
    }

    #[test]
    fn difficulty_bonus_raises_the_kill_threshold() {
        let mut c = core();
        c.set_difficulty(1, 0);
        let mut e = Enemy { hp: 3, damage: 0x0104, ..walker(&mut c, &[], 0) };
        assert_eq!(c.resolve_damage(&mut e), Damage::Flash, "4 不大於 3 + 1");
        e.damage = 0x0105;
        assert_eq!(c.resolve_damage(&mut e), Damage::Killed);
    }

    #[test]
    fn indestructible_enemy_with_999_hp_never_dies_from_one_byte_of_damage() {
        let mut c = core();
        let mut e = Enemy { hp: 999, damage: 0x01FF, ..walker(&mut c, &[], 0) };
        assert_eq!(c.resolve_damage(&mut e), Damage::Flash);
    }

    #[test]
    fn a_killed_enemy_skips_the_rest_of_the_frame_and_keeps_the_previous_sprite() {
        let mut c = core();
        let alive = walker(&mut c, &[0], 6);
        put_shape(&mut c, 0x7000, 1, 1, 1);
        let dead = Enemy { damage: 0x0105, hp: 0, ..alive };
        place(&mut c, 0, alive);
        place(&mut c, 1, dead);
        c.update_enemies();
        assert!(c.en.slots[0].is_active() && !c.en.slots[1].is_active());
        assert_eq!(c.en.slots[0].sprite.image, 6);
        assert_eq!(c.en.slots[1].sprite, c.en.slots[0].sprite, "死掉的槽拿到前一個敵人的繪製值");
        assert_eq!(c.en.slots[1].flash, NO);
        assert_eq!(c.en.slots[1].enemy.script, 0x7800, "沒有讀腳本");
        assert_eq!(c.en.slots[1].enemy.damage, 0x0005, "傷害字組寫回時高位元組已清");
        assert_eq!(c.grids.peek8(c.grid1_row_start(5) + 20), 1, "槽 1 沒有寫足跡，G1 是槽 0 的格值");
    }

    #[test]
    fn death_flag_in_the_data_segment_follows_the_last_enemy_updated() {
        // word_1EDAA 同時是 G3、G4 列起點表的越界列，所以每個敵人更新完都要寫進資料段，讀它的人在同一幀就看得到。
        let mut c = core();
        let alive = walker(&mut c, &[0], 0);
        let dead = Enemy { damage: 0x0105, hp: 0, ..alive };
        place(&mut c, 0, alive);
        c.update_enemies();
        assert_eq!(c.en.death_flag, u16::from(b'N'));
        assert_eq!(c.grid3_cell_addr(0x14, 0), c.grid3_cell_addr(0x30, 0), "越界列的起點是這個字組");
        let before = c.grid3_cell_addr(0x14, 0);
        place(&mut c, 1, dead);
        c.update_enemies();
        assert_eq!(c.en.death_flag, u16::from(b'Y'), "槽 1 最後更新，被打死");
        assert_eq!(c.grid3_cell_addr(0x14, 0), before + (u16::from(b'Y') - u16::from(b'N')));
        c.update_enemies();
        assert_eq!(c.en.death_flag, u16::from(b'N'), "槽 1 已釋放，槽 0 是最後一個");
        c.en.release_all();
        c.update_enemies();
        assert_eq!(c.en.death_flag, u16::from(b'N'), "沒有使用中的敵人就維持原值");
    }

    #[test]
    fn a_hit_survivor_flashes_for_the_frame() {
        let mut c = core();
        let e = Enemy { damage: 0x0101, hp: 3, ..walker(&mut c, &[0], 0) };
        place(&mut c, 0, e);
        c.update_enemies();
        assert_eq!(c.en.slots[0].flash, YES);
        c.update_enemies();
        assert_eq!(c.en.slots[0].flash, NO);
    }

    #[test]
    fn stage_clear_bullet_letter_sets_the_flag_when_the_core_dies() {
        let mut c = core();
        let e = Enemy { bullet: u16::from(b'E'), damage: 0x0109, hp: 0, ..walker(&mut c, &[1], 0) };
        place(&mut c, 3, e);
        assert!(!c.stage_cleared());
        c.update_enemies();
        assert!(c.stage_cleared());
        assert!(!c.en.slots[3].is_active());
    }

    #[test]
    fn embedded_enemy_leaves_a_wreck_in_scratch_b() {
        let mut c = core();
        let e = Enemy { kind: u16::from(b'K'), damage: 0x0109, hp: 0, carrier: u16::from(b'P'), ..walker(&mut c, &[1], 0) };
        // 圖框表指向 (圖 19, x 39, y 10)。
        c.tables_mut().put_words(0x7100, &[19, 39, 10, 0, 0, 0, 0, 0, 0]);
        place(&mut c, 0, e);
        c.update_enemies();
        assert!(!c.en.slots[0].is_active());
        assert_eq!(c.scratch_select(), SCRATCH_B);
        let blits: Vec<_> = c.draws.iter().filter_map(|d| if let DrawCmd::Blit(b) = d { Some(*b) } else { None }).collect();
        assert_eq!(blits.len(), 1);
        assert_eq!((blits[0].op, blits[0].x, blits[0].y, blits[0].index, blits[0].scratch), (Op::StageToScratch, 39, 10, 19, SCRATCH_B));
        assert_eq!(blits[0].obj, None);
        assert!(c.eb.slots.iter().all(|s| !s.is_active()), "K 型不掉道具，即使是載體");
    }

    #[test]
    fn carrier_drops_its_item_when_it_dies() {
        let mut c = core();
        c.set_players([Some((10, 10)), None]);
        let e = Enemy { carrier: u16::from(b'P'), bullet: u16::from(b'l'), damage: 0x0109, hp: 0, ..walker(&mut c, &[1], 0) };
        place(&mut c, 0, e);
        c.update_enemies();
        assert!(c.eb.slots.iter().any(|s| s.is_active()), "道具球");
        let capsule = c.eb.slots.iter().find(|s| s.is_active()).unwrap().bullet;
        assert_eq!((capsule.col, capsule.row), (20, 7), "起點在敵人下方 2 列");
        // 沒有 P 就不掉。
        let mut c = core();
        let e = Enemy { bullet: u16::from(b'l'), damage: 0x0109, hp: 0, ..walker(&mut c, &[1], 0) };
        place(&mut c, 0, e);
        c.update_enemies();
        assert!(c.eb.slots.iter().all(|s| !s.is_active()));
    }

    #[test]
    fn firing_roll_advances_the_index_even_when_the_enemy_never_fires() {
        let mut c = core();
        for i in 0..FIRE_ROLLS {
            c.tables_mut().put_fire_roll(usize::from(i), 15);
        }
        c.set_difficulty(0, 3);
        let mut quiet = Enemy { fire_threshold: NEVER_FIRES, ..Enemy::default() };
        assert!(!c.rolls_to_fire(&mut quiet));
        assert_eq!(quiet.rand_index, 1);
        // 亂數 15 + 加成 3 = 18 大於 17 才開火，等於 18 不開。
        let mut loud = Enemy { fire_threshold: 17, ..Enemy::default() };
        assert!(c.rolls_to_fire(&mut loud));
        let mut edge = Enemy { fire_threshold: 18, ..Enemy::default() };
        assert!(!c.rolls_to_fire(&mut edge));
    }

    #[test]
    fn firing_roll_index_wraps_after_74() {
        let mut c = core();
        for i in 0..FIRE_ROLLS {
            c.tables_mut().put_fire_roll(usize::from(i), i);
        }
        let mut e = Enemy { fire_threshold: 0, rand_index: 73, ..Enemy::default() };
        assert!(c.rolls_to_fire(&mut e));
        assert_eq!(e.rand_index, 74);
        let mut e = Enemy { fire_threshold: 0, rand_index: 74, ..Enemy::default() };
        assert!(!c.rolls_to_fire(&mut e), "索引 75 繞回 0，表項 0 的值 0 不大於 0");
        assert_eq!(e.rand_index, 0);
    }

    #[test]
    fn a_firing_enemy_shoots_from_its_own_position() {
        let mut c = core();
        c.set_players([Some((20, 10)), None]);
        for i in 0..FIRE_ROLLS {
            c.tables_mut().put_fire_roll(usize::from(i), 15);
        }
        let e = Enemy { fire_threshold: 4, bullet: u16::from(b'F'), ..walker(&mut c, &[0], 0) };
        place(&mut c, 0, e);
        c.update_enemies();
        let fired = c.eb.slots.iter().find(|s| s.is_active()).expect("開火");
        assert_eq!((fired.bullet.col, fired.bullet.row), (20, 5));
    }

    #[test]
    fn footprint_g1_adds_the_slot_number_and_overwrites() {
        let mut c = core();
        let mut e = walker(&mut c, &[0], 0);
        c.tables_mut().put_shape_cells(0x7000, 3, 2, &[1, 0xFE, 0xFF, 0, 1, 1]);
        e.col = 10;
        e.row = 4;
        place(&mut c, 2, e);
        c.update_enemies();
        let row4 = c.grid1_row_start(4);
        let row5 = c.grid1_row_start(5);
        assert_eq!([c.grids.peek8(row4 + 10), c.grids.peek8(row4 + 11), c.grids.peek8(row4 + 12)], [3, 0xFE, 0xFF], "格值 1 加槽號 2；裝甲與惰性格不加");
        assert_eq!([c.grids.peek8(row5 + 10), c.grids.peek8(row5 + 11), c.grids.peek8(row5 + 12)], [0, 3, 3], "0 不寫");
        // 槽號大的蓋過槽號小的。
        let mut other = e;
        other.col = 11;
        place(&mut c, 7, other);
        c.update_enemies();
        assert_eq!(c.grids.peek8(row5 + 12), 8, "槽 7 後更新，覆蓋槽 2 的 3");
        assert_eq!(c.grids.peek8(row5 + 11), 3, "槽 7 的形狀在這格是 0，不會清掉槽 2 的格值");
    }

    #[test]
    fn footprint_clips_to_columns_0_through_39() {
        let mut c = core();
        let mut e = walker(&mut c, &[0], 0);
        put_shape(&mut c, 0x7000, 4, 1, 1);
        e.col = 38;
        e.row = 2;
        place(&mut c, 0, e);
        c.update_enemies();
        let row = c.grid1_row_start(2);
        assert_eq!([c.grids.peek8(row + 37), c.grids.peek8(row + 38), c.grids.peek8(row + 39)], [0, 1, 1]);
        assert_eq!(c.grids.peek8(row + 40), 0, "欄 40 以後不溢進下一列");
        assert_eq!(c.grids.peek8(c.grid1_row_start(3)), 0);
        // 負的欄：欄 -2、-1 不寫，0、1 寫。
        c.clear_grid1();
        let mut e = e;
        e.col = (-2i16) as u16;
        place(&mut c, 0, e);
        c.update_enemies();
        assert_eq!([c.grids.peek8(row), c.grids.peek8(row + 1), c.grids.peek8(row + 2)], [1, 1, 0]);
        assert_eq!(c.grids.peek8(row - 1), 0, "也不溢進上一列");
    }

    #[test]
    fn footprint_skips_rows_above_the_screen_and_stops_at_row_13h() {
        let mut c = core();
        let mut e = walker(&mut c, &[0], 0);
        // 3 列高，列 0 是 1、列 1 是 2、列 2 是 3；敵人在列 -1：第一列跳過。
        put_shape(&mut c, 0x7000, 1, 3, 0);
        c.tables_mut().put_shape_cells(0x7000, 1, 3, &[1, 2, 3]);
        e.col = 5;
        e.row = (-1i16) as u16;
        place(&mut c, 0, e);
        c.update_enemies();
        assert_eq!([c.grids.peek8(c.grid1_row_start(0) + 5), c.grids.peek8(c.grid1_row_start(1) + 5)], [2, 3], "形狀的第 2、3 列落在列 0、1，第 1 列跳過");
    }

    #[test]
    fn footprint_rows_from_13h_are_not_written() {
        let mut c = core();
        let mut e = walker(&mut c, &[0], 0);
        put_shape(&mut c, 0x7000, 1, 4, 1);
        e.col = 5;
        e.row = 0x11;
        place(&mut c, 0, e);
        c.update_enemies();
        assert_eq!(c.grids.peek8(c.grid1_row_start(0x11) + 5), 1);
        assert_eq!(c.grids.peek8(c.grid1_row_start(0x12) + 5), 1);
        assert_eq!(c.grids.peek8(c.grid1_row_start(0x13) + 5), 0, "列 13h 不寫");
        // 整個形狀在列 13h 以下：什麼都不寫。
        c.clear_grid1();
        e.row = 0x20;
        place(&mut c, 0, e);
        c.update_enemies();
        assert!(c.grids.g1.iter().all(|&b| b == 0));
    }

    #[test]
    fn footprint_g3_is_or_ed_inside_its_own_row() {
        let mut c = core();
        let mut e = walker(&mut c, &[0], 0);
        put_shape(&mut c, 0x7000, 3, 1, 0x05);
        e.col = 38;
        e.row = 3;
        place(&mut c, 0, e);
        let (a38, a39, a40) = (c.grid3_cell_addr(3, 38), c.grid3_cell_addr(3, 39), c.grid3_cell_addr(3, 40));
        c.grids.poke8(a38, 0x02);
        c.update_enemies();
        assert_eq!([c.grids.peek8(a38), c.grids.peek8(a39)], [0x07, 0x05], "以 or 累積");
        assert_eq!(c.grids.peek8(a40), 0, "欄 40 超出列右緣，不溢進下一列");
        assert_eq!(c.grids.peek8(c.grid3_cell_addr(4, 0)), 0);
    }

    #[test]
    fn footprint_sets_the_stride_register_to_the_shape_width() {
        let mut c = core();
        let mut e = walker(&mut c, &[0], 0);
        put_shape(&mut c, 0x7000, 7, 1, 0);
        e.row = 0x30;
        place(&mut c, 0, e);
        c.update_enemies();
        c.overwrite_my();
        let Some(DrawCmd::Blit(b)) = c.draws.last().copied() else { panic!("沒有繪製命令") };
        assert_eq!(b.stride_reg, Some(7), "形狀寬是下一個繪製命令看到的殘值，即使整個形狀都在畫面下方");
    }

    #[test]
    fn spawn_copies_the_template_into_the_first_free_slot() {
        let mut c = core();
        let template = Enemy { col: 40, row: 3, hp: 5, spill: 0x1234, ..Enemy::default() };
        put_template(&mut c, 0x7400, &template);
        c.en.slots[0].link = 1;
        c.spawn_enemy(0x7400);
        assert_eq!(c.en.slots[1].link, enemy_record_link(1));
        let got = c.en.slots[1].enemy;
        assert_eq!((got.col, got.blit_x, got.row, got.blit_y), (40, 40, 3, 30));
        assert_eq!((got.hp, got.spill), (5, 0x1234));
        assert_eq!(c.en.slots[0].enemy, Enemy::default(), "槽 0 不動");
    }

    /// 在 `cols` 欄的戰場生成 x 為 `x` 的樣板，回傳生成後的欄與繪製 x。
    fn spawned_col(cols: u16, x: i16, boss: bool) -> (u16, u16) {
        let mut c = core();
        c.enter_field(cols);
        put_template(&mut c, 0x7400, &Enemy { col: x as u16, row: 3, ..Enemy::default() });
        c.spawn_row(0x7400, boss);
        let e = c.en.slots[0].enemy;
        (e.col, e.blit_x)
    }

    #[test]
    fn a_wider_field_moves_right_edge_spawns_to_the_new_right_edge() {
        // 欄 40 以上（右緣與交錯進場）整體加上多出來的欄數：120% 加 8，150% 加 20。
        for (cols, extra) in [(48u16, 8i16), (60, 20)] {
            for x in [40i16, 41, 52, 84] {
                let (col, bx) = spawned_col(cols, x, false);
                assert_eq!((col as i16, bx as i16), (x + extra, x + extra), "{cols} 欄 x={x}");
            }
        }
    }

    #[test]
    fn a_wider_field_scales_inside_and_off_screen_spawns_in_proportion() {
        // 畫面內與左緣進場的列：x × cols ÷ 40，整數除法往零取整。
        for (cols, x, want) in [(48u16, 10i16, 12i16), (48, 38, 45), (60, 10, 15), (60, 38, 57), (60, 0, 0), (60, 39, 58), (60, -2, -3), (60, -10, -15), (48, -3, -3)] {
            let (col, _) = spawned_col(cols, x, false);
            assert_eq!(col as i16, want, "{cols} 欄 x={x}");
        }
    }

    #[test]
    fn a_wider_field_right_aligns_boss_rows_even_when_x_is_negative_or_inside() {
        // 頭目分件（合成圖本體的 x = -20、嵌在合成圖上的砲台）和合成圖一起靠右對齊。
        for (x, want) in [(-20i16, 0i16), (-19, 1), (19, 39), (7, 27), (34, 54)] {
            let (col, bx) = spawned_col(60, x, true);
            assert_eq!((col as i16, bx as i16), (want, want), "x={x}");
        }
    }

    #[test]
    fn a_normal_field_never_touches_the_spawn_column() {
        for x in [-20i16, -2, 0, 10, 39, 40, 84] {
            for boss in [false, true] {
                assert_eq!(spawned_col(40, x, boss).0 as i16, x);
            }
        }
        assert_eq!(c_none_emitted(), 0);
    }

    /// 欄數 40 接 40 不送 `Field`（命令串與原版相同）。
    fn c_none_emitted() -> usize {
        let mut c = core();
        c.draws.clear();
        c.enter_field(40);
        c.begin_stage_field();
        c.leave_field();
        c.draws.len()
    }

    #[test]
    fn the_boss_rows_of_each_timetable_are_the_documented_frames() {
        assert!(Timetable::Stage1.is_boss_row(623) && !Timetable::Stage1.is_boss_row(585));
        for f in [263, 264, 347, 348, 630] {
            assert!(Timetable::Stage2.is_boss_row(f), "{f}");
        }
        for f in [24, 290, 300, 320, 350, 620] {
            assert!(!Timetable::Stage2.is_boss_row(f), "{f}");
        }
        assert!(Timetable::Stage3.is_boss_row(1133) && !Timetable::Stage3.is_boss_row(1000));
        assert!(Timetable::Stage4.is_boss_row(1060) && !Timetable::Stage4.is_boss_row(720));
        assert!(!Timetable::Ending.is_boss_row(1));
    }

    #[test]
    fn spawn_with_a_full_pool_is_dropped() {
        let mut c = core();
        for i in 0..POOL_SIZE {
            c.en.slots[i].link = 1;
        }
        let before = c.en.clone().slots;
        put_template(&mut c, 0x7400, &Enemy { hp: 9, ..Enemy::default() });
        c.spawn_enemy(0x7400);
        assert_eq!(c.en.slots, before);
    }

    #[test]
    fn spawn_wraps_a_negative_row_into_pixels_modulo_16_bits() {
        let mut c = core();
        put_template(&mut c, 0x7400, &Enemy { row: (-3i16) as u16, ..Enemy::default() });
        c.spawn_enemy(0x7400);
        assert_eq!(c.en.slots[0].enemy.blit_y, (-30i16) as u16);
    }

    #[test]
    fn timetable_spawns_every_row_that_matches_the_frame() {
        let mut c = core();
        put_template(&mut c, 0x7400, &Enemy { hp: 1, ..Enemy::default() });
        put_template(&mut c, 0x7500, &Enemy { hp: 2, ..Enemy::default() });
        // 表不依幀排序，同一幀可以有多列。
        c.tables_mut().put_timetable(Timetable::Stage2, &[(30, 0x7400), (10, 0x7500), (30, 0x7500)]);
        c.set_stage_frame(30);
        c.spawn_scheduled(Timetable::Stage2);
        assert_eq!((c.en.slots[0].enemy.hp, c.en.slots[1].enemy.hp), (1, 2));
        assert!(!c.en.slots[2].is_active());
        c.set_stage_frame(10);
        c.spawn_scheduled(Timetable::Stage2);
        assert_eq!(c.en.slots[2].enemy.hp, 2);
        c.set_stage_frame(31);
        c.spawn_scheduled(Timetable::Stage2);
        assert!(!c.en.slots[3].is_active());
    }

    #[test]
    fn spawn_bumps_the_generation_so_a_reused_slot_is_a_new_object() {
        let mut c = core();
        c.tables_mut().put_enemy(0x7400, Enemy::default());
        c.spawn_enemy(0x7400);
        let before = c.gens[Pool::Entity as usize][0];
        c.en.slots[0].link = 0;
        c.spawn_enemy(0x7400);
        assert_eq!(c.gens[Pool::Entity as usize][0], before + 1);
    }

    #[test]
    fn draw_pass_one_paints_mask_then_shifted_planes() {
        let mut c = core();
        c.en.slots[2] = Slot { link: 1, sprite: Sprite { image: 7, x: 12, y: 80, kind: u16::from(b'O') }, flash: NO, ..Slot::default() };
        c.draw_enemies();
        let blits: Vec<_> = c.draws.iter().filter_map(|d| if let DrawCmd::Blit(b) = d { Some(*b) } else { None }).collect();
        assert_eq!(blits.iter().map(|b| b.op).collect::<Vec<_>>(), [Op::StageCut, Op::StageOverlay]);
        assert_eq!((blits[0].x, blits[0].y, blits[0].index), (12, 80, 7));
        assert_eq!((blits[1].x, blits[1].y, blits[1].index), (12, 81, 7), "平面的 y 加 1");
        assert!(blits.iter().all(|b| b.obj.map(|o| (o.pool, o.slot)) == Some((Pool::Entity, 2))));
    }

    #[test]
    fn draw_pass_one_flash_frame_paints_only_the_flash_mask_after_the_mask() {
        let mut c = core();
        c.en.slots[0] = Slot { link: 1, sprite: Sprite { image: 7, x: 12, y: 80, kind: u16::from(b'O') }, flash: YES, ..Slot::default() };
        c.draw_enemies();
        let ops: Vec<_> = c.draws.iter().filter_map(|d| if let DrawCmd::Blit(b) = d { Some(b.op) } else { None }).collect();
        assert_eq!(ops, [Op::StageCut, Op::StageFlash]);
    }

    #[test]
    fn draw_pass_one_skips_hidden_embedded_overlay_and_free_slots() {
        let mut c = core();
        for (i, kind) in [b'N', b'K', b'S'].into_iter().enumerate() {
            c.en.slots[i] = Slot { link: 1, sprite: Sprite { kind: u16::from(kind), ..Sprite::default() }, ..Slot::default() };
        }
        c.en.slots[3] = Slot { link: 0, sprite: Sprite { kind: u16::from(b'O'), ..Sprite::default() }, ..Slot::default() };
        c.draw_enemies();
        assert!(c.draws.is_empty());
    }

    #[test]
    fn draw_pass_two_paints_only_overlay_planes_without_a_mask() {
        let mut c = core();
        c.en.slots[0] = Slot { link: 1, sprite: Sprite { image: 3, x: 5, y: 60, kind: u16::from(b'S') }, ..Slot::default() };
        c.en.slots[1] = Slot { link: 1, sprite: Sprite { image: 4, x: 5, y: 60, kind: u16::from(b'O') }, ..Slot::default() };
        c.en.slots[2] = Slot { link: 1, sprite: Sprite { image: 4, x: 5, y: 60, kind: u16::from(b'K') }, ..Slot::default() };
        c.draw_enemy_overlays();
        let blits: Vec<_> = c.draws.iter().filter_map(|d| if let DrawCmd::Blit(b) = d { Some(*b) } else { None }).collect();
        assert_eq!(blits.len(), 1);
        assert_eq!((blits[0].op, blits[0].x, blits[0].y, blits[0].index), (Op::StageOverwrite, 5, 60, 3));
    }

    #[test]
    fn draws_use_the_stored_sprite_not_the_live_record() {
        let mut c = core();
        let e = Enemy { blit_x: 99, blit_y: 99, ..Enemy::default() };
        c.en.slots[0] = Slot { link: 1, enemy: e, sprite: Sprite { image: 1, x: 2, y: 3, kind: u16::from(b'O') }, flash: NO };
        c.draw_enemies();
        let Some(DrawCmd::Blit(b)) = c.draws.first().copied() else { panic!() };
        assert_eq!((b.x, b.y), (2, 3));
    }

    #[test]
    fn pool_word_index_runs_past_the_links_into_the_records() {
        let mut en = Enemies::default();
        en.slots[3].link = enemy_record_link(3);
        en.spare_link = 0x5A5A;
        en.slots[0].enemy.hp = 0x1111;
        en.slots[0].enemy.spill = 0x2222;
        en.slots[1].enemy.hp = 0x3333;
        en.slots[9].enemy.anim = 0x4444;
        assert_eq!(en.word_at(3), enemy_record_link(3));
        assert_eq!(en.word_at(15), 0x5A5A, "備用字組");
        assert_eq!(en.word_at(16), 0x1111, "索引 16 是槽 0 的 w0");
        assert_eq!(en.word_at(16 + 23), 0x2222, "槽 0 的 w23");
        assert_eq!(en.word_at(16 + 24), 0x3333, "每 24 項換一槽");
        assert_eq!(en.word_at(0xFD), 0x4444, "裝甲格的索引 FDh 是槽 9 的 anim");
        assert_eq!(en.word_at(0xFF), 0, "最大索引 FFh 是槽 9 的 w23");
    }

    #[test]
    fn damage_is_added_to_the_record_with_the_flag_set_and_no_carry() {
        let mut c = core();
        let link = EntityLink::Slot(4);
        c.en.slots[4].link = enemy_record_link(4);
        c.en.slots[4].enemy.damage = 0x0203;
        assert_eq!(c.add_entity_damage(link, 3), 0x0106, "高位元組設 1，低位元組相加");
        assert_eq!(c.en.slots[4].enemy.damage, 0x0106);
        c.en.slots[4].enemy.damage = 0x01FF;
        c.add_entity_damage(link, 2);
        assert_eq!(c.en.slots[4].enemy.damage, 0x0101, "低位元組環繞，不進位");
    }

    #[test]
    fn damage_to_link_zero_lands_at_dseg_0020() {
        let mut c = core();
        c.en.stray_damage = 0x00FF;
        assert_eq!(c.add_entity_damage(EntityLink::Stray(0), 2), 0x0101);
        assert_eq!(c.en.stray_damage, 0x0101, "指標 0 加 20h：dseg:0020h");
        assert!(c.en.slots.iter().all(|s| s.enemy.damage == 0), "沒有動到任何敵人");
        // 不是任何一槽起點、也不是 0 的指標：原版寫到別處，重製丟掉寫入、讀回 0（見 `add_entity_damage`）。
        c.add_entity_damage(EntityLink::Stray(0x7200), 1);
        assert_eq!(c.entity_damage(EntityLink::Stray(0x7200)), 0);
        assert_eq!(c.en.stray_damage, 0x0101);
    }

    #[test]
    fn entity_link_reads_the_pool_without_a_range_check() {
        let mut c = core();
        c.en.slots[3].link = enemy_record_link(3);
        assert_eq!(c.entity_link(3), EntityLink::Slot(3));
        assert_eq!(c.entity_link(4), EntityLink::Stray(0), "空槽讀到 0");
        c.en.slots[9].enemy.anim = 0x1111;
        assert_eq!(c.entity_link(0xFD), EntityLink::Stray(0x1111), "索引 FDh 讀到池之後的記錄區");
    }

    #[test]
    fn release_all_clears_links_and_the_spare_but_keeps_records() {
        let mut en = Enemies::default();
        en.slots[4] = Slot { link: 0x1234, enemy: Enemy { hp: 9, ..Enemy::default() }, sprite: Sprite { image: 5, ..Sprite::default() }, flash: YES };
        en.spare_link = 7;
        en.release_all();
        assert_eq!(en.slots[4], Slot { link: 0, enemy: Enemy { hp: 9, ..Enemy::default() }, sprite: Sprite { image: 5, ..Sprite::default() }, flash: YES });
        assert_eq!(en.spare_link, 0);
        assert!(en.slots.iter().all(|s| !s.is_active()));
    }

    /// 真實時間表逐列盤點（景物綁定敵人的調查）：每列的幀、x、列、模式字母、畫法字母、耐久、單步與腳本前 24 個位元組。
    /// 需要 `RS2_ORIG`：`tools/rust.sh test -p rs2-game entity::tests::real_timetables_inventory -- --ignored --nocapture`。
    #[test]
    #[ignore]
    fn real_timetables_inventory() {
        let dir = std::path::PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh 跑"));
        let image = rs2_formats::unpack_s_exe(&std::fs::read(dir.join("S.EXE")).unwrap()).unwrap();
        let tables = crate::tables::Tables::parse(&image).unwrap();
        for (name, table) in [("S1", Timetable::Stage1), ("S2", Timetable::Stage2), ("S3", Timetable::Stage3), ("S4", Timetable::Stage4), ("END", Timetable::Ending)] {
            for &(frame, template) in tables.timetable(table) {
                let e = tables.enemy_template(template);
                let mut script = String::new();
                let mut at = e.script;
                for _ in 0..24 {
                    let Some(b) = tables.script_byte_opt(at) else {
                        script.push_str("-- ");
                        break;
                    };
                    script.push_str(&format!("{b:02X} "));
                    at = at.wrapping_add(1);
                    if b == CODE_DIE {
                        break;
                    }
                }
                eprintln!(
                    "[inv] {name} 幀 {frame} 樣板 {template:#06x} x {} 列 {} 模式 {} 型 {} hp {} bullet {} dcol {} bdx {} boss {} 腳本 {script}",
                    e.col as i16,
                    e.row as i16,
                    e.mode as u8 as char,
                    e.kind as u8 as char,
                    e.hp,
                    e.bullet as u8 as char,
                    e.dcol,
                    e.blit_dx,
                    table.is_boss_row(frame)
                );
            }
        }
    }

    /// 真實時間表的平移統計（規格 005「戰場大小」的生成規則）：每關的列數依 x 分類，頭目列與其他列的數量。
    /// 需要 `RS2_ORIG`：`tools/rust.sh test -p rs2-game entity::tests::real_ -- --ignored --nocapture`。
    #[test]
    #[ignore]
    fn real_timetables_spawn_shift_statistics() {
        let dir = std::path::PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh 跑"));
        let image = rs2_formats::unpack_s_exe(&std::fs::read(dir.join("S.EXE")).unwrap()).unwrap();
        let tables = crate::tables::Tables::parse(&image).unwrap();
        let mut c = core();
        *c.tables_mut() = tables;
        c.enter_field(60);
        let mut grand = [0usize; 5];
        for (name, table) in [("第一關", Timetable::Stage1), ("第二關", Timetable::Stage2), ("第三關", Timetable::Stage3), ("第四關", Timetable::Stage4), ("片尾", Timetable::Ending)] {
            // 類別：頭目列、x 在 40 以上（平移）、x 在 0 至 39（等比放大）、x 為負（等比放大）。
            let (mut boss, mut right, mut inside, mut negative, mut total) = (0, 0, 0, 0, 0);
            let mut max_x = i16::MIN;
            let mut right_xs = std::collections::BTreeSet::new();
            for &(frame, template) in c.tables().timetable(table) {
                let x = c.tables().enemy_template(template).col as i16;
                total += 1;
                max_x = max_x.max(x);
                if table.is_boss_row(frame) {
                    boss += 1;
                } else if x >= 40 {
                    right += 1;
                    right_xs.insert(x);
                } else if x >= 0 {
                    inside += 1;
                } else {
                    negative += 1;
                }
            }
            eprintln!("[spawn] {name}：共 {total} 列，頭目 {boss}、右緣以右（平移）{right}、畫面內（等比）{inside}、負值（等比）{negative}，最大 x {max_x}，右側 x 種類 {right_xs:?}");
            for (g, v) in grand.iter_mut().zip([total, boss, right, inside, negative]) {
                *g += v;
            }
        }
        eprintln!("[spawn] 全部：共 {} 列，頭目 {}、平移 {}、等比（畫面內）{}、等比（負值）{}", grand[0], grand[1], grand[2], grand[3], grand[4]);
        assert!(grand[0] > 100, "時間表有內容");
    }
}
