//! 圖庫素材。啟動時從玩家自備的原版載入，雜湊不符就拒絕。
//!
//! 檔案與載入端對照見 `docs/re/02-file-formats.md`「檔案與載入端對照」。
//! - `MY`、`SHT1`～`SHT3`、`SHT5`：`.GO` 5 區段，第 0 段是遮罩（`sub_1B071`）。
//! - `SHT4.GO` 只有 4 個平面區段，遮罩在 `SHT4.GOT`（`sub_1AEBC`：`sub_1B218` 讀 `.GO`，
//!   `sub_1C738` 把副檔名第 3 字元換成 `T` 後，`sub_1C757` 解到遮罩段 `word_1CF5C`）。
//! - `BH1`、`BH2`、`BH4`、`J1`～`J4`、`MAP`：`.GO` 4 區段，沒有遮罩。
//! - `BEG1`：`BEG1.BD1`～`BD4` 四個檔各是一個平面（`sub_1C24F`：`BD1` 進 `word_1CF46`，
//!   也就是 `sub_19068` 第一個選的平面 3），每個檔是單獨成檔的 RLE 圖庫。

use rs2_game::{BegGroup, ShtGroup};
use rs2_formats::{BankItem, EXPECTED, GoFile, check_hash, parse_bank, parse_go, unrle};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Assets {
    pub my: GoFile,
    pub sht1: GoFile,
    pub sht2: GoFile,
    pub sht3: GoFile,
    /// `SHT4.GO` 的 4 個平面，`mask` 是 `SHT4.GOT`。
    pub sht4: GoFile,
    pub sht5: GoFile,
    pub bh1: GoFile,
    pub bh2: GoFile,
    pub bh4: GoFile,
    /// `MAP.GO`，第三關的 8x10 圖塊。
    pub map: GoFile,
    pub j1: GoFile,
    pub j2: GoFile,
    pub j3: GoFile,
    pub j4: GoFile,
    /// `BEG1.BD1`～`BD4`，`planes[0]` 是 `BD1`（平面 3）。
    pub beg1: GoFile,
    /// 載入時 `EXPECTED` 沒有預期雜湊、因此沒有驗證的檔名。
    pub unverified: Vec<String>,
}

fn empty_go() -> GoFile {
    GoFile { mask: None, planes: Default::default() }
}

impl Assets {
    /// 全部圖庫都是空的。給測試用 `Assets { my, ..Assets::empty() }` 只填要用的庫。
    pub fn empty() -> Assets {
        Assets {
            my: empty_go(),
            sht1: empty_go(),
            sht2: empty_go(),
            sht3: empty_go(),
            sht4: empty_go(),
            sht5: empty_go(),
            bh1: empty_go(),
            bh2: empty_go(),
            bh4: empty_go(),
            map: empty_go(),
            j1: empty_go(),
            j2: empty_go(),
            j3: empty_go(),
            j4: empty_go(),
            beg1: empty_go(),
            unverified: Vec::new(),
        }
    }

    /// 讀全部圖庫，先驗 SHA-256 再解碼。`EXPECTED` 查得到的檔才驗，查不到的記進 `unverified`。
    pub fn load(dir: &Path) -> Result<Assets, String> {
        let mut unverified: Vec<String> = Vec::new();
        let mut read = |name: &str| -> Result<Vec<u8>, String> {
            let data = std::fs::read(dir.join(name)).map_err(|e| format!("讀不到 {name}：{e}"))?;
            match EXPECTED.iter().find(|(n, _)| *n == name) {
                Some((_, want)) => check_hash(name, &data, want)?,
                None => unverified.push(name.to_string()),
            }
            Ok(data)
        };
        let mut go = |name: &str| -> Result<GoFile, String> {
            let data = read(name)?;
            parse_go(&data).map_err(|e| format!("{name} 解碼失敗：{e}"))
        };
        let my = go("MY.GO")?;
        let sht1 = go("SHT1.GO")?;
        let sht2 = go("SHT2.GO")?;
        let sht3 = go("SHT3.GO")?;
        let mut sht4 = go("SHT4.GO")?;
        let sht5 = go("SHT5.GO")?;
        let bh1 = go("BH1.GO")?;
        let bh2 = go("BH2.GO")?;
        let bh4 = go("BH4.GO")?;
        let map = go("MAP.GO")?;
        let j1 = go("J1.GO")?;
        let j2 = go("J2.GO")?;
        let j3 = go("J3.GO")?;
        let j4 = go("J4.GO")?;
        if sht4.mask.is_some() {
            return Err("SHT4.GO 預期沒有遮罩區段（遮罩在 SHT4.GOT）".into());
        }
        // 單獨成檔的 RLE 圖庫：長度含表頭 2 位元組，`unrle` 在檔尾停止。
        let mut bank = |name: &str| -> Result<Vec<BankItem>, String> {
            let data = read(name)?;
            if data.len() < 2 {
                return Err(format!("{name} 太短"));
            }
            parse_bank(&unrle(&data, 0)).map_err(|e| format!("{name} 解碼失敗：{e}"))
        };
        sht4.mask = Some(bank("SHT4.GOT")?);
        let planes = [bank("BEG1.BD1")?, bank("BEG1.BD2")?, bank("BEG1.BD3")?, bank("BEG1.BD4")?];
        let beg1 = GoFile { mask: None, planes };
        Ok(Assets { my, sht1, sht2, sht3, sht4, sht5, bh1, bh2, bh4, map, j1, j2, j3, j4, beg1, unverified })
    }

    /// `sub_19ADC`、`sub_18F5D` 等那組段變數（`word_1CF5E～64`）目前指向的圖庫。
    pub fn sht(&self, g: ShtGroup) -> &GoFile {
        match g {
            ShtGroup::Bh1 => &self.bh1,
            ShtGroup::Bh2 => &self.bh2,
            ShtGroup::Bh4 => &self.bh4,
            ShtGroup::Sht1 => &self.sht1,
            ShtGroup::Sht2 => &self.sht2,
            ShtGroup::Sht3 => &self.sht3,
            ShtGroup::Sht4 => &self.sht4,
            ShtGroup::Sht5 => &self.sht5,
        }
    }

    /// SHT 遮罩段（`word_1CF5C`）目前指向的遮罩庫。BH 沒有遮罩區段，所以 `Bh*` 回傳 `None`。
    pub fn sht_mask(&self, g: ShtGroup) -> Option<&[BankItem]> {
        self.sht(g).mask.as_deref()
    }

    /// `sub_19068`、`sub_19B40` 那組段變數（`word_1CF46～4C`）目前指向的圖庫。
    pub fn beg(&self, g: BegGroup) -> &GoFile {
        match g {
            BegGroup::Beg1 => &self.beg1,
            BegGroup::J1 => &self.j1,
            BegGroup::J2 => &self.j2,
            BegGroup::J3 => &self.j3,
            BegGroup::J4 => &self.j4,
        }
    }
}
