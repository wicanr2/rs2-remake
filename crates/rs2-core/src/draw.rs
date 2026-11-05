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
    /// 清暫存區 A（`sub_11504`，從 `A000:1F40` 填 `20BFh` 個字組）。
    S11504,
    /// 清兩頁（`sub_18894`）。
    S18894,
    /// MY 4 平面，反相覆寫。
    S18C6E,
    /// BEG1／J 4 平面，覆寫。
    S19068,
    /// BEG1／J 4 平面，反相。
    S190CC,
    /// BEG1／J 4 平面寫進暫存區。
    S19B40,
    /// 暫存區列複製到顯示頁，垂直鏡射。
    S19C4F,
    /// 暫存區逐平面 OR，每列 2 位元組。
    S19E66,
    /// 暫存區整列 OR，垂直鏡射。
    S19F3B,
    /// 閂鎖複製，每列 2 位元組。
    S19FF9,
    /// 暫存區 AND，每列 2 位元組。
    S1A144,
    /// 暫存區整列 AND，垂直鏡射。
    S1A1E6,
    /// MAP 圖塊寫進暫存區。
    S18ACE,
    /// 第三關捲動（`sub_17EBA`）：暫存區 A 前 3E80h 位元組複製到 B，GC 旋轉 4。語意見 `workspace/progress/ops-needed.md`。
    S17EBA,
    /// 第三關捲動（`sub_17EEA`）：暫存區 B 就地左移 4 像素，位元遮罩 0Fh。
    S17EEA,
    /// 第三關捲動（`sub_17F26`）：暫存區 A 的一欄（200 列）轉 4 位元複製到 B。
    S17F26,
    /// 第三關捲動（`sub_17F5D`）：暫存區 B 的接縫欄取下一欄低半，位元遮罩 0Fh。
    S17F5D,
    /// 第三關捲動（`sub_17FA0`）：暫存區（`word_2685C`）的 40 欄 x 190 列以寫入模式 1 複製到顯示頁。
    S17FA0,
    /// 第三關捲動（`sub_18031`）：同 `S17FA0`，`word_2685C` 為 8000h。
    S18031,
    /// 第三關捲動（`sub_180C6`）：暫存區 A、B 各整體左移 28h 位元組，寫入模式 1。
    S180C6,
    /// MGA（Hercules）專用（`sub_1117F`）：目前頁 `0000h`～`1FFFh` 複製到 `2000h`～`3FFFh`，也就是偶數列那一組複製到
    /// 中間那條掃描線。`byte_1CF74` 為 `E` 時直接 `retn`，所以 VGA、EGA 與 CGA 沒有動作。
    /// 原版呼叫端在一幀畫完之後、翻頁之前，位置見 `docs/re/14-display-modes.md`。這個命令不帶參數，只有 `page` 有意義。
    S1117F,
}

/// `sub_19ADC` 那組段變數目前指向的圖庫。BH 與 SHT 共用同一組，載入 BH 時會蓋掉 SHT。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShtGroup {
    Bh1,
    Bh2,
    Bh4,
    Sht1,
    Sht2,
    Sht3,
    Sht4,
    Sht5,
}

/// `sub_19068`、`sub_19B40` 那組段變數目前指向的圖庫。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BegGroup {
    Beg1,
    J1,
    J2,
    J3,
    J4,
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
    /// SHT 遮罩段目前指向的圖庫。BH 沒有遮罩區段，載入 BH 不會改它。第四關的遮罩在 `SHT4.GOT`。
    pub mask_group: ShtGroup,
    pub beg_group: BegGroup,
    /// 上一個繪製命令之後，邏輯寫進 `word_2687E` 的值。`None` 表示沒寫過。
    /// 這個字同時是邏輯的形狀寬與繪製的每列位元組殘值，`y < 0` 的列複製會讀到它。
    pub stride_reg: Option<u16>,
    pub obj: Option<ObjId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawCmd {
    Blit(BlitCmd),
    /// 寫 16 個 DAC 暫存器（`sub_11170`，INT 10h AX=1012h），值是 6 位元。
    SetDac { start: u8, rgb: [u8; 48] },
    /// 寫 1 個 DAC 暫存器（INT 10h AX=1010h）。
    SetDacOne { index: u8, rgb: [u8; 3] },
    /// EGA 屬性暫存器（INT 10h AX=1002h）：16 個調色盤暫存器接邊框色，值是 6 位元的 rgbRGB。
    /// 調色盤函式在 `byte_1CF75` 為 `Y`（EGA）時寫它，為 `N`（VGA）時寫 DAC（`SetDac`）。核心兩種都記，
    /// 接收端依顯示模式取用。`SetDac` 的 `start` 為 0 時一起記，表在 DAC 表之前 11h 位元組（`loader` 例外，見 `flow.rs`）。
    EgaAttr { regs: [u8; 17] },
    /// 對顯示記憶體 `rep stosw`，平面由目前的 map mask 決定。
    VramFill { off: u16, words: u16, value: u16 },
    /// 設序列器 map mask（`sub_1882E` 的 `bl` 轉換後、`sub_18878` 的 0Fh）。
    SetMapMask { mask: u8 },
}

/// 載入到音樂緩衝區的曲目。`sub_1AA38` 載 `A.MIX`，`sub_1AA52`、`1AA6C`、`1AA86`、`1AAA0` 載 `B.MIX`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixFile {
    A,
    B,
}

/// 給音訊端的命令，和繪製命令一樣在一步結束時交出去。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCmd {
    /// `10AD4h`：`AX=0`、`AX=2`、`AX=4`，循環播放。
    Play(MixFile),
    /// `10B12h`：`AX=5`、`AX=3`、`AX=0`。
    Stop,
    /// `sub_187C3` 把 `sub_187B3`、`sub_187BB` 的第一個位元組改成 `retn`（`false`）或 `nop`（`true`）。
    Speaker(bool),
}
