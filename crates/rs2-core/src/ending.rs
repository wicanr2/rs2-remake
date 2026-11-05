//! 片尾（原版 `sub_141D0`）。
//!
//! 先顯示兩張 `SHT5` 圖庫項，各等任意鍵：第 3 項（地景）與第 4 項（人物與行星）。
//! 接著每幀把第 4 項當背景重畫，再由片尾的時間表（`Timetable::Ending`，幀 1、10、1 共三列）
//! 生成 END 三個字母的敵人樣板，字母依各自的移動腳本從右側飛入。
//! 按 Q 離開，按 R 把幀計數歸零重播。這兩個鍵不需要 Ctrl：機器碼只比對按鍵表的 Q 與 R 兩項
//! （`byte_2749B`、`byte_2749E`，掃描碼 10h、13h）。`docs/re/12-stages-presentation.md` 第 4.6 節寫的是
//! Ctrl+Q 與 Ctrl+R，與機器碼和 `docs/re/03-game-logic.md` 的「單按 Q 結束」不一致，以機器碼為準。
//!
//! 片尾和第四關共用敵人池，第四關結束時還有活著的實體會帶進片尾（文件第 4.6 節）。
//!
//! 不移植：`sub_142BB`、`sub_143A8` 的 DOS 記憶體配置與釋放（寫入檔名工作區與 `1CF5Ch` 至 `1CF6Eh` 的段落，
//! 都在 `A0_EXEMPT`）、`sub_1117F`（E 模式為空，單色模式由 `mirror_scan_lines` 的接收端處理）、CGA、MGA、純 EGA 分支。

use crate::entity::Timetable;
use crate::game::Core;
use crate::mirror::{Key, SCRATCH_B};
use crate::scene::{Page, Palette, Placement};

/// `SHT5` 圖庫項：片尾的兩張背景。
const LANDSCAPE: u16 = 3;
const PEOPLE_AND_PLANET: u16 = 4;

impl Core {
    /// 片尾。按 Q 回來；按 R 重播時幀計數歸零，敵人池和資料段其他部分不重設。
    pub(crate) async fn run_ending(&mut self) {
        self.show_ending_pictures().await;
        loop {
            self.reset_stage_frame();
            loop {
                self.show_page(Page::Zero);
                self.ending_frame().await;
                self.show_page(Page::One);
                self.ending_frame().await;
                if self.key_down(Key::Q) {
                    return;
                }
                if self.key_down(Key::R) {
                    break;
                }
            }
        }
    }

    /// 載入 `SHT5`，`ending_a` 調色盤下顯示第 3 項並等任意鍵，再換 `ending_b` 顯示第 4 項並等任意鍵
    /// （原版 `sub_141FC`）。兩張都先畫進暫存區 B，再整頁複製到頁 1，翻過去。
    async fn show_ending_pictures(&mut self) {
        self.load_ending_images();
        self.select_scratch(SCRATCH_B);
        self.clear_scratch();
        self.show_ending_picture(Palette::EndingA, LANDSCAPE).await;
        self.show_ending_picture(Palette::EndingB, PEOPLE_AND_PLANET).await;
    }

    async fn show_ending_picture(&mut self, palette: Palette, image: u16) {
        self.load_palette(palette);
        self.draw_sht_to_scratch(Placement::at(0, 0, image));
        self.show_page(Page::One);
        self.draw_on(Page::One);
        self.restore_background();
        self.wait_key().await;
    }

    /// 片尾的一幀（原版 `sub_1429E`）：時間表生成字母、蓋回背景、更新並畫實體、幀計數加 1、等 1 個 tick。
    async fn ending_frame(&mut self) {
        self.spawn_scheduled(Timetable::Ending);
        self.restore_background();
        self.update_enemies();
        self.draw_enemies();
        self.draw_enemy_overlays();
        self.advance_stage_frame();
        self.mirror_scan_lines();
        self.await_timer_ticks(1).await;
    }
}

/// 冒煙測試：片尾的兩次按鍵等待、每幀 1 tick 的迴圈、Q 離開。需要原版檔，
/// 由 `tools/rust.sh test -p rs2-core ending_smoke -- --ignored --nocapture` 跑。
#[cfg(test)]
mod ending_smoke {
    use crate::game::{Entry, Exit, Fixes, Game, GameFiles, Input};

    #[test]
    #[ignore]
    fn two_key_pages_then_loop_then_quit() {
        let dir = std::path::PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh 跑"));
        let exe = std::fs::read(dir.join("S.EXE")).unwrap();
        let image = rs2_formats::unpack_s_exe(&exe).unwrap();
        let mut snap = vec![0u8; 0x10000];
        let src = &image[rs2_formats::DSEG_IMAGE_OFFSET..];
        let n = src.len().min(0x10000);
        snap[..n].copy_from_slice(&src[..n]);
        let files = GameFiles { image, ..GameFiles::default() };
        let mut g = Game::from_snapshot(files, &snap, Entry::Ending, Fixes::NONE);
        let mut waits: Vec<u16> = Vec::new();
        let mut exit = None;
        let mut keys = 0;
        for step in 0..10_000 {
            // 第一頁、第二頁各一次任意鍵，再跑 10 個等待之後按 Q。
            let sc: Vec<u8> = if step == 3 || step == 6 {
                keys += 1;
                vec![0x1E]
            } else if waits.len() == 10 {
                vec![0x10]
            } else {
                Vec::new()
            };
            let mut w2 = Vec::new();
            let f = g.step_full(&Input { scancodes: &sc }, &mut |_g, _pt| {}, &mut |_g, cx| w2.push(cx));
            waits.extend(w2);
            if f.exit.is_some() {
                exit = f.exit;
                break;
            }
        }
        assert_eq!(keys, 2);
        assert!(waits.len() >= 10 && waits.iter().all(|&c| c == 1), "迴圈每幀等 1 tick：{waits:?}");
        assert!(matches!(exit, Some(Exit::Quit)), "單按 Q 離開，離開碼是 Quit：{exit:?}");
    }
}
