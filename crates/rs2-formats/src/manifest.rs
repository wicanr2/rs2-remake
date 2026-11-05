//! 預期的原版檔案雜湊，來源是 `source-manifest.json`。只有雜湊，不含原版位元組。
//! 範圍是規格 002「輸入」列的全部檔案（遊戲邏輯與圖庫）；音樂 `A.MIX`、`B.MIX` 屬規格 003。

use crate::sha256::sha256_hex;

/// 檔名與預期 SHA-256。
pub const EXPECTED: [(&str, &str); 26] = [
    ("A.MIX", "6bff9b44dc94c09b6a92a07da8c59c7e6b05972798bc1ba4ab488b171f5cc545"),
    ("B.MIX", "89c17de8dcefdae25bfbff1feeab75e6da187bed31d5107e0ea2a2d8da847eba"),
    ("S.EXE", "a7445e9cbe1ecb5fb33cbcad84f54e00ed07757d8ce1da57f9aad2d686b2ea3f"),
    ("MY.GO", "0ee566a1de718835c891be31558ea0a372ce8b54b9c86a4781a11d3c7407ec44"),
    ("SHT1.GO", "0c2a1a66a126a7307e1f07c62ac8e560c96def7c52781dd9f57d432641ce043d"),
    ("SHT2.GO", "4a5ec94fe40a42edee814b2fdf1f50f7bf45c6b9bc224fc55bd42d92ad058921"),
    ("SHT3.GO", "b1f512d26cc2ef24c13eea77564c828cae797e99ede47d13067a825fe4baa8f8"),
    ("SHT4.GO", "ff2aaefcf4ce74d91bc7b199718d9a6355d5f94b14939e112a357a29a8663973"),
    ("SHT5.GO", "007612c38aafa3b64e6a28488da9353fd6f8e64a76405731951f57802e644ac8"),
    ("SHT4.GOT", "f5357c3a08f68178cd8e21a8da8e52f12ba4f8f4dcada14090b8438d19987657"),
    ("BH1.GO", "e52639c267f54acefdd6c1f1ab8a0b2519accbdd271b750b9939365c0417a3f5"),
    ("BH2.GO", "30a98aea79e0d9780a89b661824f5eeba8b7afb98a079630d2c9d341a762ccee"),
    ("BH4.GO", "b8a28e8e3fa6da6a8f238b4e928f172c156d8508c1863414613dc291d6d8dc04"),
    ("MAP.GO", "895ac5550c46363d7271924174e4a74a751ce67a2ab922c256b90509be79ce28"),
    ("J1.GO", "79d75fab23b2c678d28bd4083f75111f8aa26626b1b6f3bd8db3698a0093b8b8"),
    ("J2.GO", "96d22c0de4b7049e3813550cfe17ca5d26fd666b7cdb8e98102239aa5d8f320a"),
    ("J3.GO", "80199e42faa231c5d5ad96bf4833c389b41823185f6ca3edc055233810ae7df3"),
    ("J4.GO", "7c8dcf195ecf61cc154231066c4d4d76d87e0f32010f46abd66d9f5ab93d22dc"),
    ("BEG1.BD1", "d1cdf3fd1f5bb2ed2e5d139e8ed716c1dd6a22a5bafeb7754d3d55bcd00a987d"),
    ("BEG1.BD2", "1767a0cd85181f954b7ff3f4cebf7c4bde131d1609b8e6603613836477f09f40"),
    ("BEG1.BD3", "4c0cbf9be6d7fc5c5527507fffc0cbee0ac2b986c80dcc690e7259225f85180f"),
    ("BEG1.BD4", "25cb15829dc12c4d0a03f5bb6a570a8dd0be868650fcaefd44c0220ab3a9a3fa"),
    ("SHT.SCN", "3c64ca68d5add5f30534f05b105904270cae54435a78252fd2f9c7368ca29f93"),
    ("SHT.SCR", "1dd20dc60b3c224343f6ed3d866ae0b1e81fee8365290b75cba35a6e04bcdb37"),
    ("DM1.LCK", "610650d996f2746c4344117142d7d44655b40cc7cb2067e7ae899678ea0ee5d0"),
    ("DM2.LCK", "fa53561144adee07052f12d8ed71d1e55d7258538f1d238d38755885f244ef83"),
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
