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
    pub(crate) fn spawn_scheduled(&mut self, table: Timetable) {
        let tables = self.tables();
        for &(frame, template) in tables.timetable(table) {
            if frame == self.stage_frame() {
                self.spawn_enemy(template);
            }
        }
    }

    /// 把樣板複製進第一個空槽（原版 `sub_15D41`），繪製位置跟著邏輯位置設：x 取欄，y 取列乘 10。
    /// 沒有空槽就放棄。傷害字組、動畫計數與亂數索引都是樣板的值，所以每個敵人的亂數序列從頭開始。
    pub(crate) fn spawn_enemy(&mut self, template: u16) {
        let Some(slot) = self.en.first_free() else { return };
        let mut enemy = self.enemy_template(template);
        enemy.blit_x = enemy.col;
        enemy.blit_y = enemy.row.wrapping_mul(ROW_PIXELS);
        let s = &mut self.en.slots[slot];
        s.link = enemy_record_link(slot);
        s.enemy = enemy;
        self.bump_gen(Pool::Entity, slot as u8);
        // AI 強化只認得生成表生成的敵人（`ai.rs`）；這一步不碰資料段。
        self.ai.on_spawn(slot, template);
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
        let flashing = match self.resolve_damage(enemy) {
            Damage::Killed => return Step { flashing: false, dying: true },
            Damage::Flash => true,
            Damage::None => false,
        };
        let rules = self.rules();
        self.ai.tick_enemy(slot);
        let code = match enemy.mode() {
            Mode::Chase => {
                let target = self.aim_heading(enemy.col, enemy.row);
                enemy.script = turn_toward(target, enemy.script);
                enemy.script as u8
            }
            Mode::Script | Mode::Scroll => {
                let code = self.read_command(enemy);
                // 敵人會反應（`ai_react`）：只換這一幀要執行的移動碼，腳本照常前進。
                if rules.ai_react { self.ai_react(enemy, slot, code) } else { code }
            }
        };
        let dying = self.apply_motion(enemy, code);
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
        assert_eq!((blits[0].op, blits[0].x, blits[0].y, blits[0].index, blits[0].scratch), (Op::S19ADC, 39, 10, 19, SCRATCH_B));
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
        assert_eq!(blits.iter().map(|b| b.op).collect::<Vec<_>>(), [Op::S18F1A, Op::S18F5D]);
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
        assert_eq!(ops, [Op::S18F1A, Op::S19025]);
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
        assert_eq!((blits[0].op, blits[0].x, blits[0].y, blits[0].index), (Op::S18FC1, 5, 60, 3));
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
}
