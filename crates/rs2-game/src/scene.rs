//! 開場、過場、選單與片尾共用的場景步驟。
//!
//! 這四個領域的程式都是腳本：一連串有名字的步驟，例如換調色盤、把圖庫項畫進暫存區、
//! 整頁複製到顯示頁、翻頁、等 tick、等任意鍵。這裡給每個步驟一個名字與有型別的參數，
//! 腳本檔（`intro.rs`、`cards.rs`、`menu.rs`、`ending.rs`、`title.rs`）只用這些步驟寫成。
//! 圖庫項的「放在 [`Placement`]，再用某種畫法畫」是 `video.rs` 繪製動作加上位置的組合。
//!
//! 每個步驟寫進繪製參數、繪製命令的種類與順序都和原版相同。
//! `rs2-gfx` 與前端讀這些命令，順序錯了單色收據（`rs2-trace` 的 `tests/mono.rs`）會分歧。
//! `BlitCmd` 的 `cx`、`dx` 欄位只在命令真的讀它時（整頁複製的列數）才設值，見 `video.rs` 檔頭的表。
//!
//! 畫面是雙緩衝：頁 0 在顯示記憶體偏移 0，頁 1 在 8000h。另有兩塊暫存區（A、B）放不會動的背景，
//! 開場與過場把圖庫項先畫進暫存區 B，每幀再整頁複製到要畫的那一頁，字母與飛船疊在背景上。

use crate::draw::DrawCmd;
use crate::game::Core;
pub(crate) use crate::flow::Palette;
use crate::video::{MyCopy, Page, RowCopy, SCRATCH_B};

/// 整頁複製的列數：畫面 200 列。
pub(crate) const SCREEN_ROWS: u16 = 200;

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

    /// 把 `page` 整頁填 0（原版 `sub_1291A`，頁 1）。原版不碰 map mask，進來時已是 0Fh，
    /// 所以 `sub_18878` 不改變結果，只是讓 `VramFill` 的平面範圍在命令串裡看得見。
    pub(crate) fn blank_page(&mut self, page: Page) {
        self.select_all_planes();
        self.draws.push(DrawCmd::VramFill { off: page.offset(), words: 0x0FA0, value: 0 });
    }

    // ---- 圖庫項 ----

    /// `BEG1`／`J` 圖庫項，4 平面覆寫到目前的頁（原版 `sub_19068`）。
    pub(crate) fn draw_beg(&mut self, at: Placement) {
        self.set_blit_params(at.x, at.y, at.image);
        self.overwrite_beg();
    }

    /// `BEG1`／`J` 圖庫項寫進暫存區（原版 `sub_19B40`），由 [`Core::select_scratch`] 決定 A 或 B。
    pub(crate) fn draw_beg_to_scratch(&mut self, at: Placement) {
        self.set_blit_params(at.x, at.y, at.image);
        self.draw_current_beg_to_scratch();
    }

    /// `MY` 圖庫項，4 平面覆寫（原版 `sub_18C0A`）。
    pub(crate) fn draw_my(&mut self, at: Placement) {
        self.set_blit_params(at.x, at.y, at.image);
        self.overwrite_my();
    }

    /// `MY` 圖庫項，反相覆寫（原版 `sub_18C6E`），選單用來標出選中的列。
    pub(crate) fn draw_my_inverted(&mut self, at: Placement) {
        self.set_blit_params(at.x, at.y, at.image);
        self.overwrite_my_inverted();
    }

    /// 過場字樣：先用遮罩 AND（`sub_18D46`）挖出字形，再把平面 OR 上去（`sub_18CE2`）。
    /// 座標有號，進場時字在畫面外（負值），位元組寬的欄與像素列都取 16 位元補數。
    pub(crate) fn draw_act(&mut self, glyph: Glyph, x: i16, y: i16) {
        self.set_blit_params(x as u16, y as u16, glyph.image());
        self.cut_my(MyCopy::General);
        self.overlay_my(MyCopy::General);
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
        self.copy_scratch_rows(RowCopy::ReplaceShifted, SCREEN_ROWS);
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
}
