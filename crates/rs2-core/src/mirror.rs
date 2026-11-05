//! 領域狀態與原版資料段位移之間的對應（`Mirror`）。
//!
//! 重寫的方法見 `docs/spec/004-rewrite-and-delivery.md`：一個領域一個領域從平坦的 `Dseg` 搬進有型別的結構。
//! 搬完的領域，狀態住在結構裡，`Core::ds` 中對應的位元組不再維護。全流程對拍要逐位元組比資料段，
//! 所以每個領域實作 [`Mirror`]：
//!
//! - `load`：從資料段傾印讀入。`Game::from_snapshot`、`Game::restart`、`poke` 之後用。
//! - `store`：寫回原版位移。交回控制權時（`Core::suspend`）、`Game.ds` 給對拍比較用。
//!
//! 規則：
//!
//! - 原版位移只准出現在這個檔案與註解。領域的邏輯程式不得有 `a(0x...)` 或資料段位移常數。
//! - `load` 與 `store` 必須互為反函數：任何資料段內容讀進再寫回，位元組不變。所以結構的欄位用原版的字組寬度，
//!   不做「合理化」的轉換（例如把指標換成列舉）。轉換留到表格階段。
//! - 領域內先寫後讀的工作變數不進結構，改成區域變數或參數，位移列進 `DS_EXEMPT`（`game.rs`）並寫明讀寫它的函式。
//! - 跨呼叫保留的殘值（例如特效的位置游標）是狀態，要進結構並 `store`。
//!
//! 本檔後半是「尚未搬走的共用資料段」：搬完的領域還要讀寫其他領域擁有的 `Dseg` 變數（唯讀表格、格網），
//! 一律經這裡的具名存取函式。那個領域搬完時，把存取函式換成真正的結構。

use crate::dseg::{Dseg, a};
use crate::ebullet::{Bullet, Capsule, EnemyBullets, POOL_SIZE as EB_POOL_SIZE, Slot, Template};
use crate::fx::{Effect, Effects, POOL_SIZE, Script};
use crate::game::Core;
use crate::pbullet::{Facing, POOL_SIZE as PB_POOL_SIZE, PlayerBullet, PlayerBullets, Shot, Slot as PbSlot};

/// 一個領域的狀態與資料段之間的對應。
pub(crate) trait Mirror {
    /// 從資料段傾印讀入這個領域的全部狀態。
    fn load(&mut self, ds: &Dseg);
    /// 把這個領域的全部狀態寫回原版位移。
    fn store(&self, ds: &mut Dseg);
}

impl Core {
    /// 所有已搬走的領域從 `self.ds` 讀入。
    pub(crate) fn load_mirrors(&mut self) {
        self.fx.load(&self.ds);
        self.eb.load(&self.ds);
        self.pb.load(&self.ds);
    }

    /// 所有已搬走的領域寫回 `self.ds`。
    pub(crate) fn store_mirrors(&mut self) {
        self.fx.store(&mut self.ds);
        self.eb.store(&mut self.ds);
        self.pb.store(&mut self.ds);
    }
}

// ---- 特效池 ----

/// 過關連爆的起點欄，`word_20B7A`。關卡初始化寫 0，過關連爆每輪減 3。
const FX_CHAIN_BASE_X: u16 = a(0x20B7A);
/// 位置游標，`word_20B7C`、`word_20B7E`。生成與每格更新都會寫，單顆 `E` 沿用上一次的值。
const FX_CURSOR_X: u16 = a(0x20B7C);
const FX_CURSOR_Y: u16 = a(0x20B7E);
/// 池的三個平行陣列，各 30 個字組：x（`3FC2h`）、y（`3FFEh`）、腳本讀取位置（`403Ah`）。
const FX_POOL_X: u16 = 0x3FC2;
const FX_POOL_Y: u16 = 0x3FFE;
const FX_POOL_SCRIPT: u16 = 0x403A;

impl Mirror for Effects {
    fn load(&mut self, ds: &Dseg) {
        self.chain_base_x = ds.r16(FX_CHAIN_BASE_X);
        self.cursor = (ds.r16(FX_CURSOR_X), ds.r16(FX_CURSOR_Y));
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let o = (i * 2) as u16;
            *slot = Effect {
                x: ds.r16(FX_POOL_X + o),
                y: ds.r16(FX_POOL_Y + o),
                script_pos: ds.r16(FX_POOL_SCRIPT + o),
            };
        }
    }

    fn store(&self, ds: &mut Dseg) {
        ds.w16(FX_CHAIN_BASE_X, self.chain_base_x);
        ds.w16(FX_CURSOR_X, self.cursor.0);
        ds.w16(FX_CURSOR_Y, self.cursor.1);
        for (i, slot) in self.slots.iter().enumerate() {
            let o = (i * 2) as u16;
            ds.w16(FX_POOL_X + o, slot.x);
            ds.w16(FX_POOL_Y + o, slot.y);
            ds.w16(FX_POOL_SCRIPT + o, slot.script_pos);
        }
    }
}

const _: () = assert!(POOL_SIZE == 30, "池大小與原版三個陣列的長度一致");

// ---- 敵方子彈池 ----

/// 池指標表，20 個字組（`32B3h`）。0 是空槽，使用中是該槽記錄的位址。
const EB_LINKS: u16 = 0x32B3;
/// 記錄區，20 筆連續的 15 字組記錄（`32DBh` 起，每筆 1Eh 位元組）。
const EB_RECORDS: u16 = 0x32DB;
const EB_RECORD_BYTES: u16 = 0x1E;

const _: () = assert!(EB_POOL_SIZE == 20, "池大小與原版指標表的長度一致");
const _: () = assert!(EB_RECORDS == EB_LINKS + 2 * EB_POOL_SIZE as u16, "記錄區緊接在指標表之後");

/// 第 `slot` 槽記錄的位址，也就是使用中的槽寫進池指標表的值。
pub(crate) const fn record_link(slot: usize) -> u16 {
    EB_RECORDS + EB_RECORD_BYTES * slot as u16
}

/// 記錄的欄位順序與位移（`Bullet` 的欄位依序對應 `w0` 至 `w14`）：
///
/// | 位移 | 欄位 | 位移 | 欄位 |
/// |---|---|---|---|
/// | +00h | `heading` | +10h | `library` |
/// | +02h | `frames` | +12h | `col` |
/// | +04h | `blit_x` | +14h | `row` |
/// | +06h | `blit_y` | +16h | `dcol` |
/// | +08h | `blit_dx` | +18h | `drow` |
/// | +0Ah | `blit_dy` | +1Ah | `script` |
/// | +0Ch | `spare` | +1Ch | `shape` |
/// | +0Eh | `trail` | | |
impl Bullet {
    pub(crate) fn from_words(w: [u16; 15]) -> Bullet {
        Bullet {
            heading: w[0],
            frames: w[1],
            blit_x: w[2],
            blit_y: w[3],
            blit_dx: w[4],
            blit_dy: w[5],
            spare: w[6],
            trail: w[7],
            library: w[8],
            col: w[9],
            row: w[10],
            dcol: w[11],
            drow: w[12],
            script: w[13],
            shape: w[14],
        }
    }

    pub(crate) fn to_words(&self) -> [u16; 15] {
        [
            self.heading,
            self.frames,
            self.blit_x,
            self.blit_y,
            self.blit_dx,
            self.blit_dy,
            self.spare,
            self.trail,
            self.library,
            self.col,
            self.row,
            self.dcol,
            self.drow,
            self.script,
            self.shape,
        ]
    }

    fn read(ds: &Dseg, at: u16) -> Bullet {
        Bullet::from_words(std::array::from_fn(|k| ds.r16(at.wrapping_add(2 * k as u16))))
    }
}

impl Mirror for EnemyBullets {
    fn load(&mut self, ds: &Dseg) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            *slot = Slot { link: ds.r16(EB_LINKS + 2 * i as u16), bullet: Bullet::read(ds, record_link(i)) };
        }
    }

    fn store(&self, ds: &mut Dseg) {
        for (i, slot) in self.slots.iter().enumerate() {
            ds.w16(EB_LINKS + 2 * i as u16, slot.link);
            for (k, w) in slot.bullet.to_words().into_iter().enumerate() {
                ds.w16(record_link(i) + 2 * k as u16, w);
            }
        }
    }
}

impl EnemyBullets {
    /// 原版刪除敵彈寫的是「池指標表第 `idx` 項」（`mov [bx+32B3h], 0`，`bx` = 槽號 × 2），沒有檢查 `idx` < 20。
    /// 記錄區緊接在指標表之後，超出的項就是記錄區的字組：項 20 是槽 0 的 `w0`，之後每 15 項換一槽。
    /// 玩家碰到格值低四位為 0 的敵彈格時槽號減 1 繞回 FFh，落在項 255，也就是槽 15 的 `w10`（`row`）。
    pub(crate) fn clear_link_word(&mut self, idx: usize) {
        if idx < EB_POOL_SIZE {
            self.slots[idx].link = 0;
            return;
        }
        let word = idx - EB_POOL_SIZE;
        let Some(slot) = self.slots.get_mut(word / 15) else { return };
        let mut w = slot.bullet.to_words();
        w[word % 15] = 0;
        slot.bullet = Bullet::from_words(w);
    }
}

// ---- 玩家子彈池 ----

/// 在飛的追蹤彈數，`word_1F1A8`。
const PB_SEEKERS: u16 = a(0x1F1A8);
/// 池指標表，20 個字組（`2607h`）。0 是空槽，使用中是該槽記錄的位址。
const PB_LINKS: u16 = 0x2607;
/// 記錄區，20 筆連續的 16 字組記錄（`262Fh` 起，每筆 20h 位元組）。
const PB_RECORDS: u16 = 0x262F;
const PB_RECORD_BYTES: u16 = 0x20;

const _: () = assert!(PB_POOL_SIZE == 20, "池大小與原版指標表的長度一致");
const _: () = assert!(PB_RECORDS == PB_LINKS + 2 * PB_POOL_SIZE as u16, "記錄區緊接在指標表之後");

/// 第 `slot` 槽記錄的位址，也就是使用中的槽寫進池指標表的值。
pub(crate) const fn pb_record_link(slot: usize) -> u16 {
    PB_RECORDS + PB_RECORD_BYTES * slot as u16
}

/// 記錄的欄位順序與位移（`PlayerBullet` 的欄位依序對應 `w0` 至 `w15`）：
///
/// | 位移 | 欄位 | 位移 | 欄位 |
/// |---|---|---|---|
/// | +00h | `kind` | +10h | `spare` |
/// | +02h | `image` | +12h | `col` |
/// | +04h | `blit_x` | +14h | `row` |
/// | +06h | `blit_y` | +16h | `dcol` |
/// | +08h | `blit_dx` | +18h | `drow` |
/// | +0Ah | `blit_dy` | +1Ah | `script` |
/// | +0Ch | `heading` | +1Ch | `shape` |
/// | +0Eh | `owner` | +1Eh | `hit_effect` |
impl PlayerBullet {
    pub(crate) fn from_words(w: [u16; 16]) -> PlayerBullet {
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

    pub(crate) fn to_words(&self) -> [u16; 16] {
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

    fn read(ds: &Dseg, at: u16) -> PlayerBullet {
        PlayerBullet::from_words(std::array::from_fn(|k| ds.r16(at.wrapping_add(2 * k as u16))))
    }
}

impl Mirror for PlayerBullets {
    fn load(&mut self, ds: &Dseg) {
        self.seekers_in_flight = ds.r16(PB_SEEKERS);
        for (i, slot) in self.slots.iter_mut().enumerate() {
            *slot = PbSlot { link: ds.r16(PB_LINKS + 2 * i as u16), bullet: PlayerBullet::read(ds, pb_record_link(i)) };
        }
    }

    fn store(&self, ds: &mut Dseg) {
        ds.w16(PB_SEEKERS, self.seekers_in_flight);
        for (i, slot) in self.slots.iter().enumerate() {
            ds.w16(PB_LINKS + 2 * i as u16, slot.link);
            for (k, w) in slot.bullet.to_words().into_iter().enumerate() {
                ds.w16(pb_record_link(i) + 2 * k as u16, w);
            }
        }
    }
}

// ---- 尚未搬走的共用資料段 ----
//
// 搬完的領域還要讀寫其他領域擁有的 `Dseg` 變數（唯讀表格、玩家狀態、格網），一律經下面的具名存取函式。

impl Script {
    /// 動畫表在資料段的位移。唯讀表格，表格階段改成解析 `S.EXE` 得到的結構。
    pub(crate) const fn table_offset(self) -> u16 {
        match self {
            Script::SparkRight => 0x4094,
            Script::SparkLeft => 0x40B2,
            Script::BigExplosion => 0x40D0,
            Script::Blast => 0x40EE,
            Script::Trail => 0x413C,
            Script::SeekerTrail => 0x4162,
            Script::PlayerDeath => 0x4188,
        }
    }
}

impl Capsule {
    /// 道具球樣板的起點（朝向 1）。四種各 8 個樣板，連續排列。
    const fn table_base(self) -> u16 {
        match self {
            Capsule::L => 0x3737,
            Capsule::B => 0x3827,
            Capsule::K => 0x3917,
            Capsule::S => 0x3A07,
        }
    }
}

impl Template {
    /// 敵彈樣板在資料段的位移。唯讀表格，表格階段改成解析 `S.EXE` 得到的結構。
    /// 樣板表從 `3557h` 起共 72 筆，每筆 1Eh 位元組；八向的樣板依朝向 1 至 8 連續排列。
    /// `3647h` 至 `3719h` 的 8 筆（形狀 `3F40h`、圖庫索引 0）沒有任何程式引用，不在這裡。
    pub(crate) const fn table_offset(self) -> u16 {
        match self {
            Template::Aimed(h) => 0x3557 + EB_RECORD_BYTES * h.index(),
            Template::Capsule(c, h) => c.table_base() + EB_RECORD_BYTES * h.index(),
            Template::Unused => 0x3AF7,
            Template::Wide(t) => 0x3B15 + EB_RECORD_BYTES * t as u16,
            Template::Spread(s) => 0x3B6F + EB_RECORD_BYTES * s as u16,
            Template::WideFlash => 0x3C05,
            Template::Flash => 0x3C23,
            Template::Straight(h) => 0x3C41 + EB_RECORD_BYTES * h.index(),
            Template::SlowSw => 0x3D31,
            Template::SlowW => 0x3D4F,
            Template::SlowNw => 0x3D6D,
            Template::BouncerNw => 0x3D8B,
            Template::BouncerNe => 0x3DA9,
        }
    }
}

impl Shot {
    /// 玩家彈樣板在資料段的位移。唯讀表格，表格階段改成解析 `S.EXE` 得到的結構。
    /// 樣板表從 `28D5h` 起共 20 筆，每筆 20h 位元組。`28F5h`、`2915h`（`165FCh` 的備用雙彈碼，前面是 `retn`）
    /// 與 `2A35h`、`2A55h`、`2B15h` 沒有任何可達的程式引用，不在這裡。掃射兩個朝向共用 `2B35h`。
    pub(crate) const fn table_offset(self, facing: Facing) -> u16 {
        match (self, facing) {
            (Shot::Plain, Facing::Right) => 0x28D5,
            (Shot::Plain, Facing::Left) => 0x2A15,
            (Shot::Big, Facing::Right) => 0x2935,
            (Shot::Big, Facing::Left) => 0x2A75,
            (Shot::SpreadUp, Facing::Right) => 0x2955,
            (Shot::SpreadDown, Facing::Right) => 0x2975,
            (Shot::SpreadUp, Facing::Left) => 0x2A95,
            (Shot::SpreadDown, Facing::Left) => 0x2AB5,
            (Shot::SeekerUp, Facing::Right) => 0x2995,
            (Shot::SeekerDown, Facing::Right) => 0x29B5,
            (Shot::SeekerUp, Facing::Left) => 0x2AD5,
            (Shot::SeekerDown, Facing::Left) => 0x2AF5,
            (Shot::Laser, Facing::Right) => 0x29D5,
            (Shot::Laser, Facing::Left) => 0x29F5,
            (Shot::Sweep, _) => 0x2B35,
        }
    }
}

impl Core {
    /// 讀一筆敵彈樣板（15 個字組）。
    pub(crate) fn bullet_template(&self, template: Template) -> Bullet {
        Bullet::read(&self.ds, template.table_offset())
    }

    /// 讀一筆玩家彈樣板（16 個字組）。
    pub(crate) fn player_bullet_template(&self, shot: Shot, facing: Facing) -> PlayerBullet {
        PlayerBullet::read(&self.ds, shot.table_offset(facing))
    }

    /// 唯讀表格的字組與位元組：腳本、圖框表、形狀。
    pub(crate) fn table_word(&self, ptr: u16) -> u16 {
        self.ds.r16(ptr)
    }

    pub(crate) fn table_byte(&self, ptr: u16) -> u8 {
        self.ds.r8(ptr)
    }
}

// ---- 別的領域擁有的共用變數 ----

/// `byte_1CFDD`：存活的玩家，位元 0 是玩家 1，位元 4 是玩家 2。
const PLAYERS_ALIVE: u16 = a(0x1CFDD);
/// 玩家在格網上的位置：玩家 1 欄 `word_1DACB`、列 `word_1DAC9`；玩家 2 欄 `word_1DB39`、列 `word_1DB37`。
const PLAYER_CELLS: [(u16, u16); 2] = [(a(0x1DACB), a(0x1DAC9)), (a(0x1DB39), a(0x1DB37))];
/// `word_26872`：目前繪製頁的偏移，0 或 8000h。
const DRAW_PAGE: u16 = a(0x26872);

impl Core {
    /// 玩家（0 是玩家 1，1 是玩家 2）是否存活。
    pub(crate) fn player_alive(&self, player: usize) -> bool {
        let bit = if player == 0 { 0x01 } else { 0x10 };
        self.ds.r8(PLAYERS_ALIVE) & bit != 0
    }

    /// 玩家在格網上的位置：(欄, 列)。
    pub(crate) fn player_cell(&self, player: usize) -> (u16, u16) {
        let (col, row) = PLAYER_CELLS[player];
        (self.ds.r16(col), self.ds.r16(row))
    }

    /// 目前繪製頁的偏移：奇偶幀交替，0 或 8000h。
    pub(crate) fn draw_page(&self) -> u16 {
        self.ds.r16(DRAW_PAGE)
    }

    /// 測試用：設玩家的存活與位置。`None` 表示不存活，位置不動。
    #[cfg(test)]
    pub(crate) fn set_players(&mut self, players: [Option<(u16, u16)>; 2]) {
        let mut alive = 0u8;
        for (i, p) in players.into_iter().enumerate() {
            if let Some((col, row)) = p {
                alive |= if i == 0 { 0x01 } else { 0x10 };
                self.set_player_cell(i, col, row);
            }
        }
        self.ds.w8(PLAYERS_ALIVE, alive);
    }

    #[cfg(test)]
    pub(crate) fn set_player_cell(&mut self, player: usize, col: u16, row: u16) {
        let (c, r) = PLAYER_CELLS[player];
        self.ds.w16(c, col);
        self.ds.w16(r, row);
    }

    #[cfg(test)]
    pub(crate) fn set_draw_page(&mut self, page: u16) {
        self.ds.w16(DRAW_PAGE, page);
    }

    /// 測試用：擺上原版的兩張列表（`列 × 28h`、`列 × 50h + 1`），不用原版檔的測試要它才能算格位址。
    #[cfg(test)]
    pub(crate) fn fill_row_tables(&mut self) {
        for r in 0..20u16 {
            self.ds.w16(GRID_ROW_BASE_TABLE + 2 * r, r * 0x28);
            self.ds.w16(GRID_ROW_TABLE + 2 * r, r * 0x50 + 1);
        }
        self.ds.w16(GRID_ROW_TABLE + 2 * 20, 0x59);
    }
}

/// 實體池指標表（`2294h`）：15 個槽，每槽一個字組。G1 的格值是槽號加 1，沒有範圍檢查，
/// 所以索引可以讀到表之後的資料（裝甲格 FEh 的索引 FDh 讀到 `248Eh`）。
const ENTITY_LINKS: u16 = 0x2294;
/// 實體記錄裡的傷害字組（w16，記錄 +20h）：低位元組是累計傷害，高位元組為 1 表示本幀被打中。
const ENTITY_DAMAGE: u16 = 0x20;
/// `byte_1CFDB`：目前的關卡字母，`C` 是第三關。
const STAGE_LETTER: u16 = a(0x1CFDB);
const STAGE_THREE: u8 = b'C';

impl Core {
    /// 實體池指標表第 `index` 項（槽號，G1 格值減 1）。0 表示該槽已釋放。
    pub(crate) fn entity_link(&self, index: u8) -> u16 {
        self.ds.r16(ENTITY_LINKS.wrapping_add(2 * u16::from(index)))
    }

    /// 對指標為 `link` 的實體累計 `damage` 點傷害（玩家子彈命中，原版 `16946h`）：
    /// 低位元組 8 位元相加，高位元組設 1。`link` 為 0（已釋放的槽）時，寫到 `dseg:0020h`。
    pub(crate) fn add_entity_damage(&mut self, link: u16, damage: u8) {
        let at = link.wrapping_add(ENTITY_DAMAGE);
        let low = (self.ds.r16(at) as u8).wrapping_add(damage);
        self.ds.w16(at, 0x0100 | u16::from(low));
    }

    /// 目前是不是第三關（有 G4 地形格）。
    pub(crate) fn on_terrain_stage(&self) -> bool {
        self.ds.r8(STAGE_LETTER) == STAGE_THREE
    }

    /// 測試用：擺一個實體，實體池指標表第 `index` 項指向 `link`，傷害字組設成 `damage`。
    #[cfg(test)]
    pub(crate) fn set_entity(&mut self, index: u8, link: u16, damage: u16) {
        self.ds.w16(ENTITY_LINKS + 2 * u16::from(index), link);
        self.ds.w16(link.wrapping_add(ENTITY_DAMAGE), damage);
    }

    /// 測試用：指標為 `link` 的實體的傷害字組。`link` 為 0 時是 `dseg:0020h`。
    #[cfg(test)]
    pub(crate) fn entity_damage(&self, link: u16) -> u16 {
        self.ds.r16(link.wrapping_add(ENTITY_DAMAGE))
    }

    /// 測試用：設關卡字母。
    #[cfg(test)]
    pub(crate) fn set_stage_letter(&mut self, letter: u8) {
        self.ds.w8(STAGE_LETTER, letter);
    }
}

// ---- 格網 ----

/// G1 實體格網（`1B32h`，800 位元組）的起點。這裡只用來說明 G2 列表的別名。
pub(crate) const GRID1_START: u16 = 0x1B32;
/// G1 的終點（不含）。
pub(crate) const GRID1_END: u16 = 0x1E2A;
/// G1 與 G2 的列距：40 欄，每格一位元組。
pub(crate) const GRID1_ROW_BYTES: u16 = 0x28;
/// 一列最後可測的格相對列起點的位移加 1：列尾（欄 39）嚴格在它之前。
const GRID1_ROW_SPAN: u16 = 0x27;
/// 玩家彈取 G1 列起點時，列（無號）大於等於這個值一律改取 [`GRID1_FAR_ROW`]。
const GRID1_NEAR_ROWS: u16 = 0x17C;
/// 越界列的索引：列基底表之後第 1F4h 項，位址 `1EF2h`，G1 之後的資料，實測為 0。
const GRID1_FAR_ROW: u16 = 0x1F4;
/// G4 地形格（第三關，每列 50h 位元組，奇數位址的字組）的範圍，閉區間。
const GRID4_START: u16 = 0x900C;
const GRID4_LAST: u16 = 0x95FC;

/// 一列 G1 的列尾位址（不含）：列起點加 27h。
pub(crate) const fn grid1_row_end(row_start: u16) -> u16 {
    row_start.wrapping_add(GRID1_ROW_SPAN)
}

impl Core {
    /// G1 某一列的起點（原版 `sub_16CC6`，玩家彈用）。列（無號）大於等於 17Ch 改取索引 1F4h；
    /// 小於的列直接以列號索引列基底表，所以列 14h 至 17Bh 讀到的是 G1 自己的內容，原版行為。
    /// 列基底表第 14h 項就是 G1 的前兩個位元組，列 0 之前的負列落在越界列。
    pub(crate) fn grid1_row_start(&self, row: u16) -> u16 {
        let row = if row >= GRID1_NEAR_ROWS { GRID1_FAR_ROW } else { row };
        self.ds.r16(row.wrapping_shl(1).wrapping_add(GRID_ROW_BASE_TABLE)).wrapping_add(GRID1_START)
    }

    /// `addr` 的 G1 格值，沒有範圍檢查（追蹤彈掃描與雷射掃描直接讀）。
    pub(crate) fn grid1_byte(&self, addr: u16) -> u8 {
        self.ds.r8(addr)
    }

    /// 玩家彈碰撞測的 G1 格（原版 `sub_16B49`）：位址須在 G1 之內，嚴格在該列起點之後、嚴格在列尾之前。
    /// 不合格回傳 `None`。
    pub(crate) fn grid1_probe(&self, row_start: u16, addr: u16) -> Option<u8> {
        let inside = addr < GRID1_END && addr > GRID1_START && addr < grid1_row_end(row_start) && addr > row_start;
        inside.then(|| self.ds.r8(addr))
    }

    /// G4 地形格的位址（原版 `sub_16A2C`）：`列` 大於等於 14h 改取 14h，位址 = G4 起點 + 列起點表 + 2 × 欄 + 1。
    /// 與 G3 共用列起點表（[`Core::grid3_cell_addr`]）。
    pub(crate) fn grid4_cell_addr(&self, row: u16, col: u16) -> u16 {
        let row = row.min(GRID_LAST_ROW);
        let row_start = self.ds.r16(row.wrapping_shl(1).wrapping_add(GRID_ROW_TABLE));
        row_start.wrapping_add(GRID4_START).wrapping_add(col.wrapping_shl(1).wrapping_add(1))
    }

    /// G4 在 `addr` 的字組，沒有範圍檢查。
    pub(crate) fn grid4_word(&self, addr: u16) -> u16 {
        self.ds.r16(addr)
    }

    /// `addr` 是否在 G4 的範圍內（原版 `169E0h` 的兩個比較）。
    pub(crate) fn grid4_contains(&self, addr: u16) -> bool {
        (GRID4_START..=GRID4_LAST).contains(&addr)
    }
}
/// G2 敵彈格網（`2C1Ch`，40 欄 × 20 列，每格一位元組，列距 28h）。
pub(crate) const GRID2_START: u16 = 0x2C1C;
const GRID2_BYTES: u16 = 0x320;
/// 敵彈寫 G2 的位址上限（含）。比 G2 的最後一格再多一格。
const GRID2_LAST_WRITABLE: u16 = GRID2_START + GRID2_BYTES;
/// 列基底表（`1B0Ah`）：每列一個字組，`列 × 28h`，20 項。
const GRID_ROW_BASE_TABLE: u16 = 0x1B0A;
/// 可寫的欄範圍是列基底之後、列基底 + 27h 之前（欄 1 至 38）。
const GRID2_ROW_END: u16 = 0x27;

const _: () = assert!(
    GRID_ROW_BASE_TABLE + 2 * GRID_LAST_ROW == GRID1_START,
    "列基底表的第 20 項（索引 14h）就是 G1 的前兩個位元組"
);

impl Core {
    /// 清 G2（原版 `sub_178BB`，每幀末）。修正模式另存的槽號一起清。
    pub(crate) fn clear_grid2(&mut self) {
        self.ds.fill16(GRID2_START, GRID2_BYTES / 2, 0);
        self.g2_slots.fill(0);
    }

    /// G2 某一列的基底位址（原版 `sub_17898`）。列（無號）大於等於 14h，含負列，改取索引 14h；
    /// 該項的位址是 G1 的前兩個位元組，所以列基底取到的是 G1 的內容，原版行為。
    /// 道具球 2×3 在列 18 的移除幀會寫到列 20，走到這個分支。
    pub(crate) fn grid2_row_base(&self, row: u16) -> u16 {
        let row = row.min(GRID_LAST_ROW);
        self.ds.r16(row.wrapping_shl(1).wrapping_add(GRID_ROW_BASE_TABLE)).wrapping_add(GRID2_START)
    }

    /// `addr` 是否可寫：落在 `2C1Ch` 至 `2F3Ch` 之間，且嚴格在列基底之後（欄 0 不寫）、嚴格在列基底 + 27h 之前。
    pub(crate) fn grid2_writable(&self, row_base: u16, addr: u16) -> bool {
        addr <= GRID2_LAST_WRITABLE && addr >= GRID2_START && addr < row_base.wrapping_add(GRID2_ROW_END) && addr > row_base
    }

    pub(crate) fn grid2_write(&mut self, addr: u16, v: u8) {
        self.ds.w8(addr, v);
    }

    /// `addr` 在 G2 裡的索引，`g2_slots` 用。
    pub(crate) fn grid2_index(&self, addr: u16) -> usize {
        addr.wrapping_sub(GRID2_START) as usize
    }
}

/// G3 格網（碰撞用的髒格，每列 50h 位元組，奇數位元組）的範圍。
const GRID3_START: u16 = 0x964C;
const GRID3_END: u16 = 0x9C3C;
/// 一列 40 格，每格 2 位元組，所以列距 50h，列內可寫的範圍是 `[第一格, 第一格 + 4Eh]`。
pub(crate) const GRID3_ROW_BYTES: u16 = 0x50;
const GRID3_ROW_LAST_CELL: u16 = 0x4E;
/// 列起點表（`21C2h`）：每列一個字組。列 0 至 13h 從 0001h 起每列加 50h，列 14h 是越界列（0059h）。G3 與 G4 共用。
const GRID_ROW_TABLE: u16 = 0x21C2;
/// 列號（無號）大於等於這個值一律當作這一列。
const GRID_LAST_ROW: u16 = 0x14;

impl Core {
    /// 以 `or` 寫進 G3 的一格，限定在 `first`（該列第一格的位址）起的這一列內。
    /// 敵彈用這個：超出該列右緣的格略過，不溢進下一列。
    pub(crate) fn grid3_or_in_row(&mut self, first: u16, addr: u16, v: u8) {
        if first <= addr && addr <= first.wrapping_add(GRID3_ROW_LAST_CELL) {
            let old = self.ds.r8(addr);
            self.ds.w8(addr, old | v);
        }
    }

    /// G3 的格位址（原版 `sub_17D5F`）：`列` 大於等於 14h 改取 14h，位址 = G3 起點 + 列起點表 + 2 × 欄 + 1。
    /// 全部以 16 位元環繞運算，所以越界的位址由 [`Core::grid3_or`] 擋掉。
    pub(crate) fn grid3_cell_addr(&self, row: u16, col: u16) -> u16 {
        let row = row.min(GRID_LAST_ROW);
        let row_start = self.ds.r16(row.wrapping_shl(1).wrapping_add(GRID_ROW_TABLE));
        row_start.wrapping_add(GRID3_START).wrapping_add(col.wrapping_shl(1).wrapping_add(1))
    }

    /// 以 `or` 寫進 G3 的一格。位址落在 G3 範圍之外的格略過。
    pub(crate) fn grid3_or(&mut self, addr: u16, v: u8) {
        if (GRID3_START..=GRID3_END).contains(&addr) {
            let old = self.ds.r8(addr);
            self.ds.w8(addr, old | v);
        }
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

    #[test]
    fn grid3_or_ignores_addresses_outside_g3() {
        let mut c = core();
        c.grid3_or(GRID3_START, 0x03);
        c.grid3_or(GRID3_START, 0x04);
        c.grid3_or(GRID3_END, 0x80);
        assert_eq!((c.ds.r8(GRID3_START), c.ds.r8(GRID3_END)), (0x07, 0x80), "範圍內的格以 or 累積，兩端都算");
        let before = c.ds.bytes().to_vec();
        for addr in [GRID3_START - 1, GRID3_END + 1, 0, 0xFFFF] {
            c.grid3_or(addr, 0xFF);
        }
        assert_eq!(c.ds.bytes(), &before[..], "範圍外的格不寫");
    }

    #[test]
    fn grid3_rows_from_14h_share_the_out_of_range_row() {
        let mut c = core();
        c.ds.w16(GRID_ROW_TABLE + 2 * 0x13, 0x05F1);
        c.ds.w16(GRID_ROW_TABLE + 2 * 0x14, 0x0059);
        assert_eq!(c.grid3_cell_addr(0x14, 3), c.grid3_cell_addr(0x1234, 3));
        assert_eq!(c.grid3_cell_addr(0xFFFF, 3), c.grid3_cell_addr(0x14, 3), "負的列號也是越界列");
        assert_ne!(c.grid3_cell_addr(0x13, 3), c.grid3_cell_addr(0x14, 3), "列 13h 是最後一個一般列");
    }

    #[test]
    fn grid3_cell_addr_is_start_plus_row_table_plus_two_cols_plus_one() {
        let mut c = core();
        c.ds.w16(GRID_ROW_TABLE + 2 * 5, 0x0101);
        assert_eq!(c.grid3_cell_addr(5, 7), GRID3_START + 0x0101 + 2 * 7 + 1);
        // 環繞：欄乘 2 超過 16 位元時高位元丟掉。
        assert_eq!(c.grid3_cell_addr(5, 0x8001), GRID3_START + 0x0101 + 2 + 1);
    }

    #[test]
    fn grid3_or_in_row_only_writes_inside_the_row() {
        let mut c = core();
        let first = GRID3_START + 0x51;
        c.grid3_or_in_row(first, first, 0x01);
        c.grid3_or_in_row(first, first, 0x02);
        c.grid3_or_in_row(first, first + 0x4E, 0x80);
        assert_eq!((c.ds.r8(first), c.ds.r8(first + 0x4E)), (0x03, 0x80), "列內的格以 or 累積，兩端都算");
        c.grid3_or_in_row(first, first + 0x50, 0xFF);
        c.grid3_or_in_row(first, first - 1, 0xFF);
        assert_eq!((c.ds.r8(first + 0x50), c.ds.r8(first - 1)), (0, 0), "列外不寫，即使仍在 G3 內");
    }

    fn pseudo_random_image() -> Vec<u8> {
        let mut image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
        let mut s = 0x9E37_79B9u32;
        for b in image[rs2_formats::DSEG_IMAGE_OFFSET..].iter_mut() {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *b = (s >> 24) as u8;
        }
        image
    }

    #[test]
    fn enemy_bullets_mirror_is_the_inverse_of_load_for_any_dump() {
        let ds = Dseg::from_image(&pseudo_random_image());
        let mut eb = EnemyBullets::default();
        eb.load(&ds);
        let mut same = ds.clone();
        eb.store(&mut same);
        assert_eq!(same.bytes(), ds.bytes(), "store(load(ds)) 要等於 ds");
        // 寫進全 0 的資料段再讀回，結構相同：store 真的寫了每個欄位。
        let mut blank = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        eb.store(&mut blank);
        let mut back = EnemyBullets::default();
        back.load(&blank);
        assert_eq!(back.slots, eb.slots);
        assert_ne!(eb.slots, EnemyBullets::default().slots, "測試資料不該全 0");
    }

    #[test]
    fn enemy_bullet_fields_land_at_the_documented_offsets() {
        let mut eb = EnemyBullets::default();
        for (i, slot) in eb.slots.iter_mut().enumerate() {
            let w: [u16; 15] = std::array::from_fn(|k| 0x100 * (i as u16 + 1) + k as u16 + 1);
            *slot = Slot { link: 0x7000 + i as u16, bullet: Bullet::from_words(w) };
        }
        let mut ds = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        eb.store(&mut ds);
        for i in 0..EB_POOL_SIZE as u16 {
            assert_eq!(ds.r16(0x32B3 + 2 * i), 0x7000 + i, "槽 {i} 的指標在 32B3h + 2i");
            let rec = 0x32DB + 0x1E * i;
            for k in 0..15u16 {
                assert_eq!(ds.r16(rec + 2 * k), 0x100 * (i + 1) + k + 1, "槽 {i} 的 w{k} 在記錄 +{:02X}h", 2 * k);
            }
        }
        // 具名欄位：w0 heading、w1 frames、w2 blit_x、w3 blit_y、w4 blit_dx、w5 blit_dy、w6 spare、w7 trail、
        // w8 library、w9 col、w10 row、w11 dcol、w12 drow、w13 script、w14 shape。
        let b = eb.slots[0].bullet;
        assert_eq!(
            [b.heading, b.frames, b.blit_x, b.blit_y, b.blit_dx, b.blit_dy, b.spare, b.trail],
            [0x101, 0x102, 0x103, 0x104, 0x105, 0x106, 0x107, 0x108]
        );
        assert_eq!(
            [b.library, b.col, b.row, b.dcol, b.drow, b.script, b.shape],
            [0x109, 0x10A, 0x10B, 0x10C, 0x10D, 0x10E, 0x10F]
        );
        assert_eq!(Bullet::from_words(b.to_words()), b);
        assert_eq!(record_link(3), 0x32DB + 3 * 0x1E);
    }

    #[test]
    fn player_bullets_mirror_is_the_inverse_of_load_for_any_dump() {
        let ds = Dseg::from_image(&pseudo_random_image());
        let mut pb = PlayerBullets::default();
        pb.load(&ds);
        let mut same = ds.clone();
        pb.store(&mut same);
        assert_eq!(same.bytes(), ds.bytes(), "store(load(ds)) 要等於 ds");
        // 寫進全 0 的資料段再讀回，結構相同：store 真的寫了每個欄位。
        let mut blank = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        pb.store(&mut blank);
        let mut back = PlayerBullets::default();
        back.load(&blank);
        assert_eq!((back.slots, back.seekers_in_flight), (pb.slots, pb.seekers_in_flight));
        assert_ne!(pb.slots, PlayerBullets::default().slots, "測試資料不該全 0");
        assert_ne!(pb.seekers_in_flight, 0);
    }

    #[test]
    fn player_bullet_fields_land_at_the_documented_offsets() {
        let mut pb = PlayerBullets::default();
        pb.seekers_in_flight = 0x4321;
        for (i, slot) in pb.slots.iter_mut().enumerate() {
            let w: [u16; 16] = std::array::from_fn(|k| 0x100 * (i as u16 + 1) + k as u16 + 1);
            *slot = PbSlot { link: 0x7000 + i as u16, bullet: PlayerBullet::from_words(w) };
        }
        let mut ds = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        pb.store(&mut ds);
        assert_eq!(ds.r16(0x25E8), 0x4321, "追蹤彈計數在 25E8h（word_1F1A8）");
        for i in 0..PB_POOL_SIZE as u16 {
            assert_eq!(ds.r16(0x2607 + 2 * i), 0x7000 + i, "槽 {i} 的指標在 2607h + 2i");
            let rec = 0x262F + 0x20 * i;
            for k in 0..16u16 {
                assert_eq!(ds.r16(rec + 2 * k), 0x100 * (i + 1) + k + 1, "槽 {i} 的 w{k} 在記錄 +{:02X}h", 2 * k);
            }
        }
        // 具名欄位：w0 kind、w1 image、w2 blit_x、w3 blit_y、w4 blit_dx、w5 blit_dy、w6 heading、w7 owner、
        // w8 spare、w9 col、w10 row、w11 dcol、w12 drow、w13 script、w14 shape、w15 hit_effect。
        let b = pb.slots[0].bullet;
        assert_eq!(
            [b.kind, b.image, b.blit_x, b.blit_y, b.blit_dx, b.blit_dy, b.heading, b.owner],
            [0x101, 0x102, 0x103, 0x104, 0x105, 0x106, 0x107, 0x108]
        );
        assert_eq!(
            [b.spare, b.col, b.row, b.dcol, b.drow, b.script, b.shape, b.hit_effect],
            [0x109, 0x10A, 0x10B, 0x10C, 0x10D, 0x10E, 0x10F, 0x110]
        );
        assert_eq!(PlayerBullet::from_words(b.to_words()), b);
        assert_eq!(pb_record_link(3), 0x262F + 3 * 0x20);
        // 追蹤彈計數、指標表、記錄區互不重疊，也不碰敵彈池與特效池。
        assert_eq!(ds.r16(0x25E8 + 2), 0, "計數之後的字組不動");
        assert_eq!(ds.r16(0x2607 + 0x28 - 2), 0x7000 + 19);
        assert_eq!(ds.r16(0x262F + 0x280), 0, "記錄區之後的字組（word_1F46F）不動");
    }

    #[test]
    fn player_shot_template_addresses_match_the_original_tables() {
        // docs/re/11-entities-combat.md 第 6.3 節與機器碼 165CFh、1669Fh 的立即值。
        let right = [
            Shot::Plain,
            Shot::Big,
            Shot::SpreadUp,
            Shot::SpreadDown,
            Shot::SeekerUp,
            Shot::SeekerDown,
            Shot::Laser,
            Shot::Sweep,
        ]
        .map(|s| s.table_offset(Facing::Right));
        assert_eq!(right, [0x28D5, 0x2935, 0x2955, 0x2975, 0x2995, 0x29B5, 0x29D5, 0x2B35]);
        let left = [
            Shot::Plain,
            Shot::Big,
            Shot::SpreadUp,
            Shot::SpreadDown,
            Shot::SeekerUp,
            Shot::SeekerDown,
            Shot::Laser,
            Shot::Sweep,
        ]
        .map(|s| s.table_offset(Facing::Left));
        assert_eq!(left, [0x2A15, 0x2A75, 0x2A95, 0x2AB5, 0x2AD5, 0x2AF5, 0x29F5, 0x2B35]);
    }

    #[test]
    fn player_bullet_template_reads_sixteen_words_at_the_table_offset() {
        let mut c = core();
        let at = Shot::Laser.table_offset(Facing::Left);
        for k in 0..16u16 {
            c.ds.w16(at + 2 * k, 0x600 + k);
        }
        let b = c.player_bullet_template(Shot::Laser, Facing::Left);
        assert_eq!(b.to_words(), std::array::from_fn(|k| 0x600 + k as u16));
        assert_eq!(c.player_bullet_template(Shot::Laser, Facing::Right), PlayerBullet::default());
    }

    #[test]
    fn grid1_row_start_reads_the_row_table_and_then_g1_itself() {
        let mut c = core();
        c.fill_row_tables();
        assert_eq!(c.grid1_row_start(0), GRID1_START);
        assert_eq!(c.grid1_row_start(5), GRID1_START + 5 * 0x28);
        // 列 14h 至 17Bh 直接以列號索引列基底表，讀到 G1 的內容（索引 14h 就是 G1 的前兩個位元組）。
        c.ds.w16(GRID1_START, 0x0123);
        c.ds.w16(GRID1_START + 4, 0x0200);
        assert_eq!(c.grid1_row_start(0x14), GRID1_START + 0x0123);
        assert_eq!(c.grid1_row_start(0x16), GRID1_START + 0x0200);
        // 列（無號）大於等於 17Ch，包含負列，一律取索引 1F4h（1EF2h，G1 之後，這裡是 0）。
        c.ds.w16(0x1EF2, 0x0040);
        for row in [0x17C, 0x200, 0xFFFF] {
            assert_eq!(c.grid1_row_start(row), GRID1_START + 0x0040, "列 {row:#x}");
        }
        assert_ne!(c.grid1_row_start(0x17B), c.grid1_row_start(0x17C));
    }

    #[test]
    fn grid1_probe_excludes_the_row_ends_and_the_outside_of_g1() {
        let mut c = core();
        c.fill_row_tables();
        let start = c.grid1_row_start(3);
        c.ds.w8(start, 7);
        c.ds.w8(start + 1, 8);
        c.ds.w8(start + 0x26, 9);
        c.ds.w8(start + 0x27, 10);
        assert_eq!(c.grid1_probe(start, start), None, "欄 0");
        assert_eq!(c.grid1_probe(start, start + 1), Some(8));
        assert_eq!(c.grid1_probe(start, start + 0x26), Some(9), "欄 38");
        assert_eq!(c.grid1_probe(start, start + 0x27), None, "欄 39");
        // 列起點在 G1 之外：位址必須先在 G1 之內。
        assert_eq!(c.grid1_probe(GRID1_START - 0x40, GRID1_START - 0x20), None);
        assert_eq!(c.grid1_probe(GRID1_END - 5, GRID1_END - 2), Some(0));
        assert_eq!(c.grid1_probe(GRID1_END - 5, GRID1_END), None);
    }

    #[test]
    fn grid4_cell_addr_is_start_plus_row_table_plus_two_cols_plus_one() {
        let mut c = core();
        c.fill_row_tables();
        assert_eq!(c.grid4_cell_addr(3, 5), GRID4_START + 3 * 0x50 + 1 + 2 * 5 + 1);
        assert_eq!(c.grid4_cell_addr(0xFFFF, 5), c.grid4_cell_addr(0x14, 5), "負列也是越界列");
        assert!(c.grid4_contains(GRID4_START) && c.grid4_contains(GRID4_LAST));
        assert!(!c.grid4_contains(GRID4_START - 1) && !c.grid4_contains(GRID4_LAST + 1));
    }

    #[test]
    fn entity_accessors_follow_the_pointer_table_without_a_range_check() {
        let mut c = core();
        c.ds.w16(0x2294 + 2 * 3, 0x22F0);
        assert_eq!(c.entity_link(3), 0x22F0);
        c.ds.w16(0x248E, 0x1111);
        assert_eq!(c.entity_link(0xFD), 0x1111, "索引 FDh 讀到表之後的 248Eh");
        c.ds.w16(0x22F0 + 0x20, 0x0203);
        c.add_entity_damage(0x22F0, 3);
        assert_eq!(c.ds.r16(0x22F0 + 0x20), 0x0106, "低位元組相加，高位元組設 1");
        c.ds.w16(0x0020, 0x00FF);
        c.add_entity_damage(0, 2);
        assert_eq!(c.ds.r16(0x0020), 0x0101, "指標 0 寫到 dseg:0020h，低位元組環繞");
    }

    #[test]
    fn terrain_stage_is_the_stage_letter_c() {
        let mut c = core();
        assert!(!c.on_terrain_stage());
        c.ds.w8(crate::dseg::a(0x1CFDB), b'C');
        assert!(c.on_terrain_stage());
        c.ds.w8(crate::dseg::a(0x1CFDB), b'c');
        assert!(!c.on_terrain_stage(), "只有大寫 C（43h）算，小寫 c（63h）不算");
    }

    #[test]
    fn clear_link_word_past_the_table_lands_in_the_records() {
        let mut eb = EnemyBullets::default();
        for s in eb.slots.iter_mut() {
            s.link = 0x1234;
            s.bullet = Bullet::from_words([0xFFFF; 15]);
        }
        eb.clear_link_word(3);
        assert_eq!(eb.slots[3].link, 0);
        eb.clear_link_word(20);
        assert_eq!(eb.slots[0].bullet.heading, 0, "項 20 是槽 0 的 w0");
        eb.clear_link_word(20 + 15 + 14);
        assert_eq!(eb.slots[1].bullet.shape, 0, "槽 1 的 w14");
        eb.clear_link_word(255);
        assert_eq!(eb.slots[15].bullet.row, 0, "項 255 是槽 15 的 w10");
        // 只動一個字組。
        let zeros = eb.slots.iter().map(|s| s.bullet.to_words().iter().filter(|&&w| w == 0).count()).sum::<usize>();
        assert_eq!(zeros, 3);
        assert_eq!(eb.slots.iter().filter(|s| s.link == 0).count(), 1);
    }

    #[test]
    fn template_addresses_match_the_original_tables() {
        use crate::ebullet::{Heading as H, Spread, Tilt};
        // docs/re/11-entities-combat.md 第 7.2 節與機器碼 16D3Bh 起的立即值。
        assert_eq!(Template::Aimed(H::N).table_offset(), 0x3557);
        assert_eq!(Template::Aimed(H::NW).table_offset(), 0x3629);
        assert_eq!(Template::Capsule(Capsule::L, H::N).table_offset(), 0x3737);
        assert_eq!(Template::Capsule(Capsule::B, H::N).table_offset(), 0x3827);
        assert_eq!(Template::Capsule(Capsule::K, H::N).table_offset(), 0x3917);
        assert_eq!(Template::Capsule(Capsule::S, H::NW).table_offset(), 0x3A07 + 7 * 0x1E);
        assert_eq!(Template::Unused.table_offset(), 0x3AF7);
        assert_eq!(
            [Tilt::NorthWest, Tilt::West, Tilt::SouthWest].map(|t| Template::Wide(t).table_offset()),
            [0x3B15, 0x3B33, 0x3B51]
        );
        assert_eq!(
            [Spread::W, Spread::NW, Spread::SW, Spread::N, Spread::S].map(|s| Template::Spread(s).table_offset()),
            [0x3B6F, 0x3B8D, 0x3BAB, 0x3BC9, 0x3BE7]
        );
        assert_eq!(Template::WideFlash.table_offset(), 0x3C05);
        assert_eq!(Template::Flash.table_offset(), 0x3C23);
        assert_eq!(
            [H::N, H::NE, H::E, H::SE, H::S, H::SW, H::W, H::NW].map(|h| Template::Straight(h).table_offset()),
            [0x3C41, 0x3C5F, 0x3C7D, 0x3C9B, 0x3CB9, 0x3CD7, 0x3CF5, 0x3D13]
        );
        assert_eq!(
            [Template::SlowSw, Template::SlowW, Template::SlowNw].map(|t| t.table_offset()),
            [0x3D31, 0x3D4F, 0x3D6D]
        );
        assert_eq!([Template::BouncerNw, Template::BouncerNe].map(|t| t.table_offset()), [0x3D8B, 0x3DA9]);
    }

    #[test]
    fn bullet_template_reads_fifteen_words_at_the_table_offset() {
        let mut c = core();
        let at = Template::Flash.table_offset();
        for k in 0..15u16 {
            c.ds.w16(at + 2 * k, 0x500 + k);
        }
        let b = c.bullet_template(Template::Flash);
        assert_eq!(b.to_words(), std::array::from_fn(|k| 0x500 + k as u16));
        assert_eq!(c.bullet_template(Template::WideFlash), Bullet::default());
    }

    #[test]
    fn player_accessors_read_the_player_variables() {
        let mut c = core();
        c.set_players([Some((3, 4)), Some((5, 6))]);
        assert_eq!((c.player_alive(0), c.player_alive(1)), (true, true));
        assert_eq!((c.player_cell(0), c.player_cell(1)), ((3, 4), (5, 6)));
        c.set_players([None, Some((9, 8))]);
        assert_eq!((c.player_alive(0), c.player_alive(1)), (false, true));
        assert_eq!(c.player_cell(1), (9, 8));
    }

    #[test]
    fn grid2_writable_excludes_column_0_and_39_but_allows_one_cell_past_g2() {
        let c = core();
        let base = GRID2_START + 5 * 0x28;
        assert!(!c.grid2_writable(base, base), "欄 0 不可寫");
        assert!(c.grid2_writable(base, base + 1));
        assert!(c.grid2_writable(base, base + 0x26), "欄 38 可寫");
        assert!(!c.grid2_writable(base, base + 0x27), "欄 39 不可寫");
        // 範圍上下限：列基底落在 G2 之外的別名列。
        let last = GRID2_START + 19 * 0x28;
        assert!(c.grid2_writable(last, GRID2_START + GRID2_BYTES - 2), "列 19 的欄 38");
        assert!(!c.grid2_writable(GRID2_START - 5, GRID2_START - 1), "低於 G2 起點不可寫");
        assert!(c.grid2_writable(GRID2_START + GRID2_BYTES - 0x10, GRID2_START + GRID2_BYTES), "2F3Ch 可寫（G2 之後一格）");
        assert!(!c.grid2_writable(GRID2_START + GRID2_BYTES - 0x10, GRID2_START + GRID2_BYTES + 1));
    }

    #[test]
    fn clear_grid2_zeroes_all_of_g2_and_the_slot_shadow() {
        let mut c = core();
        for a in GRID2_START..GRID2_START + GRID2_BYTES {
            c.ds.w8(a, 0x55);
        }
        c.ds.w8(GRID2_START + GRID2_BYTES, 0x66);
        c.g2_slots.fill(7);
        c.clear_grid2();
        assert!((GRID2_START..GRID2_START + GRID2_BYTES).all(|a| c.ds.r8(a) == 0));
        assert_eq!(c.ds.r8(GRID2_START + GRID2_BYTES), 0x66, "G2 之後的一格不清");
        assert!(c.g2_slots.iter().all(|&s| s == 0));
    }
}
