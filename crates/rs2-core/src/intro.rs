//! 開場動畫。
//!
//! 標題迴圈（`sub_11556`）先放這段動畫，期間任何一個鍵都會中斷；放完沒有按鍵就進示範。
//! 每一段開始前先清暫存區 B 與兩頁，所有 `BEG1` 圖都先寫進暫存區 B（`word_2685C` 為 8000h），
//! 再用 `sub_19D08` 搬到顯示頁。內容與座標見 `docs/re/12-stages-presentation.md` 第 8.2 節。
//!
//! | 段 | 原版 | 內容 |
//! |---|---|---|
//! | 1 | `sub_11910`、`sub_1199F` | 地球從畫面左緣滑入，太空船接近，結尾灰階閃光 |
//! | 2 | `sub_1175B` | 手臂特寫往右滑 |
//! | 3 | `sub_11806`、`sub_118A2` | 太空船正面逐級放大，`RS-S` 字樣從右飛入 |
//! | 4 | `sub_116F3` | 眼部特寫從畫面下方捲上來，畫出大宇資訊字樣 |
//! | 5 | `sub_117BF` | 製作名單與 SOFTSTAR 標誌對向移動 |
//!
//! 每段內部的迴圈每步都看一次按鍵，有鍵就立刻結束這一段。原版的檢查點全部保留，
//! 因為每個檢查點之後的等待和繪製長度都會出現在收據裡。

use crate::game::Core;
use crate::scene::{Page, Palette, Placement, SCREEN_ROWS, ScratchView};

/// 開場動畫怎麼結束的。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntroEnd {
    /// 有人按鍵。
    Interrupted,
    /// 五段放完、再等 10 x 4 個 tick 都沒有按鍵：標題停留夠久，改放示範。
    TimedOut,
}

/// `BEG1` 圖庫項。
const EYE_CLOSEUP: u16 = 0;
const ARM: u16 = 1;
const EARTH: u16 = 7;
const TITLE_BANNER: u16 = 14;

/// `MY` 圖庫項。
const MY_PUBLISHER: u16 = 0x2F;
const MY_CREDIT_LIST: u16 = 0x30;
const MY_SOFTSTAR: u16 = 0x0F;

/// 製作名單與標誌所在的像素列。
const CREDITS_Y: u16 = 0x98;

/// 太空船正面由小到大：(x, 圖庫項, 畫完等幾個 tick)。畫在同一列，同位置覆寫放大。
/// 最後一級（第 2 項）不在這裡，它畫完接 `RS-S` 字樣。
const SHIP_FRONT_ZOOM: [(u16, u16, u16); 4] = [(0x12, 6, 6), (0x11, 5, 3), (0x10, 4, 2), (0x0E, 3, 2)];
const SHIP_FRONT_Y: u16 = 0x64;
const SHIP_FRONT_LAST: Placement = Placement::at(0x0D, SHIP_FRONT_Y, 2);

/// 太空船接近的一步：先重畫地球背景，依序畫船與噴焰（或行星），畫完等 `wait` 個 tick。
/// 偶數步畫完顯示頁 1，奇數步顯示頁 0。
struct ApproachStep {
    sprites: &'static [Placement],
    wait: u16,
}

const APPROACH: [ApproachStep; 12] = [
    ApproachStep { sprites: &[Placement::at(0x1B, 0x35, 8)], wait: 5 },
    ApproachStep { sprites: &[Placement::at(0x1A, 0x3B, 9), Placement::at(0x1E, 0x37, 0x15)], wait: 5 },
    ApproachStep { sprites: &[Placement::at(0x19, 0x40, 0x0A), Placement::at(0x1E, 0x37, 0x16)], wait: 5 },
    ApproachStep { sprites: &[Placement::at(0x18, 0x45, 0x0B), Placement::at(0x1E, 0x35, 0x17)], wait: 4 },
    ApproachStep { sprites: &[Placement::at(0x16, 0x4C, 0x0C), Placement::at(0x1E, 0x34, 0x18)], wait: 4 },
    ApproachStep { sprites: &[Placement::at(0x13, 0x52, 0x0D), Placement::at(0x1F, 0x33, 0x19)], wait: 3 },
    ApproachStep { sprites: &[Placement::at(0x10, 0x5A, 0x12), Placement::at(0x1F, 0x39, 0x1A)], wait: 3 },
    ApproachStep { sprites: &[Placement::at(0x0A, 0x63, 0x13), Placement::at(0x1F, 0x37, 0x1B)], wait: 2 },
    ApproachStep { sprites: &[Placement::at(3, 0x7A, 0x14), Placement::at(0x1F, 0x37, 0x1C)], wait: 1 },
    // 行星：最後三步只有一張圖，越來越近。
    ApproachStep { sprites: &[Placement::at(0x1D, 0x28, 0x1D)], wait: 1 },
    ApproachStep { sprites: &[Placement::at(0x19, 3, 0x1E)], wait: 1 },
    ApproachStep { sprites: &[Placement::at(0x13, 0, 0x1F)], wait: 1 },
];

impl Core {
    /// 放開場動畫。呼叫前暫存區選擇已設為 B。
    pub(crate) async fn play_intro(&mut self) -> IntroEnd {
        self.earth_slides_in().await;
        self.load_palette(Palette::TitleD);
        if self.key_pressed() {
            return IntroEnd::Interrupted;
        }
        self.clear_scratch();
        self.clear_pages();
        self.arm_slides_in().await;
        if self.key_pressed() {
            return IntroEnd::Interrupted;
        }
        self.clear_scratch();
        self.load_palette(Palette::TitleE);
        self.clear_pages();
        self.ship_front_zooms_in().await;
        if self.key_pressed() {
            return IntroEnd::Interrupted;
        }
        self.clear_scratch();
        self.load_palette(Palette::TitleD);
        self.clear_pages();
        self.eye_scrolls_up();
        if self.key_pressed() {
            return IntroEnd::Interrupted;
        }
        self.credits_meet();
        // 製作名單停留 10 x 4 個 tick，每 4 個 tick 看一次按鍵。
        for _ in 0..10 {
            self.wait_ticks(4).await;
            if self.key_pressed() {
                return IntroEnd::Interrupted;
            }
        }
        IntroEnd::TimedOut
    }

    /// 段 1 前半（`sub_11910`）：`BEG1` 第 7 項（地球，240x200）寫在暫存區 B 的 (8,0)，
    /// 取景的來源欄從 28h 每頁減 1，視窗往左移，地球從畫面左緣滑入。20 輪，每輪兩頁。
    /// 滑完接太空船接近。
    async fn earth_slides_in(&mut self) {
        self.load_palette(Palette::Beg1A);
        self.clear_scratch();
        self.draw_beg_to_scratch(Placement::at(8, 0, EARTH));
        let mut view = ScratchView { src_x: 0x28, ..ScratchView::ORIGIN };
        self.set_scratch_view(view);
        self.show_page(Page::One);
        self.draw_on(Page::One);
        for _ in 0..20 {
            for page in [Page::Zero, Page::One] {
                self.show_page(page);
                self.clear_draw_page();
                self.copy_scratch_rows(SCREEN_ROWS);
                view.src_x = view.src_x.wrapping_sub(1);
                self.set_scratch_view(view);
                self.mirror_scan_lines();
            }
            if self.key_pressed() {
                return;
            }
        }
        // 滑完再補畫一次最後的取景，翻到頁 0。
        self.mirror_scan_lines();
        self.clear_draw_page();
        self.copy_scratch_rows(SCREEN_ROWS);
        self.mirror_scan_lines();
        self.show_page(Page::Zero);
        self.wait_ticks(1).await;
        self.ship_approaches().await;
    }

    /// 段 1 後半（`sub_1199F`）：太空船接近的 12 步，每步先重畫地球背景，雙頁交替。
    /// 步 11 之後切 `beg1_b` 灰階閃光等 20 個 tick，再切回 `beg1_a`。
    async fn ship_approaches(&mut self) {
        for (i, step) in APPROACH.iter().enumerate() {
            self.redraw_earth();
            for sprite in step.sprites {
                self.draw_beg(*sprite);
            }
            self.mirror_scan_lines();
            self.wait_ticks(step.wait).await;
            self.show_page(if i % 2 == 0 { Page::One } else { Page::Zero });
        }
        // 只剩背景，閃光。
        self.redraw_earth();
        self.mirror_scan_lines();
        self.wait_ticks(4).await;
        self.load_palette(Palette::Beg1B);
        self.wait_ticks(0x14).await;
        self.show_page(Page::One);
        self.load_palette(Palette::Beg1A);
        self.wait_ticks(3).await;
        if self.key_pressed() {
            return;
        }
        self.wait_ticks(0x0F).await;
    }

    /// 清目前的頁，再把暫存區 B 的地球整頁複製上去（原版 `sub_11C26`）。取景沿用地球滑完的位置。
    fn redraw_earth(&mut self) {
        self.clear_draw_page();
        self.set_blit_position(0, 0);
        self.copy_scratch_rows(SCREEN_ROWS);
    }

    /// 段 2（`sub_1175B`）：`BEG1` 第 1 項（手臂，224x153）寫在暫存區 B 的 (1,0)，顯示在 y=40，
    /// 水平位移從 -10 每步 -1 共 33 步，先等 1 加 15 個 tick 再開始位移。
    async fn arm_slides_in(&mut self) {
        self.draw_beg_to_scratch(Placement::at(1, 0, ARM));
        let mut view = ScratchView { shift: (-10i16) as u16, y: 0x28, ..ScratchView::ORIGIN };
        self.set_scratch_view(view);
        self.show_page(Page::One);
        self.draw_on(Page::One);
        self.copy_scratch_rows(SCREEN_ROWS);
        self.wait_ticks(1).await;
        self.wait_ticks(0x0F).await;
        for _ in 0..33 {
            self.copy_scratch_rows(SCREEN_ROWS);
            view.shift = view.shift.wrapping_sub(1);
            self.set_scratch_view(view);
            if self.key_pressed() {
                return;
            }
        }
    }

    /// 段 3（`sub_11806`）：`BEG1` 第 6 到 2 項（太空船正面 32x21 到 112x92）依序畫在 y=100，
    /// 前四級畫完各等幾個 tick，最後一級直接接 `RS-S` 字樣。每一級之後看一次按鍵。
    async fn ship_front_zooms_in(&mut self) {
        for (x, image, wait) in SHIP_FRONT_ZOOM {
            self.draw_beg(Placement::at(x, SHIP_FRONT_Y, image));
            self.wait_ticks(wait).await;
            if self.key_pressed() {
                return;
            }
        }
        self.draw_beg(SHIP_FRONT_LAST);
        self.banner_flies_in().await;
        self.wait_ticks(1).await;
        if self.key_pressed() {
            return;
        }
        self.wait_ticks(0x0E).await;
    }

    /// 段 3 尾（`sub_118A2`）：`BEG1` 第 14 項（`RS-S` 字樣，272x57）在 y=30 從 x=40 往左 38 步，
    /// 等 1 加 8 個 tick，再依序疊上第 15、16、17 項（56x57）畫在 x=29，最後的 `S` 變成 `2`。
    async fn banner_flies_in(&mut self) {
        let mut x = 0x28u16;
        for _ in 0..38 {
            self.draw_beg(Placement::at(x, 0x1E, TITLE_BANNER));
            x = x.wrapping_sub(1);
            if self.key_pressed() {
                return;
            }
        }
        self.wait_ticks(1).await;
        self.wait_ticks(8).await;
        self.draw_beg(Placement::at(0x1D, 0x1E, 0x0F));
        self.wait_ticks(2).await;
        if self.key_pressed() {
            return;
        }
        self.draw_beg(Placement::at(0x1D, 0x1E, 0x10));
        self.wait_ticks(2).await;
        self.draw_beg(Placement::at(0x1D, 0x1E, 0x11));
    }

    /// 段 4（`sub_116F3`）：`BEG1` 第 0 項（眼部特寫，256x100）寫在暫存區 B 的 (0,0)，
    /// 顯示起點 y=240，每步 -5 共 40 步，從畫面下方捲到 y=40；之後 `MY` 第 47 項（大宇資訊字樣）畫在 (15,20)。
    fn eye_scrolls_up(&mut self) {
        self.draw_beg_to_scratch(Placement::at(0, 0, EYE_CLOSEUP));
        let mut view = ScratchView { x: 5, y: 0xF0, ..ScratchView::ORIGIN };
        self.set_scratch_view(view);
        self.show_page(Page::One);
        self.draw_on(Page::One);
        for _ in 0..40 {
            self.copy_scratch_rows(SCREEN_ROWS);
            view.y = view.y.wrapping_sub(5);
            self.set_scratch_view(view);
            if self.key_pressed() {
                return;
            }
        }
        self.draw_my(Placement::at(0x0F, 0x14, MY_PUBLISHER));
    }

    /// 段 5（`sub_117BF`）：`MY` 第 48 項（製作名單，2 行文字）在 y=152 從 x=40 每步 -1，
    /// `MY` 第 15 項（SOFTSTAR 標誌）從 x=-15 每步 +1，20 步後在中央相會。沒有等待。
    fn credits_meet(&mut self) {
        let (mut list_x, mut logo_x) = (0x28i16, -15i16);
        for _ in 0..20 {
            self.draw_my(Placement::at(list_x as u16, CREDITS_Y, MY_CREDIT_LIST));
            self.draw_my(Placement::at(logo_x as u16, CREDITS_Y, MY_SOFTSTAR));
            logo_x += 1;
            list_x -= 1;
            if self.key_pressed() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approach_waits_match_the_documented_table() {
        // docs/re/12-stages-presentation.md 第 8.2 節的步驟表：等待依序為 5、5、5、4、4、3、3、2、1、1、1、1。
        assert_eq!(APPROACH.map(|s| s.wait), [5, 5, 5, 4, 4, 3, 3, 2, 1, 1, 1, 1]);
        // 步 0 只有船，步 1 到 8 船加噴焰，步 9 到 11 只有行星。
        assert_eq!(APPROACH.map(|s| s.sprites.len()), [1, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1]);
        // 船的圖庫項 8 到 13 與 18 到 20，噴焰 21 到 28（兩者交替出現），行星 29 到 31。
        let images: Vec<u16> = APPROACH.iter().flat_map(|s| s.sprites.iter().map(|p| p.image)).collect();
        assert_eq!(images, [8, 9, 21, 10, 22, 11, 23, 12, 24, 13, 25, 18, 26, 19, 27, 20, 28, 29, 30, 31]);
    }

    #[test]
    fn ship_front_zoom_matches_the_documented_table() {
        // 同一節：第 6、5、4、3、2 項，x=18、17、16、14、13，前四級畫完各等 6、3、2、2 個 tick。
        assert_eq!(SHIP_FRONT_ZOOM.map(|(x, image, _)| (x, image)), [(18, 6), (17, 5), (16, 4), (14, 3)]);
        assert_eq!(SHIP_FRONT_ZOOM.map(|(_, _, wait)| wait), [6, 3, 2, 2]);
        assert_eq!((SHIP_FRONT_LAST.x, SHIP_FRONT_LAST.image, SHIP_FRONT_LAST.y), (13, 2, 100));
    }
}
