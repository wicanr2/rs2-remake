//! 遊戲狀態與原版 64 KB 資料段之間的對應（`Mirror`）。原版位移只存在這裡與測試裡，遊戲 crate 不認識它們。
//!
//! 全流程對拍要逐位元組比資料段，所以每個領域實作 [`Layout`]：
//!
//! - `load`：從資料段傾印讀入這個領域的全部狀態（快照、poke 用）。
//! - `store`：把這個領域的全部狀態寫回原版位移（對拍比較用）。
//!
//! [`Mirror`] 持有原版映像的資料段當底：傾印 = 映像加上全部領域的 `store`，所以遊戲沒有搬進狀態的位元組
//! （唯讀表格、堆疊殘值等）維持映像的值。
//!
//! 規則：
//!
//! - `load` 與 `store` 互為反函數：任何資料段內容讀進再寫回，位元組不變（池指標只認 0 與否，記錄位址由槽號決定）。
//!   所以結構的欄位用原版的字組寬度，不做「合理化」的轉換。
//! - 領域內先寫後讀的工作變數不進狀態，位移列進 [`DS_EXEMPT`] 並寫明讀寫它的函式。
//! - 池指標（敵人、敵彈、玩家彈）在遊戲裡是槽的記號，原版是記錄的資料段位址；這裡換算。

use rs2_game::boss_scroll::{BossLayer, BossLayers};
use rs2_game::ebullet::{self, Bullet, EnemyBullets};
use rs2_game::entity::{self, Enemies, Enemy, Sprite};
use rs2_game::flow::{Cheat, Clock, Controls, DifficultyBonus, DosBook, FireFlags, Flow, KeyTable, Scenes};
use rs2_game::fx::{Effect, Effects};
use rs2_game::grid::Grids;
use rs2_game::loader::{DEMO_BYTES, DemoTape};
use rs2_game::menu::Menu;
use rs2_game::pbullet::{self, PlayerBullet, PlayerBullets};
use rs2_game::player::{PLAYER_COUNT, Player, Players, Pods};
use rs2_game::scenery::{Prop, Scenery};
use rs2_game::sound::{Sound, SpeakerParams};
use rs2_game::stage1::Backdrop as Stage1Backdrop;
use rs2_game::stage2::{Backdrop as Stage2Backdrop, Strip};
use rs2_game::stage3::MapScroll;
use rs2_game::stage4::Tunnel;
use rs2_game::title::Title;
use rs2_game::video::DrawParams;
use rs2_game::{State, ebullet::Slot as EbSlot, entity::Slot as EnSlot, pbullet::Slot as PbSlot};
use rs2_formats::DSEG_IMAGE_OFFSET;

/// IDA 線性位址換成 `dseg` 偏移：`dseg:X` = 線性 `1CBC0h + X`。
pub const fn a(linear: u32) -> u16 {
    (linear - 0x1CBC0) as u16
}

/// 平坦的 64 KB 資料段。
#[derive(Clone)]
pub struct Dseg {
    mem: Box<[u8; 0x10000]>,
}

impl Dseg {
    /// 取解包映像位移 `0CBC0h` 起到檔尾，其餘補零。
    pub fn from_image(image: &[u8]) -> Self {
        let mut mem: Box<[u8; 0x10000]> = vec![0u8; 0x10000].into_boxed_slice().try_into().unwrap();
        let src = &image[DSEG_IMAGE_OFFSET..];
        let n = src.len().min(0x10000);
        mem[..n].copy_from_slice(&src[..n]);
        Dseg { mem }
    }

    pub fn r8(&self, o: u16) -> u8 {
        self.mem[o as usize]
    }

    pub fn w8(&mut self, o: u16, v: u8) {
        self.mem[o as usize] = v;
    }

    /// 小端序，偏移環繞。
    pub fn r16(&self, o: u16) -> u16 {
        u16::from_le_bytes([self.mem[o as usize], self.mem[o.wrapping_add(1) as usize]])
    }

    pub fn w16(&mut self, o: u16, v: u16) {
        let [lo, hi] = v.to_le_bytes();
        self.mem[o as usize] = lo;
        self.mem[o.wrapping_add(1) as usize] = hi;
    }

    pub fn bytes(&self) -> &[u8] {
        &self.mem[..]
    }

    /// 載入一份 64 KB 傾印。
    pub fn load(&mut self, bytes: &[u8]) {
        self.mem.copy_from_slice(&bytes[..0x10000]);
    }
}

/// 一個領域的狀態與資料段之間的對應。
pub trait Layout {
    /// 從資料段傾印讀入這個領域的全部狀態。
    fn load(&mut self, ds: &Dseg);
    /// 把這個領域的全部狀態寫回原版位移。
    fn store(&self, ds: &mut Dseg);
}

/// 狀態與原版資料段的對應：持有映像的資料段當底。
#[derive(Clone)]
pub struct Mirror {
    base: Dseg,
}

impl Mirror {
    /// `image` 是 `S.EXE` 的解包映像。
    pub fn new(image: &[u8]) -> Mirror {
        Mirror { base: Dseg::from_image(image) }
    }

    /// 以一份 64 KB 傾印當底。單色收據的入口傾印帶著單色模式的啟動結果（顯示列位址表、顯示模式參數），
    /// 這些位元組遊戲不建模，維持入口的值。
    pub fn from_dump(dump: &[u8]) -> Mirror {
        let mut base = Dseg { mem: vec![0u8; 0x10000].into_boxed_slice().try_into().unwrap() };
        base.load(dump);
        Mirror { base }
    }

    /// 底（遊戲沒有建模的位元組）。
    pub fn base(&self) -> &Dseg {
        &self.base
    }

    /// 原版這時的 64 KB 資料段：映像加上狀態的全部欄位。
    pub fn dump(&self, state: &State) -> Dseg {
        let mut ds = self.base.clone();
        state.fx.store(&mut ds);
        state.eb.store(&mut ds);
        state.pb.store(&mut ds);
        state.en.store(&mut ds);
        state.sc.store(&mut ds);
        state.pl.store(&mut ds);
        state.title.store(&mut ds);
        state.flow.store(&mut ds);
        state.scenes.store(&mut ds);
        state.draw_params.store(&mut ds);
        state.sound.store(&mut ds);
        state.grids.store(&mut ds);
        state.tape.store(&mut ds);
        ds
    }

    /// 從資料段傾印讀出狀態。
    pub fn load(&self, ds: &Dseg) -> State {
        let mut s = State::default();
        s.fx.load(ds);
        s.eb.load(ds);
        s.pb.load(ds);
        s.en.load(ds);
        s.sc.load(ds);
        s.pl.load(ds);
        s.title.load(ds);
        s.flow.load(ds);
        s.scenes.load(ds);
        s.draw_params.load(ds);
        s.sound.load(ds);
        s.grids.load(ds);
        s.tape.load(ds);
        s
    }

    /// 從 64 KB 的傾印檔讀出狀態。
    pub fn load_bytes(&self, dump: &[u8]) -> State {
        let mut ds = self.base.clone();
        ds.load(dump);
        self.load(&ds)
    }

    /// 在資料段寫一個位元組再讀回狀態（測試與對拍的 poke）。
    /// 位移不在狀態裡（唯讀表格、工作變數）就 `panic`：寫了讀不回來的 poke 等於沒套用，不能默默略過。
    pub fn poke8(&self, state: &mut State, offset: u16, value: u8) {
        assert!(state.grids.cols() == 40, "poke 只用在 100% 的戰場：寬戰場的格網不在原版資料段，讀回來會被重設");
        let mut ds = self.dump(state);
        ds.w8(offset, value);
        *state = self.load(&ds);
        assert!(self.dump(state).r8(offset) == value, "poke {offset:04X}h = {value:02X} 不在狀態裡，讀不回來");
    }

    /// 同 [`Mirror::poke8`]，寫一個字組。
    pub fn poke16(&self, state: &mut State, offset: u16, value: u16) {
        assert!(state.grids.cols() == 40, "poke 只用在 100% 的戰場：寬戰場的格網不在原版資料段，讀回來會被重設");
        let mut ds = self.dump(state);
        ds.w16(offset, value);
        *state = self.load(&ds);
        assert!(self.dump(state).r16(offset) == value, "poke {offset:04X}h = {value:04X} 不在狀態裡，讀不回來");
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

impl Layout for Effects {
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

const _: () = assert!(rs2_game::fx::POOL_SIZE == 30, "池大小與原版三個陣列的長度一致");

// ---- 敵方子彈池 ----

/// 池指標表，20 個字組（`32B3h`）。0 是空槽，使用中是該槽記錄的位址。
const EB_LINKS: u16 = 0x32B3;
/// 記錄區，20 筆連續的 15 字組記錄（`32DBh` 起，每筆 1Eh 位元組）。
const EB_RECORDS: u16 = 0x32DB;
const EB_RECORD_BYTES: u16 = 0x1E;

const _: () = assert!(ebullet::POOL_SIZE == 20, "池大小與原版指標表的長度一致");
const _: () = assert!(EB_RECORDS == EB_LINKS + 2 * ebullet::POOL_SIZE as u16, "記錄區緊接在指標表之後");

/// 第 `slot` 槽記錄的位址，也就是使用中的槽寫進池指標表的值。
const fn eb_record(slot: usize) -> u16 {
    EB_RECORDS + EB_RECORD_BYTES * slot as u16
}

/// 從資料段讀一筆記錄（15 個字組）。
fn read_bullet(ds: &Dseg, at: u16) -> Bullet {
    Bullet::from_words(std::array::from_fn(|k| ds.r16(at.wrapping_add(2 * k as u16))))
}

/// 池指標表的項：0 是空槽，使用中是該槽記錄的位址，遊戲裡換成槽的記號。
fn token(raw: u16, record_link: u16) -> u16 {
    if raw != 0 { record_link } else { 0 }
}

impl Layout for EnemyBullets {
    fn load(&mut self, ds: &Dseg) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            *slot = EbSlot { link: token(ds.r16(EB_LINKS + 2 * i as u16), ebullet::record_link(i)), bullet: read_bullet(ds, eb_record(i)) };
        }
    }

    fn store(&self, ds: &mut Dseg) {
        for (i, slot) in self.slots.iter().enumerate() {
            ds.w16(EB_LINKS + 2 * i as u16, if slot.link != 0 { eb_record(i) } else { 0 });
            for (k, w) in slot.bullet.to_words().into_iter().enumerate() {
                ds.w16(eb_record(i) + 2 * k as u16, w);
            }
        }
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

const _: () = assert!(pbullet::POOL_SIZE == 20, "池大小與原版指標表的長度一致");
const _: () = assert!(PB_RECORDS == PB_LINKS + 2 * pbullet::POOL_SIZE as u16, "記錄區緊接在指標表之後");

/// 第 `slot` 槽記錄的位址，也就是使用中的槽寫進池指標表的值。
const fn pb_record(slot: usize) -> u16 {
    PB_RECORDS + PB_RECORD_BYTES * slot as u16
}

/// 從資料段讀一筆記錄（16 個字組）。
fn read_player_bullet(ds: &Dseg, at: u16) -> PlayerBullet {
    PlayerBullet::from_words(std::array::from_fn(|k| ds.r16(at.wrapping_add(2 * k as u16))))
}

impl Layout for PlayerBullets {
    fn load(&mut self, ds: &Dseg) {
        self.seekers_in_flight = ds.r16(PB_SEEKERS);
        for (i, slot) in self.slots.iter_mut().enumerate() {
            *slot = PbSlot { link: token(ds.r16(PB_LINKS + 2 * i as u16), pbullet::pb_record_link(i)), bullet: read_player_bullet(ds, pb_record(i)) };
        }
    }

    fn store(&self, ds: &mut Dseg) {
        ds.w16(PB_SEEKERS, self.seekers_in_flight);
        for (i, slot) in self.slots.iter().enumerate() {
            ds.w16(PB_LINKS + 2 * i as u16, if slot.link != 0 { pb_record(i) } else { 0 });
            for (k, w) in slot.bullet.to_words().into_iter().enumerate() {
                ds.w16(pb_record(i) + 2 * k as u16, w);
            }
        }
    }
}

// ---- 敵人池 ----

/// 池指標表（`2294h`）：15 槽加一個只會被清零、沒有人讀的備用字組，共 16 個字組。
/// 0 是空槽，使用中是該槽記錄的位址。
const EN_LINKS: u16 = 0x2294;
const EN_SPARE_LINK: u16 = EN_LINKS + 2 * entity::POOL_SIZE as u16;
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

const _: () = assert!(entity::POOL_SIZE == 15, "池大小與原版指標表的長度一致");
const _: () = assert!(EN_RECORDS == EN_SPARE_LINK + 2, "記錄區緊接在指標表之後");
const _: () = assert!(EN_SPRITE_IMAGE == ENEMY_DEATH_FLAG + 2, "繪製陣列緊接在死亡旗標之後");
const _: () = assert!(EN_SPRITE_X == EN_SPRITE_IMAGE + 0x20 && EN_SPRITE_Y == EN_SPRITE_X + 0x20, "繪製陣列各 16 個字組");
const _: () = assert!(EN_SPRITE_KIND == EN_SPRITE_Y + 0x20 && EN_FLASH == EN_SPRITE_KIND + 0x20, "繪製陣列各 16 個字組");
const _: () = assert!(EN_LAST_SPRITE == EN_FLASH + 0x20 && EN_LINKS == EN_LAST_SPRITE + 8, "繪製值緊接在閃爍陣列之後，指標表緊接在它之後");

/// 死亡旗標 `word_1EDAA`（`21EAh`）。它的位址同時是列起點表的越界列（見 `rs2-game` 的 `grid.rs`）。
const ENEMY_DEATH_FLAG: u16 = a(0x1EDAA);
/// 傷害寫到指標 0 加 20h 的字組（`dseg:0020h`）。
const ENEMY_STRAY_DAMAGE: u16 = 0x0020;

/// 第 `slot` 槽記錄的位址，也就是使用中的槽寫進池指標表的值。
const fn enemy_record(slot: usize) -> u16 {
    EN_RECORDS + EN_RECORD_BYTES * slot as u16
}

/// 從資料段讀一筆記錄（24 個字組）。
fn read_enemy(ds: &Dseg, at: u16) -> Enemy {
    Enemy::from_words(std::array::from_fn(|k| ds.r16(at.wrapping_add(2 * k as u16))))
}

impl Layout for Enemies {
    fn load(&mut self, ds: &Dseg) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let o = 2 * i as u16;
            *slot = EnSlot {
                link: token(ds.r16(EN_LINKS + o), entity::enemy_record_link(i)),
                enemy: read_enemy(ds, enemy_record(i)),
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
        self.death_flag = ds.r16(ENEMY_DEATH_FLAG);
        self.stray_damage = ds.r16(ENEMY_STRAY_DAMAGE);
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
            ds.w16(EN_LINKS + o, if slot.link != 0 { enemy_record(i) } else { 0 });
            for (k, w) in slot.enemy.to_words().into_iter().enumerate() {
                ds.w16(enemy_record(i) + 2 * k as u16, w);
            }
            ds.w16(EN_SPRITE_IMAGE + o, slot.sprite.image);
            ds.w16(EN_SPRITE_X + o, slot.sprite.x);
            ds.w16(EN_SPRITE_Y + o, slot.sprite.y);
            ds.w16(EN_SPRITE_KIND + o, slot.sprite.kind);
            ds.w16(EN_FLASH + o, slot.flash);
        }
        ds.w16(EN_SPARE_LINK, self.spare_link);
        ds.w16(ENEMY_DEATH_FLAG, self.death_flag);
        ds.w16(ENEMY_STRAY_DAMAGE, self.stray_damage);
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

const _: () = assert!(rs2_game::scenery::POOL_SIZE == 10, "池大小與原版四個陣列的長度一致");
const _: () = assert!(SC_X == SC_DX + 2 * rs2_game::scenery::POOL_SIZE as u16 && SC_Y == SC_X + 2 * rs2_game::scenery::POOL_SIZE as u16, "四個陣列連續");
const _: () = assert!(SC_IMAGE == SC_Y + 2 * rs2_game::scenery::POOL_SIZE as u16 && SC_GROUPS == SC_IMAGE + 2 * rs2_game::scenery::POOL_SIZE as u16, "群組表緊接在陣列之後");

impl Layout for Scenery {
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

impl Layout for Players {
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

// ---- 關卡流程 ----

/// 關卡字母 `byte_1CFDB`、過關旗標 `byte_1CFAD`、連爆輪數 `byte_1CFAE`、完整性記號 `byte_1CFAA`（皆 1 位元組）。
const FLOW_STAGE: u16 = a(0x1CFDB);
const FLOW_CLEAR: u16 = a(0x1CFAD);
const FLOW_BURSTS: u16 = a(0x1CFAE);
const FLOW_INTEGRITY_MARK: u16 = a(0x1CFAA);
/// `word_1B02`。
const FLOW_TAMPER_COUNTDOWN: u16 = 0x1B02;
/// 關卡的幀計數 `word_1CFE2`、展示旗標 `byte_1CFDC`、`byte_1CFAB`（標題迴圈來的）、`byte_1CFAC`（標題迴圈開始）。
const FLOW_FRAME: u16 = a(0x1CFE2);
const FLOW_DEMO: u16 = a(0x1CFDC);
const FLOW_FROM_TITLE: u16 = a(0x1CFAB);
const FLOW_TITLE_ENTERED: u16 = a(0x1CFAC);
/// 密碼檢查的兩個旗標：通過 `byte_1D3A2`、未通過 `word_1D3B0`。
const FLOW_PASSWORD_ACCEPTED: u16 = a(0x1D3A2);
const FLOW_PASSWORD_REJECTED: u16 = a(0x1D3B0);
/// 換磁片提示的磁碟號 `byte_274EC`。
const FLOW_DISK_HINT: u16 = a(0x274EC);
/// 難度加成：耐久 `word_1E6B2`、開火 `word_1E6B4`。
const DIFFICULTY_HP: u16 = a(0x1E6B2);
const DIFFICULTY_FIRE: u16 = a(0x1E6B4);
/// DOS 配置簿記：錯誤訊息的兩個三字元前綴（`byte_1CEF6` 起與 `byte_1CF08` 起）、十個段值（`word_1CF5C` 起）。
const DOS_MESSAGE_A: u16 = a(0x1CEF6);
const DOS_MESSAGE_B: u16 = a(0x1CF08);
const DOS_SEGMENTS: u16 = a(0x1CF5C);
/// 按鍵狀態表，`2748Bh + 掃描碼`，每鍵一個位元組。最近按下的鍵（ASCII）`byte_27422`、音效開關鍵旗標 `byte_2739A`。
const CONTROLS_KEYS: u16 = a(0x2748B);
const CONTROLS_LAST_KEY: u16 = a(0x27422);
const CONTROLS_SOUND_KEY: u16 = a(0x2739A);
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

impl Layout for Flow {
    fn load(&mut self, ds: &Dseg) {
        self.stage = ds.r8(FLOW_STAGE);
        self.clear = ds.r8(FLOW_CLEAR);
        self.bursts = ds.r8(FLOW_BURSTS);
        self.integrity_mark = ds.r8(FLOW_INTEGRITY_MARK);
        self.tamper_countdown = ds.r16(FLOW_TAMPER_COUNTDOWN);
        self.frame = ds.r16(FLOW_FRAME);
        self.demo = ds.r8(FLOW_DEMO);
        self.from_title = ds.r8(FLOW_FROM_TITLE);
        self.title_entered = ds.r8(FLOW_TITLE_ENTERED);
        self.password_accepted = ds.r8(FLOW_PASSWORD_ACCEPTED);
        self.password_rejected = ds.r16(FLOW_PASSWORD_REJECTED);
        self.disk_hint = ds.r8(FLOW_DISK_HINT);
        self.difficulty = DifficultyBonus { hp: ds.r16(DIFFICULTY_HP), fire: ds.r16(DIFFICULTY_FIRE) };
        self.dos = DosBook {
            message_a: std::array::from_fn(|k| ds.r8(DOS_MESSAGE_A + k as u16)),
            message_b: std::array::from_fn(|k| ds.r8(DOS_MESSAGE_B + k as u16)),
            segments: std::array::from_fn(|k| ds.r16(DOS_SEGMENTS + 2 * k as u16)),
        };
        self.clock = Clock {
            total: ds.r16(CLOCK_TOTAL),
            since_wait: ds.r16(CLOCK_SINCE_WAIT),
            since_sound_toggle: ds.r16(CLOCK_SINCE_SOUND_TOGGLE),
            cycle_a: ds.r16(CLOCK_CYCLE_A),
            cycle_b: ds.r16(CLOCK_CYCLE_B),
        };
        self.controls = Controls {
            keys: KeyTable(std::array::from_fn(|k| ds.r8(CONTROLS_KEYS + k as u16))),
            last_key: ds.r8(CONTROLS_LAST_KEY),
            sound_key: ds.r8(CONTROLS_SOUND_KEY),
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
        ds.w16(FLOW_FRAME, self.frame);
        ds.w8(FLOW_DEMO, self.demo);
        ds.w8(FLOW_FROM_TITLE, self.from_title);
        ds.w8(FLOW_TITLE_ENTERED, self.title_entered);
        ds.w8(FLOW_PASSWORD_ACCEPTED, self.password_accepted);
        ds.w16(FLOW_PASSWORD_REJECTED, self.password_rejected);
        ds.w8(FLOW_DISK_HINT, self.disk_hint);
        ds.w16(DIFFICULTY_HP, self.difficulty.hp);
        ds.w16(DIFFICULTY_FIRE, self.difficulty.fire);
        for (k, &v) in self.dos.message_a.iter().enumerate() {
            ds.w8(DOS_MESSAGE_A + k as u16, v);
        }
        for (k, &v) in self.dos.message_b.iter().enumerate() {
            ds.w8(DOS_MESSAGE_B + k as u16, v);
        }
        for (k, &v) in self.dos.segments.iter().enumerate() {
            ds.w16(DOS_SEGMENTS + 2 * k as u16, v);
        }
        ds.w16(CLOCK_TOTAL, self.clock.total);
        ds.w16(CLOCK_SINCE_WAIT, self.clock.since_wait);
        ds.w16(CLOCK_SINCE_SOUND_TOGGLE, self.clock.since_sound_toggle);
        ds.w16(CLOCK_CYCLE_A, self.clock.cycle_a);
        ds.w16(CLOCK_CYCLE_B, self.clock.cycle_b);
        let c = &self.controls;
        for (k, &v) in c.keys.0.iter().enumerate() {
            ds.w8(CONTROLS_KEYS + k as u16, v);
        }
        ds.w8(CONTROLS_LAST_KEY, c.last_key);
        ds.w8(CONTROLS_SOUND_KEY, c.sound_key);
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

impl Layout for Scenes {
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

impl Layout for Title {
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

// ---- 繪製參數 ----

/// 目的欄 `word_26880`、目的像素列 `word_26882`、圖庫索引 `word_26888`。
const DRAW_X: u16 = a(0x26880);
const DRAW_Y: u16 = a(0x26882);
const DRAW_INDEX: u16 = a(0x26888);
/// 繪製頁的偏移 `word_26872`、暫存區選擇 `word_2685C`。
const DRAW_PAGE: u16 = a(0x26872);
const DRAW_SCRATCH: u16 = a(0x2685C);
/// 暫存區複製的來源欄 `word_26864`、來源列 `word_26866`、水平位移 `word_26868`。
const DRAW_SOURCE_COL: u16 = a(0x26864);
const DRAW_SOURCE_ROW: u16 = a(0x26866);
const DRAW_SHIFT: u16 = a(0x26868);
/// 數字的欄 `word_2688A` 與列 `word_2688E`。
const DRAW_TEXT_COL: u16 = a(0x2688A);
const DRAW_TEXT_ROW: u16 = a(0x2688E);
/// 形狀寬 `word_2687E`。
const DRAW_SHAPE_WIDTH: u16 = a(0x2687E);
/// 形狀寬字組在「上一個繪製命令之後邏輯沒有寫過」時的值。形狀寬不會是這個值。
/// 繪製對這個字的寫入在接收端，核心不會更新，留記號才分得出之後邏輯有沒有再寫（`DrawParams::shape_width`）。
const SHAPE_WIDTH_UNTOUCHED: u16 = 0xFFFF;

/// 資料段上的形狀寬字組換成狀態。
fn shape_width_from_word(word: u16) -> Option<u16> {
    (word != SHAPE_WIDTH_UNTOUCHED).then_some(word)
}

/// 全部繪製參數都是狀態：邏輯只改寫要變的幾個，其餘沿用上一次的值，被繪製命令帶走（`video.rs`）。
/// 下面的位移在 `DS_EXEMPT` 的範圍內（`9CB0..9CB3` 頁、`9CBC..9CC8` 形狀寬與目的位置與索引的低位元組、
/// `9CCA` 數字欄的低位元組），沒有豁免、逐位元組比對的是 `word_2685C`、`26864`、`26866`、`26868`、`2688E`
/// 與 `26888`、`2688A` 的高位元組。
impl Layout for DrawParams {
    fn load(&mut self, ds: &Dseg) {
        *self = DrawParams {
            x: ds.r16(DRAW_X),
            y: ds.r16(DRAW_Y),
            index: ds.r16(DRAW_INDEX),
            page: ds.r16(DRAW_PAGE),
            scratch: ds.r16(DRAW_SCRATCH),
            source_col: ds.r16(DRAW_SOURCE_COL),
            source_row: ds.r16(DRAW_SOURCE_ROW),
            shift: ds.r16(DRAW_SHIFT),
            text_col: ds.r16(DRAW_TEXT_COL),
            text_row: ds.r16(DRAW_TEXT_ROW),
            shape_width: shape_width_from_word(ds.r16(DRAW_SHAPE_WIDTH)),
        };
    }

    fn store(&self, ds: &mut Dseg) {
        ds.w16(DRAW_X, self.x);
        ds.w16(DRAW_Y, self.y);
        ds.w16(DRAW_INDEX, self.index);
        ds.w16(DRAW_PAGE, self.page);
        ds.w16(DRAW_SCRATCH, self.scratch);
        ds.w16(DRAW_SOURCE_COL, self.source_col);
        ds.w16(DRAW_SOURCE_ROW, self.source_row);
        ds.w16(DRAW_SHIFT, self.shift);
        ds.w16(DRAW_TEXT_COL, self.text_col);
        ds.w16(DRAW_TEXT_ROW, self.text_row);
        ds.w16(DRAW_SHAPE_WIDTH, self.shape_width.unwrap_or(SHAPE_WIDTH_UNTOUCHED));
    }
}

// ---- 音效開關 ----

/// 音樂可用 `byte_1CBCE`、靜音 `byte_2684E`、喇叭音效旗標 `byte_2684F`（皆 1 位元組）。
const SOUND_MUSIC: u16 = a(0x1CBCE);
const SOUND_MUTE: u16 = a(0x2684E);
const SOUND_SPEAKER_FLAGS: u16 = a(0x2684F);
/// 喇叭參數：`word_26850` duration、`26852` period、`26854` threshold、`26856` pitch、`26858` counter。
const SPEAKER_DURATION: u16 = a(0x26850);
const SPEAKER_PERIOD: u16 = a(0x26852);
const SPEAKER_THRESHOLD: u16 = a(0x26854);
const SPEAKER_PITCH: u16 = a(0x26856);
const SPEAKER_COUNTER: u16 = a(0x26858);

impl Layout for Sound {
    fn load(&mut self, ds: &Dseg) {
        *self = Sound {
            music: ds.r8(SOUND_MUSIC),
            mute: ds.r8(SOUND_MUTE),
            speaker_flags: ds.r8(SOUND_SPEAKER_FLAGS),
            speaker: SpeakerParams {
                duration: ds.r16(SPEAKER_DURATION),
                period: ds.r16(SPEAKER_PERIOD),
                threshold: ds.r16(SPEAKER_THRESHOLD),
                pitch: ds.r16(SPEAKER_PITCH),
                counter: ds.r16(SPEAKER_COUNTER),
            },
        };
    }

    fn store(&self, ds: &mut Dseg) {
        ds.w8(SOUND_MUSIC, self.music);
        ds.w8(SOUND_MUTE, self.mute);
        ds.w8(SOUND_SPEAKER_FLAGS, self.speaker_flags);
        ds.w16(SPEAKER_DURATION, self.speaker.duration);
        ds.w16(SPEAKER_PERIOD, self.speaker.period);
        ds.w16(SPEAKER_THRESHOLD, self.speaker.threshold);
        ds.w16(SPEAKER_PITCH, self.speaker.pitch);
        ds.w16(SPEAKER_COUNTER, self.speaker.counter);
    }
}

// ---- 戰場格網 ----

/// G1 實體格（`1B32h`，20 列 × 40 欄）、G2 敵彈格（`2C1Ch`，同上加溢位格）。
const GRID1: u16 = 0x1B32;
const GRID2: u16 = 0x2C1C;
/// G4 地形格（`900Ch`）、G3 髒格（`964Ch`）與第三關新欄的寫入游標（`word_2684C`，`9C8Ch`）三塊相鄰，各 640h 位元組，游標 2 位元組。
const TERRAIN: u16 = 0x900C;

/// 戰場加大（規格 005「戰場大小」）時的格網比原版的格網大，原版資料段沒有對應的位置：傾印與載入都略過，
/// 不去蓋別的變數。要比對寬戰場的格網就比 [`Grids`] 的位元組本身（`Session::dseg_hash` 這樣做）。
impl Layout for Grids {
    fn load(&mut self, ds: &Dseg) {
        if self.cols() != 40 {
            return;
        }
        for (k, v) in self.g1.iter_mut().enumerate() {
            *v = ds.r8(GRID1 + k as u16);
        }
        for (k, v) in self.g2.iter_mut().enumerate() {
            *v = ds.r8(GRID2 + k as u16);
        }
        for (k, v) in self.terrain.iter_mut().enumerate() {
            *v = ds.r8(TERRAIN + k as u16);
        }
    }

    fn store(&self, ds: &mut Dseg) {
        if self.cols() != 40 {
            return;
        }
        for (k, &v) in self.g1.iter().enumerate() {
            ds.w8(GRID1 + k as u16, v);
        }
        for (k, &v) in self.g2.iter().enumerate() {
            ds.w8(GRID2 + k as u16, v);
        }
        for (k, &v) in self.terrain.iter().enumerate() {
            ds.w8(TERRAIN + k as u16, v);
        }
    }
}

// ---- 展示錄影 ----

/// 展示錄影區（`AAE2h` 起 3000 位元組，`DM1.LCK` 或 `DM2.LCK` 載入，非展示時玩家 1 錄製）。
const DEMO_TAPE: u16 = 0xAAE2;

impl Layout for DemoTape {
    fn load(&mut self, ds: &Dseg) {
        for (k, v) in self.0.iter_mut().enumerate() {
            *v = ds.r8(DEMO_TAPE + k as u16);
        }
    }

    fn store(&self, ds: &mut Dseg) {
        for (k, &v) in self.0.iter().enumerate() {
            ds.w8(DEMO_TAPE + k as u16, v);
        }
    }
}

const _: () = assert!(DEMO_BYTES == 3000);

// ---- 對拍的豁免範圍 ----

/// 第一幀的 `dseg` 差異豁免範圍（規格 001「初始狀態」），閉區間。
pub const A0_EXEMPT: &[(u16, u16)] = &[
    (0x000E, 0x000E),
    (0x0018, 0x0020),
    (0x0336, 0x034A),
    (0x0386, 0x03AF),
    (0x03D5, 0x03E3),
    (0x0586, 0x0592),
    (0x07E2, 0x07EE),
    (0x0EED, 0x0EFC),
    (0x9CB0, 0x9CB3),
    (0x9CBC, 0x9CC8),
    (0x9CE1, 0x9CE5),
    (0xA7DA, 0xA7DC),
    (0xA7F7, 0xA7F8),
    (0xA852, 0xA858),
    (0xA92A, 0xA931),
    (0xA94B, 0xA94C),
    (0xA9AB, 0xA9AB),
    (0xA8E7, 0xA8E7),
    (0xA8FA, 0xA8FA),
    (0xA904, 0xA904),
    (0xE340, 0xFFFF),
];

/// 全流程對拍（規格 002）的資料段豁免範圍，閉區間。起始內容是規格 001 的 `A0_EXEMPT`、
/// 差分模糊測試另外發現的三處、等待計時器。每一項寫明哪些函式讀寫它；新增的項目也要寫理由。
/// 收據（`wt_*`）與 B1 都沒有出現差異的 A0 項目已刪（`000Eh`、`0586h`～`0592h` 標題動畫計數、按鍵表 `A8E7h`、`A8FAh`、`A904h`），
/// `0336h`～`034Ah` 收成實際會變的兩段。`RS2_DS_REPORT=1` 跑 `tests/wait.rs` 會列出含豁免的全部差異，可重新檢查。
pub const DS_EXEMPT: &[(u16, u16, &str)] = &[
    // ---- A0_EXEMPT（規格 001「初始狀態」）----
    (0x0018, 0x0020, "文字畫面的游標與屬性；啟動函式專用"),
    (0x0336, 0x0338, "sub_113B2、sub_1149F 的工作變數；載入專用"),
    (0x0348, 0x034A, "sub_113B2、sub_1149F 的工作變數；載入專用"),
    (0x0386, 0x03AF, "sub_10162 的列位址；只有 MGA 分支 sub_1117F 讀"),
    (0x03D5, 0x03E3, "屬性暫存器工作表；調色盤由 vram／dac 雜湊比對涵蓋"),
    (0x07E2, 0x07EE, "密碼函式與 INT 8 的 word_1D3A8、word_1D3AE；byte_1D3A2 第二關才讀（130AFh），其餘屬密碼"),
    (0x0EED, 0x0EFC, "JOYMOUSE 輪詢值；byte_1DAAC = N 時不進按鍵表"),
    (0x9CB0, 0x9CB3, "圖庫指標與頁；繪製路徑先寫再讀"),
    (0x9CBC, 0x9CC8, "繪製參數；繪製路徑先寫再讀"),
    (0x9CE1, 0x9CE5, "數字緩衝；sub_1890D 先寫再讀"),
    (0xA7DA, 0xA7DC, "音樂驅動與 INT 8 的 word_2739B；音樂不在範圍"),
    (0xA7F7, 0xA7F8, "word_273B7，見下方計時器"),
    (0xA852, 0xA858, "舊中斷向量；啟動與離開專用"),
    (0xA92A, 0xA931, "檔名工作區；載入專用"),
    (0xA94B, 0xA94C, "檔名工作區；載入專用"),
    (0xA9AB, 0xA9AB, "檔名工作區；載入專用"),
    (0xE340, 0xFFFF, "堆疊殘值；映像外，邏輯不讀"),
    // ---- 差分模糊測試另外發現的（tests/fuzz.rs DS_EXTRA_EXEMPT）----
    (0x9C9E, 0x9CA3, "word_2685E、26860、26862 blit 暫存；只有 sub_19DA8、19E66、19F3B、19FF9、1A09F、1A144、1A1E6 讀寫"),
    (0x9CCA, 0x9CCA, "word_2688A 繪製參數；sub_1100A、1117F、162B1 等寫，sub_18993 與 sub_19A0B（blit）讀"),
    (0x9CD0, 0x9CD0, "byte_26890 數字繪製暫存；sub_1890D、18993、189C1、162B1 寫，sub_18A1F、18A61（數字繪製）讀"),
    // ---- 全流程對拍發現的（規格 002）----
    (0xAA2C, 0xAA2C, "檔名緩衝 SHT4.GO（AA25h，7 個字元加結尾）的第 8 個位元組。sub_1AEBC 經 sub_1C738（1C738h 起，加 3 後 mov [bx], dl）\
                      把它改成 'T' 以開 SHT4.GOT；只有檔案載入（sub_1C82C）讀。重製不碰檔名緩衝（檔案在啟動時已驗過）"),
    // ---- 等待計時器（規格 002）----
    (0xA7F7, 0xA7FA, "word_273B7、word_273B9 的 tick 計數。INT 8（1A7CEh、1A7D2h）加 1，只有 sub_11097（11097h、1109Dh）讀寫。\
                      原版在 1109Dh 取樣時 273B9 還沒歸零（值 >= cx），重製在 WaitDone 已歸零；輪詢迴圈裡 tick 數也不同"),
    // ---- 特效領域重寫（規格 004）：函式內先寫後讀的工作變數與參數，改成區域變數與函式參數，不再寫回資料段 ----
    // 查證：tools/q.sh refs word_20B74／20B76／20B78／20B80，另 grep 反組譯裡所有 3FB4h 至 3FC1h 的位移形式，沒有其他讀寫。
    (0x3FB4, 0x3FB5, "word_20B74 腳本指標暫存；只有 sub_17C6C 寫（17C6Ch）後讀（17C92h）"),
    (0x3FB6, 0x3FB7, "word_20B76 形狀指標；只有 sub_17CB6 寫（17D01h）、sub_17D2C 讀（17D2Ch）"),
    (0x3FB8, 0x3FB9, "word_20B78 特效字母；呼叫端（sub_10CD0、13205、13A51、13DAF、14679、147E1、151A7、1530F、15F8D、167E9、16946、173E1）寫，\
                      只有 sub_17A3A 讀（17A3Ah 起的比較）。重製改成 EffectKind 參數"),
    (0x3FC0, 0x3FC1, "word_20B80 只有 sub_17CB6 寫（17CC8h），沒有任何指令讀"),
    // ---- 敵彈領域重寫（規格 004）：函式內先寫後讀的工作變數與呼叫參數，改成區域變數與函式參數，不再寫回資料段 ----
    // 查證：tools/q.sh refs 與 dsrefs 列出每個變數的全部讀寫（結果記在 workspace/progress/rewrite-ebullet.md）。
    // 池指標表 32B3h、記錄區 32DBh 是狀態，走 Mirror，不在這裡。
    (0x2C02, 0x2C11, "word_1F7C2 起 8 個字組的轉向表；sub_1744A、1747E、174B2、174E6、1751A、1754E、17582、175B6 寫，只有 sub_175EA 讀。\
                      重製改成 TURN_TABLES 常數與 turn_toward 參數"),
    (0x2C12, 0x2C12, "byte_1F7D2 移動碼；sub_17631 寫（17634h），只有 sub_1779B 讀（177A1h）。重製改成 step_bullet 的區域變數"),
    (0x2C13, 0x2C16, "word_1F7D3、word_1F7D5 瞄準目標的欄與列；sub_172DE 寫，只有 sub_16F9F 讀（sub_171E5 的 1724Ch 起是 retn 之後無跳入的碼）。\
                      重製改成 aim_target 的回傳值"),
    (0x2C1B, 0x2C1B, "byte_1F7DB 槽號；sub_17361 歸零並每槽加 1（17367h、17394h），只有 sub_17855 讀（17888h）。重製改成 stamp_grid2 的參數"),
    (0x32AD, 0x32B2, "word_1FE6D、word_1FE6F 開火起點的欄與列：呼叫端（sub_15DBE、15E68、15FD9、1606A、1617A）寫，只有 sub_16D3B 一族（sub_16CEA、16F9F、172DE）\
                      讀寫。word_1FE71 本槽釋放旗標：sub_17631 一族（sub_176D0、176F6、1771C、17742、17955、1798E、179C7、17A00）寫，\
                      只有它們與 sub_17361 讀。重製改成 fire_enemy_bullets 的參數與 move_bullet 的回傳值"),
    (0x3533, 0x3552, "word_200F3 與工作區 3535h 起 15 個字組（word_200F5 至 word_20111）。sub_1739B 寫滿 15 字組後才由 sub_173C5 一族讀（17631、176B5、\
                      176D0、176F6、1771C、17742、17768、1779B、177D2、17821、17855、17898、173E1、17955、1798E、179C7、17A00），sub_173B0 複製回記錄；\
                      word_200F3 另是 sub_16D3B 一族寫、sub_16CEA 讀的樣板位址。重製改成 step_bullet 的區域 Bullet 與 spawn_bullet 的 Template 參數"),
    // ---- 玩家彈領域重寫（規格 004）：函式內先寫後讀的工作變數與呼叫參數，改成區域變數與函式參數，不再寫回資料段 ----
    // 查證：tools/q.sh dsrefs 列出每個變數的全部讀寫（結果記在 workspace/progress/rewrite-pbullet.md）。
    // 池指標表 2607h、記錄區 262Fh、追蹤彈計數 25E8h 是狀態，走 Mirror，不在這裡。
    // 命中計數器 word_1F1B8、1F1BA（25F8h 至 25FBh）只在不可達的碼裡寫，內容不變，所以不豁免。
    (0x25EA, 0x25F7, "word_1F1AA、1F1AC、1F1AE、1F1B0 四向格距與 word_1F1B2、1F1B4、1F1B6 掃描起點與範圍；sub_16BA8 先寫 1F1B2、1F1B4、1F1B6，\
                      sub_16BF5、16C18、16C3B、16C5C 讀它們並各寫一個格距，格距只有 sub_16BA8 讀。重製改成 distance_to_wall 的參數與回傳值"),
    (0x25FC, 0x2606, "word_1F1BC 武器字母、1F1BE 朝向、byte_1F1C0 命中格值、word_1F1C1 欄、1F1C3 列、1F1C5 釋放旗標。\
                      1F1BC、1F1BE、1F1C1、1F1C3 由呼叫端（sub_14501、1480A、1502F、15338）寫，只有 sub_16578、165CF、1669F 讀，\
                      165CF 一族就地改 1F1C1、1F1C3。1F1C0 由 sub_16B49 歸零、169BF 與雷射 16A4E、16ACA 設值，只有 sub_167A3、1691A、16946 讀。\
                      1F1C5 由 sub_16817 歸 N、移動與碰撞設 Y，只有 sub_1673E 讀。重製改成 fire_player_bullets 的參數、Impact 與回傳值"),
    (0x28AF, 0x28D0, "word_1F46F 記錄位址與 28B1h 起 16 個字組的工作區（word_1F471 至 word_1F48F）。sub_16779 把記錄複製進工作區並寫 1F46F，\
                      之後才由 sub_167A3 一族讀寫，sub_1678E 複製回 1F46F 指的記錄；sub_165CF 一族寫樣板位址進 1F46F，只有 sub_16578 讀。\
                      重製改成 update_player_bullets 的區域 PlayerBullet 與 spawn_player_bullet 的 Shot 參數"),
    // ---- 敵人領域重寫（規格 004）：函式內先寫後讀的工作變數與呼叫參數，改成區域變數與函式參數，不再寫回資料段 ----
    // 查證：tools/q.sh refs 與 dsrefs 列出每個變數的全部讀寫（結果記在 workspace/progress/rewrite-entity.md）。
    // 池指標表 2294h、記錄區 22B4h、每槽繪製值 21ECh 至 228Bh、最近繪製值 228Ch、景物池 198Ch 至 19DBh 是狀態，走 Mirror，不在這裡。
    // 死亡旗標 word_1EDAA（21EAh）看似工作變數，其實不能豁免：它的位址是 G3、G4 列起點表的越界列，格網位址函式與玩家領域
    // （尚未搬走）會從資料段直接讀，所以留在資料段，由 `Core::set_enemy_death_flag`（mirror.rs）寫。
    (0x042A, 0x042B, "word_1CFEA 亂數索引的傳遞字：sub_10E33 寫 0（10E4Eh、10E8Dh），sub_1617A 先寫（1617Dh）後讀（16183h），\
                      sub_110A4 讀寫（110A4h、110B0h）。重製改成 rolls_to_fire 的區域變數"),
    (0x1AF6, 0x1AF7, "word_1E6B6 閃白旗標：sub_15F8D 寫（15F8Dh、15FB6h），sub_15E44 讀（15E60h），sub_164F3 寫（1650Ch）後讀（1652Eh），\
                      每次使用前都在同一個敵人的處理裡寫過。重製改成 Step::flashing 與 Slot::flash"),
    (0x1B06, 0x1B09, "word_1E6C6 圖框、byte_1E6C8 移動碼、byte_1E6C9 槽號。1E6C6：sub_15F4C 寫（15F63h、15F6Bh）並讀（15F7Bh），sub_16167 讀（16173h）。\
                      1E6C8：sub_15E68（15E84h、15EB4h）與 sub_15ECD（15EE4h、15F2Dh）寫，只有 sub_15F4C 讀（15F52h）。\
                      1E6C9：sub_15D8C 歸零並每槽加 1（15D92h、15DB7h），只有 sub_16233 讀（16278h）。\
                      重製改成 select_frame 的回傳值、step_enemy 的 code 與 stamp_footprint_g1 的 slot 參數"),
    (0x25B4, 0x25E5, "word_1F174 記錄位址與 25B6h 起 24 個字組的工作區（word_1F176 至 word_1F1A4）。sub_15E1A 與 sub_15ECD 的變身（15F06h）\
                      用 rep movsw 寫滿 24 字組，之後才由 sub_15E68 一族與 sub_15DBE 讀寫，sub_15E2F 複製回 1F174 指的記錄；\
                      sub_15D1C 寫 1F174 的樣板位址，只有 sub_15D41 讀；sub_164F3、16541 另把渲染陣列的 kind 寫進 w18（1F19A），之後沒有人讀。\
                      重製改成 update_enemies 的區域 Enemy 與 spawn_enemy 的樣板參數"),
    // ---- 玩家領域重寫（規格 004）：函式內先寫後讀的工作變數與呼叫參數，改成區域變數與函式參數，不再寫回資料段 ----
    // 查證：tools/q.sh refs 與 dsrefs 列出每個變數的全部讀寫，並用 tools/q.sh callers 確認讀取端只在同一個呼叫鏈裡
    // （結果記在 workspace/progress/rewrite-player.md）。其餘玩家變數（血量、位置、圖、旗標、砲座、光柱、輸入位元、存活位元組）
    // 是狀態，走 Mirror，不在這裡。
    (0x0ECA, 0x0ECA, "byte_1DA8A 玩家 1 砲座接觸的格值：sub_14B63 先歸零（14B63h）、掃描中寫（14B9Fh），只有它呼叫的 sub_14C18 讀（14C18h）。\
                      重製改成 pod_touches_entities 的區域變數"),
    (0x0F2C, 0x0F2C, "byte_1DAEC 玩家 2 砲座接觸的格值：sub_15691 先歸零（15691h）、掃描中寫（156CDh），只有它呼叫的 sub_15746 讀（15746h）。\
                      重製改成 pod_touches_entities 的區域變數"),
    (0x0EDC, 0x0EDF, "word_1DA9C、word_1DA9E 玩家 1 砲座掃描的欄與列：sub_1480A 每次掃描前寫（14815h、1481Bh、14855h、1485Eh），\
                      只有 sub_14B63（14B78h）、sub_14BAE（14BBFh）、sub_14BF4（14C0Bh）、sub_14C3D（14C54h）讀。重製改成 pod_collide 的參數"),
    (0x0F3E, 0x0F41, "word_1DAFE、word_1DB00 玩家 2 砲座掃描的欄與列：sub_15338 每次掃描前寫（15343h、15349h、15383h、1538Ch），\
                      只有 sub_15691（156A6h）、sub_156DC（156EDh）、sub_15722（15739h）、sub_1576B（15782h）讀。重製改成 pod_collide 的參數"),
    (0x0EE8, 0x0EE8, "byte_1DAA8 玩家 1 本幀撿到的道具：sub_14C85 先設 N（14C8Ch）、sub_14CD4 寫（14CF9h），只有 sub_14C85 呼叫的 sub_14D04 讀（14D04h）。\
                      重製改成 body_touches_bullets 的區域變數與 upgrade 的參數"),
    (0x0F4A, 0x0F4A, "byte_1DB0A 玩家 2 本幀撿到的道具：sub_157B3 先設 N（157BAh）、sub_15802 寫（15827h），只有 sub_157B3 呼叫的 sub_15832 讀（15832h）。\
                      重製改成 body_touches_bullets 的區域變數與 upgrade 的參數"),
    // ---- 標題與片尾領域重寫（規格 004）：過場字樣的位置暫存，改成每張卡的區域結構 ----
    // 查證：tools/q.sh refs word_1D148 至 word_1D156（十個符號）列出全部讀寫，另 grep 反組譯裡所有 580h 至 597h 的位移形式，
    // 除了 word_1D140 至 word_1D146（走 Mirror）沒有其他存取，也沒有 [bx+位移] 的表格式存取。
    // 選單的列、難度、角色（580h 至 585h）與示範輪播（586h）是狀態，走 Mirror，不在這裡。
    (0x0588, 0x0597, "word_1D148 至 word_1D156 過場與開場的字樣位置。每個寫入它們的函式都先寫初值再讀：sub_117BF 寫 1D148、1D14A（117BFh、117C5h）後讀；\
                      sub_11C6B 寫 1D148、1D152、1D14C（11C71h、11C77h、11C7Dh）與 1D14E（11CD0h）；sub_11E46 寫 1D148 至 1D14E、1D156（11E4Ch 至 11E64h）\
                      與 1D152、1D154（11EB7h、11EE2h）；sub_121DD 寫 1D150 至 1D156（121E0h 至 121F2h）；sub_12379 寫 1D148 至 1D14E（1237Ch 至 1238Eh）。\
                      讀者只有各自卡片的步驟函式（sub_11CFC、11D58、11DAC、11F58、11FB3、1200A、12062、120B6、12132、1221E、123BD）。\
                      重製改成 cards.rs 的 Stage1Letters 至 Stage4Letters 與 intro.rs 的區域變數"),
    // ---- 流程與關卡領域重寫（規格 004）：函式內先寫後讀的工作變數，改成區域變數與函式參數，不再寫回資料段 ----
    // 查證：tools/q.sh refs 列出每個變數的全部讀寫（結果記在 workspace/progress/rewrite-flow.md）。
    // 各關的背景位置、頭目合成圖的三層位置、流程旗標、輸入與計時是狀態，走 Mirror，不在這裡。
    (0x0F9A, 0x0F9E, "byte_1DB5A 腳本位元組、word_1DB5B 與 word_1DB5D 頭目合成圖的位置：第一、二、三關的 sub_15930、15983、159D6、15A24、15A72 \
                      先從該層的位置（1DB5F 一族）複製進來才呼叫 sub_15BB0（直譯器）與 sub_15AC5、15AF4、15B23、15B52、15B81（繪製），\
                      byte_1DB5A 只在 sub_15930、15983、15A72 呼叫 sub_15BB0 之後立刻讀。重製改成 BossLayer::step 直接更新那一層"),
    (0x0FAB, 0x0FAC, "word_1DB6B 腳本讀取位置：同上，每次由 sub_15930、15983、159D6、15A24、15A72 複製進來，sub_15BB0 就地前進後複製回去。\
                      重製改成 BossLayer::script"),
    (0x900A, 0x900B, "word_25BCA 第三關地圖的索引游標：sub_17DF9 開頭先寫（17E13h）、迴圈內讀寫（17E3Ah、17E4Eh、17E5Eh），sub_182CD 同樣先寫（182F4h）\
                      再讀寫（18310h、18325h）。其他寫入者都不在 E 模式的可達路徑：sub_17DB2 的 17DBEh 起（`17DBDh` retn 之後）、\
                      sub_1840E 的 18430h 起（retn 之後）、單色分支 sub_185AA、sub_18613、sub_1812E、sub_181E1。\
                      重製改成 draw_initial_map 與 scroll_map_second_page 的區域變數"),
];

/// `off` 是否落在 [`DS_EXEMPT`]。
pub fn ds_exempt(off: u16) -> bool {
    DS_EXEMPT.iter().any(|&(lo, hi, _)| lo <= off && off <= hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 映像大小的檔案，資料段每個位元組都是 `fill`。
    fn image_of(fill: u8) -> Vec<u8> {
        vec![fill; DSEG_IMAGE_OFFSET + 0x10000]
    }

    fn blank() -> Dseg {
        Dseg::from_image(&image_of(0))
    }

    /// 偽隨機的資料段，池指標表依槽號交替成 0 或該槽記錄的位址（真實的傾印只有這兩種值）。
    fn random_dump() -> Dseg {
        let mut image = image_of(0);
        let mut s = 0x9E37_79B9u32;
        for b in image[DSEG_IMAGE_OFFSET..].iter_mut() {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *b = (s >> 24) as u8;
        }
        let mut ds = Dseg::from_image(&image);
        for i in 0..20usize {
            ds.w16(EB_LINKS + 2 * i as u16, if i % 2 == 0 { eb_record(i) } else { 0 });
            ds.w16(PB_LINKS + 2 * i as u16, if i % 2 == 0 { pb_record(i) } else { 0 });
        }
        for i in 0..entity::POOL_SIZE {
            ds.w16(EN_LINKS + 2 * i as u16, if i % 2 == 0 { enemy_record(i) } else { 0 });
        }
        ds
    }

    /// 狀態的傾印 `store` 實際寫到的位元組：底分別用 00h 與 FFh，兩份傾印相同的位元組就是 `store` 寫的。
    fn written_bytes(state: &State) -> (Dseg, Vec<bool>) {
        let (d0, d1) = (Mirror::new(&image_of(0x00)).dump(state), Mirror::new(&image_of(0xFF)).dump(state));
        let covered = (0..0x10000).map(|i| d0.bytes()[i] == d1.bytes()[i]).collect();
        (d0, covered)
    }

    #[test]
    fn wide_grids_are_not_in_the_original_data_segment_and_neither_dump_nor_load_touches_it() {
        // 戰場加大的格網比原版大，傾印略過它，不去蓋別的變數；原版大小的格網照舊寫進資料段。
        for cols in [48u16, 60] {
            let mut state = State::default();
            state.grids = rs2_game::Grids::with_cols(cols);
            state.grids.g1.fill(0xAB);
            state.grids.g2.fill(0xCD);
            state.grids.terrain.fill(0xEF);
            let m = Mirror::new(&image_of(0));
            let dumped = m.dump(&state);
            let plain = m.dump(&State::default());
            assert_eq!(dumped.bytes(), plain.bytes(), "{cols} 欄：傾印與沒有格網內容的狀態相同");
            // 載入也不去動寬格網：載進一份全新的（40 欄）狀態，格網還是原版大小。
            let back = m.load(&dumped);
            assert_eq!(back.grids.cols(), 40);
        }
        let mut state = State::default();
        state.grids.g1.fill(0xAB);
        let dumped = Mirror::new(&image_of(0)).dump(&state);
        assert_eq!(dumped.r8(GRID1), 0xAB, "40 欄的格網照舊寫進 1B32h");
    }

    #[test]
    #[should_panic(expected = "poke 只用在 100% 的戰場")]
    fn poking_a_wide_state_is_refused_because_the_load_would_reset_the_grids() {
        let mut state = State::default();
        state.grids = rs2_game::Grids::with_cols(48);
        Mirror::new(&image_of(0)).poke8(&mut state, 0x0100, 1);
    }

    #[test]
    fn store_is_the_inverse_of_load_on_every_byte_it_writes() {
        let ds = random_dump();
        let m = Mirror::new(&image_of(0));
        let state = m.load(&ds);
        let (back, covered) = written_bytes(&state);
        let mut bad = Vec::new();
        for i in 0..0x10000usize {
            if covered[i] && back.bytes()[i] != ds.bytes()[i] {
                bad.push(format!("{i:04X}: 讀進 {:02X} 寫回 {:02X}", ds.bytes()[i], back.bytes()[i]));
            }
        }
        assert!(bad.is_empty(), "{} 個位元組寫回不同：{:?}", bad.len(), &bad[..bad.len().min(20)]);
        let written = covered.iter().filter(|&&c| c).count();
        // 目前 store 寫 10704 個位元組（其餘是唯讀表格、圖庫緩衝與映像殘值）；明顯變少代表有領域沒寫。
        assert!(written > 10_000, "store 寫到的位元組數太少：{written}");
        // 反方向：狀態寫成傾印再讀回，狀態不變。
        assert_eq!(m.load(&m.dump(&state)), state);
    }

    #[test]
    fn enemy_bullet_fields_land_at_the_documented_offsets() {
        let mut eb = EnemyBullets::default();
        for (i, slot) in eb.slots.iter_mut().enumerate() {
            let w: [u16; 15] = std::array::from_fn(|k| 0x100 * (i as u16 + 1) + k as u16 + 1);
            *slot = EbSlot { link: ebullet::record_link(i), bullet: Bullet::from_words(w) };
        }
        let mut ds = blank();
        eb.store(&mut ds);
        for i in 0..ebullet::POOL_SIZE as u16 {
            assert_eq!(ds.r16(0x32B3 + 2 * i), 0x32DB + 0x1E * i, "槽 {i} 的指標是記錄位址，表在 32B3h + 2i");
            for k in 0..15u16 {
                assert_eq!(ds.r16(0x32DB + 0x1E * i + 2 * k), 0x100 * (i + 1) + k + 1, "槽 {i} 的 w{k}");
            }
        }
        let mut back = EnemyBullets::default();
        back.load(&ds);
        assert_eq!(back, eb);
        eb.slots[2].link = 0;
        eb.store(&mut ds);
        assert_eq!(ds.r16(0x32B3 + 4), 0, "空槽的指標寫 0");
    }

    #[test]
    fn player_bullet_fields_land_at_the_documented_offsets() {
        let mut pb = PlayerBullets { seekers_in_flight: 0x4321, ..PlayerBullets::default() };
        for (i, slot) in pb.slots.iter_mut().enumerate() {
            let w: [u16; 16] = std::array::from_fn(|k| 0x100 * (i as u16 + 1) + k as u16 + 1);
            *slot = PbSlot { link: pbullet::pb_record_link(i), bullet: PlayerBullet::from_words(w) };
        }
        let mut ds = blank();
        pb.store(&mut ds);
        assert_eq!(ds.r16(0x25E8), 0x4321, "追蹤彈計數在 25E8h");
        for i in 0..pbullet::POOL_SIZE as u16 {
            assert_eq!(ds.r16(0x2607 + 2 * i), 0x262F + 0x20 * i, "槽 {i} 的指標在 2607h + 2i");
            for k in 0..16u16 {
                assert_eq!(ds.r16(0x262F + 0x20 * i + 2 * k), 0x100 * (i + 1) + k + 1, "槽 {i} 的 w{k}");
            }
        }
        assert_eq!(ds.r16(0x262F + 0x280), 0, "記錄區之後的字組不動");
        let mut back = PlayerBullets::default();
        back.load(&ds);
        assert_eq!(back, pb);
    }

    #[test]
    fn enemy_slots_and_the_death_flag_land_at_the_documented_offsets() {
        let mut en = Enemies { death_flag: 0x1111, stray_damage: 0x2222, spare_link: 0x3333, ..Enemies::default() };
        en.slots[1] = EnSlot { link: entity::enemy_record_link(1), flash: 0x0707, ..EnSlot::default() };
        en.slots[1].sprite = Sprite { image: 0xA1, x: 0xA2, y: 0xA3, kind: 0xA4 };
        let mut ds = blank();
        en.store(&mut ds);
        assert_eq!(ds.r16(0x21EA), 0x1111, "死亡旗標 word_1EDAA 在 21EAh");
        assert_eq!(ds.r16(0x0020), 0x2222, "指標 0 加 20h 的雜散傷害在 0020h");
        assert_eq!(ds.r16(0x2294 + 2), 0x22B4 + 0x30, "槽 1 的指標是記錄位址");
        assert_eq!(ds.r16(0x2294 + 2 * 15), 0x3333, "備用指標字組在表尾");
        assert_eq!([ds.r16(0x21EC + 2), ds.r16(0x220C + 2), ds.r16(0x222C + 2), ds.r16(0x224C + 2), ds.r16(0x226C + 2)], [0xA1, 0xA2, 0xA3, 0xA4, 0x0707]);
        let mut back = Enemies::default();
        back.load(&ds);
        assert_eq!(back, en);
    }

    #[test]
    fn player_fields_land_at_the_documented_offsets() {
        // 位移是機器碼的線性位址減 1CBC0h。(欄位, 玩家 1 的位移, 玩家 2 的位移, 寬)
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
        let mut ds = blank();
        pl.store(&mut ds);
        assert_eq!(ds.r8(0x041D), 0x5A, "存活位元組在 041Dh");
        for (k, (name, offsets, width)) in layout.iter().enumerate() {
            for who in 0..PLAYER_COUNT {
                let want = value(who, k);
                let (got, want) = if *width == 2 { (ds.r16(offsets[who]), want) } else { (u16::from(ds.r8(offsets[who])), want & 0xFF) };
                assert_eq!(got, want, "玩家 {} 的 {name} 在 {:04X}h", who + 1, offsets[who]);
            }
        }
        // 寫了的位元組數剛好等於各欄位寬度的總和（值都不是 0）：欄位不重疊，也沒有碰其他位元組。
        let touched = ds.bytes().iter().filter(|&&b| b != 0).count();
        let expected: usize = layout.iter().map(|(_, _, w)| usize::from(*w)).sum::<usize>() * PLAYER_COUNT + 1;
        assert_eq!(touched, expected, "非零位元組數等於欄位總寬");
        let mut back = Players::default();
        back.load(&ds);
        assert_eq!(back, pl);
    }

    #[test]
    fn the_work_variables_that_left_the_structs_are_not_written_by_store() {
        // 這些位移在原版是函式內先寫後讀的工作變數（DS_EXEMPT），Mirror 不碰。
        let mut ds = Dseg::from_image(&image_of(0xAA));
        let mut pl = Players::default();
        for p in &mut pl.ship {
            *p = Player { hp: 0xFFFF, col: 0xFFFF, row: 0xFFFF, ..Player::default() };
        }
        pl.store(&mut ds);
        for off in [0x0ECA, 0x0EDC, 0x0EDD, 0x0EDE, 0x0EDF, 0x0EE8, 0x0F2C, 0x0F3E, 0x0F3F, 0x0F40, 0x0F41, 0x0F4A] {
            assert_eq!(ds.r8(off), 0xAA, "{off:04X}h 不屬於玩家結構");
        }
        let mut ds = Dseg::from_image(&image_of(0xAA));
        let mut scenes = Scenes::default();
        scenes.boss.layers[0].y = 0xFFFF;
        scenes.store(&mut ds);
        for off in [0x0F9A, 0x0F9B, 0x0F9C, 0x0F9D, 0x0F9E, 0x0FAB, 0x0FAC, 0x900A, 0x900B, 0x9C8C, 0x9C8D] {
            assert_eq!(ds.r8(off), 0xAA, "{off:04X}h 不屬於場景結構");
        }
    }

    #[test]
    fn the_untouched_shape_width_is_ffffh_on_the_data_segment() {
        let mut ds = blank();
        let mut p = DrawParams::default();
        p.store(&mut ds);
        assert_eq!(ds.r16(0x9CBE), 0xFFFF, "沒寫過");
        p.load(&ds);
        assert_eq!(p.shape_width, None);
        p.shape_width = Some(0);
        p.store(&mut ds);
        assert_eq!(ds.r16(0x9CBE), 0, "形狀寬 0 是寫過的值");
        p.shape_width = None;
        p.load(&ds);
        assert_eq!(p.shape_width, Some(0));
    }

    #[test]
    fn the_exemption_tables_are_well_formed() {
        for (lo, hi, why) in DS_EXEMPT {
            assert!(lo <= hi && !why.is_empty(), "{lo:04X}..{hi:04X} 要有理由且 lo <= hi");
        }
        for (lo, hi) in A0_EXEMPT {
            assert!(lo <= hi);
        }
        assert!(ds_exempt(0xA7F7) && !ds_exempt(0x0422), "時脈字組豁免、幀計數不豁免");
    }

    #[test]
    fn a_poke_goes_through_the_data_segment_and_back() {
        let m = Mirror::new(&image_of(0));
        let mut state = State::default();
        m.poke16(&mut state, 0x041E, 0x00FF);
        assert_eq!(state.pl.ship[0].hp, 0x00FF);
        m.poke8(&mut state, 0x041D, 0x11);
        assert_eq!(state.pl.alive, 0x11);
        assert_eq!(state.pl.ship[0].hp, 0x00FF, "別的欄位不變");
    }
}
