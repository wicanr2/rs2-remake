//! 預期的原版檔案雜湊，來源是 `source-manifest.json`。只有雜湊，不含原版位元組。

use crate::sha256::sha256_hex;

/// 檔名與預期 SHA-256。
pub const EXPECTED: [(&str, &str); 5] = [
    ("S.EXE", "a7445e9cbe1ecb5fb33cbcad84f54e00ed07757d8ce1da57f9aad2d686b2ea3f"),
    ("MY.GO", "0ee566a1de718835c891be31558ea0a372ce8b54b9c86a4781a11d3c7407ec44"),
    ("SHT1.GO", "0c2a1a66a126a7307e1f07c62ac8e560c96def7c52781dd9f57d432641ce043d"),
    ("BH1.GO", "e52639c267f54acefdd6c1f1ab8a0b2519accbdd271b750b9939365c0417a3f5"),
    ("DM1.LCK", "610650d996f2746c4344117142d7d44655b40cc7cb2067e7ae899678ea0ee5d0"),
];

/// 解包映像（`S.unpacked.bin`）的 SHA-256。
pub const UNPACKED_SHA256: &str = "8475b41b45507ab10f0430238e5d20a11563b7d1b202997680f73a7cab34de72";

/// `dseg:0` 在解包映像內的位移。
pub const DSEG_IMAGE_OFFSET: usize = 0xCBC0;

pub fn check_hash(name: &str, data: &[u8], expected: &str) -> Result<(), String> {
    let got = sha256_hex(data);
    if got == expected {
        Ok(())
    } else {
        Err(format!("{name} 的雜湊不符：預期 {expected}，實際 {got}。這一版只支援已知的原版檔案。"))
    }
}
