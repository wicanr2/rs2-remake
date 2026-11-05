//! 繪製命令。`rs2-core` 不碰繪圖，只把原版 blit 的呼叫與參數依序記下，交給接收端。
//! 語意見 `docs/re/12-stages-presentation.md` 第 2.3 節。

/// 原版的繪製函式。名稱是 `sub_` 位址。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// MY 4 平面，覆寫。
    S18C0A,
    /// MY 4 平面，OR。
    S18CE2,
    /// MY 遮罩，AND。
    S18D46,
    /// MY 遮罩，閃白。
    S18D89,
    /// MY 4 平面，OR，並產生喇叭音（特效）。
    S18DCC,
    /// MY 遮罩，AND。
    S18E30,
    /// MY 遮罩，AND。
    S18E73,
    /// MY 4 平面，OR。
    S18EB6,
    /// SHT 遮罩，AND。
    S18F1A,
    /// SHT 4 平面，OR。
    S18F5D,
    /// SHT 4 平面，覆寫。
    S18FC1,
    /// SHT 遮罩，閃白。
    S19025,
    /// SHT／BH 4 平面寫進暫存區。
    S19ADC,
    /// 暫存區列複製到顯示頁。
    S19BA4,
    /// 清除目前的頁（190 列）。
    S19CF9,
    /// 暫存區列複製到顯示頁，帶 x 位移。
    S19D08,
    /// 暫存區整列 OR 到顯示頁。
    S19DA8,
    /// 暫存區整列 AND 到顯示頁。
    S1A09F,
    /// 填實色。`dx` 是色號，`cx` 是列數。
    S1A287,
    /// 血條的一格。`dx` 是色號。
    S1929F,
    /// 5 位數字，`cx` 是數值，位置在 `text_x`、`text_y`。幀計數在 (12h, BFh)。
    /// `sub_16C7D` 的擊中計數也走這裡，條件是子彈 w7 為 `1`、`2`，發行版的武器字母碰不到。
    S1890D,
    /// 清暫存區 B（`sub_1152D`，E 模式從 `A800:1F40` 填 `20BFh` 個字組）。
    S1152D,
}

/// `sub_19ADC` 那組段變數目前指向的圖庫。BH 與 SHT 共用同一組，載入 BH 時會蓋掉 SHT。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShtGroup {
    Bh1,
    Sht1,
}

/// 物件所屬的池，前端補間用。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pool {
    Entity,
    PlayerBullet,
    EnemyBullet,
    Effect,
    Scenery,
    Player,
    Pod,
    /// 背景層，槽號是層編號。
    Background,
    /// 頭目合成圖（`sub_15AC5`），和碰撞體、可見分件走同一條腳本，要一起內插才不會錯開。
    Boss,
}

/// 穩定的物件編號。世代在槽被生成函式配置時加 1，所以同一幀釋放又重用的槽不會被當成同一個物件。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjId {
    pub pool: Pool,
    pub slot: u8,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlitCmd {
    pub op: Op,
    /// `word_26880`，單位是位元組（8 像素）。
    pub x: u16,
    /// `word_26882`，像素列。
    pub y: u16,
    /// `word_26888`，圖庫索引。
    pub index: u16,
    /// `word_26872`，頁偏移。
    pub page: u16,
    /// `word_2685C`，暫存區選擇。
    pub scratch: u16,
    /// `word_26864`，來源欄。
    pub src_x: u16,
    /// `word_26866`，來源列。
    pub src_y: u16,
    /// `word_26868`，水平位移。
    pub shift: u16,
    pub cx: u16,
    pub dx: u16,
    /// `word_2688A`，`sub_1890D` 的欄。
    pub text_x: u16,
    /// `word_2688E`，`sub_1890D` 的列。
    pub text_y: u16,
    pub sht_group: ShtGroup,
    pub obj: Option<ObjId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawCmd {
    Blit(BlitCmd),
    /// 寫 16 個 DAC 暫存器（`sub_11170`，INT 10h AX=1012h），值是 6 位元。
    SetDac { start: u8, rgb: [u8; 48] },
}
