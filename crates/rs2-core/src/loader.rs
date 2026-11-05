//! 檔案載入。原版讀 DOS 檔案進段記憶體，磁片提示與重試都在這一層。
//! 重製在啟動時已驗過全部雜湊，這裡只記下圖庫組的切換，並把會進 `dseg` 的資料寫進去。
//! 檔名對照見 `docs/re/02-file-formats.md`。

use crate::draw::{BegGroup, MixFile, ShtGroup};
use crate::game::Core;

impl Core {
    /// `sub_1AE3B`：`SHT1.GO`，5 區段，遮罩一起換。
    pub(crate) fn s1ae3b(&mut self) {
        self.sht_group = ShtGroup::Sht1;
        self.mask_group = ShtGroup::Sht1;
    }
    /// `sub_1AE66`：`SHT2.GO`。
    pub(crate) fn s1ae66(&mut self) {
        self.sht_group = ShtGroup::Sht2;
        self.mask_group = ShtGroup::Sht2;
    }
    /// `sub_1AE91`：`SHT3.GO`。
    pub(crate) fn s1ae91(&mut self) {
        self.sht_group = ShtGroup::Sht3;
        self.mask_group = ShtGroup::Sht3;
    }
    /// `sub_1AEBC`：`SHT4.GO` 與 `SHT4.GOT` 遮罩。
    pub(crate) fn s1aebc(&mut self) {
        self.sht_group = ShtGroup::Sht4;
        self.mask_group = ShtGroup::Sht4;
    }
    /// `sub_1AF10`：`SHT5.GO`。
    pub(crate) fn s1af10(&mut self) {
        self.sht_group = ShtGroup::Sht5;
        self.mask_group = ShtGroup::Sht5;
    }
    /// `sub_1AF3B`：`BH1.GO`，4 區段，遮罩不動。
    pub(crate) fn s1af3b(&mut self) {
        self.sht_group = ShtGroup::Bh1;
    }
    /// `sub_1AF66`：`BH2.GO`。
    pub(crate) fn s1af66(&mut self) {
        self.sht_group = ShtGroup::Bh2;
    }
    /// `sub_1AF91`：`BH4.GO`。
    pub(crate) fn s1af91(&mut self) {
        self.sht_group = ShtGroup::Bh4;
    }
    /// `sub_1AFBC`：`MAP.GO`，有自己的段變數，`Op::S18ACE` 固定讀它。
    pub(crate) fn s1afbc(&mut self) {}
    /// `sub_1B008`：`J1.GO`。
    pub(crate) fn s1b008(&mut self) {
        self.beg_group = BegGroup::J1;
    }
    /// `sub_1B01D`：`J2.GO`。
    pub(crate) fn s1b01d(&mut self) {
        self.beg_group = BegGroup::J2;
    }
    /// `sub_1B032`：`J3.GO`。
    pub(crate) fn s1b032(&mut self) {
        self.beg_group = BegGroup::J3;
    }
    /// `sub_1B047`：`J4.GO`。
    pub(crate) fn s1b047(&mut self) {
        self.beg_group = BegGroup::J4;
    }
    /// `sub_1BC4D`：`BEG1.BD1` 至 `BD4`。
    pub(crate) fn s1bc4d(&mut self) {
        self.beg_group = BegGroup::Beg1;
    }
    /// `sub_1BED5`：`DM1.LCK` 讀進 `dseg:AAE2h`，3000 位元組。
    pub(crate) fn s1bed5(&mut self) {
        let d = self.files.clone();
        self.load_demo(&d.dm1);
    }
    /// `sub_1BEEF`：`DM2.LCK`。
    pub(crate) fn s1beef(&mut self) {
        let d = self.files.clone();
        self.load_demo(&d.dm2);
    }
    fn load_demo(&mut self, data: &[u8]) {
        for (i, &b) in data.iter().enumerate().take(0xBB8) {
            self.ds.w8(0xAAE2u16.wrapping_add(i as u16), b);
        }
    }
    /// `sub_1BF71`：`SHT.SCN`，第三關地圖。資料在 `Core.files.scn`，讀取端自己索引。
    pub(crate) fn s1bf71(&mut self) {}
    /// `sub_1BFCF`：`SHT.SCR`，資料在 `Core.files.scr`。
    pub(crate) fn s1bfcf(&mut self) {}
    /// `sub_1AA38`：`A.MIX`。
    pub(crate) fn s1aa38(&mut self) {
        self.mix = MixFile::A;
    }
    /// `sub_1AA52`：`B.MIX`。
    pub(crate) fn s1aa52(&mut self) {
        self.mix = MixFile::B;
    }
    /// `sub_1AA6C`：`B.MIX`。
    pub(crate) fn s1aa6c(&mut self) {
        self.mix = MixFile::B;
    }
    /// `sub_1AA86`：`B.MIX`。
    pub(crate) fn s1aa86(&mut self) {
        self.mix = MixFile::B;
    }
    /// `sub_1AAA0`：`B.MIX`。
    pub(crate) fn s1aaa0(&mut self) {
        self.mix = MixFile::B;
    }
}
