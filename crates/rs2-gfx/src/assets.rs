//! 圖庫素材。啟動時從玩家自備的原版載入，雜湊不符就拒絕。

use rs2_formats::{EXPECTED, GoFile, check_hash, parse_go};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Assets {
    pub my: GoFile,
    pub sht1: GoFile,
    pub bh1: GoFile,
}

impl Assets {
    /// 讀 `MY.GO`、`SHT1.GO`、`BH1.GO`，先驗 SHA-256 再解碼。
    pub fn load(dir: &Path) -> Result<Assets, String> {
        let one = |name: &str| -> Result<GoFile, String> {
            let data = std::fs::read(dir.join(name)).map_err(|e| format!("讀不到 {name}：{e}"))?;
            let (_, want) = EXPECTED
                .iter()
                .find(|(n, _)| *n == name)
                .ok_or_else(|| format!("{name} 沒有預期雜湊"))?;
            check_hash(name, &data, want)?;
            parse_go(&data).map_err(|e| format!("{name} 解碼失敗：{e}"))
        };
        Ok(Assets { my: one("MY.GO")?, sht1: one("SHT1.GO")?, bh1: one("BH1.GO")? })
    }
}
