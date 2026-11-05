//! 戰場格網：G1 實體格、G2 敵彈格、G3 髒格、G4 地形格。
//!
//! 四張格網原版都在 16 位元資料段裡，位址運算全是 16 位元環繞。玩家彈、玩家掃描與敵彈的列起點會別名到
//! 別的格網或位址空間（見各函式的文件），所以位址在這裡仍用原版的 16 位元位置表示，三塊記憶體各有一段
//! 位址範圍（[`Grids`] 的記憶體圖）。位址落在三塊之外的讀取回傳 0（原版是變數之間的零值間隙，沒有寫入者），
//! 寫入丟掉。這是遊戲裡唯一留著原版資料段位置的地方，因為越界別名的算法本身就是對那組位置做 16 位元運算。
//!
//! 三塊記憶體：
//!
//! | 區塊 | 內容 |
//! |---|---|
//! | `g1` | G1，20 列 × 40 欄，每格一位元組 |
//! | `g2` | G2，同上，加上一格寫得到的溢位格 |
//! | `terrain` | G4（20 列 × 40 欄 × 2 位元組）、G3（同）、第三關新欄的寫入游標（一個字組）依序相鄰 |
//!
//! G4、G3 與游標相鄰是原版行為的前提：第三關捲動時 G4 的最後一個字組取自 G3 的第一個，G3 的最後一個字組取自游標。
//!
//! # 戰場加大（規格 005「戰場大小」）
//!
//! 欄數不是 40 時（48 或 60，進關時由 [`Core::enter_field`] 決定，存在 [`Grids::cols`]），格網改用新寬度，
//! 位址仍是 16 位元的位置，只是起點相同、列距與大小隨欄數（[`Layout`]）。越界一律乾淨處理：
//! 列不在 0 至 19 的格回傳一個不在任何格網內的位址（[`VOID`]），讀回 0、寫入丟掉，
//! 不模擬原版把列基底表、死亡旗標與 G1 的內容當位址的別名（只有欄數為 40 時存在）。

use crate::draw::DrawCmd;
use crate::game::Core;

/// G1 實體格網的起點位置與大小：20 列，列距 40。
pub(crate) const GRID1_START: u16 = 0x1B32;
const GRID1_BYTES: u16 = 0x320;
/// G1 的終點（不含）。玩家彈的碰撞測只看 19 列，G1 最後一列之後不算。
pub(crate) const GRID1_END: u16 = 0x1E2A;
/// G1 與 G2 的列距：40 欄，每格一位元組。
pub(crate) const GRID1_ROW_BYTES: u16 = 0x28;
/// 戰場的欄數，G1、G2、G3、G4 都是 40 欄。敵人的足跡超出這個範圍的格不寫。
pub(crate) const GRID_COLS: u16 = 0x28;
/// 一列最後可測的格相對列起點的位移加 1：列尾（欄 39）嚴格在它之前。
const GRID1_ROW_SPAN: u16 = 0x27;
/// 玩家掃描用的格網窗與列寬：玩家掃 G1、G2 時，位址要嚴格在窗的起點與上限之間，也嚴格在列起點加 45h 之前
/// （比玩家彈的 27h 寬）。
const SCAN_ROW_SPAN: u16 = 0x45;

/// G2 敵彈格網（40 欄 × 20 列，每格一位元組，列距 28h）。
pub(crate) const GRID2_START: u16 = 0x2C1C;
const GRID2_BYTES: u16 = 0x320;
/// 敵彈寫 G2 的位址上限（含）。比 G2 的最後一格再多一格，那一格是 `g2` 的溢位格。
const GRID2_LAST_WRITABLE: u16 = GRID2_START + GRID2_BYTES;
/// 可寫的欄範圍是列基底之後、列基底 + 27h 之前（欄 1 至 38）。
const GRID2_ROW_END: u16 = 0x27;

/// G4 地形格（第三關，每列 50h 位元組，奇數位址的字組）的範圍，閉區間。
pub(crate) const GRID4_START: u16 = 0x900C;
const GRID4_LAST: u16 = 0x95FC;
/// G4 的列距：40 欄 × 2 位元組。第三關捲動的新欄與碰撞讀取都用這個列距。
pub(crate) const TERRAIN_ROW_BYTES: u16 = 0x50;
/// G4 與 G3 各 20 列 × 40 欄 = 320h 個字組。
const TERRAIN_WORDS: u16 = 0x320;
/// G3 格網（碰撞用的髒格，每列 50h 位元組，奇數位元組）的範圍。
pub(crate) const GRID3_START: u16 = 0x964C;
const GRID3_END: u16 = 0x9C3C;
/// 一列 40 格，每格 2 位元組，所以列距 50h，列內可寫的範圍是 `[第一格, 第一格 + 4Eh]`。
pub(crate) const GRID3_ROW_BYTES: u16 = 0x50;
const GRID3_ROW_LAST_CELL: u16 = 0x4E;
/// 第三關新欄的寫入游標（位元組位移），捲動步開始設成一列（50h），每寫一格加一列。位置緊接在 G3 之後。
const TERRAIN_CURSOR: u16 = GRID3_START + 2 * TERRAIN_WORDS;
/// `terrain` 區塊的總長：G4、G3 與游標。
const TERRAIN_BLOCK: usize = 2 * (2 * TERRAIN_WORDS as usize) + 2;

/// 列基底表的位置：緊接在 G1 之前。
const ROW_BASE_TABLE: u16 = 0x1B0A;
/// 列基底表：每列一個字組，`列 × 28h`，20 項，G1、G2 的列起點。
/// 第 20 項（索引 14h）的位置就是 G1 的前兩個位元組，索引 14h 到 17Bh 因此讀到 G1 自己的內容。
const ROW_BASE_COUNT: u16 = 0x14;
/// 玩家彈與玩家掃描取列基底時，列（無號）大於等於這個值一律改取 [`ROW_BASE_FAR`]。
const ROW_BASE_NEAR_LIMIT: u16 = 0x17C;
/// 越界列的索引：列基底表之後第 1F4h 項，在 G1 之後的零值間隙，實測為 0。
const ROW_BASE_FAR: u16 = 0x1F4;
/// 列基底表的第 1F4h 項的值。
const ROW_BASE_FAR_VALUE: u16 = 0;
/// 列號（無號）大於等於這個值，G3、G4、敵彈寫入 G2 一律當作這一列。
pub(crate) const GRID_LAST_ROW: u16 = 0x14;
/// 第三關地圖視窗比戰場寬幾欄（畫面右緣外多畫的兩欄）。
pub(crate) const WINDOW_EXTRA_COLS: u16 = 2;

/// 戰場加大時，列不在格網內的格的位址：不在三塊記憶體內，讀回 0，寫入丟掉。
const VOID: u16 = 0;

/// 玩家掃描的格網。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanGrid {
    /// G1 實體格：窗的上限 `217Ch`，比 G1 的終點多出去的部分是 G1 之後的零值間隙。
    Entities,
    /// G2 敵彈格：窗的上限 `3266h`，G2 之後直到敵彈池指標表之前的零值間隙。
    Bullets,
}

impl ScanGrid {
    const fn start(self) -> u16 {
        match self {
            ScanGrid::Entities => GRID1_START,
            ScanGrid::Bullets => GRID2_START,
        }
    }

    /// 窗的上限（不含）。欄數為 40 是原版的值；加大時就是格網的終點，沒有間隙。
    const fn limit(self, lay: Layout) -> u16 {
        if lay.wide() {
            return self.start() + 20 * lay.cols;
        }
        match self {
            ScanGrid::Entities => 0x217C,
            ScanGrid::Bullets => 0x3266,
        }
    }
}

/// 格網的版面：全由欄數決定。欄數 40 是原版，每個值都等於上面的常數；其他欄數是戰場加大（規格 005）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) cols: u16,
}

impl Layout {
    pub(crate) const fn new(cols: u16) -> Layout {
        Layout { cols }
    }

    /// 戰場加大：不是原版的 40 欄。
    pub(crate) const fn wide(self) -> bool {
        self.cols != GRID_COLS
    }

    /// G1、G2 每列的位元組數，也是列距。
    pub(crate) const fn row_bytes(self) -> u16 {
        self.cols
    }

    const fn g1_bytes(self) -> u16 {
        20 * self.cols
    }

    /// G1 的終點（不含）：玩家彈的碰撞測只看 19 列。
    pub(crate) const fn g1_end(self) -> u16 {
        GRID1_START + 19 * self.cols
    }

    /// 一列最後可測的格相對列起點的位移加 1：列尾（最後一欄）嚴格在它之前。
    pub(crate) const fn row_span(self) -> u16 {
        self.cols - 1
    }

    const fn g2_bytes(self) -> u16 {
        20 * self.cols
    }

    /// 敵彈寫 G2 的位址上限（含）。
    const fn g2_last_writable(self) -> u16 {
        GRID2_START + self.g2_bytes()
    }

    /// G2 可寫的欄範圍是列基底之後、列基底加這個值之前（欄 1 至 `cols - 2`）。
    const fn g2_row_end(self) -> u16 {
        self.cols - 1
    }

    /// 玩家掃描時，位址要嚴格在列起點加這個值之前。原版 45h，比一列寬；加大時就是一列。
    const fn scan_row_span(self) -> u16 {
        if self.wide() { self.cols } else { SCAN_ROW_SPAN }
    }

    /// G4 與 G3 各自的字組數。
    const fn terrain_words(self) -> u16 {
        20 * self.cols
    }

    /// G4 與 G3 的列距：每格 2 位元組。
    pub(crate) const fn terrain_row_bytes(self) -> u16 {
        2 * self.cols
    }

    pub(crate) const fn g3_start(self) -> u16 {
        GRID4_START + 2 * self.terrain_words()
    }

    const fn g3_end(self) -> u16 {
        self.g3_start() + 19 * self.terrain_row_bytes()
    }

    const fn g4_last(self) -> u16 {
        GRID4_START + 19 * self.terrain_row_bytes()
    }

    /// 列內可寫的範圍是 `[第一格, 第一格 + 這個值]`。
    const fn g3_row_last_cell(self) -> u16 {
        2 * (self.cols - 1)
    }

    /// 第三關新欄的寫入游標（位元組位移），位置緊接在 G3 之後。
    const fn terrain_cursor(self) -> u16 {
        self.g3_start() + 2 * self.terrain_words()
    }

    const fn terrain_block(self) -> usize {
        2 * (2 * self.terrain_words() as usize) + 2
    }
}

/// 四張格網的記憶體。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grids {
    pub g1: Vec<u8>,
    pub g2: Vec<u8>,
    pub terrain: Vec<u8>,
    /// 這一關的欄數：40 是原版，戰場加大時是 48 或 60。進關時由 [`Core::enter_field`] 設定，關內不變。
    cols: u16,
}

impl Default for Grids {
    fn default() -> Grids {
        Grids::with_cols(GRID_COLS)
    }
}

/// 位址所在的記憶體區塊與區塊內的位移。
enum Spot {
    G1(usize),
    G2(usize),
    Terrain(usize),
    Gap,
}

impl Grids {
    /// 全部清 0、欄數為 `cols` 的格網。
    pub fn with_cols(cols: u16) -> Grids {
        let lay = Layout::new(cols);
        Grids {
            g1: vec![0; usize::from(lay.g1_bytes())],
            g2: vec![0; usize::from(lay.g2_bytes()) + 1],
            terrain: vec![0; lay.terrain_block()],
            cols,
        }
    }

    /// 這一關的欄數。
    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub(crate) fn layout(&self) -> Layout {
        Layout::new(self.cols)
    }

    fn locate(&self, addr: u16) -> Spot {
        let lay = self.layout();
        let within = |start: u16, len: usize| {
            let off = usize::from(addr.wrapping_sub(start));
            (off < len).then_some(off)
        };
        if let Some(off) = within(GRID1_START, usize::from(lay.g1_bytes())) {
            Spot::G1(off)
        } else if let Some(off) = within(GRID2_START, usize::from(lay.g2_bytes()) + 1) {
            Spot::G2(off)
        } else if let Some(off) = within(GRID4_START, lay.terrain_block()) {
            Spot::Terrain(off)
        } else {
            Spot::Gap
        }
    }

    /// 位址上的位元組。三塊記憶體之外是零值間隙。
    pub(crate) fn peek8(&self, addr: u16) -> u8 {
        match self.locate(addr) {
            Spot::G1(i) => self.g1[i],
            Spot::G2(i) => self.g2[i],
            Spot::Terrain(i) => self.terrain[i],
            Spot::Gap => 0,
        }
    }

    /// 位址上的字組，小端序，位址環繞。
    pub(crate) fn peek16(&self, addr: u16) -> u16 {
        u16::from_le_bytes([self.peek8(addr), self.peek8(addr.wrapping_add(1))])
    }

    /// 寫位址上的位元組。三塊記憶體之外丟掉。
    pub(crate) fn poke8(&mut self, addr: u16, v: u8) {
        match self.locate(addr) {
            Spot::G1(i) => self.g1[i] = v,
            Spot::G2(i) => self.g2[i] = v,
            Spot::Terrain(i) => self.terrain[i] = v,
            Spot::Gap => {}
        }
    }

    pub(crate) fn poke16(&mut self, addr: u16, v: u16) {
        let [lo, hi] = v.to_le_bytes();
        self.poke8(addr, lo);
        self.poke8(addr.wrapping_add(1), hi);
    }

    /// 第三關新欄的寫入游標。
    pub(crate) fn terrain_cursor(&self) -> u16 {
        self.peek16(self.layout().terrain_cursor())
    }

    pub(crate) fn set_terrain_cursor(&mut self, v: u16) {
        self.poke16(self.layout().terrain_cursor(), v);
    }

    /// `rep movsw`：逐字組複製，重疊時行為與 8086 相同。
    fn copy_words(&mut self, dst: u16, src: u16, words: u16) {
        let (mut d, mut s) = (dst, src);
        for _ in 0..words {
            let v = self.peek16(s);
            self.poke16(d, v);
            d = d.wrapping_add(2);
            s = s.wrapping_add(2);
        }
    }
}

impl Core {
    /// 目前這一關的格網版面。
    pub(crate) fn layout(&self) -> Layout {
        self.grids.layout()
    }

    /// 這一關的戰場欄數：40 是原版，戰場加大時是 48 或 60（規格 005「戰場大小」）。
    pub fn field_cols(&self) -> u16 {
        self.grids.cols()
    }

    /// 戰場比原版多出來的欄數（`field_cols() - 40`）：右緣、敵人進場點與頭目都跟著往右移這麼多。
    pub(crate) fn field_extra(&self) -> u16 {
        self.grids.cols() - GRID_COLS
    }

    /// 一列 G1 的列尾位址（不含），欄數隨這一關。
    pub(crate) fn grid1_row_end_of(&self, row_start: u16) -> u16 {
        row_start.wrapping_add(self.layout().row_span())
    }

    /// 子彈左右出界的門檻（無號比較）：欄大於等於它就出界。原版 27h，戰場加大時是欄數減 1。
    pub(crate) fn side_limit(&self) -> u16 {
        self.layout().row_span()
    }

    /// 進關時決定這一關的戰場欄數（規格 005「戰場大小」），關內不變。
    ///
    /// 欄數 40 接 40 什麼都不做，和原版逐位元組相同。欄數變了就換一組全清的格網，修正模式另存的敵彈槽號跟著換大小，
    /// 並送出 [`DrawCmd::Field`] 讓接收端換成這個寬度。戰場是加大的就每關都送一次，接收端對同一個欄數重送是空動作。
    pub(crate) fn enter_field(&mut self, cols: u16) {
        let now = self.grids.cols();
        if cols == now && cols == GRID_COLS {
            return;
        }
        if cols != now {
            self.grids = Grids::with_cols(cols);
        }
        self.sync_g2_slots();
        self.draws.push(DrawCmd::Field { cols });
    }

    /// 修正模式另存的敵彈槽號，每個 G2 格一項（加一個溢位格），大小跟著欄數。
    pub(crate) fn sync_g2_slots(&mut self) {
        let want = 20 * usize::from(self.grids.cols()) + 1;
        if self.g2_slots.len() != want {
            self.g2_slots = vec![0; want];
        }
    }

    /// 進關：依強化選項（[`Rules::field`](crate::game::Rules)）決定這一關的欄數。展示錄影一律 100%（`Core::rules`）。
    pub(crate) fn begin_stage_field(&mut self) {
        let cols = self.rules().field.cols();
        self.enter_field(cols);
    }

    /// 離開關卡時回到原版的欄數，標題、過場與片尾都是 40 欄。
    pub(crate) fn leave_field(&mut self) {
        self.enter_field(GRID_COLS);
    }

    // ---- G1 實體格 ----

    /// G1 某一列的起點（原版 `sub_16CC6`，玩家彈用）。列（無號）大於等於 17Ch 改取索引 1F4h；
    /// 小於的列直接以列號索引列基底表，所以列 14h 至 17Bh 讀到的是 G1 自己的內容，原版行為。
    /// 列基底表第 14h 項就是 G1 的前兩個位元組，列 0 之前的負列落在越界列。
    ///
    /// 戰場加大時列不在 0 至 19 就回傳 [`VOID`]，沒有別名。
    pub(crate) fn grid1_row_start(&self, row: u16) -> u16 {
        let lay = self.layout();
        if lay.wide() {
            return if row < GRID_LAST_ROW { GRID1_START + row * lay.cols } else { VOID };
        }
        self.row_base_offset(row).wrapping_add(GRID1_START)
    }

    /// 列基底表的第 `row` 項：列（無號）大於等於 17Ch，包含負列，一律取索引 1F4h。
    /// G1、G2 的玩家掃描與玩家彈共用。
    ///
    /// 索引 0 至 13h 是表的 20 項；索引 14h 至 17Bh 是 G1 自己的位元組（索引 14h 對應 G1 的第 0 個位元組，
    /// 每項兩個位元組）；索引 1F4h 是 G1 之後的零值間隙。
    fn row_base_offset(&self, row: u16) -> u16 {
        let index = if row >= ROW_BASE_NEAR_LIMIT { ROW_BASE_FAR } else { row };
        if index < ROW_BASE_COUNT {
            self.tables.row_base[usize::from(index)]
        } else if index == ROW_BASE_FAR {
            ROW_BASE_FAR_VALUE
        } else {
            let at = 2 * usize::from(index - ROW_BASE_COUNT);
            u16::from_le_bytes([self.grids.g1[at], self.grids.g1[at + 1]])
        }
    }

    /// 玩家掃描的一格（原版 `sub_14C61`、`sub_14C3D`、`sub_14D68`、`sub_14BF4` 一族算位址，加上迴圈裡的範圍檢查）：
    /// 位址須嚴格在格網窗之內，也嚴格在該列起點之後、列起點加 45h 之前。窗比格網大，列加 45h 比一列的 28h 寬，
    /// 所以位置正常時範圍檢查都成立；列號落在表外時掃到的是格網之外的資料，照原版。不合格回傳 `None`。
    pub(crate) fn scan_cell(&self, grid: ScanGrid, row_start: u16, addr: u16) -> Option<u8> {
        let lay = self.layout();
        let inside = addr < grid.limit(lay) && addr > grid.start() && addr < row_start.wrapping_add(lay.scan_row_span()) && addr > row_start;
        inside.then(|| self.grids.peek8(addr))
    }

    /// 玩家掃描時第 `row` 列的起點：列基底表加格網起點。戰場加大時列不在 0 至 19 回傳 [`VOID`]。
    pub(crate) fn scan_row_start(&self, grid: ScanGrid, row: u16) -> u16 {
        let lay = self.layout();
        if lay.wide() {
            return if row < GRID_LAST_ROW { grid.start() + row * lay.cols } else { VOID };
        }
        self.row_base_offset(row).wrapping_add(grid.start())
    }

    /// `addr` 的 G1 格值，沒有範圍檢查（追蹤彈掃描與雷射掃描直接讀）。
    ///
    /// 列基底表緊接在 G1 之前（20 個字組，小端序），所以 G1 起點的前一個位元組是表最後一項的高位元組（02h）。
    /// 子彈欄為負（飛出左緣）時，追蹤彈的第一次讀取（`cell = 列起點 + 欄`）落在那裡，讀到非 0 就當作撞牆，
    /// 原版因此把它判成距離 0 的牆。G1 之後的位址（G1 之後直到 `217Ch` 的間隙）是 0。
    pub(crate) fn grid1_byte(&self, addr: u16) -> u8 {
        if self.layout().wide() {
            return self.grids.peek8(addr);
        }
        let off = usize::from(addr.wrapping_sub(ROW_BASE_TABLE));
        match self.tables.row_base.get(off / 2) {
            Some(word) if off < 2 * usize::from(ROW_BASE_COUNT) => word.to_le_bytes()[off % 2],
            _ => self.grids.peek8(addr),
        }
    }

    /// 玩家彈碰撞測的 G1 格（原版 `sub_16B49`）：位址須在 G1 之內，嚴格在該列起點之後、嚴格在列尾之前。
    /// 不合格回傳 `None`。
    pub(crate) fn grid1_probe(&self, row_start: u16, addr: u16) -> Option<u8> {
        let lay = self.layout();
        let inside = addr < lay.g1_end() && addr > GRID1_START && addr < row_start.wrapping_add(lay.row_span()) && addr > row_start;
        inside.then(|| self.grids.peek8(addr))
    }

    /// 清空 G1（原版 `sub_162B1`，每幀末）：從 `1B32h` 起 190h 個字組，比有人寫入的 19 列多一列。
    pub(crate) fn clear_grid1(&mut self) {
        self.grids.g1.fill(0);
    }

    /// 敵人把形狀的一格值寫進 G1（原版 `sub_16233`）：覆蓋，不是 `or`，所以槽號大的敵人蓋過槽號小的。
    /// 欄（無號）大於 27h 的格略過，不會溢進相鄰的列；列起點取自列基底表（[`Core::grid1_row_start`]）。
    pub(crate) fn grid1_put(&mut self, row: u16, col: u16, v: u8) {
        if col < self.layout().cols {
            let at = self.grid1_row_start(row).wrapping_add(col);
            self.grids.poke8(at, v);
        }
    }

    // ---- G2 敵彈格 ----

    /// 清 G2（原版 `sub_178BB`，每幀末）。修正模式另存的槽號一起清。
    pub(crate) fn clear_grid2(&mut self) {
        let bytes = usize::from(self.layout().g2_bytes());
        self.grids.g2[..bytes].fill(0);
        self.g2_slots.fill(0);
    }

    /// G2 某一列的基底位址（原版 `sub_17898`）。列（無號）大於等於 14h，含負列，改取索引 14h；
    /// 該項的位址是 G1 的前兩個位元組，所以列基底取到的是 G1 的內容，原版行為。
    /// 道具球 2×3 在列 18 的移除幀會寫到列 20，走到這個分支。戰場加大時列不在 0 至 19 回傳 [`VOID`]，不寫。
    pub(crate) fn grid2_row_base(&self, row: u16) -> u16 {
        let lay = self.layout();
        if lay.wide() {
            return if row < GRID_LAST_ROW { GRID2_START + row * lay.cols } else { VOID };
        }
        self.row_base_offset(row.min(GRID_LAST_ROW)).wrapping_add(GRID2_START)
    }

    /// `addr` 是否可寫：落在 `2C1Ch` 至 `2F3Ch` 之間，且嚴格在列基底之後（欄 0 不寫）、嚴格在列基底 + 27h 之前。
    /// 戰場加大時上限與列寬隨欄數（欄 1 至 `cols - 2`）。
    pub(crate) fn grid2_writable(&self, row_base: u16, addr: u16) -> bool {
        let lay = self.layout();
        addr <= lay.g2_last_writable() && addr >= GRID2_START && addr < row_base.wrapping_add(lay.g2_row_end()) && addr > row_base
    }

    pub(crate) fn grid2_write(&mut self, addr: u16, v: u8) {
        self.grids.poke8(addr, v);
    }

    /// 可寫位址上的 G2 格值。
    #[cfg(test)]
    pub(crate) fn grid2_byte(&self, addr: u16) -> u8 {
        self.grids.peek8(addr)
    }

    /// `addr` 在 G2 裡的索引，`g2_slots` 用。
    pub(crate) fn grid2_index(&self, addr: u16) -> usize {
        addr.wrapping_sub(GRID2_START) as usize
    }

    // ---- G3 髒格 ----

    /// 列起點表（`21C2h`，G3 與 G4 共用）的第 `row` 項，`row` 已限制在 0 至 14h。列 0 至 13h 從 0001h 起每列加 50h。
    /// 第 21 項（列 14h，越界列）的位置就是敵人的死亡旗標 `word_1EDAA`，映像裡的初值是 0059h，
    /// 但敵人每更新一個就把它寫成 `N`（004Eh）或 `Y`（0059h），所以越界列的起點隨敵人的狀態變。
    fn row_start_word(&self, row: u16) -> u16 {
        if row >= GRID_LAST_ROW { self.en.death_flag } else { self.tables.row_start[usize::from(row)] }
    }

    /// 戰場加大時 G3、G4 一格的位址：`start` 是格網的起點，列不在 0 至 19 回傳 [`VOID`]。
    /// 列內的欄不在這裡擋，沿用原版的 16 位元環繞，由 [`Core::grid3_or_in_row`]、[`Core::grid4_contains`] 擋。
    fn wide_terrain_cell(&self, start: u16, row: u16, col: u16) -> u16 {
        if row >= GRID_LAST_ROW {
            return VOID;
        }
        let row_bytes = self.layout().terrain_row_bytes();
        start.wrapping_add(row * row_bytes + 1).wrapping_add(col.wrapping_shl(1).wrapping_add(1))
    }

    /// 記下最近一個更新的敵人本幀要不要釋放（原版 `word_1EDAA`）：`Y` 釋放，`N` 不釋放。
    /// 列起點表的越界列（G3、G4 的列 14h 以上）讀的就是這個字組。
    pub(crate) fn set_enemy_death_flag(&mut self, dying: bool) {
        self.en.death_flag = if dying { u16::from(b'Y') } else { u16::from(b'N') };
    }

    /// 以 `or` 寫進 G3 的一格，限定在 `first`（該列第一格的位址）起的這一列內。
    /// 敵彈用這個：超出該列右緣的格略過，不溢進下一列。
    pub(crate) fn grid3_or_in_row(&mut self, first: u16, addr: u16, v: u8) {
        if first <= addr && addr <= first.wrapping_add(self.layout().g3_row_last_cell()) {
            let old = self.grids.peek8(addr);
            self.grids.poke8(addr, old | v);
        }
    }

    /// G3 的格位址（原版 `sub_17D5F`）：`列` 大於等於 14h 改取 14h，位址 = G3 起點 + 列起點表 + 2 × 欄 + 1。
    /// 全部以 16 位元環繞運算，所以越界的位址由 [`Core::grid3_or`] 擋掉。戰場加大時列不在 0 至 19 回傳 [`VOID`]。
    pub(crate) fn grid3_cell_addr(&self, row: u16, col: u16) -> u16 {
        let lay = self.layout();
        if lay.wide() {
            return self.wide_terrain_cell(lay.g3_start(), row, col);
        }
        let row_start = self.row_start_word(row.min(GRID_LAST_ROW));
        row_start.wrapping_add(GRID3_START).wrapping_add(col.wrapping_shl(1).wrapping_add(1))
    }

    /// 以 `or` 寫進 G3 的一格。位址落在 G3 範圍之外的格略過。
    pub(crate) fn grid3_or(&mut self, addr: u16, v: u8) {
        let lay = self.layout();
        if (lay.g3_start()..=lay.g3_end()).contains(&addr) {
            let old = self.grids.peek8(addr);
            self.grids.poke8(addr, old | v);
        }
    }

    /// 玩家本體佔的 G3 髒格（原版 `sub_14A2C`、`sub_1555A`）：從 (`row`, `col`) 起 `rows` 列 × `cols` 欄，
    /// 每格寫一個字組 `value`，連奇數位元組一起覆寫。位址是 G3 起點加 `2 × (低位元組(row) × 28h + col + 1)`，
    /// 不經列起點表也不檢查範圍。
    ///
    /// 列號超出畫面時（例如死亡動畫往下走）寫得到 G3 之後的位址：先是游標，再往後是別的變數。
    /// 游標之後的位置原版是繪製與喇叭參數等變數，重製不維護它們的位元組（它們有自己的欄位），寫入丟掉。
    /// 這條路徑沒有收據分辨，是靜態推論。
    ///
    /// 戰場加大時改成乾淨的邊界：列在 0 至 18、欄在 0 至 `cols - 1` 的格才寫，其餘丟掉（G3 沒有人寫列 19，
    /// 再往下是游標）。
    pub(crate) fn grid3_mark_block(&mut self, row: u16, col: u16, rows: u16, cols: u16, value: u16) {
        let lay = self.layout();
        if lay.wide() {
            for r in 0..rows {
                for c in 0..cols {
                    let (rr, cc) = (i32::from(row as i16) + i32::from(r), i32::from(col as i16) + i32::from(c));
                    if (0..19).contains(&rr) && (0..i32::from(lay.cols)).contains(&cc) {
                        let at = self.wide_terrain_cell(lay.g3_start(), rr as u16, cc as u16);
                        self.grids.poke16(at, value);
                    }
                }
            }
            return;
        }
        let first = (u16::from(row as u8) * GRID_COLS).wrapping_add(col).wrapping_add(1).wrapping_shl(1);
        for r in 0..rows {
            for c in 0..cols {
                let at = first.wrapping_add(r * GRID3_ROW_BYTES).wrapping_add(c * 2);
                self.grids.poke16(GRID3_START.wrapping_add(at), value);
            }
        }
    }

    // ---- G4 地形格與第三關捲動 ----

    /// G4 地形格的位址（原版 `sub_16A2C`）：`列` 大於等於 14h 改取 14h，位址 = G4 起點 + 列起點表 + 2 × 欄 + 1。
    /// 與 G3 共用列起點表（[`Core::grid3_cell_addr`]）。戰場加大時列不在 0 至 19 回傳 [`VOID`]。
    pub(crate) fn grid4_cell_addr(&self, row: u16, col: u16) -> u16 {
        if self.layout().wide() {
            return self.wide_terrain_cell(GRID4_START, row, col);
        }
        let row_start = self.row_start_word(row.min(GRID_LAST_ROW));
        row_start.wrapping_add(GRID4_START).wrapping_add(col.wrapping_shl(1).wrapping_add(1))
    }

    /// G4 在 `addr` 的字組，沒有範圍檢查。
    pub(crate) fn grid4_word(&self, addr: u16) -> u16 {
        self.grids.peek16(addr)
    }

    /// `addr` 是否在 G4 的範圍內（原版 `169E0h` 的兩個比較）。
    pub(crate) fn grid4_contains(&self, addr: u16) -> bool {
        (GRID4_START..=self.layout().g4_last()).contains(&addr)
    }

    /// 第三關捲動一格：G4 與 G3 各整體左移一個字組（原版 `sub_1840E` 的兩次 `rep movsw`，各 320h 個字組）。
    /// 兩次搬移的讀取都超出自己的範圍，照原版：
    ///
    /// - G4 的最後一個字組取自 G3 的第一個字組（G3 起點，在 G3 左移之前讀）。
    /// - G3 的最後一個字組取自 G3 之後的游標（G3 起點加 640h），所以 G3 最後一格的髒標記是新欄寫入游標的值，
    ///   那個位置沒有找到其他引用。
    /// - 結尾的 `stosb` 把游標的低位元組清 0；`scroll_map_second_page` 隨後才把游標設成 50h，所以沒有後續效果。
    pub(crate) fn scroll_terrain_grids(&mut self) {
        let lay = self.layout();
        self.grids.copy_words(GRID4_START, GRID4_START + 2, lay.terrain_words());
        self.grids.copy_words(lay.g3_start(), lay.g3_start() + 2, lay.terrain_words());
        self.grids.poke8(lay.terrain_cursor(), 0);
    }

    /// 清第三關的地形格 G4（原版 `sub_18599`）：從 G4 起點起 320h 個字組。每個關卡進場都清。
    pub(crate) fn clear_terrain_grid(&mut self) {
        let len = 2 * usize::from(self.layout().terrain_words());
        self.grids.terrain[..len].fill(0);
    }

    /// 新欄的第一格從游標 50h 起，也就是 G4 讀取格狀的（列 0，欄 39）（原版 `18306h`）。
    /// 戰場加大時是列 0 的最後一欄，游標是一列的位元組數。
    pub(crate) fn start_terrain_column(&mut self) {
        self.grids.set_terrain_cursor(self.layout().terrain_row_bytes());
    }

    /// 新欄的一格圖塊索引存進 G4（原版 `sub_1839F`）：寫在 G4 起點加游標，游標加一列。
    pub(crate) fn store_terrain_tile(&mut self, tile: u16) {
        let cursor = self.grids.terrain_cursor();
        self.grids.poke16(cursor.wrapping_add(GRID4_START), tile);
        self.grids.set_terrain_cursor(cursor.wrapping_add(self.layout().terrain_row_bytes()));
    }

    /// 初始視窗的第 `cell` 格圖塊索引存進 G4（原版 `sub_17DF9`）：從 G4 起點加 2 起連續寫，不分列，
    /// 視窗每列 42 格，所以列距是 54h 位元組，與碰撞讀取的列距（50h）不同。
    ///
    /// 戰場加大時改成對齊碰撞讀取的列距：視窗每列 `cols + 2` 格，只有前 `cols` 格存進 G4，
    /// 所以第一幀起碰撞格就和畫面上的圖塊一致，沒有原版前 40 個捲動步的錯位。
    pub(crate) fn fill_terrain_cell(&mut self, cell: u16, tile: u16) {
        let lay = self.layout();
        if lay.wide() {
            let window = lay.cols + WINDOW_EXTRA_COLS;
            let (row, col) = (cell / window, cell % window);
            if col < lay.cols {
                self.grids.poke16(GRID4_START + 2 + row * lay.terrain_row_bytes() + 2 * col, tile);
            }
            return;
        }
        self.grids.poke16(GRID4_START.wrapping_add(2).wrapping_add(2u16.wrapping_mul(cell)), tile);
    }

    #[cfg(test)]
    pub(crate) fn set_terrain_cursor(&mut self, v: u16) {
        self.grids.set_terrain_cursor(v);
    }

    #[cfg(test)]
    pub(crate) fn terrain_cursor(&self) -> u16 {
        self.grids.terrain_cursor()
    }
}

const _: () = assert!(GRID3_START + 2 * TERRAIN_WORDS == TERRAIN_CURSOR, "游標緊接在 G3 之後");
const _: () = assert!(GRID4_START + 2 * TERRAIN_WORDS == GRID3_START, "G3 緊接在 G4 之後");

/// 欄數 40 的版面必須和原版的常數逐項相同，戰場加大的版面是同一組公式換了欄數。
const _: () = {
    const ORIGINAL: Layout = Layout::new(GRID_COLS);
    assert!(ORIGINAL.g1_bytes() == GRID1_BYTES);
    assert!(ORIGINAL.g1_end() == GRID1_END);
    assert!(ORIGINAL.row_span() == GRID1_ROW_SPAN);
    assert!(ORIGINAL.row_bytes() == GRID1_ROW_BYTES);
    assert!(ORIGINAL.g2_bytes() == GRID2_BYTES);
    assert!(ORIGINAL.g2_last_writable() == GRID2_LAST_WRITABLE);
    assert!(ORIGINAL.g2_row_end() == GRID2_ROW_END);
    assert!(ORIGINAL.scan_row_span() == SCAN_ROW_SPAN);
    assert!(ORIGINAL.terrain_words() == TERRAIN_WORDS);
    assert!(ORIGINAL.terrain_row_bytes() == TERRAIN_ROW_BYTES);
    assert!(ORIGINAL.terrain_row_bytes() == GRID3_ROW_BYTES);
    assert!(ORIGINAL.g3_start() == GRID3_START);
    assert!(ORIGINAL.g3_end() == GRID3_END);
    assert!(ORIGINAL.g4_last() == GRID4_LAST);
    assert!(ORIGINAL.g3_row_last_cell() == GRID3_ROW_LAST_CELL);
    assert!(ORIGINAL.terrain_cursor() == TERRAIN_CURSOR);
    assert!(ORIGINAL.terrain_block() == TERRAIN_BLOCK);
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Fixes, Start};

    fn core() -> Core {
        let mut c = Core::for_test(Start::Demo, Fixes::NONE);
        c.fill_row_tables();
        c
    }

    #[test]
    fn unmapped_addresses_read_zero_and_drop_writes() {
        let mut g = Grids::default();
        let before = g.clone();
        for addr in [0x0000, GRID1_START - 1, GRID1_START + GRID1_BYTES, GRID2_START - 1, GRID4_START - 1, TERRAIN_CURSOR + 2, 0xFFFF] {
            g.poke8(addr, 0xAA);
            assert_eq!(g.peek8(addr), 0, "{addr:04X}h 沒有對應的記憶體");
        }
        // 字組逐位元組寫，兩個位元組都沒有對應的記憶體才不留痕跡。
        for addr in [0x0000, GRID1_START + GRID1_BYTES + 2, TERRAIN_CURSOR + 2, 0xFFFF] {
            g.poke16(addr, 0xBEEF);
            assert_eq!(g.peek16(addr), 0);
        }
        assert_eq!(g, before, "寫到沒有對應的位址不留痕跡");
        g.poke16(GRID1_START, 0x1234);
        assert_eq!((g.peek8(GRID1_START), g.peek8(GRID1_START + 1)), (0x34, 0x12), "字組小端序");
    }

    #[test]
    fn g2_has_one_cell_past_its_last_row_and_terrain_is_g4_then_g3_then_the_cursor() {
        let mut g = Grids::default();
        g.poke8(GRID2_START + GRID2_BYTES, 7);
        assert_eq!(g.peek8(GRID2_START + GRID2_BYTES), 7, "G2 之後多一格");
        g.poke8(GRID2_START + GRID2_BYTES + 1, 7);
        assert_eq!(g.peek8(GRID2_START + GRID2_BYTES + 1), 0);
        g.poke16(GRID3_START - 2, 0x1111);
        g.poke16(GRID3_START, 0x2222);
        assert_eq!((g.peek16(GRID3_START - 2), g.peek16(GRID3_START)), (0x1111, 0x2222), "G4 的末端與 G3 的起點相鄰");
        g.set_terrain_cursor(0x0640);
        assert_eq!(g.peek16(GRID3_START + 2 * TERRAIN_WORDS), 0x0640, "游標緊接在 G3 之後");
    }

    #[test]
    fn grid3_or_ignores_addresses_outside_g3() {
        let mut c = core();
        c.grid3_or(GRID3_START, 0x03);
        c.grid3_or(GRID3_START, 0x04);
        c.grid3_or(GRID3_END, 0x80);
        assert_eq!((c.grids.peek8(GRID3_START), c.grids.peek8(GRID3_END)), (0x07, 0x80), "範圍內的格以 or 累積，兩端都算");
        let before = c.grids.clone();
        for addr in [GRID3_START - 1, GRID3_END + 1, 0, 0xFFFF] {
            c.grid3_or(addr, 0xFF);
        }
        assert_eq!(c.grids, before, "範圍外的格不寫");
    }

    #[test]
    fn grid3_rows_from_14h_share_the_out_of_range_row() {
        let c = core();
        assert_eq!(c.grid3_cell_addr(0x14, 3), c.grid3_cell_addr(0x1234, 3));
        assert_eq!(c.grid3_cell_addr(0xFFFF, 3), c.grid3_cell_addr(0x14, 3), "負的列號也是越界列");
        assert_ne!(c.grid3_cell_addr(0x13, 3), c.grid3_cell_addr(0x14, 3), "列 13h 是最後一個一般列");
    }

    #[test]
    fn the_out_of_range_row_start_is_the_enemy_death_flag() {
        // 列起點表第 21 項就是敵人的死亡旗標：越界列的起點隨敵人狀態在 `N` 與 `Y` 之間變，G3 與 G4 都一樣。
        let mut c = core();
        for (dying, flag) in [(false, 0x4E), (true, 0x59)] {
            c.set_enemy_death_flag(dying);
            assert_eq!(c.grid3_cell_addr(0x14, 0), GRID3_START + flag + 1);
            assert_eq!(c.grid4_cell_addr(0x20, 0), GRID4_START + flag + 1);
        }
    }

    #[test]
    fn grid3_cell_addr_is_start_plus_row_start_plus_two_cols_plus_one() {
        let c = core();
        assert_eq!(c.grid3_cell_addr(5, 7), GRID3_START + 5 * 0x50 + 1 + 2 * 7 + 1);
        // 環繞：欄乘 2 超過 16 位元時高位元丟掉。
        assert_eq!(c.grid3_cell_addr(5, 0x8001), GRID3_START + 5 * 0x50 + 1 + 2 + 1);
    }

    #[test]
    fn grid3_or_in_row_only_writes_inside_the_row() {
        let mut c = core();
        let first = GRID3_START + 0x51;
        c.grid3_or_in_row(first, first, 0x01);
        c.grid3_or_in_row(first, first, 0x02);
        c.grid3_or_in_row(first, first + 0x4E, 0x80);
        assert_eq!((c.grids.peek8(first), c.grids.peek8(first + 0x4E)), (0x03, 0x80), "列內的格以 or 累積，兩端都算");
        c.grid3_or_in_row(first, first + 0x50, 0xFF);
        c.grid3_or_in_row(first, first - 1, 0xFF);
        assert_eq!((c.grids.peek8(first + 0x50), c.grids.peek8(first - 1)), (0, 0), "列外不寫，即使仍在 G3 內");
    }

    #[test]
    fn grid3_mark_block_writes_words_without_a_range_check_and_uses_the_low_byte_of_the_row() {
        let mut c = core();
        c.grids.poke8(GRID3_START + 1, 0x77);
        c.grid3_mark_block(0, 0, 2, 4, 1);
        // 第 0 列第 0 欄在 G3 起點加 2；每格一個字組 0001h，相鄰格之間隔 2 位元組，列距 50h。
        for (row, col) in [(0u16, 0u16), (0, 3), (1, 0), (1, 3)] {
            let at = GRID3_START + 2 + row * GRID3_ROW_BYTES + 2 * col;
            assert_eq!((c.grids.peek8(at), c.grids.peek8(at + 1)), (1, 0), "列 {row} 欄 {col}");
        }
        assert_eq!(c.grids.peek8(GRID3_START + 1), 0x77, "起點前一個位元組不動");
        assert_eq!(c.grids.peek8(GRID3_START + 2 + 8), 0, "第 5 欄不標");
        // 列只取低位元組：列 101h 與列 1 同位址。
        let (mut a, mut b) = (core(), core());
        a.grid3_mark_block(0x101, 7, 1, 1, 0xBEEF);
        b.grid3_mark_block(1, 7, 1, 1, 0xBEEF);
        assert_eq!(a.grids, b.grids);
        assert_ne!(a.grids, Grids::default());
        // 不經範圍檢查：列 20 在 G3 之外，落在游標之後沒有對應記憶體的位址，丟棄。
        let mut c = core();
        let before = c.grids.clone();
        c.grid3_mark_block(20, 0, 1, 1, 0x0001);
        assert_eq!(c.grids, before);
    }

    #[test]
    fn grid4_cell_addr_is_start_plus_row_start_plus_two_cols_plus_one() {
        let c = core();
        assert_eq!(c.grid4_cell_addr(3, 5), GRID4_START + 3 * 0x50 + 1 + 2 * 5 + 1);
        assert_eq!(c.grid4_cell_addr(0xFFFF, 5), c.grid4_cell_addr(0x14, 5), "負列也是越界列");
        assert!(c.grid4_contains(GRID4_START) && c.grid4_contains(GRID4_LAST));
        assert!(!c.grid4_contains(GRID4_START - 1) && !c.grid4_contains(GRID4_LAST + 1));
    }

    #[test]
    fn grid1_row_start_reads_the_row_table_and_then_g1_itself() {
        let mut c = core();
        assert_eq!(c.grid1_row_start(0), GRID1_START);
        assert_eq!(c.grid1_row_start(5), GRID1_START + 5 * 0x28);
        // 列 14h 至 17Bh 直接以列號索引列基底表，讀到 G1 的內容（索引 14h 就是 G1 的前兩個位元組）。
        c.grids.poke16(GRID1_START, 0x0123);
        c.grids.poke16(GRID1_START + 4, 0x0200);
        assert_eq!(c.grid1_row_start(0x14), GRID1_START + 0x0123);
        assert_eq!(c.grid1_row_start(0x16), GRID1_START + 0x0200);
        // 列（無號）大於等於 17Ch，包含負列，一律取索引 1F4h，G1 之後的零值間隙。
        for row in [0x17C, 0x200, 0xFFFF] {
            assert_eq!(c.grid1_row_start(row), GRID1_START, "列 {row:#x}");
        }
        // 列 17Bh 是最後一個直接索引的列（G1 的第 2DEh 個位元組），17Ch 起改取間隙。
        c.grids.poke16(GRID1_START + 2 * (0x17B - 0x14), 0x0050);
        assert_eq!(c.grid1_row_start(0x17B), GRID1_START + 0x50);
        assert_ne!(c.grid1_row_start(0x17B), c.grid1_row_start(0x17C));
    }

    #[test]
    fn grid1_byte_reads_the_end_of_the_row_table_just_before_g1() {
        let mut c = core();
        // 列基底表緊接在 G1 之前：G1 起點的前一個位元組是表最後一項（列 13h，13h * 28h = 2F8h）的高位元組。
        assert_eq!(c.grid1_byte(GRID1_START - 1), 0x02);
        assert_eq!(c.grid1_byte(GRID1_START - 2), 0xF8);
        c.grids.poke8(GRID1_START, 9);
        assert_eq!(c.grid1_byte(GRID1_START), 9, "G1 內讀 G1");
        assert_eq!(c.grid1_byte(GRID1_START - 0x40), 0, "表之前沒有對應的記憶體");
    }

    #[test]
    fn grid1_probe_excludes_the_row_ends_and_the_outside_of_g1() {
        let mut c = core();
        let start = c.grid1_row_start(3);
        c.grids.poke8(start, 7);
        c.grids.poke8(start + 1, 8);
        c.grids.poke8(start + 0x26, 9);
        c.grids.poke8(start + 0x27, 10);
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
    fn clear_grid1_zeroes_all_of_g1_and_grid1_put_stays_in_its_row() {
        let mut c = core();
        c.grid1_put(3, 5, 9);
        assert_eq!(c.grids.peek8(c.grid1_row_start(3) + 5), 9);
        c.grid1_put(3, 0x28, 9);
        assert_eq!(c.grids.peek8(c.grid1_row_start(4)), 0, "欄 28h 以上略過，不溢進下一列");
        c.grids.poke8(GRID1_START + GRID1_BYTES - 1, 5);
        c.clear_grid1();
        assert_eq!(c.grids.g1, [0; GRID1_BYTES as usize], "清到 G1 的最後一個位元組");
    }

    #[test]
    fn clear_grid2_zeroes_all_of_g2_and_the_slot_shadow() {
        let mut c = core();
        c.grid2_write(GRID2_START + 0x50, 4);
        c.grid2_write(GRID2_START + GRID2_BYTES - 1, 4);
        c.g2_slots[3] = 7;
        c.clear_grid2();
        assert_eq!(c.grid2_byte(GRID2_START + 0x50), 0);
        assert_eq!(c.grid2_byte(GRID2_START + GRID2_BYTES - 1), 0);
        assert!(c.g2_slots.iter().all(|&s| s == 0));
    }

    #[test]
    fn grid2_writable_excludes_column_0_and_39_but_allows_one_cell_past_g2() {
        let c = core();
        let base = GRID2_START + 5 * 0x28;
        assert!(!c.grid2_writable(base, base), "欄 0 不可寫");
        assert!(c.grid2_writable(base, base + 1));
        assert!(c.grid2_writable(base, base + 0x26), "欄 38 可寫");
        assert!(!c.grid2_writable(base, base + 0x27), "欄 39 不可寫");
        let last = GRID2_START + 19 * 0x28;
        assert!(c.grid2_writable(last, GRID2_START + GRID2_BYTES - 2), "列 19 的欄 38");
        assert!(!c.grid2_writable(GRID2_START - 5, GRID2_START - 1), "低於 G2 起點不可寫");
        assert!(c.grid2_writable(GRID2_START + GRID2_BYTES - 0x10, GRID2_START + GRID2_BYTES), "G2 之後一格可寫");
        assert!(!c.grid2_writable(GRID2_START + GRID2_BYTES - 0x10, GRID2_START + GRID2_BYTES + 1));
    }

    #[test]
    fn player_scan_window_has_its_own_limits() {
        let c = core();
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
        // 負列與 17Ch 以上的列取列基底表索引 1F4h（零），不像敵彈的 G2 寫入取索引 14h。
        for row in [0x17Cu16, 0xFFFD] {
            assert_eq!(c.scan_row_start(ScanGrid::Entities, row), GRID1_START);
            assert_eq!(c.scan_row_start(ScanGrid::Bullets, row), GRID2_START);
        }
        let mut c = core();
        c.grids.poke16(GRID1_START, 0x0040);
        assert_ne!(c.scan_row_start(ScanGrid::Bullets, 0xFFFD), c.grid2_row_base(0xFFFD), "玩家掃描與敵彈寫入的越界列規則不同");
    }

    #[test]
    fn terrain_tiles_go_into_the_documented_cells() {
        let mut c = core();
        c.start_terrain_column();
        c.store_terrain_tile(0x0111);
        c.store_terrain_tile(0x0222);
        assert_eq!(c.grids.peek16(GRID4_START + TERRAIN_ROW_BYTES), 0x0111, "新欄的第一格在游標 50h，也就是列 1 欄 0");
        assert_eq!(c.grids.peek16(GRID4_START + 2 * TERRAIN_ROW_BYTES), 0x0222);
        assert_eq!(c.terrain_cursor(), 3 * TERRAIN_ROW_BYTES);
        c.fill_terrain_cell(0, 0x0333);
        c.fill_terrain_cell(42, 0x0444);
        assert_eq!(c.grids.peek16(GRID4_START + 2), 0x0333, "初始視窗從 G4 起點加 2 連續寫");
        assert_eq!(c.grids.peek16(GRID4_START + 2 + 84), 0x0444, "每列 42 格");
    }
}
