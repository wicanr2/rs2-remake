//! 片尾。
//! 逐函式移植自機器碼，查法見 `tools/q.sh`。規格：`docs/spec/002-full-game.md`。
//!
//! 不移植：`sub_142BB`、`sub_143A8` 的 DOS 記憶體配置與釋放（寫入檔名工作區與
//! `1CF5Ch` 至 `1CF6Eh` 的段落，都在 `A0_EXEMPT`）、`sub_1117F`（E 模式為空）、CGA、MGA、純 EGA 分支。

use crate::dseg::a;
use crate::entity::Timetable;
use crate::game::Core;

impl Core {
    /// `sub_141D0`：片尾。`byte_2749B`（Ctrl+Q）離開，`byte_2749E`（Ctrl+R）把幀計數歸零重播。
    pub(crate) async fn s141d0(&mut self) {
        self.s142bb();
        self.s143a8();
        self.s141fc().await;
        loop {
            // loc_141D9
            self.ds.w16(a(0x1CFE2), 0);
            loop {
                // loc_141DF
                self.s19a37();
                self.s1429e().await;
                self.s19a88();
                self.s1429e().await;
                if self.ds.r8(a(0x2749B)) == 1 {
                    return;
                }
                if self.ds.r8(a(0x2749E)) == 1 {
                    break;
                }
            }
        }
    }

    /// `sub_141FC`：載入 `SHT5`，`ending_a` 調色盤下顯示第 3 項（地景）並等任意鍵，
    /// 再換 `ending_b` 顯示第 4 項（人物與行星）並等任意鍵。
    pub(crate) async fn s141fc(&mut self) {
        self.s142bb();
        self.s143a8();
        self.s1af10();
        self.ds.w16(a(0x2685C), 0x8000);
        self.s1152d();
        self.s143ea();
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26882), 0);
        self.ds.w16(a(0x26888), 3);
        self.s19adc();
        self.s19a88();
        self.s18819();
        self.s14273();
        self.ds.w8(a(0x27422), 0);
        while self.ds.r8(a(0x27422)) == 0 {
            self.poll_tick().await;
        }
        self.ds.w8(a(0x27422), 0);
        self.s14419();
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26882), 0);
        self.ds.w16(a(0x26888), 4);
        self.s19adc();
        self.s19a88();
        self.s18819();
        self.s14273();
        self.ds.w8(a(0x27422), 0);
        while self.ds.r8(a(0x27422)) == 0 {
            self.poll_tick().await;
        }
        self.ds.w8(a(0x27422), 0);
    }

    /// `sub_14273`：把暫存區 B 整頁複製到目前的頁，來源與位移都歸零。
    pub(crate) fn s14273(&mut self) {
        self.ds.w16(a(0x2685C), 0x8000);
        self.ds.w16(a(0x26864), 0);
        self.ds.w16(a(0x26866), 0);
        self.ds.w16(a(0x26880), 0);
        self.ds.w16(a(0x26868), 0);
        self.ds.w16(a(0x26882), 0);
        self.r.cx = 0xC8;
        self.s19d08();
    }

    /// `sub_1429E`：片尾的一幀。時間表生成 END 字樣，重畫背景，更新實體與子彈，幀計數加 1，等 1 tick。
    pub(crate) async fn s1429e(&mut self) {
        self.spawn_scheduled(Timetable::Ending);
        self.s14273();
        self.update_enemies();
        self.draw_enemies();
        self.draw_enemy_overlays();
        let v = self.ds.r16(a(0x1CFE2)).wrapping_add(1);
        self.ds.w16(a(0x1CFE2), v);
        self.s1117f();
        self.r.cx = 1;
        self.s11097().await;
    }

    /// `sub_142BB`：DOS 記憶體配置（片尾用的 10 個緩衝區段落），不移植。
    pub(crate) fn s142bb(&mut self) {}

    /// `sub_143A8`：釋放 `sub_142BB` 配置的記憶體，不移植。
    pub(crate) fn s143a8(&mut self) {}

    /// `sub_143EA`：`ending_a` 調色盤。
    pub(crate) fn s143ea(&mut self) {
        self.r.bx = 0;
        self.r.dx = 0x0E54;
        self.s14448();
        self.r.dx = 0x0E54;
        self.r.bx = 0x30;
        self.s14448();
    }

    /// `sub_14419`：`ending_b` 調色盤。
    pub(crate) fn s14419(&mut self) {
        self.r.bx = 0;
        self.r.dx = 0x0E95;
        self.s14448();
        self.r.dx = 0x0E95;
        self.r.bx = 0x30;
        self.s14448();
    }

    /// `sub_14448`：INT 10h AX=1012h，從 DAC 暫存器 `bx` 起寫 16 個，表在 `ds:dx`。
    pub(crate) fn s14448(&mut self) {
        self.r.set_ah(0x10);
        self.r.set_al(0x12);
        self.r.cx = 0x10;
        self.set_dac(self.r.bx as u8, self.r.dx);
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
        assert!(matches!(exit, Some(Exit::Quit) | Some(Exit::End) | Some(Exit::Clear)), "Q 應離開片尾");
    }
}
