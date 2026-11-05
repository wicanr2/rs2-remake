//! 唯讀表格：啟動時從 `S.EXE` 解包映像解析成有名字的結構，雜湊不符就拒絕。解析完不留映像。
//!
//! 映像裡的位置只出現在這個檔案的解析程式裡，它描述 `S.EXE` 的版面；遊戲邏輯拿到的是有型別的資料。
//! 記錄裡的指標欄位（腳本、圖框、形狀）保留檔案裡的原值，當作這裡各張表的鍵。每張表只收解析得到的物件：
//! 形狀、圖框表、位元組腳本、特效腳本的每一格、敵人與子彈的樣板、時間表、景物群組、調色盤等。
//! 鍵不在表內的查詢是 `panic`（fail-closed）：遊戲跑到解析範圍之外代表解析漏了東西，不能默默當 0。
//!
//! 解析規則（格式依據 `tools/export_entities.py` 與 `docs/re/11-entities-combat.md`）：
//!
//! | 物件 | 範圍 |
//! |---|---|
//! | 形狀 | `寬, 高, 寬 × 高 位元組`，寬 1 至 80、高 1 至 24 |
//! | 圖框表、動畫上限表 | 9 個字組；`K` 型敵人的圖框指標開頭是損壞貼片的 `(圖, x, y)` |
//! | 敵人腳本 | 逐位元組到結束碼：`0F` 結束，`F0` 跳轉與 `E0` 變身各帶一個字組 |
//! | 敵彈腳本 | 逐位元組到 `F?`（往回跳）或 `00`（結束）為止，高 4 位元只能是 0 或 F |
//! | 玩家彈腳本 | 逐位元組到 `F?` 為止，高 4 位元只能是 0 或 F |
//! | 頭目腳本 | 逐位元組到第一個 `0F` |
//! | 特效腳本 | 每格 4 個字組（圖、dx、dy、形狀），圖號為 `0F` 結束 |

use crate::ai::{Boss, Role};
use crate::ebullet::{Bullet, Capsule, Heading, Spread, Template, Tilt};
use crate::entity::{Enemy, Timetable};
use crate::flow::Palette;
use crate::fx::Script;
use crate::game::Core;
use crate::pbullet::{Facing, PlayerBullet, Shot};
use crate::player::PlayerShape;
use crate::scenery::Prop;
use rs2_formats::{DSEG_IMAGE_OFFSET, UNPACKED_SHA256, check_hash};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

/// 列基底表與列起點表的列數。
const GRID_ROWS: usize = 20;
/// 開火亂數表的項數。
const FIRE_ROLLS: usize = 75;
/// 鍵盤中斷的掃描碼轉 ASCII 表的項數。
pub(crate) const ASCII_ENTRIES: usize = 0x5E;
/// 一張調色盤：EGA 屬性表接 DAC 表。
const ATTR_BYTES: usize = 17;
const DAC_BYTES: usize = 48;
/// 完整性檢查累加的字組數與初值。
const INTEGRITY_WORDS: u16 = 0x190;
const INTEGRITY_SEED: u16 = 0x64;

/// 一張形狀：`寬, 高, 寬 × 高 位元組`的格值，列優先。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Shape {
    pub width: u16,
    pub height: u16,
    cells: Box<[u8]>,
}

impl Shape {
    pub(crate) fn new(width: u16, height: u16, cells: Vec<u8>) -> Shape {
        assert_eq!(usize::from(width) * usize::from(height), cells.len(), "形狀的格數要等於寬乘高");
        Shape { width, height, cells: cells.into_boxed_slice() }
    }

    /// 第 `index` 格的值（列優先）。
    pub(crate) fn cell(&self, index: u16) -> u8 {
        match self.cells.get(usize::from(index)) {
            Some(&v) => v,
            None => panic!("形狀 {}x{} 沒有第 {index} 格", self.width, self.height),
        }
    }
}

/// 特效腳本的一格。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FxFrame {
    pub image: u16,
    pub dx: u16,
    pub dy: u16,
    /// 形狀指標。
    pub shape: u16,
}

/// 一張調色盤：EGA 屬性表與 DAC 表。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PaletteData {
    pub attr: [u8; ATTR_BYTES],
    pub dac: [u8; DAC_BYTES],
}

impl Default for PaletteData {
    fn default() -> PaletteData {
        PaletteData { attr: [0; ATTR_BYTES], dac: [0; DAC_BYTES] }
    }
}

/// 位元組腳本：每段是從某個位置起到結束碼的連續位元組，鍵是第一個位元組的位置。
#[derive(Clone, Debug, Default)]
struct Runs {
    runs: BTreeMap<u16, Box<[u8]>>,
}

impl Runs {
    fn insert(&mut self, start: u16, bytes: Vec<u8>) {
        self.runs.insert(start, bytes.into_boxed_slice());
    }

    /// 位置已經在某一段裡就不用再收。
    fn covers(&self, at: u16) -> bool {
        self.byte(at).is_some()
    }

    fn byte(&self, at: u16) -> Option<u8> {
        let (&start, bytes) = self.runs.range(..=at).next_back()?;
        bytes.get(usize::from(at - start)).copied()
    }
}

/// 所有唯讀表格。
#[derive(Clone, Debug)]
pub struct Tables {
    /// 列基底表：G1、G2 每列的起點相對格網起點的位移，`列 × 28h`。
    pub(crate) row_base: [u16; GRID_ROWS],
    /// 列起點表：G3、G4 每列的起點，`列 × 50h + 1`。
    pub(crate) row_start: [u16; GRID_ROWS],
    /// 敵人開火的固定亂數表，值域 0 至 15。
    pub(crate) fire_rolls: [u16; FIRE_ROLLS],
    /// 掃描碼轉 ASCII。
    pub(crate) ascii: [u8; ASCII_ENTRIES],
    /// 完整性檢查累加的和（原版 `sub_12F95`）。
    pub(crate) integrity_sum: u16,
    palettes: Vec<PaletteData>,
    shapes: BTreeMap<u16, Shape>,
    /// 圖框表、動畫上限表與損壞貼片：位元組是字組的小端序，鍵是表的位置。
    word_tables: BTreeMap<u16, Box<[u8]>>,
    scripts: Runs,
    fx_frames: BTreeMap<u16, FxFrame>,
    enemies: BTreeMap<u16, Enemy>,
    timetables: HashMap<Timetable, Vec<(u16, u16)>>,
    bullets: HashMap<Template, Bullet>,
    shots: HashMap<(Shot, Facing), PlayerBullet>,
    /// 景物群組表：`(幀, 景物記錄位置)`，幀為 0 是表尾（含表尾那一列）。
    scenery_groups: Vec<(u16, u16)>,
    scenery_records: BTreeMap<u16, Prop>,
    /// 第四關通道腳本，含結尾的 `0F`。
    tunnel: Box<[u8]>,
    /// 頭目腳本的起點位置。
    boss_starts: [u16; 5],
    /// 玩家的形狀：兩位玩家，每位本體、砲座、尾焰。
    player_shapes: [[Shape; 3]; 2],
}

impl Default for Tables {
    fn default() -> Tables {
        Tables {
            row_base: [0; GRID_ROWS],
            row_start: [0; GRID_ROWS],
            fire_rolls: [0; FIRE_ROLLS],
            ascii: [0; ASCII_ENTRIES],
            integrity_sum: 0,
            palettes: vec![PaletteData::default(); Palette::ALL.len()],
            shapes: BTreeMap::new(),
            word_tables: BTreeMap::new(),
            scripts: Runs::default(),
            fx_frames: BTreeMap::new(),
            enemies: BTreeMap::new(),
            timetables: HashMap::new(),
            bullets: HashMap::new(),
            shots: HashMap::new(),
            scenery_groups: vec![(0, 0)],
            scenery_records: BTreeMap::new(),
            tunnel: Box::new([]),
            boss_starts: [0; 5],
            player_shapes: std::array::from_fn(|_| std::array::from_fn(|_| Shape::new(1, 1, vec![0]))),
        }
    }
}

// ---- 映像的版面（位置只在這一節）----

/// 解包映像的唯讀檢視。位置是資料段位移。
struct Image<'a>(&'a [u8]);

type Parsed<T> = Result<T, String>;

impl Image<'_> {
    fn u8(&self, ds: u16) -> Parsed<u8> {
        self.0.get(DSEG_IMAGE_OFFSET + usize::from(ds)).copied().ok_or_else(|| format!("映像沒有資料段位移 {ds:04X}h"))
    }

    fn u16(&self, ds: u16) -> Parsed<u16> {
        Ok(u16::from_le_bytes([self.u8(ds)?, self.u8(ds.wrapping_add(1))?]))
    }

    fn bytes<const N: usize>(&self, ds: u16) -> Parsed<[u8; N]> {
        let mut out = [0u8; N];
        for (k, v) in out.iter_mut().enumerate() {
            *v = self.u8(ds.wrapping_add(k as u16))?;
        }
        Ok(out)
    }

    fn words(&self, ds: u16, n: u16) -> Parsed<Vec<u16>> {
        (0..n).map(|k| self.u16(ds.wrapping_add(2 * k))).collect()
    }
}

/// 調色盤的 DAC 表與 EGA 屬性表位置。屬性表在 DAC 表之前 11h 位元組，只有載入畫面例外。
const fn palette_offsets(p: Palette) -> (u16, u16) {
    let dac = match p {
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
    };
    let attr = match p {
        Palette::Loader => 0x056E,
        _ => dac - 0x11,
    };
    (dac, attr)
}

/// 特效腳本的起點。
const fn fx_script_start(script: Script) -> u16 {
    match script {
        Script::SparkRight => 0x4094,
        Script::SparkLeft => 0x40B2,
        Script::BigExplosion => 0x40D0,
        Script::Blast => 0x40EE,
        Script::Trail => 0x413C,
        Script::SeekerTrail => 0x4162,
        Script::PlayerDeath => 0x4188,
    }
}

/// 道具球樣板的起點（朝向 1）。四種各 8 個樣板，連續排列。
const fn capsule_base(capsule: Capsule) -> u16 {
    match capsule {
        Capsule::L => 0x3737,
        Capsule::B => 0x3827,
        Capsule::K => 0x3917,
        Capsule::S => 0x3A07,
    }
}

/// 敵彈樣板的位置。樣板表從 `3557h` 起共 72 筆，每筆 1Eh 位元組；八向的樣板依朝向 1 至 8 連續排列。
/// `3647h` 至 `3719h` 的 8 筆（形狀 `3F40h`、圖庫索引 0）沒有任何程式引用，不收。
const fn bullet_template_at(template: Template) -> u16 {
    const BYTES: u16 = 0x1E;
    match template {
        Template::Aimed(h) => 0x3557 + BYTES * h.index(),
        Template::Capsule(c, h) => capsule_base(c) + BYTES * h.index(),
        Template::Unused => 0x3AF7,
        Template::Wide(t) => 0x3B15 + BYTES * t as u16,
        Template::Spread(s) => 0x3B6F + BYTES * s as u16,
        Template::WideFlash => 0x3C05,
        Template::Flash => 0x3C23,
        Template::Straight(h) => 0x3C41 + BYTES * h.index(),
        Template::SlowSw => 0x3D31,
        Template::SlowW => 0x3D4F,
        Template::SlowNw => 0x3D6D,
        Template::BouncerNw => 0x3D8B,
        Template::BouncerNe => 0x3DA9,
    }
}

/// 玩家彈樣板的位置。樣板表從 `28D5h` 起共 20 筆，每筆 20h 位元組。`28F5h`、`2915h`（備用雙彈碼，前面是 `retn`）
/// 與 `2A35h`、`2A55h`、`2B15h` 沒有任何可達的程式引用，不收。掃射兩個朝向共用 `2B35h`。
const fn shot_template_at(shot: Shot, facing: Facing) -> u16 {
    match (shot, facing) {
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

/// 時間表的起點。原版每關一個只有 `mov si, 表; call sub_15D1C` 的包裝函式。
const fn timetable_at(table: Timetable) -> u16 {
    match table {
        Timetable::Stage1 => 0x41BE,
        Timetable::Stage2 => 0x54FE,
        Timetable::Stage3 => 0x68E0,
        Timetable::Stage4 => 0x7922,
        Timetable::Ending => 0x8D1C,
    }
}

/// 玩家形狀表的位置：本體、砲座、尾焰；玩家 1 與玩家 2 內容相同，位址不同。
const fn player_shape_at(shape: PlayerShape, player: usize) -> u16 {
    match (shape, player) {
        (PlayerShape::Body, 0) => 0x0F0D,
        (PlayerShape::Pod, 0) => 0x0F16,
        (PlayerShape::Flame, 0) => 0x0F1E,
        (PlayerShape::Body, _) => 0x0F7B,
        (PlayerShape::Pod, _) => 0x0F85,
        (PlayerShape::Flame, _) => 0x0F8D,
    }
}

/// 頭目腳本的起點位置，順序同 [`crate::boss_scroll::BossScript`] 的 `index`。
const BOSS_STARTS: [u16; 5] = [0x0FB3, 0x127E, 0x1313, 0x13D1, 0x1692];
/// 第四關通道腳本的位置。
const TUNNEL_SCRIPT: u16 = 0x0DE4;
/// 景物群組表的位置。
const SCENERY_GROUPS: u16 = 0x19DC;
/// 開火亂數表、ASCII 表、列表的位置。
const FIRE_ROLL_TABLE: u16 = 0x042C;
const ASCII_TABLE: u16 = 0xA86D;
const ROW_BASE_TABLE: u16 = 0x1B0A;
const ROW_START_TABLE: u16 = 0x21C2;
/// 完整性檢查累加的資料：`7F4h` 起 400 個字組。
const INTEGRITY_TABLE: u16 = 0x07F4;

const HEADINGS: [Heading; 8] = [Heading::N, Heading::NE, Heading::E, Heading::SE, Heading::S, Heading::SW, Heading::W, Heading::NW];
const CAPSULES: [Capsule; 4] = [Capsule::L, Capsule::B, Capsule::K, Capsule::S];
const TILTS: [Tilt; 3] = [Tilt::NorthWest, Tilt::West, Tilt::SouthWest];
const SPREADS: [Spread; 5] = [Spread::W, Spread::NW, Spread::SW, Spread::N, Spread::S];
const TIMETABLES: [Timetable; 5] = [Timetable::Stage1, Timetable::Stage2, Timetable::Stage3, Timetable::Stage4, Timetable::Ending];
const SHOTS: [Shot; 8] = [Shot::Plain, Shot::Big, Shot::SpreadUp, Shot::SpreadDown, Shot::SeekerUp, Shot::SeekerDown, Shot::Laser, Shot::Sweep];
const FX_SCRIPTS: [Script; 7] =
    [Script::SparkRight, Script::SparkLeft, Script::BigExplosion, Script::Blast, Script::Trail, Script::SeekerTrail, Script::PlayerDeath];
const PLAYER_SHAPES: [PlayerShape; 3] = [PlayerShape::Body, PlayerShape::Pod, PlayerShape::Flame];

/// 全部 64 種被引用的敵彈樣板。
fn all_bullet_templates() -> Vec<Template> {
    let mut out = Vec::new();
    out.extend(HEADINGS.map(Template::Aimed));
    for c in CAPSULES {
        out.extend(HEADINGS.map(|h| Template::Capsule(c, h)));
    }
    out.push(Template::Unused);
    out.extend(TILTS.map(Template::Wide));
    out.extend(SPREADS.map(Template::Spread));
    out.push(Template::WideFlash);
    out.push(Template::Flash);
    out.extend(HEADINGS.map(Template::Straight));
    out.extend([Template::SlowSw, Template::SlowW, Template::SlowNw, Template::BouncerNw, Template::BouncerNe]);
    out
}

impl Tables {
    /// 從解包映像解析全部表格。映像的雜湊要等於 [`UNPACKED_SHA256`]，否則拒絕。
    pub fn parse(image: &[u8]) -> Result<Tables, String> {
        check_hash("解包映像", image, UNPACKED_SHA256)?;
        Tables::parse_unchecked(image)
    }

    /// 同 [`Tables::parse`]，不驗雜湊。結構不合時回傳錯誤。
    pub fn parse_unchecked(image: &[u8]) -> Result<Tables, String> {
        let img = Image(image);
        let mut t = Tables::default();
        for row in 0..GRID_ROWS {
            t.row_base[row] = img.u16(ROW_BASE_TABLE + 2 * row as u16)?;
            t.row_start[row] = img.u16(ROW_START_TABLE + 2 * row as u16)?;
        }
        for (k, v) in t.fire_rolls.iter_mut().enumerate() {
            *v = img.u16(FIRE_ROLL_TABLE + 2 * k as u16)?;
        }
        t.ascii = img.bytes::<ASCII_ENTRIES>(ASCII_TABLE)?;
        t.integrity_sum = (0..INTEGRITY_WORDS).try_fold(INTEGRITY_SEED, |sum, k| Ok::<u16, String>(sum.wrapping_add(img.u16(INTEGRITY_TABLE + 2 * k)?)))?;
        for p in Palette::ALL {
            let (dac, attr) = palette_offsets(p);
            t.palettes[p as usize] = PaletteData { attr: img.bytes(attr)?, dac: img.bytes(dac)? };
        }
        t.boss_starts = BOSS_STARTS;
        for start in BOSS_STARTS {
            let bytes = scan_until(&img, start, |b| b == 0x0F, 4096)?;
            t.scripts.insert(start, bytes);
        }
        t.tunnel = scan_until(&img, TUNNEL_SCRIPT, |b| b == 0x0F, 256)?.into_boxed_slice();
        t.parse_effects(&img)?;
        t.parse_scenery(&img)?;
        t.parse_player_shapes(&img)?;
        t.parse_player_bullets(&img)?;
        t.parse_enemy_bullets(&img)?;
        t.parse_enemies(&img)?;
        Ok(t)
    }

    fn take_shape(&mut self, img: &Image, ptr: u16) -> Parsed<()> {
        if self.shapes.contains_key(&ptr) {
            return Ok(());
        }
        let (width, height) = (img.u16(ptr)?, img.u16(ptr.wrapping_add(2))?);
        if !(1..=80).contains(&width) || !(1..=24).contains(&height) {
            return Err(format!("形狀 {ptr:04X}h 的尺寸 {width}x{height} 異常"));
        }
        let cells = (0..width * height).map(|k| img.u8(ptr.wrapping_add(4 + k))).collect::<Parsed<Vec<u8>>>()?;
        self.shapes.insert(ptr, Shape::new(width, height, cells));
        Ok(())
    }

    fn take_words(&mut self, img: &Image, ptr: u16, count: u16) -> Parsed<()> {
        if !self.word_tables.contains_key(&ptr) {
            let bytes = (0..2 * count).map(|k| img.u8(ptr.wrapping_add(k))).collect::<Parsed<Vec<u8>>>()?;
            self.word_tables.insert(ptr, bytes.into_boxed_slice());
        }
        Ok(())
    }

    fn parse_effects(&mut self, img: &Image) -> Parsed<()> {
        for script in FX_SCRIPTS {
            let mut at = fx_script_start(script);
            let mut ended = false;
            for _ in 0..64 {
                let frame = FxFrame {
                    image: img.u16(at)?,
                    dx: img.u16(at.wrapping_add(2))?,
                    dy: img.u16(at.wrapping_add(4))?,
                    shape: img.u16(at.wrapping_add(6))?,
                };
                self.fx_frames.insert(at, frame);
                if frame.image == 0x0F {
                    ended = true;
                    break;
                }
                self.take_shape(img, frame.shape)?;
                at = at.wrapping_add(8);
            }
            if !ended {
                return Err(format!("特效腳本 {:04X}h 沒有結束", fx_script_start(script)));
            }
        }
        Ok(())
    }

    fn parse_scenery(&mut self, img: &Image) -> Parsed<()> {
        self.scenery_groups.clear();
        let mut at = SCENERY_GROUPS;
        for _ in 0..500 {
            let (frame, record) = (img.u16(at)?, img.u16(at + 2)?);
            self.scenery_groups.push((frame, record));
            if frame == 0 {
                break;
            }
            let w = img.words(record, 4)?;
            self.scenery_records.insert(record, Prop::from_words([w[0], w[1], w[2], w[3]]));
            at += 4;
        }
        if self.scenery_groups.last().map(|g| g.0) != Some(0) {
            return Err("景物群組表沒有結束".into());
        }
        Ok(())
    }

    fn parse_player_shapes(&mut self, img: &Image) -> Parsed<()> {
        for who in 0..2 {
            for (k, shape) in PLAYER_SHAPES.into_iter().enumerate() {
                let ptr = player_shape_at(shape, who);
                self.take_shape(img, ptr)?;
                self.player_shapes[who][k] = self.shapes[&ptr].clone();
            }
        }
        Ok(())
    }

    /// 子彈腳本：低 4 位元是移動碼，高 4 位元為 F 是往回跳（本身是最後一個位元組），敵彈的 `00` 是結束。
    fn take_bullet_script(&mut self, img: &Image, start: u16, enemy: bool) -> Parsed<()> {
        if self.scripts.covers(start) {
            return Ok(());
        }
        let mut bytes = Vec::new();
        for k in 0..64u16 {
            let b = img.u8(start.wrapping_add(k))?;
            bytes.push(b);
            if b >> 4 == 0xF || (enemy && b == 0) {
                self.scripts.insert(start, bytes);
                return Ok(());
            }
            if b >> 4 != 0 {
                return Err(format!("子彈腳本 {start:04X}h 第 {k} 個位元組 {b:02X} 的高 4 位元不是 0 也不是 F"));
            }
        }
        Err(format!("子彈腳本 {start:04X}h 過長"))
    }

    fn parse_player_bullets(&mut self, img: &Image) -> Parsed<()> {
        for shot in SHOTS {
            for facing in [Facing::Right, Facing::Left] {
                let w = img.words(shot_template_at(shot, facing), 16)?;
                let bullet = PlayerBullet::from_words(std::array::from_fn(|k| w[k]));
                self.take_bullet_script(img, bullet.script, false)?;
                self.take_shape(img, bullet.shape)?;
                self.shots.insert((shot, facing), bullet);
            }
        }
        Ok(())
    }

    fn parse_enemy_bullets(&mut self, img: &Image) -> Parsed<()> {
        for template in all_bullet_templates() {
            let w = img.words(bullet_template_at(template), 15)?;
            let bullet = Bullet::from_words(std::array::from_fn(|k| w[k]));
            self.take_words(img, bullet.frames, 9)?;
            // 未使用的樣板（`Template::Unused`）的腳本指標不指向腳本，沒有任何路徑讀它。
            if template != Template::Unused {
                self.take_bullet_script(img, bullet.script, true)?;
            }
            self.take_shape(img, bullet.shape)?;
            self.bullets.insert(template, bullet);
        }
        Ok(())
    }

    /// 敵人：從時間表的樣板出發，跟著腳本裡的 `E0` 變身目標與 `F0` 跳轉目標收集。
    fn parse_enemies(&mut self, img: &Image) -> Parsed<()> {
        /// 待收的東西：一筆樣板，或一段從跳轉目標開始的腳本。
        enum Todo {
            Template(u16),
            Script(u16),
        }
        let mut todo = Vec::new();
        for table in TIMETABLES {
            let mut rows = Vec::new();
            let mut at = timetable_at(table);
            loop {
                let frame = img.u16(at)?;
                if frame == 0 {
                    break;
                }
                let template = img.u16(at.wrapping_add(2))?;
                rows.push((frame, template));
                todo.push(Todo::Template(template));
                at = at.wrapping_add(4);
                if rows.len() > 2000 {
                    return Err(format!("時間表 {:04X}h 過長", timetable_at(table)));
                }
            }
            self.timetables.insert(table, rows);
        }
        while let Some(item) = todo.pop() {
            match item {
                Todo::Template(ptr) => {
                    if self.enemies.contains_key(&ptr) {
                        continue;
                    }
                    let w = img.words(ptr, 24)?;
                    let enemy = Enemy::from_words(std::array::from_fn(|k| w[k]));
                    // 圖框表是 9 個字組。`K` 型（地面砲台）的圖框指標開頭是損壞貼片的 `(圖, x, y)` 三個字組，
                    // 圖框挑選仍以同一個指標讀 9 項（圖框值只進繪製值，那一型不畫），所以一樣收 9 個字組。
                    self.take_words(img, enemy.frames, 9)?;
                    if enemy.anim_limits != 0 {
                        self.take_words(img, enemy.anim_limits, 9)?;
                    }
                    self.take_shape(img, enemy.shape)?;
                    // `A` 型（道具載體）的腳本欄是初始朝向，不是指標。
                    if enemy.mode != u16::from(b'A') {
                        todo.push(Todo::Script(enemy.script));
                    }
                    self.enemies.insert(ptr, enemy);
                }
                Todo::Script(start) => {
                    let (morphs, jumps) = self.take_enemy_script(img, start)?;
                    todo.extend(morphs.into_iter().map(Todo::Template));
                    todo.extend(jumps.into_iter().map(Todo::Script));
                }
            }
        }
        Ok(())
    }

    /// 收一段敵人腳本，回傳 (變身目標, 跳轉目標)。
    fn take_enemy_script(&mut self, img: &Image, start: u16) -> Parsed<(Vec<u16>, Vec<u16>)> {
        if self.scripts.covers(start) {
            return Ok((Vec::new(), Vec::new()));
        }
        let (mut bytes, mut morphs, mut jumps) = (Vec::new(), Vec::new(), Vec::new());
        for k in 0..4000u16 {
            let at = start.wrapping_add(k);
            let b = img.u8(at)?;
            bytes.push(b);
            match b {
                0x0F => break,
                0xF0 | 0xE0 => {
                    let operand = img.u16(at.wrapping_add(1))?;
                    bytes.extend(operand.to_le_bytes());
                    if b == 0xF0 { jumps.push(operand) } else { morphs.push(operand) }
                    break;
                }
                _ => {}
            }
        }
        if !matches!(bytes.last(), Some(0x0F)) && jumps.is_empty() && morphs.is_empty() {
            return Err(format!("敵人腳本 {start:04X}h 沒有結束"));
        }
        self.scripts.insert(start, bytes);
        Ok((morphs, jumps))
    }
}

/// 從 `start` 起逐位元組收到 `stop` 成立的那一個（含）。
fn scan_until(img: &Image, start: u16, stop: impl Fn(u8) -> bool, limit: u16) -> Parsed<Vec<u8>> {
    let mut bytes = Vec::new();
    for k in 0..limit {
        let b = img.u8(start.wrapping_add(k))?;
        bytes.push(b);
        if stop(b) {
            return Ok(bytes);
        }
    }
    Err(format!("{start:04X}h 起 {limit} 個位元組內沒有結束碼"))
}

// ---- 查表 ----

fn missing(what: &str, key: u16) -> ! {
    panic!("表格 {what} 沒有位置 {key:04X}h（解析範圍之外）")
}

impl Tables {
    pub(crate) fn palette(&self, p: Palette) -> &PaletteData {
        &self.palettes[p as usize]
    }

    pub(crate) fn shape(&self, ptr: u16) -> &Shape {
        self.shapes.get(&ptr).unwrap_or_else(|| missing("形狀", ptr))
    }

    /// 圖框表或動畫上限表在位元組位移 `offset` 的字組（小端序，位移不一定對齊）。
    pub(crate) fn table_word(&self, ptr: u16, offset: u16) -> u16 {
        let bytes = self.word_tables.get(&ptr).unwrap_or_else(|| missing("字組表", ptr));
        let at = usize::from(offset);
        match (bytes.get(at), bytes.get(at + 1)) {
            (Some(&lo), Some(&hi)) => u16::from_le_bytes([lo, hi]),
            _ => panic!("字組表 {ptr:04X}h 沒有位移 {offset} 的字組"),
        }
    }

    /// 位元組腳本在 `at` 的位元組。
    pub(crate) fn script_byte(&self, at: u16) -> u8 {
        self.scripts.byte(at).unwrap_or_else(|| missing("腳本", at))
    }

    /// 位元組腳本在 `at` 的位元組，表外回傳 `None`（不 panic：偷看腳本開頭判斷類型用，腳本可能比讀的長度短）。
    pub(crate) fn script_byte_opt(&self, at: u16) -> Option<u8> {
        self.scripts.byte(at)
    }

    /// 位元組腳本在 `at` 的字組。
    pub(crate) fn script_word(&self, at: u16) -> u16 {
        u16::from_le_bytes([self.script_byte(at), self.script_byte(at.wrapping_add(1))])
    }

    pub(crate) fn fx_frame(&self, at: u16) -> FxFrame {
        *self.fx_frames.get(&at).unwrap_or_else(|| missing("特效腳本", at))
    }

    /// 特效腳本的起點位置。
    pub(crate) fn fx_start(&self, script: Script) -> u16 {
        fx_script_start(script)
    }

    pub(crate) fn enemy_template(&self, ptr: u16) -> Enemy {
        *self.enemies.get(&ptr).unwrap_or_else(|| missing("敵人樣板", ptr))
    }

    /// 時間表的列：`(幀, 樣板位置)`。
    pub(crate) fn timetable(&self, table: Timetable) -> &[(u16, u16)] {
        self.timetables.get(&table).map_or(&[], |v| v.as_slice())
    }

    pub(crate) fn bullet_template(&self, template: Template) -> Bullet {
        *self.bullets.get(&template).unwrap_or_else(|| panic!("表格沒有敵彈樣板 {template:?}"))
    }

    pub(crate) fn shot_template(&self, shot: Shot, facing: Facing) -> PlayerBullet {
        *self.shots.get(&(shot, facing)).unwrap_or_else(|| panic!("表格沒有玩家彈樣板 {shot:?} {facing:?}"))
    }

    /// 景物群組表第 `row` 列：(幀, 景物記錄位置)。幀為 0 是表尾。
    pub(crate) fn scenery_group(&self, row: u16) -> (u16, u16) {
        self.scenery_groups.get(usize::from(row)).copied().unwrap_or((0, 0))
    }

    pub(crate) fn scenery_record(&self, at: u16) -> Prop {
        *self.scenery_records.get(&at).unwrap_or_else(|| missing("景物記錄", at))
    }

    /// 第四關通道腳本第 `pos` 個位元組。
    pub(crate) fn tunnel_script_byte(&self, pos: u16) -> u8 {
        match self.tunnel.get(usize::from(pos)) {
            Some(&b) => b,
            None => panic!("通道腳本沒有第 {pos} 個位元組"),
        }
    }

    /// 頭目腳本的起點位置。
    pub(crate) fn boss_start(&self, index: usize) -> u16 {
        self.boss_starts[index]
    }

    pub(crate) fn player_shape(&self, shape: PlayerShape, player: usize) -> &Shape {
        &self.player_shapes[player.min(1)][shape as usize]
    }
}

/// 單元測試擺表格用：沒有原版檔，測試自己放需要的項目。
#[cfg(test)]
impl Tables {
    /// 空表格：兩種子彈樣板全為 0（零映像的樣子），其餘由測試自己擺。
    pub(crate) fn for_test() -> Tables {
        let mut t = Tables::default();
        for template in all_bullet_templates() {
            t.bullets.insert(template, Bullet::default());
        }
        for shot in SHOTS {
            for facing in [Facing::Right, Facing::Left] {
                t.shots.insert((shot, facing), PlayerBullet::default());
            }
        }
        t
    }

    /// 擺上原版的兩張列表（`列 × 28h`、`列 × 50h + 1`）。
    pub(crate) fn with_row_tables(mut self) -> Tables {
        for row in 0..GRID_ROWS {
            self.row_base[row] = row as u16 * 0x28;
            self.row_start[row] = row as u16 * 0x50 + 1;
        }
        self
    }

    /// 在 `ptr` 擺一個 `w` × `h` 的形狀，格值全為 `cell`。
    pub(crate) fn put_shape(&mut self, ptr: u16, w: u16, h: u16, cell: u8) {
        self.shapes.insert(ptr, Shape::new(w, h, vec![cell; usize::from(w * h)]));
    }

    /// 在 `ptr` 擺一個 `w` × `h` 的形狀，格值照 `cells`（列優先）。
    pub(crate) fn put_shape_cells(&mut self, ptr: u16, w: u16, h: u16, cells: &[u8]) {
        self.shapes.insert(ptr, Shape::new(w, h, cells.to_vec()));
    }

    /// 在 `at` 擺一段位元組腳本。
    pub(crate) fn put_bytes(&mut self, at: u16, bytes: &[u8]) {
        self.scripts.insert(at, bytes.to_vec());
    }

    /// 在 `ptr` 擺一張字組表（圖框表、動畫上限表）。
    pub(crate) fn put_words(&mut self, ptr: u16, words: &[u16]) {
        self.word_tables.insert(ptr, words.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>().into_boxed_slice());
    }

    /// 在 `at` 擺特效腳本的一格。
    pub(crate) fn put_fx_frame(&mut self, at: u16, image: u16, dx: u16, dy: u16, shape: u16) {
        self.fx_frames.insert(at, FxFrame { image, dx, dy, shape });
    }

    pub(crate) fn put_enemy(&mut self, ptr: u16, enemy: Enemy) {
        self.enemies.insert(ptr, enemy);
    }

    pub(crate) fn put_bullet(&mut self, template: Template, bullet: Bullet) {
        self.bullets.insert(template, bullet);
    }

    pub(crate) fn put_shot(&mut self, shot: Shot, facing: Facing, bullet: PlayerBullet) {
        self.shots.insert((shot, facing), bullet);
    }

    pub(crate) fn put_timetable(&mut self, table: Timetable, rows: &[(u16, u16)]) {
        self.timetables.insert(table, rows.to_vec());
    }

    /// 景物群組表第 `row` 列放 `(幀, 記錄位置)`，記錄是 `prop`。
    pub(crate) fn put_scenery_group(&mut self, row: u16, frame: u16, record: u16, prop: Prop) {
        let row = usize::from(row);
        if self.scenery_groups.len() <= row {
            self.scenery_groups.resize(row + 1, (0, 0));
        }
        self.scenery_groups[row] = (frame, record);
        self.scenery_records.insert(record, prop);
    }

    pub(crate) fn set_player_shape(&mut self, shape: PlayerShape, player: usize, value: Shape) {
        self.player_shapes[player.min(1)][shape as usize] = value;
    }

    pub(crate) fn put_fire_roll(&mut self, index: usize, value: u16) {
        self.fire_rolls[index] = value;
    }

    /// 調色盤 `p` 的 DAC 表從第 `at` 個位元組起換成 `bytes`。
    pub(crate) fn put_dac(&mut self, p: Palette, at: usize, bytes: &[u8]) {
        self.palettes[p as usize].dac[at..at + bytes.len()].copy_from_slice(bytes);
    }
}

impl Core {
    /// 讀一筆敵彈樣板。
    pub(crate) fn bullet_template(&self, template: Template) -> Bullet {
        self.tables.bullet_template(template)
    }

    /// 讀一筆敵人樣板（24 個字組）。樣板本身 23 個字組，相鄰樣板緊鄰排列，所以第 24 個字組（`spill`）
    /// 是下一個樣板的 `hp`（文件第 2.1 節）。
    pub(crate) fn enemy_template(&self, ptr: u16) -> Enemy {
        self.tables.enemy_template(ptr)
    }

    /// 讀一筆玩家彈樣板。
    pub(crate) fn player_bullet_template(&self, shot: Shot, facing: Facing) -> PlayerBullet {
        self.tables.shot_template(shot, facing)
    }

    /// 開火亂數表第 `index` 項（`index` < 75）。
    pub(crate) fn fire_roll_at(&self, index: u16) -> u16 {
        self.tables.fire_rolls[usize::from(index)]
    }

    pub(crate) fn tables(&self) -> Rc<Tables> {
        Rc::clone(&self.tables)
    }
}

// ---- 敵人角色（AI 強化用）----
//
// AI 強化要分得出頭目分件、頭目核心與普通敵人。敵人記錄不帶樣板位置，所以生成時依樣板位置分類一次，
// 結果存在 `ai.rs` 的 `AiState`。位置取自 `workspace/extracted/entities.json` 的 `timelines`
// 與文件 12 第 6 節的頭目表（靜態已證實）。

/// 第一關頭目：控制器（N）與六個損壞貼片（K）。
const STAGE1_BOSS_PARTS: [u16; 7] = [0x43E4, 0x45DE, 0x460C, 0x463A, 0x4668, 0x4696, 0x46C4];
/// 第一關頭目核心（`bullet` = `E`，死亡即過關）。
const STAGE1_BOSS_CORE: u16 = 0x47AA;
/// 第二關中頭目的控制器、損壞貼片與砲口（兩段各一份）。
const STAGE2_MID_PARTS: [u16; 10] = [0x59D4, 0x5A02, 0x5A5E, 0x5A8C, 0x5B16, 0x5ABA, 0x5AE8, 0x5B44, 0x5BA0, 0x5BCE];
/// 第二關頭目的控制器與損壞貼片。
const STAGE2_BOSS_PARTS: [u16; 2] = [0x5A30, 0x5B72];
/// 第二關頭目核心（K 型，`bullet` = `E`）。
const STAGE2_BOSS_CORE: u16 = 0x5BFC;
/// 第三關頭目的五個分件。
const STAGE3_BOSS_PARTS: [u16; 5] = [0x7168, 0x7196, 0x71C4, 0x71F2, 0x7220];
/// 第三關頭目核心（噴嘴，`bullet` = `E`）。
const STAGE3_BOSS_CORE: u16 = 0x724E;
/// 第四關中頭目（大魚）。
const STAGE4_MID_BOSS: u16 = 0x83E5;
/// 第四關頭目（圓盤，腳本 96 步後變身成自己）。
const STAGE4_BOSS_CORE: u16 = 0x8441;
/// 第四關附表（片尾的 END 字樣，`8D1Ch` 起）之後的樣板：不是戰鬥敵人。
const ENDING_TEMPLATES_FROM: u16 = 0x8D1C;

/// 生成敵人時依樣板位置分類。
pub(crate) fn enemy_role(template: u16) -> Role {
    if template == STAGE1_BOSS_CORE {
        Role::BossCore(Boss::Stage1)
    } else if template == STAGE2_BOSS_CORE {
        Role::BossCore(Boss::Stage2)
    } else if template == STAGE3_BOSS_CORE {
        Role::BossCore(Boss::Stage3)
    } else if template == STAGE4_BOSS_CORE {
        Role::BossCore(Boss::Stage4)
    } else if STAGE1_BOSS_PARTS.contains(&template)
        || STAGE2_MID_PARTS.contains(&template)
        || STAGE2_BOSS_PARTS.contains(&template)
        || STAGE3_BOSS_PARTS.contains(&template)
        || template == STAGE4_MID_BOSS
    {
        Role::BossPart
    } else if template >= ENDING_TEMPLATES_FROM {
        Role::Inert
    } else {
        Role::Grunt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::Palette;

    /// 需要原版檔的測試用：解包映像。
    fn original_image() -> Vec<u8> {
        let dir = std::path::PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh 跑"));
        rs2_formats::unpack_s_exe(&std::fs::read(dir.join("S.EXE")).unwrap()).unwrap()
    }

    #[test]
    fn enemy_bullet_template_positions_follow_the_original_table() {
        // docs/re/11-entities-combat.md 第 7.2 節與機器碼 16D3Bh 起的立即值。
        use Heading as H;
        assert_eq!(bullet_template_at(Template::Aimed(H::N)), 0x3557);
        assert_eq!(bullet_template_at(Template::Aimed(H::NW)), 0x3629);
        assert_eq!(bullet_template_at(Template::Capsule(Capsule::L, H::N)), 0x3737);
        assert_eq!(bullet_template_at(Template::Capsule(Capsule::B, H::N)), 0x3827);
        assert_eq!(bullet_template_at(Template::Capsule(Capsule::K, H::N)), 0x3917);
        assert_eq!(bullet_template_at(Template::Capsule(Capsule::S, H::NW)), 0x3A07 + 7 * 0x1E);
        assert_eq!(bullet_template_at(Template::Unused), 0x3AF7);
        assert_eq!(TILTS.map(|t| bullet_template_at(Template::Wide(t))), [0x3B15, 0x3B33, 0x3B51]);
        assert_eq!(SPREADS.map(|s| bullet_template_at(Template::Spread(s))), [0x3B6F, 0x3B8D, 0x3BAB, 0x3BC9, 0x3BE7]);
        assert_eq!(bullet_template_at(Template::WideFlash), 0x3C05);
        assert_eq!(bullet_template_at(Template::Flash), 0x3C23);
        assert_eq!(HEADINGS.map(|h| bullet_template_at(Template::Straight(h))), [0x3C41, 0x3C5F, 0x3C7D, 0x3C9B, 0x3CB9, 0x3CD7, 0x3CF5, 0x3D13]);
        assert_eq!([Template::SlowSw, Template::SlowW, Template::SlowNw].map(bullet_template_at), [0x3D31, 0x3D4F, 0x3D6D]);
        assert_eq!([Template::BouncerNw, Template::BouncerNe].map(bullet_template_at), [0x3D8B, 0x3DA9]);
        assert_eq!(all_bullet_templates().len(), 64);
    }

    #[test]
    fn player_shot_template_positions_follow_the_original_table() {
        // docs/re/11-entities-combat.md 第 6.3 節與機器碼 165CFh、1669Fh 的立即值。
        assert_eq!(SHOTS.map(|s| shot_template_at(s, Facing::Right)), [0x28D5, 0x2935, 0x2955, 0x2975, 0x2995, 0x29B5, 0x29D5, 0x2B35]);
        assert_eq!(SHOTS.map(|s| shot_template_at(s, Facing::Left)), [0x2A15, 0x2A75, 0x2A95, 0x2AB5, 0x2AD5, 0x2AF5, 0x29F5, 0x2B35]);
    }

    #[test]
    fn timetable_and_palette_positions_follow_the_original_wrappers() {
        assert_eq!(TIMETABLES.map(timetable_at), [0x41BE, 0x54FE, 0x68E0, 0x7922, 0x8D1C]);
        // 機器碼 `mov dx, 表`：1295Bh、12989h、129B7h、129E5h、12A13h、12A41h、12A6Fh、12A9Dh、12ACBh、143EAh、14419h。
        let dac = [
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
        .map(|p| palette_offsets(p).0);
        assert_eq!(dac, [0x5A9, 0x5EA, 0x62B, 0x66C, 0x6AD, 0x6EE, 0x72F, 0x770, 0x7B1, 0xE54, 0xE95]);
        // 開場與過場的前九張相鄰排列，每張 41h 位元組（11h 屬性表加 30h 的 DAC 值）。
        assert!(dac[..9].windows(2).all(|w| w[1] - w[0] == 0x41));
        // 屬性表在 DAC 表之前 11h 位元組，只有載入畫面例外。
        for p in Palette::ALL {
            let (dac, attr) = palette_offsets(p);
            assert_eq!(attr, if p == Palette::Loader { 0x056E } else { dac - 0x11 }, "{p:?}");
        }
    }

    #[test]
    fn a_non_original_image_is_rejected_before_anything_is_read() {
        let image = vec![0u8; DSEG_IMAGE_OFFSET + 0x10000];
        let err = Tables::parse(&image).expect_err("全 0 的映像不是原版");
        assert!(err.contains("解包映像"), "錯誤訊息要寫出是哪一份輸入：{err}");
        // 不驗雜湊的入口遇到結構不合的映像也回傳錯誤，不 panic。
        assert!(Tables::parse_unchecked(&image).is_err());
    }

    #[test]
    #[should_panic(expected = "解析範圍之外")]
    fn a_lookup_outside_the_parsed_tables_is_a_panic_not_a_zero() {
        Tables::default().shape(0x1234);
    }

    #[test]
    fn a_byte_script_covers_only_the_bytes_it_was_given() {
        let mut t = Tables::default();
        t.put_bytes(0x7000, &[1, 2, 3]);
        assert_eq!((t.script_byte(0x7000), t.script_byte(0x7002), t.script_word(0x7001)), (1, 3, 0x0302));
        assert!(std::panic::catch_unwind(move || t.script_byte(0x7003)).is_err(), "腳本之後的位置沒有解析過");
    }

    #[test]
    fn scenery_groups_and_records_come_from_the_tables() {
        let mut t = Tables::default();
        t.put_scenery_group(2, 385, 0x7000, Prop { image: 0x0E, x: 40, y: 20, dx: 4 });
        assert_eq!(t.scenery_group(2), (385, 0x7000));
        assert_eq!(t.scenery_group(0), (0, 0), "沒擺的列是表尾");
        assert_eq!(t.scenery_record(0x7000), Prop { image: 0x0E, x: 40, y: 20, dx: 4 });
    }

    #[test]
    fn the_original_image_parses_into_every_table_the_game_reads() {
        let image = original_image();
        let t = Tables::parse(&image).expect("原版映像要能解析");
        assert_eq!(t.row_base[3], 3 * 0x28);
        assert_eq!(t.row_start[3], 3 * 0x50 + 1);
        assert_eq!(t.integrity_sum, 0x227C, "完整性檢查的累加和等於檢查要求的值");
        assert!(t.fire_rolls.iter().all(|&v| v < 16), "固定亂數表的值域是 0 至 15");
        assert_eq!(all_bullet_templates().into_iter().filter(|&b| t.bullets.contains_key(&b)).count(), 64);
        assert_eq!(t.shots.len(), 16);
        let rows = |table| t.timetable(table).iter().filter(|r| r.0 != 0).count();
        assert_eq!(TIMETABLES.map(rows), [56, 90, 85, 111, 3]);
        assert_eq!(t.palette(Palette::Stage1).dac, std::array::from_fn(|k| image[DSEG_IMAGE_OFFSET + 0x04FD + k]));
        for p in Palette::ALL {
            assert_ne!(t.palette(p).dac, [0; DAC_BYTES], "{p:?} 的 DAC 表不該是空的");
        }
    }

    #[test]
    fn one_changed_byte_makes_the_original_image_unacceptable() {
        let image = original_image();
        assert!(Tables::parse(&image).is_ok());
        for at in [0, image.len() / 2, image.len() - 1] {
            let mut bad = image.clone();
            bad[at] ^= 0x01;
            assert!(Tables::parse(&bad).is_err(), "位移 {at} 改一個位元組要拒絕");
        }
    }
}
