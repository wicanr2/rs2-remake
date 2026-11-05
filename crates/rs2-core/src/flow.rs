//! 關卡流程：各關共用的狀態與步驟。
//!
//! 每一關有自己的入口與每幀函式（`stage1.rs` 到 `stage4.rs`），呼叫順序各關不同，不合併成同一個主迴圈。
//! 這裡放各關共用的步驟：進關卡時的重設、Esc 暫停、熱鍵、過關連爆、離開，以及流程狀態 [`Flow`]。
//!
//! | 檔案 | 內容 |
//! |---|---|
//! | `flow/clock.rs` | 計時中斷與等待 |
//! | `flow/keyboard.rs` | 鍵盤中斷：開火旗標、秘技、搖桿與音效開關 |
//! | `flow/hud.rs` | 血條、幀計數、完整性檢查 |
//! | `flow/palette.rs` | 調色盤 |

mod clock;
mod hud;
mod keyboard;
mod palette;

pub(crate) use clock::Clock;
pub(crate) use keyboard::{Cheat, Controls, FLAG_OFF, FLAG_ON, FireFlags};

use crate::boss_scroll::BossLayers;
use crate::game::{Core, Exit, Req};
use crate::mirror::Key;
use crate::stage1::Backdrop as Stage1Backdrop;
use crate::stage2::Backdrop as Stage2Backdrop;
use crate::stage3::MapScroll;
use crate::stage4::Tunnel;

/// 每幀等的 tick 數：INT 8 每秒 18.2 次，所以遊戲每秒更新 9.1 次。
pub(crate) const FRAME_TICKS: u16 = 2;
/// 過關連爆放幾輪（第 21 輪起結束）。
const CLEAR_BURST_ROUNDS: u8 = 0x14;
/// 過關連爆每輪特效鏈的起點往左退幾欄。
const CLEAR_BURST_CHAIN_STEP: u16 = 3;

/// 關卡字母（`byte_1CFDB`）。第三關頭目段改成小寫 `c`。
pub(crate) const STAGE_1: u8 = b'A';
pub(crate) const STAGE_2: u8 = b'B';
pub(crate) const STAGE_3: u8 = b'C';
pub(crate) const STAGE_3_BOSS: u8 = b'c';
pub(crate) const STAGE_4: u8 = b'D';

/// 過關旗標（`byte_1CFAD`）：`N` 進行中，`F` 已過關（主迴圈改播連爆，玩家邏輯與頭目背景停止）。
pub(crate) const STAGE_RUNNING: u8 = b'N';
pub(crate) const STAGE_CLEARED: u8 = b'F';

/// `word_1B02` 的起始值與下限：第一關每輪減 1，減到 D2h 為止。
const TAMPER_START: u16 = 0x10E;
const TAMPER_FLOOR: u16 = 0xD2;

/// 關卡流程的狀態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Flow {
    /// `byte_1CFDB`：目前的關卡字母。
    pub stage: u8,
    /// `byte_1CFAD`：過關旗標。
    pub clear: u8,
    /// `byte_1CFAE`：過關連爆已放的輪數。
    pub bursts: u8,
    /// `byte_1CFAA`：映像裡是 `N`，完整性檢查通過時寫 `Y`（`Core::integrity_check`）。第二關幀 260 的中頭目合成讀它。
    pub integrity_mark: u8,
    /// `word_1B02`：進關卡時設 10Eh，第一關每輪減 1 到 D2h。沒有人讀它的值，只在自己的函式裡比較。
    pub tamper_countdown: u16,
    pub clock: Clock,
    pub controls: Controls,
}

/// 各關背景的狀態。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Scenes {
    pub stage1: Stage1Backdrop,
    pub stage2: Stage2Backdrop,
    pub map: MapScroll,
    pub tunnel: Tunnel,
    pub boss: BossLayers,
}

/// 過關連爆裡，特效鏈的起點往左退的時機。第一、三、四關先退再比較輪數；第二關先比較，沒結束才退。
/// 結束的那一輪起點差 3 欄，特效池的資料段比對看得到。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChainShift {
    BeforeLimit,
    AfterLimit,
}

impl Flow {
    pub(crate) fn is_cleared(&self) -> bool {
        self.clear == STAGE_CLEARED
    }
}

impl Core {
    /// 進關卡的重設（原版 `sub_10E33`）：清地形格、地圖位置、幀計數、三個池，放開捲動與熱鍵，玩家回起點。
    pub(crate) fn reset_stage(&mut self) {
        self.clear_terrain_grid();
        self.scenes.map.left_col = 0;
        self.scenes.map.top_row = 0;
        self.reset_stage_frame();
        self.flow.tamper_countdown = TAMPER_START;
        self.en.release_all();
        self.eb.release_all();
        self.pb.release_all();
        for key in [Key::Down, Key::Left, Key::Right, Key::Up, Key::RightShift] {
            self.release_key(key);
        }
        self.place_players_for_stage();
        self.scenes.map.window = 0;
        self.pb.seekers_in_flight = 0;
        self.flow.clear = STAGE_RUNNING;
        self.flow.bursts = 0;
        for key in [Key::Ctrl, Key::R, Key::D, Key::W, Key::C] {
            self.release_key(key);
        }
    }

    /// 第一關每輪開頭減 1（原版 `sub_10FEA` 前半），減到下限為止。
    ///
    /// 原版這個函式後半在標題旗標（`byte_1CFAC`）是 `N` 時轉入標題迴圈。進關卡之前標題旗標已經是 `Y`，
    /// 這條路走不到。
    pub(crate) fn count_down_tamper_timer(&mut self) {
        if self.flow.tamper_countdown >= TAMPER_FLOOR {
            self.flow.tamper_countdown = self.flow.tamper_countdown.wrapping_sub(1);
        }
    }

    /// Esc 暫停（原版 `sub_10ECB`）：按住時把狀態改成 2，輪詢到它不再是 2（下一次按下 INT 9 寫回 1；
    /// 放開碼也是 2，所以放開不會結束暫停），再清 0 並清最近的按鍵。
    pub(crate) async fn pause_if_requested(&mut self) {
        if !self.key_down(Key::Esc) {
            return;
        }
        self.mark_key_released(Key::Esc);
        while self.key_released(Key::Esc) {
            self.suspend(Req::Poll { paused: true }).await;
        }
        self.release_key(Key::Esc);
        self.clear_last_key();
    }

    /// 熱鍵與存活（原版 `sub_10D99` 接 `sub_10D8D`），回傳要離開關卡的原因。
    ///
    /// | 按鍵 | 效果 |
    /// |---|---|
    /// | Ctrl+R | 展示旗標清成一般模式，離開 `End` |
    /// | Ctrl+Q | `Quit`，優先於 Ctrl+R |
    /// | Ctrl+D | 重播音樂，不離開（取消同一輪的 Ctrl+R） |
    /// | Ctrl+C | `Clear` |
    /// | 展示中按任意鍵 | `End` |
    ///
    /// 兩位玩家都不在場時一律 `End`，蓋過其他原因。Alt+W 與 Ctrl+W 是傾印畫面（`sub_1BDF7`），重製不移植，
    /// Alt+W 只清按鍵狀態。
    pub(crate) fn poll_hotkeys(&mut self) -> Option<Exit> {
        if self.key_down(Key::Alt) && self.key_down(Key::W) {
            self.release_key(Key::W);
            self.release_key(Key::Alt);
        }
        let exit = self.read_hotkeys();
        if self.any_player_alive() { exit } else { Some(Exit::End) }
    }

    fn read_hotkeys(&mut self) -> Option<Exit> {
        if !self.key_down(Key::Ctrl) {
            return (self.demo_playing() && self.last_key() != 0).then_some(Exit::End);
        }
        let mut exit = None;
        if self.key_down(Key::R) {
            self.set_demo_mode(false);
            exit = Some(Exit::End);
        }
        if self.key_down(Key::Q) {
            return Some(Exit::Quit);
        }
        if self.key_down(Key::D) {
            self.start_music();
            return None;
        }
        if self.key_down(Key::C) {
            exit = Some(Exit::Clear);
        }
        exit
    }

    /// 過關連爆的一輪：過關旗標成立後每輪放一排爆炸，放滿 [`CLEAR_BURST_ROUNDS`] 輪回傳 `Clear`。
    /// 爆炸字母看繪製頁：第二頁（`word_26872` 非 0）大寫，第一頁小寫（`spawn_stage_clear_burst`）。
    pub(crate) fn clear_burst_round(&mut self, order: ChainShift) -> Option<Exit> {
        self.flow.bursts = self.flow.bursts.wrapping_add(1);
        if order == ChainShift::BeforeLimit {
            self.fx.shift_chain_left(CLEAR_BURST_CHAIN_STEP);
        }
        if self.flow.bursts > CLEAR_BURST_ROUNDS {
            return Some(Exit::Clear);
        }
        if order == ChainShift::AfterLimit {
            self.fx.shift_chain_left(CLEAR_BURST_CHAIN_STEP);
        }
        let page = self.draw_page();
        self.spawn_stage_clear_burst(page);
        None
    }

    /// 開始放關卡音樂：音效開關鍵生效，開始播放。
    pub(crate) fn start_stage_music(&mut self) {
        self.start_music();
        self.set_sound_key_enabled(true);
    }

    /// 離開關卡：音效開關鍵失效，停止音樂。
    pub(crate) fn end_stage(&mut self, exit: Exit) -> Exit {
        self.set_sound_key_enabled(false);
        self.stop_music();
        exit
    }

    /// 第二、三、四關一幀的收尾：清兩個碰撞格網、暫停檢查、頁複製（MGA）、等待、幀計數加 1、取樣。
    /// 第一關的順序不同（幀計數先加，取樣在等待之前），自己寫。
    pub(crate) async fn close_frame(&mut self, sample: u32) {
        self.clear_grid1();
        self.clear_grid2();
        self.pause_if_requested().await;
        self.end_of_frame_copy();
        self.await_timer_ticks(FRAME_TICKS).await;
        self.advance_stage_frame();
        self.suspend(Req::Sample(sample)).await;
    }

    /// 第二、三、四關每幀共用的後半：敵彈、玩家彈、兩位玩家的繪製、敵人疊圖、特效，接著畫幀計數。
    /// 順序：先更新敵彈與玩家彈，再畫兩位玩家（第一關的玩家 1 更新與繪製分在敵人與子彈前後，不適用）。
    pub(crate) fn step_shots_and_draw_players(&mut self) {
        self.draw_enemies();
        self.update_enemy_bullets();
        self.update_player_bullets();
        self.draw_player(0);
        self.draw_player(1);
        self.draw_enemy_overlays();
        self.update_effects();
        self.draw_frame_counter();
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{Core, Exit, Fixes, Start};
    use crate::mirror::Key;

    fn core() -> Core {
        let image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
        Core::new(&image, &[], Start::NewGame { difficulty: None }, Fixes::NONE)
    }

    #[test]
    fn nothing_pressed_and_one_player_alive_keeps_the_stage_going() {
        let mut c = core();
        assert_eq!(c.poll_hotkeys(), None);
    }

    #[test]
    fn no_player_alive_ends_the_stage_over_every_other_reason() {
        let mut c = core();
        c.set_players_present(0);
        assert_eq!(c.poll_hotkeys(), Some(Exit::End));
        c.note_key_down(Key::Ctrl.scancode() as u8);
        c.note_key_down(Key::C.scancode() as u8);
        assert_eq!(c.poll_hotkeys(), Some(Exit::End), "Ctrl+C 也被蓋過");
    }

    #[test]
    fn ctrl_combinations_pick_their_exit_in_the_original_order() {
        let mut c = core();
        c.note_key_down(Key::Ctrl.scancode() as u8);
        c.note_key_down(Key::C.scancode() as u8);
        assert_eq!(c.poll_hotkeys(), Some(Exit::Clear));
        c.note_key_down(Key::R.scancode() as u8);
        assert_eq!(c.poll_hotkeys(), Some(Exit::Clear), "Ctrl+C 後判斷，蓋過 Ctrl+R");
        c.note_key_down(Key::Q.scancode() as u8);
        assert_eq!(c.poll_hotkeys(), Some(Exit::Quit), "Ctrl+Q 立刻返回");
        c.release_key(Key::Q);
        c.release_key(Key::C);
        assert_eq!(c.poll_hotkeys(), Some(Exit::End), "只剩 Ctrl+R");
    }

    #[test]
    fn ctrl_r_leaves_demo_mode() {
        let mut c = core();
        c.set_demo_playing(true);
        c.note_key_down(Key::Ctrl.scancode() as u8);
        c.note_key_down(Key::R.scancode() as u8);
        assert_eq!(c.poll_hotkeys(), Some(Exit::End));
        assert!(!c.demo_playing());
    }

    #[test]
    fn ctrl_d_restarts_the_music_and_cancels_a_ctrl_r_of_the_same_round() {
        let mut c = core();
        c.note_key_down(Key::Ctrl.scancode() as u8);
        c.note_key_down(Key::R.scancode() as u8);
        c.note_key_down(Key::D.scancode() as u8);
        c.enable_music();
        c.audio.clear();
        assert_eq!(c.poll_hotkeys(), None);
        assert_eq!(c.audio.len(), 1, "音樂重播");
    }

    #[test]
    fn any_key_ends_a_demo_but_only_without_ctrl() {
        let mut c = core();
        c.set_demo_playing(true);
        assert_eq!(c.poll_hotkeys(), None);
        c.set_last_key(b'x');
        assert_eq!(c.poll_hotkeys(), Some(Exit::End));
        let mut c = core();
        c.set_last_key(b'x');
        assert_eq!(c.poll_hotkeys(), None, "一般模式按鍵不離開");
    }

    #[test]
    fn alt_w_only_clears_the_two_keys() {
        let mut c = core();
        c.note_key_down(Key::Alt.scancode() as u8);
        c.note_key_down(Key::W.scancode() as u8);
        assert_eq!(c.poll_hotkeys(), None);
        assert!(!c.key_down(Key::W) && !c.key_down(Key::Alt));
    }

    #[test]
    fn the_burst_ends_after_twenty_one_rounds_and_the_chain_shift_order_differs_by_stage() {
        use super::ChainShift::{AfterLimit, BeforeLimit};
        for (order, shifted_at_the_end) in [(BeforeLimit, true), (AfterLimit, false)] {
            let mut c = core();
            c.fx.reset_chain();
            let start = c.fx.chain_base_x;
            for round in 1..=0x14u8 {
                assert_eq!(c.clear_burst_round(order), None, "第 {round} 輪");
            }
            let before_last = c.fx.chain_base_x;
            assert_eq!(before_last, start.wrapping_sub(3 * 0x14));
            assert_eq!(c.clear_burst_round(order), Some(Exit::Clear));
            let moved = before_last.wrapping_sub(c.fx.chain_base_x);
            assert_eq!(moved == 3, shifted_at_the_end, "{order:?}：結束那一輪是否退 3 欄");
        }
    }

    #[test]
    fn reset_stage_clears_the_flow_state_and_releases_held_keys() {
        let mut c = core();
        c.flow.clear = super::STAGE_CLEARED;
        c.flow.bursts = 9;
        c.scenes.map.window = 7;
        c.scenes.map.left_col = 8;
        c.scenes.map.top_row = 9;
        c.note_key_down(Key::Ctrl.scancode() as u8);
        c.note_key_down(Key::Up.scancode() as u8);
        c.note_key_down(Key::Esc.scancode() as u8);
        c.reset_stage();
        assert_eq!((c.flow.clear, c.flow.bursts), (super::STAGE_RUNNING, 0));
        assert_eq!((c.scenes.map.window, c.scenes.map.left_col, c.scenes.map.top_row), (0, 0, 0));
        assert_eq!(c.flow.tamper_countdown, 0x10E);
        assert_eq!(c.stage_frame(), 0);
        assert!(!c.key_down(Key::Ctrl) && !c.key_down(Key::Up), "放開捲動與熱鍵");
        assert!(c.key_down(Key::Esc), "Esc 不在重設的鍵裡");
    }

    #[test]
    fn the_tamper_countdown_stops_at_its_floor() {
        let mut c = core();
        c.flow.tamper_countdown = 0xD3;
        c.count_down_tamper_timer();
        assert_eq!(c.flow.tamper_countdown, 0xD2);
        c.count_down_tamper_timer();
        assert_eq!(c.flow.tamper_countdown, 0xD1, "大於等於下限才減，所以會到 D1h");
        c.count_down_tamper_timer();
        assert_eq!(c.flow.tamper_countdown, 0xD1);
    }
}
