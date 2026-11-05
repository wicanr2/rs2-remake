//! 調色盤。每個場景的調色盤是資料段的一張 65 位元組表：17 位元組的 EGA 屬性表接 48 位元組的 DAC 表。
//! 設定函式把 DAC 表寫兩次（起點 0 與 30h），EGA 屬性表只附在第一次。

use crate::draw::DrawCmd;
use crate::game::Core;

/// 一個調色盤函式對 DAC 的第二次寫入的起點。
const SECOND_DAC_START: u8 = 0x30;

/// 場景的調色盤（唯讀表格）。每張是 EGA 屬性表接 DAC 表。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Palette {
    /// 載入畫面與第一關進場（`sub_11112`），DAC 表與標題的 `title_d` 共用，屬性表另存。
    Loader,
    Stage1,
    Stage1Boss,
    Stage2,
    /// 第二關中頭目（`stage2_b`）。
    Stage2Mid,
    /// 第二關頭目（`stage2_c`）。
    Stage2Boss,
    /// 第二關淡出工作表所在的位置。工作表的內容在 `Stage2Backdrop::fade`，只有最後 3 位元組和屬性表讀這裡。
    Stage2Fade,
    Stage3,
    Stage4,
    /// 標題動畫的手臂、眼部特寫與製作名單。DAC 表與 `Loader` 共用。
    TitleD,
    /// 標題動畫的太空船正面與 `RS-S` 字樣。
    TitleE,
    /// 選單。
    Menu,
    /// 標題動畫的地球與太空船。
    Beg1A,
    /// 太空船飛過時的灰階閃光。
    Beg1B,
    /// 第一關到第四關的過場圖（J1 到 J4）。
    J1,
    J2,
    J3,
    J4,
    /// 片尾的地景與人物行星。
    EndingA,
    EndingB,
}

impl Palette {
    pub(crate) const ALL: [Palette; 20] = [
        Palette::Loader,
        Palette::Stage1,
        Palette::Stage1Boss,
        Palette::Stage2,
        Palette::Stage2Mid,
        Palette::Stage2Boss,
        Palette::Stage2Fade,
        Palette::Stage3,
        Palette::Stage4,
        Palette::TitleD,
        Palette::TitleE,
        Palette::Menu,
        Palette::Beg1A,
        Palette::Beg1B,
        Palette::J1,
        Palette::J2,
        Palette::J3,
        Palette::J4,
        Palette::EndingA,
        Palette::EndingB,
    ];
}

impl Core {
    /// 設場景的調色盤：DAC 表寫兩次，EGA 屬性表跟著第一次。
    pub(crate) fn load_palette(&mut self, palette: Palette) {
        let data = *self.tables.palette(palette);
        self.push_dac(0, data.dac, Some(data.attr));
        self.push_dac(SECOND_DAC_START, data.dac, None);
    }

    /// 同 [`Core::load_palette`] 的一次寫入，DAC 表的前幾個位元組由呼叫端給（第二關淡出的工作表在結構裡），
    /// 其餘取調色盤表。`start` 為 0 時附上屬性表，原版的 EGA 分支只呼叫一次 INT 10h AX=1002h。
    pub(crate) fn set_dac_overlay(&mut self, start: u8, palette: Palette, overlay: &[u8]) {
        let data = *self.tables.palette(palette);
        let mut rgb = data.dac;
        rgb[..overlay.len()].copy_from_slice(overlay);
        self.push_dac(start, rgb, (start == 0).then_some(data.attr));
    }

    fn push_dac(&mut self, start: u8, rgb: [u8; 48], attr: Option<[u8; 17]>) {
        self.draws.push(DrawCmd::SetDac { start, rgb });
        if let Some(regs) = attr {
            self.draws.push(DrawCmd::EgaAttr { regs });
        }
    }
}

