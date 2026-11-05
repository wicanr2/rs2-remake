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

use crate::boss_scroll::{BossLayer, BossLayers};
use crate::dseg::{Dseg, a};
use crate::ebullet::{Bullet, Capsule, EnemyBullets, POOL_SIZE as EB_POOL_SIZE, Slot, Template};
use crate::entity::{Enemies, Enemy, POOL_SIZE as EN_POOL_SIZE, Slot as EnSlot, Sprite, Timetable};
use crate::flow::{Clock, Cheat, Controls, FLAG_OFF, FLAG_ON, FireFlags, Flow, STAGE_1, STAGE_3, STAGE_CLEARED, Scenes};
use crate::fx::{Effect, Effects, POOL_SIZE, Script};
use crate::game::Core;
use crate::pbullet::{Facing, POOL_SIZE as PB_POOL_SIZE, PlayerBullet, PlayerBullets, Shot, Slot as PbSlot};
use crate::player::{PLAYER_COUNT, Player, Players, Pods};
use crate::menu::Menu;
use crate::scenery::{POOL_SIZE as SC_POOL_SIZE, Prop, Scenery};
use crate::title::Title;
use crate::stage1::Backdrop as Stage1Backdrop;
use crate::stage2::{Backdrop as Stage2Backdrop, Strip};
use crate::stage3::MapScroll;
use crate::stage4::Tunnel;

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
        self.en.load(&self.ds);
        self.sc.load(&self.ds);
        self.pl.load(&self.ds);
        self.title.load(&self.ds);
        self.flow.load(&self.ds);
        self.scenes.load(&self.ds);
    }

    /// 所有已搬走的領域寫回 `self.ds`。
    pub(crate) fn store_mirrors(&mut self) {
        self.fx.store(&mut self.ds);
        self.eb.store(&mut self.ds);
        self.pb.store(&mut self.ds);
        self.en.store(&mut self.ds);
        self.sc.store(&mut self.ds);
        self.pl.store(&mut self.ds);
        self.title.store(&mut self.ds);
        self.flow.store(&mut self.ds);
        self.scenes.store(&mut self.ds);
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

// ---- 敵人池 ----

/// 池指標表（`2294h`）：15 槽加一個只會被清零、沒有人讀的備用字組，共 16 個字組。
/// 0 是空槽，使用中是該槽記錄的位址。
const EN_LINKS: u16 = 0x2294;
const EN_SPARE_LINK: u16 = EN_LINKS + 2 * EN_POOL_SIZE as u16;
/// 記錄區：15 筆連續的 24 字組記錄（`22B4h` 起，每筆 30h 位元組）。
const EN_RECORDS: u16 = 0x22B4;
const EN_RECORD_BYTES: u16 = 0x30;
/// 每槽的繪製值：五個平行陣列，各 16 個字組（20h 位元組），只用前 15 項。緊接在 `word_1EDAA`（`21EAh`）之後，
/// 它不在這裡，見 [`ENEMY_DEATH_FLAG`]。
const EN_SPRITE_IMAGE: u16 = 0x21EC;
const EN_SPRITE_X: u16 = 0x220C;
const EN_SPRITE_Y: u16 = 0x222C;
const EN_SPRITE_KIND: u16 = 0x224C;
const EN_FLASH: u16 = 0x226C;
/// 最近一個做完整幀的敵人留下的繪製值，4 個字組，`word_1EE4C` 起。
const EN_LAST_SPRITE: u16 = a(0x1EE4C);

const _: () = assert!(EN_POOL_SIZE == 15, "池大小與原版指標表的長度一致");
const _: () = assert!(EN_RECORDS == EN_SPARE_LINK + 2, "記錄區緊接在指標表之後");
const _: () = assert!(EN_SPRITE_IMAGE == ENEMY_DEATH_FLAG + 2, "繪製陣列緊接在死亡旗標之後");
const _: () = assert!(EN_SPRITE_X == EN_SPRITE_IMAGE + 0x20 && EN_SPRITE_Y == EN_SPRITE_X + 0x20, "繪製陣列各 16 個字組");
const _: () = assert!(EN_SPRITE_KIND == EN_SPRITE_Y + 0x20 && EN_FLASH == EN_SPRITE_KIND + 0x20, "繪製陣列各 16 個字組");
const _: () = assert!(EN_LAST_SPRITE == EN_FLASH + 0x20 && EN_LINKS == EN_LAST_SPRITE + 8, "繪製值緊接在閃爍陣列之後，指標表緊接在它之後");

/// 傷害字組（`damage`，w16）在記錄裡的位移。
pub(crate) const ENEMY_DAMAGE_OFFSET: u16 = 0x20;

/// 第 `slot` 槽記錄的位址，也就是使用中的槽寫進池指標表的值。
pub(crate) const fn enemy_record_link(slot: usize) -> u16 {
    EN_RECORDS + EN_RECORD_BYTES * slot as u16
}

/// 位址 `link` 是不是某一槽記錄的起點，是的話回傳槽號。
pub(crate) fn enemy_record_slot(link: u16) -> Option<usize> {
    let offset = link.checked_sub(EN_RECORDS)?;
    let slot = usize::from(offset / EN_RECORD_BYTES);
    (offset % EN_RECORD_BYTES == 0 && slot < EN_POOL_SIZE).then_some(slot)
}

/// 記錄的欄位順序與位移（`Enemy` 的欄位依序對應 `w0` 至 `w23`），工作區 `25B6h` 與樣板表同一版面：
///
/// | 位移 | 欄位 | 位移 | 欄位 | 位移 | 欄位 |
/// |---|---|---|---|---|---|
/// | +00h | `hp` | +10h | `mode` | +20h | `damage` |
/// | +02h | `frames` | +12h | `col` | +22h | `explosion` |
/// | +04h | `blit_x` | +14h | `row` | +24h | `kind` |
/// | +06h | `blit_y` | +16h | `dcol` | +26h | `carrier` |
/// | +08h | `blit_dx` | +18h | `drow` | +28h | `anim_limits` |
/// | +0Ah | `blit_dy` | +1Ah | `script` | +2Ah | `anim` |
/// | +0Ch | `fire_threshold` | +1Ch | `shape` | +2Ch | `rand_index` |
/// | +0Eh | `bullet` | +1Eh | `spare` | +2Eh | `spill` |
impl Enemy {
    pub(crate) fn from_words(w: [u16; 24]) -> Enemy {
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

    pub(crate) fn to_words(&self) -> [u16; 24] {
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

    fn read(ds: &Dseg, at: u16) -> Enemy {
        Enemy::from_words(std::array::from_fn(|k| ds.r16(at.wrapping_add(2 * k as u16))))
    }
}

impl Mirror for Enemies {
    fn load(&mut self, ds: &Dseg) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let o = 2 * i as u16;
            *slot = EnSlot {
                link: ds.r16(EN_LINKS + o),
                enemy: Enemy::read(ds, enemy_record_link(i)),
                sprite: Sprite {
                    image: ds.r16(EN_SPRITE_IMAGE + o),
                    x: ds.r16(EN_SPRITE_X + o),
                    y: ds.r16(EN_SPRITE_Y + o),
                    kind: ds.r16(EN_SPRITE_KIND + o),
                },
                flash: ds.r16(EN_FLASH + o),
            };
        }
        self.spare_link = ds.r16(EN_SPARE_LINK);
        self.last_sprite = Sprite {
            image: ds.r16(EN_LAST_SPRITE),
            x: ds.r16(EN_LAST_SPRITE + 2),
            y: ds.r16(EN_LAST_SPRITE + 4),
            kind: ds.r16(EN_LAST_SPRITE + 6),
        };
    }

    fn store(&self, ds: &mut Dseg) {
        for (i, slot) in self.slots.iter().enumerate() {
            let o = 2 * i as u16;
            ds.w16(EN_LINKS + o, slot.link);
            for (k, w) in slot.enemy.to_words().into_iter().enumerate() {
                ds.w16(enemy_record_link(i) + 2 * k as u16, w);
            }
            ds.w16(EN_SPRITE_IMAGE + o, slot.sprite.image);
            ds.w16(EN_SPRITE_X + o, slot.sprite.x);
            ds.w16(EN_SPRITE_Y + o, slot.sprite.y);
            ds.w16(EN_SPRITE_KIND + o, slot.sprite.kind);
            ds.w16(EN_FLASH + o, slot.flash);
        }
        ds.w16(EN_SPARE_LINK, self.spare_link);
        ds.w16(EN_LAST_SPRITE, self.last_sprite.image);
        ds.w16(EN_LAST_SPRITE + 2, self.last_sprite.x);
        ds.w16(EN_LAST_SPRITE + 4, self.last_sprite.y);
        ds.w16(EN_LAST_SPRITE + 6, self.last_sprite.kind);
    }
}

// ---- 景物池 ----

/// 景物池的四個平行陣列，各 10 個字組：每幀往左的欄數（`198Ch`）、x（`19A0h`）、y（`19B4h`）、圖庫索引（`19C8h`）。
/// 圖庫索引同時是狀態，0 是空槽。四個陣列連續，之後緊接群組表。
const SC_DX: u16 = 0x198C;
const SC_X: u16 = 0x19A0;
const SC_Y: u16 = 0x19B4;
const SC_IMAGE: u16 = 0x19C8;
/// 群組表（`19DCh`）：每列 `(幀, 景物記錄位址)`，幀為 0 結束。
const SC_GROUPS: u16 = 0x19DC;
const SC_GROUP_BYTES: u16 = 4;

const _: () = assert!(SC_POOL_SIZE == 10, "池大小與原版四個陣列的長度一致");
const _: () = assert!(SC_X == SC_DX + 2 * SC_POOL_SIZE as u16 && SC_Y == SC_X + 2 * SC_POOL_SIZE as u16, "四個陣列連續");
const _: () = assert!(SC_IMAGE == SC_Y + 2 * SC_POOL_SIZE as u16 && SC_GROUPS == SC_IMAGE + 2 * SC_POOL_SIZE as u16, "群組表緊接在陣列之後");

impl Prop {
    /// 景物記錄（群組表指向的 4 個字組）的欄位順序：`(圖庫索引, x, y, 每幀往左的欄數)`。
    pub(crate) fn from_words(w: [u16; 4]) -> Prop {
        Prop { image: w[0], x: w[1], y: w[2], dx: w[3] }
    }
}

impl Mirror for Scenery {
    fn load(&mut self, ds: &Dseg) {
        for (i, prop) in self.props.iter_mut().enumerate() {
            let o = 2 * i as u16;
            *prop = Prop { image: ds.r16(SC_IMAGE + o), x: ds.r16(SC_X + o), y: ds.r16(SC_Y + o), dx: ds.r16(SC_DX + o) };
        }
    }

    fn store(&self, ds: &mut Dseg) {
        for (i, prop) in self.props.iter().enumerate() {
            let o = 2 * i as u16;
            ds.w16(SC_IMAGE + o, prop.image);
            ds.w16(SC_X + o, prop.x);
            ds.w16(SC_Y + o, prop.y);
            ds.w16(SC_DX + o, prop.dx);
        }
    }
}

// ---- 玩家 ----

/// 存活位元組，`byte_1CFDD`：位元 0 是玩家 1，位元 4 是玩家 2。
const PLAYERS_ALIVE: u16 = a(0x1CFDD);

/// 一位玩家的變數在資料段的位移。兩位玩家的變數分在三段（血量在 `1CFDEh` 附近，砲座與狀態在 `1DA8Ah` 一帶，
/// 位置在 `1DAC5h` 與 `1DB37h`），段內的間距也不同，所以逐項列，不用固定的位移差。
///
/// 玩家 1 的欄位與位移（玩家 2 的在 [`PLAYER_OFFSETS`]，兩邊逐項對應）：
///
/// | 欄位 | 位移 | 寬 | 欄位 | 位移 | 寬 |
/// |---|---|---|---|---|---|
/// | `hp` | `041Eh` | 2 | `pods.facing` | `0ECFh` | 2 |
/// | `charge` | `0ECBh` | 2 | `pods.image` | `0ED4h` | 2 |
/// | `pods.first` | `0ECDh` | 1 | `pods.anim` | `0ED6h` | 2 |
/// | `pods.second` | `0ECEh` | 1 | `pods.col` | `0ED8h` | 2 |
/// | `pods.row` | `0EDAh` | 2 | `pods.blit_x` | `0EE0h` | 2 |
/// | `pods.blit_y` | `0EE2h` | 2 | `input_bits` | `0EE4h` | 1 |
/// | `fired` | `0EE5h` | 1 | `beam_y` | `0EE6h` | 2 |
/// | `weapon` | `0EE9h` | 2 | `flame` | `0EEBh` | 1 |
/// | `image` | `0EFDh` | 2 | `hit` | `0F00h` | 1 |
/// | `invincible` | `0F01h` | 1 | `dying` | `0F03h` | 1 |
/// | `blit_y` | `0F05h` | 2 | `blit_x` | `0F07h` | 2 |
/// | `row` | `0F09h` | 2 | `col` | `0F0Bh` | 2 |
struct PlayerOffsets {
    hp: u16,
    charge: u16,
    pod_first: u16,
    pod_second: u16,
    pod_facing: u16,
    pod_image: u16,
    pod_anim: u16,
    pod_col: u16,
    pod_row: u16,
    pod_blit_x: u16,
    pod_blit_y: u16,
    input_bits: u16,
    fired: u16,
    beam_y: u16,
    weapon: u16,
    flame: u16,
    image: u16,
    hit: u16,
    invincible: u16,
    dying: u16,
    blit_y: u16,
    blit_x: u16,
    row: u16,
    col: u16,
}

const PLAYER_OFFSETS: [PlayerOffsets; PLAYER_COUNT] = [
    PlayerOffsets {
        hp: a(0x1CFDE),
        charge: a(0x1DA8B),
        pod_first: a(0x1DA8D),
        pod_second: a(0x1DA8E),
        pod_facing: a(0x1DA8F),
        pod_image: a(0x1DA94),
        pod_anim: a(0x1DA96),
        pod_col: a(0x1DA98),
        pod_row: a(0x1DA9A),
        pod_blit_x: a(0x1DAA0),
        pod_blit_y: a(0x1DAA2),
        input_bits: a(0x1DAA4),
        fired: a(0x1DAA5),
        beam_y: a(0x1DAA6),
        weapon: a(0x1DAA9),
        flame: a(0x1DAAB),
        image: a(0x1DABD),
        hit: a(0x1DAC0),
        invincible: a(0x1DAC1),
        dying: a(0x1DAC3),
        blit_y: a(0x1DAC5),
        blit_x: a(0x1DAC7),
        row: a(0x1DAC9),
        col: a(0x1DACB),
    },
    PlayerOffsets {
        hp: a(0x1CFE0),
        charge: a(0x1DAED),
        pod_first: a(0x1DAEF),
        pod_second: a(0x1DAF0),
        pod_facing: a(0x1DAF1),
        pod_image: a(0x1DAF6),
        pod_anim: a(0x1DAF8),
        pod_col: a(0x1DAFA),
        pod_row: a(0x1DAFC),
        pod_blit_x: a(0x1DB02),
        pod_blit_y: a(0x1DB04),
        input_bits: a(0x1DB06),
        fired: a(0x1DB07),
        beam_y: a(0x1DB08),
        weapon: a(0x1DB0B),
        flame: a(0x1DB0D),
        image: a(0x1DB17),
        hit: a(0x1DB1A),
        invincible: a(0x1DB1B),
        dying: a(0x1DB1D),
        blit_y: a(0x1DB1F),
        blit_x: a(0x1DB21),
        row: a(0x1DB37),
        col: a(0x1DB39),
    },
];

impl PlayerOffsets {
    fn read(&self, ds: &Dseg) -> Player {
        Player {
            hp: ds.r16(self.hp),
            charge: ds.r16(self.charge),
            pods: Pods {
                first: ds.r8(self.pod_first),
                second: ds.r8(self.pod_second),
                facing: ds.r16(self.pod_facing),
                image: ds.r16(self.pod_image),
                anim: ds.r16(self.pod_anim),
                col: ds.r16(self.pod_col),
                row: ds.r16(self.pod_row),
                blit_x: ds.r16(self.pod_blit_x),
                blit_y: ds.r16(self.pod_blit_y),
            },
            input_bits: ds.r8(self.input_bits),
            fired: ds.r8(self.fired),
            beam_y: ds.r16(self.beam_y),
            weapon: ds.r16(self.weapon),
            flame: ds.r8(self.flame),
            image: ds.r16(self.image),
            hit: ds.r8(self.hit),
            invincible: ds.r8(self.invincible),
            dying: ds.r8(self.dying),
            blit_y: ds.r16(self.blit_y),
            blit_x: ds.r16(self.blit_x),
            row: ds.r16(self.row),
            col: ds.r16(self.col),
        }
    }

    fn write(&self, ds: &mut Dseg, p: &Player) {
        ds.w16(self.hp, p.hp);
        ds.w16(self.charge, p.charge);
        ds.w8(self.pod_first, p.pods.first);
        ds.w8(self.pod_second, p.pods.second);
        ds.w16(self.pod_facing, p.pods.facing);
        ds.w16(self.pod_image, p.pods.image);
        ds.w16(self.pod_anim, p.pods.anim);
        ds.w16(self.pod_col, p.pods.col);
        ds.w16(self.pod_row, p.pods.row);
        ds.w16(self.pod_blit_x, p.pods.blit_x);
        ds.w16(self.pod_blit_y, p.pods.blit_y);
        ds.w8(self.input_bits, p.input_bits);
        ds.w8(self.fired, p.fired);
        ds.w16(self.beam_y, p.beam_y);
        ds.w16(self.weapon, p.weapon);
        ds.w8(self.flame, p.flame);
        ds.w16(self.image, p.image);
        ds.w8(self.hit, p.hit);
        ds.w8(self.invincible, p.invincible);
        ds.w8(self.dying, p.dying);
        ds.w16(self.blit_y, p.blit_y);
        ds.w16(self.blit_x, p.blit_x);
        ds.w16(self.row, p.row);
        ds.w16(self.col, p.col);
    }
}

impl Mirror for Players {
    fn load(&mut self, ds: &Dseg) {
        self.alive = ds.r8(PLAYERS_ALIVE);
        for (ship, offsets) in self.ship.iter_mut().zip(&PLAYER_OFFSETS) {
            *ship = offsets.read(ds);
        }
    }

    fn store(&self, ds: &mut Dseg) {
        ds.w8(PLAYERS_ALIVE, self.alive);
        for (ship, offsets) in self.ship.iter().zip(&PLAYER_OFFSETS) {
            offsets.write(ds, ship);
        }
    }
}

impl Players {
    /// 從一份資料段讀出玩家狀態，給只有資料段複本的呼叫端用（`Game::player_status`、`Game::hud_status`）。
    pub(crate) fn from_dseg(ds: &Dseg) -> Players {
        let mut players = Players::default();
        players.load(ds);
        players
    }
}

// ---- 關卡流程 ----

/// 關卡字母 `byte_1CFDB`、過關旗標 `byte_1CFAD`、連爆輪數 `byte_1CFAE`、完整性記號 `byte_1CFAA`（皆 1 位元組）。
const FLOW_STAGE: u16 = a(0x1CFDB);
const FLOW_CLEAR: u16 = a(0x1CFAD);
const FLOW_BURSTS: u16 = a(0x1CFAE);
const FLOW_INTEGRITY_MARK: u16 = a(0x1CFAA);
/// `word_1B02`。
const FLOW_TAMPER_COUNTDOWN: u16 = 0x1B02;
/// 計時中斷的五個字組：`273B7`、`273B9`、`2739B`、`1D3A8`、`1D3AE`。
const CLOCK_TOTAL: u16 = a(0x273B7);
const CLOCK_SINCE_WAIT: u16 = a(0x273B9);
const CLOCK_SINCE_SOUND_TOGGLE: u16 = a(0x2739B);
const CLOCK_CYCLE_A: u16 = a(0x1D3A8);
const CLOCK_CYCLE_B: u16 = a(0x1D3AE);
/// 輸入：最後的掃描碼 `byte_2742C`、搖桿旗標 `byte_1DAAC`。
const CONTROLS_LAST_SCAN: u16 = a(0x2742C);
const CONTROLS_JOYMOUSE: u16 = a(0x1DAAC);
/// 開火旗標，玩家 1 與玩家 2：新按 `273A7`、`273A8`，事件 `273A9`、`273AB`，轉向 `273AA`、`273AC`（皆 1 位元組）。
const FIRE_NEW_PRESS: [u16; 2] = [a(0x273A7), a(0x273A8)];
const FIRE_EVENT: [u16; 2] = [a(0x273A9), a(0x273AB)];
const FIRE_TURN: [u16; 2] = [a(0x273AA), a(0x273AC)];
/// 秘技（皆字組）：解鎖 `2739D`，三個按鍵計數 `2739F`、`273A1`、`273A3`，無敵 `273A5`。
const CHEAT_UNLOCKED: u16 = a(0x2739D);
const CHEAT_PRESSES: [u16; 3] = [a(0x2739F), a(0x273A1), a(0x273A3)];
const CHEAT_INVINCIBLE: u16 = a(0x273A5);

impl Mirror for Flow {
    fn load(&mut self, ds: &Dseg) {
        self.stage = ds.r8(FLOW_STAGE);
        self.clear = ds.r8(FLOW_CLEAR);
        self.bursts = ds.r8(FLOW_BURSTS);
        self.integrity_mark = ds.r8(FLOW_INTEGRITY_MARK);
        self.tamper_countdown = ds.r16(FLOW_TAMPER_COUNTDOWN);
        self.clock = Clock {
            total: ds.r16(CLOCK_TOTAL),
            since_wait: ds.r16(CLOCK_SINCE_WAIT),
            since_sound_toggle: ds.r16(CLOCK_SINCE_SOUND_TOGGLE),
            cycle_a: ds.r16(CLOCK_CYCLE_A),
            cycle_b: ds.r16(CLOCK_CYCLE_B),
        };
        self.controls = Controls {
            last_scan: ds.r8(CONTROLS_LAST_SCAN),
            fire: FireFlags {
                new_press: FIRE_NEW_PRESS.map(|at| ds.r8(at)),
                event: FIRE_EVENT.map(|at| ds.r8(at)),
                turn: FIRE_TURN.map(|at| ds.r8(at)),
            },
            cheat: Cheat {
                unlocked: ds.r16(CHEAT_UNLOCKED),
                presses: CHEAT_PRESSES.map(|at| ds.r16(at)),
                invincible: ds.r16(CHEAT_INVINCIBLE),
            },
            joymouse: ds.r8(CONTROLS_JOYMOUSE),
        };
    }

    fn store(&self, ds: &mut Dseg) {
        ds.w8(FLOW_STAGE, self.stage);
        ds.w8(FLOW_CLEAR, self.clear);
        ds.w8(FLOW_BURSTS, self.bursts);
        ds.w8(FLOW_INTEGRITY_MARK, self.integrity_mark);
        ds.w16(FLOW_TAMPER_COUNTDOWN, self.tamper_countdown);
        ds.w16(CLOCK_TOTAL, self.clock.total);
        ds.w16(CLOCK_SINCE_WAIT, self.clock.since_wait);
        ds.w16(CLOCK_SINCE_SOUND_TOGGLE, self.clock.since_sound_toggle);
        ds.w16(CLOCK_CYCLE_A, self.clock.cycle_a);
        ds.w16(CLOCK_CYCLE_B, self.clock.cycle_b);
        let c = &self.controls;
        ds.w8(CONTROLS_LAST_SCAN, c.last_scan);
        for who in 0..2 {
            ds.w8(FIRE_NEW_PRESS[who], c.fire.new_press[who]);
            ds.w8(FIRE_EVENT[who], c.fire.event[who]);
            ds.w8(FIRE_TURN[who], c.fire.turn[who]);
        }
        ds.w16(CHEAT_UNLOCKED, c.cheat.unlocked);
        for (at, v) in CHEAT_PRESSES.into_iter().zip(c.cheat.presses) {
            ds.w16(at, v);
        }
        ds.w16(CHEAT_INVINCIBLE, c.cheat.invincible);
        ds.w8(CONTROLS_JOYMOUSE, c.joymouse);
    }
}

// ---- 各關背景 ----

/// 第一關背景（皆字組）：下移 `1E6B8`、天空下移 `1E6BA`、波浪相位 `1E6BC`、岩相位 `1E6BE`。
const S1_DROP: u16 = a(0x1E6B8);
const S1_SKY_DROP: u16 = a(0x1E6BA);
const S1_WAVE_PHASE: u16 = a(0x1E6BC);
const S1_ROCK_PHASE: u16 = a(0x1E6BE);
/// 第二關背景（皆字組）：火焰面底端 `1D7BE`、頂端 `1D7C0`、相位 `1D7C2`；近景條 A（相位 `1D7C4`、起點 `1D7C6`）、
/// B（`1D7C8`、`1D7CA`）、C（`1D7CC`、`1D7CE`）；柱 `1D7D0`。
const S2_FLAME_BOTTOM: u16 = a(0x1D7BE);
const S2_FLAME_TOP: u16 = a(0x1D7C0);
const S2_FLAME_PHASE: u16 = a(0x1D7C2);
const S2_STRIP_A: u16 = a(0x1D7C4);
const S2_STRIP_B: u16 = a(0x1D7C8);
const S2_STRIP_C: u16 = a(0x1D7CC);
const S2_PILLAR: u16 = a(0x1D7D0);
/// 第二關淡出工作表：`stage2` 調色盤的複本，45 位元組。
const S2_FADE: u16 = 0x0CA7;
/// 第三關地圖位置（皆字組）：視窗 `25BC4`、地圖左緣 `25BC6`、地圖第一列 `25BC8`。
/// 之後的 `25BCA` 是 `draw_initial_map` 與 `scroll_map_second_page` 的工作變數（`DS_EXEMPT`）。
const S3_WINDOW: u16 = a(0x25BC4);
const S3_LEFT_COL: u16 = a(0x25BC6);
const S3_TOP_ROW: u16 = a(0x25BC8);
/// 第四關通道（皆字組）：偏移 `1D91D`、下壁相位 `1D91F`、上壁相位 `1D921`、腳本位置 `1D9A2`。
const S4_OFFSET: u16 = a(0x1D91D);
const S4_FLOOR_PHASE: u16 = a(0x1D91F);
const S4_CEILING_PHASE: u16 = a(0x1D921);
const S4_SCRIPT_POS: u16 = a(0x1D9A2);
/// 頭目合成圖三層，每層三個字組：(y, x, 腳本位置)。第 0 層 `1DB5F`、`1DB61`、`1DB6D`；第 1 層 `1DB63`、`1DB65`、`1DB6F`；
/// 第 2 層 `1DB67`、`1DB69`、`1DB71`。
const BOSS_LAYER_WORDS: [[u16; 3]; 3] = [
    [a(0x1DB5F), a(0x1DB61), a(0x1DB6D)],
    [a(0x1DB63), a(0x1DB65), a(0x1DB6F)],
    [a(0x1DB67), a(0x1DB69), a(0x1DB71)],
];

impl Mirror for Scenes {
    fn load(&mut self, ds: &Dseg) {
        self.stage1 = Stage1Backdrop {
            drop: ds.r16(S1_DROP),
            sky_drop: ds.r16(S1_SKY_DROP),
            wave_phase: ds.r16(S1_WAVE_PHASE),
            rock_phase: ds.r16(S1_ROCK_PHASE),
        };
        let strip = |at: u16| Strip { phase: ds.r16(at), start: ds.r16(at + 2) };
        self.stage2 = Stage2Backdrop {
            flame_bottom: ds.r16(S2_FLAME_BOTTOM),
            flame_top: ds.r16(S2_FLAME_TOP),
            flame_phase: ds.r16(S2_FLAME_PHASE),
            strip_a: strip(S2_STRIP_A),
            strip_b: strip(S2_STRIP_B),
            strip_c: strip(S2_STRIP_C),
            pillar: ds.r16(S2_PILLAR),
            fade: std::array::from_fn(|k| ds.r8(S2_FADE + k as u16)),
        };
        self.map = MapScroll { window: ds.r16(S3_WINDOW), left_col: ds.r16(S3_LEFT_COL), top_row: ds.r16(S3_TOP_ROW) };
        self.tunnel = Tunnel {
            offset: ds.r16(S4_OFFSET),
            floor_phase: ds.r16(S4_FLOOR_PHASE),
            ceiling_phase: ds.r16(S4_CEILING_PHASE),
            script_pos: ds.r16(S4_SCRIPT_POS),
        };
        self.boss = BossLayers {
            layers: BOSS_LAYER_WORDS.map(|[y, x, script]| BossLayer { y: ds.r16(y), x: ds.r16(x), script: ds.r16(script) }),
        };
    }

    fn store(&self, ds: &mut Dseg) {
        let s1 = &self.stage1;
        ds.w16(S1_DROP, s1.drop);
        ds.w16(S1_SKY_DROP, s1.sky_drop);
        ds.w16(S1_WAVE_PHASE, s1.wave_phase);
        ds.w16(S1_ROCK_PHASE, s1.rock_phase);
        let s2 = &self.stage2;
        ds.w16(S2_FLAME_BOTTOM, s2.flame_bottom);
        ds.w16(S2_FLAME_TOP, s2.flame_top);
        ds.w16(S2_FLAME_PHASE, s2.flame_phase);
        for (at, strip) in [(S2_STRIP_A, s2.strip_a), (S2_STRIP_B, s2.strip_b), (S2_STRIP_C, s2.strip_c)] {
            ds.w16(at, strip.phase);
            ds.w16(at + 2, strip.start);
        }
        ds.w16(S2_PILLAR, s2.pillar);
        for (k, &level) in s2.fade.iter().enumerate() {
            ds.w8(S2_FADE + k as u16, level);
        }
        ds.w16(S3_WINDOW, self.map.window);
        ds.w16(S3_LEFT_COL, self.map.left_col);
        ds.w16(S3_TOP_ROW, self.map.top_row);
        ds.w16(S4_OFFSET, self.tunnel.offset);
        ds.w16(S4_FLOOR_PHASE, self.tunnel.floor_phase);
        ds.w16(S4_CEILING_PHASE, self.tunnel.ceiling_phase);
        ds.w16(S4_SCRIPT_POS, self.tunnel.script_pos);
        for (words, layer) in BOSS_LAYER_WORDS.iter().zip(&self.boss.layers) {
            ds.w16(words[0], layer.y);
            ds.w16(words[1], layer.x);
            ds.w16(words[2], layer.script);
        }
    }
}

/// 場景的調色盤（唯讀表格）。每張是 17 位元組的 EGA 屬性表接 48 位元組的 DAC 表，位址指向 DAC 表。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Palette {
    /// 載入畫面與第一關進場（`sub_11112`），DAC 表與標題的 `title_d` 共用，屬性表在 `56Eh`。
    Loader,
    Stage1,
    Stage1Boss,
    Stage2,
    /// 第二關中頭目（`stage2_b`）。
    Stage2Mid,
    /// 第二關頭目（`stage2_c`）。
    Stage2Boss,
    /// 第二關淡出工作表所在的位置（`0CA7h`）。工作表的內容在 [`Stage2Backdrop::fade`]，只有最後 3 位元組和屬性表讀這裡。
    Stage2Fade,
    Stage3,
    Stage4,
    /// 標題動畫的手臂、眼部特寫與製作名單。DAC 表與 `Loader` 共用。
    TitleD,
    /// 標題動畫的太空船正面與 `RS-S` 字樣。
    TitleE,
    /// 選單。
    Menu,
    /// 標題動畫的地球與太空船。
    Beg1A,
    /// 太空船飛過時的灰階閃光。
    Beg1B,
    /// 第一關到第四關的過場圖（J1 到 J4）。
    J1,
    J2,
    J3,
    J4,
    /// 片尾的地景與人物行星。
    EndingA,
    EndingB,
}

impl Palette {
    pub(crate) const fn table_offset(self) -> u16 {
        match self {
            Palette::Loader => 0x05A9,
            Palette::Stage1 => 0x04FD,
            Palette::Stage1Boss => 0x053E,
            Palette::Stage2 => 0x0C25,
            Palette::Stage2Mid => 0x0C66,
            Palette::Stage2Boss => 0x0CE8,
            Palette::Stage2Fade => 0x0CA7,
            Palette::Stage3 => 0x0D29,
            Palette::Stage4 => 0x0D82,
            Palette::TitleD => 0x05A9,
            Palette::TitleE => 0x05EA,
            Palette::Menu => 0x062B,
            Palette::Beg1A => 0x066C,
            Palette::Beg1B => 0x06AD,
            Palette::J1 => 0x06EE,
            Palette::J2 => 0x072F,
            Palette::J3 => 0x0770,
            Palette::J4 => 0x07B1,
            Palette::EndingA => 0x0E54,
            Palette::EndingB => 0x0E95,
        }
    }

    /// EGA 屬性表：在 DAC 表之前 11h 位元組，只有 `Loader` 例外。
    pub(crate) const fn attr_offset(self) -> u16 {
        match self {
            Palette::Loader => 0x056E,
            other => other.table_offset() - 0x11,
        }
    }
}

/// 頭目合成圖的捲動腳本（唯讀表格）。腳本是八向指令的位元組串，結束碼見 `boss_scroll.rs`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BossScript {
    /// 第一關頭目，714 步，起點 x = −20、y = 200。
    Stage1,
    /// 第二關中頭目第一層，148 步。
    Stage2MidA,
    /// 第二關中頭目第二層，189 步。
    Stage2MidB,
    /// 第二關頭目，704 步，以 1Fh 結束。
    Stage2Boss,
    /// 第三關頭目，758 步。
    Stage3,
}

impl BossScript {
    /// 腳本在資料段的位移。
    pub(crate) const fn start(self) -> u16 {
        match self {
            BossScript::Stage1 => 0x0FB3,
            BossScript::Stage2MidA => 0x127E,
            BossScript::Stage2MidB => 0x1313,
            BossScript::Stage2Boss => 0x13D1,
            BossScript::Stage3 => 0x1692,
        }
    }
}

/// 第四關通道背景的腳本（唯讀表格）：76 位元組，1 讓偏移減 10、2 加 10、0Fh 結尾。
const TUNNEL_SCRIPT: u16 = 0x0DE4;

/// 各關入口向 DOS 配置十塊圖庫緩衝區再立刻釋放（INT 21h AH=48h、49h）的對象。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StageBuffers {
    Stage2,
    Stage3,
    Stage4,
}

/// 錯誤訊息的兩個三字元前綴（`byte_1CEF6` 起與 `byte_1CF08` 起）。`sub_10210`、`sub_1026C` 每塊把第三個字元加 1。
const DOS_MESSAGE_A: u16 = a(0x1CEF6);
const DOS_MESSAGE_B: u16 = a(0x1CF08);
/// 配置結果：十個段值（`word_1CF5C` 起）。
const DOS_SEGMENTS: u16 = a(0x1CF5C);
const DOS_BLOCKS: u16 = 10;
/// 訊息第三個字元的初值。
const DOS_COUNTER_START: u8 = 0x30;

impl StageBuffers {
    /// 配置前寫進訊息前綴的前兩個字元。
    const fn alloc_prefix(self) -> (u8, u8) {
        match self {
            StageBuffers::Stage2 | StageBuffers::Stage3 => (0x41, 0x2B),
            StageBuffers::Stage4 => (0x41, 0x34),
        }
    }

    /// 釋放前寫進訊息前綴的前兩個字元；第四關不寫。
    const fn free_prefix(self) -> Option<(u8, u8)> {
        match self {
            StageBuffers::Stage2 | StageBuffers::Stage3 => Some((0x42, 0x2D)),
            StageBuffers::Stage4 => None,
        }
    }

    /// 十塊的段落數（16 位元組為一段落）。第三關的配置函式不記錄段值，只加計數。
    const fn paragraphs(self) -> Option<[u16; DOS_BLOCKS as usize]> {
        match self {
            StageBuffers::Stage2 => Some([0x600, 0x600, 0x600, 0x600, 0x600, 0x10, 0x10, 0x10, 0x10, 1]),
            StageBuffers::Stage3 => None,
            StageBuffers::Stage4 => Some([0x600, 0x600, 0x600, 0x600, 0x600, 1, 1, 1, 1, 1]),
        }
    }
}

impl Core {
    /// DOS 配置十塊的副作用（原版 `sub_138D6`、`sub_13BE0`、`sub_140A1`）：DOS 不在模擬範圍，只留下資料段的簿記，
    /// 它們全在 `A0_EXEMPT`，沒有邏輯讀。
    ///
    /// 模型：配置結果用第一個段值現有的值當基底（上一關釋放後最低的空洞起點，每關相同），之後每塊接在前一塊後面，
    /// 中間隔一個 MCB 段落：`seg[k] = seg[k-1] + 段落數[k-1] + 1`。第一關配置的傾印 `4989h, 4E8Ah, …` 符合這個公式。
    /// 假設不成立時對拍會在這 10 個字組先出現差異。
    pub(crate) fn dos_allocate_buffers(&mut self, which: StageBuffers) {
        let (first, second) = which.alloc_prefix();
        self.write_dos_message(first, second);
        if let Some(blocks) = which.paragraphs() {
            let mut seg = self.ds.r16(DOS_SEGMENTS);
            for (k, paragraphs) in blocks.into_iter().enumerate() {
                self.ds.w16(DOS_SEGMENTS + 2 * k as u16, seg);
                seg = seg.wrapping_add(paragraphs).wrapping_add(1);
            }
        }
        self.count_dos_calls();
    }

    /// 依相反順序釋放十塊（原版 `sub_139CD`、`sub_13CCD`、`sub_1418E`）。段值仍留在配置結果的十個字組。
    pub(crate) fn dos_free_buffers(&mut self, which: StageBuffers) {
        if let Some((first, second)) = which.free_prefix() {
            self.write_dos_message(first, second);
        }
        self.count_dos_calls();
    }

    fn write_dos_message(&mut self, first: u8, second: u8) {
        for at in [DOS_MESSAGE_A, DOS_MESSAGE_B] {
            self.ds.w8(at, first);
            self.ds.w8(at + 1, second);
            self.ds.w8(at + 2, DOS_COUNTER_START);
        }
    }

    /// 十次 DOS 呼叫，每次把兩個訊息前綴的第三個字元加 1。
    fn count_dos_calls(&mut self) {
        for at in [DOS_MESSAGE_A + 2, DOS_MESSAGE_B + 2] {
            let v = self.ds.r8(at).wrapping_add(DOS_BLOCKS as u8);
            self.ds.w8(at, v);
        }
    }

    /// 唯讀表格連續 `N` 個位元組。
    pub(crate) fn table_bytes<const N: usize>(&self, ptr: u16) -> [u8; N] {
        std::array::from_fn(|k| self.ds.r8(ptr.wrapping_add(k as u16)))
    }

    /// 第四關通道腳本第 `pos` 個位元組。
    pub(crate) fn tunnel_script_byte(&self, pos: u16) -> u8 {
        self.ds.r8(TUNNEL_SCRIPT.wrapping_add(pos))
    }
}

/// 玩家的三張形狀表（唯讀表格）。每張是 `寬, 高, 寬 × 高 位元組`，表格階段改成解析 `S.EXE` 得到的結構。
/// 兩位玩家各有一份，內容相同，位址不同。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerShape {
    /// 本體，5 × 1（玩家 1 在 `0F0Dh`，玩家 2 在 `0F7Bh`）。
    Body,
    /// 砲座，2 × 2（`0F16h`、`0F85h`）。
    Pod,
    /// 尾焰的髒格，3 × 3（`0F1Eh`、`0F8Dh`）。
    Flame,
}

impl PlayerShape {
    pub(crate) const fn table_offset(self, player: usize) -> u16 {
        match (self, player) {
            (PlayerShape::Body, 0) => 0x0F0D,
            (PlayerShape::Pod, 0) => 0x0F16,
            (PlayerShape::Flame, 0) => 0x0F1E,
            (PlayerShape::Body, _) => 0x0F7B,
            (PlayerShape::Pod, _) => 0x0F85,
            (PlayerShape::Flame, _) => 0x0F8D,
        }
    }
}

// ---- 標題 ----

/// 選單與示範輪播的四個字組，連續排列（`580h` 起）。
/// 之後 `588h` 至 `597h` 是開場與過場字樣的位置暫存（`DS_EXEMPT`），不在這裡。
///
/// | 位移 | 欄位 | 原版 |
/// |---|---|---|
/// | `580h` | `Menu::row` | `word_1D140`：選單選中的列 |
/// | `582h` | `Menu::level` | `word_1D142`：難度 |
/// | `584h` | `Menu::role` | `word_1D144`：角色 |
/// | `586h` | `Title::demo_round` | `word_1D146`：示範輪播 |
const TITLE_MENU_ROW: u16 = a(0x1D140);
pub(crate) const TITLE_LEVEL: u16 = a(0x1D142);
const TITLE_ROLE: u16 = a(0x1D144);
const TITLE_DEMO_ROUND: u16 = a(0x1D146);

impl Mirror for Title {
    fn load(&mut self, ds: &Dseg) {
        self.menu = Menu { row: ds.r16(TITLE_MENU_ROW), level: ds.r16(TITLE_LEVEL), role: ds.r16(TITLE_ROLE) };
        self.demo_round = ds.r16(TITLE_DEMO_ROUND);
    }

    fn store(&self, ds: &mut Dseg) {
        ds.w16(TITLE_MENU_ROW, self.menu.row);
        ds.w16(TITLE_LEVEL, self.menu.level);
        ds.w16(TITLE_ROLE, self.menu.role);
        ds.w16(TITLE_DEMO_ROUND, self.demo_round);
    }
}

/// 形狀表的寬與高。格值接在後面，要逐格讀時用 [`Core::stamp_shape`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Footprint {
    pub width: u16,
    pub height: u16,
}

// ---- 尚未搬走的共用資料段 ----
//
// 搬完的領域還要讀寫其他領域擁有的 `Dseg` 變數（唯讀表格、輸入、格網），一律經下面的具名存取函式。

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

impl Timetable {
    /// 時間表在資料段的位移。唯讀表格，表格階段改成解析 `S.EXE` 得到的結構。
    /// 原版每關一個只有 `mov si, 表; call sub_15D1C` 的包裝函式：`sub_17D82`、`sub_17D8E`、`sub_17D9A`、`sub_17DA6`、`sub_17DB2`。
    pub(crate) const fn table_offset(self) -> u16 {
        match self {
            Timetable::Stage1 => 0x41BE,
            Timetable::Stage2 => 0x54FE,
            Timetable::Stage3 => 0x68E0,
            Timetable::Stage4 => 0x7922,
            Timetable::Ending => 0x8D1C,
        }
    }
}

/// 敵人開火的固定亂數表（`42Ch`）：75 個字組，值域 0 至 15。
const FIRE_ROLLS: u16 = 0x042C;

impl Core {
    /// 讀一筆敵彈樣板（15 個字組）。
    pub(crate) fn bullet_template(&self, template: Template) -> Bullet {
        Bullet::read(&self.ds, template.table_offset())
    }

    /// 讀一筆敵人樣板（24 個字組）。樣板本身 23 個字組，相鄰樣板緊鄰排列，所以第 24 個字組（`spill`）
    /// 是下一個樣板的 `hp`（文件第 2.1 節）。
    pub(crate) fn enemy_template(&self, at: u16) -> Enemy {
        Enemy::read(&self.ds, at)
    }

    /// 景物群組表第 `row` 列：(幀, 景物記錄位址)。幀為 0 是表尾。
    pub(crate) fn scenery_group(&self, row: u16) -> (u16, u16) {
        let at = SC_GROUPS.wrapping_add(row.wrapping_mul(SC_GROUP_BYTES));
        (self.table_word(at), self.table_word(at.wrapping_add(2)))
    }

    /// 讀一筆景物記錄（4 個字組）。
    pub(crate) fn scenery_record(&self, at: u16) -> Prop {
        Prop::from_words(std::array::from_fn(|k| self.table_word(at.wrapping_add(2 * k as u16))))
    }

    /// 開火亂數表第 `index` 項（`index` < 75）。
    pub(crate) fn fire_roll_at(&self, index: u16) -> u16 {
        self.table_word(FIRE_ROLLS.wrapping_add(index.wrapping_mul(2)))
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

/// `word_26872`：目前繪製頁的偏移，0 或 8000h。
const DRAW_PAGE: u16 = a(0x26872);

impl Core {
    /// 目前繪製頁的偏移：奇偶幀交替，0 或 8000h。
    pub(crate) fn draw_page(&self) -> u16 {
        self.ds.r16(DRAW_PAGE)
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

/// `word_1CFE2`：關卡的幀計數。標題與片尾（片尾自己數幀）仍直接讀寫資料段，所以留在資料段。
const STAGE_FRAME: u16 = a(0x1CFE2);
/// `byte_1CFAD`：過關旗標（`Flow::clear`）的位移，只給 [`stage_cleared`] 讀資料段複本用。
const STAGE_CLEAR_FLAG: u16 = a(0x1CFAD);
/// `byte_1CFDC`：展示旗標，標題寫（`Y` 是示範，`N` 是正式遊戲），關卡與玩家讀。
const DEMO_FLAG: u16 = a(0x1CFDC);
/// 展示錄影在資料段的起點，`DM1.LCK` 的 3000 位元組（載入領域寫入）。
const DEMO_RECORD: u16 = 0xAAE2;
/// `byte_1CFAB`：標題迴圈每圈寫 `Y`。`sub_10CD0`（`10D17h`）見到不是 `N` 就換 `B.MIX` 開音樂。
const STARTED_FROM_TITLE: u16 = a(0x1CFAB);
/// `byte_2739A`：音效開關鍵（INT 9 的 `sub_1A891`）是否生效，`N` 不生效。標題期間 `N`，進關卡改 `Y`，離開改 `N`。
const SOUND_KEY: u16 = a(0x2739A);
/// `byte_2684E`：音效開關，位元 0 為 1 是靜音。INT 9 的 S 鍵切換，`sub_187C3` 讀。
const SOUND_MUTE: u16 = a(0x2684E);
/// `byte_2684F`：喇叭音效旗標，`sub_18672` 依位元選參數。
const SPEAKER_FLAGS: u16 = a(0x2684F);
/// `byte_1D3A2`：密碼檢查通過旗標，映像是 `N`，通過時寫 `Y`。第二關 `130AFh` 不是 `Y` 就停在無限迴圈。
const PASSWORD_ACCEPTED: u16 = a(0x1D3A2);
/// `word_1D3B0`：密碼檢查未通過的旗標。破解組補丁讓檢查恆通過，重製裡沒有人寫它，恆為 0。
const PASSWORD_REJECTED: u16 = a(0x1D3B0);
/// `byte_1CBCE`：音樂可用（`Y`）。
#[cfg(test)]
const MUSIC_ENABLED: u16 = a(0x1CBCE);
/// 完整性檢查累加的資料段範圍：`7F4h` 起 400 個字組（`sub_12F95`），初值 64h。
const INTEGRITY_TABLE: u16 = 0x07F4;
const INTEGRITY_WORDS: u16 = 0x190;
const INTEGRITY_SEED: u16 = 0x64;

/// 測試用：關卡的整合測試從資料段複本（`Game::ds`）讀狀態時用的位移。
#[cfg(test)]
pub(crate) mod probe {
    use crate::dseg::a;

    pub(crate) const STAGE_FRAME: u16 = a(0x1CFE2);
    pub(crate) const MAP_LEFT_COL: u16 = a(0x25BC6);
    pub(crate) const PLAYER_1_HP: u16 = a(0x1CFDE);
    pub(crate) const PLAYER_2_HP: u16 = a(0x1CFE0);
    pub(crate) const PLAYERS_ALIVE: u16 = a(0x1CFDD);
}

/// 按鍵狀態表，`2748Bh + 掃描碼`，每鍵一個位元組：1 按住，2 剛放開。INT 9 維護，標題與片尾直接讀寫。
const KEY_TABLE: u16 = a(0x2748B);
const KEY_HELD: u8 = 1;
const KEY_RELEASED: u8 = 2;
/// 掃描碼轉 ASCII 的對照表，`0A86Dh` 起（唯讀）。表只到掃描碼 5Dh，之後讀到的是按鍵狀態表的前面幾項。
const ASCII_TABLE: u16 = 0xA86D;
/// `byte_27422`：最近一次按下的鍵（ASCII），0 表示沒有。標題的各段動畫每步看一次它是不是 0。
const LAST_KEY: u16 = a(0x27422);

/// 標題、片尾與關卡讀的鍵。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Up,
    Down,
    Left,
    Right,
    /// 標題與片尾的選單用，關卡不讀。
    #[allow(dead_code)]
    Enter,
    Space,
    Q,
    R,
    Esc,
    Digit1,
    Digit2,
    Digit3,
    W,
    Ctrl,
    S,
    D,
    J,
    K,
    LeftShift,
    Z,
    C,
    Slash,
    RightShift,
    KeypadStar,
    Alt,
    Keypad5,
}

impl Key {
    /// set 1 掃描碼，也是按鍵表的索引。
    pub(crate) const fn scancode(self) -> u16 {
        match self {
            Key::Up => 0x48,
            Key::Down => 0x50,
            Key::Left => 0x4B,
            Key::Right => 0x4D,
            Key::Enter => 0x1C,
            Key::Space => 0x39,
            Key::Q => 0x10,
            Key::R => 0x13,
            Key::Esc => 0x01,
            Key::Digit1 => 0x02,
            Key::Digit2 => 0x03,
            Key::Digit3 => 0x04,
            Key::W => 0x11,
            Key::Ctrl => 0x1D,
            Key::S => 0x1F,
            Key::D => 0x20,
            Key::J => 0x24,
            Key::K => 0x25,
            Key::LeftShift => 0x2A,
            Key::Z => 0x2C,
            Key::C => 0x2E,
            Key::Slash => 0x35,
            Key::RightShift => 0x36,
            Key::KeypadStar => 0x37,
            Key::Alt => 0x38,
            Key::Keypad5 => 0x4C,
        }
    }
}

/// 玩家的開火旗標，INT 9 的 `sub_1A9B5` 依按鍵事件設成 `Y`，玩家邏輯處理後清成 `N`。
/// 旗標本身在 [`crate::flow::FireFlags`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FireFlag {
    /// 開火鍵放開過，下一次按下算新按（玩家 1 `273A7h`，玩家 2 `273A8h`）。
    NewPress,
    /// 本幀有開火鍵事件，含按住時的自動重複（`273A9h`、`273ABh`）。
    Event,
    /// 轉向鍵被按下（`273AAh`、`273ACh`）。
    Turn,
}

/// 過關旗標是不是已設。`Game::player_status` 只有資料段複本，所以是自由函式。
pub(crate) fn stage_cleared(ds: &Dseg) -> bool {
    ds.r8(STAGE_CLEAR_FLAG) == STAGE_CLEARED
}

/// 是不是在播放展示錄影。
pub(crate) fn demo_playing(ds: &Dseg) -> bool {
    ds.r8(DEMO_FLAG) == b'Y'
}
/// 難度加成：`word_1E6B2` 加在敵人的耐久上，`word_1E6B4` 加在開火亂數上（文件第 9.1 節）。
const DIFFICULTY_HP: u16 = a(0x1E6B2);
const DIFFICULTY_FIRE: u16 = a(0x1E6B4);
/// `word_2685C`：繪製的暫存區選擇，0 是 A，8000h 是 B。
const SCRATCH_SELECT: u16 = a(0x2685C);
/// 暫存區 A 的選擇值。
pub(crate) const SCRATCH_A: u16 = 0;
/// 暫存區 B 的選擇值。
pub(crate) const SCRATCH_B: u16 = 0x8000;

impl Core {
    /// 目前是不是第三關（有 G4 地形格）。頭目段關卡字母改成小寫 `c`，不算。
    pub(crate) fn on_terrain_stage(&self) -> bool {
        self.flow.stage == STAGE_3
    }

    /// 關卡的幀計數（原版 `word_1CFE2`）。
    pub(crate) fn stage_frame(&self) -> u16 {
        self.ds.r16(STAGE_FRAME)
    }

    /// 幀計數歸零（原版 `word_1CFE2`）。進關卡與片尾重播用。
    pub(crate) fn reset_stage_frame(&mut self) {
        self.ds.w16(STAGE_FRAME, 0);
    }

    /// 幀計數加 1。
    pub(crate) fn advance_stage_frame(&mut self) {
        let frame = self.ds.r16(STAGE_FRAME).wrapping_add(1);
        self.ds.w16(STAGE_FRAME, frame);
    }

    /// 幀計數設成 `frame`：第一關釘在 288h、第三關釘在 474h、第四關進場設 1。
    pub(crate) fn set_stage_frame(&mut self, frame: u16) {
        self.ds.w16(STAGE_FRAME, frame);
    }

    /// 設過關旗標（原版 `byte_1CFAD` = `F`）。
    pub(crate) fn mark_stage_cleared(&mut self) {
        self.flow.clear = STAGE_CLEARED;
    }

    /// 過關旗標是不是已設（原版 `byte_1CFAD` = `F`）：主迴圈改播連爆，玩家邏輯與頭目背景停止。
    pub(crate) fn stage_cleared(&self) -> bool {
        self.flow.is_cleared()
    }

    /// 是不是第一關（原版 `byte_1CFDB` = `A`）。
    pub(crate) fn on_first_stage(&self) -> bool {
        self.flow.stage == STAGE_1
    }

    /// 是不是在播放展示錄影（原版 `byte_1CFDC` = `Y`）。
    pub(crate) fn demo_playing(&self) -> bool {
        demo_playing(&self.ds)
    }

    /// 示範模式的旗標（原版 `byte_1CFDC`）：`on` 寫 `Y`，否則寫 `N`。
    pub(crate) fn set_demo_mode(&mut self, on: bool) {
        self.ds.w8(DEMO_FLAG, if on { b'Y' } else { b'N' });
    }

    /// 標題迴圈來的（原版 `byte_1CFAB` 不是 `N`）：第一關要換 `B.MIX` 開音樂。
    pub(crate) fn started_from_title(&self) -> bool {
        self.ds.r8(STARTED_FROM_TITLE) != b'N'
    }

    /// 音效開關鍵是否生效（原版 `byte_2739A` 不是 `N`）。
    pub(crate) fn sound_key_enabled(&self) -> bool {
        self.ds.r8(SOUND_KEY) != b'N'
    }

    /// 音效開關鍵是否生效（原版 `byte_2739A`）：標題期間 `N`，進關卡 `Y`。
    pub(crate) fn set_sound_key_enabled(&mut self, on: bool) {
        self.ds.w8(SOUND_KEY, if on { b'Y' } else { b'N' });
    }

    /// 切換靜音（原版 `byte_2684E` 的位元 0，INT 9 的 `sub_1A891`）。
    pub(crate) fn toggle_sound_mute(&mut self) {
        let v = self.ds.r8(SOUND_MUTE) ^ 1;
        self.ds.w8(SOUND_MUTE, v);
    }

    /// 喇叭音效旗標（原版 `byte_2684F`），接著呼叫 `sub_18672` 依位元選參數。
    pub(crate) fn set_speaker_flags(&mut self, flags: u8) {
        self.ds.w8(SPEAKER_FLAGS, flags);
    }

    /// 密碼檢查通過了（原版 `byte_1D3A2` 是 `Y`）。
    pub(crate) fn password_accepted(&self) -> bool {
        self.ds.r8(PASSWORD_ACCEPTED) == b'Y'
    }

    /// 密碼檢查是不是沒通過（原版 `word_1D3B0` 非 0）。
    pub(crate) fn password_rejected(&self) -> bool {
        self.ds.r16(PASSWORD_REJECTED) != 0
    }

    /// 完整性檢查累加的和（原版 `sub_12F95`）：`7F4h` 起 400 個字組，初值 64h，16 位元環繞。
    pub(crate) fn integrity_table_sum(&self) -> u16 {
        (0..INTEGRITY_WORDS).fold(INTEGRITY_SEED, |sum, k| sum.wrapping_add(self.ds.r16(INTEGRITY_TABLE.wrapping_add(2 * k))))
    }

    /// 掃描碼對應的 ASCII（原版 INT 9 的 `xlat`，表在 `0A86Dh`）。表只有 5Eh 項，掃描碼 5Eh 以上
    /// 讀到的是緊接在表後的按鍵狀態表。
    pub(crate) fn ascii_of(&self, scan: u8) -> u8 {
        self.ds.r8(ASCII_TABLE.wrapping_add(u16::from(scan)))
    }

    /// 最近一次按下的鍵（ASCII），0 表示沒有。
    pub(crate) fn last_key(&self) -> u8 {
        self.ds.r8(LAST_KEY)
    }

    pub(crate) fn set_last_key(&mut self, ascii: u8) {
        self.ds.w8(LAST_KEY, ascii);
    }

    pub(crate) fn clear_last_key(&mut self) {
        self.ds.w8(LAST_KEY, 0);
    }

    /// 按鍵是不是按著（按鍵表的值為 1）。放開後 INT 9 寫 2，所以放開的鍵不算。
    pub(crate) fn key_down(&self, key: Key) -> bool {
        self.ds.r8(KEY_TABLE.wrapping_add(key.scancode())) == KEY_HELD
    }

    /// 按鍵剛放開（按鍵表的值為 2）。關卡的暫停與開火旗標用：暫停中 Esc 的狀態維持 2 直到下一次按下。
    pub(crate) fn key_released(&self, key: Key) -> bool {
        self.ds.r8(KEY_TABLE.wrapping_add(key.scancode())) == KEY_RELEASED
    }

    /// 清掉按鍵表裡這個鍵的狀態（寫 0）。
    pub(crate) fn release_key(&mut self, key: Key) {
        self.ds.w8(KEY_TABLE.wrapping_add(key.scancode()), 0);
    }

    /// 把這個鍵的狀態寫成 2（剛放開）。轉向鍵處理後用，暫停用。
    pub(crate) fn mark_key_released(&mut self, key: Key) {
        self.ds.w8(KEY_TABLE.wrapping_add(key.scancode()), KEY_RELEASED);
    }

    /// 按住就消耗掉（寫 0）並回傳 true：一次按鍵只處理一次。
    pub(crate) fn take_key_press(&mut self, key: Key) -> bool {
        let pressed = self.key_down(key);
        if pressed {
            self.release_key(key);
        }
        pressed
    }

    /// INT 9 收到按下碼：按鍵表寫 1。
    pub(crate) fn note_key_down(&mut self, scan: u8) {
        self.ds.w8(KEY_TABLE.wrapping_add(u16::from(scan)), KEY_HELD);
    }

    /// INT 9 收到放開碼：按鍵表寫 2。
    pub(crate) fn note_key_up(&mut self, scan: u8) {
        self.ds.w8(KEY_TABLE.wrapping_add(u16::from(scan)), KEY_RELEASED);
    }

    /// 無敵秘技等測試要看按鍵表原始值。
    #[cfg(test)]
    pub(crate) fn key_raw(&self, scan: u8) -> u8 {
        self.ds.r8(KEY_TABLE.wrapping_add(u16::from(scan)))
    }

    /// 是不是靜音。
    #[cfg(test)]
    pub(crate) fn sound_muted(&self) -> bool {
        self.ds.r8(SOUND_MUTE) & 1 != 0
    }

    /// 展示錄影（`DM1.LCK`，`AAE2h` 起 3000 位元組）第 `index` 個位元組，位址 16 位元環繞。
    /// 每幀一個位元組，記錄玩家 1 的輸入位元（[`crate::player::InputBit`]）。
    pub(crate) fn demo_byte(&self, index: u16) -> u8 {
        self.ds.r8(DEMO_RECORD.wrapping_add(index))
    }

    /// 非展示時，玩家 1 把本幀的輸入位元錄在展示錄影的第 `index` 個位元組。
    pub(crate) fn record_demo_byte(&mut self, index: u16, v: u8) {
        self.ds.w8(DEMO_RECORD.wrapping_add(index), v);
    }

    /// 按鍵是不是按住（按鍵狀態表 `2748Bh + 掃描碼` 的值是 1；2 是剛放開）。表由 INT 9 維護。
    pub(crate) fn key_held(&self, scan: u8) -> bool {
        self.ds.r8(KEY_TABLE.wrapping_add(u16::from(scan))) == KEY_HELD
    }

    /// 展示模式還原輸入時直接寫按鍵狀態表。
    pub(crate) fn set_key(&mut self, scan: u8, held: bool) {
        self.ds.w8(KEY_TABLE.wrapping_add(u16::from(scan)), u8::from(held));
    }

    /// 玩家的開火旗標（`273A7h` 一族，INT 9 的 `sub_1A9B5` 設，玩家邏輯清）。
    pub(crate) fn fire_flag(&self, player: usize, flag: FireFlag) -> bool {
        let fire = &self.flow.controls.fire;
        let slot = match flag {
            FireFlag::NewPress => fire.new_press[player],
            FireFlag::Event => fire.event[player],
            FireFlag::Turn => fire.turn[player],
        };
        slot == FLAG_ON
    }

    pub(crate) fn set_fire_flag(&mut self, player: usize, flag: FireFlag, on: bool) {
        let fire = &mut self.flow.controls.fire;
        let slot = match flag {
            FireFlag::NewPress => &mut fire.new_press[player],
            FireFlag::Event => &mut fire.event[player],
            FireFlag::Turn => &mut fire.turn[player],
        };
        *slot = if on { FLAG_ON } else { FLAG_OFF };
    }

    /// 難度對敵人耐久的加成（原版 `word_1E6B2`，0 或 1）。
    pub(crate) fn hp_bonus(&self) -> u16 {
        self.ds.r16(DIFFICULTY_HP)
    }

    /// 難度對開火亂數的加成（原版 `word_1E6B4`，0、2 或 3）。
    pub(crate) fn fire_bonus(&self) -> u16 {
        self.ds.r16(DIFFICULTY_FIRE)
    }

    /// 繪製的暫存區選擇（原版 `word_2685C`），之後的繪製命令都帶這個值，直到有人再設。
    pub(crate) fn select_scratch(&mut self, select: u16) {
        self.ds.w16(SCRATCH_SELECT, select);
    }

    /// 表外的資料段字組：不屬於任何已搬走的領域，也不在唯讀表格內。實體傷害寫到指標 0 加 20h 的
    /// 原版行為（`dseg:0020h`，啟動時文字畫面的色彩屬性）用它。
    pub(crate) fn stray_word(&self, at: u16) -> u16 {
        self.ds.r16(at)
    }

    pub(crate) fn set_stray_word(&mut self, at: u16, v: u16) {
        self.ds.w16(at, v);
    }

    /// 測試用：設關卡字母。
    #[cfg(test)]
    pub(crate) fn set_stage_letter(&mut self, letter: u8) {
        self.flow.stage = letter;
    }

    /// 測試用：設展示旗標。
    #[cfg(test)]
    pub(crate) fn set_demo_playing(&mut self, on: bool) {
        self.set_demo_mode(on);
    }

    /// 測試用：設無敵秘技旗標。
    #[cfg(test)]
    pub(crate) fn set_invincible_cheat(&mut self, on: bool) {
        self.flow.controls.cheat.invincible = u16::from(if on { FLAG_ON } else { FLAG_OFF });
    }

    /// 測試用：密碼檢查通過旗標。
    #[cfg(test)]
    pub(crate) fn set_password_accepted(&mut self, on: bool) {
        self.ds.w8(PASSWORD_ACCEPTED, if on { b'Y' } else { b'N' });
    }

    /// 測試用：完整性檢查表的第 `k` 個字組。
    #[cfg(test)]
    pub(crate) fn set_integrity_table_word(&mut self, k: u16, v: u16) {
        self.ds.w16(INTEGRITY_TABLE.wrapping_add(2 * k), v);
    }

    /// 測試用：音樂可用，`start_music` 才會送命令。
    #[cfg(test)]
    pub(crate) fn enable_music(&mut self) {
        self.ds.w8(MUSIC_ENABLED, b'Y');
    }

    /// 測試用：設難度加成。
    #[cfg(test)]
    pub(crate) fn set_difficulty(&mut self, hp: u16, fire: u16) {
        self.ds.w16(DIFFICULTY_HP, hp);
        self.ds.w16(DIFFICULTY_FIRE, fire);
    }

    /// 測試用：目前的暫存區選擇。
    #[cfg(test)]
    pub(crate) fn scratch_select(&self) -> u16 {
        self.ds.r16(SCRATCH_SELECT)
    }
}

// ---- 按鍵與標題流程的旗標 ----
//
// 標題與片尾讀寫的共用變數：INT 9 寫的按鍵表、標題流程與各關之間傳遞的旗標。


/// `byte_1CFAC`：初值 `N`，`sub_10FEA` 見到 `N` 轉入標題迴圈；標題迴圈第一行改 `Y`。
const TITLE_ENTERED: u16 = a(0x1CFAC);
/// `byte_1CFDC`：`N` 一般模式，其他值都當示範模式（原版 `cmp byte_1CFDC, 4Eh`）。標題寫 `Y` 或 `N`。
const DEMO_MODE: u16 = a(0x1CFDC);
/// `byte_274EC`：讀檔失敗時換磁片提示的磁碟號字元。標題在第二關起設成 `'2'`，重製沒有人讀。
const DISK_HINT: u16 = a(0x274EC);

/// `byte_1CFDD` 的活動玩家位元：玩家 1 在位元 0，玩家 2 在位元 4。
pub(crate) const PLAYER1_ACTIVE: u8 = 0x01;
pub(crate) const PLAYER2_ACTIVE: u8 = 0x10;

impl Core {
    pub(crate) fn key_pressed(&self) -> bool {
        self.last_key() != 0
    }

    /// 標題迴圈開始（原版 `byte_1CFAC` = `Y`）。
    pub(crate) fn mark_title_entered(&mut self) {
        self.ds.w8(TITLE_ENTERED, b'Y');
    }

    /// 通知第一關：玩家是從標題進來的，要換 `B.MIX` 開音樂（原版 `byte_1CFAB` = `Y`）。
    pub(crate) fn request_stage_music(&mut self) {
        self.ds.w8(STARTED_FROM_TITLE, b'Y');
    }

    /// 是不是示範模式：只有 `N` 是一般模式。
    pub(crate) fn in_demo_mode(&self) -> bool {
        self.ds.r8(DEMO_MODE) != b'N'
    }

    /// 活動玩家位元（原版 `byte_1CFDD`）：[`PLAYER1_ACTIVE`]、[`PLAYER2_ACTIVE`] 或兩者。
    /// 玩家領域從這個位元組讀誰在場，選單和示範都在這裡設定。
    pub(crate) fn set_active_players(&mut self, mask: u8) {
        self.set_players_present(mask);
    }

    /// 難度加成（原版 `word_1E6B2` 耐久、`word_1E6B4` 開火），敵人領域從 [`Core::hp_bonus`]、[`Core::fire_bonus`] 讀。
    pub(crate) fn set_difficulty_bonuses(&mut self, hp: u16, fire: u16) {
        self.ds.w16(DIFFICULTY_HP, hp);
        self.ds.w16(DIFFICULTY_FIRE, fire);
    }

    /// 換磁片提示的磁碟號字元（原版 `byte_274EC`）。
    pub(crate) fn set_disk_hint(&mut self, disk: u8) {
        self.ds.w8(DISK_HINT, disk);
    }

}

// ---- 格網 ----

/// G1 實體格網（`1B32h`，800 位元組）的起點。這裡只用來說明 G2 列表的別名。
pub(crate) const GRID1_START: u16 = 0x1B32;
/// G1 的終點（不含）。
pub(crate) const GRID1_END: u16 = 0x1E2A;
/// G1 與 G2 的列距：40 欄，每格一位元組。
pub(crate) const GRID1_ROW_BYTES: u16 = 0x28;
/// 戰場的欄數，G1、G2、G3、G4 都是 40 欄。敵人的足跡超出這個範圍的格不寫。
pub(crate) const GRID_COLS: u16 = 0x28;
/// `sub_162B1` 清掉的位元組數：20 列。
const GRID1_BYTES: u16 = 0x320;
/// 一列最後可測的格相對列起點的位移加 1：列尾（欄 39）嚴格在它之前。
const GRID1_ROW_SPAN: u16 = 0x27;
/// 玩家掃描用的格網窗與列寬：玩家掃 G1、G2 時，位址要嚴格在窗的起點與上限之間，也嚴格在列起點加 45h 之前
/// （比玩家彈的 27h 寬）。
const SCAN_ROW_SPAN: u16 = 0x45;

/// 玩家掃描的格網。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanGrid {
    /// G1 實體格：窗的上限 `217Ch`，比 G1 的終點 `1E2Ah` 多出去的部分是 G1 之後的資料。
    Entities,
    /// G2 敵彈格：窗的上限 `3266h`，G2 之後直到敵彈池指標表之前的資料。
    Bullets,
}

impl ScanGrid {
    const fn start(self) -> u16 {
        match self {
            ScanGrid::Entities => GRID1_START,
            ScanGrid::Bullets => GRID2_START,
        }
    }

    const fn limit(self) -> u16 {
        match self {
            ScanGrid::Entities => 0x217C,
            ScanGrid::Bullets => 0x3266,
        }
    }
}
/// 玩家彈取 G1 列起點時，列（無號）大於等於這個值一律改取 [`GRID1_FAR_ROW`]。
const GRID1_NEAR_ROWS: u16 = 0x17C;
/// 越界列的索引：列基底表之後第 1F4h 項，位址 `1EF2h`，G1 之後的資料，實測為 0。
const GRID1_FAR_ROW: u16 = 0x1F4;
/// G4 地形格（第三關，每列 50h 位元組，奇數位址的字組）的範圍，閉區間。
pub(crate) const GRID4_START: u16 = 0x900C;
const GRID4_LAST: u16 = 0x95FC;
/// G4 的列距：40 欄 × 2 位元組。第三關捲動的新欄與碰撞讀取都用這個列距。
pub(crate) const TERRAIN_ROW_BYTES: u16 = 0x50;
/// G4 與 G3 各 20 列 × 40 欄 = 320h 個字組。
const TERRAIN_WORDS: u16 = 0x320;
/// `word_2684C`（`9C8Ch`）：第三關新欄的 G4 寫入游標，位元組位移，捲動步開始設成一列（50h），每寫一格加一列。
/// 它的位址緊接在 G3 之後（G3 起點加 640h），是 G3 越界的第一個字組：玩家本體的髒格標記（`grid3_mark_block`）
/// 沒有範圍檢查，列號超出畫面時（例如死亡動畫往下走）寫得到 G3 之後的位址，包含這個字組。所以它和 G3 一樣留在資料段，
/// 與原版共用同一塊記憶體，不進 `MapScroll`。這條路徑沒有收據分辨，是靜態推論。
const TERRAIN_CURSOR: u16 = a(0x2684C);

/// 一列 G1 的列尾位址（不含）：列起點加 27h。
pub(crate) const fn grid1_row_end(row_start: u16) -> u16 {
    row_start.wrapping_add(GRID1_ROW_SPAN)
}

impl Core {
    /// G1 某一列的起點（原版 `sub_16CC6`，玩家彈用）。列（無號）大於等於 17Ch 改取索引 1F4h；
    /// 小於的列直接以列號索引列基底表，所以列 14h 至 17Bh 讀到的是 G1 自己的內容，原版行為。
    /// 列基底表第 14h 項就是 G1 的前兩個位元組，列 0 之前的負列落在越界列。
    pub(crate) fn grid1_row_start(&self, row: u16) -> u16 {
        self.row_base_offset(row).wrapping_add(GRID1_START)
    }

    /// 列基底表（`1B0Ah`）的第 `row` 項：列（無號）大於等於 17Ch，包含負列，一律取索引 1F4h。
    /// G1、G2 的玩家掃描與玩家彈共用。
    fn row_base_offset(&self, row: u16) -> u16 {
        let row = if row >= GRID1_NEAR_ROWS { GRID1_FAR_ROW } else { row };
        self.ds.r16(row.wrapping_shl(1).wrapping_add(GRID_ROW_BASE_TABLE))
    }

    /// 玩家掃描的一格（原版 `sub_14C61`、`sub_14C3D`、`sub_14D68`、`sub_14BF4` 一族算位址，加上迴圈裡的範圍檢查）：
    /// 位址須嚴格在格網窗之內，也嚴格在該列起點之後、列起點加 45h 之前。窗比格網大，列加 45h 比一列的 28h 寬，
    /// 所以位置正常時範圍檢查都成立；列號落在表外時掃到的是格網之外的資料，照原版。不合格回傳 `None`。
    pub(crate) fn scan_cell(&self, grid: ScanGrid, row_start: u16, addr: u16) -> Option<u8> {
        let inside = addr < grid.limit() && addr > grid.start() && addr < row_start.wrapping_add(SCAN_ROW_SPAN) && addr > row_start;
        inside.then(|| self.ds.r8(addr))
    }

    /// 玩家掃描時第 `row` 列的起點：列基底表加格網起點。
    pub(crate) fn scan_row_start(&self, grid: ScanGrid, row: u16) -> u16 {
        self.row_base_offset(row).wrapping_add(grid.start())
    }

    /// 玩家本體佔的 G3 髒格（原版 `sub_14A2C`、`sub_1555A`）：從 (`row`, `col`) 起 `rows` 列 × `cols` 欄，
    /// 每格寫一個字組 `value`，連奇數位元組一起覆寫。位址是 G3 起點加 `2 × (低位元組(row) × 28h + col + 1)`，
    /// 不經列起點表也不檢查範圍。
    pub(crate) fn grid3_mark_block(&mut self, row: u16, col: u16, rows: u16, cols: u16, value: u16) {
        let first = (u16::from(row as u8) * GRID_COLS).wrapping_add(col).wrapping_add(1).wrapping_shl(1);
        for r in 0..rows {
            for c in 0..cols {
                let at = first.wrapping_add(r * GRID3_ROW_BYTES).wrapping_add(c * 2);
                self.ds.w16(GRID3_START.wrapping_add(at), value);
            }
        }
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

    /// 清空 G1（原版 `sub_162B1`，每幀末）：從 `1B32h` 起 190h 個字組，比有人寫入的 19 列多一列。
    pub(crate) fn clear_grid1(&mut self) {
        self.ds.fill16(GRID1_START, GRID1_BYTES / 2, 0);
    }

    /// 敵人把形狀的一格值寫進 G1（原版 `sub_16233`）：覆蓋，不是 `or`，所以槽號大的敵人蓋過槽號小的。
    /// 欄（無號）大於 27h 的格略過，不會溢進相鄰的列；列起點取自列基底表（[`Core::grid1_row_start`]）。
    pub(crate) fn grid1_put(&mut self, row: u16, col: u16, v: u8) {
        if col < GRID_COLS {
            let at = self.grid1_row_start(row).wrapping_add(col);
            self.ds.w8(at, v);
        }
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

    /// 第三關捲動一格：G4 與 G3 各整體左移一個字組（原版 `sub_1840E` 的兩次 `rep movsw`，各 320h 個字組）。
    /// 兩次搬移的讀取都超出自己的範圍，照原版：
    ///
    /// - G4 的最後一個字組取自 G3 的第一個字組（G3 起點，在 G3 左移之前讀）。
    /// - G3 的最後一個字組取自 G3 之後的 `word_2684C`（G3 起點加 640h），所以 G3 最後一格的髒標記是新欄寫入游標
    ///   的值，那個位置沒有找到其他引用。
    /// - 結尾的 `stosb` 把 `word_2684C` 的低位元組清 0；`scroll_map_second_page` 隨後才把游標設成 50h，所以沒有後續效果。
    pub(crate) fn scroll_terrain_grids(&mut self) {
        self.ds.copy16(GRID4_START, GRID4_START + 2, TERRAIN_WORDS);
        self.ds.copy16(GRID3_START, GRID3_START + 2, TERRAIN_WORDS);
        self.ds.w8(GRID3_START.wrapping_add(2 * TERRAIN_WORDS), 0);
    }

    /// 新欄的第一格從游標 50h 起，也就是 G4 讀取格狀的（列 0，欄 39）（原版 `18306h`）。
    pub(crate) fn start_terrain_column(&mut self) {
        self.ds.w16(TERRAIN_CURSOR, TERRAIN_ROW_BYTES);
    }

    /// 新欄的一格圖塊索引存進 G4（原版 `sub_1839F`）：寫在 G4 起點加游標，游標加一列。
    pub(crate) fn store_terrain_tile(&mut self, tile: u16) {
        let cursor = self.ds.r16(TERRAIN_CURSOR);
        self.ds.w16(cursor.wrapping_add(GRID4_START), tile);
        self.ds.w16(TERRAIN_CURSOR, cursor.wrapping_add(TERRAIN_ROW_BYTES));
    }

    /// 初始視窗的第 `cell` 格圖塊索引存進 G4（原版 `sub_17DF9`）：從 G4 起點加 2 起連續寫，不分列，
    /// 視窗每列 42 格，所以列距是 54h 位元組，與碰撞讀取的列距（50h）不同。
    pub(crate) fn fill_terrain_cell(&mut self, cell: u16, tile: u16) {
        self.ds.w16(GRID4_START.wrapping_add(2).wrapping_add(2u16.wrapping_mul(cell)), tile);
    }

    #[cfg(test)]
    pub(crate) fn set_terrain_cursor(&mut self, v: u16) {
        self.ds.w16(TERRAIN_CURSOR, v);
    }

    #[cfg(test)]
    pub(crate) fn terrain_cursor(&self) -> u16 {
        self.ds.r16(TERRAIN_CURSOR)
    }
}

const _: () = assert!(GRID3_START + 2 * TERRAIN_WORDS == TERRAIN_CURSOR, "word_2684C 緊接在 G3 之後");
const _: () = assert!(GRID4_START + 2 * TERRAIN_WORDS == GRID3_START, "G3 緊接在 G4 之後");
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
pub(crate) const GRID3_START: u16 = 0x964C;
const GRID3_END: u16 = 0x9C3C;
/// 一列 40 格，每格 2 位元組，所以列距 50h，列內可寫的範圍是 `[第一格, 第一格 + 4Eh]`。
pub(crate) const GRID3_ROW_BYTES: u16 = 0x50;
const GRID3_ROW_LAST_CELL: u16 = 0x4E;
/// 列起點表（`21C2h`）：每列一個字組。列 0 至 13h 從 0001h 起每列加 50h。G3 與 G4 共用。
/// 第 21 項（列 14h，越界列）的位址 `21EAh` 就是敵人的死亡旗標 `word_1EDAA`（[`ENEMY_DEATH_FLAG`]），
/// 映像裡的初值是 0059h，但敵人每更新一個就把它寫成 `N`（004Eh）或 `Y`（0059h），所以越界列的起點隨敵人的狀態變。
const GRID_ROW_TABLE: u16 = 0x21C2;
/// 列號（無號）大於等於這個值一律當作這一列。
const GRID_LAST_ROW: u16 = 0x14;

const _: () = assert!(GRID_ROW_TABLE + 2 * GRID_LAST_ROW == ENEMY_DEATH_FLAG, "列起點表的越界列就是敵人的死亡旗標");

/// 敵人的死亡旗標，`word_1EDAA`（`21EAh`）。敵人領域每更新一個敵人就寫一次，`N` 或 `Y`。
/// 它同時是上面列起點表的越界列，格網位址函式與玩家領域（尚未搬走）都從資料段直接讀，所以留在資料段，不進 `Enemies`。
const ENEMY_DEATH_FLAG: u16 = a(0x1EDAA);

impl Core {
    /// 記下最近一個更新的敵人本幀要不要釋放（原版 `word_1EDAA`）：`Y` 釋放，`N` 不釋放。
    /// 列起點表的越界列（G3、G4 的列 14h 以上）讀的就是這個字組。
    pub(crate) fn set_enemy_death_flag(&mut self, dying: bool) {
        self.ds.w16(ENEMY_DEATH_FLAG, if dying { u16::from(b'Y') } else { u16::from(b'N') });
    }

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
    fn the_out_of_range_row_start_is_the_enemy_death_flag() {
        // 列起點表第 21 項的位址就是 word_1EDAA：越界列的起點隨敵人狀態在 `N` 與 `Y` 之間變，G3 與 G4 都一樣。
        let mut c = core();
        for (dying, flag) in [(false, 0x4E), (true, 0x59)] {
            c.set_enemy_death_flag(dying);
            assert_eq!(c.grid3_cell_addr(0x14, 0), GRID3_START + flag + 1);
            assert_eq!(c.grid4_cell_addr(0x20, 0), GRID4_START + flag + 1);
        }
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
    fn players_mirror_is_the_inverse_of_load_for_any_dump() {
        let ds = Dseg::from_image(&pseudo_random_image());
        let mut pl = Players::default();
        pl.load(&ds);
        let mut same = ds.clone();
        pl.store(&mut same);
        assert_eq!(same.bytes(), ds.bytes(), "store(load(ds)) 要等於 ds");
        // 寫進全 0 的資料段再讀回，結構相同：store 真的寫了每個欄位。
        let mut blank = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        pl.store(&mut blank);
        let mut back = Players::default();
        back.load(&blank);
        assert_eq!((back.alive, back.ship), (pl.alive, pl.ship));
        assert_ne!(pl.ship, Players::default().ship, "測試資料不該全 0");
        assert_ne!(pl.ship[0], pl.ship[1]);
    }

    #[test]
    fn player_fields_land_at_the_documented_offsets() {
        // 每個欄位一個獨一無二的值，位移是機器碼的線性位址減 1CBC0h，獨立列在這裡，不引用上面的常數。
        // (欄位, 玩家 1 的位移, 玩家 2 的位移, 寬)
        let layout: [(&str, [u16; 2], u8); 24] = [
            ("hp", [0x041E, 0x0420], 2),
            ("charge", [0x0ECB, 0x0F2D], 2),
            ("pods.first", [0x0ECD, 0x0F2F], 1),
            ("pods.second", [0x0ECE, 0x0F30], 1),
            ("pods.facing", [0x0ECF, 0x0F31], 2),
            ("pods.image", [0x0ED4, 0x0F36], 2),
            ("pods.anim", [0x0ED6, 0x0F38], 2),
            ("pods.col", [0x0ED8, 0x0F3A], 2),
            ("pods.row", [0x0EDA, 0x0F3C], 2),
            ("pods.blit_x", [0x0EE0, 0x0F42], 2),
            ("pods.blit_y", [0x0EE2, 0x0F44], 2),
            ("input_bits", [0x0EE4, 0x0F46], 1),
            ("fired", [0x0EE5, 0x0F47], 1),
            ("beam_y", [0x0EE6, 0x0F48], 2),
            ("weapon", [0x0EE9, 0x0F4B], 2),
            ("flame", [0x0EEB, 0x0F4D], 1),
            ("image", [0x0EFD, 0x0F57], 2),
            ("hit", [0x0F00, 0x0F5A], 1),
            ("invincible", [0x0F01, 0x0F5B], 1),
            ("dying", [0x0F03, 0x0F5D], 1),
            ("blit_y", [0x0F05, 0x0F5F], 2),
            ("blit_x", [0x0F07, 0x0F61], 2),
            ("row", [0x0F09, 0x0F77], 2),
            ("col", [0x0F0B, 0x0F79], 2),
        ];
        let value = |who: usize, k: usize| -> u16 { 0x0101 * (who as u16 * 0x30 + k as u16 + 1) };
        let mut pl = Players { alive: 0x5A, ..Players::default() };
        for who in 0..PLAYER_COUNT {
            let get = |k: usize| value(who, k);
            let p = &mut pl.ship[who];
            p.hp = get(0);
            p.charge = get(1);
            p.pods.first = get(2) as u8;
            p.pods.second = get(3) as u8;
            p.pods.facing = get(4);
            p.pods.image = get(5);
            p.pods.anim = get(6);
            p.pods.col = get(7);
            p.pods.row = get(8);
            p.pods.blit_x = get(9);
            p.pods.blit_y = get(10);
            p.input_bits = get(11) as u8;
            p.fired = get(12) as u8;
            p.beam_y = get(13);
            p.weapon = get(14);
            p.flame = get(15) as u8;
            p.image = get(16);
            p.hit = get(17) as u8;
            p.invincible = get(18) as u8;
            p.dying = get(19) as u8;
            p.blit_y = get(20);
            p.blit_x = get(21);
            p.row = get(22);
            p.col = get(23);
        }
        let mut ds = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        pl.store(&mut ds);
        assert_eq!(ds.r8(0x041D), 0x5A, "存活位元組在 041Dh（byte_1CFDD）");
        for (k, (name, offsets, width)) in layout.iter().enumerate() {
            for who in 0..PLAYER_COUNT {
                let want = value(who, k);
                let got = if *width == 2 { ds.r16(offsets[who]) } else { u16::from(ds.r8(offsets[who])) };
                let want = if *width == 2 { want } else { want & 0xFF };
                assert_eq!(got, want, "玩家 {} 的 {name} 在 {:04X}h", who + 1, offsets[who]);
            }
        }
        // 寫了的位元組數剛好等於各欄位寬度的總和（值都不是 0）：沒有欄位重疊，也沒有碰其他位元組。
        let touched = ds.bytes().iter().filter(|&&b| b != 0).count();
        let expected: usize = layout.iter().map(|(_, _, w)| usize::from(*w)).sum::<usize>() * PLAYER_COUNT + 1;
        assert_eq!(touched, expected, "非零位元組數等於欄位總寬");
        let mut back = Players::default();
        back.load(&ds);
        assert_eq!((back.alive, back.ship), (pl.alive, pl.ship));
    }

    #[test]
    fn the_player_work_variables_that_left_the_struct_are_not_written_by_store() {
        // 這些位移在原版是函式內先寫後讀的工作變數（DS_EXEMPT），Mirror 不碰。
        let mut pl = Players::default();
        for p in &mut pl.ship {
            *p = Player { hp: 0xFFFF, col: 0xFFFF, row: 0xFFFF, ..Player::default() };
        }
        let mut ds = Dseg::from_image(&vec![0xAAu8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000]);
        pl.store(&mut ds);
        for off in [0x0ECA, 0x0EDC, 0x0EDD, 0x0EDE, 0x0EDF, 0x0EE8, 0x0F2C, 0x0F3E, 0x0F3F, 0x0F40, 0x0F41, 0x0F4A] {
            assert_eq!(ds.r8(off), 0xAA, "{off:04X}h 不屬於結構");
        }
    }

    #[test]
    fn player_shapes_and_flags_are_read_from_the_documented_places() {
        let mut c = core();
        // 形狀表：玩家 1 本體 0F0Dh、砲座 0F16h、尾焰 0F1Eh；玩家 2 的 0F7Bh、0F85h、0F8Dh。
        let table = [(PlayerShape::Body, 0x0F0D, 0x0F7B), (PlayerShape::Pod, 0x0F16, 0x0F85), (PlayerShape::Flame, 0x0F1E, 0x0F8D)];
        for (shape, one, two) in table {
            assert_eq!((shape.table_offset(0), shape.table_offset(1)), (one, two));
        }
        // 開火旗標：玩家 1 在 273A7h、273A9h、273AAh，玩家 2 在 273A8h、273ABh、273ACh（dseg 位移 A7E7h 起）。
        for (player, flag, off) in [
            (0, FireFlag::NewPress, 0xA7E7u16),
            (0, FireFlag::Event, 0xA7E9),
            (0, FireFlag::Turn, 0xA7EA),
            (1, FireFlag::NewPress, 0xA7E8),
            (1, FireFlag::Event, 0xA7EB),
            (1, FireFlag::Turn, 0xA7EC),
        ] {
            c.set_fire_flag(player, flag, true);
            let mut ds = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
            c.flow.store(&mut ds);
            assert_eq!(ds.r8(off), b'Y', "{flag:?} 玩家 {}", player + 1);
            c.set_fire_flag(player, flag, false);
            c.flow.store(&mut ds);
            assert_eq!(ds.r8(off), b'N');
        }
        // 按鍵狀態表在 A8CBh + 掃描碼，值 1 才算按住（2 是剛放開）。
        c.ds.w8(0xA8CB + 0x48, 1);
        c.ds.w8(0xA8CB + 0x50, 2);
        assert!(c.key_held(0x48) && !c.key_held(0x50) && !c.key_held(0x4B));
        c.set_key(0x4B, true);
        assert_eq!(c.ds.r8(0xA8CB + 0x4B), 1);
        c.set_key(0x4B, false);
        assert_eq!(c.ds.r8(0xA8CB + 0x4B), 0);
        // 展示錄影在 AAE2h 起，位址 16 位元環繞。
        c.record_demo_byte(5, 0x7E);
        assert_eq!(c.ds.r8(0xAAE2 + 5), 0x7E);
        assert_eq!(c.demo_byte(0xFFFF), c.ds.r8(0xAAE2u16.wrapping_add(0xFFFF)));
    }

    #[test]
    fn player_scan_window_has_its_own_limits() {
        let mut c = core();
        c.fill_row_tables();
        // G1：嚴格大於 1B32h、嚴格小於 217Ch；每列嚴格在列起點之後、列起點加 45h 之前（玩家彈是 27h）。
        let row3 = c.scan_row_start(ScanGrid::Entities, 3);
        assert_eq!(row3, GRID1_START + 3 * 0x28);
        assert_eq!(c.scan_cell(ScanGrid::Entities, row3, row3), None, "列起點本身不掃");
        assert!(c.scan_cell(ScanGrid::Entities, row3, row3 + 1).is_some());
        assert!(c.scan_cell(ScanGrid::Entities, row3, row3 + 0x44).is_some(), "欄 44h 還在窗內，比玩家彈寬");
        assert_eq!(c.scan_cell(ScanGrid::Entities, row3, row3 + 0x45), None);
        assert_eq!(c.scan_cell(ScanGrid::Entities, GRID1_START - 8, GRID1_START), None, "G1 起點本身不掃");
        assert!(c.scan_cell(ScanGrid::Entities, GRID1_START - 8, GRID1_START + 1).is_some());
        assert!(c.scan_cell(ScanGrid::Entities, 0x2150, 0x217B).is_some());
        assert_eq!(c.scan_cell(ScanGrid::Entities, 0x2150, 0x217C), None, "上限 217Ch 不掃");
        // G2：2C1Ch 與 3266h。
        let row5 = c.scan_row_start(ScanGrid::Bullets, 5);
        assert_eq!(row5, GRID2_START + 5 * 0x28);
        assert!(c.scan_cell(ScanGrid::Bullets, row5, row5 + 1).is_some());
        assert_eq!(c.scan_cell(ScanGrid::Bullets, GRID2_START - 8, GRID2_START), None);
        assert!(c.scan_cell(ScanGrid::Bullets, 0x3240, 0x3265).is_some());
        assert_eq!(c.scan_cell(ScanGrid::Bullets, 0x3240, 0x3266), None);
        // 負列與 17Ch 以上的列取列基底表索引 1F4h（1EF2h），不像敵彈的 G2 寫入取索引 14h。
        c.ds.w16(0x1EF2, 0x0040);
        for row in [0x17Cu16, 0xFFFD] {
            assert_eq!(c.scan_row_start(ScanGrid::Entities, row), GRID1_START + 0x40);
            assert_eq!(c.scan_row_start(ScanGrid::Bullets, row), GRID2_START + 0x40);
        }
        assert_ne!(c.scan_row_start(ScanGrid::Bullets, 0xFFFD), c.grid2_row_base(0xFFFD), "玩家掃描與敵彈寫入的越界列規則不同");
    }

    #[test]
    fn grid3_mark_block_writes_words_without_a_range_check_and_uses_the_low_byte_of_the_row() {
        let mut c = core();
        c.ds.w8(GRID3_START + 1, 0x77);
        c.grid3_mark_block(0, 0, 2, 4, 1);
        // 第 0 列第 0 欄在 G3 起點加 2；每格一個字組 0001h，奇數位元組被蓋成 0，相鄰格之間隔 2 位元組，列距 50h。
        for (row, col) in [(0u16, 0u16), (0, 3), (1, 0), (1, 3)] {
            let at = GRID3_START + 2 + row * GRID3_ROW_BYTES + 2 * col;
            assert_eq!((c.ds.r8(at), c.ds.r8(at + 1)), (1, 0), "列 {row} 欄 {col}");
        }
        assert_eq!(c.ds.r8(GRID3_START + 1), 0x77, "起點前一個位元組不動");
        assert_eq!(c.ds.r8(GRID3_START + 2 + 8), 0, "第 5 欄不標");
        // 列只取低位元組：列 101h 與列 1 同位址。
        let mut a = core();
        let mut b = core();
        a.grid3_mark_block(0x101, 7, 1, 1, 0xBEEF);
        b.grid3_mark_block(1, 7, 1, 1, 0xBEEF);
        assert_eq!(a.ds.bytes(), b.ds.bytes());
        // 不經範圍檢查：列 20（G3 之外）照寫。
        let mut c = core();
        c.grid3_mark_block(20, 0, 1, 1, 0x0001);
        assert_eq!(c.ds.r16(GRID3_START + 2 + 20 * GRID3_ROW_BYTES), 1);
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
    fn enemies_mirror_is_the_inverse_of_load_for_any_dump() {
        let ds = Dseg::from_image(&pseudo_random_image());
        let mut en = Enemies::default();
        en.load(&ds);
        let mut same = ds.clone();
        en.store(&mut same);
        assert_eq!(same.bytes(), ds.bytes(), "store(load(ds)) 要等於 ds");
        // 寫進全 0 的資料段再讀回，結構相同：store 真的寫了每個欄位。
        let mut blank = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        en.store(&mut blank);
        let mut back = Enemies::default();
        back.load(&blank);
        assert_eq!((back.slots, back.spare_link, back.last_sprite), (en.slots, en.spare_link, en.last_sprite));
        assert_ne!(en.slots, Enemies::default().slots, "測試資料不該全 0");
    }

    #[test]
    fn enemy_fields_land_at_the_documented_offsets() {
        let mut en = Enemies::default();
        for (i, slot) in en.slots.iter_mut().enumerate() {
            let w: [u16; 24] = std::array::from_fn(|k| 0x100 * (i as u16 + 1) + k as u16 + 1);
            slot.link = 0x7000 + i as u16;
            slot.enemy = Enemy::from_words(w);
            slot.sprite = Sprite { image: 0x8000 + i as u16, x: 0x8100 + i as u16, y: 0x8200 + i as u16, kind: 0x8300 + i as u16 };
            slot.flash = 0x8400 + i as u16;
        }
        en.spare_link = 0x6666;
        en.last_sprite = Sprite { image: 0x9001, x: 0x9002, y: 0x9003, kind: 0x9004 };
        let mut ds = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        en.store(&mut ds);
        for i in 0..EN_POOL_SIZE as u16 {
            let o = 2 * i;
            assert_eq!(ds.r16(0x2294 + o), 0x7000 + i, "槽 {i} 的指標在 2294h + 2i");
            let rec = 0x22B4 + 0x30 * i;
            for k in 0..24u16 {
                assert_eq!(ds.r16(rec + 2 * k), 0x100 * (i + 1) + k + 1, "槽 {i} 的 w{k} 在記錄 +{:02X}h", 2 * k);
            }
            assert_eq!(
                [ds.r16(0x21EC + o), ds.r16(0x220C + o), ds.r16(0x222C + o), ds.r16(0x224C + o), ds.r16(0x226C + o)],
                [0x8000 + i, 0x8100 + i, 0x8200 + i, 0x8300 + i, 0x8400 + i],
                "槽 {i} 的圖、x、y、畫法、閃白在五個陣列的第 i 項"
            );
        }
        assert_eq!(ds.r16(0x2294 + 0x1E), 0x6666, "備用字組在 22B2h");
        assert_eq!([ds.r16(0x228C), ds.r16(0x228E), ds.r16(0x2290), ds.r16(0x2292)], [0x9001, 0x9002, 0x9003, 0x9004]);
        // 具名欄位依序對應 w0 至 w23；傷害字組是 w16，位移 20h。
        let e = en.slots[0].enemy;
        assert_eq!(
            [e.hp, e.frames, e.blit_x, e.blit_y, e.blit_dx, e.blit_dy, e.fire_threshold, e.bullet],
            [0x101, 0x102, 0x103, 0x104, 0x105, 0x106, 0x107, 0x108]
        );
        assert_eq!([e.mode, e.col, e.row, e.dcol, e.drow, e.script, e.shape, e.spare], [0x109, 0x10A, 0x10B, 0x10C, 0x10D, 0x10E, 0x10F, 0x110]);
        assert_eq!(
            [e.damage, e.explosion, e.kind, e.carrier, e.anim_limits, e.anim, e.rand_index, e.spill],
            [0x111, 0x112, 0x113, 0x114, 0x115, 0x116, 0x117, 0x118]
        );
        assert_eq!(ds.r16(0x22B4 + ENEMY_DAMAGE_OFFSET), 0x111);
        assert_eq!(Enemy::from_words(e.to_words()), e);
        // 池指標表、備用字組、繪製陣列、記錄區互不重疊，也不碰緊鄰的資料。
        assert_eq!(ds.r16(0x22B4 + 0x2D0), 0, "記錄區之後的字組不動");
        assert_eq!(ds.r16(0x21EA), 0, "繪製陣列之前的死亡旗標 word_1EDAA 不在 Enemies 裡");
        assert_eq!((enemy_record_link(3), enemy_record_slot(0x22B4 + 3 * 0x30)), (0x22B4 + 3 * 0x30, Some(3)));
        assert_eq!([enemy_record_slot(0x22B3), enemy_record_slot(0x22B5), enemy_record_slot(0x22B4 + 15 * 0x30), enemy_record_slot(0)], [None; 4]);
    }

    #[test]
    fn scenery_mirror_is_the_inverse_of_load_for_any_dump() {
        let ds = Dseg::from_image(&pseudo_random_image());
        let mut sc = Scenery::default();
        sc.load(&ds);
        let mut same = ds.clone();
        sc.store(&mut same);
        assert_eq!(same.bytes(), ds.bytes(), "store(load(ds)) 要等於 ds");
        let mut blank = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        sc.store(&mut blank);
        let mut back = Scenery::default();
        back.load(&blank);
        assert_eq!(back.props, sc.props);
        assert_ne!(sc.props, Scenery::default().props, "測試資料不該全 0");
    }

    #[test]
    fn scenery_fields_land_at_the_documented_offsets() {
        let mut sc = Scenery::default();
        for (i, p) in sc.props.iter_mut().enumerate() {
            let i = i as u16;
            *p = Prop { image: 0x100 + i, x: 0x200 + i, y: 0x300 + i, dx: 0x400 + i };
        }
        let mut ds = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        sc.store(&mut ds);
        for i in 0..SC_POOL_SIZE as u16 {
            let o = 2 * i;
            assert_eq!(
                [ds.r16(0x198C + o), ds.r16(0x19A0 + o), ds.r16(0x19B4 + o), ds.r16(0x19C8 + o)],
                [0x400 + i, 0x200 + i, 0x300 + i, 0x100 + i],
                "槽 {i}：dx、x、y、圖庫索引在四個陣列的第 i 項"
            );
        }
        assert_eq!(ds.r16(0x19DC), 0, "群組表不動");
        assert_eq!(ds.r16(0x198A), 0, "陣列之前不動");
    }

    #[test]
    fn title_mirror_is_the_inverse_of_load_for_any_dump() {
        let ds = Dseg::from_image(&pseudo_random_image());
        let mut title = Title::default();
        title.load(&ds);
        let mut same = ds.clone();
        title.store(&mut same);
        assert_eq!(same.bytes(), ds.bytes(), "store(load(ds)) 要等於 ds");
        // 寫進全 0 的資料段再讀回，結構相同：store 真的寫了每個欄位。
        let mut blank = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        title.store(&mut blank);
        let mut back = Title::default();
        back.load(&blank);
        assert_eq!(back, title);
        assert_ne!(title, Title::default(), "測試資料不該全 0");
    }

    #[test]
    fn title_fields_land_at_the_documented_offsets() {
        let title = Title { menu: Menu { row: 0x1111, level: 0x2222, role: 0x3333 }, demo_round: 0x4444 };
        let mut ds = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        title.store(&mut ds);
        assert_eq!([ds.r16(0x0580), ds.r16(0x0582), ds.r16(0x0584), ds.r16(0x0586)], [0x1111, 0x2222, 0x3333, 0x4444]);
        assert_eq!(TITLE_LEVEL, 0x0582);
        // 前後的字組不動：之前是 57Eh，之後 588h 起是過場字樣的位置暫存（DS_EXEMPT）。
        assert_eq!((ds.r16(0x057E), ds.r16(0x0588)), (0, 0));
    }

    #[test]
    fn palette_tables_match_the_original_immediates() {
        // 機器碼 `mov dx, 表`：1295Bh、12989h、129B7h、129E5h、12A13h、12A41h、12A6Fh、12A9Dh、12ACBh、143EAh、14419h。
        let tables = [
            Palette::TitleD,
            Palette::TitleE,
            Palette::Menu,
            Palette::Beg1A,
            Palette::Beg1B,
            Palette::J1,
            Palette::J2,
            Palette::J3,
            Palette::J4,
            Palette::EndingA,
            Palette::EndingB,
        ]
        .map(Palette::table_offset);
        assert_eq!(tables, [0x5A9, 0x5EA, 0x62B, 0x66C, 0x6AD, 0x6EE, 0x72F, 0x770, 0x7B1, 0xE54, 0xE95]);
        // 開場與過場的前九張相鄰排列，每張 41h 位元組（11h 屬性表加 30h 的 DAC 值）。
        assert!(tables[..9].windows(2).all(|w| w[1] - w[0] == 0x41));
    }

    #[test]
    fn key_accessors_use_the_scancode_table() {
        let mut c = core();
        // 1Ch（Enter）的按鍵表項在 A8CBh + 1Ch = A8E7h：INT 9 寫 1 按下、2 放開。
        c.ds.w8(0xA8E7, 1);
        assert!(c.key_down(Key::Enter) && !c.key_down(Key::Space));
        c.ds.w8(0xA8E7, 2);
        assert!(!c.key_down(Key::Enter), "放開的鍵不算按著");
        c.ds.w8(0xA8E7, 1);
        c.release_key(Key::Enter);
        assert_eq!(c.ds.r8(0xA8E7), 0);
        // 其餘的鍵對應：上 48h、下 50h、左 4Bh、右 4Dh、空白 39h、Q 10h、R 13h。
        for (key, sc) in [(Key::Up, 0x48u16), (Key::Down, 0x50), (Key::Left, 0x4B), (Key::Right, 0x4D), (Key::Space, 0x39), (Key::Q, 0x10), (Key::R, 0x13)] {
            c.ds.w8(0xA8CB + sc, 1);
            assert!(c.key_down(key), "{key:?}");
        }
        c.ds.w8(0xA862, 0x43);
        assert_eq!((c.last_key(), c.key_pressed()), (0x43, true));
        c.clear_last_key();
        assert!(!c.key_pressed());
    }

    #[test]
    fn demo_flag_is_normal_only_for_the_letter_n() {
        let mut c = core();
        c.set_demo_mode(false);
        assert!(!c.in_demo_mode());
        c.set_demo_mode(true);
        assert!(c.in_demo_mode());
        c.ds.w8(0x041C, b'x');
        assert!(c.in_demo_mode(), "N 以外的值都當示範，照原版 cmp byte_1CFDC, 4Eh");
    }

    #[test]
    fn scenery_groups_and_records_are_read_from_the_tables() {
        let mut c = core();
        c.ds.w16(0x19DC + 4 * 2, 385);
        c.ds.w16(0x19DC + 4 * 2 + 2, 0x7000);
        for (k, v) in [0x0E, 40, 20, 4].into_iter().enumerate() {
            c.ds.w16(0x7000 + 2 * k as u16, v);
        }
        assert_eq!(c.scenery_group(2), (385, 0x7000));
        assert_eq!(c.scenery_group(0), (0, 0));
        assert_eq!(c.scenery_record(0x7000), Prop { image: 0x0E, x: 40, y: 20, dx: 4 }, "記錄是 (圖庫索引, x, y, dx)");
    }

    #[test]
    fn enemy_template_reads_24_words_including_the_next_templates_first_word() {
        let mut c = core();
        for k in 0..24u16 {
            c.ds.w16(0x7000 + 2 * k, 0x700 + k);
        }
        let e = c.enemy_template(0x7000);
        assert_eq!(e.to_words(), std::array::from_fn(|k| 0x700 + k as u16));
        assert_eq!(e.spill, 0x700 + 23, "第 24 個字組是緊鄰的下一個樣板的 w0");
    }

    #[test]
    fn timetable_and_fire_roll_tables_are_read_from_the_data_segment() {
        let mut c = core();
        c.ds.w16(0x042C + 2 * 74, 9);
        assert_eq!(c.fire_roll_at(74), 9);
        assert_eq!(c.fire_roll_at(0), 0);
    }

    #[test]
    fn clear_grid1_zeroes_20_rows_and_grid1_put_stays_in_its_row() {
        let mut c = core();
        c.fill_row_tables();
        for a in GRID1_START..GRID1_START + 0x320 {
            c.ds.w8(a, 0x55);
        }
        c.ds.w8(GRID1_START + 0x320, 0x66);
        c.ds.w8(GRID1_START - 1, 0x77);
        c.clear_grid1();
        assert!((GRID1_START..GRID1_START + 0x320).all(|a| c.ds.r8(a) == 0));
        assert_eq!((c.ds.r8(GRID1_START + 0x320), c.ds.r8(GRID1_START - 1)), (0x66, 0x77), "前後各一格不清");
        c.grid1_put(3, 0, 1);
        c.grid1_put(3, 39, 2);
        c.grid1_put(3, 40, 3);
        c.grid1_put(3, 0xFFFF, 4);
        let row = c.grid1_row_start(3);
        assert_eq!([c.ds.r8(row), c.ds.r8(row + 39), c.ds.r8(row + 40), c.ds.r8(row - 1)], [1, 2, 0, 0], "欄 40 起與負欄不寫");
    }

    #[test]
    fn terrain_stage_is_the_stage_letter_c() {
        let mut c = core();
        assert!(!c.on_terrain_stage());
        c.set_stage_letter(b'C');
        assert!(c.on_terrain_stage());
        c.set_stage_letter(b'c');
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

    fn blank_ds() -> Dseg {
        Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET])
    }

    #[test]
    fn flow_mirror_is_the_inverse_of_load_for_any_dump() {
        let ds = Dseg::from_image(&pseudo_random_image());
        let mut flow = Flow::default();
        flow.load(&ds);
        let mut same = ds.clone();
        flow.store(&mut same);
        assert_eq!(same.bytes(), ds.bytes(), "store(load(ds)) 要等於 ds");
        let mut blank = blank_ds();
        flow.store(&mut blank);
        let mut back = Flow::default();
        back.load(&blank);
        assert_eq!(back, flow);
        assert_ne!(flow, Flow::default(), "測試資料不該全 0");
    }

    #[test]
    fn flow_fields_land_at_the_documented_offsets() {
        // 每個欄位一個位元組都非 0 的獨一無二的值。位移是機器碼的線性位址減 1CBC0h，獨立列在這裡。
        let mut flow = Flow { stage: 0x11, clear: 0x12, bursts: 0x13, integrity_mark: 0x14, tamper_countdown: 0x1501, ..Flow::default() };
        flow.clock = Clock { total: 0x2101, since_wait: 0x2202, since_sound_toggle: 0x2303, cycle_a: 0x2404, cycle_b: 0x2505 };
        flow.controls = Controls {
            last_scan: 0x31,
            fire: FireFlags { new_press: [0x41, 0x42], event: [0x43, 0x44], turn: [0x45, 0x46] },
            cheat: Cheat { unlocked: 0x5101, presses: [0x5202, 0x5303, 0x5404], invincible: 0x5505 },
            joymouse: 0x32,
        };
        let mut ds = blank_ds();
        flow.store(&mut ds);
        let bytes: [(u16, u8); 4] = [(0x041B, 0x11), (0x03ED, 0x12), (0x03EE, 0x13), (0x03EA, 0x14)];
        for (at, want) in bytes {
            assert_eq!(ds.r8(at), want, "{at:04X}h");
        }
        let words: [(u16, u16); 11] = [
            (0x1B02, 0x1501),
            (0xA7F7, 0x2101),
            (0xA7F9, 0x2202),
            (0xA7DB, 0x2303),
            (0x07E8, 0x2404),
            (0x07EE, 0x2505),
            (0xA7DD, 0x5101),
            (0xA7DF, 0x5202),
            (0xA7E1, 0x5303),
            (0xA7E3, 0x5404),
            (0xA7E5, 0x5505),
        ];
        for (at, want) in words {
            assert_eq!(ds.r16(at), want, "{at:04X}h");
        }
        // 輸入：最後的掃描碼 A86Ch、搖桿 0EECh；開火旗標玩家 1 在 A7E7h（新按）、A7E9h（事件）、A7EAh（轉向），
        // 玩家 2 在 A7E8h、A7EBh、A7ECh。
        let inputs: [(u16, u8); 8] =
            [(0xA86C, 0x31), (0x0EEC, 0x32), (0xA7E7, 0x41), (0xA7E8, 0x42), (0xA7E9, 0x43), (0xA7EB, 0x44), (0xA7EA, 0x45), (0xA7EC, 0x46)];
        for (at, want) in inputs {
            assert_eq!(ds.r8(at), want, "{at:04X}h");
        }
        // 寫了的位元組數剛好等於各欄位寬度的總和（值的每個位元組都不是 0）：沒有欄位重疊，也沒有碰其他位元組。
        let touched = ds.bytes().iter().filter(|&&b| b != 0).count();
        assert_eq!(touched, 4 + 2 + 5 * 2 + 2 + 6 + 5 * 2);
    }

    #[test]
    fn scenes_mirror_is_the_inverse_of_load_for_any_dump() {
        let ds = Dseg::from_image(&pseudo_random_image());
        let mut scenes = Scenes::default();
        scenes.load(&ds);
        let mut same = ds.clone();
        scenes.store(&mut same);
        assert_eq!(same.bytes(), ds.bytes(), "store(load(ds)) 要等於 ds");
        let mut blank = blank_ds();
        scenes.store(&mut blank);
        let mut back = Scenes::default();
        back.load(&blank);
        assert_eq!(back, scenes);
        assert_ne!(scenes, Scenes::default(), "測試資料不該全 0");
    }

    #[test]
    fn scene_fields_land_at_the_documented_offsets() {
        let mut scenes = Scenes::default();
        scenes.stage1 = Stage1Backdrop { drop: 0x0101, sky_drop: 0x0202, wave_phase: 0x0303, rock_phase: 0x0404 };
        scenes.stage2 = Stage2Backdrop {
            flame_bottom: 0x1101,
            flame_top: 0x1202,
            flame_phase: 0x1303,
            strip_a: Strip { phase: 0x1404, start: 0x1505 },
            strip_b: Strip { phase: 0x1606, start: 0x1707 },
            strip_c: Strip { phase: 0x1808, start: 0x1909 },
            pillar: 0x1A0A,
            fade: std::array::from_fn(|k| 0x40 + k as u8),
        };
        scenes.map = MapScroll { window: 0x2101, left_col: 0x2202, top_row: 0x2303 };
        scenes.tunnel = Tunnel { offset: 0x3101, floor_phase: 0x3202, ceiling_phase: 0x3303, script_pos: 0x3404 };
        for (k, layer) in scenes.boss.layers.iter_mut().enumerate() {
            let k = k as u16;
            *layer = BossLayer { y: 0x4101 + 0x300 * k, x: 0x4202 + 0x300 * k, script: 0x4303 + 0x300 * k };
        }
        let mut ds = blank_ds();
        scenes.store(&mut ds);
        let words: [(u16, u16); 4 + 10 + 3 + 4 + 9] = [
            (0x1AF8, 0x0101),
            (0x1AFA, 0x0202),
            (0x1AFC, 0x0303),
            (0x1AFE, 0x0404),
            (0x0BFE, 0x1101),
            (0x0C00, 0x1202),
            (0x0C02, 0x1303),
            (0x0C04, 0x1404),
            (0x0C06, 0x1505),
            (0x0C08, 0x1606),
            (0x0C0A, 0x1707),
            (0x0C0C, 0x1808),
            (0x0C0E, 0x1909),
            (0x0C10, 0x1A0A),
            (0x9004, 0x2101),
            (0x9006, 0x2202),
            (0x9008, 0x2303),
            (0x0D5D, 0x3101),
            (0x0D5F, 0x3202),
            (0x0D61, 0x3303),
            (0x0DE2, 0x3404),
            (0x0F9F, 0x4101),
            (0x0FA1, 0x4202),
            (0x0FAD, 0x4303),
            (0x0FA3, 0x4401),
            (0x0FA5, 0x4502),
            (0x0FAF, 0x4603),
            (0x0FA7, 0x4701),
            (0x0FA9, 0x4802),
            (0x0FB1, 0x4903),
        ];
        for (at, want) in words {
            assert_eq!(ds.r16(at), want, "{at:04X}h");
        }
        // 淡出工作表在 0CA7h 起 45 位元組。
        for k in 0..45u16 {
            assert_eq!(ds.r8(0x0CA7 + k), 0x40 + k as u8, "工作表第 {k} 個位元組");
        }
        assert_eq!(ds.r8(0x0CA7 + 45), 0, "工作表之後不動");
        // 位元組數：4 + 10 個字組 + 45 + 3 + 4 + 9 個字組（0x40 起的 45 個值都不是 0）。
        let touched = ds.bytes().iter().filter(|&&b| b != 0).count();
        assert_eq!(touched, 2 * (4 + 10 + 3 + 4 + 9) + 45);
    }

    #[test]
    fn the_stage_work_variables_that_left_the_structs_are_not_written_by_store() {
        // 這些位移在原版是函式內先寫後讀的工作變數（DS_EXEMPT），或是 G3 越界的字組，Mirror 不碰。
        let mut ds = Dseg::from_image(&vec![0xAAu8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000]);
        let mut scenes = Scenes::default();
        scenes.boss.layers[0].y = 0xFFFF;
        scenes.store(&mut ds);
        for off in [0x0F9A, 0x0F9B, 0x0F9C, 0x0F9D, 0x0F9E, 0x0FAB, 0x0FAC, 0x900A, 0x900B, 0x9C8C, 0x9C8D] {
            assert_eq!(ds.r8(off), 0xAA, "{off:04X}h 不屬於結構");
        }
    }

    #[test]
    fn palette_attribute_tables_sit_11h_before_the_dac_tables_except_the_loader() {
        for p in [Palette::Stage1, Palette::Stage1Boss, Palette::Stage2, Palette::Stage2Mid, Palette::Stage2Boss, Palette::Stage2Fade, Palette::Stage3, Palette::Stage4] {
            assert_eq!(p.attr_offset(), p.table_offset() - 0x11);
        }
        assert_eq!((Palette::Loader.table_offset(), Palette::Loader.attr_offset()), (0x05A9, 0x056E));
        assert_eq!(
            [Palette::Stage1, Palette::Stage1Boss, Palette::Stage2, Palette::Stage2Mid, Palette::Stage2Boss, Palette::Stage3, Palette::Stage4].map(Palette::table_offset),
            [0x04FD, 0x053E, 0x0C25, 0x0C66, 0x0CE8, 0x0D29, 0x0D82]
        );
    }

    #[test]
    fn dos_bookkeeping_follows_the_original_message_and_segment_chain() {
        let mut c = core();
        c.ds.w16(0x039C, 0x4989);
        c.dos_allocate_buffers(StageBuffers::Stage2);
        // 段值：第一塊維持現值，之後每塊接在前一塊後面，中間隔一個段落。
        let want: Vec<u16> = [0x600u16, 0x600, 0x600, 0x600, 0x600, 0x10, 0x10, 0x10, 0x10, 1]
            .iter()
            .scan(0x4989u16, |seg, &para| {
                let this = *seg;
                *seg += para + 1;
                Some(this)
            })
            .collect();
        for (k, &seg) in want.iter().enumerate() {
            assert_eq!(c.ds.r16(0x039C + 2 * k as u16), seg, "第 {k} 塊");
        }
        // 訊息前綴：A+、B 與計數 30h 加 10 次呼叫。
        assert_eq!([c.ds.r8(0x0336), c.ds.r8(0x0337), c.ds.r8(0x0338)], [0x41, 0x2B, 0x3A]);
        assert_eq!([c.ds.r8(0x0348), c.ds.r8(0x0349), c.ds.r8(0x034A)], [0x41, 0x2B, 0x3A]);
        c.dos_free_buffers(StageBuffers::Stage2);
        assert_eq!([c.ds.r8(0x0336), c.ds.r8(0x0337), c.ds.r8(0x0338)], [0x42, 0x2D, 0x3A]);
        // 第四關釋放不重寫前綴，計數接著加。
        c.dos_allocate_buffers(StageBuffers::Stage4);
        assert_eq!([c.ds.r8(0x0336), c.ds.r8(0x0337), c.ds.r8(0x0338)], [0x41, 0x34, 0x3A]);
        c.dos_free_buffers(StageBuffers::Stage4);
        assert_eq!(c.ds.r8(0x0338), 0x44);
        // 第三關只動前綴與計數，不動段值。
        c.ds.w16(0x039C, 0x1111);
        c.dos_allocate_buffers(StageBuffers::Stage3);
        assert_eq!(c.ds.r16(0x039C), 0x1111);
        assert_eq!(c.ds.r8(0x0338), 0x3A);
    }

    #[test]
    fn the_integrity_sum_adds_400_words_from_7f4h_to_the_seed() {
        let mut c = core();
        assert_eq!(c.integrity_table_sum(), 0x64, "全 0 的表只剩初值");
        c.ds.w16(0x07F4, 1);
        c.ds.w16(0x07F4 + 2 * 399, 2);
        c.ds.w16(0x07F4 + 2 * 400, 0x5555);
        assert_eq!(c.integrity_table_sum(), 0x64 + 3, "第 400 個字組之後不算");
    }

    #[test]
    fn the_ascii_table_runs_into_the_key_table_past_scan_code_5dh() {
        let mut c = core();
        c.ds.w8(0xA86D + 0x1E, b'a');
        assert_eq!(c.ascii_of(0x1E), b'a');
        // 掃描碼 5Eh 起讀到的是按鍵狀態表（0A8CBh）的前幾項。
        c.note_key_down(0);
        c.note_key_up(1);
        assert_eq!([c.ascii_of(0x5E), c.ascii_of(0x5F)], [1, 2]);
    }

    #[test]
    fn terrain_tiles_go_into_the_documented_cells() {
        let mut c = core();
        c.fill_terrain_cell(0, 0x0A0B);
        c.fill_terrain_cell(41, 0x0C0D);
        assert_eq!((c.ds.r16(GRID4_START + 2), c.ds.r16(GRID4_START + 2 + 82)), (0x0A0B, 0x0C0D));
        c.start_terrain_column();
        assert_eq!(c.terrain_cursor(), TERRAIN_ROW_BYTES);
        c.store_terrain_tile(0x1234);
        c.store_terrain_tile(0x5678);
        assert_eq!((c.ds.r16(GRID4_START + 0x50), c.ds.r16(GRID4_START + 0xA0)), (0x1234, 0x5678));
        assert_eq!(c.terrain_cursor(), 0xF0);
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
