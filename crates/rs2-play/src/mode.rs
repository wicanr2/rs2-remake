//! 顯示模式：VGA、EGA、CGA、MGA（Hercules）。原版開機時在文字選單選一次（`docs/re/14-display-modes.md` 第 1 節），
//! 重製可以在遊戲中切換。
//!
//! 切換只換「畫面從哪一個接收端來」，不碰模擬：`Session` 永遠把同一批繪製命令餵給所有接收端，
//! 所以切換當下的畫面一定是對的。唯一和模擬有關的是第三關的地形來源（`Game::set_terrain_scr`），
//! 它在進第三關時讀取，見 `Session::set_view`。

use crate::ui::text;
use rs2_gfx::MonoKind;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DisplayMode {
    /// 原版 E 模式、VGA 的 DAC 調色盤。預設。
    #[default]
    Vga,
    /// 原版 E 模式、EGA 的屬性暫存器調色盤（16 色）。
    Ega,
    /// 原版 C 模式：BIOS 模式 6，640×200，1 位元。
    Cga,
    /// 原版 H 模式：Hercules，640×300，1 位元。
    Mga,
}

impl DisplayMode {
    /// 循環的順序，也是工具列與說明文字列出的順序。
    pub const ALL: [DisplayMode; 4] = [DisplayMode::Vga, DisplayMode::Ega, DisplayMode::Cga, DisplayMode::Mga];

    /// 下一個模式：VGA、EGA、CGA、MGA 輪流。
    pub fn next(self) -> DisplayMode {
        match self {
            DisplayMode::Vga => DisplayMode::Ega,
            DisplayMode::Ega => DisplayMode::Cga,
            DisplayMode::Cga => DisplayMode::Mga,
            DisplayMode::Mga => DisplayMode::Vga,
        }
    }

    /// 下一個選得到的模式。單色圖庫不可用時（`mono_available` 為假）跳過 CGA 與 MGA。
    pub fn next_available(self, mono_available: bool) -> DisplayMode {
        let mut m = self.next();
        while m.is_mono() && !mono_available {
            m = m.next();
        }
        m
    }

    /// CGA 與 MGA：單色圖庫、第三關讀 `SHT.SCR`、沒有喇叭附帶音。
    pub fn is_mono(self) -> bool {
        matches!(self, DisplayMode::Cga | DisplayMode::Mga)
    }

    /// 單色模式對應的接收端種類。
    pub fn mono_kind(self) -> Option<MonoKind> {
        match self {
            DisplayMode::Cga => Some(MonoKind::Cga),
            DisplayMode::Mga => Some(MonoKind::Mga),
            _ => None,
        }
    }

    /// 繪圖附帶的 PC 喇叭聲。原版只有 E 的特效 blit 驅動喇叭，CGA、MGA 沒有（`docs/re/14-display-modes.md` 5.4）。
    pub fn speaker(self) -> bool {
        !self.is_mono()
    }

    /// 設定檔與命令列用的小寫名稱。
    pub fn code(self) -> &'static str {
        match self {
            DisplayMode::Vga => "vga",
            DisplayMode::Ega => "ega",
            DisplayMode::Cga => "cga",
            DisplayMode::Mga => "mga",
        }
    }

    /// `code` 的反向，不分大小寫。不認得回傳 `None`。
    pub fn from_code(s: &str) -> Option<DisplayMode> {
        DisplayMode::ALL.into_iter().find(|m| m.code().eq_ignore_ascii_case(s.trim()))
    }

    /// 工具列「顯示模式：」後面接的字。
    pub fn label(self) -> &'static str {
        match self {
            DisplayMode::Vga => text::MODE_VGA,
            DisplayMode::Ega => text::MODE_EGA,
            DisplayMode::Cga => text::MODE_CGA,
            DisplayMode::Mga => text::MODE_MGA,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::font;

    #[test]
    fn cycle_visits_the_four_modes_in_order_and_wraps() {
        let mut m = DisplayMode::default();
        assert_eq!(m, DisplayMode::Vga);
        let mut seen = vec![m];
        for _ in 0..4 {
            m = m.next();
            seen.push(m);
        }
        use DisplayMode::*;
        assert_eq!(seen, vec![Vga, Ega, Cga, Mga, Vga]);
        assert_eq!(DisplayMode::ALL.to_vec(), vec![Vga, Ega, Cga, Mga]);
    }

    #[test]
    fn cycle_skips_mono_when_the_bank_files_are_missing() {
        use DisplayMode::*;
        assert_eq!(Vga.next_available(false), Ega);
        assert_eq!(Ega.next_available(false), Vga, "EGA 的下一個是 CGA，被跳過，再跳 MGA，回到 VGA");
        assert_eq!(Cga.next_available(false), Vga);
        assert_eq!(Mga.next_available(false), Vga);
        for m in DisplayMode::ALL {
            assert_eq!(m.next_available(true), m.next());
        }
    }

    #[test]
    fn mono_flag_kind_and_speaker_agree() {
        for m in DisplayMode::ALL {
            assert_eq!(m.is_mono(), m.mono_kind().is_some(), "{m:?}");
            assert_eq!(m.speaker(), !m.is_mono(), "{m:?}");
        }
        assert_eq!(DisplayMode::Cga.mono_kind(), Some(MonoKind::Cga));
        assert_eq!(DisplayMode::Mga.mono_kind(), Some(MonoKind::Mga));
    }

    #[test]
    fn code_roundtrip_and_case_insensitive_parse() {
        for m in DisplayMode::ALL {
            assert_eq!(DisplayMode::from_code(m.code()), Some(m));
            assert_eq!(DisplayMode::from_code(&m.code().to_ascii_uppercase()), Some(m));
        }
        assert_eq!(DisplayMode::from_code(" cga "), Some(DisplayMode::Cga));
        for bad in ["", "vg", "hercules", "h", "640x200", "1"] {
            assert_eq!(DisplayMode::from_code(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn labels_are_the_original_names_and_have_glyphs() {
        let want = ["VGA", "EGA", "CGA", "MGA"];
        let f = font::builtin();
        for (m, w) in DisplayMode::ALL.into_iter().zip(want) {
            assert_eq!(m.label(), w);
            assert!(f.missing(m.label()).is_empty());
        }
    }
}
