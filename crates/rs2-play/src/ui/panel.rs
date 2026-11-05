//! 設定畫面：蓋在遊戲畫面上，開著時前端暫停。畫到 RGBA 緩衝區，版面與命中測試不依賴 SDL，可單元測試。
//!
//! | 頁 | 內容 |
//! |---|---|
//! | 手把 | 偵測到的手把，指派給第 1 人、第 2 人。不能重複指派，拔除時取消指派 |
//! | 手把按鍵 | 開火、轉向、分離、暫停、確定、選單的按鈕（各兩格）。方向固定十字鍵與左搖桿 |
//! | 鍵盤 | 兩人各十一個欄位的實體鍵。點欄位或 Enter 後按新鍵，Delete 清空，Esc 取消，衝突不覆寫 |
//! | 遊戲 | 規格 005「選項」表的全部選項，分群組。沒實作的灰色、標開發中、不能切換 |
//! | 顯示 | 顯示模式、補間、幀計數、倍率、工具列 |
//!
//! 操作：F8 換頁（`next_page`），Tab 選欄位，Enter 啟動，Esc 關閉；滑鼠與觸控可以點，上下拖曳或滾輪捲動。
//! 內容比視窗高時可以捲動，焦點移動時自動捲到看得見。

use crate::controls::{Action, Controls, Effect};
use crate::keymap::{ACTS, Act, BindError, Btn, PADFNS, PadBindError, PadFn};
use crate::mode::DisplayMode;
use crate::options::{FEEL_INDEX, Kind, OPTIONS, OptDef};
use crate::pad::{AssignError, Devices};
use crate::settings::SCALES;
use crate::toolbar::{INK, INK_NOTE, INK_OFF, INK_WARN, metrics};
use crate::ui::style::{self, Rgba, Vis, color};
use crate::ui::{Align, Canvas, Rect, font, text};
use sdl2::keyboard::Scancode as K;

/// 設定畫面後面的遊戲畫面調暗的程度：60%（規格 005）。
pub const DIM_ALPHA: u8 = 153;
/// 面板開啟時淡入的毫秒數（規格 005：120 毫秒），不影響模擬時間。
pub const FADE_IN_MS: u64 = 120;

const BG: Rgba = color::PANEL;
const CARD: Rgba = color::SURFACE;
const GREEN: Rgba = color::ON;
const SCROLL: Rgba = [143, 163, 194, 120];
/// 卡片的圓角。
const CARD_RAD: f32 = 14.0;

/// 開啟後經過 `elapsed_ms` 毫秒的不透明度：線性淡入，`FADE_IN_MS` 之後是 1.0。
pub fn fade_in(elapsed_ms: u64) -> f32 {
    (elapsed_ms as f32 / FADE_IN_MS as f32).clamp(0.0, 1.0)
}

/// 淡入分成幾級。呼叫端用級數當重畫的依據，同一級不重畫。
pub const FADE_STEPS: u32 = 8;

/// 不透明度量化成 `0..=FADE_STEPS` 級。
pub fn fade_step(elapsed_ms: u64) -> u32 {
    (fade_in(elapsed_ms) * FADE_STEPS as f32).round() as u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Pad,
    PadKeys,
    Keyboard,
    Game,
    Display,
}

pub const PAGES: [Page; 5] = [Page::Pad, Page::PadKeys, Page::Keyboard, Page::Game, Page::Display];

impl Page {
    pub fn label(self) -> &'static str {
        match self {
            Page::Pad => text::TAB_PAD,
            Page::PadKeys => text::TAB_PADKEYS,
            Page::Keyboard => text::TAB_KEYBOARD,
            Page::Game => text::TAB_GAME,
            Page::Display => text::TAB_DISPLAY,
        }
    }

    /// 標題：前三頁是控制設定，後兩頁是遊戲設定。
    pub fn title(self) -> &'static str {
        match self {
            Page::Pad | Page::PadKeys | Page::Keyboard => text::TITLE_CONTROLS,
            Page::Game | Page::Display => text::TITLE_GAME,
        }
    }

    /// 命令列 `--open-panel` 的頁名：中文頁名或英文名稱。
    pub fn from_name(s: &str) -> Option<Page> {
        let s = s.trim();
        PAGES.iter().copied().find(|p| p.label() == s).or(match s.to_ascii_lowercase().as_str() {
            "pad" | "gamepad" => Some(Page::Pad),
            "padkeys" | "padbuttons" => Some(Page::PadKeys),
            "keyboard" | "keys" => Some(Page::Keyboard),
            "game" => Some(Page::Game),
            "display" | "view" => Some(Page::Display),
            _ => None,
        })
    }

    fn index(self) -> usize {
        PAGES.iter().position(|p| *p == self).unwrap_or(0)
    }
}

/// 欄位的身分。手把用 SDL 的 instance id，裝置清單變動時焦點不會跑掉。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id {
    PadAssign { dev: u32, player: usize },
    PadFn { f: PadFn, slot: usize },
    PadReset,
    Key { player: usize, act: Act },
    KeyReset,
    Opt(usize),
    Tween,
    Counter,
    Scale,
    Toolbar,
    Mode,
}

/// 設定畫面碰得到的前端狀態。
pub struct Cx<'a> {
    pub ctl: &'a mut Controls,
    pub devs: &'a mut Devices,
}

/// 輸入處理的結果。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PanelOut {
    pub eff: Effect,
    /// 要關閉設定畫面。
    pub close: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Capture {
    /// 等一個鍵。`wait` 是啟動重綁的那顆鍵，它放開之前不當成新鍵（Enter 的按下事件還沒放開）。
    Key { player: usize, act: Act, wait: Option<K> },
    /// 等一個手把按鈕。
    Pad { f: PadFn, slot: usize, wait: Option<K> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CK {
    Header,
    Label,
    Note,
    /// 有框的欄位或按鈕。
    Box,
    Tag,
}

#[derive(Clone, Debug)]
struct Cell {
    id: Option<Id>,
    kind: CK,
    rect: Rect,
    text: String,
    ink: [u8; 4],
    enabled: bool,
    /// 字高。
    fh: i32,
    align: Align,
}

struct Layout {
    cells: Vec<Cell>,
    /// 群組卡片，內容座標。
    groups: Vec<Rect>,
    content_h: i32,
    /// 以下都是卡片內的座標，原點在卡片左上角。
    view: Rect,
    title: Rect,
    close: Rect,
    tabs: Vec<(Page, Rect)>,
    msg: Rect,
    /// 卡片在設定畫面內的位置。
    card: Rect,
}

#[derive(Clone, Copy, Debug)]
struct Geom {
    w: i32,
    h: i32,
    mobile: bool,
    dp: f32,
}

/// 尺寸。
#[derive(Clone, Copy, Debug)]
struct Pm {
    fh: i32,
    nfh: i32,
    rh: i32,
    gap: i32,
    pad: i32,
    mar: i32,
    /// 欄位與按鈕的圓角。
    rad: f32,
    /// 群組卡片內的左右、上下留白。
    cpx: i32,
    cpy: i32,
}

fn pm(g: &Geom) -> Pm {
    let m = metrics(g.w.max(1) as u32, g.mobile, g.dp);
    // 說明文字用小一點的字：桌面版取內附的點陣，手機版取 8 成。
    let nfh = if g.mobile {
        (m.fh * 4 / 5).max(14)
    } else {
        match m.fh {
            16 | 20 => 16,
            24 => 20,
            32 => 24,
            f => f * 4 / 5,
        }
    };
    let rh = if g.mobile { m.btn_h } else { m.fh * 7 / 5 };
    let gap = if g.mobile { m.gap } else { (m.fh / 6).max(3) };
    Pm { fh: m.fh, nfh, rh, gap, pad: m.fh / 2, mar: (m.fh * 3 / 4).max(8), rad: m.rad as f32, cpx: m.fh * 2 / 3, cpy: gap * 2 }
}

fn tw(s: &str, h: i32) -> i32 {
    font::builtin().text_width(s, h)
}

/// 面板卡片在設定畫面（視窗工具列以下的整塊）內的位置：置中，四周留一圈，讓後面調暗的遊戲畫面露出來。
fn card_rect(g: &Geom) -> Rect {
    let m = metrics(g.w.max(1) as u32, g.mobile, g.dp);
    let (mx, my) = if g.mobile { (m.fh / 2, m.fh / 3) } else { (m.fh.max(g.w / 28), (m.fh * 3 / 4).max(g.h / 30)) };
    let (w, h) = ((g.w - 2 * mx).max(1), (g.h - 2 * my).max(1));
    Rect::new((g.w - w) / 2, (g.h - h) / 2, w, h)
}

/// 開關的高度（偶數，圓頭才對稱）。
fn switch_h(p: &Pm) -> i32 {
    (if p.rh >= 40 { p.rh * 3 / 5 } else { p.rh * 2 / 3 }).max(12) & !1
}

fn switch_w(p: &Pm) -> i32 {
    style::switch_width(switch_h(p))
}

/// 分段按鈕每段左右的留白。
fn seg_pad(p: &Pm) -> i32 {
    p.pad * 3 / 4
}

/// 分段按鈕的總寬：各段加左右各 2 像素的內縮。
fn seg_total(labels: &[&str], p: &Pm) -> i32 {
    style::seg_widths(labels, p.fh, seg_pad(p)).iter().sum::<i32>() + 4
}

fn opt_labels(o: &OptDef) -> Vec<&'static str> {
    (0..o.values()).map(|v| o.value_label(v)).collect()
}

/// 遊戲頁選項的控制項寬度：開關，或各值排成一列的分段按鈕。
fn opt_control_width(o: &OptDef, p: &Pm) -> i32 {
    match o.kind {
        Kind::Toggle => switch_w(p) + 2,
        _ => seg_total(&opt_labels(o), p),
    }
}

fn scale_labels() -> Vec<String> {
    SCALES.iter().map(|n| text::scale_value(*n)).collect()
}

fn mode_labels() -> Vec<String> {
    DisplayMode::ALL.iter().map(|m| m.label().to_string()).collect()
}

/// 顯示頁值欄的寬度：開關與顯示模式的分段按鈕，桌面版再加倍率的分段按鈕。
fn display_control_width(p: &Pm, mobile: bool) -> i32 {
    let modes = mode_labels();
    let m: Vec<&str> = modes.iter().map(|s| s.as_str()).collect();
    let mut vw = (switch_w(p) + 2).max(seg_total(&m, p));
    if !mobile {
        let l = scale_labels();
        let r: Vec<&str> = l.iter().map(|s| s.as_str()).collect();
        vw = vw.max(seg_total(&r, p));
    }
    vw
}

/// 欄位畫成什麼控制項。
#[derive(Clone, Debug, PartialEq)]
enum Widget {
    /// 沒有 id 的框（裝置名稱），不能操作。
    Static,
    /// 鍵位與手把按鈕的欄位。
    Field,
    /// 動作按鈕（指派、還原預設）。
    Button,
    /// 布林選項：滑動開關。
    Switch(bool),
    /// 多值選項：分段按鈕。
    Seg { labels: Vec<String>, sel: Option<usize> },
}

/// 一頁內容的建造器。座標是內容座標，原點在捲動區左上角。
struct Builder {
    p: Pm,
    /// 目前可用的內容寬度。在群組卡片裡是卡片內側的寬度。
    cw: i32,
    /// 整個內容區的寬度。
    full: i32,
    y: i32,
    /// 目前的左側縮進（在群組卡片裡是卡片的左右留白）。`push` 會把它加到每個欄位的 x。
    x0: i32,
    cells: Vec<Cell>,
    groups: Vec<Rect>,
    open: Option<i32>,
}

impl Builder {
    fn new(p: Pm, cw: i32) -> Builder {
        Builder { p, cw, full: cw, y: 0, x0: 0, cells: Vec::new(), groups: Vec::new(), open: None }
    }

    fn push(&mut self, id: Option<Id>, kind: CK, rect: Rect, text: String, ink: [u8; 4], enabled: bool, fh: i32, align: Align) {
        let rect = rect.offset(self.x0, 0);
        self.cells.push(Cell { id, kind, rect, text, ink, enabled, fh, align });
    }

    /// 開一張群組卡片：之後的內容縮進，上下留白。分組用卡片，不用線。
    fn begin_card(&mut self) {
        self.open = Some(self.y);
        self.y += self.p.cpy;
        self.x0 = self.p.cpx;
        self.cw = (self.full - 2 * self.p.cpx).max(1);
    }

    /// 收起群組卡片。最後一列後面本來就留了一個列距。
    fn end_card(&mut self) {
        if let Some(top) = self.open.take() {
            self.y += self.p.cpy - self.p.gap.min(self.p.cpy);
            self.groups.push(Rect::new(0, top, self.full, self.y - top));
            self.y += self.p.gap * 2;
            self.x0 = 0;
            self.cw = self.full;
        }
    }

    /// 群組標題。
    fn header(&mut self, s: &str) {
        let h = self.p.nfh + self.p.gap * 2;
        self.push(None, CK::Header, Rect::new(0, self.y, self.cw, h), s.to_string(), INK_NOTE, true, self.p.fh, Align::Left);
        self.y += h + self.p.gap;
    }

    /// 一行不折的字。
    fn line(&mut self, s: String, ink: [u8; 4]) {
        let h = self.p.rh;
        self.push(None, CK::Label, Rect::new(0, self.y, self.cw, h), s, ink, true, self.p.fh, Align::Left);
        self.y += h + self.p.gap;
    }

    /// 說明文字：依寬度折行，小字。
    fn note(&mut self, s: &str, ink: [u8; 4]) {
        let lines = font::builtin().wrap(s, self.cw - 2, self.p.nfh);
        let lh = self.p.nfh + self.p.nfh / 4;
        for l in lines {
            self.push(None, CK::Note, Rect::new(0, self.y, self.cw, lh), l, ink, true, self.p.nfh, Align::Left);
            self.y += lh;
        }
        self.y += self.p.gap;
    }

    /// 單獨一個按鈕，寬度由字決定。
    fn button(&mut self, id: Id, s: &str) {
        let w = (tw(s, self.p.fh) + 2 * self.p.pad).min(self.cw);
        self.push(Some(id), CK::Box, Rect::new(0, self.y, w, self.p.rh), s.to_string(), INK, true, self.p.fh, Align::Center);
        self.y += self.p.rh + self.p.gap;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// 指標按下之後的追蹤。
#[derive(Clone, Copy, Debug)]
struct Press {
    y: i32,
    /// 按下時的捲動量。
    scroll0: i32,
    dragged: bool,
}

/// 供測試與驗證讀的欄位資訊，座標是設定畫面（面板）座標，已套用捲動。
#[derive(Clone, Debug)]
pub struct CellInfo {
    pub id: Option<Id>,
    pub rect: Rect,
    pub text: String,
    pub enabled: bool,
    pub focused: bool,
    pub visible: bool,
}

pub struct Panel {
    page: Page,
    focus: Option<Id>,
    scroll: i32,
    capture: Option<Capture>,
    msg: String,
    press: Option<Press>,
    geom: Geom,
    version: u64,
    /// 指標目前在哪個目標上（變亮）。觸控沒有。
    hover: Hit,
    /// 指標按下時所在的目標（變深）。
    down: Hit,
    /// 調暗的背景加陰影的點陣，鍵是（視窗寬、高、卡片位置與大小、字高）。
    base: std::cell::RefCell<Option<(BaseKey, Vec<u8>)>>,
}

/// `Panel::base` 的鍵。
type BaseKey = (i32, i32, i32, i32, i32, i32, i32);

impl Panel {
    /// `w × h` 是設定畫面的大小（視窗寬度、工具列以下的高度）。
    pub fn new(page: Page, w: i32, h: i32, mobile: bool, dp: f32) -> Panel {
        Panel { page, focus: None, scroll: 0, capture: None, msg: String::new(), press: None, geom: Geom { w, h, mobile, dp }, version: 1, hover: Hit::None, down: Hit::None, base: std::cell::RefCell::new(None) }
    }

    pub fn page(&self) -> Page {
        self.page
    }

    /// 畫面有變就加一，呼叫端用它決定要不要重畫、重傳貼圖。
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn message(&self) -> &str {
        &self.msg
    }

    pub fn capturing(&self) -> bool {
        self.capture.is_some()
    }

    pub fn size(&self) -> (i32, i32) {
        (self.geom.w, self.geom.h)
    }

    pub fn resize(&mut self, w: i32, h: i32, mobile: bool, dp: f32) {
        let g = Geom { w, h, mobile, dp };
        if g.w != self.geom.w || g.h != self.geom.h || g.mobile != self.geom.mobile || g.dp != self.geom.dp {
            self.geom = g;
            self.version += 1;
        }
    }

    pub fn set_page(&mut self, p: Page) {
        if p != self.page {
            self.page = p;
            self.focus = None;
            self.scroll = 0;
            self.capture = None;
            self.msg.clear();
            self.version += 1;
        }
    }

    /// F8：換頁。`dir` 為 1 往後，-1 往前，循環。
    pub fn next_page(&mut self, dir: i32) {
        let n = PAGES.len() as i32;
        let i = (self.page.index() as i32 + dir).rem_euclid(n) as usize;
        self.set_page(PAGES[i]);
    }

    // -----------------------------------------------------------------------
    // 版面
    // -----------------------------------------------------------------------

    fn layout(&self, ctl: &Controls, devs: &Devices) -> Layout {
        let g = self.geom;
        let p = pm(&g);
        let card = card_rect(&g);
        let (w, h) = (card.w.max(1), card.h.max(1));
        let mar = p.mar;
        let cw = (w - 2 * mar).max(1);
        // 上方：標題列與關閉鈕，再來是頁籤。
        let close_w = tw(text::PANEL_CLOSE, p.fh) + 2 * p.pad;
        let close = Rect::new((w - mar - close_w).max(mar), mar, close_w.min(cw), p.rh);
        let widths: Vec<i32> = PAGES.iter().map(|pg| tw(pg.label(), p.fh) + 2 * p.pad).collect();
        let total: i32 = widths.iter().sum::<i32>() + p.gap * (PAGES.len() as i32 - 1);
        // 手機版放得下就把標題、頁籤、關閉鈕排成一列，省下一列 48 dp 的高度。
        let title_w = tw(self.page.title(), p.fh);
        let one_row = self.geom.mobile && title_w + p.gap * 3 + total + p.gap * 3 + close_w <= cw;
        let (title, tab_y, tab_x) = if one_row {
            (Rect::new(mar, mar, title_w, p.rh), mar, mar + title_w + p.gap * 3)
        } else {
            (Rect::new(mar, mar, (close.x - mar - p.gap).max(0), p.rh), mar + p.rh + p.gap * 2, mar)
        };
        let mut tabs = Vec::new();
        let mut x = tab_x;
        for (i, pg) in PAGES.iter().enumerate() {
            // 放不下就平均分，字會被截斷。
            let tw_ = if total <= cw { widths[i] } else { ((cw - p.gap * (PAGES.len() as i32 - 1)) / PAGES.len() as i32).max(1) };
            tabs.push((*pg, Rect::new(x, tab_y, tw_, p.rh)));
            x += tw_ + p.gap;
        }
        // 下方：訊息列（一行字高，不用按鈕那麼高）。中間是捲動區。
        let msg_h = if self.geom.mobile { p.fh * 3 / 2 } else { p.rh };
        let msg = Rect::new(mar, (h - mar - msg_h).max(0), cw, msg_h);
        let vy = tab_y + p.rh + p.gap;
        let view = Rect::new(mar, vy, cw, (msg.y - p.gap - vy).max(0));
        let (cells, groups, content_h) = self.build(ctl, devs, p, cw);
        Layout { cells, groups, content_h, view, title, close, tabs, msg, card }
    }

    fn build(&self, ctl: &Controls, devs: &Devices, p: Pm, cw: i32) -> (Vec<Cell>, Vec<Rect>, i32) {
        let mut b = Builder::new(p, cw);
        match self.page {
            Page::Pad => self.build_pad(&mut b, devs),
            Page::PadKeys => self.build_padkeys(&mut b, ctl),
            Page::Keyboard => self.build_keyboard(&mut b, ctl),
            Page::Game => self.build_game(&mut b, ctl),
            Page::Display => self.build_display(&mut b, ctl),
        }
        let h = b.y;
        (b.cells, b.groups, h)
    }

    fn build_pad(&self, b: &mut Builder, devs: &Devices) {
        let p = b.p;
        b.begin_card();
        b.header(text::PAD_LIST_HEAD);
        if devs.list().is_empty() {
            b.line(text::PAD_NONE.to_string(), INK_WARN);
            b.end_card();
            return;
        }
        let mut bw = 0;
        for pl in 0..2 {
            for a in [false, true] {
                bw = bw.max(tw(&text::pad_assign_button(pl, a), p.fh));
            }
        }
        let bw = bw + 2 * p.pad;
        for d in devs.list() {
            if d.supported {
                let name_w = (b.cw - 2 * (bw + p.gap)).max(1);
                b.push(None, CK::Box, Rect::new(0, b.y, name_w, p.rh), d.name.clone(), INK, true, p.fh, Align::Left);
                for pl in 0..2usize {
                    let assigned = devs.player_of(d.id) == Some(pl);
                    let x = name_w + p.gap + pl as i32 * (bw + p.gap);
                    b.push(
                        Some(Id::PadAssign { dev: d.id, player: pl }),
                        CK::Box,
                        Rect::new(x, b.y, bw, p.rh),
                        text::pad_assign_button(pl, assigned),
                        if assigned { GREEN } else { INK },
                        true,
                        p.fh,
                        Align::Center,
                    );
                }
            } else {
                b.push(None, CK::Box, Rect::new(0, b.y, b.cw, p.rh), text::pad_unsupported_name(&d.name), INK_OFF, false, p.fh, Align::Left);
            }
            b.y += p.rh + p.gap;
        }
        b.end_card();
        b.begin_card();
        b.header(text::PAD_ASSIGN_HEAD);
        for pl in 0..2 {
            let name = devs.assigned_dev(pl).map(|d| d.name.as_str());
            b.line(text::pad_player_line(pl, name), if name.is_some() { INK } else { INK_NOTE });
        }
        b.note(text::PAD_NOTE, INK_NOTE);
        b.end_card();
    }

    fn build_padkeys(&self, b: &mut Builder, ctl: &Controls) {
        let p = b.p;
        b.begin_card();
        let mut lw = tw(text::PADFN_COL_FN, p.fh);
        for f in PADFNS {
            lw = lw.max(tw(f.label(), p.fh));
        }
        let lw = lw + 2 * p.pad;
        let fw = ((b.cw - lw - 2 * p.gap) / 2).max(1);
        let hh = p.nfh + p.gap * 2;
        b.push(None, CK::Header, Rect::new(0, b.y, lw, hh), text::PADFN_COL_FN.to_string(), INK_NOTE, true, p.fh, Align::Left);
        b.push(None, CK::Header, Rect::new(lw + p.gap, b.y, fw, hh), text::PADFN_COL_1.to_string(), INK_NOTE, true, p.fh, Align::Center);
        b.push(None, CK::Header, Rect::new(lw + 2 * p.gap + fw, b.y, fw, hh), text::PADFN_COL_2.to_string(), INK_NOTE, true, p.fh, Align::Center);
        b.y += hh + p.gap;
        for f in PADFNS {
            b.push(None, CK::Label, Rect::new(0, b.y, lw, p.rh), f.label().to_string(), INK, true, p.fh, Align::Left);
            for slot in 0..2usize {
                let cap = matches!(self.capture, Some(Capture::Pad { f: cf, slot: cs, .. }) if cf == f && cs == slot);
                let s = if cap { text::PADKEY_PROMPT.to_string() } else { ctl.settings.pad.get(f, slot).map(|x| x.name().to_string()).unwrap_or_else(|| text::UNBOUND.to_string()) };
                let ink = if cap {
                    INK_WARN
                } else if ctl.settings.pad.get(f, slot).is_none() {
                    INK_OFF
                } else {
                    INK
                };
                b.push(Some(Id::PadFn { f, slot }), CK::Box, Rect::new(lw + p.gap + slot as i32 * (fw + p.gap), b.y, fw, p.rh), s, ink, true, p.fh, Align::Center);
            }
            b.y += p.rh + p.gap;
        }
        b.button(Id::PadReset, text::PADKEY_RESET);
        b.end_card();
        b.note(text::PADKEY_NOTE1, INK_NOTE);
        b.note(text::PADKEY_NOTE2, INK_NOTE);
    }

    fn build_keyboard(&self, b: &mut Builder, ctl: &Controls) {
        let p = b.p;
        // 兩人各一張卡片並排。
        let col_gap = p.gap * 3;
        let colw = ((b.cw - col_gap) / 2).max(1);
        let inner = (colw - 2 * p.cpx).max(1);
        let mut lw = 0;
        for a in ACTS {
            lw = lw.max(tw(a.label(), p.fh));
        }
        let lw = lw + 2 * p.pad;
        let top = b.y;
        let mut bottom = top;
        for pl in 0..2usize {
            let x0 = pl as i32 * (colw + col_gap) + p.cpx;
            let mut y = top + p.cpy;
            let hh = p.nfh + p.gap * 2;
            b.push(None, CK::Header, Rect::new(x0, y, inner, hh), text::kb_head(pl), INK_NOTE, true, p.fh, Align::Left);
            y += hh + p.gap;
            for a in ACTS {
                b.push(None, CK::Label, Rect::new(x0, y, lw, p.rh), a.label().to_string(), INK, true, p.fh, Align::Left);
                let cap = matches!(self.capture, Some(Capture::Key { player, act, .. }) if player == pl && act == a);
                let key = ctl.settings.keys.get(pl, a);
                let s = if cap { text::KB_PROMPT.to_string() } else { key.map(text::key_label).unwrap_or_else(|| text::UNBOUND.to_string()) };
                let ink = if cap {
                    INK_WARN
                } else if key.is_none() {
                    INK_OFF
                } else {
                    INK
                };
                b.push(Some(Id::Key { player: pl, act: a }), CK::Box, Rect::new(x0 + lw, y, (inner - lw).max(1), p.rh), s, ink, true, p.fh, Align::Left);
                y += p.rh + p.gap;
            }
            bottom = bottom.max(y);
        }
        let card_h = bottom - top + p.cpy - p.gap;
        for pl in 0..2i32 {
            b.groups.push(Rect::new(pl * (colw + col_gap), top, colw, card_h));
        }
        b.y = top + card_h + p.gap * 2;
        b.button(Id::KeyReset, text::KB_RESET);
        b.note(text::KB_NOTE, INK_NOTE);
        b.note(text::KB_NOTE_SPLIT, INK_NOTE);
        b.note(text::KB_NOTE_KEYS, INK_NOTE);
    }

    fn build_game(&self, b: &mut Builder, ctl: &Controls) {
        let p = b.p;
        let mut vw = 0;
        for o in OPTIONS.iter() {
            vw = vw.max(opt_control_width(o, &p));
        }
        let tag_w = tw(text::IN_DEVELOPMENT, p.nfh) + 2 * p.pad;
        // 群組名放在卡片左邊一欄，只寫在群組的第一列。
        let mut gw = 0;
        for o in OPTIONS.iter() {
            gw = gw.max(tw(o.group.label(), p.fh));
        }
        let gw = gw + p.pad;
        let mut last = None;
        for (i, o) in OPTIONS.iter().enumerate() {
            if last != Some(o.group) {
                if last.is_some() {
                    b.end_card();
                }
                b.begin_card();
                b.push(None, CK::Header, Rect::new(0, b.y, gw, p.rh), o.group.label().to_string(), INK_NOTE, true, p.fh, Align::Left);
                last = Some(o.group);
            }
            let label_x = gw + p.gap;
            let label_w = (b.cw - label_x - vw - tag_w - 2 * p.gap).max(1);
            b.push(None, CK::Label, Rect::new(label_x, b.y, label_w, p.rh), o.label.to_string(), if o.implemented { INK } else { INK_OFF }, true, p.fh, Align::Left);
            if !o.implemented {
                b.push(None, CK::Tag, Rect::new(label_x + label_w + p.gap, b.y, tag_w, p.rh), text::IN_DEVELOPMENT.to_string(), INK_WARN, true, p.nfh, Align::Center);
            }
            let v = ctl.settings.opt_value(i);
            let ink = if !o.implemented { INK_OFF } else { INK };
            b.push(Some(Id::Opt(i)), CK::Box, Rect::new(b.cw - vw, b.y, vw, p.rh), o.value_label(v).to_string(), ink, o.implemented, p.fh, Align::Center);
            b.y += p.rh + p.gap;
        }
        b.end_card();
        b.note(text::GAME_NOTE1, INK_NOTE);
        b.note(text::GAME_NOTE2, INK_NOTE);
    }

    fn build_display(&self, b: &mut Builder, ctl: &Controls) {
        let p = b.p;
        let s = &ctl.settings;
        let on_off = |v: bool| (if v { text::ON } else { text::OFF }).to_string();
        let mut rows: Vec<(Id, &str, String, [u8; 4])> = vec![
            (Id::Mode, text::DISP_MODE, s.mode.label().to_string(), INK),
            (Id::Tween, text::DISP_TWEEN, on_off(s.tween), if s.tween { GREEN } else { INK }),
            (Id::Counter, text::DISP_COUNTER, on_off(s.counter), if s.counter { GREEN } else { INK }),
        ];
        if !self.geom.mobile {
            rows.push((Id::Scale, text::DISP_SCALE, text::scale_value(s.scale), INK));
        }
        rows.push((Id::Toolbar, text::DISP_TOOLBAR, (if s.toolbar { text::DISP_SHOWN } else { text::DISP_HIDDEN }).to_string(), INK));
        let vw = display_control_width(&p, self.geom.mobile);
        b.begin_card();
        for (id, label, value, ink) in rows {
            let label_w = (b.cw - vw - p.gap).max(1);
            b.push(None, CK::Label, Rect::new(0, b.y, label_w, p.rh), label.to_string(), INK, true, p.fh, Align::Left);
            b.push(Some(id), CK::Box, Rect::new(b.cw - vw, b.y, vw, p.rh), value, ink, true, p.fh, Align::Center);
            b.y += p.rh + p.gap;
        }
        b.end_card();
        b.note(text::DISP_NOTE, INK_NOTE);
        if !self.geom.mobile {
            b.note(text::DISP_KEYS, INK_NOTE);
        }
    }

    fn focusables(lay: &Layout) -> Vec<(Id, Rect)> {
        lay.cells.iter().filter_map(|c| c.id.filter(|_| c.enabled).map(|id| (id, c.rect))).collect()
    }

    fn max_scroll(lay: &Layout) -> i32 {
        (lay.content_h - lay.view.h).max(0)
    }

    fn clamp_scroll(&mut self, lay: &Layout) {
        self.scroll = self.scroll.clamp(0, Self::max_scroll(lay));
    }

    /// 目前的焦點。沒有、或已經不在版面上（例如那支手把拔掉了）就取第一個。
    fn current_focus(&self, lay: &Layout) -> Option<Id> {
        let f = Self::focusables(lay);
        match self.focus {
            Some(id) if f.iter().any(|(i, _)| *i == id) => Some(id),
            _ => f.first().map(|(i, _)| *i),
        }
    }

    fn ensure_visible(&mut self, lay: &Layout, id: Id) {
        if let Some((_, r)) = Self::focusables(lay).into_iter().find(|(i, _)| *i == id) {
            let gap = pm(&self.geom).gap;
            let top = r.y - gap;
            let bottom = r.bottom() + gap;
            if top < self.scroll {
                self.scroll = top;
            } else if bottom > self.scroll + lay.view.h {
                self.scroll = bottom - lay.view.h;
            }
        }
        self.clamp_scroll(lay);
    }

    /// 欄位資訊，座標是面板座標（已套用捲動）。
    pub fn describe(&self, ctl: &Controls, devs: &Devices) -> Vec<CellInfo> {
        let lay = self.layout(ctl, devs);
        let focus = self.current_focus(&lay);
        let scroll = self.scroll.clamp(0, Self::max_scroll(&lay));
        let (ox, oy) = (lay.card.x, lay.card.y);
        let view = lay.view.offset(ox, oy);
        lay.cells
            .iter()
            .map(|c| {
                let r = c.rect.offset(ox + lay.view.x, oy + lay.view.y - scroll);
                CellInfo { id: c.id, rect: r, text: c.text.clone(), enabled: c.enabled, focused: c.id.is_some() && c.id == focus, visible: r.intersect(&view).h > 0 }
            })
            .collect()
    }

    /// 內容總高與捲動區高度，測試用。
    pub fn scroll_range(&self, ctl: &Controls, devs: &Devices) -> (i32, i32, i32) {
        let lay = self.layout(ctl, devs);
        (self.scroll, lay.content_h, lay.view.h)
    }

    // -----------------------------------------------------------------------
    // 繪製
    // -----------------------------------------------------------------------

    /// 畫成 `w × h` 的 RGBA：後面的遊戲畫面調暗 60%（半透明黑），面板卡片置中、有陰影。
    pub fn render(&self, ctl: &Controls, devs: &Devices) -> Vec<u8> {
        self.render_faded(ctl, devs, 1.0)
    }

    /// 同 `render`，整張圖的不透明度再乘上 `fade`（0.0 到 1.0），開啟時淡入用。
    pub fn render_faded(&self, ctl: &Controls, devs: &Devices, fade: f32) -> Vec<u8> {
        let lay = self.layout(ctl, devs);
        let p = pm(&self.geom);
        let card = lay.card;
        // 調暗的背景加陰影只和視窗與卡片的幾何有關，換內容時不用重畫。
        let key = (self.geom.w, self.geom.h, card.x, card.y, card.w, card.h, p.fh);
        let base = {
            let mut cache = self.base.borrow_mut();
            match &*cache {
                Some((k, px)) if *k == key => px.clone(),
                _ => {
                    let mut b = Canvas::new(self.geom.w, self.geom.h, [0, 0, 0, DIM_ALPHA]);
                    style::shadow(&mut b, card, CARD_RAD, (p.fh * 3 / 2).max(16), p.fh / 2, [0, 0, 0, 170]);
                    *cache = Some((key, b.px.clone()));
                    b.px
                }
            }
        };
        let mut full = Canvas::new(1, 1, [0, 0, 0, 0]);
        full.w = self.geom.w.max(1);
        full.h = self.geom.h.max(1);
        full.px = base;
        let mut c = Canvas::new(card.w, card.h, BG);
        self.draw_card(&mut c, &lay, &p, ctl, devs);
        // 圓角遮罩貼到畫面上：四個角各 `CARD_RAD` 見方用距離函式，其餘整段複製。
        let local = Rect::new(0, 0, card.w, card.h);
        let r = CARD_RAD.ceil() as i32 + 1;
        for y in 0..card.h {
            let (dy, edge_row) = (card.y + y, y < r || y >= card.h - r);
            if dy < 0 || dy >= full.h {
                continue;
            }
            let (x0, x1) = (if edge_row { card.w } else { r }, if edge_row { card.w } else { card.w - r });
            for x in (0..x0.min(card.w)).chain(x1.min(card.w)..card.w) {
                let cv = style::cov(style::sd_rrect(x as f32 + 0.5, y as f32 + 0.5, local, CARD_RAD));
                let dx = card.x + x;
                if cv > 0.0 && dx >= 0 && dx < full.w {
                    let so = ((y * card.w + x) * 4) as usize;
                    let d = ((dy * full.w + dx) * 4) as usize;
                    let src = [c.px[so], c.px[so + 1], c.px[so + 2], c.px[so + 3]];
                    style::over(&mut full.px[d..d + 4], src, cv);
                }
            }
            if !edge_row && x0 < x1 {
                // 中段完全在圓角之內：不透明，整段複製（超出視窗的部分略過）。
                let lo = (card.x + x0).max(0);
                let hi = (card.x + x1).min(full.w);
                if hi > lo {
                    let so = ((y * card.w + (lo - card.x)) * 4) as usize;
                    let d = ((dy * full.w + lo) * 4) as usize;
                    let n = ((hi - lo) * 4) as usize;
                    full.px[d..d + n].copy_from_slice(&c.px[so..so + n]);
                }
            }
        }
        style::stroke_rrect(&mut full, card, CARD_RAD, 1.0, [255, 255, 255, 34]);
        if fade < 1.0 {
            let f = fade.clamp(0.0, 1.0);
            for px in full.px.chunks_exact_mut(4) {
                px[3] = (px[3] as f32 * f).round() as u8;
            }
        }
        full.px
    }

    fn widget_of(&self, c: &Cell, ctl: &Controls) -> Widget {
        match c.id {
            None => Widget::Static,
            Some(Id::PadAssign { .. }) | Some(Id::PadReset) | Some(Id::KeyReset) => Widget::Button,
            Some(Id::Key { .. }) | Some(Id::PadFn { .. }) => Widget::Field,
            Some(Id::Tween) => Widget::Switch(ctl.settings.tween),
            Some(Id::Counter) => Widget::Switch(ctl.settings.counter),
            Some(Id::Toolbar) => Widget::Switch(ctl.settings.toolbar),
            Some(Id::Scale) => Widget::Seg { labels: scale_labels(), sel: SCALES.iter().position(|n| *n == ctl.settings.scale) },
            Some(Id::Mode) => Widget::Seg { labels: mode_labels(), sel: DisplayMode::ALL.iter().position(|m| *m == ctl.settings.mode) },
            Some(Id::Opt(i)) => {
                let o = &OPTIONS[i];
                match o.kind {
                    Kind::Toggle => Widget::Switch(ctl.settings.opt_value(i) != 0),
                    _ => Widget::Seg { labels: opt_labels(o).into_iter().map(String::from).collect(), sel: Some(ctl.settings.opt_value(i) as usize) },
                }
            }
        }
    }

    /// 控制項本體在欄位矩形裡的位置：開關靠右，分段按鈕填滿。
    fn control_rect(w: &Widget, cell: Rect, p: &Pm) -> Rect {
        match w {
            Widget::Switch(_) => {
                let (sw, sh) = (switch_w(p), switch_h(p));
                Rect::new(cell.right() - sw - 1, cell.y + (cell.h - sh) / 2, sw, sh)
            }
            _ => Rect::new(cell.x, cell.y + 1, cell.w, (cell.h - 2).max(1)),
        }
    }

    fn vis_of(&self, id: Option<Id>, enabled: bool, focused: bool) -> Vis {
        let t = id.map(Hit::Cell);
        Vis { hover: enabled && t.is_some_and(|t| self.hover == t), down: enabled && t.is_some_and(|t| self.down == t), focus: focused, enabled }
    }

    /// 在卡片座標系畫卡片的全部內容。
    fn draw_card(&self, c: &mut Canvas, lay: &Layout, p: &Pm, ctl: &Controls, devs: &Devices) {
        let focus = self.current_focus(lay);
        let scroll = self.scroll.clamp(0, Self::max_scroll(lay));
        let _ = devs;

        // 標題與說明。
        let title = self.page.title();
        c.text_in(lay.title, title, p.fh, 0, Align::Left, INK);
        // 桌面版的操作說明在標題旁；手機版沒有鍵盤，說明放在訊息列，有訊息時讓位。
        if !self.geom.mobile {
            let tx = lay.title.x + tw(title, p.fh) + p.fh;
            let room = Rect::new(tx, lay.title.y, (lay.title.right() - tx).max(0), lay.title.h);
            if room.w > p.fh * 4 {
                c.text_in(room, text::PANEL_HINT, p.nfh, 0, Align::Left, INK_NOTE);
            }
        }
        let cv = Vis { hover: self.hover == Hit::Close, down: self.down == Hit::Close, focus: false, enabled: true };
        style::button(c, lay.close, p.rad, cv, false);
        c.text_in(lay.close, text::PANEL_CLOSE, p.fh, p.pad / 2, Align::Center, INK);

        // 頁籤：一條膠囊，選中的頁是主色。
        if let (Some((_, first)), Some((_, last))) = (lay.tabs.first(), lay.tabs.last()) {
            let strip = Rect::new(first.x - 3, first.y, last.right() - first.x + 6, first.h);
            style::fill_rrect(c, strip, p.rad + 2.0, color::BG);
            style::stroke_rrect(c, strip, p.rad + 2.0, 1.0, [255, 255, 255, 24]);
        }
        for (pg, r) in &lay.tabs {
            let sel = *pg == self.page;
            let inner = r.inset(2);
            let hv = self.hover == Hit::Tab(*pg);
            let dn = self.down == Hit::Tab(*pg);
            if sel {
                let top = if dn { style::darken(color::PRIMARY, 0.2) } else { color::PRIMARY };
                style::fill_rrect_vgrad(c, inner, p.rad, style::lighten(top, 0.14), top);
            } else if hv || dn {
                style::fill_rrect(c, inner, p.rad, [255, 255, 255, if dn { 8 } else { 22 }]);
            }
            let ink = if sel { color::BG } else if hv { INK } else { INK_NOTE };
            c.text_in(inner, pg.label(), p.fh, p.pad / 2, Align::Center, ink);
        }

        // 捲動區：先畫群組卡片，再畫欄位。
        c.set_clip(Some(lay.view));
        for g in &lay.groups {
            let r = g.offset(lay.view.x, lay.view.y - scroll);
            if r.intersect(&lay.view).h <= 0 {
                continue;
            }
            style::fill_rrect(c, r, p.rad + 4.0, CARD);
            style::stroke_rrect(c, r, p.rad + 4.0, 1.0, [255, 255, 255, 20]);
        }
        for cell in &lay.cells {
            let r = cell.rect.offset(lay.view.x, lay.view.y - scroll);
            if r.intersect(&lay.view).h <= 0 {
                continue;
            }
            let focused = cell.id.is_some() && cell.id == focus;
            let capturing = self.is_capture_target(cell.id);
            match cell.kind {
                CK::Box => {
                    let vis = self.vis_of(cell.id, cell.enabled, focused);
                    let w = self.widget_of(cell, ctl);
                    match &w {
                        Widget::Static => {
                            style::fill_rrect(c, r, p.rad, if cell.enabled { color::FIELD } else { color::BG });
                            style::stroke_rrect(c, r, p.rad, 1.0, [255, 255, 255, 18]);
                            c.text_in(r, &cell.text, cell.fh, p.pad, cell.align, cell.ink);
                        }
                        Widget::Field => {
                            style::field(c, r, p.rad, vis, capturing);
                            c.text_in(r, &cell.text, cell.fh, p.pad, cell.align, cell.ink);
                        }
                        Widget::Button => {
                            // 已指派的按鈕用開啟色的調子。
                            style::button(c, r, p.rad, vis, cell.ink == GREEN);
                            c.text_in(r, &cell.text, cell.fh, p.pad, cell.align, cell.ink);
                        }
                        Widget::Switch(on) => {
                            let sr = Self::control_rect(&w, r, p);
                            style::draw_switch(c, sr, *on, vis);
                        }
                        Widget::Seg { labels, sel } => {
                            let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
                            let widths = style::seg_widths(&refs, p.fh, seg_pad(p));
                            style::draw_segmented(c, Self::control_rect(&w, r, p), labels, &widths, *sel, p.fh, vis);
                        }
                    }
                }
                CK::Tag => {
                    let th = (cell.fh + cell.fh / 2).min(r.h);
                    let chip = Rect::new(r.x, r.y + (r.h - th) / 2, r.w, th);
                    style::fill_rrect(c, chip, p.rad, style::with_alpha(color::WARN, 28));
                    style::stroke_rrect(c, chip, p.rad, 1.0, style::with_alpha(color::WARN, 90));
                    c.text_in(r, &cell.text, cell.fh, 0, cell.align, cell.ink);
                }
                CK::Header | CK::Label | CK::Note => {
                    c.text_in(r, &cell.text, cell.fh, 0, cell.align, cell.ink);
                }
            }
        }
        c.set_clip(None);
        // 捲軸：畫在捲動區右邊的邊界裡，不超出面板。
        let max = Self::max_scroll(lay);
        if max > 0 && lay.view.h > 0 {
            let bar_w = (p.mar / 3).clamp(3, 6);
            let th = ((lay.view.h as i64 * lay.view.h as i64 / lay.content_h as i64) as i32).clamp(p.fh.min(lay.view.h), lay.view.h);
            let ty = lay.view.y + ((lay.view.h - th) as i64 * scroll as i64 / max as i64) as i32;
            style::fill_rrect(c, Rect::new(lay.view.right() + 2, ty, bar_w, th), bar_w as f32 / 2.0, SCROLL);
        }

        // 訊息列。
        if !self.msg.is_empty() {
            c.text_in(lay.msg, &self.msg, p.fh, 0, Align::Left, INK_WARN);
        } else if self.geom.mobile {
            c.text_in(lay.msg, text::PANEL_HINT_TOUCH, p.nfh, 0, Align::Left, INK_NOTE);
        }
    }

    fn is_capture_target(&self, id: Option<Id>) -> bool {
        match (self.capture, id) {
            (Some(Capture::Key { player, act, .. }), Some(Id::Key { player: p, act: a })) => player == p && act == a,
            (Some(Capture::Pad { f, slot, .. }), Some(Id::PadFn { f: pf, slot: ps })) => f == pf && slot == ps,
            _ => false,
        }
    }

    // -----------------------------------------------------------------------
    // 輸入
    // -----------------------------------------------------------------------

    fn cancel_capture(&mut self) {
        match self.capture.take() {
            Some(Capture::Key { .. }) => self.msg = text::KB_CANCELLED.to_string(),
            Some(Capture::Pad { .. }) => self.msg = text::PADKEY_CANCELLED.to_string(),
            None => {}
        }
        self.version += 1;
    }

    /// 視窗失去焦點：取消重綁，免得按鍵事件收不到而卡在等待。
    pub fn focus_lost(&mut self) {
        if self.capture.take().is_some() {
            self.msg = text::KB_LOST_FOCUS.to_string();
            self.version += 1;
        }
    }

    /// 鍵放開。啟動重綁的那顆鍵放開之後，下一個按下的鍵才算新鍵。
    pub fn key_up(&mut self, key: K) {
        let wait = match &mut self.capture {
            Some(Capture::Key { wait, .. }) | Some(Capture::Pad { wait, .. }) => wait,
            None => return,
        };
        if *wait == Some(key) {
            *wait = None;
        }
    }

    fn start_wait(&self, via: Option<K>) -> Option<K> {
        via
    }

    /// 鍵按下（不含 F8，換頁由 `next_page` 處理）。`shift` 是 Shift 有沒有按著，Shift+Tab 往前。
    pub fn key_down(&mut self, key: K, shift: bool, cx: &mut Cx) -> PanelOut {
        self.version += 1;
        let mut out = PanelOut::default();
        // 重綁中：Esc 取消，Delete 清空，其餘是新鍵。
        if let Some(cap) = self.capture {
            match cap {
                Capture::Key { player, act, wait } => {
                    if wait == Some(key) {
                        return out;
                    }
                    self.capture_key(player, act, key, cx, &mut out);
                }
                Capture::Pad { f, slot, wait } => {
                    if wait == Some(key) {
                        return out;
                    }
                    match key {
                        K::Escape => self.cancel_capture(),
                        K::Delete => {
                            cx.ctl.settings.pad.clear(f, slot);
                            self.capture = None;
                            self.msg = text::msg_padkey_cleared(f.label(), slot);
                            out.eff.save = true;
                        }
                        _ => {}
                    }
                }
            }
            return out;
        }
        let lay = self.layout(cx.ctl, cx.devs);
        let cur = self.current_focus(&lay);
        match key {
            K::Escape => out.close = true,
            K::Tab => self.step_focus(&lay, cur, if shift { -1 } else { 1 }),
            K::Down => self.move_focus(&lay, cur, Dir::Down),
            K::Up => self.move_focus(&lay, cur, Dir::Up),
            K::Left | K::Right => {
                let d = if key == K::Left { Dir::Left } else { Dir::Right };
                match cur {
                    Some(id @ (Id::Opt(_) | Id::Tween | Id::Counter | Id::Scale | Id::Toolbar | Id::Mode)) => {
                        self.adjust(id, if d == Dir::Left { -1 } else { 1 }, cx, &mut out);
                    }
                    _ => self.move_focus(&lay, cur, d),
                }
            }
            K::Return | K::KpEnter | K::Space => {
                if let Some(id) = cur {
                    self.activate(id, Some(key), cx, &mut out);
                }
            }
            K::Delete => match cur {
                Some(Id::Key { player, act }) => {
                    cx.ctl.settings.keys.clear(player, act);
                    self.msg = text::msg_kb_cleared(player, act.label());
                    out.eff.save = true;
                }
                Some(Id::PadFn { f, slot }) => {
                    cx.ctl.settings.pad.clear(f, slot);
                    self.msg = text::msg_padkey_cleared(f.label(), slot);
                    out.eff.save = true;
                }
                _ => {}
            },
            K::PageDown => self.scroll_by(&lay, lay.view.h),
            K::PageUp => self.scroll_by(&lay, -lay.view.h),
            _ => {}
        }
        out
    }

    fn capture_key(&mut self, player: usize, act: Act, key: K, cx: &mut Cx, out: &mut PanelOut) {
        match key {
            K::Escape => self.cancel_capture(),
            K::Delete => {
                cx.ctl.settings.keys.clear(player, act);
                self.capture = None;
                self.msg = text::msg_kb_cleared(player, act.label());
                out.eff.save = true;
            }
            k => match cx.ctl.settings.keys.bind(player, act, k) {
                Ok(()) => {
                    self.capture = None;
                    self.msg = text::msg_kb_set(player, act.label(), &text::key_label(k));
                    out.eff.save = true;
                }
                // 不能綁的鍵與衝突：不覆寫，留在等待狀態讓使用者換一個鍵或按 Esc。
                Err(BindError::NotBindable) => self.msg = text::msg_kb_reserved(&text::key_label(k)),
                Err(BindError::Conflict(p, a)) => self.msg = text::msg_kb_conflict(&text::key_label(k), p, a.label()),
            },
        }
    }

    /// 手把按鈕按下。只在等手把按鈕的重綁中有作用。
    pub fn pad_button(&mut self, b: Btn, cx: &mut Cx) -> PanelOut {
        let mut out = PanelOut::default();
        let Some(Capture::Pad { f, slot, .. }) = self.capture else { return out };
        self.version += 1;
        match cx.ctl.settings.pad.bind(f, slot, b) {
            Ok(()) => {
                self.capture = None;
                self.msg = text::msg_padkey_set(f.label(), slot, b.name());
                out.eff.save = true;
            }
            Err(PadBindError::Taken(of)) => self.msg = text::msg_padkey_taken(b.name(), of.label()),
        }
        out
    }

    fn step_focus(&mut self, lay: &Layout, cur: Option<Id>, delta: i32) {
        let f = Self::focusables(lay);
        if f.is_empty() {
            return;
        }
        let i = cur.and_then(|c| f.iter().position(|(id, _)| *id == c)).unwrap_or(0) as i32;
        let next = f[(i + delta).rem_euclid(f.len() as i32) as usize].0;
        self.focus = Some(next);
        self.ensure_visible(lay, next);
    }

    fn move_focus(&mut self, lay: &Layout, cur: Option<Id>, d: Dir) {
        let f = Self::focusables(lay);
        let Some(cur) = cur else { return };
        let Some((_, r)) = f.iter().find(|(id, _)| *id == cur) else { return };
        let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
        let mut best: Option<(i64, Id)> = None;
        for (id, o) in &f {
            if *id == cur {
                continue;
            }
            let (ox, oy) = (o.x + o.w / 2, o.y + o.h / 2);
            let (dx, dy) = ((ox - cx) as i64, (oy - cy) as i64);
            let score = match d {
                Dir::Up if dy < 0 => -dy + 2 * dx.abs(),
                Dir::Down if dy > 0 => dy + 2 * dx.abs(),
                Dir::Left if dx < 0 && dy.abs() < r.h as i64 => -dx + 2 * dy.abs(),
                Dir::Right if dx > 0 && dy.abs() < r.h as i64 => dx + 2 * dy.abs(),
                _ => continue,
            };
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, *id));
            }
        }
        if let Some((_, id)) = best {
            self.focus = Some(id);
            self.ensure_visible(lay, id);
        }
    }

    fn scroll_by(&mut self, lay: &Layout, dy: i32) {
        self.scroll = (self.scroll + dy).clamp(0, Self::max_scroll(lay));
    }

    /// 滑鼠滾輪。`dy` 正值往上捲（內容往下），單位是列。
    pub fn wheel(&mut self, dy: i32, ctl: &Controls, devs: &Devices) {
        let lay = self.layout(ctl, devs);
        let step = pm(&self.geom).rh * 2;
        self.scroll_by(&lay, -dy * step);
        self.version += 1;
    }

    /// 欄位被啟動（Enter 或點擊）。
    fn activate(&mut self, id: Id, via: Option<K>, cx: &mut Cx, out: &mut PanelOut) {
        self.focus = Some(id);
        match id {
            Id::PadAssign { dev, player } => {
                let name = cx.devs.find(dev).map(|d| d.name.clone()).unwrap_or_default();
                self.msg = match cx.devs.toggle(player, dev) {
                    Ok(true) => text::msg_pad_assigned(&name, player),
                    Ok(false) => text::msg_pad_unassigned(player),
                    Err(AssignError::Taken(o)) => text::msg_pad_taken(&name, o),
                    Err(AssignError::Unsupported) => text::msg_pad_unsupported(&name),
                    Err(AssignError::Gone) => text::MSG_PAD_GONE.to_string(),
                };
            }
            Id::PadFn { f, slot } => {
                self.capture = Some(Capture::Pad { f, slot, wait: self.start_wait(via) });
                self.msg.clear();
            }
            Id::PadReset => {
                cx.ctl.settings.pad.reset();
                self.msg = text::PADKEY_RESET_DONE.to_string();
                out.eff.save = true;
            }
            Id::Key { player, act } => {
                self.capture = Some(Capture::Key { player, act, wait: self.start_wait(via) });
                self.msg.clear();
            }
            Id::KeyReset => {
                cx.ctl.settings.keys.reset();
                self.msg = text::KB_RESET_DONE.to_string();
                out.eff.save = true;
            }
            Id::Opt(_) | Id::Tween | Id::Counter | Id::Scale | Id::Toolbar | Id::Mode => self.adjust(id, 1, cx, out),
        }
    }

    /// 開關與選項換值。`delta` 為 -1 往前、1 往後。
    fn adjust(&mut self, id: Id, delta: i32, cx: &mut Cx, out: &mut PanelOut) {
        match id {
            Id::Opt(i) => {
                let o = &OPTIONS[i];
                if !o.implemented {
                    self.msg = text::msg_in_development(o.label);
                    return;
                }
                if i == FEEL_INDEX {
                    out.eff.merge(cx.ctl.apply(Action::Feel));
                    return;
                }
                let v = cx.ctl.settings.opt_value(i);
                cx.ctl.settings.set_opt_value(i, if delta < 0 { o.prev(v) } else { o.next(v) });
                out.eff.save = true;
            }
            Id::Tween => out.eff.merge(cx.ctl.apply(Action::Tween)),
            Id::Counter => out.eff.merge(cx.ctl.apply(Action::Counter)),
            Id::Scale => out.eff.merge(cx.ctl.apply(Action::Scale)),
            Id::Toolbar => out.eff.merge(cx.ctl.apply(Action::Toolbar)),
            Id::Mode => {
                // 動作只有「下一個」，往前等於繞一圈少一步。單色圖庫不可用時只有 VGA 與 EGA 兩個。
                let available = if cx.ctl.mono_available { DisplayMode::ALL.len() } else { 2 };
                let steps = if delta < 0 { available - 1 } else { 1 };
                for _ in 0..steps {
                    out.eff.merge(cx.ctl.apply(Action::Mode));
                }
            }
            _ => {}
        }
    }

    /// 指標按下（面板座標）。
    pub fn press(&mut self, _x: i32, y: i32) {
        self.press = Some(Press { y, scroll0: self.scroll, dragged: false });
        // 按下的目標就是指標所在的目標（`hover_at` 追蹤）；觸控沒有 hover，就沒有按下的變深。
        self.down = self.hover;
        self.version += 1;
    }

    /// 指標移動到 `(x, y)`（設定畫面座標）：追蹤滑鼠移過的目標，變了就要重畫。
    pub fn hover_at(&mut self, x: i32, y: i32, ctl: &Controls, devs: &Devices) {
        let lay = self.layout(ctl, devs);
        let h = self.hit(&lay, x - lay.card.x, y - lay.card.y);
        if h != self.hover {
            self.hover = h;
            self.version += 1;
        }
    }

    /// 指標離開視窗或設定畫面關閉時清掉移過與按下的狀態。
    pub fn clear_hover(&mut self) {
        if self.hover != Hit::None || self.down != Hit::None {
            self.hover = Hit::None;
            self.down = Hit::None;
            self.version += 1;
        }
    }

    /// 目前移過的是不是 `id` 這個欄位（測試用）。
    pub fn hovering(&self, id: Id) -> bool {
        self.hover == Hit::Cell(id)
    }

    /// 指標拖曳。垂直移動超過門檻就變成捲動，放開時不算點擊。
    pub fn drag(&mut self, y: i32, ctl: &Controls, devs: &Devices) {
        let Some(pr) = self.press else { return };
        let threshold = (pm(&self.geom).fh / 2).max(6);
        let dy = y - pr.y;
        let dragged = pr.dragged || dy.abs() > threshold;
        if dragged {
            let lay = self.layout(ctl, devs);
            self.scroll = (pr.scroll0 - dy).clamp(0, Self::max_scroll(&lay));
            self.version += 1;
        }
        self.press = Some(Press { dragged, ..pr });
    }

    /// 指標放開。沒有拖曳就是點擊。
    pub fn release(&mut self, x: i32, y: i32, cx: &mut Cx) -> PanelOut {
        self.down = Hit::None;
        self.version += 1;
        let Some(pr) = self.press.take() else { return PanelOut::default() };
        if pr.dragged {
            return PanelOut::default();
        }
        self.click(x, y, cx)
    }

    /// 點擊（面板座標）。
    pub fn click(&mut self, x: i32, y: i32, cx: &mut Cx) -> PanelOut {
        self.version += 1;
        let mut out = PanelOut::default();
        let lay = self.layout(cx.ctl, cx.devs);
        let (x, y) = (x - lay.card.x, y - lay.card.y);
        let hit = self.hit(&lay, x, y);
        // 重綁中點任何地方：先取消。點到的是別的欄位就接著啟動它。
        let was_capturing = self.capture.is_some();
        if was_capturing {
            let target = self.capture;
            self.cancel_capture();
            if let Hit::Cell(id) = hit {
                let same = match (target, id) {
                    (Some(Capture::Key { player, act, .. }), Id::Key { player: p, act: a }) => player == p && act == a,
                    (Some(Capture::Pad { f, slot, .. }), Id::PadFn { f: pf, slot: ps }) => f == pf && slot == ps,
                    _ => false,
                };
                if !same {
                    self.activate(id, None, cx, &mut out);
                }
            }
            return out;
        }
        match hit {
            Hit::Close => out.close = true,
            Hit::Tab(p) => self.set_page(p),
            Hit::Cell(id) => match self.seg_target(&lay, cx.ctl, id, x) {
                Some(i) => self.pick_segment(id, i, cx, &mut out),
                None => self.activate(id, None, cx, &mut out),
            },
            Hit::None => {}
        }
        out
    }

    /// 點在分段按鈕的哪一段（0 起算）。不是分段按鈕或停用的回傳 `None`。`x` 是卡片座標。
    fn seg_target(&self, lay: &Layout, ctl: &Controls, id: Id, x: i32) -> Option<usize> {
        let cell = lay.cells.iter().find(|c| c.id == Some(id) && c.enabled)?;
        let w = self.widget_of(cell, ctl);
        let Widget::Seg { labels, .. } = &w else { return None };
        let p = pm(&self.geom);
        let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        let widths = style::seg_widths(&refs, p.fh, seg_pad(&p));
        let rects = style::seg_rects(Self::control_rect(&w, cell.rect, &p), &widths, 2);
        let lx = x - lay.view.x;
        rects.iter().position(|r| lx >= r.x && lx < r.right())
    }

    /// 點分段按鈕的第 `i` 段。已經是這一段，或只有兩個值時，行為同一般點擊（換到下一個值）；
    /// 三個值以上點別段就直接換到那一段。
    fn pick_segment(&mut self, id: Id, i: usize, cx: &mut Cx, out: &mut PanelOut) {
        self.focus = Some(id);
        match id {
            Id::Scale => {
                let target = SCALES.get(i).copied();
                match target {
                    Some(t) if t != cx.ctl.settings.scale => {
                        // 倍率只有「下一個」的動作，轉到目標為止（最多繞一圈）。
                        for _ in 0..SCALES.len() {
                            out.eff.merge(cx.ctl.apply(Action::Scale));
                            if cx.ctl.settings.scale == t {
                                break;
                            }
                        }
                    }
                    _ => self.adjust(id, 1, cx, out),
                }
            }
            Id::Mode => match DisplayMode::ALL.get(i).copied() {
                Some(t) if t.is_mono() && !cx.ctl.mono_available => self.msg = text::ST_MONO_MISSING.to_string(),
                Some(t) if t != cx.ctl.settings.mode => {
                    for _ in 0..DisplayMode::ALL.len() {
                        out.eff.merge(cx.ctl.apply(Action::Mode));
                        if cx.ctl.settings.mode == t {
                            break;
                        }
                    }
                }
                _ => self.adjust(id, 1, cx, out),
            },
            Id::Opt(o) if OPTIONS[o].implemented && OPTIONS[o].values() > 2 && i < OPTIONS[o].values() as usize && cx.ctl.settings.opt_value(o) as usize != i => {
                cx.ctl.settings.set_opt_value(o, i as u8);
                out.eff.save = true;
            }
            _ => self.activate(id, None, cx, out),
        }
    }

    fn hit(&self, lay: &Layout, x: i32, y: i32) -> Hit {
        if lay.close.contains(x, y) {
            return Hit::Close;
        }
        if let Some((p, _)) = lay.tabs.iter().find(|(_, r)| r.contains(x, y)) {
            return Hit::Tab(*p);
        }
        if lay.view.contains(x, y) {
            let scroll = self.scroll.clamp(0, Self::max_scroll(lay));
            let (cx, cy) = (x - lay.view.x, y - lay.view.y + scroll);
            if let Some(c) = lay.cells.iter().find(|c| c.id.is_some() && c.enabled && c.rect.contains(cx, cy)) {
                return Hit::Cell(c.id.unwrap_or(Id::KeyReset));
            }
            // 灰掉的欄位（開發中）也要有反應：點了顯示訊息。
            if let Some(c) = lay.cells.iter().find(|c| c.id.is_some() && c.rect.contains(cx, cy)) {
                return Hit::Cell(c.id.unwrap_or(Id::KeyReset));
            }
        }
        Hit::None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Close,
    Tab(Page),
    Cell(Id),
    None,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn ctl() -> Controls {
        Controls::new(Settings::default(), true)
    }

    fn panel(page: Page) -> Panel {
        Panel::new(page, 960, 600, false, 1.0)
    }

    fn find(p: &Panel, c: &Controls, d: &Devices, id: Id) -> CellInfo {
        p.describe(c, d).into_iter().find(|i| i.id == Some(id)).unwrap_or_else(|| panic!("版面上沒有 {id:?}"))
    }

    fn click_id(p: &mut Panel, c: &mut Controls, d: &mut Devices, id: Id) -> PanelOut {
        // 先捲到看得見，再點中心。
        let lay = p.layout(c, d);
        p.ensure_visible(&lay, id);
        let r = find(p, c, d, id).rect;
        p.click(r.x + r.w / 2, r.y + r.h / 2, &mut Cx { ctl: c, devs: d })
    }

    fn key(p: &mut Panel, c: &mut Controls, d: &mut Devices, k: K) -> PanelOut {
        p.key_down(k, false, &mut Cx { ctl: c, devs: d })
    }

    fn devs2() -> Devices {
        let mut d = Devices::new();
        d.add(10, "Pad A", true);
        d.add(20, "Pad B", true);
        d
    }

    // -------------------------------------------------------------------
    // 版面：每頁在各種視窗大小都排得下、不超出、不重疊
    // -------------------------------------------------------------------

    fn sizes() -> Vec<(i32, i32, bool, f32)> {
        vec![
            (640, 400, false, 1.0),
            (640, 354, false, 1.0),
            (960, 600, false, 1.0),
            (1280, 800, false, 1.0),
            (1600, 1000, false, 1.0),
            (2560, 1000, false, 1.0),
            (2400, 880, true, 2.5),
            (1280, 560, true, 2.0),
            (1920, 800, true, 3.0),
            (800, 400, true, 1.5),
        ]
    }

    fn all_states() -> Vec<(Controls, Devices)> {
        let mut many = Devices::new();
        many.add(1, "Xbox Wireless Controller", true);
        many.add(2, "一支名字非常非常長非常非常長的手把裝置 Pro Edition", true);
        many.add(3, "No Mapping Joystick", false);
        many.assign(0, 1).unwrap();
        let mut c1 = ctl();
        c1.settings.keys.clear(0, Act::Up);
        let mut c2 = ctl();
        c2.hd_available = false;
        vec![(ctl(), Devices::new()), (ctl(), devs2()), (c1, many), (c2, devs2())]
    }

    #[test]
    fn every_page_fits_every_window_size_without_overlap_or_overflow() {
        let f = font::builtin();
        for (w, h, mobile, dp) in sizes() {
            for (c, d) in all_states() {
                for page in PAGES {
                    let mut c = c;
                    c.mobile = mobile;
                    c.dp = dp;
                    let pn = Panel::new(page, w, h, mobile, dp);
                    let lay = pn.layout(&c, &d);
                    let p = pm(&pn.geom);
                    let tag = format!("{page:?} {w}x{h} mobile={mobile}");
                    // 捲動區在面板之內，頁籤、標題與關閉鈕互不重疊。
                    assert!(lay.view.x >= 0 && lay.view.right() <= w && lay.view.bottom() <= h, "{tag} view {:?}", lay.view);
                    assert!(lay.msg.bottom() <= h && lay.msg.right() <= w, "{tag}");
                    assert!(lay.close.right() <= w && lay.close.bottom() <= lay.view.y, "{tag}");
                    assert!(lay.title.right() <= lay.close.x, "{tag}");
                    for (i, (_, r)) in lay.tabs.iter().enumerate() {
                        assert!(r.right() <= w && r.bottom() <= lay.view.y, "{tag} 頁籤 {r:?}");
                        if i > 0 {
                            assert!(r.x >= lay.tabs[i - 1].1.right(), "{tag} 頁籤重疊");
                        }
                        // 頁籤不蓋到標題與關閉鈕（手機版同一列時要排在它們之間）。
                        for other in [lay.title, lay.close] {
                            let o = r.intersect(&other);
                            assert!(o.w == 0 || o.h == 0, "{tag} 頁籤 {r:?} 蓋到 {other:?}");
                        }
                    }
                    for (pg, r) in &lay.tabs {
                        let _ = pg;
                        // 頁籤的字放得進去（放不下才會截斷，這些視窗都放得下）。
                        if w >= 640 {
                            assert!(f.text_width(pg.label(), p.fh) <= r.w, "{tag} 頁籤字 {}", pg.label());
                        }
                    }
                    assert!(lay.view.h > p.rh * 2, "{tag} 捲動區太矮 {}", lay.view.h);
                    // 內容：在寬度之內，互不重疊，字放得進框。
                    for (i, a) in lay.cells.iter().enumerate() {
                        assert!(a.rect.x >= 0 && a.rect.right() <= lay.view.w + 1 && a.rect.y >= 0 && a.rect.bottom() <= lay.content_h, "{tag} 超出內容 {a:?}");
                        if a.kind == CK::Box || a.kind == CK::Tag {
                            let inner = a.rect.w - 2 * if a.kind == CK::Box { p.pad } else { 0 };
                            if a.kind == CK::Box && a.id.is_some() {
                                assert!(f.text_width(&a.text, a.fh) <= inner, "{tag} 欄位字超出：{:?} {} > {}", a.text, f.text_width(&a.text, a.fh), inner);
                            }
                        }
                        if matches!(a.kind, CK::Label | CK::Header | CK::Note | CK::Tag) {
                            assert!(f.text_width(&a.text, a.fh) <= a.rect.w, "{tag} 文字超出：{:?} {} > {}", a.text, f.text_width(&a.text, a.fh), a.rect.w);
                            let from_device = d.list().iter().any(|x| a.text.contains(&x.name));
                            assert!(from_device || f.missing(&a.text).is_empty(), "{tag} 缺字 {:?}", f.missing(&a.text));
                        }
                        for b in &lay.cells[i + 1..] {
                            let o = a.rect.intersect(&b.rect);
                            assert!(o.w == 0 || o.h == 0, "{tag} 重疊：{:?} 與 {:?}", a.text, b.text);
                        }
                        // 手機版可點的欄位至少 48 dp 高。
                        if mobile && a.id.is_some() {
                            assert!(a.rect.h as f32 >= 48.0 * dp - 1.0, "{tag} 欄位高 {} < 48 dp", a.rect.h);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rendered_text_stays_inside_cells_and_the_viewport() {
        // 畫出來的像素只在標題、頁籤、捲動區與訊息列之內：卡片裡捲動區與頁籤之間、訊息列與捲動區之間的縫，
        // 以及卡片左右的留白，都是卡片底色。
        for (w, h, mobile, dp) in [(640, 400, false, 1.0), (960, 600, false, 1.0), (2400, 880, true, 2.5)] {
            for page in PAGES {
                let mut c = ctl();
                c.mobile = mobile;
                c.dp = dp;
                let d = devs2();
                let mut pn = Panel::new(page, w, h, mobile, dp);
                pn.msg = "測試訊息".into();
                // 捲到最下面再畫一次，內容被裁在捲動區裡。
                for scroll_to_end in [false, true] {
                    if scroll_to_end {
                        pn.scroll = i32::MAX / 2;
                        let lay = pn.layout(&c, &d);
                        pn.clamp_scroll(&lay);
                    }
                    let lay = pn.layout(&c, &d);
                    let px = pn.render(&c, &d);
                    let p = pm(&pn.geom);
                    let at = |x: i32, y: i32| {
                        let o = (((lay.card.y + y) * w + lay.card.x + x) * 4) as usize;
                        [px[o], px[o + 1], px[o + 2], px[o + 3]]
                    };
                    let gap_rows = [(lay.tabs[0].1.bottom()..lay.view.y), (lay.view.bottom()..lay.msg.y)];
                    for rows in gap_rows {
                        for y in rows {
                            for x in 2..lay.card.w - 2 {
                                assert_eq!(at(x, y), BG, "{page:?} {w}x{h}：縫 ({x},{y}) 有東西");
                            }
                        }
                    }
                    // 卡片左右的留白（避開圓角與邊線）是底色。
                    for y in 16..lay.card.h - 16 {
                        for x in (2..p.mar / 2).chain(lay.card.w - p.mar / 2..lay.card.w - 2) {
                            assert_eq!(at(x, y), BG, "{page:?} {w}x{h}：留白 ({x},{y})");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn long_pages_scroll_and_short_pages_do_not() {
        let c = ctl();
        let d = devs2();
        // 640x354 的小視窗：鍵盤頁放不下，要能捲。
        let small = Panel::new(Page::Keyboard, 640, 354, false, 1.0);
        let (s, content, view) = small.scroll_range(&c, &d);
        assert!(content > view && s == 0, "{content} {view}");
        // 大視窗：顯示頁放得下。
        let big = Panel::new(Page::Display, 1600, 1000, false, 1.0);
        let (_, content, view) = big.scroll_range(&c, &d);
        assert!(content <= view);
    }

    // -------------------------------------------------------------------
    // 換頁、焦點
    // -------------------------------------------------------------------

    #[test]
    fn page_names_and_titles() {
        assert_eq!(Page::from_name("手把"), Some(Page::Pad));
        assert_eq!(Page::from_name("手把按鍵"), Some(Page::PadKeys));
        assert_eq!(Page::from_name("鍵盤"), Some(Page::Keyboard));
        assert_eq!(Page::from_name("遊戲"), Some(Page::Game));
        assert_eq!(Page::from_name("顯示"), Some(Page::Display));
        assert_eq!(Page::from_name("keyboard"), Some(Page::Keyboard));
        assert_eq!(Page::from_name("PAD"), Some(Page::Pad));
        assert_eq!(Page::from_name("不存在"), None);
        assert_eq!(Page::Pad.title(), "控制設定");
        assert_eq!(Page::Keyboard.title(), "控制設定");
        assert_eq!(Page::Game.title(), "遊戲設定");
        assert_eq!(Page::Display.title(), "遊戲設定");
    }

    #[test]
    fn f8_cycles_the_pages_in_order() {
        let mut p = panel(Page::Pad);
        let v = p.version();
        let mut seen = vec![p.page()];
        for _ in 0..5 {
            p.next_page(1);
            seen.push(p.page());
        }
        assert_eq!(seen, vec![Page::Pad, Page::PadKeys, Page::Keyboard, Page::Game, Page::Display, Page::Pad]);
        p.next_page(-1);
        assert_eq!(p.page(), Page::Display);
        assert!(p.version() > v);
    }

    #[test]
    fn tab_walks_the_fields_and_wraps_and_shift_tab_goes_back() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Keyboard);
        let first = p.current_focus(&p.layout(&c, &d));
        assert_eq!(first, Some(Id::Key { player: 0, act: Act::Up }));
        let mut order = vec![first.unwrap()];
        for _ in 0..(2 * 11 + 1) {
            key(&mut p, &mut c, &mut d, K::Tab);
            order.push(p.current_focus(&p.layout(&c, &d)).unwrap());
        }
        // 先走完第 1 人再走第 2 人，再來是還原預設，再回到開頭。
        assert_eq!(order[1], Id::Key { player: 0, act: Act::Down });
        assert_eq!(order[11], Id::Key { player: 1, act: Act::Up });
        assert_eq!(order[22], Id::KeyReset);
        assert_eq!(order[23], order[0], "循環");
        p.key_down(K::Tab, true, &mut Cx { ctl: &mut c, devs: &mut d });
        assert_eq!(p.current_focus(&p.layout(&c, &d)), Some(Id::KeyReset), "Shift+Tab 往前，從開頭繞到最後");
    }

    #[test]
    fn arrow_keys_move_the_focus_geometrically() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Keyboard);
        key(&mut p, &mut c, &mut d, K::Down);
        assert_eq!(p.current_focus(&p.layout(&c, &d)), Some(Id::Key { player: 0, act: Act::Down }));
        key(&mut p, &mut c, &mut d, K::Right);
        assert_eq!(p.current_focus(&p.layout(&c, &d)), Some(Id::Key { player: 1, act: Act::Down }), "右邊是第 2 人同一列");
        key(&mut p, &mut c, &mut d, K::Left);
        key(&mut p, &mut c, &mut d, K::Up);
        assert_eq!(p.current_focus(&p.layout(&c, &d)), Some(Id::Key { player: 0, act: Act::Up }));
        key(&mut p, &mut c, &mut d, K::Up);
        assert_eq!(p.current_focus(&p.layout(&c, &d)), Some(Id::Key { player: 0, act: Act::Up }), "最上面再往上不動");
    }

    #[test]
    fn focus_follows_scrolling() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = Panel::new(Page::Keyboard, 640, 354, false, 1.0);
        for _ in 0..22 {
            key(&mut p, &mut c, &mut d, K::Tab);
        }
        let f = find(&p, &c, &d, Id::KeyReset);
        assert!(f.visible && f.focused, "焦點移到最後一個欄位時自動捲到看得見");
        assert!(p.scroll_range(&c, &d).0 > 0);
    }

    #[test]
    fn escape_and_close_button_close_the_panel() {
        let (mut c, mut d) = (ctl(), devs2());
        let mut p = panel(Page::Game);
        assert!(key(&mut p, &mut c, &mut d, K::Escape).close);
        let lay = p.layout(&c, &d);
        let r = lay.close.offset(lay.card.x, lay.card.y);
        assert!(p.click(r.x + 1, r.y + 1, &mut Cx { ctl: &mut c, devs: &mut d }).close);
    }

    #[test]
    fn tabs_switch_pages_by_click() {
        let (mut c, mut d) = (ctl(), devs2());
        let mut p = panel(Page::Pad);
        for pg in PAGES {
            let lay = p.layout(&c, &d);
            let r = lay.tabs.iter().find(|(x, _)| *x == pg).unwrap().1.offset(lay.card.x, lay.card.y);
            let out = p.click(r.x + r.w / 2, r.y + r.h / 2, &mut Cx { ctl: &mut c, devs: &mut d });
            assert!(!out.close);
            assert_eq!(p.page(), pg);
        }
    }

    // -------------------------------------------------------------------
    // 鍵盤頁：重綁、衝突、清空、還原預設（規格 005 E5）
    // -------------------------------------------------------------------

    #[test]
    fn rebind_by_click_then_key() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Keyboard);
        let id = Id::Key { player: 0, act: Act::Fire };
        let out = click_id(&mut p, &mut c, &mut d, id);
        assert!(p.capturing() && !out.eff.save);
        assert_eq!(find(&p, &c, &d, id).text, "請按新鍵");
        let out = key(&mut p, &mut c, &mut d, K::Space);
        assert!(!p.capturing() && out.eff.save, "綁好之後要存檔");
        assert_eq!(c.settings.keys.get(0, Act::Fire), Some(K::Space));
        assert_eq!(find(&p, &c, &d, id).text, "空白鍵");
        assert_eq!(p.message(), "第 1 人開火：空白鍵");
    }

    #[test]
    fn rebind_by_enter_waits_for_the_enter_key_to_be_released() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Keyboard);
        key(&mut p, &mut c, &mut d, K::Return);
        assert!(p.capturing());
        // Enter 還沒放開：它的重複事件不算新鍵。
        let out = key(&mut p, &mut c, &mut d, K::Return);
        assert!(p.capturing() && !out.eff.save);
        assert_eq!(c.settings.keys.get(0, Act::Up), Some(K::Up));
        p.key_up(K::Return);
        // 現在才能綁 Enter 本身。
        let out = key(&mut p, &mut c, &mut d, K::Return);
        assert!(!p.capturing() && out.eff.save);
        assert_eq!(c.settings.keys.get(0, Act::Up), Some(K::Return));
        // 啟動重綁之後按別的鍵，不必等 Enter 放開。
        key(&mut p, &mut c, &mut d, K::Down);
        key(&mut p, &mut c, &mut d, K::Return);
        key(&mut p, &mut c, &mut d, K::W);
        assert_eq!(c.settings.keys.get(0, Act::Down), Some(K::W));
    }

    #[test]
    fn conflict_is_reported_and_never_overwrites() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Keyboard);
        let before = c.settings.keys;
        click_id(&mut p, &mut c, &mut d, Id::Key { player: 1, act: Act::Up });
        // 第 1 人的右 Shift 已經是開火。
        let out = key(&mut p, &mut c, &mut d, K::RShift);
        assert_eq!(c.settings.keys, before, "衝突時不覆寫");
        assert!(!out.eff.save);
        assert_eq!(p.message(), "右 Shift 已綁定第 1 人開火，沒有覆寫。");
        assert!(p.capturing(), "留在等待狀態，可以換一個鍵");
        // 不能綁的鍵（F 鍵是前端熱鍵）：訊息，不改。
        key(&mut p, &mut c, &mut d, K::F2);
        assert_eq!(c.settings.keys, before);
        assert!(p.message().ends_with("不能綁定。") || p.message().contains("不能綁定"), "{}", p.message());
        // 小鍵盤 8 是第 1 人上的孿生鍵，也衝突。
        key(&mut p, &mut c, &mut d, K::Kp8);
        assert!(p.message().contains("第 1 人上"), "{}", p.message());
        assert_eq!(c.settings.keys, before);
        // 換一個沒用到的鍵就成功。
        key(&mut p, &mut c, &mut d, K::U);
        assert_eq!(c.settings.keys.get(1, Act::Up), Some(K::U));
        assert!(!p.capturing());
    }

    #[test]
    fn escape_cancels_and_keeps_the_old_binding() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Keyboard);
        let before = c.settings.keys;
        click_id(&mut p, &mut c, &mut d, Id::Key { player: 0, act: Act::Left });
        let out = key(&mut p, &mut c, &mut d, K::Escape);
        assert!(!out.close, "重綁中 Esc 只取消重綁，不關設定畫面");
        assert!(!p.capturing() && c.settings.keys == before);
        assert_eq!(p.message(), "已取消，保留原鍵位。");
        // 再按一次 Esc 才關閉。
        assert!(key(&mut p, &mut c, &mut d, K::Escape).close);
    }

    #[test]
    fn delete_clears_a_binding_both_while_capturing_and_on_a_focused_field() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Keyboard);
        click_id(&mut p, &mut c, &mut d, Id::Key { player: 1, act: Act::Turn });
        let out = key(&mut p, &mut c, &mut d, K::Delete);
        assert!(out.eff.save && !p.capturing());
        assert_eq!(c.settings.keys.get(1, Act::Turn), None);
        assert_eq!(find(&p, &c, &d, Id::Key { player: 1, act: Act::Turn }).text, "未綁定");
        // 不在重綁中，焦點在欄位上按 Delete。
        p.focus = Some(Id::Key { player: 0, act: Act::Split });
        key(&mut p, &mut c, &mut d, K::Delete);
        assert_eq!(c.settings.keys.get(0, Act::Split), None);
        assert!(p.message().starts_with("已清空第 1 人"), "{}", p.message());
        // 清空後那顆鍵可以給別人。
        click_id(&mut p, &mut c, &mut d, Id::Key { player: 0, act: Act::Up });
        key(&mut p, &mut c, &mut d, K::Z);
        assert_eq!(c.settings.keys.get(0, Act::Up), Some(K::Z));
    }

    #[test]
    fn reset_button_restores_both_players_defaults() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Keyboard);
        c.settings.keys.clear(0, Act::Fire);
        c.settings.keys.bind(0, Act::Fire, K::Space).unwrap();
        c.settings.keys.clear(1, Act::Up);
        let out = click_id(&mut p, &mut c, &mut d, Id::KeyReset);
        assert!(out.eff.save);
        assert_eq!(c.settings.keys, crate::keymap::KeyMap::default());
        assert_eq!(p.message(), "已還原兩人鍵盤預設。");
        // Enter 啟動也行。
        c.settings.keys.clear(0, Act::Up);
        p.focus = Some(Id::KeyReset);
        assert!(key(&mut p, &mut c, &mut d, K::Return).eff.save);
        assert_eq!(c.settings.keys, crate::keymap::KeyMap::default());
    }

    #[test]
    fn clicking_elsewhere_while_capturing_cancels_or_switches() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Keyboard);
        let a = Id::Key { player: 0, act: Act::Up };
        let b = Id::Key { player: 0, act: Act::Down };
        click_id(&mut p, &mut c, &mut d, a);
        click_id(&mut p, &mut c, &mut d, a);
        assert!(!p.capturing(), "再點同一個欄位取消");
        click_id(&mut p, &mut c, &mut d, a);
        click_id(&mut p, &mut c, &mut d, b);
        assert!(p.capturing() && p.is_capture_target(Some(b)), "點別的欄位改成重綁那個");
        p.focus_lost();
        assert!(!p.capturing());
        assert_eq!(p.message(), "失去焦點，已取消重綁。");
    }

    #[test]
    fn keys_pressed_in_the_panel_never_reach_the_game_mapper() {
        // 設定畫面開著時輸入對映器被擋住：用 Enter、方向鍵、Esc 操作設定畫面，一個碼都不送進遊戲。
        use crate::input::{InputMapper, KeyResult, Source};
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Keyboard);
        let mut m = InputMapper::new();
        m.block();
        for k in [K::Down, K::Return, K::W, K::Right, K::Tab, K::Escape] {
            let r = m.key_down(k, false, 0);
            assert!(!matches!(r, KeyResult::Game), "{k:?} 被送進遊戲");
            key(&mut p, &mut c, &mut d, k);
        }
        for k in [K::Down, K::Return, K::W, K::Right, K::Tab, K::Escape] {
            m.key_up(k);
            p.key_up(k);
        }
        m.unblock();
        assert!(m.is_armed(Source::Keyboard));
        let mut v = m.take();
        v.extend(m.take());
        assert!(v.is_empty(), "設定畫面的按鍵外漏進遊戲：{v:?}");
    }

    // -------------------------------------------------------------------
    // 手把頁：指派
    // -------------------------------------------------------------------

    #[test]
    fn no_pad_shows_the_pairing_hint() {
        let (c, d) = (ctl(), Devices::new());
        let p = panel(Page::Pad);
        let cells = p.describe(&c, &d);
        assert!(cells.iter().any(|i| i.text == "尚未偵測到手把。USB／藍牙配對請由作業系統完成。"));
        assert!(cells.iter().all(|i| i.id.is_none()), "沒有手把就沒有可操作的欄位");
    }

    #[test]
    fn assigning_pads_to_players_and_rejecting_duplicates() {
        let (mut c, mut d) = (ctl(), devs2());
        let mut p = panel(Page::Pad);
        let out = click_id(&mut p, &mut c, &mut d, Id::PadAssign { dev: 10, player: 0 });
        assert_eq!(d.assigned_id(0), Some(10));
        assert!(!out.eff.save, "手把指派不存檔");
        assert_eq!(p.message(), "已把 Pad A 指派給第 1 人。");
        assert_eq!(find(&p, &c, &d, Id::PadAssign { dev: 10, player: 0 }).text, "第 1 人：已指派");
        // 同一支給第 2 人：拒絕。
        click_id(&mut p, &mut c, &mut d, Id::PadAssign { dev: 10, player: 1 });
        assert_eq!(d.assigned_id(1), None);
        assert_eq!(d.assigned_id(0), Some(10), "拒絕時不改原本的指派");
        assert_eq!(p.message(), "Pad A 已經指派給第 1 人，不能重複指派。");
        // 另一支給第 2 人：成功。
        click_id(&mut p, &mut c, &mut d, Id::PadAssign { dev: 20, player: 1 });
        assert_eq!((d.assigned_id(0), d.assigned_id(1)), (Some(10), Some(20)));
        // 再按一次已指派的欄位就取消。
        click_id(&mut p, &mut c, &mut d, Id::PadAssign { dev: 10, player: 0 });
        assert_eq!(d.assigned_id(0), None);
        assert_eq!(p.message(), "已取消第 1 人的手把指派。");
        // 現在第 1 人那一支可以給第 2 人嗎：10 沒指派，第 2 人換成 10，20 被放掉。
        click_id(&mut p, &mut c, &mut d, Id::PadAssign { dev: 10, player: 1 });
        assert_eq!((d.assigned_id(0), d.assigned_id(1)), (None, Some(10)));
    }

    #[test]
    fn unsupported_pad_is_listed_but_cannot_be_assigned() {
        let (c, mut d) = (ctl(), devs2());
        d.add(30, "Odd Stick", false);
        let p = panel(Page::Pad);
        let cells = p.describe(&c, &d);
        assert!(cells.iter().any(|i| i.text == "Odd Stick（沒有標準配置，不支援）"));
        assert!(!cells.iter().any(|i| matches!(i.id, Some(Id::PadAssign { dev: 30, .. }))), "不支援的手把沒有指派鈕");
        assert!(d.assign(0, 30).is_err());
    }

    #[test]
    fn unplugging_removes_the_assignment_from_the_screen() {
        let (c, mut d) = (ctl(), devs2());
        d.assign(0, 10).unwrap();
        let p = panel(Page::Pad);
        assert!(p.describe(&c, &d).iter().any(|i| i.text == "第 1 人：Pad A"));
        d.remove(10);
        let cells = p.describe(&c, &d);
        assert!(cells.iter().any(|i| i.text == "第 1 人：未指派"));
        assert!(!cells.iter().any(|i| matches!(i.id, Some(Id::PadAssign { dev: 10, .. }))));
        // 焦點原本在已拔掉的手把上，會落到第一個欄位，不會 panic。
        let mut p = p;
        p.focus = Some(Id::PadAssign { dev: 10, player: 0 });
        assert_eq!(p.current_focus(&p.layout(&c, &d)), Some(Id::PadAssign { dev: 20, player: 0 }));
    }

    // -------------------------------------------------------------------
    // 手把按鍵頁
    // -------------------------------------------------------------------

    #[test]
    fn pad_buttons_can_be_rebound_and_conflicts_are_rejected() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::PadKeys);
        let fire = Id::PadFn { f: PadFn::Fire, slot: 0 };
        click_id(&mut p, &mut c, &mut d, fire);
        assert!(p.capturing());
        assert_eq!(find(&p, &c, &d, fire).text, "請按按鈕");
        // B 是轉向用的：拒絕。
        let out = p.pad_button(Btn::B, &mut Cx { ctl: &mut c, devs: &mut d });
        assert!(!out.eff.save && p.capturing());
        assert_eq!(p.message(), "B 已經用在「轉向」，沒有覆寫。");
        assert_eq!(c.settings.pad.get(PadFn::Fire, 0), Some(Btn::A));
        // L3 沒人用：成功。
        let out = p.pad_button(Btn::L3, &mut Cx { ctl: &mut c, devs: &mut d });
        assert!(out.eff.save && !p.capturing());
        assert_eq!(c.settings.pad.get(PadFn::Fire, 0), Some(Btn::L3));
        assert_eq!(find(&p, &c, &d, fire).text, "L3");
        // 不在重綁中時手把按鈕沒有作用。
        assert_eq!(p.pad_button(Btn::X, &mut Cx { ctl: &mut c, devs: &mut d }), PanelOut::default());
    }

    #[test]
    fn pad_button_escape_delete_and_reset() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::PadKeys);
        let id = Id::PadFn { f: PadFn::Pause, slot: 0 };
        click_id(&mut p, &mut c, &mut d, id);
        key(&mut p, &mut c, &mut d, K::Escape);
        assert!(!p.capturing());
        assert_eq!(p.message(), "已取消，保留原按鈕。");
        assert_eq!(c.settings.pad.get(PadFn::Pause, 0), Some(Btn::Start));
        click_id(&mut p, &mut c, &mut d, id);
        let out = key(&mut p, &mut c, &mut d, K::Delete);
        assert!(out.eff.save);
        assert_eq!(c.settings.pad.get(PadFn::Pause, 0), None);
        assert_eq!(find(&p, &c, &d, id).text, "未綁定");
        let out = click_id(&mut p, &mut c, &mut d, Id::PadReset);
        assert!(out.eff.save);
        assert_eq!(c.settings.pad, crate::keymap::PadMap::default());
    }

    #[test]
    fn pad_page_lists_the_six_functions_with_two_slots() {
        let (c, d) = (ctl(), Devices::new());
        let p = panel(Page::PadKeys);
        let cells = p.describe(&c, &d);
        for l in ["開火", "轉向", "分離", "暫停", "確定", "選單", "按鈕 1", "按鈕 2", "還原手把預設"] {
            assert!(cells.iter().any(|i| i.text == l), "缺 {l}");
        }
        assert_eq!(cells.iter().filter(|i| matches!(i.id, Some(Id::PadFn { .. }))).count(), 12);
        assert!(cells.iter().any(|i| i.text.contains("方向固定為十字鍵與左搖桿")));
    }

    // -------------------------------------------------------------------
    // 遊戲頁與顯示頁
    // -------------------------------------------------------------------

    #[test]
    fn game_page_lists_every_option_and_marks_unfinished_ones() {
        let (c, d) = (ctl(), Devices::new());
        let p = Panel::new(Page::Game, 960, 1600, false, 1.0);
        let cells = p.describe(&c, &d);
        for o in OPTIONS.iter() {
            assert!(cells.iter().any(|i| i.text == o.label), "缺 {}", o.label);
        }
        for g in ["視野", "射擊", "練習", "操控", "AI"] {
            assert!(cells.iter().any(|i| i.text == g), "缺群組 {g}");
        }
        let tags = cells.iter().filter(|i| i.text == "開發中").count();
        assert_eq!(tags, OPTIONS.iter().filter(|o| !o.implemented).count());
        // 沒實作的欄位是灰的、不能聚焦。
        for (i, o) in OPTIONS.iter().enumerate() {
            let cell = cells.iter().find(|x| x.id == Some(Id::Opt(i))).unwrap();
            assert_eq!(cell.enabled, o.implemented, "{}", o.id);
        }
    }

    #[test]
    fn unfinished_options_cannot_be_switched_and_do_not_pretend_to_take_effect() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = Panel::new(Page::Game, 960, 1600, false, 1.0);
        let before = c.settings;
        for (i, o) in OPTIONS.iter().enumerate() {
            if o.implemented {
                continue;
            }
            // 點了不能改值，只顯示訊息。
            let out = click_id(&mut p, &mut c, &mut d, Id::Opt(i));
            assert!(!out.eff.save, "{} 不該存檔", o.id);
            assert!(p.message().contains("開發中"), "{}：{}", o.id, p.message());
        }
        assert_eq!(c.settings, before, "沒實作的選項一個值都沒變");
        assert!(!c.settings.rules_enhanced());
        // 鍵盤也選不到它們：Tab 走一圈，焦點只會落在已實作的選項上。
        let mut seen = Vec::new();
        for _ in 0..8 {
            key(&mut p, &mut c, &mut d, K::Tab);
            seen.push(p.current_focus(&p.layout(&c, &d)).unwrap());
        }
        for id in seen {
            if let Id::Opt(i) = id {
                assert!(OPTIONS[i].implemented, "焦點落在沒實作的選項 {}", OPTIONS[i].id);
            }
        }
        // 即使檔案裡把沒實作的選項寫成開，也不算強化規則，因為它不會生效。
        let mut forced = Controls::new(Settings::default(), true);
        for (i, o) in OPTIONS.iter().enumerate() {
            if !o.implemented {
                forced.settings.set_opt_value(i, 1);
            }
        }
        assert!(!forced.settings.rules_enhanced() && !forced.bar_state().rules);
    }

    #[test]
    fn invincible_toggles_and_lights_the_status_flags() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = Panel::new(Page::Game, 960, 1600, false, 1.0);
        let id = Id::Opt(crate::options::INVINCIBLE_INDEX);
        let out = click_id(&mut p, &mut c, &mut d, id);
        assert!(out.eff.save && c.settings.invincible());
        assert_eq!(find(&p, &c, &d, id).text, "開");
        let b = c.bar_state();
        assert!(b.rules && b.god);
        // Enter 與左右鍵也能切。
        p.focus = Some(id);
        key(&mut p, &mut c, &mut d, K::Return);
        assert!(!c.settings.invincible());
        key(&mut p, &mut c, &mut d, K::Right);
        assert!(c.settings.invincible());
        key(&mut p, &mut c, &mut d, K::Left);
        assert!(!c.settings.invincible() && !c.bar_state().rules);
    }

    #[test]
    fn feel_option_switches_between_new_and_original_timing() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = Panel::new(Page::Game, 960, 1600, false, 1.0);
        let id = Id::Opt(FEEL_INDEX);
        assert_eq!(find(&p, &c, &d, id).text, "新");
        click_id(&mut p, &mut c, &mut d, id);
        assert!(!c.settings.feel);
        assert_eq!(find(&p, &c, &d, id).text, "原版時序");
        assert!(!c.bar_state().rules, "手感不改變規則");
        click_id(&mut p, &mut c, &mut d, id);
        assert!(c.settings.feel);
    }

    #[test]
    fn display_page_toggles_go_through_the_same_actions_as_the_hotkeys() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Display);
        let out = click_id(&mut p, &mut c, &mut d, Id::Tween);
        assert!(!c.settings.tween && out.eff.save);
        click_id(&mut p, &mut c, &mut d, Id::Counter);
        assert!(c.settings.counter);
        let out = click_id(&mut p, &mut c, &mut d, Id::Scale);
        assert_eq!(c.settings.scale, 4);
        assert_eq!(out.eff.resize, crate::controls::Resize::Scale);
        assert_eq!(find(&p, &c, &d, Id::Scale).text, "4 倍");
        let out = click_id(&mut p, &mut c, &mut d, Id::Toolbar);
        assert!(!c.settings.toolbar && out.eff.resize == crate::controls::Resize::Toolbar);
        assert_eq!(find(&p, &c, &d, Id::Toolbar).text, "隱藏");
    }

    #[test]
    fn display_mode_row_goes_through_the_same_action_as_the_hotkey() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Display);
        assert_eq!(find(&p, &c, &d, Id::Mode).text, "VGA");
        // 四段，點中心落在第三段。點分段按鈕直接到那一段；鍵盤的左右鍵往前往後。
        let out = click_id(&mut p, &mut c, &mut d, Id::Mode);
        assert_eq!(c.settings.mode, DisplayMode::Cga);
        assert!(out.eff.save);
        let out = {
            let mut cx = Cx { ctl: &mut c, devs: &mut d };
            let mut out = PanelOut::default();
            p.pick_segment(Id::Mode, 3, &mut cx, &mut out);
            out
        };
        assert_eq!(c.settings.mode, DisplayMode::Mga);
        assert!(out.eff.save);
        let mut out = PanelOut::default();
        p.adjust(Id::Mode, -1, &mut Cx { ctl: &mut c, devs: &mut d }, &mut out);
        assert_eq!(c.settings.mode, DisplayMode::Cga);
    }

    #[test]
    fn display_mode_row_refuses_mono_when_the_bank_files_are_missing() {
        let (mut c, mut d) = (ctl(), Devices::new());
        c.set_mono_available(false);
        let mut p = panel(Page::Display);
        let mut out = PanelOut::default();
        p.pick_segment(Id::Mode, 2, &mut Cx { ctl: &mut c, devs: &mut d }, &mut out);
        assert_eq!(c.settings.mode, DisplayMode::Vga, "CGA 不可選");
        assert_eq!(p.msg, text::ST_MONO_MISSING);
        let mut out = PanelOut::default();
        p.adjust(Id::Mode, -1, &mut Cx { ctl: &mut c, devs: &mut d }, &mut out);
        assert_eq!(c.settings.mode, DisplayMode::Ega, "只有兩個可選，往前也是換到另一個");
    }

    #[test]
    fn mobile_display_page_has_no_scale_row() {
        let (c, d) = (ctl(), Devices::new());
        let p = Panel::new(Page::Display, 2400, 880, true, 2.5);
        let cells = p.describe(&c, &d);
        assert!(!cells.iter().any(|i| i.id == Some(Id::Scale) || i.text == "倍率"), "手機版沒有倍率");
        assert!(cells.iter().any(|i| i.id == Some(Id::Toolbar)));
        let desktop = panel(Page::Display).describe(&c, &d);
        assert!(desktop.iter().any(|i| i.id == Some(Id::Scale)));
    }

    // -------------------------------------------------------------------
    // 指標：點擊、拖曳捲動、滾輪
    // -------------------------------------------------------------------

    #[test]
    fn drag_scrolls_and_does_not_click_but_a_tap_does() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = Panel::new(Page::Keyboard, 640, 354, false, 1.0);
        let id = Id::Key { player: 0, act: Act::Up };
        let r = find(&p, &c, &d, id).rect;
        // 拖曳：往上拖，內容往上捲，放開不啟動欄位。
        p.press(r.x + 5, r.y + 5);
        p.drag(r.y - 60, &c, &d);
        let out = p.release(r.x + 5, r.y - 60, &mut Cx { ctl: &mut c, devs: &mut d });
        assert_eq!(out, PanelOut::default());
        assert!(!p.capturing() && p.scroll_range(&c, &d).0 > 0);
        // 輕點：沒有移動，放開時啟動欄位。
        let mut q = Panel::new(Page::Keyboard, 640, 354, false, 1.0);
        q.press(r.x + 5, r.y + 5);
        q.drag(r.y + 6, &c, &d);
        q.release(r.x + 5, r.y + 5, &mut Cx { ctl: &mut c, devs: &mut d });
        assert!(q.capturing(), "小於門檻的移動還是點擊");
        // 滾輪。
        let mut w = Panel::new(Page::Keyboard, 640, 354, false, 1.0);
        w.wheel(-1, &c, &d);
        assert!(w.scroll_range(&c, &d).0 > 0);
        w.wheel(10, &c, &d);
        assert_eq!(w.scroll_range(&c, &d).0, 0);
    }

    #[test]
    fn clicks_in_the_gaps_and_outside_do_nothing() {
        let (mut c, mut d) = (ctl(), devs2());
        let mut p = panel(Page::Game);
        let before = c.settings;
        for (x, y) in [(0, 0), (959, 599), (5, 300), (480, 590)] {
            let out = p.click(x, y, &mut Cx { ctl: &mut c, devs: &mut d });
            assert_eq!(out, PanelOut::default(), "({x},{y})");
        }
        assert_eq!(c.settings, before);
    }

    #[test]
    fn version_changes_with_every_visible_change() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Keyboard);
        let v0 = p.version();
        key(&mut p, &mut c, &mut d, K::Tab);
        let v1 = p.version();
        assert!(v1 > v0);
        p.resize(960, 600, false, 1.0);
        assert_eq!(p.version(), v1, "大小沒變不用重畫");
        p.resize(1280, 800, false, 1.0);
        assert!(p.version() > v1);
    }

    // -------------------------------------------------------------------
    // 繪製
    // -------------------------------------------------------------------

    #[test]
    fn render_dims_the_game_to_60_percent_and_centres_an_opaque_card_with_a_shadow() {
        let (mut c, d) = (ctl(), devs2());
        let (w, h) = (1600, 1000);
        let mut seen = std::collections::HashSet::new();
        for page in PAGES {
            let p = Panel::new(page, w, h, false, 1.0);
            let a = p.render(&c, &d);
            assert_eq!(a.len(), (w * h * 4) as usize);
            let at = |x: i32, y: i32| {
                let o = ((y * w + x) * 4) as usize;
                [a[o], a[o + 1], a[o + 2], a[o + 3]]
            };
            // 遠處的角落是調暗 60% 的半透明黑。
            assert_eq!(at(2, 2), [0, 0, 0, DIM_ALPHA], "{page:?}");
            assert_eq!(DIM_ALPHA, 153, "60% 的 255");
            // 卡片置中且不透明，四周留邊，陰影讓卡片外緣附近比純調暗更深。
            let lay = p.layout(&c, &d);
            let card = lay.card;
            assert_eq!(card.x * 2 + card.w, w, "水平置中");
            assert_eq!(card.y * 2 + card.h, h, "垂直置中");
            assert!(card.x > 0 && card.y > 0 && card.w < w && card.h < h, "四周留一圈");
            assert_eq!(at(card.x + card.w / 2, card.y + 3)[3], 255, "卡片內不透明");
            assert!(at(card.x + card.w / 2, card.y + card.h + 6)[3] > DIM_ALPHA, "卡片下緣有陰影");
            assert!(a.chunks_exact(4).filter(|px| px[3] == 255).count() > 200_000, "{page:?} 卡片畫得太空");
            assert!(seen.insert(a.iter().map(|b| *b as u64).sum::<u64>() * 31 + a[1000] as u64), "{page:?} 和別頁一樣");
        }
        let p = Panel::new(Page::Display, w, h, false, 1.0);
        let a = p.render(&c, &d);
        c.settings.tween = false;
        assert_ne!(a, p.render(&c, &d), "設定值改變，畫面跟著變");
        let mut q = Panel::new(Page::Display, w, h, false, 1.0);
        q.msg = "訊息".into();
        assert_ne!(a, q.render(&c, &d));
    }

    #[test]
    fn opening_fades_in_over_120_ms_without_touching_the_pixels_beneath() {
        assert_eq!(fade_in(0), 0.0);
        assert!((fade_in(60) - 0.5).abs() < 1e-6);
        assert_eq!(fade_in(120), 1.0);
        assert_eq!(fade_in(5000), 1.0);
        assert_eq!((fade_step(0), fade_step(60), fade_step(120), fade_step(999)), (0, FADE_STEPS / 2, FADE_STEPS, FADE_STEPS));
        let (c, d) = (ctl(), devs2());
        let p = panel(Page::Game);
        let full = p.render(&c, &d);
        let half = p.render_faded(&c, &d, 0.5);
        let none = p.render_faded(&c, &d, 0.0);
        assert_eq!(full, p.render_faded(&c, &d, 1.0));
        assert!(none.chunks_exact(4).all(|px| px[3] == 0), "淡入第一刻完全透明");
        for (f, hf) in full.chunks_exact(4).zip(half.chunks_exact(4)) {
            assert_eq!(f[..3], hf[..3], "只改不透明度，顏色不動");
            assert!((hf[3] as i32 - (f[3] as f32 * 0.5).round() as i32).abs() <= 1);
        }
    }

    #[test]
    fn capture_field_is_highlighted_and_focus_ring_is_drawn() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Keyboard);
        let idle = p.render(&c, &d);
        click_id(&mut p, &mut c, &mut d, Id::Key { player: 0, act: Act::Up });
        let capturing = p.render(&c, &d);
        assert_ne!(idle, capturing);
        let r = find(&p, &c, &d, Id::Key { player: 0, act: Act::Up }).rect;
        let at = |buf: &[u8], x: i32, y: i32| {
            let o = ((y * 960 + x) * 4) as usize;
            [buf[o], buf[o + 1], buf[o + 2]]
        };
        // 欄位左側靠近邊緣、字之前的位置。
        let (px, py) = (r.x + 5, r.y + r.h / 2);
        let (a, b) = (at(&idle, px, py), at(&capturing, px, py));
        assert!(b[0] > b[2] + 8, "重綁中的欄位偏琥珀色：{b:?}");
        assert!(a[2] >= a[0], "閒置的欄位是藍灰色：{a:?}");
        // 鍵盤焦點畫主色外框：閒置時焦點在第一個欄位，上緣外一圈是主色；沒有焦點的欄位沒有。
        let up = find(&p, &c, &d, Id::Key { player: 0, act: Act::Up });
        assert!(up.focused);
        let ring = at(&idle, up.rect.x + up.rect.w / 2, up.rect.y - 1);
        assert!(ring[2] > 200 && ring[2] > ring[0] + 80, "焦點外框是主色：{ring:?}");
        let down = find(&p, &c, &d, Id::Key { player: 0, act: Act::Down });
        assert!(!down.focused);
        let plain = at(&idle, down.rect.x + down.rect.w / 2, down.rect.y - 1);
        assert!(plain[2] < 150, "沒有焦點的欄位沒有外框：{plain:?}");
    }

    #[test]
    fn controls_are_switches_and_segments_not_on_off_words() {
        let (c, d) = (ctl(), Devices::new());
        let p = Panel::new(Page::Game, 960, 1600, false, 1.0);
        let lay = p.layout(&c, &d);
        let cell = |id: Id| lay.cells.iter().find(|x| x.id == Some(id)).unwrap();
        for (i, o) in OPTIONS.iter().enumerate() {
            let w = p.widget_of(cell(Id::Opt(i)), &c);
            match o.kind {
                Kind::Toggle => assert!(matches!(w, Widget::Switch(_)), "{} 布林選項畫成開關", o.id),
                Kind::Size => assert!(matches!(&w, Widget::Seg { labels, .. } if labels.len() == 3), "{} 戰場大小是三段", o.id),
                Kind::Feel => assert!(matches!(&w, Widget::Seg { labels, sel: Some(1) } if labels == &["原版時序", "新"]), "{w:?} 手感預設是新"),
            }
        }
        let dp = Panel::new(Page::Display, 960, 600, false, 1.0);
        let lay = dp.layout(&c, &d);
        let cell = |id: Id| lay.cells.iter().find(|x| x.id == Some(id)).unwrap();
        assert_eq!(dp.widget_of(cell(Id::Tween), &c), Widget::Switch(true));
        assert_eq!(dp.widget_of(cell(Id::Counter), &c), Widget::Switch(false));
        assert_eq!(dp.widget_of(cell(Id::Toolbar), &c), Widget::Switch(true));
        match dp.widget_of(cell(Id::Scale), &c) {
            Widget::Seg { labels, sel } => {
                assert_eq!(labels, vec!["2 倍", "3 倍", "4 倍", "5 倍"]);
                assert_eq!(sel, Some(1), "預設 3 倍");
            }
            w => panic!("倍率應該是分段按鈕：{w:?}"),
        }
    }

    #[test]
    fn clicking_a_segment_picks_it_directly_and_a_switch_just_toggles() {
        let (mut c, mut d) = (ctl(), Devices::new());
        let mut p = panel(Page::Display);
        let r = find(&p, &c, &d, Id::Scale).rect;
        let refs = ["2 倍", "3 倍", "4 倍", "5 倍"];
        let pm_ = pm(&p.geom);
        let widths = style::seg_widths(&refs, pm_.fh, seg_pad(&pm_));
        let segs = style::seg_rects(Panel::control_rect(&Widget::Seg { labels: vec![], sel: None }, r, &pm_), &widths, 2);
        // 點最右邊的 5 倍：直接到 5，不經過 4。
        let (x, y) = (segs[3].x + segs[3].w / 2, r.y + r.h / 2);
        let out = p.click(x, y, &mut Cx { ctl: &mut c, devs: &mut d });
        assert_eq!(c.settings.scale, 5);
        assert!(out.eff.save && out.eff.resize == crate::controls::Resize::Scale);
        // 點最左邊的 2 倍。
        let (x, y) = (segs[0].x + 3, r.y + r.h / 2);
        p.click(x, y, &mut Cx { ctl: &mut c, devs: &mut d });
        assert_eq!(c.settings.scale, 2);
        // 再點一次已選的段：換到下一個值（行為同一般點擊）。
        p.click(x, y, &mut Cx { ctl: &mut c, devs: &mut d });
        assert_eq!(c.settings.scale, 3);
        // 開關：點任何位置都切換。
        let t = find(&p, &c, &d, Id::Counter).rect;
        p.click(t.x + 2, t.y + t.h / 2, &mut Cx { ctl: &mut c, devs: &mut d });
        assert!(c.settings.counter);
    }

    #[test]
    fn hover_and_press_change_only_the_target_control() {
        let (c, d) = (ctl(), devs2());
        let mut p = Panel::new(Page::Game, 960, 1600, false, 1.0);
        let id = Id::Opt(crate::options::INVINCIBLE_INDEX);
        let r = find(&p, &c, &d, id).rect;
        let idle = p.render(&c, &d);
        let v0 = p.version();
        p.hover_at(r.x + r.w / 2, r.y + r.h / 2, &c, &d);
        assert!(p.hovering(id) && p.version() > v0);
        let hover = p.render(&c, &d);
        assert_ne!(idle, hover, "移過時變亮");
        let v1 = p.version();
        p.hover_at(r.x + r.w / 2 + 1, r.y + r.h / 2, &c, &d);
        assert_eq!(p.version(), v1, "還在同一個目標上，不重畫");
        p.press(r.x + 2, r.y + 2);
        let down = p.render(&c, &d);
        assert_ne!(hover, down, "按下時變深");
        p.clear_hover();
        assert!(!p.hovering(id));
        // 移到沒有控制項的地方：沒有 hover。
        p.hover_at(2, 2, &c, &d);
        assert!(!p.hovering(id));
    }

    #[test]
    fn tabs_use_the_primary_color_for_the_selected_page() {
        let (c, d) = (ctl(), Devices::new());
        let p = panel(Page::Keyboard);
        let lay = p.layout(&c, &d);
        let img = p.render(&c, &d);
        let at = |x: i32, y: i32| {
            let o = (((lay.card.y + y) * 960 + lay.card.x + x) * 4) as usize;
            [img[o], img[o + 1], img[o + 2]]
        };
        let sel = lay.tabs.iter().find(|(pg, _)| *pg == Page::Keyboard).unwrap().1;
        let other = lay.tabs.iter().find(|(pg, _)| *pg == Page::Pad).unwrap().1;
        let s = at(sel.x + 4, sel.y + sel.h / 2);
        let o = at(other.x + 4, other.y + other.h / 2);
        assert!(s[2] > 200 && s[2] > o[2] + 100, "選中的頁籤是主色：{s:?} {o:?}");
    }

    /// 存圖（`RS2_SHOTS` 指定目錄才存）：滑鼠移過、按下、重綁等待與捲動的狀態，疊在假的遊戲畫面上。
    #[test]
    fn ui_shots_panel_states() {
        let Some(dir) = std::env::var("RS2_SHOTS").ok().map(std::path::PathBuf::from) else { return };
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (1280, 800);
        // 假的遊戲畫面：橘紅的漸層加幾條亮線。
        let backdrop = || {
            let mut v = vec![0u8; (w * h * 4) as usize];
            for y in 0..h {
                for x in 0..w {
                    let o = ((y * w + x) * 4) as usize;
                    let t = y as f32 / h as f32;
                    let stripe = if (x / 40 + y / 40) % 2 == 0 { 18.0 } else { 0.0 };
                    v[o..o + 4].copy_from_slice(&[(230.0 - 120.0 * t + stripe) as u8, (110.0 - 70.0 * t) as u8, (30.0 + 20.0 * t) as u8, 255]);
                }
            }
            v
        };
        let compose = |layer: &[u8]| {
            let mut bg = backdrop();
            for (d, s) in bg.chunks_exact_mut(4).zip(layer.chunks_exact(4)) {
                style::over(d, [s[0], s[1], s[2], s[3]], 1.0);
            }
            bg
        };
        let (mut c, mut d) = (ctl(), devs2());
        // 遊戲頁：滑鼠移到「無敵」的開關，再按下「手感」的分段按鈕。
        let mut p = Panel::new(Page::Game, w, h, false, 1.0);
        let inv = find(&p, &c, &d, Id::Opt(crate::options::INVINCIBLE_INDEX)).rect;
        p.hover_at(inv.x + inv.w - 10, inv.y + inv.h / 2, &c, &d);
        let a = compose(&p.render(&c, &d));
        std::fs::write(dir.join("panel_game_hover_switch.png"), rs2_gfx::png::encode_rgba(w as usize, h as usize, &a)).unwrap();
        let feel = find(&p, &c, &d, Id::Opt(FEEL_INDEX)).rect;
        p.hover_at(feel.x + 8, feel.y + feel.h / 2, &c, &d);
        p.press(feel.x + 8, feel.y + feel.h / 2);
        c.settings.set_opt_value(crate::options::INVINCIBLE_INDEX, 1);
        let a = compose(&p.render(&c, &d));
        std::fs::write(dir.join("panel_game_press_segment_switch_on.png"), rs2_gfx::png::encode_rgba(w as usize, h as usize, &a)).unwrap();
        // 鍵盤頁：重綁等待中、訊息列有字。
        let mut k = Panel::new(Page::Keyboard, w, h, false, 1.0);
        click_id(&mut k, &mut c, &mut d, Id::Key { player: 0, act: Act::Fire });
        k.msg = "右 Shift 已綁定第 1 人開火，沒有覆寫。".into();
        let a = compose(&k.render(&c, &d));
        std::fs::write(dir.join("panel_keyboard_capturing.png"), rs2_gfx::png::encode_rgba(w as usize, h as usize, &a)).unwrap();
        // 小視窗：鍵盤頁要捲動，捲到中間。
        let mut s = Panel::new(Page::Keyboard, 640, 400, false, 1.0);
        s.scroll = 90;
        let small = {
            let mut v = vec![40u8; 640 * 400 * 4];
            for (dst, src) in v.chunks_exact_mut(4).zip(s.render(&c, &d).chunks_exact(4)) {
                style::over(dst, [src[0], src[1], src[2], src[3]], 1.0);
            }
            v
        };
        std::fs::write(dir.join("panel_keyboard_small_window_scrolled.png"), rs2_gfx::png::encode_rgba(640, 400, &small)).unwrap();
        // 淡入中途（60 毫秒）。
        let half = compose(&Panel::new(Page::Display, w, h, false, 1.0).render_faded(&c, &d, fade_in(60)));
        std::fs::write(dir.join("panel_fade_in_60ms.png"), rs2_gfx::png::encode_rgba(w as usize, h as usize, &half)).unwrap();
    }
}
