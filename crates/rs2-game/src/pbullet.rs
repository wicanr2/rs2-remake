//! 玩家子彈：發射、移動、繪製、碰撞與命中結算。
//!
//! 池有 20 槽。每槽一筆記錄（[`PlayerBullet`]，原版 16 個字組）加一個池指標；指標為 0 表示空槽，
//! 使用中時等於這一槽記錄的位址。發射（原版 `sub_165CF`、`sub_1669F`）依武器字母與朝向查
//! [`Weapon::steps`] 的步驟表，每一步從樣板表複製一筆記錄、填入起點（原版 `sub_16578`）。
//! 每幀（原版 `sub_1673E`）對每個使用中的槽讀一個腳本位元組、移動、畫圖、寫格網 G3、
//! 對 G1（實體）與 G4（第三關地形）做碰撞、結算命中，出界或命中就釋放。
//! 語意見 `docs/re/11-entities-combat.md` 第 5、6 節。
//!
//! 狀態與資料段位移的對應在 `mirror.rs`，這個檔案不出現原版位移。
//!
//! 對外的入口：
//!
//! | 入口 | 原版 |
//! |---|---|
//! | [`Core::fire_player_bullets`] | `sub_165CF`（朝右）、`sub_1669F`（朝左）。呼叫端：主機普通彈 `sub_14501`、`sub_1502F`，砲座 `sub_1480A`、`sub_15338` |
//! | [`Core::fire_sweep`] | `sub_165CF` 的 `F` 分支。呼叫端：主機放開右 Shift 的大掃射 `sub_14501`、`sub_1502F` |
//! | [`Core::fire_wave`] | 沒有原版對應。波動砲（規格 005），呼叫端 `player/enhanced.rs` |
//! | [`Core::update_player_bullets`] | `sub_1673E`（各關每幀） |
//! | [`PlayerBullets::release_all`] | `sub_10E33` 內清池指標表的那一段 |
//!
//! 武器派發表（[`Weapon`]、[`Step`]、[`Shot`]）與子彈的傷害、穿透（[`Behavior`]）各自獨立，
//! 之後的強化模式（規格 005）只換這幾處。
//!
//! 已查證不移植的碼：`sub_16C7D`（命中計數器，每次碰到格子都會呼叫）只有在記錄的 `owner` 是
//! `1`（31h）或 `2`（32h）時才有作用，而 `owner` 取自發射時的武器字母（`word_1F1BC`）。`word_1F1BC` 的寫入端是
//! `sub_14501`（`1460Ch` 寫 `F`、`14661h` 寫 0）、`sub_1502F`（`1513Ah`、`1518Fh`）、
//! `sub_1480A`（`1488Bh`、`148AAh`，值取自 `word_1DAA9`）與 `sub_15338`（`153B9h`、`153D8h`，值取自 `word_1DB0B`）；
//! 後兩個武器字組的寫入端只寫 `L`、`B`、`K`、`S`、0（`14D2Ah` 至 `14D3Fh`、`14F73h`、`15858h` 至 `1586Dh`、`158E6h`）。
//! 所以 `owner` 不會是 31h 或 32h，那兩個計數器（`word_1F1B8`、`word_1F1BA`）從不被寫入。
//! `sub_165CF` 在 `165FCh` 至 `16619h` 的雙彈碼前面是 `retn`，無跳入，也不移植。

use crate::draw::Pool;
use crate::entity::ARMOR_CELL;
use crate::fx::{EffectKind, loop_count};
use crate::game::Core;
#[cfg(test)]
use crate::grid::GRID1_ROW_BYTES;
use crate::grid::GRID1_START;
use crate::video::MyCopy;

/// 池的槽數。
pub const POOL_SIZE: usize = 20;

/// 出界門檻（無符號）：列 >= 12h、欄 >= 27h。往上與往下共用同一個列門檻（敵彈往上是 13h，這裡不同）。
const ROW_LIMIT: u16 = 0x12;
const COL_LIMIT: u16 = 0x27;
const _: () = assert!(COL_LIMIT == crate::grid::GRID_COLS - 1);

/// 腳本位元組低 4 位元為 0Ah：追蹤指令。
const SEEK_CODE: u8 = 0x0A;
/// 腳本位元組高 4 位元為 Fh：往回跳，低 4 位元是往回的位元組數。
const JUMP_BACK: u8 = 0xF0;

/// 同時在飛的追蹤彈超過這個數就不再發射（原版 `cmp word_1F1A8, 5; ja`）。
const SEEKER_LIMIT: u16 = 5;
/// 追蹤彈沿某方向找不到格的距離。
const NOT_FOUND: u16 = 0xFF;

/// 命中結算的目標：沒有實體要扣血（命中地形，或雷射沒打到東西），仍播命中特效。
const NO_ENTITY: u8 = 0xFF;

/// 雷射光束的圖：繪製頁為 0 用圖 4，否則用圖 19h。
const BEAM_IMAGE: u16 = 4;
const BEAM_IMAGE_ODD_PAGE: u16 = 0x19;

/// 玩家機與砲座的朝向。原版以字組 `>`（3Eh）表示朝右，玩家朝左時是 `<`（3Ch），判斷只看是不是 3Eh。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Facing {
    Right,
    Left,
}

impl Facing {
    pub(crate) fn from_word(word: u16) -> Facing {
        if word == u16::from(b'>') { Facing::Right } else { Facing::Left }
    }
}

/// 武器種類，由發射時的字母決定。字母同時寫進記錄的 `owner`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Weapon {
    /// 其他字母（含 0）：主機的普通彈。
    Plain,
    /// `K`：4×3 大彈。
    Big,
    /// `S`：兩顆，前 3 幀分別斜上、斜下，之後直線。
    Spread,
    /// `B`：兩顆追蹤彈。
    Seeker,
    /// `L`：雷射。
    Laser,
    /// `F`：主機放開右 Shift 的大掃射，六顆穿透彈。
    Sweep,
}

impl Weapon {
    /// 原版派發的比較順序是 `K`、`S`、`B`、`L`、`F`，其餘為普通彈。
    pub(crate) fn from_letter(letter: u16) -> Weapon {
        match letter {
            0x4B => Weapon::Big,
            0x53 => Weapon::Spread,
            0x42 => Weapon::Seeker,
            0x4C => Weapon::Laser,
            0x46 => Weapon::Sweep,
            _ => Weapon::Plain,
        }
    }

    /// 這種武器朝這個方向依序要做的事（原版 `sub_165CF` 與 `sub_1669F` 的分支）。
    pub(crate) fn steps(self, facing: Facing) -> &'static [Step] {
        match (self, facing) {
            (Weapon::Plain, _) => PLAIN,
            (Weapon::Big, _) => BIG,
            (Weapon::Spread, Facing::Right) => SPREAD_RIGHT,
            (Weapon::Spread, Facing::Left) => SPREAD_LEFT,
            (Weapon::Seeker, _) => SEEKERS,
            (Weapon::Laser, Facing::Right) => LASER_RIGHT,
            (Weapon::Laser, Facing::Left) => LASER_LEFT,
            (Weapon::Sweep, _) => SWEEP,
        }
    }
}

/// 樣板表裡的一筆記錄，生成時複製進池。每種有朝右與朝左兩份（掃射兩邊共用同一份向右的），
/// 樣板的內容在 `tables.rs`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Shot {
    /// 普通彈：每幀 4 欄，直線。
    Plain,
    /// 4×3 大彈（圖 5 與 60）：每幀 5 欄，直線。
    Big,
    /// `S` 武器的第一顆：腳本前 3 幀斜上。
    SpreadUp,
    /// `S` 武器的第二顆：腳本前 3 幀斜下。
    SpreadDown,
    /// `B` 武器的第一顆追蹤彈，初始朝向右上。
    SeekerUp,
    /// `B` 武器的第二顆追蹤彈，初始朝向右下。
    SeekerDown,
    /// 雷射，沿列掃描，生成當幀就畫光束並釋放。
    Laser,
    /// 大掃射的穿透彈，傷害 3。
    Sweep,
}

/// 武器圖案的一步。起點（欄、列）從呼叫端給的位置起算，`Shift` 與 `SetRow` 累積往後的步驟。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// 起點平移。
    Shift { cols: i16, rows: i16 },
    /// 起點列改成固定值（掃射從列 1 起）。
    SetRow(u16),
    /// 從樣板生成一顆。池滿了這顆放棄，後面的步驟照常做。
    Fire(Shot),
    /// 在飛的追蹤彈超過 [`SEEKER_LIMIT`] 就放棄整個圖案。
    SeekerGate,
    /// 在飛的追蹤彈計數加上這個數。原版不看前面的 `Fire` 是否真的配到槽，一律加 2；
    /// `Fixes::seeker_spawn_leak` 開啟時改加實際配到的顆數。
    CountSeekers(u16),
}

const fn shift(cols: i16, rows: i16) -> Step {
    Step::Shift { cols, rows }
}

const fn fire(shot: Shot) -> Step {
    Step::Fire(shot)
}

const PLAIN: &[Step] = &[shift(0, 1), fire(Shot::Plain)];
const BIG: &[Step] = &[fire(Shot::Big)];
/// 朝右列 +1（`1661Ah`），朝左不修正（`166D7h`）。
const SPREAD_RIGHT: &[Step] = &[shift(0, 1), fire(Shot::SpreadUp), fire(Shot::SpreadDown)];
const SPREAD_LEFT: &[Step] = &[fire(Shot::SpreadUp), fire(Shot::SpreadDown)];
const SEEKERS: &[Step] = &[shift(0, 1), Step::SeekerGate, fire(Shot::SeekerUp), fire(Shot::SeekerDown), Step::CountSeekers(2)];
/// 雷射：右 +3 欄、左 -2 欄，列 +1。
const LASER_RIGHT: &[Step] = &[shift(3, 1), fire(Shot::Laser)];
const LASER_LEFT: &[Step] = &[shift(-2, 1), fire(Shot::Laser)];
/// 掃射：列 1、4、7、10、13、16 各一顆，欄不變。
const SWEEP: &[Step] = &[
    Step::SetRow(1),
    fire(Shot::Sweep),
    shift(0, 3),
    fire(Shot::Sweep),
    shift(0, 3),
    fire(Shot::Sweep),
    shift(0, 3),
    fire(Shot::Sweep),
    shift(0, 3),
    fire(Shot::Sweep),
    shift(0, 3),
    fire(Shot::Sweep),
];

/// 子彈的行為，由記錄的 `kind` 字組決定。傷害與穿透都從這裡取。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Behavior {
    /// 0 與其他值：直線飛行，傷害 1，命中消失。
    Plain,
    /// `A`：追蹤彈，傷害 3，消失時在飛計數減 1。
    Seeker,
    /// `L`：朝右的雷射。
    LaserRight,
    /// `l`：朝左的雷射。
    LaserLeft,
    /// `S`：掃射，傷害 3，命中不消失。
    Piercing,
    /// `W`：波動砲第一級（規格 005），不屬於原版。傷害 [`WAVE_DAMAGE`] 第 0 項，命中消失。
    WaveBolt,
    /// `w`：波動砲第二級（規格 005），不屬於原版。傷害 [`WAVE_DAMAGE`] 第 1 項，命中不消失。
    WaveLance,
}

impl Behavior {
    fn from_word(kind: u16) -> Behavior {
        match kind {
            0x41 => Behavior::Seeker,
            0x4C => Behavior::LaserRight,
            0x6C => Behavior::LaserLeft,
            0x53 => Behavior::Piercing,
            KIND_WAVE_BOLT => Behavior::WaveBolt,
            KIND_WAVE_LANCE => Behavior::WaveLance,
            _ => Behavior::Plain,
        }
    }

    /// 命中實體時累計的傷害（原版 `16962h` 至 `16972h`：型別 `S`、`A` 先加 2 再加 1）。
    pub(crate) const fn damage(self) -> u8 {
        match self {
            Behavior::Seeker | Behavior::Piercing => 3,
            Behavior::Plain | Behavior::LaserRight | Behavior::LaserLeft => 1,
            Behavior::WaveBolt => WAVE_DAMAGE[0],
            Behavior::WaveLance => WAVE_DAMAGE[1],
        }
    }

    /// 命中實體後是否繼續飛（原版 `16B8Fh`：型別 `S` 不設釋放旗標）。
    pub(crate) const fn pierces(self) -> bool {
        matches!(self, Behavior::Piercing | Behavior::WaveLance)
    }
}

// ---- 波動砲（規格 005「射擊」，remake-owned，沒有原版收據）----

/// 波動砲子彈記錄的 `owner` 字母。原版的字母只有 0、`L`、`B`、`K`、`S`、`F`，邏輯不讀 `owner`。
pub(crate) const WAVE_LETTER: u16 = b'W' as u16;
/// 第一級、第二級子彈記錄的行為字組（原版只有 0、`A`、`L`、`l`、`S`）。
const KIND_WAVE_BOLT: u16 = b'W' as u16;
const KIND_WAVE_LANCE: u16 = b'w' as u16;
/// 第一級、第二級命中實體時累計的傷害。初值，規格 005「未決定」，使用者試玩後調。
/// 第三級沿用大掃射的穿透彈，傷害 3（[`Behavior::Piercing`]）。
pub(crate) const WAVE_DAMAGE: [u8; 2] = [2, 3];
/// 第三級七顆穿透彈的起點列，每顆形狀 3 列，合起來涵蓋列 0 至 18，只有列 17 與前一顆重疊一列。
/// 原版大掃射是 1、4、7、10、13、16（涵蓋列 1 至 18，漏掉列 0）。最後一顆是 17 不是 18：
/// 列 18 的形狀會讀到列 20，落在 G1 的越界列（見 [`Core::grid1_row_start`]）。
const WAVE_FULL_ROWS: [u16; 7] = [0, 3, 6, 9, 12, 15, 17];

/// 池指標：第 `slot` 槽記錄的記號，0 是空槽。記號只在遊戲裡用，不是原版的記錄位址。
pub const fn pb_record_link(slot: usize) -> u16 {
    slot as u16 + 1
}

/// 一顆玩家子彈的記錄。欄位是原版的 16 個字組，數值沿用原版字組寬度。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerBullet {
    /// 行為字組：0 普通、`A` 追蹤、`L`、`l` 雷射、`S` 穿透，見 [`PlayerBullet::behavior`]。
    pub kind: u16,
    /// `MY.GO` 圖庫索引。
    pub image: u16,
    /// 繪製位置，單位是位元組（8 像素）。
    pub blit_x: u16,
    /// 繪製位置，單位是像素列。
    pub blit_y: u16,
    /// 每步的繪製位移：x（位元組）、y（像素列）。
    pub blit_dx: u16,
    pub blit_dy: u16,
    /// 追蹤彈的朝向：樣板給 2、4、6、8（斜向），選定方向後存 1、3、5、7。
    pub heading: u16,
    /// 發射時的武器字母（原版 `word_1F1BC`），邏輯不讀。
    pub owner: u16,
    /// 沒有程式碼讀寫的字組（所有樣板都是 0），只在複製記錄時帶著走。
    pub spare: u16,
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
    /// 命中特效字母：`>`、`<`、`B`，見 [`EffectKind::from_letter`]。
    pub hit_effect: u16,
}

impl PlayerBullet {
    /// 記錄的欄位順序（原版 16 個字組）：`PlayerBullet` 的欄位依序對應 `w0` 至 `w15`。
    pub fn from_words(w: [u16; 16]) -> PlayerBullet {
        PlayerBullet {
            kind: w[0],
            image: w[1],
            blit_x: w[2],
            blit_y: w[3],
            blit_dx: w[4],
            blit_dy: w[5],
            heading: w[6],
            owner: w[7],
            spare: w[8],
            col: w[9],
            row: w[10],
            dcol: w[11],
            drow: w[12],
            script: w[13],
            shape: w[14],
            hit_effect: w[15],
        }
    }

    pub fn to_words(&self) -> [u16; 16] {
        [
            self.kind,
            self.image,
            self.blit_x,
            self.blit_y,
            self.blit_dx,
            self.blit_dy,
            self.heading,
            self.owner,
            self.spare,
            self.col,
            self.row,
            self.dcol,
            self.drow,
            self.script,
            self.shape,
            self.hit_effect,
        ]
    }

    pub(crate) fn behavior(&self) -> Behavior {
        Behavior::from_word(self.kind)
    }

    /// 往上（原版 `sub_1688E`）：繪製 y 減 `blit_dy`、列減 `drow`。回傳是否出界。
    fn move_up(&mut self) -> bool {
        self.blit_y = self.blit_y.wrapping_sub(self.blit_dy);
        self.row = self.row.wrapping_sub(self.drow);
        self.row >= ROW_LIMIT
    }

    /// 往下（原版 `sub_168B1`）。
    fn move_down(&mut self) -> bool {
        self.blit_y = self.blit_y.wrapping_add(self.blit_dy);
        self.row = self.row.wrapping_add(self.drow);
        self.row >= ROW_LIMIT
    }

    /// 往左（原版 `sub_168D4`）：繪製 x 減 `blit_dx`、欄減 `dcol`。`side` 是側邊出界門檻
    /// （原版 [`COL_LIMIT`]，戰場加大時是新的欄數減 1，見 `Core::side_limit`）。
    fn move_left(&mut self, side: u16) -> bool {
        self.blit_x = self.blit_x.wrapping_sub(self.blit_dx);
        self.col = self.col.wrapping_sub(self.dcol);
        self.col >= side
    }

    /// 往右（原版 `sub_168F7`）。
    fn move_right(&mut self, side: u16) -> bool {
        self.blit_x = self.blit_x.wrapping_add(self.blit_dx);
        self.col = self.col.wrapping_add(self.dcol);
        self.col >= side
    }

    /// 依移動碼走一步（原版 `sub_16817`）。碼 1、3、5、7 是上、右、下、左，2、4、6、8 是右上、右下、左下、左上，
    /// 其他碼（含追蹤指令未選出方向時的 0Ah）不動。斜向的兩段都會執行，出界旗標是兩段的聯集。
    /// 回傳是否出界：出界的這一幀仍會畫、寫格網、做碰撞，釋放在最後。
    fn advance(&mut self, code: u8, side: u16) -> bool {
        match code {
            1 => self.move_up(),
            3 => self.move_right(side),
            5 => self.move_down(),
            7 => self.move_left(side),
            2 => self.move_up() | self.move_right(side),
            4 => self.move_down() | self.move_right(side),
            6 => self.move_down() | self.move_left(side),
            8 => self.move_left(side) | self.move_up(),
            _ => false,
        }
    }
}

/// 池的一槽。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    /// 池指標表的項：0 表示空槽，使用中是這一槽的記號（只有生成會寫，見 [`pb_record_link`]）。邏輯只看是否為 0。
    pub link: u16,
    /// 記錄。空槽也保留舊內容：原版釋放只清指標，記錄原封不動，資料段比對會看到它。
    pub bullet: PlayerBullet,
}

impl Slot {
    pub(crate) fn is_active(&self) -> bool {
        self.link != 0
    }
}

/// 玩家子彈領域的全部狀態。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerBullets {
    pub slots: [Slot; POOL_SIZE],
    /// 在飛的追蹤彈數（原版 `word_1F1A8`）。`B` 武器發射時加，型別 `A` 的子彈釋放時減；
    /// 關卡初始化歸零（`sub_10E33` 的 `10E9Fh`）。超過 [`SEEKER_LIMIT`] 就不再發射追蹤彈。
    pub seekers_in_flight: u16,
}

impl PlayerBullets {
    fn first_free(&self) -> Option<usize> {
        self.slots.iter().position(|s| !s.is_active())
    }

    /// 關卡初始化：全部槽釋放（只清指標，記錄不動）。
    pub(crate) fn release_all(&mut self) {
        for s in &mut self.slots {
            s.link = 0;
        }
    }
}

/// 一顆子彈這一幀撞到什麼。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Impact {
    /// 命中結算的目標（原版 `byte_1F1C0`）：0 沒有，1 至 FDh 是實體格值（槽號加 1），[`NO_ENTITY`] 只播特效。
    target: u8,
    /// 子彈是否消失。
    vanishes: bool,
}

impl Core {
    /// 依武器字母在 (`col`, `row`) 發射（原版 `sub_165CF`、`sub_1669F`）。`letter` 同時寫進記錄的 `owner`；
    /// 池滿了該顆放棄，後面的步驟照常做。
    pub(crate) fn fire_player_bullets(&mut self, letter: u16, facing: Facing, col: u16, row: u16) {
        let (mut col, mut row) = (col, row);
        let mut spawned = 0u16;
        for step in Weapon::from_letter(letter).steps(facing) {
            match *step {
                Step::Shift { cols, rows } => {
                    col = col.wrapping_add(cols as u16);
                    row = row.wrapping_add(rows as u16);
                }
                Step::SetRow(r) => row = r,
                Step::Fire(shot) => {
                    if self.spawn_player_bullet(shot, facing, letter, col, row).is_some() {
                        spawned += 1;
                    }
                }
                Step::SeekerGate => {
                    if self.pb.seekers_in_flight > SEEKER_LIMIT {
                        return;
                    }
                }
                Step::CountSeekers(n) => {
                    let added = if self.fixes.seeker_spawn_leak { spawned } else { n };
                    self.pb.seekers_in_flight = self.pb.seekers_in_flight.wrapping_add(added);
                }
            }
        }
    }

    /// 主機放開右 Shift（或砲座同型的左 Shift）時的大掃射，從玩家所在的欄射出（原版 `sub_14501` 的 `14606h`）。
    /// 掃射從列 1 起，不看玩家的列，兩個朝向用同一份向右的樣板。
    pub(crate) fn fire_sweep(&mut self, col: u16) {
        self.fire_player_bullets(u16::from(b'F'), Facing::Right, col, 0);
    }

    /// 把樣板複製進第一個空槽，起點取 (`col`, `row`)，繪製位置跟著設，`owner` 取武器字母（原版 `sub_16578`）。
    /// 回傳配到的槽，池滿了是 `None`。
    ///
    /// 繪製位置 y 是列 × 10 加 `Enhanced::spawn_dy`。原版恆為 0；細步移動（規格 005）的半步位置讓機身與砲座
    /// 的繪製 y 比列起點多 5 像素，子彈只在顯示上跟著，碰撞仍用整數列。
    fn spawn_player_bullet(&mut self, shot: Shot, facing: Facing, letter: u16, col: u16, row: u16) -> Option<usize> {
        let slot = self.pb.first_free()?;
        let mut bullet = self.player_bullet_template(shot, facing);
        bullet.col = col;
        bullet.blit_x = col;
        bullet.row = row;
        bullet.blit_y = row.wrapping_mul(10).wrapping_add(self.enh.spawn_dy);
        bullet.owner = letter;
        self.pb.slots[slot] = Slot { link: pb_record_link(slot), bullet };
        self.bump_gen(Pool::PlayerBullet, slot as u8);
        Some(slot)
    }

    /// 波動砲（規格 005「射擊」）：依充能級數 `level`（1 至 3）射出，`col` 是玩家所在的欄，`row` 是玩家所在的列。
    ///
    /// | 級 | 子彈 | 傷害 | 命中 |
    /// |---|---|---|---|
    /// | 1 | 大彈樣板（`K` 武器的 4×3，每幀 5 欄），起點同大彈 | [`WAVE_DAMAGE`] 第 0 項 | 消失 |
    /// | 2 | 同上 | [`WAVE_DAMAGE`] 第 1 項 | 穿透 |
    /// | 3 | 大掃射的穿透彈樣板，七顆從列 0 起每 3 列一顆（[`WAVE_FULL_ROWS`]），列不看玩家 | 3 | 穿透 |
    ///
    /// 池滿了的那一顆放棄，和原版的發射一樣。
    pub(crate) fn fire_wave(&mut self, level: u8, col: u16, row: u16) {
        match level {
            0 => {}
            1 | 2 => {
                let kind = if level == 1 { KIND_WAVE_BOLT } else { KIND_WAVE_LANCE };
                if let Some(slot) = self.spawn_player_bullet(Shot::Big, Facing::Right, WAVE_LETTER, col, row) {
                    self.pb.slots[slot].bullet.kind = kind;
                }
            }
            _ => {
                // 列是畫面上的絕對列，不跟著機身的半列位移。
                let dy = std::mem::take(&mut self.enh.spawn_dy);
                for r in WAVE_FULL_ROWS {
                    self.spawn_player_bullet(Shot::Sweep, Facing::Right, WAVE_LETTER, col, r);
                }
                self.enh.spawn_dy = dy;
            }
        }
    }

    /// 每幀更新 20 顆玩家子彈（原版 `sub_1673E`）：每個使用中的槽走一步，出界或命中就釋放，
    /// 釋放追蹤彈時在飛計數減 1。
    pub(crate) fn update_player_bullets(&mut self) {
        for slot in 0..POOL_SIZE {
            if !self.pb.slots[slot].is_active() {
                continue;
            }
            self.set_obj(Pool::PlayerBullet, slot as u8);
            let mut bullet = self.pb.slots[slot].bullet;
            let release = self.step_player_bullet(&mut bullet);
            self.pb.slots[slot].bullet = bullet;
            if release {
                self.pb.slots[slot].link = 0;
                if bullet.behavior() == Behavior::Seeker {
                    self.pb.seekers_in_flight = self.pb.seekers_in_flight.wrapping_sub(1);
                }
            }
        }
        self.clear_obj();
    }

    /// 一顆子彈的一幀（原版 `sub_167A3`）。回傳是否要釋放。雷射當幀畫完光束就釋放。
    fn step_player_bullet(&mut self, bullet: &mut PlayerBullet) -> bool {
        match bullet.behavior() {
            Behavior::LaserRight => {
                self.fire_laser(bullet, Facing::Right);
                true
            }
            Behavior::LaserLeft => {
                self.fire_laser(bullet, Facing::Left);
                true
            }
            _ => {
                let code = self.next_code(bullet);
                let out_of_field = bullet.advance(code, self.side_limit());
                let vanishes = self.draw_and_collide(bullet);
                out_of_field | vanishes
            }
        }
    }

    /// 讀這一幀的移動碼（原版 `sub_167A3` 前半、`sub_167E9`、`sub_16873`）。
    ///
    /// 腳本位元組的低 4 位元是碼，指標前進一格。碼為 0Ah 時由追蹤彈選方向（[`Core::steer_seeker`]），
    /// 結果取代整個字組，高位元組也跟著換。高 4 位元為 Fh 時，碼是往回跳的位元組數：指標（已前進）減去它，
    /// 改以該處位元組的低 4 位元為碼，指標不再前進，所以被跳回的那個位元組在跳回幀與下一幀連續生效兩次。
    /// 追蹤彈腳本最後的 `F5` 那一幀，追蹤判斷在跳回之前，所以跳回後的 0Ah 不會再選方向，該幀不移動。
    fn next_code(&mut self, bullet: &mut PlayerBullet) -> u8 {
        let byte = self.tables.script_byte(bullet.script);
        bullet.script = bullet.script.wrapping_add(1);
        let (mut code, mut jump) = (byte & 0x0F, byte & 0xF0);
        if code == SEEK_CODE {
            let word = self.steer_seeker(bullet);
            (code, jump) = (word as u8, (word >> 8) as u8);
        }
        if jump == JUMP_BACK {
            bullet.script = bullet.script.wrapping_sub(u16::from(code));
            code = self.tables.script_byte(bullet.script) & 0x0F;
        }
        code
    }

    /// 追蹤指令（原版 `sub_167E9`）：在子彈位置播尾跡特效，找最近的實體選方向。
    /// 回傳原版留在 `ax` 的字組：選出方向時是方向碼（高位元組 0），找不到時是記錄原本的朝向。
    fn steer_seeker(&mut self, bullet: &mut PlayerBullet) -> u16 {
        self.spawn_effect(EffectKind::SeekerTrail, bullet.col, bullet.row);
        let (distance, code) = self.seek_direction(bullet);
        if distance == NOT_FOUND {
            return bullet.heading;
        }
        bullet.heading = code;
        code
    }

    /// 沿 G1 的上、右、下、左四向，從子彈所在格起數到第一個非空格（0、FFh 視為空）的格距（原版 `sub_16BA8`）。
    /// 回傳 (距離, 方向碼 1、3、5、7)。平手時後者優先：左 > 下 > 右 > 上。
    ///
    /// 原版比到左邊時只更新方向碼，不更新距離（`16BEBh` 之後直接 `retn`）。呼叫端只用距離是否為 FFh 判斷有沒有找到，
    /// 所以只有左邊有東西、其餘三向都找不到時，回傳的距離仍是 FFh，視同沒找到。
    fn seek_direction(&self, bullet: &PlayerBullet) -> (u16, u16) {
        let lay = self.layout();
        let start = self.grid1_row_start(bullet.row);
        let end = self.grid1_row_end_of(start);
        let cell = start.wrapping_add(bullet.col);
        let up = self.distance_to_wall(cell, |at| at.wrapping_sub(lay.row_bytes()), |at| at > GRID1_START);
        let right = self.distance_to_wall(cell, |at| at.wrapping_add(1), |at| at < end);
        let down = self.distance_to_wall(cell, |at| at.wrapping_add(lay.row_bytes()), |at| at < lay.g1_end());
        let left = self.distance_to_wall(cell, |at| at.wrapping_sub(1), |at| at > start);
        let (mut distance, mut code) = (up, 1);
        if distance >= right {
            (distance, code) = (right, 3);
        }
        if distance >= down {
            (distance, code) = (down, 5);
        }
        if distance >= left {
            code = 7;
        }
        (distance, code)
    }

    /// 從 `from` 起每次 `next` 一格，數到第一個非空格的格距（原版 `sub_16BF5`、`16C18`、`16C3B`、`16C5C`）。
    /// 先看目前這格再前進，前進後 `within` 不成立就算找不到（FFh）。
    fn distance_to_wall(&self, from: u16, next: impl Fn(u16) -> u16, within: impl Fn(u16) -> bool) -> u16 {
        let (mut at, mut distance) = (from, 0u16);
        loop {
            let cell = self.grid1_byte(at);
            if cell != 0 && cell != 0xFF {
                return distance;
            }
            at = next(at);
            distance = distance.wrapping_add(1);
            if !within(at) {
                return NOT_FOUND;
            }
        }
    }

    /// 畫子彈、寫 G3、對 G1 與 G4 做碰撞、結算命中（原版 `sub_1691A`）。回傳是否因碰撞消失。
    fn draw_and_collide(&mut self, bullet: &PlayerBullet) -> bool {
        self.set_blit_params(bullet.blit_x, bullet.blit_y, bullet.image);
        self.cut_my(MyCopy::PlayerShot);
        self.overlay_my(MyCopy::PlayerShot);
        // 形狀寬同時寫進繪製的每列位元組殘值，三段碰撞碼寫的是同一個值，只寫一次（見 `Core::stamp_shape`）。
        let tables = self.tables();
        self.stamp_shape(tables.shape(bullet.shape), bullet.col, bullet.row);
        let mut impact = self.collide_entities(bullet);
        if self.collides_with_terrain(bullet) {
            impact = Impact { target: NO_ENTITY, vanishes: true };
        }
        if impact.target != 0 {
            self.strike(bullet, impact.target);
        }
        impact.vanishes
    }

    /// 子彈形狀逐格對 G1 做碰撞（原版 `sub_16B49`）。
    ///
    /// 每格先過範圍檢查（[`Core::grid1_probe`]），格值 0、FFh 略過，FEh（裝甲）不記錄目標，
    /// 其他記錄為目標（後掃到的覆蓋前面）。非 0、非 FFh 的格（含 FEh）都讓子彈消失，穿透彈除外。
    /// 列起點每一列重算，列號超過 13h 的列落在 [`Core::grid1_row_start`] 的越界規則。
    fn collide_entities(&self, bullet: &PlayerBullet) -> Impact {
        let shape = self.tables.shape(bullet.shape);
        let mut impact = Impact::default();
        for k in 0..loop_count(shape.height) {
            let start = self.grid1_row_start(bullet.row.wrapping_add(k as u16));
            let mut at = start.wrapping_add(bullet.col);
            for _ in 0..loop_count(shape.width) {
                if let Some(cell) = self.grid1_probe(start, at) {
                    if cell != 0 && cell != 0xFF {
                        if cell != ARMOR_CELL {
                            impact.target = cell;
                        }
                        if !bullet.behavior().pierces() {
                            impact.vanishes = true;
                        }
                    }
                }
                at = at.wrapping_add(1);
            }
        }
        impact
    }

    /// 第三關：子彈形狀逐格對 G4 地形碰撞（原版 `sub_169BF`）。列從子彈列減 1 起，字組非 0 就算撞到。
    /// 讀的是奇數位址的字組，跨相鄰兩格。其他關不檢查。
    fn collides_with_terrain(&self, bullet: &PlayerBullet) -> bool {
        if !self.on_terrain_stage() {
            return false;
        }
        let shape = self.tables.shape(bullet.shape);
        let mut hit = false;
        for k in 0..loop_count(shape.height) {
            let mut at = self.grid4_cell_addr(bullet.row.wrapping_sub(1).wrapping_add(k as u16), bullet.col);
            for _ in 0..loop_count(shape.width) {
                if self.grid4_contains(at) && self.grid4_word(at) != 0 {
                    hit = true;
                }
                at = at.wrapping_add(2);
            }
        }
        hit
    }

    /// 命中結算（原版 `sub_16946`）：`target` 是格值，實體索引 = 格值 - 1。對那個實體累計傷害，
    /// 然後在子彈目前位置播命中特效。[`NO_ENTITY`] 只播特效。
    ///
    /// 裝甲格與已釋放的槽的表外寫入見 [`Core::damage_entity_at_cell`]，砲座接觸共用同一份判斷。
    pub(crate) fn strike(&mut self, bullet: &PlayerBullet, target: u8) {
        if target != NO_ENTITY {
            self.damage_entity_at_cell(target, bullet.behavior().damage());
        }
        self.spawn_effect(EffectKind::from_letter(bullet.hit_effect), bullet.col, bullet.row);
    }

    /// 雷射（原版 `sub_16A4E`、`16ACA`、`16A99`、`16B18`）：從子彈欄往行進方向逐格掃，畫光束，結算命中。
    ///
    /// 每走一格先看 G4 同列上一列的字組（非 0 就停，不限第三關），再看 G1：0、FFh 繼續，其他格記錄為命中並停；
    /// 朝右掃到列尾（欄 39）、朝左掃到列首也停。沒有裝甲格檢查，FEh 會被記錄。
    /// 光束長度是走過的格數加 1（停止的那格也算），圖 4 或 19h（繪製頁奇偶交替），欄跟著前進，
    /// 所以命中特效在光束末端。沒有命中時目標是 [`NO_ENTITY`]，仍會播特效。
    fn fire_laser(&mut self, bullet: &mut PlayerBullet, facing: Facing) {
        let (length, target) = self.scan_beam(bullet, facing);
        let image = if self.draw_page() != 0 { BEAM_IMAGE_ODD_PAGE } else { BEAM_IMAGE };
        let mut x = bullet.blit_x;
        for _ in 0..loop_count(length) {
            self.set_blit_params(x, bullet.blit_y, image);
            self.overwrite_my();
            (x, bullet.col) = match facing {
                Facing::Right => (x.wrapping_add(1), bullet.col.wrapping_add(1)),
                Facing::Left => (x.wrapping_sub(1), bullet.col.wrapping_sub(1)),
            };
        }
        self.strike(bullet, target);
    }

    /// 雷射的掃描：回傳 (光束長度, 命中的格值或 [`NO_ENTITY`])。
    fn scan_beam(&self, bullet: &PlayerBullet, facing: Facing) -> (u16, u8) {
        let start = self.grid1_row_start(bullet.row);
        let end = self.grid1_row_end_of(start);
        let mut cell = start.wrapping_add(bullet.col);
        let mut terrain = self.grid4_cell_addr(bullet.row.wrapping_sub(1), bullet.col);
        let mut length = 1u16;
        loop {
            (cell, terrain) = match facing {
                Facing::Right => (cell.wrapping_add(1), terrain.wrapping_add(2)),
                Facing::Left => (cell.wrapping_sub(1), terrain.wrapping_sub(2)),
            };
            length = length.wrapping_add(1);
            let at_edge = match facing {
                Facing::Right => cell >= end,
                Facing::Left => cell <= start,
            };
            if at_edge || self.grid4_word(terrain) != 0 {
                return (length, NO_ENTITY);
            }
            let value = self.grid1_byte(cell);
            if value != 0 && value != 0xFF {
                return (length, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityLink;
    use crate::game::{Fixes, Start};

    fn core_with(fixes: Fixes) -> Core {
        let mut c = Core::for_test(Start::Demo, fixes);
        c.fill_row_tables();
        c
    }

    fn core() -> Core {
        core_with(Fixes::NONE)
    }

    /// 在 `at` 擺一個 `w` × `h` 的形狀，格值全為 `cell`。
    fn put_shape(c: &mut Core, at: u16, w: u16, h: u16, cell: u8) {
        c.tables_mut().put_shape(at, w, h, cell);
    }

    /// 在 G1 的 (`col`, `row`) 擺一個格值。
    fn put_g1(c: &mut Core, col: u16, row: u16, cell: u8) {
        c.grids.poke8(GRID1_START + row * GRID1_ROW_BYTES + col, cell);
    }

    /// 一顆在 (`col`, `row`) 的 1×1 子彈，形狀在 `0x7000`。
    fn bullet_at(c: &mut Core, col: u16, row: u16) -> PlayerBullet {
        put_shape(c, 0x7000, 1, 1, 1);
        PlayerBullet { col, row, blit_x: col, blit_y: row.wrapping_mul(10), shape: 0x7000, ..PlayerBullet::default() }
    }

    fn active(c: &Core) -> Vec<(usize, PlayerBullet)> {
        c.pb.slots.iter().enumerate().filter(|(_, s)| s.is_active()).map(|(i, s)| (i, s.bullet)).collect()
    }

    #[test]
    fn letters_pick_the_documented_weapons() {
        for (letter, want) in [
            (0, Weapon::Plain),
            (u16::from(b'N'), Weapon::Plain),
            (u16::from(b'K'), Weapon::Big),
            (u16::from(b'S'), Weapon::Spread),
            (u16::from(b'B'), Weapon::Seeker),
            (u16::from(b'L'), Weapon::Laser),
            (u16::from(b'F'), Weapon::Sweep),
        ] {
            assert_eq!(Weapon::from_letter(letter), want, "字母 {letter:#x}");
        }
        assert_eq!(Facing::from_word(0x3E), Facing::Right);
        assert_eq!(Facing::from_word(0x3C), Facing::Left);
        assert_eq!(Facing::from_word(0), Facing::Left, "只有 3Eh 朝右");
    }

    #[test]
    fn weapon_patterns_place_the_documented_starting_points() {
        // (字母, 朝向) -> 依序生成的 (樣板, 欄, 列)，玩家在欄 10、列 5。
        let cases: [(u8, Facing, Vec<(Shot, u16, u16)>); 11] = [
            (0, Facing::Right, vec![(Shot::Plain, 10, 6)]),
            (0, Facing::Left, vec![(Shot::Plain, 10, 6)]),
            (b'K', Facing::Right, vec![(Shot::Big, 10, 5)]),
            (b'S', Facing::Right, vec![(Shot::SpreadUp, 10, 6), (Shot::SpreadDown, 10, 6)]),
            (b'S', Facing::Left, vec![(Shot::SpreadUp, 10, 5), (Shot::SpreadDown, 10, 5)]),
            (b'B', Facing::Right, vec![(Shot::SeekerUp, 10, 6), (Shot::SeekerDown, 10, 6)]),
            (b'L', Facing::Right, vec![(Shot::Laser, 13, 6)]),
            (b'L', Facing::Left, vec![(Shot::Laser, 8, 6)]),
            (b'K', Facing::Left, vec![(Shot::Big, 10, 5)]),
            (b'B', Facing::Left, vec![(Shot::SeekerUp, 10, 6), (Shot::SeekerDown, 10, 6)]),
            (b'F', Facing::Left, [1u16, 4, 7, 10, 13, 16].iter().map(|&r| (Shot::Sweep, 10, r)).collect()),
        ];
        for (letter, facing, want) in cases {
            let mut got = Vec::new();
            let mut col = 10u16;
            let mut row = 5u16;
            for step in Weapon::from_letter(u16::from(letter)).steps(facing) {
                match *step {
                    Step::Shift { cols, rows } => {
                        col = col.wrapping_add(cols as u16);
                        row = row.wrapping_add(rows as u16);
                    }
                    Step::SetRow(r) => row = r,
                    Step::Fire(shot) => got.push((shot, col, row)),
                    Step::SeekerGate | Step::CountSeekers(_) => {}
                }
            }
            assert_eq!(got, want, "字母 {} 朝 {facing:?}", letter as char);
        }
    }

    #[test]
    fn spawn_copies_the_template_and_fills_the_five_words() {
        let mut c = core();
        c.tables_mut().put_shot(Shot::Big, Facing::Right, PlayerBullet::from_words(std::array::from_fn(|k| 0x900 + k as u16)));
        c.fire_player_bullets(u16::from(b'K'), Facing::Right, 7, 3);
        let slot = c.pb.slots[0];
        assert_eq!(slot.link, pb_record_link(0));
        let b = slot.bullet;
        assert_eq!((b.col, b.blit_x, b.row, b.blit_y, b.owner), (7, 7, 3, 30, u16::from(b'K')), "欄、列、列 × 10、武器字母");
        assert_eq!((b.kind, b.image, b.blit_dx, b.blit_dy, b.heading, b.spare), (0x900, 0x901, 0x904, 0x905, 0x906, 0x908), "其餘取自樣板");
        assert_eq!((b.dcol, b.drow, b.script, b.shape, b.hit_effect), (0x90B, 0x90C, 0x90D, 0x90E, 0x90F));
        assert!(!c.pb.slots[1].is_active());
    }

    #[test]
    fn spawn_takes_the_first_free_slot_and_drops_when_full() {
        let mut c = core();
        c.pb.slots[0].link = 1;
        c.pb.slots[2].link = 1;
        c.fire_player_bullets(u16::from(b'S'), Facing::Right, 1, 1);
        assert_eq!(active(&c).iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![0, 1, 2, 3], "兩顆依序填進空槽 1、3");
        for s in c.pb.slots.iter_mut() {
            s.link = 1;
        }
        let before = c.pb.slots;
        c.fire_player_bullets(u16::from(b'K'), Facing::Right, 5, 5);
        assert_eq!(c.pb.slots, before, "池滿了放棄");
    }

    #[test]
    fn seeker_gate_stops_above_five() {
        let mut c = core();
        c.pb.seekers_in_flight = 5;
        c.fire_player_bullets(u16::from(b'B'), Facing::Right, 1, 1);
        assert_eq!(c.pb.seekers_in_flight, 7, "5 還可以射，射完加 2");
        assert_eq!(active(&c).len(), 2);
        c.fire_player_bullets(u16::from(b'B'), Facing::Left, 1, 1);
        assert_eq!(c.pb.seekers_in_flight, 7, "超過 5 就不射");
        assert_eq!(active(&c).len(), 2);
    }

    #[test]
    fn release_decrements_the_seeker_count_only_for_seekers() {
        let mut c = core();
        put_shape(&mut c, 0x7000, 1, 1, 1);
        c.pb.seekers_in_flight = 2;
        for (i, kind) in [(0usize, 0x41u16), (1, 0)] {
            c.pb.slots[i] = Slot {
                link: pb_record_link(i),
                bullet: PlayerBullet { kind, col: 0x30, row: 0, shape: 0x7000, script: 0x7100, ..PlayerBullet::default() },
            };
        }
        // 欄 0x30 >= 27h：這一幀出界。腳本碼 3（右）。
        c.tables_mut().put_bytes(0x7100, &[3]);
        c.update_player_bullets();
        assert!(!c.pb.slots[0].is_active() && !c.pb.slots[1].is_active());
        assert_eq!(c.pb.seekers_in_flight, 1, "只有型別 A 的釋放會減");
        assert_eq!(c.pb.slots[0].bullet.col, 0x30, "釋放的槽保留記錄（col 加 dcol 0）");
    }

    #[test]
    fn movement_limits_and_diagonals() {
        let b = |col, row| PlayerBullet { col, row, dcol: 1, drow: 1, blit_dx: 1, blit_dy: 1, ..PlayerBullet::default() };
        let mut x = b(0x25, 5);
        assert!(!x.advance(3, COL_LIMIT) && x.col == 0x26, "欄 26h 還在內");
        assert!(x.advance(3, COL_LIMIT) && x.col == 0x27, "欄 27h 出界");
        let mut y = b(5, 0x10);
        assert!(!y.advance(5, COL_LIMIT) && y.row == 0x11);
        assert!(y.advance(5, COL_LIMIT), "列 12h 出界，往上往下同門檻");
        let mut z = b(5, 0);
        assert!(z.advance(1, COL_LIMIT) && z.row == 0xFFFF, "負列（無號）出界");
        // 斜向兩段都執行，旗標是聯集。
        let mut d = b(0x26, 5);
        assert!(d.advance(2, COL_LIMIT));
        assert_eq!((d.col, d.row), (0x27, 4));
        let mut e = b(0, 0);
        assert!(e.advance(8, COL_LIMIT), "左上：欄 0 減 1 得 FFFFh，出界");
        assert_eq!((e.col, e.row), (0xFFFF, 0xFFFF));
        let mut f = b(5, 5);
        assert!(!f.advance(0, COL_LIMIT) && !f.advance(9, COL_LIMIT) && !f.advance(0x13, COL_LIMIT), "其他碼不動");
        assert_eq!((f.col, f.row), (5, 5));
        // 戰場加大（規格 005）：側邊門檻是新的欄數減 1，欄 38 與 39 在 40 欄時已出界，60 欄時還在內。
        let mut w = b(0x26, 5);
        assert!(!w.advance(3, 59) && w.col == 0x27, "60 欄：欄 27h 還在內");
        let mut w = b(57, 5);
        assert!(!w.advance(3, 59) && w.col == 58);
        assert!(w.advance(3, 59) && w.col == 59, "欄 59 出界");
    }

    #[test]
    fn script_jump_back_repeats_the_target_byte() {
        let mut c = core();
        // 腳本 03 04 F2：第三個位元組往回跳 2，指標 3 減 2 = 1，取位元組 04，指標不再前進。
        c.tables_mut().put_bytes(0x7100, &[0x03, 0x04, 0xF2]);
        let mut bullet = PlayerBullet { script: 0x7100, ..PlayerBullet::default() };
        assert_eq!(c.next_code(&mut bullet), 3);
        assert_eq!(c.next_code(&mut bullet), 4);
        assert_eq!((c.next_code(&mut bullet), bullet.script), (4, 0x7101), "跳回幀取 04，指標停在 04 那格");
        assert_eq!((c.next_code(&mut bullet), bullet.script), (4, 0x7102), "下一幀 04 再生效一次");
        assert_eq!((c.next_code(&mut bullet), bullet.script), (4, 0x7101), "再讀到 F2 又跳回，04 每輪生效兩次");
    }

    #[test]
    fn collide_entities_records_last_cell_and_armor_only_vanishes() {
        let mut c = core();
        let mut b = bullet_at(&mut c, 5, 4);
        put_shape(&mut c, 0x7000, 3, 1, 1);
        b.shape = 0x7000;
        put_g1(&mut c, 5, 4, 2);
        put_g1(&mut c, 6, 4, 0xFE);
        put_g1(&mut c, 7, 4, 3);
        assert_eq!(c.collide_entities(&b), Impact { target: 3, vanishes: true }, "後掃到的覆蓋前面");
        put_g1(&mut c, 7, 4, 0xFF);
        assert_eq!(c.collide_entities(&b), Impact { target: 2, vanishes: true }, "FFh 略過，裝甲 FEh 不覆蓋目標");
        put_g1(&mut c, 5, 4, 0);
        assert_eq!(c.collide_entities(&b), Impact { target: 0, vanishes: true }, "只有裝甲：不記錄目標，仍消失");
        put_g1(&mut c, 6, 4, 0);
        assert_eq!(c.collide_entities(&b), Impact::default());
    }

    #[test]
    fn piercing_bullets_hit_without_vanishing() {
        let mut c = core();
        let mut b = bullet_at(&mut c, 5, 4);
        b.kind = 0x53;
        put_g1(&mut c, 5, 4, 7);
        assert_eq!(c.collide_entities(&b), Impact { target: 7, vanishes: false });
        assert_eq!((b.behavior().damage(), b.behavior().pierces()), (3, true));
        for (kind, damage) in [(0u16, 1u8), (0x41, 3), (0x4C, 1), (0x6C, 1), (0x12, 1)] {
            let behavior = PlayerBullet { kind, ..b }.behavior();
            assert_eq!((behavior.damage(), behavior.pierces()), (damage, false), "型別 {kind:#x}");
        }
    }

    #[test]
    fn collide_entities_skips_column_0_and_39_of_each_row() {
        let mut c = core();
        let b = bullet_at(&mut c, 0, 4);
        put_g1(&mut c, 0, 4, 9);
        assert_eq!(c.collide_entities(&b), Impact::default(), "欄 0 不測（嚴格在列起點之後）");
        let b = bullet_at(&mut c, 39, 4);
        put_g1(&mut c, 39, 4, 9);
        assert_eq!(c.collide_entities(&b), Impact::default(), "欄 39 不測（嚴格在列尾之前）");
        let b = bullet_at(&mut c, 38, 4);
        put_g1(&mut c, 38, 4, 9);
        assert_eq!(c.collide_entities(&b).target, 9);
        // 負列與列 19 之後：列起點取越界規則，不會碰到 G1 之內的格。
        // 負列（無號）的列起點取越界規則，等於 G1 列 0 的起點，所以讀到的是列 0 的格。
        put_g1(&mut c, 5, 0, 8);
        let b = bullet_at(&mut c, 5, 0xFFFF);
        assert_eq!(c.collide_entities(&b).target, 8);
    }

    #[test]
    fn terrain_is_checked_only_on_the_third_stage_and_wins_over_entities() {
        let mut c = core();
        let b = bullet_at(&mut c, 5, 4);
        // G4 同列上一列（列 3）的奇數位址字組。
        let at = c.grid4_cell_addr(3, 5);
        c.grids.poke16(at, 0x0100);
        assert!(!c.collides_with_terrain(&b), "不是第三關不檢查");
        c.set_stage_letter(b'C');
        assert!(c.collides_with_terrain(&b));
        c.grids.poke16(at, 0);
        assert!(!c.collides_with_terrain(&b));
        // 位址超出 95FCh 的格不測：列 19 的欄 30h 落在 G4 之後。
        let far = bullet_at(&mut c, 0x30, 20);
        let past = c.grid4_cell_addr(19, 0x30);
        assert!(!c.grid4_contains(past));
        c.grids.poke16(past, 0xFFFF);
        assert!(!c.collides_with_terrain(&far));
    }

    #[test]
    fn strike_adds_damage_to_the_entity_and_spawns_the_effect() {
        let mut c = core();
        let ent = EntityLink::Slot(1);
        c.set_entity(1, 0x0004);
        let b = PlayerBullet { col: 7, row: 2, hit_effect: u16::from(b'B'), ..PlayerBullet::default() };
        c.strike(&b, 2);
        assert_eq!(c.entity_damage(ent), 0x0105, "高位元組設 1，低位元組加 1");
        assert_eq!(c.fx.slots[0].script_pos, c.tables.fx_start(crate::fx::Script::BigExplosion), "特效字母 B");
        assert_eq!((c.fx.slots[0].x, c.fx.slots[0].y), (7, 2));
        c.strike(&PlayerBullet { kind: 0x41, ..b }, 2);
        assert_eq!(c.entity_damage(ent), 0x0108, "追蹤彈加 3");
        c.strike(&PlayerBullet { kind: 0x53, ..b }, 2);
        assert_eq!(c.entity_damage(ent), 0x010B, "穿透彈加 3");
        let used = c.fx.slots.iter().filter(|s| s.is_active()).count();
        c.strike(&b, NO_ENTITY);
        assert_eq!(c.fx.slots.iter().filter(|s| s.is_active()).count(), used + 1, "NO_ENTITY 仍播特效");
        assert_eq!(c.entity_damage(ent), 0x010B, "NO_ENTITY 不動實體");
        assert_eq!(c.entity_damage(EntityLink::Stray(0)), 0, "也不會落到 dseg:0020h");
    }

    #[test]
    fn damage_byte_wraps_without_carrying_into_the_flag() {
        let mut c = core();
        c.set_entity(0, 0x01FF);
        c.strike(&PlayerBullet::default(), 1);
        assert_eq!(c.entity_damage(EntityLink::Slot(0)), 0x0100, "低位元組 8 位元相加，高位元組固定 1");
    }

    #[test]
    fn armor_cell_writes_to_dseg_0020_unless_fixed() {
        for (fixes, want) in [(Fixes::NONE, 0x0166u16), (Fixes::ALL, 0x0165)] {
            let mut c = core_with(fixes);
            c.en.stray_damage = 0x0165;
            c.strike(&PlayerBullet::default(), ARMOR_CELL);
            assert_eq!(c.entity_damage(EntityLink::Stray(0)), want);
        }
    }

    /// 在 G1 擺一列牆，讓追蹤彈找方向。
    fn seeker_at(c: &mut Core, col: u16, row: u16) -> PlayerBullet {
        bullet_at(c, col, row)
    }

    #[test]
    fn seeker_picks_the_nearest_direction_with_later_ties_winning() {
        let mut c = core();
        let b = seeker_at(&mut c, 10, 8);
        // 上 3 格、右 5 格、下 3 格、左 6 格：上與下平手，後者（下）優先。
        put_g1(&mut c, 10, 5, 1);
        put_g1(&mut c, 15, 8, 1);
        put_g1(&mut c, 10, 11, 1);
        put_g1(&mut c, 4, 8, 1);
        assert_eq!(c.seek_direction(&b), (3, 5));
        // 上 2、左 2 平手，左優先。
        put_g1(&mut c, 10, 6, 1);
        put_g1(&mut c, 8, 8, 1);
        assert_eq!(c.seek_direction(&b), (2, 7));
        // 自己所在的格就有東西：四向都是 0，左優先。
        put_g1(&mut c, 10, 8, 1);
        assert_eq!(c.seek_direction(&b), (0, 7));
    }

    #[test]
    fn seeker_with_only_a_left_wall_reports_not_found() {
        // 原版 16BEBh：比到左邊只改方向碼不改距離，其餘三向都是 FFh 時距離仍是 FFh。
        let mut c = core();
        let b = seeker_at(&mut c, 10, 8);
        put_g1(&mut c, 7, 8, 1);
        assert_eq!(c.seek_direction(&b), (NOT_FOUND, 7));
        let mut bullet = PlayerBullet { heading: 2, ..b };
        assert_eq!(c.steer_seeker(&mut bullet), 2, "找不到沿用原本的朝向");
        assert_eq!(bullet.heading, 2);
        // 右邊 2 格也有東西：右比左近，距離 2、方向 3。
        put_g1(&mut c, 12, 8, 1);
        assert_eq!(c.seek_direction(&b), (2, 3));
        assert_eq!(c.steer_seeker(&mut bullet), 3);
        assert_eq!(bullet.heading, 3, "選出方向後存回記錄");
        // 右邊 4 格：左（3 格）比較近，方向 7，距離仍是右的 4。
        put_g1(&mut c, 12, 8, 0);
        put_g1(&mut c, 14, 8, 1);
        assert_eq!(c.seek_direction(&b), (4, 7));
    }

    #[test]
    fn seeker_treats_ff_as_empty() {
        let mut c = core();
        let b = seeker_at(&mut c, 10, 8);
        put_g1(&mut c, 10, 6, 0xFF);
        put_g1(&mut c, 10, 4, 5);
        assert_eq!(c.seek_direction(&b), (4, 1));
        // 全空：每向 FFh，平手取最後，方向碼 7。
        let c2 = core();
        assert_eq!(c2.seek_direction(&b), (NOT_FOUND, 7));
    }

    #[test]
    fn seeker_script_steers_on_0a_and_leaves_a_trail() {
        let mut c = core();
        let mut b = seeker_at(&mut c, 10, 8);
        b.heading = 4;
        b.script = 0x7100;
        c.tables_mut().put_bytes(0x7100, &[0x0A]);
        put_g1(&mut c, 14, 8, 1);
        assert_eq!(c.next_code(&mut b), 3, "右邊最近，方向 3");
        assert_eq!(b.heading, 3);
        assert_eq!(b.script, 0x7101);
        assert_eq!(c.fx.slots[0].script_pos, c.tables.fx_start(crate::fx::Script::SeekerTrail), "先播尾跡");
        assert_eq!((c.fx.slots[0].x, c.fx.slots[0].y), (10, 8));
    }

    #[test]
    fn seeker_last_script_frame_does_not_move() {
        // 腳本尾端 F5：追蹤判斷在跳回之前，跳回後的 0Ah 不再選方向，該幀碼 0Ah 不移動。
        let mut c = core();
        let mut b = seeker_at(&mut c, 10, 8);
        b.script = 0x7105;
        c.tables_mut().put_bytes(0x7100, &[0x0A, 0x0A, 0x0A, 0x0A, 0x0A, 0xF5]);
        let code = c.next_code(&mut b);
        assert_eq!(code, 0x0A);
        assert_eq!(b.script, 0x7101, "指標 7106h 減 5");
        let mut moving = PlayerBullet { dcol: 1, drow: 1, blit_dx: 1, blit_dy: 1, ..b };
        assert!(!moving.advance(code, COL_LIMIT));
        assert_eq!((moving.col, moving.row), (10, 8), "碼 0Ah 不在移動表內，不動");
        assert!(c.fx.slots.iter().all(|s| !s.is_active()), "沒有再選方向，所以沒有尾跡");
    }

    #[test]
    fn laser_scans_to_the_first_cell_and_extends_the_column() {
        let mut c = core();
        let mut b = PlayerBullet { kind: 0x4C, col: 3, row: 6, blit_x: 3, blit_y: 60, hit_effect: u16::from(b'>'), ..PlayerBullet::default() };
        c.set_entity(4, 0);
        put_g1(&mut c, 9, 6, 5);
        c.draws.clear();
        c.fire_laser(&mut b, Facing::Right);
        // 欄 4、5、...、9：走 6 步，長度 = 1 + 6 = 7，欄前進 7。
        assert_eq!(b.col, 10, "欄跟著光束前進");
        assert_eq!(c.entity_damage(EntityLink::Slot(4)), 0x0101, "格值 5 是實體索引 4，傷害 1");
        let beams = c.draws.iter().filter(|d| matches!(d, crate::draw::DrawCmd::Blit(b) if b.op == crate::draw::Op::MyOverwrite)).count();
        assert_eq!(beams, 7);
        assert_eq!((c.fx.slots[0].x, c.fx.slots[0].y), (10, 6), "命中特效在光束末端");
    }

    #[test]
    fn laser_without_a_hit_still_plays_the_effect_at_the_edge() {
        let mut c = core();
        let mut b = PlayerBullet { kind: 0x4C, col: 30, row: 6, blit_x: 30, blit_y: 60, ..PlayerBullet::default() };
        c.fire_laser(&mut b, Facing::Right);
        // 欄 31 至 38 都空，走到 39 停（先加後比）：長度 = 1 + 9 = 10。
        assert_eq!(b.col, 40);
        assert!(c.fx.slots[0].is_active());
        assert_eq!((c.fx.slots[0].x, c.fx.slots[0].y), (40, 6));
    }

    #[test]
    fn laser_left_stops_at_the_row_start_and_at_terrain() {
        let mut c = core();
        let mut b = PlayerBullet { kind: 0x6C, col: 4, row: 6, blit_x: 4, blit_y: 60, ..PlayerBullet::default() };
        put_g1(&mut c, 1, 6, 3);
        c.set_entity(2, 0);
        c.fire_laser(&mut b, Facing::Left);
        assert_eq!(b.col, 4 - 4, "欄 3、2、1 走 3 步，長度 4");
        assert_eq!(c.entity_damage(EntityLink::Slot(2)), 0x0101);
        // 地形：G4 同列上一列的字組非 0 就停，在命中判斷之前，不限第三關。
        let mut c = core();
        let mut b = PlayerBullet { kind: 0x6C, col: 10, row: 6, blit_x: 10, blit_y: 60, ..PlayerBullet::default() };
        put_g1(&mut c, 6, 6, 3);
        let wall = c.grid4_cell_addr(5, 10).wrapping_sub(2 * 2);
        c.grids.poke16(wall, 1);
        c.fire_laser(&mut b, Facing::Left);
        assert_eq!(b.col, 10 - 3, "走 2 步在第 3 步的位置看到地形停下，長度 3");
        assert_eq!(c.entity_damage(EntityLink::Stray(0)), 0, "地形不扣實體血");
    }

    #[test]
    fn update_draws_mask_then_planes_and_stamps_g3() {
        let mut c = core();
        let mut b = bullet_at(&mut c, 5, 4);
        b.image = 33;
        b.script = 0x7100;
        b.dcol = 1;
        b.blit_dx = 1;
        c.tables_mut().put_bytes(0x7100, &[3]);
        c.pb.slots[0] = Slot { link: pb_record_link(0), bullet: b };
        c.draws.clear();
        c.update_player_bullets();
        let blits: Vec<_> = c.draws.iter().filter_map(|d| if let crate::draw::DrawCmd::Blit(b) = d { Some(*b) } else { None }).collect();
        assert_eq!(blits.iter().map(|b| b.op).collect::<Vec<_>>(), vec![crate::draw::Op::MyCutPlayerShot, crate::draw::Op::MyOverlayPlayerShot]);
        assert!(blits.iter().all(|b| b.x == 6 && b.y == 40 && b.index == 33), "移動後再畫");
        assert_eq!(c.grids.peek8(c.grid3_cell_addr(4, 6)), 1, "形狀寫進 G3");
        assert_eq!(c.pb.slots[0].bullet.col, 6);
        assert!(c.pb.slots[0].is_active());
    }

    #[test]
    fn a_bullet_that_hits_is_released_and_damages() {
        let mut c = core();
        let mut b = bullet_at(&mut c, 5, 4);
        b.script = 0x7100;
        b.dcol = 1;
        b.blit_dx = 1;
        c.tables_mut().put_bytes(0x7100, &[3]);
        c.set_entity(0, 0);
        put_g1(&mut c, 6, 4, 1);
        c.pb.slots[3] = Slot { link: pb_record_link(3), bullet: b };
        c.update_player_bullets();
        assert!(!c.pb.slots[3].is_active());
        assert_eq!(c.entity_damage(EntityLink::Slot(0)), 0x0101);
    }

    #[test]
    fn release_all_clears_links_but_keeps_records() {
        let mut p = PlayerBullets::default();
        p.slots[4] = Slot { link: 0x1234, bullet: PlayerBullet { col: 9, ..PlayerBullet::default() } };
        p.seekers_in_flight = 3;
        p.release_all();
        assert_eq!(p.slots[4], Slot { link: 0, bullet: PlayerBullet { col: 9, ..PlayerBullet::default() } });
        assert_eq!(p.seekers_in_flight, 3, "追蹤彈計數另外歸零");
    }
}
