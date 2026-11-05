//! 開場、過場、選單與片尾共用的場景步驟。
//!
//! 這四個領域的程式都是腳本：一連串有名字的步驟，例如換調色盤、把圖庫項畫進暫存區、
//! 整頁複製到顯示頁、翻頁、等 tick、等任意鍵。原版每個步驟是一個以位址命名的小函式，
//! 前後用暫存器接力。這裡給每個步驟一個名字與有型別的參數，腳本檔（`intro.rs`、`cards.rs`、
//! `menu.rs`、`ending.rs`、`title.rs`）只用這些步驟寫成。
//!
//! 每個步驟寫進資料段的繪製參數、繪製命令的種類與順序都和原版相同。
//! `rs2-gfx` 與前端讀這些命令，順序錯了單色收據（`rs2-trace` 的 `tests/mono.rs`）會分歧。
//! 唯一的例外是 `BlitCmd` 的 `cx`、`dx` 兩個欄位：原版帶著前面程式留下的暫存器殘值，這裡只在命令真的讀它時
//! （整頁複製的列數）才設值，見 `callconv.rs`。
//!
//! 畫面是雙緩衝：頁 0 在顯示記憶體偏移 0，頁 1 在 8000h。另有兩塊暫存區（A、B）放不會動的背景，
//! 開場與過場把圖庫項先畫進暫存區 B，每幀再整頁複製到要畫的那一頁，字母與飛船疊在背景上。

use crate::draw::DrawCmd;
use crate::game::Core;
pub(crate) use crate::mirror::Palette;
use crate::mirror::SCRATCH_B;

/// 整頁複製的列數：畫面 200 列。
pub(crate) const SCREEN_ROWS: u16 = 200;

/// 雙緩衝的兩頁。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    /// 顯示記憶體偏移 0。
    Zero,
    /// 顯示記憶體偏移 8000h。
    One,
}

impl Page {
    /// 頁在顯示記憶體裡的偏移。
    pub(crate) const fn offset(self) -> u16 {
        match self {
            Page::Zero => 0,
            Page::One => 0x8000,
        }
    }

    /// 另一頁。
    pub(crate) const fn other(self) -> Page {
        match self {
            Page::Zero => Page::One,
            Page::One => Page::Zero,
        }
    }
}

/// 圖庫項的畫法位置：`x` 是位元組（8 像素），`y` 是像素列，`image` 是圖庫索引。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Placement {
    pub x: u16,
    pub y: u16,
    pub image: u16,
}

impl Placement {
    pub(crate) const fn at(x: u16, y: u16, image: u16) -> Placement {
        Placement { x, y, image }
    }
}

/// 過場字樣的字形：`MY` 圖庫第 61、62、63 項（32x32 的 A、C、T）與第 64 到 67 項（數字 1 到 4）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Glyph {
    A,
    C,
    T,
    One,
    Two,
    Three,
    Four,
}

impl Glyph {
    pub(crate) const fn image(self) -> u16 {
        match self {
            Glyph::A => 0x3D,
            Glyph::C => 0x3E,
            Glyph::T => 0x3F,
            Glyph::One => 0x40,
            Glyph::Two => 0x41,
            Glyph::Three => 0x42,
            Glyph::Four => 0x43,
        }
    }
}

/// 暫存區整頁複製到顯示頁時的取景：來源欄、來源列、水平位移，與畫在目的頁的欄和像素列。
/// 開場的捲動、滑入都是每幀改一個值再複製。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScratchView {
    pub src_x: u16,
    pub src_y: u16,
    pub shift: u16,
    pub x: u16,
    pub y: u16,
}

impl ScratchView {
    /// 原封不動複製：全部從 0 起。
    pub(crate) const ORIGIN: ScratchView = ScratchView { src_x: 0, src_y: 0, shift: 0, x: 0, y: 0 };
}

impl Core {
    // ---- 頁面 ----

    /// 顯示 `page`，改畫另一頁（原版 `sub_19A37`、`sub_19A88`）。
    /// 切到頁 0 時一併同步音效開關（`sub_187C3`）。
    pub(crate) fn show_page(&mut self, page: Page) {
        match page {
            Page::Zero => self.s19a37(),
            Page::One => self.s19a88(),
        }
    }

    /// 之後的繪製畫在 `page`，不改顯示的頁（原版 `sub_18812`、`sub_18819`）。
    pub(crate) fn draw_on(&mut self, page: Page) {
        match page {
            Page::Zero => self.s18812(),
            Page::One => self.s18819(),
        }
    }

    /// 清兩頁（原版 `sub_18894`）。
    pub(crate) fn clear_pages(&mut self) {
        self.s18894();
    }

    /// 清暫存區 B（原版 `sub_1152D`）。
    pub(crate) fn clear_scratch(&mut self) {
        self.s1152d();
    }

    /// 把 `page` 整頁填 0（原版 `sub_1291A`，頁 1）。原版不碰 map mask，進來時已是 0Fh，
    /// 所以 `sub_18878` 不改變結果，只是讓 `VramFill` 的平面範圍在命令串裡看得見。
    pub(crate) fn blank_page(&mut self, page: Page) {
        self.s18878();
        self.draws.push(DrawCmd::VramFill { off: page.offset(), words: 0x0FA0, value: 0 });
    }

    /// 一幀畫完、翻頁之前的 Hercules 掃描線複製（原版 `sub_1117F`）。只有單色模式有動作，
    /// 位置見 `docs/re/14-display-modes.md`，順序錯了單色收據會分歧。
    pub(crate) fn mirror_scan_lines(&mut self) {
        self.s1117f();
    }

    // ---- 調色盤 ----

    // ---- 圖庫項 ----

    /// `BEG1`／`J` 圖庫項，4 平面覆寫到目前的頁（原版 `sub_19068`）。
    pub(crate) fn draw_beg(&mut self, at: Placement) {
        self.set_blit_params(at.x, at.y, at.image);
        self.s19068();
    }

    /// `BEG1`／`J` 圖庫項寫進暫存區（原版 `sub_19B40`），由 [`Core::select_scratch`] 決定 A 或 B。
    pub(crate) fn draw_beg_to_scratch(&mut self, at: Placement) {
        self.set_blit_params(at.x, at.y, at.image);
        self.s19b40();
    }

    /// `SHT`／`BH` 圖庫項寫進暫存區（原版 `sub_19ADC`）。
    pub(crate) fn draw_sht_to_scratch(&mut self, at: Placement) {
        self.set_blit_params(at.x, at.y, at.image);
        self.s19adc();
    }

    /// `MY` 圖庫項，4 平面覆寫（原版 `sub_18C0A`）。
    pub(crate) fn draw_my(&mut self, at: Placement) {
        self.set_blit_params(at.x, at.y, at.image);
        self.s18c0a();
    }

    /// `MY` 圖庫項，反相覆寫（原版 `sub_18C6E`），選單用來標出選中的列。
    pub(crate) fn draw_my_inverted(&mut self, at: Placement) {
        self.set_blit_params(at.x, at.y, at.image);
        self.s18c6e();
    }

    /// 過場字樣：先用遮罩 AND（`sub_18D46`）挖出字形，再把平面 OR 上去（`sub_18CE2`）。
    /// 座標有號，進場時字在畫面外（負值），位元組寬的欄與像素列都取 16 位元補數。
    pub(crate) fn draw_act(&mut self, glyph: Glyph, x: i16, y: i16) {
        self.set_blit_params(x as u16, y as u16, glyph.image());
        self.s18d46();
        self.s18ce2();
    }

    // ---- 暫存區 ----

    /// 寫入暫存區複製的取景。
    pub(crate) fn set_scratch_view(&mut self, view: ScratchView) {
        self.set_blit_source_shifted(view.src_x, view.src_y, view.shift);
        self.set_blit_position(view.x, view.y);
    }

    /// 把暫存區 B 整頁複製到目前的頁，來源、位移與目的位置都歸零（原版 `sub_1282D`、`sub_14273`）。
    /// 每幀先用它把背景蓋回去，字母與飛船再疊上去。
    pub(crate) fn restore_background(&mut self) {
        self.select_scratch(SCRATCH_B);
        self.set_scratch_view(ScratchView::ORIGIN);
        self.copy_rows_to_page_shifted(SCREEN_ROWS);
    }

    // ---- 等待 ----

    /// 等任意鍵：清掉最近一個鍵，輪詢到有鍵，再清一次。
    pub(crate) async fn wait_key(&mut self) {
        self.clear_last_key();
        while !self.key_pressed() {
            self.poll_tick().await;
        }
        self.clear_last_key();
    }

    // ---- 載入與音樂 ----

    /// 載入標題用的 `A.MIX`（原版 `sub_1AA38`）。
    pub(crate) fn load_title_music(&mut self) {
        self.s1aa38();
    }

    /// 載入開場動畫的 `BEG1.BD1` 至 `BD4`（原版 `sub_1BC4D`）。
    pub(crate) fn load_intro_images(&mut self) {
        self.s1bc4d();
    }

    /// 載入片尾的 `SHT5.GO`（原版 `sub_1AF10`）。
    pub(crate) fn load_ending_images(&mut self) {
        self.s1af10();
    }

}
