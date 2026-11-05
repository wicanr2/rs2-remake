//! 標題選單：LEVEL（難度）與 CTRL（誰玩）兩個選項。
//!
//! 原版 `sub_12588`：上下鍵在兩列之間切換，選中 LEVEL 列時左右鍵改難度，選中 CTRL 列時左右鍵改角色，
//! Enter 或空白鍵離開。每次重畫都把難度對應的兩個加成寫進資料段（`sub_12754`），
//! 離開選單之後、開始遊戲之前才把角色對應的活動玩家位元寫進資料段（`sub_12438` 呼叫 `sub_127A5`）。
//! 選單的狀態在標題結構裡，離開選單、放示範再回來仍保留。
//! 內容見 `docs/re/03-game-logic.md`「選單」與 `docs/re/12-stages-presentation.md` 第 8.5 節。

use crate::game::Core;
use crate::mirror::{Key, PLAYER1_ACTIVE, PLAYER2_ACTIVE};
use crate::scene::{Page, Palette, Placement};

/// 選單的三個欄位，照原版的字組寬度保存（`mirror.rs` 的 `Title` 對應表）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Menu {
    /// 選中的列：0 是 LEVEL，1 是 CTRL。
    pub row: u16,
    /// 難度：0 EASY、1 NORMAL、2 HARD、3 HARDEST。
    pub level: u16,
    /// 角色：0 ROLE 1、1 ROLE 2、2 2 PLAYERS。
    pub role: u16,
}

/// 難度對敵人的加成，不改速度、數量或生成時間。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Difficulty {
    /// 加在敵人耐久門檻上（原版 `word_1E6B2`）。
    pub hp_bonus: u16,
    /// 加在開火亂數上（原版 `word_1E6B4`）。
    pub fire_bonus: u16,
}

/// 方向鍵的左右兩個按下狀態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Arrows {
    pub left: bool,
    pub right: bool,
}

const ROW_LEVEL: u16 = 0;
const ROW_ROLE: u16 = 1;
const LEVEL_COUNT: u16 = 4;
const ROLE_COUNT: u16 = 3;

impl Menu {
    /// 上下鍵：在 LEVEL 與 CTRL 之間切換。不是這兩列的值（只有 poke 會造成）不動。
    pub(crate) fn toggle_row(&mut self) {
        match self.row {
            ROW_LEVEL => self.row = ROW_ROLE,
            ROW_ROLE => self.row = ROW_LEVEL,
            _ => {}
        }
    }

    /// 左右鍵改選中列的值，在範圍內環繞。有改回傳 `true`，呼叫端要重畫。
    /// 兩個鍵同時按著右鍵優先（原版先判斷右）。按住不放會隨每次重畫（8 個 tick）持續改。
    pub(crate) fn adjust_selected(&mut self, arrows: Arrows) -> bool {
        match self.row {
            ROW_LEVEL => Self::step(&mut self.level, LEVEL_COUNT, arrows),
            ROW_ROLE => Self::step(&mut self.role, ROLE_COUNT, arrows),
            _ => false,
        }
    }

    /// 右鍵加 1，大於等於上限就回 0；左鍵減 1，已經是 0 就回上限。比較是無號的。
    fn step(value: &mut u16, count: u16, arrows: Arrows) -> bool {
        if arrows.right {
            *value = if *value < count - 1 { value.wrapping_add(1) } else { 0 };
            true
        } else if arrows.left {
            *value = if *value > 0 { value.wrapping_sub(1) } else { count - 1 };
            true
        } else {
            false
        }
    }

    /// 目前難度的加成。難度不是 0 到 3（只有 poke 會造成）時原版不寫，回傳 `None`。
    pub(crate) fn difficulty(&self) -> Option<Difficulty> {
        let (hp_bonus, fire_bonus) = match self.level {
            0 => (0, 0),
            1 => (0, 2),
            2 => (1, 2),
            3 => (1, 3),
            _ => return None,
        };
        Some(Difficulty { hp_bonus, fire_bonus })
    }

    /// 目前角色對應的活動玩家位元。角色不是 0 到 2 時原版不寫，回傳 `None`。
    pub(crate) fn active_players(&self) -> Option<u8> {
        match self.role {
            0 => Some(PLAYER1_ACTIVE),
            1 => Some(PLAYER2_ACTIVE),
            2 => Some(PLAYER1_ACTIVE | PLAYER2_ACTIVE),
            _ => None,
        }
    }
}

/// `MY` 圖庫項：兩個列標籤、四個難度字樣、三個角色字樣、四張隨難度變的頭像。
const LEVEL_LABEL: u16 = 0x4D;
const CTRL_LABEL: u16 = 0x52;
const LEVEL_NAMES: u16 = 0x4E;
const ROLE_NAMES: u16 = 0x53;
const FACES: u16 = 0x49;

impl Core {
    /// 選單（原版 `sub_12588`）。`menu` 調色盤，全黑背景，畫在頁 0 直接顯示。
    /// Enter 或空白鍵離開，並清掉最近一個鍵。原版的等待迴圈不讓出時間，重製每圈過一個 tick。
    pub(crate) async fn run_menu(&mut self) {
        self.load_palette(Palette::Menu);
        self.clear_pages();
        self.show_page(Page::Zero);
        self.draw_on(Page::Zero);
        loop {
            self.draw_menu().await;
            loop {
                if self.key_down(Key::Up) || self.key_down(Key::Down) {
                    self.release_key(Key::Up);
                    self.release_key(Key::Down);
                    self.title.menu.toggle_row();
                    break;
                }
                let arrows = Arrows { left: self.key_down(Key::Left), right: self.key_down(Key::Right) };
                if self.title.menu.adjust_selected(arrows) {
                    break;
                }
                if self.key_down(Key::Space) || self.key_down(Key::Enter) {
                    self.clear_last_key();
                    return;
                }
                self.poll_tick().await;
            }
        }
    }

    /// 畫選單（原版 `sub_126B8`）：選中的列標籤反相，沒選中的覆寫；右邊是目前的值，左邊是頭像。
    /// 畫完寫難度加成，等 1 加 7 個 tick。
    async fn draw_menu(&mut self) {
        let menu = self.title.menu;
        for (row, y, image) in [(ROW_LEVEL, 0x1E, LEVEL_LABEL), (ROW_ROLE, 0x28, CTRL_LABEL)] {
            let label = Placement::at(0x14, y, image);
            if menu.row == row {
                self.draw_my_inverted(label);
            } else {
                self.draw_my(label);
            }
        }
        self.draw_my(Placement::at(0x1C, 0x1E, LEVEL_NAMES.wrapping_add(menu.level)));
        self.draw_my(Placement::at(0x1C, 0x28, ROLE_NAMES.wrapping_add(menu.role)));
        self.draw_my(Placement::at(3, 0x1E, FACES.wrapping_add(menu.level)));
        self.apply_difficulty();
        self.wait_ticks(1).await;
        self.wait_ticks(7).await;
    }

    /// 難度加成寫進資料段（原版 `sub_12754`），敵人領域從那裡讀。
    fn apply_difficulty(&mut self) {
        if let Some(d) = self.title.menu.difficulty() {
            self.set_difficulty_bonuses(d.hp_bonus, d.fire_bonus);
        }
    }

    /// 選好的角色寫成活動玩家位元（原版 `sub_127A5`，由 `sub_12438` 呼叫），玩家領域從那裡讀。
    /// `sub_12438` 後面還有一段舊選單畫面，沒有任何入口，是死碼。
    pub(crate) fn apply_role(&mut self) {
        if let Some(mask) = self.title.menu.active_players() {
            self.set_active_players(mask);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RIGHT: Arrows = Arrows { left: false, right: true };
    const LEFT: Arrows = Arrows { left: true, right: false };

    fn menu(row: u16, level: u16, role: u16) -> Menu {
        Menu { row, level, role }
    }

    #[test]
    fn up_and_down_toggle_between_the_two_rows_only() {
        let mut m = menu(0, 0, 0);
        m.toggle_row();
        assert_eq!(m.row, 1);
        m.toggle_row();
        assert_eq!(m.row, 0);
        let mut odd = menu(5, 0, 0);
        odd.toggle_row();
        assert_eq!(odd.row, 5, "poke 進來的列號不動");
    }

    #[test]
    fn level_wraps_in_both_directions() {
        let mut m = menu(0, 0, 0);
        assert!(m.adjust_selected(LEFT));
        assert_eq!(m.level, 3, "0 往左回到 HARDEST");
        assert!(m.adjust_selected(RIGHT));
        assert_eq!(m.level, 0, "3 往右回到 EASY");
        for want in [1, 2, 3] {
            assert!(m.adjust_selected(RIGHT));
            assert_eq!(m.level, want);
        }
        assert_eq!(m.role, 0, "選中 LEVEL 列時不動角色");
    }

    #[test]
    fn role_wraps_in_both_directions() {
        let mut m = menu(1, 2, 0);
        assert!(m.adjust_selected(LEFT));
        assert_eq!(m.role, 2);
        assert!(m.adjust_selected(RIGHT));
        assert_eq!(m.role, 0);
        assert!(m.adjust_selected(RIGHT));
        assert!(m.adjust_selected(RIGHT));
        assert_eq!(m.role, 2);
        assert_eq!(m.level, 2, "選中 CTRL 列時不動難度");
    }

    #[test]
    fn right_wins_when_both_arrows_are_down_and_nothing_changes_without_arrows() {
        let mut m = menu(0, 1, 0);
        assert!(m.adjust_selected(Arrows { left: true, right: true }));
        assert_eq!(m.level, 2);
        let before = m;
        assert!(!m.adjust_selected(Arrows::default()));
        assert_eq!(m, before);
    }

    #[test]
    fn out_of_range_values_follow_the_unsigned_compares_of_the_original() {
        // 右鍵：值大於等於上限一律回 0（`cmp 3; jnb`）。左鍵：值為 0 才回上限，其他減 1（`cmp 0; jbe`），
        // 所以 poke 進來的大值往左只是減 1。
        let mut m = menu(0, 9, 0);
        assert!(m.adjust_selected(RIGHT));
        assert_eq!(m.level, 0);
        let mut m = menu(0, 9, 0);
        assert!(m.adjust_selected(LEFT));
        assert_eq!(m.level, 8);
        assert_eq!(m.difficulty(), None, "難度 8 不在表內，原版不寫加成");
        let mut odd_row = menu(7, 1, 1);
        assert!(!odd_row.adjust_selected(RIGHT), "不是兩列之一，左右鍵沒作用");
    }

    #[test]
    fn difficulty_table_matches_the_document() {
        // docs/re/03-game-logic.md「選單」：(word_1E6B4 開火, word_1E6B2 血量) = (0,0)、(2,0)、(2,1)、(3,1)。
        let got: Vec<(u16, u16)> = (0..4).map(|l| menu(0, l, 0).difficulty().map(|d| (d.fire_bonus, d.hp_bonus)).unwrap()).collect();
        assert_eq!(got, [(0, 0), (2, 0), (2, 1), (3, 1)]);
    }

    #[test]
    fn roles_map_to_the_active_player_bits() {
        // 同一節：ROLE 1 = 01h、ROLE 2 = 10h、2 PLAYERS = 11h。
        let got: Vec<Option<u8>> = (0..4).map(|r| menu(0, 0, r).active_players()).collect();
        assert_eq!(got, [Some(0x01), Some(0x10), Some(0x11), None]);
    }
}
