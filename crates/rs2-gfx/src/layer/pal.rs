//! 調色盤狀態與名稱辨識。
//!
//! HD 圖庫的調色盤變體以名稱（`stage1_boss` 等）分資料夾。`SetDac` 命令只帶 DAC 值，不帶名稱，
//! 所以用 DAC 表的指紋（FNV-1a 64 位元）辨識：每個名稱對應 `dseg` 內一張 48 位元組的 DAC 表
//! （位址同 `tools/extract_assets.py` 的 `PALETTES`），`rs2-trace` 的 `layer.rs` 測試逐項核對指紋
//! 與映像內容一致，所以辨識的依據是映像位元組，不是推測。指紋不含調色盤的值。

use crate::sink::AC;

/// 名稱、`dseg` 內 DAC 表的偏移、指紋。
///
/// 兩處內容相同的表：`loader` 與 `title_d` 共用 `5A9h`，用 `title_d`；`stage2_c`（`0CE8h`）與 `stage3`（`0D29h`）
/// 的 48 位元組相同（`hd-variants-manifest.json` 兩者的變體雜湊也相同），`identify` 取先出現的 `stage2_c`，變體檔內容一樣。
pub const PALETTES: [(&str, u16, u64); 18] = [
    ("stage1", 0x4FD, 0x7EE31EF19F59574A),
    ("stage1_boss", 0x53E, 0xF88777D4BD0CA4B6),
    ("title_d", 0x5A9, 0xC77144565FD13DC5),
    ("title_e", 0x5EA, 0x1DD0C3D560824D78),
    ("menu", 0x62B, 0x849255D51AA8D6DD),
    ("beg1_a", 0x66C, 0x21C02775019E8991),
    ("beg1_b", 0x6AD, 0x4816F9613D32DA3A),
    ("j1", 0x6EE, 0xECB36BA267A3F312),
    ("j2", 0x72F, 0xA436A9C7CCDB8887),
    ("j3", 0x770, 0x657C46BF606DA640),
    ("j4", 0x7B1, 0x684F8D6C31CCD846),
    ("stage2", 0xC25, 0x2293944ED66F860C),
    ("stage2_b", 0xC66, 0x5CBE684AAD9F9EC3),
    ("stage2_c", 0xCE8, 0x657D4B652CE4B806),
    ("stage3", 0xD29, 0x657D4B652CE4B806),
    ("stage4", 0xD82, 0x042A97BC2DB41531),
    ("ending_a", 0xE54, 0x98B2930E8E74B76F),
    ("ending_b", 0xE95, 0xD57907D4A80DE14E),
];

/// 48 位元組 DAC 表（6 位元值）的指紋。
pub fn fingerprint(rgb: &[u8; 48]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in rgb {
        h ^= (b & 0x3F) as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// 辨識 DAC 表，回傳 `PALETTES` 的下標。
pub fn identify(rgb: &[u8; 48]) -> Option<usize> {
    let f = fingerprint(rgb);
    PALETTES.iter().position(|p| p.2 == f)
}

/// 6 位元轉 8 位元，與 `render_page` 相同。
#[inline]
pub fn to8(v: u8) -> u8 {
    ((v as u32 * 255 + 31) / 63) as u8
}

/// 48 位元組 DAC 表的 16 色。`S14448` 這類整張表的設定寫到 DAC 0 與 30h，所以色號 8 至 15（屬性暫存器的
/// 38h 至 3Fh）是表的後 8 色，和 `visible8` 的結果相同。
pub fn colors_of(rgb: &[u8; 48]) -> [[u8; 3]; 16] {
    std::array::from_fn(|i| [to8(rgb[3 * i] & 0x3F), to8(rgb[3 * i + 1] & 0x3F), to8(rgb[3 * i + 2] & 0x3F)])
}

/// DAC 狀態與目前的調色盤名稱。
#[derive(Clone)]
pub struct Pal {
    pub dac: [[u8; 3]; 256],
    pub name: Option<usize>,
}

impl Pal {
    pub fn new() -> Pal {
        Pal { dac: [[0; 3]; 256], name: None }
    }

    /// 套用 `SetDac`。起點 0 的呼叫是整張表，用來辨識名稱；起點 30h 是同一張表的第二次載入。
    pub fn set(&mut self, start: u8, rgb: &[u8; 48]) {
        for i in 0..16usize {
            let idx = (start as usize + i) & 0xFF;
            self.dac[idx] = [rgb[3 * i] & 0x3F, rgb[3 * i + 1] & 0x3F, rgb[3 * i + 2] & 0x3F];
        }
        if start == 0 {
            self.name = identify(rgb);
        }
    }

    /// 只有一張整表時的狀態：DAC 0 與 30h 都寫這張表。
    pub fn from_table(rgb: &[u8; 48]) -> Pal {
        let mut p = Pal::new();
        p.set(0x30, rgb);
        p.set(0, rgb);
        p
    }

    /// 由 8 位元的 16 色與名稱下標建立，`visible8` 會還原這 16 色。
    pub fn from_colors(name: usize, colors: &[[u8; 3]; 16]) -> Pal {
        let mut p = Pal::new();
        p.name = Some(name);
        for (i, c) in colors.iter().enumerate() {
            let v6 = |v: u8| ((v as u32 * 63 + 127) / 255) as u8;
            p.dac[AC[i]] = [v6(c[0]), v6(c[1]), v6(c[2])];
        }
        p
    }

    /// 套用 `SetDacOne`（INT 10h AX=1010h）。名稱不變：單一暫存器的改動（淡出、閃光）不換圖庫變體。
    pub fn set_one(&mut self, index: u8, rgb: &[u8; 3]) {
        self.dac[index as usize] = [rgb[0] & 0x3F, rgb[1] & 0x3F, rgb[2] & 0x3F];
    }

    pub fn name_str(&self) -> Option<&'static str> {
        self.name.map(|i| PALETTES[i].0)
    }

    /// 色號 0 至 15 經屬性暫存器後的顏色，8 位元 RGB。
    pub fn visible8(&self) -> [[u8; 3]; 16] {
        let mut o = [[0u8; 3]; 16];
        for (i, v) in o.iter_mut().enumerate() {
            let d = self.dac[AC[i]];
            *v = [to8(d[0]), to8(d[1]), to8(d[2])];
        }
        o
    }

    /// 可見 16 色的指紋，快取鍵用。
    pub fn visible_key(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for c in self.visible8() {
            for b in c {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01B3);
            }
        }
        h
    }
}

impl Default for Pal {
    fn default() -> Self {
        Pal::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_ignores_high_bits_and_distinguishes() {
        let mut a = [0u8; 48];
        a[5] = 9;
        let mut b = a;
        b[5] = 9 | 0x40;
        assert_eq!(fingerprint(&a), fingerprint(&b));
        b[6] = 1;
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn set_dac_wraps_and_masks() {
        let mut p = Pal::new();
        let mut rgb = [0u8; 48];
        rgb[0] = 0x7F;
        p.set(0xF8, &rgb);
        assert_eq!(p.dac[0xF8], [0x3F, 0, 0]);
        let mut rgb2 = [1u8; 48];
        rgb2[45] = 63;
        p.set(0x30, &rgb2);
        assert_eq!(p.dac[0x3F], [63, 1, 1]);
        // 名稱只在起點 0 辨識
        assert!(p.name.is_none());
    }
}
