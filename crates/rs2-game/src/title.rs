//! 標題迴圈：開場動畫、示範輪播、選單，以及選好之後依序玩四關到片尾。
//!
//! 原版 `sub_11556`。程式結構：
//!
//! ```text
//! 標題迴圈
//!   ├ 開場動畫（intro.rs）
//!   │    ├ 放完、再等 10 x 4 個 tick 都沒有按鍵 → 示範輪播（第一關、第二關各一輪），回標題
//!   │    ├ 按 C → 選單（menu.rs），回開場動畫
//!   │    └ 按其他鍵 → 依序玩四關，每關先放過場卡（cards.rs），最後片尾（ending.rs）
//!   └ 任何一關離開碼是 E（玩家全滅、Ctrl+R、示範被按鍵打斷）→ 回標題；Q → 離開
//! ```
//!
//! 狀態只有選單的三個欄位與示範輪播計數，在 [`Title`]，由 `mirror.rs` 同步。
//! 步驟用 `scene.rs` 的名字寫成。
//!
//! 不移植的呼叫：
//! - `sub_12B49`、`sub_12BCF`：DOS 記憶體配置與釋放，檔名工作區與配置段落的寫入都在 `A0_EXEMPT`。
//! - `sub_1117F`：`byte_1CF74` 為 `E` 時直接 `retn`，只有 MGA 分支（`video.rs` 的 `end_of_frame_copy`
//!   照原版順序呼叫，接收端依顯示模式處理）。
//! - CGA、MGA、純 EGA 分支：`byte_1CF74` 固定 `E`、`byte_1CF75` 固定 `N`。
//!
//! `11619h` 原本是一個跳向 `retn` 的 `jmp`，破解組的補丁與密碼檢查通過後把它改成兩個 NOP，
//! 所以 `sub_12438` 之後接著 `1161Bh`。`1161Bh` 讀 `word_1D3B0`（密碼檢查未通過的旗標），
//! 重製裡沒有人寫它，恆為 0。

use crate::cards::CampaignStage;
use crate::draw::{BegGroup, MixFile};
use crate::game::{Core, Exit};
use crate::intro::IntroEnd;
use crate::menu::Menu;
use crate::player::PLAYER1_ACTIVE;
use crate::video::{Page, SCRATCH_B};

/// 標題領域的狀態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Title {
    pub menu: Menu,
    /// 示範輪播（原版 `word_1D146`），見 [`Title::next_demo`]。
    pub demo_round: u16,
}

/// 示範輪播的下一步。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DemoTurn {
    /// 放這一關的示範錄影，放完呼叫 [`Title::finish_demo`]。
    Play(CampaignStage),
    /// 輪完了，回標題。
    BackToTitle,
    /// 計數不在 1 到 4 之內：已經重設為 1，再看一次。
    Retry,
}

impl Title {
    /// 看輪播計數決定下一步：1 放第一關，2 放第二關，3 或 4 重設為 1 並回標題，其他值重設為 1 再看一次。
    /// 第三、四關沒有示範（`116DFh` 起的第三關示範在 `jmp` 之後，不可達）。
    fn next_demo(&mut self) -> DemoTurn {
        match self.demo_round {
            1 => DemoTurn::Play(CampaignStage::One),
            2 => DemoTurn::Play(CampaignStage::Two),
            3 | 4 => {
                self.demo_round = 1;
                DemoTurn::BackToTitle
            }
            _ => {
                self.demo_round = 1;
                DemoTurn::Retry
            }
        }
    }

    /// 一關示範放完，輪播計數加 1。
    fn finish_demo(&mut self) {
        self.demo_round = self.demo_round.wrapping_add(1);
    }
}

impl Core {
    /// 標題迴圈（原版 `sub_11556`）。遊戲從這裡開始，回來就是程式結束，不論離開碼是什麼。
    /// 原版離開時 `dx` 是殘值，所以沒有離開碼可回傳，呼叫端（`run_entry`）一律當作 Quit。
    pub(crate) async fn run_title(&mut self) {
        self.mark_title_entered();
        'title: loop {
            self.request_stage_music();
            self.load_mix(MixFile::A);
            self.start_music();
            self.show_page(Page::Zero);
            self.draw_on(Page::Zero);
            // 原版這裡是 DOS 記憶體配置與釋放，不移植。
            self.show_page(Page::Zero);
            self.load_beg_bank(BegGroup::Beg1);
            'attract: loop {
                self.clear_last_key();
                self.clear_scratch_b();
                self.clear_both_pages();
                self.select_scratch(SCRATCH_B);
                if self.play_intro().await == IntroEnd::TimedOut {
                    self.set_demo_mode(true);
                    self.stop_music();
                    self.set_sound_key_enabled(false);
                    if self.play_demos().await == Some(Exit::Quit) {
                        return;
                    }
                    continue 'title;
                }
                self.stop_music();
                self.set_sound_key_enabled(false);
                if self.last_key() == b'C' {
                    self.run_menu().await;
                    continue 'attract;
                }
                self.clear_both_pages();
                self.set_demo_mode(false);
                self.apply_role();
                if self.password_rejected() {
                    return;
                }
                for stage in CampaignStage::ALL {
                    match self.play_stage(stage).await {
                        Exit::Quit => return,
                        Exit::End => continue 'title,
                        Exit::Clear => {}
                    }
                }
                self.run_ending().await;
                return;
            }
        }
    }

    /// 示範輪播（原版 `sub_11685`）：依 [`Title::demo_round`] 輪流放第一關、第二關的示範錄影，
    /// 有按鍵就停。每輪開頭把活動玩家設成只有玩家 1。回傳最後一關離開時的離開碼，一關都沒放就是 `None`
    /// （原版 `dx` 此時是調色盤函式留下的位址，不會是離開碼）。
    async fn play_demos(&mut self) -> Option<Exit> {
        let mut last_exit = None;
        loop {
            self.set_active_players(PLAYER1_ACTIVE);
            if self.key_pressed() {
                return last_exit;
            }
            match self.title.next_demo() {
                DemoTurn::Play(stage) => {
                    self.reset_stage();
                    last_exit = Some(self.play_stage(stage).await);
                    self.title.finish_demo();
                }
                DemoTurn::BackToTitle => return last_exit,
                DemoTurn::Retry => {}
            }
        }
    }

    /// 一關：過場卡，再跑關卡主函式。第二關起，過場卡之後先把檔案提示的磁碟號設成 `'2'`
    /// （`byte_274EC`，換磁片提示用，重製沒有人讀）。
    async fn play_stage(&mut self, stage: CampaignStage) -> Exit {
        self.play_stage_card(stage).await;
        if stage != CampaignStage::One {
            self.set_disk_hint(b'2');
        }
        match stage {
            CampaignStage::One => self.play_stage1().await,
            CampaignStage::Two => self.play_stage2().await,
            CampaignStage::Three => self.play_stage3().await,
            CampaignStage::Four => self.play_stage4().await,
        }
    }
}

#[cfg(test)]
mod rotation {
    use super::*;

    #[test]
    fn demos_alternate_stage1_stage2_then_back_to_title() {
        // 計數初值（映像）是 0：先重設為 1 再看一次，然後第一關、第二關，輪完回標題並重設為 1。
        let mut t = Title::default();
        let mut turns = Vec::new();
        for _ in 0..6 {
            let turn = t.next_demo();
            if let DemoTurn::Play(_) = turn {
                t.finish_demo();
            }
            turns.push(turn);
            if turn == DemoTurn::BackToTitle {
                break;
            }
        }
        assert_eq!(
            turns,
            [DemoTurn::Retry, DemoTurn::Play(CampaignStage::One), DemoTurn::Play(CampaignStage::Two), DemoTurn::BackToTitle]
        );
        assert_eq!(t.demo_round, 1, "回標題時重設為 1，下次從第一關起");
    }

    #[test]
    fn round_4_and_larger_values_are_reset_to_1() {
        let mut t = Title { demo_round: 4, ..Title::default() };
        assert_eq!(t.next_demo(), DemoTurn::BackToTitle);
        assert_eq!(t.demo_round, 1);
        let mut t = Title { demo_round: 0x1234, ..Title::default() };
        assert_eq!(t.next_demo(), DemoTurn::Retry);
        assert_eq!(t.demo_round, 1);
        assert_eq!(t.next_demo(), DemoTurn::Play(CampaignStage::One));
    }
}

/// 冒煙測試：標題流程在無輸入與少量輸入下的等待序列，和機器碼的呼叫順序一致。
/// 需要原版檔，由 `tools/rust.sh test -p rs2-game smoke -- --ignored --nocapture` 跑。
/// 這不取代 `rs2-trace` 的全流程對拍（規格 002 B2）。
#[cfg(test)]
mod smoke {
    use crate::game::{Entry, Fixes, Frame, Game, GameFiles, Input, State};
    use crate::tables::Tables;

    fn load() -> Game {
        let dir = std::path::PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh 跑"));
        let exe = std::fs::read(dir.join("S.EXE")).unwrap();
        let image = rs2_formats::unpack_s_exe(&exe).unwrap();
        let dm1 = std::fs::read(dir.join("DM1.LCK")).unwrap();
        let dm2 = std::fs::read(dir.join("DM2.LCK")).unwrap();
        let state = State::power_on(&Tables::parse(&image).unwrap());
        let files = GameFiles { image, dm1, dm2, ..GameFiles::default() };
        Game::from_state(files, state, Entry::Title, Fixes::NONE)
    }

    /// 開場動畫的等待序列：`earth_slides_in` 結尾 1 tick 加 `ship_approaches` 的 15 次（含時間到才有的最後一次）。
    fn intro_first() -> Vec<u16> {
        let mut v = vec![1];
        v.extend([5, 5, 5, 4, 4, 3, 3, 2, 1, 1, 1, 1, 4, 0x14, 3]);
        v
    }

    /// 跑到等待序列長度達 `n` 再多跑 `extra` 步，`inputs(waits_so_far, step)` 給該步的掃描碼。
    fn drive(g: &mut Game, n: usize, mut inputs: impl FnMut(usize, usize) -> Vec<u8>) -> Vec<u16> {
        let mut waits: Vec<u16> = Vec::new();
        for step in 0..200_000 {
            let sc = inputs(waits.len(), step);
            let mut w2 = Vec::new();
            let f: Frame = g.step_full(&Input { scancodes: &sc }, &mut |_g, _pt| {}, &mut |_g, cx| w2.push(cx));
            waits.extend(w2);
            if waits.len() >= n || f.exit.is_some() {
                break;
            }
        }
        waits
    }

    #[test]
    #[ignore]
    fn no_input_reaches_demo_stage1() {
        let mut g = load();
        let mut want = intro_first();
        want.push(0x0F);
        want.extend([1, 0x0F]);
        want.extend([6, 3, 2, 2, 1, 8, 2, 2, 1, 0x0E]);
        want.extend([4; 10]);
        want.extend([1; 54]);
        want.push(2);
        let got = drive(&mut g, want.len(), |_, _| Vec::new());
        assert_eq!(&got[..want.len()], &want[..], "等待序列不同");
    }

    #[test]
    #[ignore]
    fn key_goes_through_stage1_card() {
        let mut g = load();
        let mut sent = false;
        let mut want = intro_first();
        want.extend([1; 54]);
        want.push(2);
        let got = drive(&mut g, want.len(), |n, step| {
            if step == 0 {
                vec![0x1E]
            } else if n == 16 && !sent {
                sent = true;
                vec![0x1E]
            } else {
                Vec::new()
            }
        });
        assert_eq!(&got[..want.len()], &want[..], "等待序列不同");
    }

    #[test]
    #[ignore]
    fn menu_changes_difficulty_and_returns() {
        let mut g = load();
        let mut want = intro_first();
        want.extend([1, 7]);
        want.extend([1, 7]);
        want.push(1);
        let mut phase = 0;
        let got = drive(&mut g, want.len(), |n, step| {
            if step == 0 {
                return vec![0x2E];
            }
            match (phase, n) {
                (0, 17) => {
                    phase = 1;
                    vec![0x4D]
                }
                (1, 18) => {
                    phase = 2;
                    vec![0xCD]
                }
                (2, 19) => {
                    phase = 3;
                    vec![0x1C]
                }
                _ => Vec::new(),
            }
        });
        assert_eq!(&got[..want.len()], &want[..], "等待序列不同");
        assert_eq!(g.state().title.menu.level, 2, "難度應從 1 變 2");
    }
}
