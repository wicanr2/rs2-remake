//! 檔案載入。原版讀 DOS 檔案進段記憶體，磁片提示與重試都在這一層。
//! 重製在啟動時已驗過全部雜湊，這裡只記下目前載入的是哪一組圖庫與曲目（[`Banks`]），
//! 並把會進資料段的展示錄影寫進去。檔名對照見 `docs/re/02-file-formats.md`。
//!
//! 原版為每個檔案各寫一個載入函式（有些內容完全相同，例如四關各一份載入 `B.MIX`），重製一個函式帶參數：
//!
//! | 函式 | 參數 | 原版 |
//! |---|---|---|
//! | [`Core::load_sht_bank`] | [`ShtGroup`] | `Sht1` 至 `Sht4`：`sub_1AE3B`、`1AE66`、`1AE91`、`1AEBC`（`SHT4.GO` 與遮罩 `SHT4.GOT`）。`Sht5`：`sub_1AF10`。`Bh1`、`Bh2`、`Bh4`：`sub_1AF3B`、`1AF66`、`1AF91` |
//! | [`Core::load_beg_bank`] | [`BegGroup`] | `Beg1`：`sub_1BC4D`（`BEG1.BD1` 至 `BD4`）。`J1` 至 `J4`：`sub_1B008`、`1B01D`、`1B032`、`1B047` |
//! | [`Core::load_mix`] | [`MixFile`] | `A`：`sub_1AA38`。`B`：`sub_1AA52`、`1AA6C`、`1AA86`、`1AAA0`（第一至四關各一份，內容相同） |
//! | [`Core::load_demo`] | [`DemoFile`] | `sub_1BED5`、`sub_1BEEF` |
//!
//! 沒有對應函式的載入：
//!
//! - `MAP.GO`（`sub_1AFBC`）：第三關的地圖圖庫有自己的段變數，`Op::MapToScratch` 固定讀它，重製沒有狀態要記。
//! - `SHT.SCN`（`sub_1BF71`）：E 模式讀它，單色模式 `1BFAAh` 起把檔名改成 `.SCR` 再讀；解壓在 [`Core::load_map`]，
//!   資料在 `Core::files`（`Game::set_terrain_scr` 選用哪一份）。
//! - `sub_1BFCF`（`SHT.SCR` 的另一個載入函式）：唯一呼叫端 `1BF68h` 在 `sub_1BF23` 的 `retn`（`1BF50h`）之後，
//!   `1BF51h` 起沒有任何分支或呼叫指到，不可達。前一個領域留下的 `s1bfcf` 已刪。

use crate::draw::{BegGroup, MixFile, ShtGroup};
use crate::game::Core;

/// 目前載入的圖庫組與曲目。BH 與 SHT 共用同一組段變數，載入 BH 會蓋掉 SHT；遮罩另有一個段變數，載入 BH 不會蓋掉它。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Banks {
    /// `sub_19ADC` 一族讀的圖庫（`word_1CF5E`～`64`）。
    pub sht: ShtGroup,
    /// SHT 遮罩（`word_1CF5C`）。BH 沒有遮罩區段。
    pub mask: ShtGroup,
    /// `sub_19068`、`sub_19B40` 讀的圖庫（`word_1CF46`～`4C`）。
    pub beg: BegGroup,
    /// 音樂緩衝區裡的曲目。
    pub mix: MixFile,
}

impl Default for Banks {
    /// 開機時的狀態。
    fn default() -> Banks {
        Banks { sht: ShtGroup::Bh1, mask: ShtGroup::Sht1, beg: BegGroup::Beg1, mix: MixFile::A }
    }
}

/// 展示錄影檔。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DemoFile {
    /// `DM1.LCK`，標題輪播與第一關。
    Dm1,
    /// `DM2.LCK`，第二關入口載入它，蓋掉第一關的錄影。
    Dm2,
}

/// 展示錄影的位元組數。
pub const DEMO_BYTES: usize = 3000;

/// 展示錄影區（原版 `dseg:AAE2h` 起 3000 位元組）：每幀一個位元組，記錄玩家 1 的輸入位元。
/// `DM1.LCK` 或 `DM2.LCK` 載入進來，非展示時玩家 1 把本幀的輸入錄在這裡。
///
/// 原版的位址是 16 位元環繞，超出 3000 位元組的位置是別的變數。重製的幀計數不會超過錄影長度
/// （展示在第 288h 幀前重播，錄製有 5DBh 的上限），超出範圍的讀回 0、寫入丟掉。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DemoTape(pub Vec<u8>);

impl Default for DemoTape {
    fn default() -> DemoTape {
        DemoTape(vec![0; DEMO_BYTES])
    }
}

impl Core {
    /// 載入關卡圖庫（`SHT`、`BH`）。`SHT` 的遮罩一起換，`BH` 沒有遮罩，不動。
    pub(crate) fn load_sht_bank(&mut self, group: ShtGroup) {
        self.banks.sht = group;
        if matches!(group, ShtGroup::Sht1 | ShtGroup::Sht2 | ShtGroup::Sht3 | ShtGroup::Sht4 | ShtGroup::Sht5) {
            self.banks.mask = group;
        }
    }

    /// 載入 `BEG1`、`J` 圖庫。
    pub(crate) fn load_beg_bank(&mut self, group: BegGroup) {
        self.banks.beg = group;
    }

    /// 載入音樂檔到音樂緩衝區。
    pub(crate) fn load_mix(&mut self, file: MixFile) {
        self.banks.mix = file;
    }

    /// 展示錄影讀進展示錄影區（`dseg:AAE2h` 起 3000 位元組）。
    pub(crate) fn load_demo(&mut self, file: DemoFile) {
        let files = self.files.clone();
        let data = match file {
            DemoFile::Dm1 => &files.dm1,
            DemoFile::Dm2 => &files.dm2,
        };
        for (i, &b) in data.iter().enumerate().take(DEMO_BYTES) {
            self.record_demo_byte(i as u16, b);
        }
    }

    /// 展示錄影第 `index` 個位元組。每幀一個位元組，記錄玩家 1 的輸入位元（`player::InputBit`）。
    pub(crate) fn demo_byte(&self, index: u16) -> u8 {
        self.tape.0.get(usize::from(index)).copied().unwrap_or(0)
    }

    /// 非展示時，玩家 1 把本幀的輸入位元錄在展示錄影的第 `index` 個位元組。
    pub(crate) fn record_demo_byte(&mut self, index: u16, v: u8) {
        if let Some(slot) = self.tape.0.get_mut(usize::from(index)) {
            *slot = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Fixes, Start};

    fn core() -> Core {
        Core::for_test(Start::Demo, Fixes::NONE)
    }

    #[test]
    fn sht_loads_replace_the_mask_and_bh_loads_keep_it() {
        let mut c = core();
        c.load_sht_bank(ShtGroup::Sht4);
        assert_eq!((c.banks.sht, c.banks.mask), (ShtGroup::Sht4, ShtGroup::Sht4));
        c.load_sht_bank(ShtGroup::Bh2);
        assert_eq!((c.banks.sht, c.banks.mask), (ShtGroup::Bh2, ShtGroup::Sht4), "載入 BH 不會蓋掉遮罩");
        c.load_sht_bank(ShtGroup::Sht5);
        assert_eq!((c.banks.sht, c.banks.mask), (ShtGroup::Sht5, ShtGroup::Sht5));
    }

    #[test]
    fn boot_state_matches_the_original_segment_variables() {
        assert_eq!(Banks::default(), Banks { sht: ShtGroup::Bh1, mask: ShtGroup::Sht1, beg: BegGroup::Beg1, mix: MixFile::A });
    }

    #[test]
    fn the_demo_record_is_3000_bytes_at_the_record_area() {
        let mut c = core();
        let mut files = (*c.files).clone();
        files.dm1 = (0..4000u32).map(|i| (i % 251) as u8 + 1).collect();
        files.dm2 = vec![0xEE; 10];
        c.files = std::rc::Rc::new(files);
        c.load_demo(DemoFile::Dm1);
        assert_eq!([c.demo_byte(0), c.demo_byte(2999), c.demo_byte(3000)], [1, (2999 % 251) as u8 + 1, 0], "只讀 3000 位元組");
        c.load_demo(DemoFile::Dm2);
        assert_eq!([c.demo_byte(0), c.demo_byte(9), c.demo_byte(10)], [0xEE, 0xEE, ((10 % 251) as u8) + 1], "較短的檔只蓋前面");
    }
}
