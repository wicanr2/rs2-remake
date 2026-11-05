//! 呼叫約定轉接層。
//!
//! 還沒重寫的函式用暫存器傳參數與回傳值：等待的長度與整頁複製的列數放在 `cx`，
//! 關卡主函式把離開碼留在 `dx`。已重寫的領域（標題、片尾）不碰暫存器，
//! 一律經這裡有型別的函式呼叫它們。暫存器消失時，只需要改這個檔案。
//!
//! 這裡的函式留下的暫存器值和原版相同（`cx` 是等待長度或列數），所以還沒重寫的程式
//! 看到的殘值沒有變。標題與片尾自己不再留下 `ax`、`bx`、`dx`、`si`、`di` 的殘值，
//! 查證見 `workspace/progress/rewrite-title.md`。

use crate::game::{Core, Exit};

impl Core {
    /// `sub_11097`：等 `ticks` 個 18.2 Hz 的 tick（原版放 `cx`）。
    pub(crate) async fn wait_ticks(&mut self, ticks: u16) {
        self.r.cx = ticks;
        self.s11097().await;
    }

    /// `sub_19D08`：把暫存區前 `rows` 列複製到目前的頁，來源、位移與目的位置取自繪製參數（原版 `rows` 放 `cx`）。
    /// 這是標題與片尾唯一讀 `BlitCmd::cx` 的繪製命令，`rs2-gfx` 用它決定複製幾列。
    pub(crate) fn copy_scratch_rows(&mut self, rows: u16) {
        self.r.cx = rows;
        self.s19d08();
    }

    /// 關卡主函式（`sub_10CD0`、`sub_13205`、`sub_13A51`、`sub_13DAF`）回來時，`dx` 裡的離開碼。
    /// 原版比對整個 16 位元字組（`cmp dx, 51h`），不是 `E`、`Q`、`C` 的值回傳 `None`。
    pub(crate) fn stage_exit(&self) -> Option<Exit> {
        match self.r.dx {
            0x51 => Some(Exit::Quit),
            0x45 => Some(Exit::End),
            0x43 => Some(Exit::Clear),
            _ => None,
        }
    }
}
