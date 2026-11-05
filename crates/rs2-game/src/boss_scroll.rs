//! 頭目合成圖的捲動：第一、二、三關的頭目段，背景是一張寫進暫存區的大圖，依八向腳本逐幀移動後複製到顯示頁。
//! 原版是 `sub_15906` 一族與腳本直譯器 `sub_15BB0`、`sub_15BCB`。
//!
//! 每一層合成圖有自己的位置與腳本讀取位置（[`BossLayer`]）。第二關有三層，第一、三關只用第 0 層。
//! 原版把「這一層的位置」複製進共用的工作變數（`word_1DB5B`、`word_1DB5D`、`word_1DB6B`），呼叫直譯器，
//! 再複製回來；繪製函式讀工作變數。工作變數每次使用前都剛寫過，重製改成 [`BossLayer::step`] 直接更新那一層
//! （`DS_EXEMPT` 的 `0F9A..0F9E`、`0FAB..0FAC`）。

use crate::draw::Pool;
use crate::game::Core;
use crate::video::RowCopy;

/// 腳本的結束位元組：讀到它不前進、不移動，每幀停在最後一個位置。
pub(crate) const SCRIPT_END: u8 = 0x0F;
/// 八向指令一步的距離：垂直方向 10 像素列（每格 10 列），水平方向 1 位元組。
const STEP_ROWS: u16 = 0x0A;
const STEP_COLS: u16 = 1;

/// 頭目合成圖的捲動腳本（唯讀表格）。腳本是八向指令的位元組串，結束碼見 [`SCRIPT_END`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BossScript {
    /// 第一關頭目，714 步，起點 x = −20、y = 200。
    Stage1 = 0,
    /// 第二關中頭目第一層，148 步。
    Stage2MidA = 1,
    /// 第二關中頭目第二層，189 步。
    Stage2MidB = 2,
    /// 第二關頭目，704 步，以 1Fh 結束。
    Stage2Boss = 3,
    /// 第三關頭目，758 步。
    Stage3 = 4,
}

/// 一層頭目合成圖。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BossLayer {
    /// 畫到顯示頁的起始像素列（會是負數，位元組補數）。
    pub y: u16,
    /// 水平位置，單位是位元組；繪製時以 `-x` 當水平位移。
    pub x: u16,
    /// 腳本讀取位置，資料段位移，指向下一個要讀的位元組。
    pub script: u16,
}

/// 三層合成圖。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BossLayers {
    pub layers: [BossLayer; 3],
}

/// 八向指令：1 上、2 右上、3 右、4 右下、5 下、6 左下、7 左、8 左上，其餘不動。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Heading {
    Hold,
    Move { up: bool, down: bool, left: bool, right: bool },
}

impl Heading {
    fn from_code(code: u8) -> Heading {
        let (up, down, left, right) = match code {
            1 => (true, false, false, false),
            2 => (true, false, false, true),
            3 => (false, false, false, true),
            4 => (false, true, false, true),
            5 => (false, true, false, false),
            6 => (false, true, true, false),
            7 => (false, false, true, false),
            8 => (true, false, true, false),
            _ => return Heading::Hold,
        };
        Heading::Move { up, down, left, right }
    }
}

impl BossLayer {
    /// 從頭開始：位置與腳本起點（`start` 是腳本表的鍵）。
    pub(crate) fn restart(&mut self, y: u16, x: u16, start: u16) {
        *self = BossLayer { y, x, script: start };
    }

    /// 腳本走一步（原版 `sub_15BB0`、`sub_15BCB`）。`read` 讀資料段的腳本位元組，`cleared` 是過關旗標：
    /// 過關後指令一律當作不動（`sub_15C1B`），但讀取位置照常前進。回傳讀到的位元組，呼叫端看它是不是結束碼。
    ///
    /// 結束位元組 [`SCRIPT_END`] 不前進、當作不動。其他位元組一律前進，不認識的碼（例如第二關的結束碼 1Fh）
    /// 也前進，之後讀到腳本之外的資料。
    pub(crate) fn step(&mut self, cleared: bool, read: impl Fn(u16) -> u8) -> u8 {
        let byte = read(self.script);
        let code = if byte == SCRIPT_END {
            0
        } else {
            self.script = self.script.wrapping_add(1);
            byte
        };
        let heading = if cleared { Heading::Hold } else { Heading::from_code(code) };
        if let Heading::Move { up, down, left, right } = heading {
            if up {
                self.y = self.y.wrapping_sub(STEP_ROWS);
            }
            if down {
                self.y = self.y.wrapping_add(STEP_ROWS);
            }
            if left {
                self.x = self.x.wrapping_sub(STEP_COLS);
            }
            if right {
                self.x = self.x.wrapping_add(STEP_COLS);
            }
        }
        byte
    }
}

impl Core {
    /// 第 `layer` 層從頭開始：位置與腳本。
    pub(crate) fn restart_boss_layer(&mut self, layer: usize, y: u16, x: u16, script: BossScript) {
        let start = self.tables.boss_start(script as usize);
        self.scenes.boss.layers[layer].restart(y, x, start);
    }

    /// 第 `layer` 層走一步，回傳腳本位元組（原版放在 `byte_1DB5A`）。
    pub(crate) fn step_boss_layer(&mut self, layer: usize) -> u8 {
        let cleared = self.stage_cleared();
        let mut l = self.scenes.boss.layers[layer];
        let byte = l.step(cleared, |at| self.tables.script_byte(at));
        self.scenes.boss.layers[layer] = l;
        byte
    }

    /// 把暫存區 `scratch` 從第 `source_row` 列起的 `rows` 列，以水平位移 `-x` 畫到顯示頁的第 `y` 列
    /// （原版 `sub_15AC5`、`sub_15AF4`、`sub_15B23`、`sub_15B52`、`sub_15B81`）。`slot` 是前端補間用的層編號。
    pub(crate) fn draw_boss_layer(&mut self, layer: usize, scratch: u16, source_row: u16, rows: u16, slot: u8) {
        let l = self.scenes.boss.layers[layer];
        self.select_scratch(scratch);
        self.set_blit_source(0, source_row);
        self.set_blit_x(0);
        self.set_blit_shift(0u16.wrapping_sub(l.x));
        self.set_blit_y(l.y);
        self.set_obj(Pool::Boss, slot);
        self.copy_scratch_rows(RowCopy::ReplaceShifted, rows);
        self.clear_obj();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(layer: &mut BossLayer, script: &[u8], cleared: bool) -> Vec<u8> {
        let base = layer.script;
        (0..script.len()).map(|_| layer.step(cleared, |at| script.get(usize::from(at - base)).copied().unwrap_or(0xEE))).collect()
    }

    #[test]
    fn the_eight_headings_move_ten_rows_and_one_column() {
        // 上、右上、右、右下、下、左下、左、左上。
        let want = [(-10i16, 0i16), (-10, 1), (0, 1), (10, 1), (10, 0), (10, -1), (0, -1), (-10, -1)];
        for (code, (dy, dx)) in (1u8..=8).zip(want) {
            let mut l = BossLayer { y: 100, x: 50, script: 0x1000 };
            l.step(false, |_| code);
            assert_eq!((l.y as i16 - 100, l.x as i16 - 50), (dy, dx), "指令 {code}");
            assert_eq!(l.script, 0x1001, "讀取位置前進");
        }
    }

    #[test]
    fn code_zero_and_unknown_codes_hold_but_still_advance() {
        for code in [0u8, 9, 0x1F, 0xFF] {
            let mut l = BossLayer { y: 7, x: 9, script: 0x1000 };
            let byte = l.step(false, |_| code);
            assert_eq!((l.y, l.x, l.script, byte), (7, 9, 0x1001, code), "碼 {code:#x}");
        }
    }

    #[test]
    fn the_end_byte_neither_moves_nor_advances() {
        let mut l = BossLayer { y: 7, x: 9, script: 0x1000 };
        let byte = l.step(false, |_| SCRIPT_END);
        assert_eq!((l.y, l.x, l.script, byte), (7, 9, 0x1000, SCRIPT_END));
    }

    #[test]
    fn after_the_stage_is_cleared_the_layer_stops_moving_but_the_script_keeps_advancing() {
        let mut l = BossLayer { y: 100, x: 50, script: 0x1000 };
        let bytes = run(&mut l, &[3, 3, 5], true);
        assert_eq!(bytes, [3, 3, 5]);
        assert_eq!((l.y, l.x, l.script), (100, 50, 0x1003));
    }

    #[test]
    fn positions_wrap_at_16_bits() {
        let mut l = BossLayer { y: 5, x: 0, script: 0x1000 };
        l.step(false, |_| 2);
        assert_eq!((l.y, l.x), (5u16.wrapping_sub(10), 1));
        l.x = 0;
        l.step(false, |_| 7);
        assert_eq!(l.x, 0xFFFF);
    }
}
