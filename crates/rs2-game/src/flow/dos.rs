//! DOS 記憶體配置的簿記。各關入口向 DOS 配置十塊圖庫緩衝區再立刻釋放（INT 21h AH=48h、49h）。
//! DOS 不在模擬範圍，只留下原版寫進資料段的訊息前綴與段值，它們沒有邏輯讀，但全流程對拍逐位元組比對。

use crate::game::Core;

/// 各關配置十塊緩衝區的對象。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StageBuffers {
    Stage2,
    Stage3,
    Stage4,
}

/// 十塊緩衝區的塊數。
const BLOCKS: usize = 10;
/// 訊息第三個字元的初值。
const COUNTER_START: u8 = 0x30;

/// 簿記的狀態。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DosBook {
    /// 錯誤訊息的兩個三字元前綴（`byte_1CEF6` 起與 `byte_1CF08` 起）。每塊配置與釋放把第三個字元加 1。
    pub message_a: [u8; 3],
    pub message_b: [u8; 3],
    /// 配置結果：十個段值（`word_1CF5C` 起）。
    pub segments: [u16; BLOCKS],
}

impl StageBuffers {
    /// 配置前寫進訊息前綴的前兩個字元。
    const fn alloc_prefix(self) -> (u8, u8) {
        match self {
            StageBuffers::Stage2 | StageBuffers::Stage3 => (0x41, 0x2B),
            StageBuffers::Stage4 => (0x41, 0x34),
        }
    }

    /// 釋放前寫進訊息前綴的前兩個字元；第四關不寫。
    const fn free_prefix(self) -> Option<(u8, u8)> {
        match self {
            StageBuffers::Stage2 | StageBuffers::Stage3 => Some((0x42, 0x2D)),
            StageBuffers::Stage4 => None,
        }
    }

    /// 十塊的段落數（16 位元組為一段落）。第三關的配置函式不記錄段值，只加計數。
    const fn paragraphs(self) -> Option<[u16; BLOCKS]> {
        match self {
            StageBuffers::Stage2 => Some([0x600, 0x600, 0x600, 0x600, 0x600, 0x10, 0x10, 0x10, 0x10, 1]),
            StageBuffers::Stage3 => None,
            StageBuffers::Stage4 => Some([0x600, 0x600, 0x600, 0x600, 0x600, 1, 1, 1, 1, 1]),
        }
    }
}

impl DosBook {
    fn write_message(&mut self, first: u8, second: u8) {
        for message in [&mut self.message_a, &mut self.message_b] {
            *message = [first, second, COUNTER_START];
        }
    }

    /// 十次 DOS 呼叫，每次把兩個訊息前綴的第三個字元加 1。
    fn count_calls(&mut self) {
        for message in [&mut self.message_a, &mut self.message_b] {
            message[2] = message[2].wrapping_add(BLOCKS as u8);
        }
    }
}

impl Core {
    /// DOS 配置十塊的副作用（原版 `sub_138D6`、`sub_13BE0`、`sub_140A1`）。
    ///
    /// 模型：配置結果用第一個段值現有的值當基底（上一關釋放後最低的空洞起點，每關相同），之後每塊接在前一塊後面，
    /// 中間隔一個 MCB 段落：`seg[k] = seg[k-1] + 段落數[k-1] + 1`。第一關配置的傾印 `4989h, 4E8Ah, …` 符合這個公式。
    /// 假設不成立時對拍會在這 10 個字組先出現差異。
    pub(crate) fn dos_allocate_buffers(&mut self, which: StageBuffers) {
        let (first, second) = which.alloc_prefix();
        let book = &mut self.flow.dos;
        book.write_message(first, second);
        if let Some(blocks) = which.paragraphs() {
            let mut seg = book.segments[0];
            for (k, paragraphs) in blocks.into_iter().enumerate() {
                book.segments[k] = seg;
                seg = seg.wrapping_add(paragraphs).wrapping_add(1);
            }
        }
        book.count_calls();
    }

    /// 依相反順序釋放十塊（原版 `sub_139CD`、`sub_13CCD`、`sub_1418E`）。段值仍留在配置結果的十個字組。
    pub(crate) fn dos_free_buffers(&mut self, which: StageBuffers) {
        let book = &mut self.flow.dos;
        if let Some((first, second)) = which.free_prefix() {
            book.write_message(first, second);
        }
        book.count_calls();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Fixes, Start};

    fn core() -> Core {
        Core::for_test(Start::Demo, Fixes::NONE)
    }

    #[test]
    fn dos_bookkeeping_follows_the_original_message_and_segment_chain() {
        let mut c = core();
        c.flow.dos.segments[0] = 0x4989;
        c.dos_allocate_buffers(StageBuffers::Stage2);
        // 段值：第一塊維持現值，之後每塊接在前一塊後面，中間隔一個段落。
        let want: Vec<u16> = [0x600u16, 0x600, 0x600, 0x600, 0x600, 0x10, 0x10, 0x10, 0x10, 1]
            .iter()
            .scan(0x4989u16, |seg, &para| {
                let this = *seg;
                *seg += para + 1;
                Some(this)
            })
            .collect();
        assert_eq!(c.flow.dos.segments.to_vec(), want);
        // 訊息前綴：A+、B 與計數 30h 加 10 次呼叫。
        assert_eq!(c.flow.dos.message_a, [0x41, 0x2B, 0x3A]);
        assert_eq!(c.flow.dos.message_b, [0x41, 0x2B, 0x3A]);
        c.dos_free_buffers(StageBuffers::Stage2);
        assert_eq!(c.flow.dos.message_a, [0x42, 0x2D, 0x3A]);
        // 第四關釋放不重寫前綴，計數接著加。
        c.dos_allocate_buffers(StageBuffers::Stage4);
        assert_eq!(c.flow.dos.message_a, [0x41, 0x34, 0x3A]);
        c.dos_free_buffers(StageBuffers::Stage4);
        assert_eq!(c.flow.dos.message_a[2], 0x44);
        // 第三關只動前綴與計數，不動段值。
        c.flow.dos.segments[0] = 0x1111;
        c.dos_allocate_buffers(StageBuffers::Stage3);
        assert_eq!(c.flow.dos.segments[0], 0x1111);
        assert_eq!(c.flow.dos.message_a[2], 0x3A);
    }
}
