//! 關間過場卡。
//!
//! 每一關開始前，一般模式先畫一張 J 圖等任意鍵，再放 ACT 字樣的動畫；示範模式跳過 J 圖，
//! 直接放字樣動畫。字樣是 `MY` 圖庫的 A、C、T 加關卡數字，每個字用遮罩 AND 加平面 OR 畫上去。
//! 內容與座標見 `docs/re/12-stages-presentation.md` 第 8.4 節。
//!
//! | 關 | 原版 | J 圖 | 字樣動畫 |
//! |---|---|---|---|
//! | 1 | `sub_11C6B`、`sub_11C3C` | `J1` 第 2 項 | A 從左、T 從右進場，C 從上落下彈回，數字 1 從左進場 |
//! | 2 | `sub_11E46`、`sub_11E17` | `J2` 第 1 項 | A、C、T 分三列從左進場，五段換位，數字 2 進場 |
//! | 3 | `sub_121DD`、`sub_121AE`、`sub_127CD` | `J3` 第 0 項，由下往上滑入 | A、C 從上、T 與數字 3 從下，夾向同一列 |
//! | 4 | `sub_12379`、`sub_1234A`、`sub_128AE` | `J4` 第 3 項 | 四個字從左右兩側進場 |
//!
//! 字樣的位置在原版是一組資料段字組（`word_1D148` 到 `word_1D156`），每張卡開始時先寫初值，
//! 之後只有它自己的步驟函式讀寫。這裡改成每張卡一個小結構，每一步回傳要畫的字，不碰資料段。
//! 結構不依賴 `Core`，終點座標可以對文件做單元測試。

use crate::game::Core;
use crate::mirror::SCRATCH_B;
use crate::scene::{Glyph, Page, Palette, Placement, SCREEN_ROWS, ScratchView};

/// 遊戲的四關。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CampaignStage {
    One,
    Two,
    Three,
    Four,
}

impl CampaignStage {
    pub(crate) const ALL: [CampaignStage; 4] = [CampaignStage::One, CampaignStage::Two, CampaignStage::Three, CampaignStage::Four];

    /// 這一關的 J 圖：調色盤與圖庫項。
    const fn picture(self) -> (Palette, u16) {
        match self {
            CampaignStage::One => (Palette::J1, 2),
            CampaignStage::Two => (Palette::J2, 1),
            CampaignStage::Three => (Palette::J3, 0),
            CampaignStage::Four => (Palette::J4, 3),
        }
    }
}

/// 一個字要畫在哪裡：欄是位元組，列是像素，有號，進場時字在畫面外。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Placed {
    pub glyph: Glyph,
    pub x: i16,
    pub y: i16,
}

impl Placed {
    const fn new(glyph: Glyph, x: i16, y: i16) -> Placed {
        Placed { glyph, x, y }
    }
}

/// 字樣每一步移動的單位：C、數字上下走 10 列。
const ROW_STEP: i16 = 10;

// ---- 第一關 ----

/// 第一關：A 在 y=50 往右、T 在 y=50 往左、C 在 x=15 從上落下再彈回，最後數字 1 從左進場。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stage1Letters {
    a_x: i16,
    t_x: i16,
    c_y: i16,
    digit_x: i16,
}

impl Stage1Letters {
    const START: Stage1Letters = Stage1Letters { a_x: -9, t_x: 0x27, c_y: -50, digit_x: 0 };
    const ROW: i16 = 0x32;
    const C_COLUMN: i16 = 0x0F;

    /// 三個字目前的位置，依序 A、T、C。
    fn letters(&self) -> Vec<Placed> {
        vec![
            Placed::new(Glyph::A, self.a_x, Self::ROW),
            Placed::new(Glyph::T, self.t_x, Self::ROW),
            Placed::new(Glyph::C, Self::C_COLUMN, self.c_y),
        ]
    }

    /// `sub_11CFC`：A 向右、T 向左各 1 欄，C 往下落 10 列。
    fn fall(&mut self) -> Vec<Placed> {
        self.a_x += 1;
        self.t_x -= 1;
        self.c_y += ROW_STEP;
        self.letters()
    }

    /// `sub_11D58`：A、T 原位，C 往上彈 10 列。
    fn bounce(&mut self) -> Vec<Placed> {
        self.c_y -= ROW_STEP;
        self.letters()
    }

    /// `sub_11DAC`：數字 1 每步右移 1 欄，先畫，A、T、C 原位重畫在後。
    fn digit_in(&mut self) -> Vec<Placed> {
        self.digit_x += 1;
        let mut draws = vec![Placed::new(Glyph::One, self.digit_x, Self::ROW)];
        draws.extend(self.letters());
        draws
    }
}

// ---- 第二關 ----

/// 第二關：A、C、T 在 y=50、90、130 三列，從左緣外進場，C、T 先後上移，三字再左移並帶出數字 2。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stage2Letters {
    a_x: i16,
    c_x: i16,
    t_x: i16,
    c_y: i16,
    t_y: i16,
    digit_x: i16,
    digit_y: i16,
}

impl Stage2Letters {
    const START: Stage2Letters = Stage2Letters { a_x: -1, c_x: -1, t_x: -1, c_y: 0x5A, t_y: 0x82, digit_x: 0x28, digit_y: 0x32 };
    const A_ROW: i16 = 0x32;

    /// 三個字目前的位置，依序 A、C、T。
    fn letters(&self) -> Vec<Placed> {
        vec![
            Placed::new(Glyph::A, self.a_x, Self::A_ROW),
            Placed::new(Glyph::C, self.c_x, self.c_y),
            Placed::new(Glyph::T, self.t_x, self.t_y),
        ]
    }

    /// 三字都在原位之後，數字 2 接在最後。
    fn letters_and_digit(&self) -> Vec<Placed> {
        let mut draws = self.letters();
        draws.push(Placed::new(Glyph::Two, self.digit_x, self.digit_y));
        draws
    }

    /// `sub_11F58`：三字各右移 1 欄。
    fn all_right(&mut self) -> Vec<Placed> {
        self.a_x += 1;
        self.c_x += 1;
        self.t_x += 1;
        self.letters()
    }

    /// `sub_11FB3`：A 不動，C、T 右移 1 欄。
    fn c_and_t_right(&mut self) -> Vec<Placed> {
        self.c_x += 1;
        self.t_x += 1;
        self.letters()
    }

    /// `sub_1200A`：C 上移 10 列，T 右移 1 欄。
    fn c_up_t_right(&mut self) -> Vec<Placed> {
        self.c_y -= ROW_STEP;
        self.t_x += 1;
        self.letters()
    }

    /// `sub_12062`：T 上移 10 列。
    fn t_up(&mut self) -> Vec<Placed> {
        self.t_y -= ROW_STEP;
        self.letters()
    }

    /// `sub_120B6`：三字左移 1 欄，數字 2 左移 1 欄、下移 10 列。
    fn all_left_digit_down(&mut self) -> Vec<Placed> {
        self.a_x -= 1;
        self.c_x -= 1;
        self.t_x -= 1;
        self.digit_y += ROW_STEP;
        self.digit_x -= 1;
        self.letters_and_digit()
    }

    /// `sub_12132`：同 `sub_120B6`，但數字 2 上移 10 列。
    fn all_left_digit_up(&mut self) -> Vec<Placed> {
        self.a_x -= 1;
        self.c_x -= 1;
        self.t_x -= 1;
        self.digit_y -= ROW_STEP;
        self.digit_x -= 1;
        self.letters_and_digit()
    }
}

// ---- 第三關 ----

/// 第三關：四個字固定在欄 12、16、20、24，A、C 從上，T 與數字 3 從下，26 步後都停在 y=75。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stage3Letters {
    a_y: i16,
    c_y: i16,
    t_y: i16,
    digit_y: i16,
}

impl Stage3Letters {
    const START: Stage3Letters = Stage3Letters { a_y: -133, c_y: -55, t_y: 205, digit_y: 283 };

    /// `sub_1221E`：A、C 往下 8、5 列，T、數字 3 往上 5、8 列。
    fn converge(&mut self) -> Vec<Placed> {
        self.a_y += 8;
        self.c_y += 5;
        self.t_y -= 5;
        self.digit_y -= 8;
        vec![
            Placed::new(Glyph::A, 0x0C, self.a_y),
            Placed::new(Glyph::C, 0x10, self.c_y),
            Placed::new(Glyph::T, 0x14, self.t_y),
            Placed::new(Glyph::Three, 0x18, self.digit_y),
        ]
    }
}

// ---- 第四關 ----

/// 第四關：四個字在 y=80，A、C 從右往左、T 與數字 4 從左往右，26 步後停在欄 8、14、21、27。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stage4Letters {
    a_x: i16,
    c_x: i16,
    t_x: i16,
    digit_x: i16,
}

impl Stage4Letters {
    const START: Stage4Letters = Stage4Letters { a_x: 0x3C, c_x: 0x28, t_x: -5, digit_x: -25 };
    const ROW: i16 = 0x50;

    /// `sub_123BD`：四字在同一列，x 依序 -2、-1、+1、+2。
    fn converge(&mut self) -> Vec<Placed> {
        self.a_x -= 2;
        self.c_x -= 1;
        self.t_x += 1;
        self.digit_x += 2;
        vec![
            Placed::new(Glyph::A, self.a_x, Self::ROW),
            Placed::new(Glyph::C, self.c_x, Self::ROW),
            Placed::new(Glyph::T, self.t_x, Self::ROW),
            Placed::new(Glyph::Four, self.digit_x, Self::ROW),
        ]
    }
}

impl Core {
    /// 這一關開始前的過場卡。
    pub(crate) async fn play_stage_card(&mut self, stage: CampaignStage) {
        match stage {
            CampaignStage::One => {
                self.load_palette(Palette::J1);
                self.show_picture(stage).await;
                let mut card = Stage1Letters::START;
                self.restore_background();
                self.animate_pages(10, || card.fall()).await;
                self.animate_pages(5, || card.bounce()).await;
                self.animate_pages(12, || card.digit_in()).await;
            }
            CampaignStage::Two => {
                self.load_palette(Palette::J2);
                self.show_picture(stage).await;
                let mut card = Stage2Letters::START;
                self.restore_background();
                self.animate_pages(14, || card.all_right()).await;
                self.animate_pages(2, || card.c_and_t_right()).await;
                self.animate_pages(2, || card.c_up_t_right()).await;
                self.animate_pages(4, || card.t_up()).await;
                self.animate_pages(5, || card.all_left_digit_down()).await;
                self.animate_pages(5, || card.all_left_digit_up()).await;
            }
            CampaignStage::Three => {
                self.show_picture(stage).await;
                let mut card = Stage3Letters::START;
                self.animate_pages(13, || card.converge()).await;
            }
            CampaignStage::Four => {
                self.show_picture(stage).await;
                let mut card = Stage4Letters::START;
                self.blank_page(Page::One);
                self.animate_pages(13, || card.converge()).await;
            }
        }
    }

    /// 兩頁交替的字樣動畫：每輪兩次，每次顯示一頁、在另一頁先蓋回背景再畫這一步的字、等 1 個 tick。
    /// 一輪就是兩步，所以字樣每輪走兩步。
    async fn animate_pages(&mut self, rounds: u16, mut step: impl FnMut() -> Vec<Placed>) {
        for _ in 0..rounds {
            for shown in [Page::One, Page::Zero] {
                self.show_page(shown);
                self.draw_on(shown.other());
                self.restore_background();
                for p in step() {
                    self.draw_act(p.glyph, p.x, p.y);
                }
                self.await_timer_ticks(1).await;
                self.mirror_scan_lines();
            }
        }
    }

    /// 這一關的 J 圖（`sub_11C3C`、`sub_11E17`、`sub_121AE`、`sub_1234A`）。
    /// 一般模式畫在暫存區 B 並整頁複製到頁 0，等任意鍵；
    /// 示範模式（`byte_1CFDC` 不是 `N`）只清暫存區 B 並換回 `title_d`，不畫。
    async fn show_picture(&mut self, stage: CampaignStage) {
        if self.in_demo_mode() {
            self.clear_scratch();
            self.load_palette(Palette::TitleD);
            return;
        }
        self.load_picture(stage);
        self.mirror_scan_lines();
        self.show_page(Page::Zero);
        self.draw_on(Page::Zero);
        self.restore_background();
        self.wait_key().await;
    }

    /// 載入這一關的 J 圖庫，換調色盤，第 N 項寫進暫存區 B（`sub_12858`、`12883`、`127CD`、`128AE`）。
    /// 第三關另外用 200 次整頁複製，圖由下往上滑入。
    fn load_picture(&mut self, stage: CampaignStage) {
        match stage {
            CampaignStage::One => self.s1b008(),   // J1.GO
            CampaignStage::Two => self.s1b01d(),   // J2.GO
            CampaignStage::Three => self.s1b032(), // J3.GO
            CampaignStage::Four => self.s1b047(),  // J4.GO
        }
        self.select_scratch(SCRATCH_B);
        self.clear_scratch();
        let (palette, image) = stage.picture();
        self.load_palette(palette);
        self.draw_beg_to_scratch(Placement::at(0, 0, image));
        if stage == CampaignStage::Three {
            self.picture_rises_from_below();
        }
    }

    /// 第三關的 J 圖：顯示起點 y=200，每步 -1 共 200 步，由下往上滑入。直接畫在顯示中的頁 1。
    fn picture_rises_from_below(&mut self) {
        let mut view = ScratchView { y: SCREEN_ROWS, ..ScratchView::ORIGIN };
        self.set_scratch_view(view);
        self.show_page(Page::One);
        self.draw_on(Page::One);
        for _ in 0..SCREEN_ROWS {
            self.copy_rows_to_page_shifted(SCREEN_ROWS);
            view.y = view.y.wrapping_sub(1);
            self.set_scratch_view(view);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<S>(state: &mut S, rounds: u16, mut step: impl FnMut(&mut S) -> Vec<Placed>) -> Vec<Placed> {
        let mut last = Vec::new();
        // 一輪兩步。
        for _ in 0..rounds * 2 {
            last = step(state);
        }
        last
    }

    fn at(draws: &[Placed], glyph: Glyph) -> (i16, i16) {
        let p = draws.iter().find(|p| p.glyph == glyph).unwrap_or_else(|| panic!("沒畫 {glyph:?}"));
        (p.x, p.y)
    }

    #[test]
    fn stage1_letters_end_where_the_document_says() {
        // 文件 8.4：A 從 x=-9 每步 +1 到 x=11，T 從 x=39 每步 -1 到 x=19，皆在 y=50；
        // C 在 x=15 從 y=-50 每步 +10 落到 y=150，再每步 -10 彈回 y=50；數字 1 從 x=0 每步 +1 進場到 x=24。
        // 三段分別是 20 步、10 步、24 步。
        let mut card = Stage1Letters::START;
        let drop = run(&mut card, 10, Stage1Letters::fall);
        assert_eq!((at(&drop, Glyph::A), at(&drop, Glyph::T), at(&drop, Glyph::C)), ((11, 50), (19, 50), (15, 150)));
        let bounce = run(&mut card, 5, Stage1Letters::bounce);
        assert_eq!(at(&bounce, Glyph::C), (15, 50));
        let digit = run(&mut card, 12, Stage1Letters::digit_in);
        assert_eq!(at(&digit, Glyph::One), (24, 50));
        assert_eq!(digit.iter().map(|p| p.glyph).collect::<Vec<_>>(), [Glyph::One, Glyph::A, Glyph::T, Glyph::C], "數字先畫，A、T、C 在後");
    }

    #[test]
    fn stage2_letters_rise_to_one_row_and_spell_act_2() {
        // 文件 8.4 沒有逐段解碼換位的座標。從機器碼的步數推：三列的 C、T 上移到 A 的列（y=50），
        // 三字再左移到 x=7、11、15，各寬 4 欄，剛好連成 ACT；數字 2 先下移 100 列再回到 y=50，停在 x=20。
        let mut card = Stage2Letters::START;
        run(&mut card, 14, Stage2Letters::all_right);
        run(&mut card, 2, Stage2Letters::c_and_t_right);
        let mid = run(&mut card, 2, Stage2Letters::c_up_t_right);
        assert_eq!(at(&mid, Glyph::C).1, 50, "C 上移 4 步到 A 的列");
        let rising = run(&mut card, 4, Stage2Letters::t_up);
        assert_eq!(at(&rising, Glyph::T).1, 50, "T 上移 8 步到 A 的列");
        let down = run(&mut card, 5, Stage2Letters::all_left_digit_down);
        assert_eq!(at(&down, Glyph::Two), (30, 150), "數字 2 左移 10 欄、下移 100 列");
        let up = run(&mut card, 5, Stage2Letters::all_left_digit_up);
        assert_eq!(
            [Glyph::A, Glyph::C, Glyph::T, Glyph::Two].map(|g| at(&up, g)),
            [(7, 50), (11, 50), (15, 50), (20, 50)]
        );
    }

    #[test]
    fn stage3_letters_meet_on_row_75() {
        // 文件 8.4：26 步後都停在 y=75，欄固定 12、16、20、24。
        let mut card = Stage3Letters::START;
        let end = run(&mut card, 13, Stage3Letters::converge);
        assert_eq!(end.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(), [(12, 75), (16, 75), (20, 75), (24, 75)]);
        assert_eq!(end.iter().map(|p| p.glyph).collect::<Vec<_>>(), [Glyph::A, Glyph::C, Glyph::T, Glyph::Three]);
    }

    #[test]
    fn stage4_letters_end_on_columns_8_14_21_27() {
        // 文件 8.4：y=80，26 步後停在 x=8、14、21、27。
        let mut card = Stage4Letters::START;
        let end = run(&mut card, 13, Stage4Letters::converge);
        assert_eq!(end.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(), [(8, 80), (14, 80), (21, 80), (27, 80)]);
    }

    #[test]
    fn every_stage_has_its_own_picture() {
        let pictures = CampaignStage::ALL.map(CampaignStage::picture);
        assert_eq!(pictures, [(Palette::J1, 2), (Palette::J2, 1), (Palette::J3, 0), (Palette::J4, 3)]);
    }
}
