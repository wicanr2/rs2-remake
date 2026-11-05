//! 8086 暫存器。跨函式的參數與回傳值照原版走暫存器，因為有些行為依賴它們的殘值，
//! 例如 `sub_11375` 第一次呼叫改寫 `ax`，`sub_1135F` 接著用 `al` 決定血條那一格的顏色。

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Regs {
    pub ax: u16,
    pub bx: u16,
    pub cx: u16,
    pub dx: u16,
    pub si: u16,
    pub di: u16,
    pub bp: u16,
    /// 旗標只在跨函式傳遞時才用，例如被呼叫端以 `stc`、`clc` 或 `cmp` 回傳結果。
    pub cf: bool,
    pub zf: bool,
}

impl Regs {
    #[inline]
    pub fn al(&self) -> u8 {
        self.ax as u8
    }
    #[inline]
    pub fn ah(&self) -> u8 {
        (self.ax >> 8) as u8
    }
    #[inline]
    pub fn bl(&self) -> u8 {
        self.bx as u8
    }
    #[inline]
    pub fn bh(&self) -> u8 {
        (self.bx >> 8) as u8
    }
    #[inline]
    pub fn cl(&self) -> u8 {
        self.cx as u8
    }
    #[inline]
    pub fn ch(&self) -> u8 {
        (self.cx >> 8) as u8
    }
    #[inline]
    pub fn dl(&self) -> u8 {
        self.dx as u8
    }
    #[inline]
    pub fn dh(&self) -> u8 {
        (self.dx >> 8) as u8
    }
    #[inline]
    pub fn set_al(&mut self, v: u8) {
        self.ax = (self.ax & 0xFF00) | v as u16;
    }
    #[inline]
    pub fn set_ah(&mut self, v: u8) {
        self.ax = (self.ax & 0x00FF) | ((v as u16) << 8);
    }
    #[inline]
    pub fn set_bl(&mut self, v: u8) {
        self.bx = (self.bx & 0xFF00) | v as u16;
    }
    #[inline]
    pub fn set_bh(&mut self, v: u8) {
        self.bx = (self.bx & 0x00FF) | ((v as u16) << 8);
    }
    #[inline]
    pub fn set_cl(&mut self, v: u8) {
        self.cx = (self.cx & 0xFF00) | v as u16;
    }
    #[inline]
    pub fn set_ch(&mut self, v: u8) {
        self.cx = (self.cx & 0x00FF) | ((v as u16) << 8);
    }
    #[inline]
    pub fn set_dl(&mut self, v: u8) {
        self.dx = (self.dx & 0xFF00) | v as u16;
    }
    #[inline]
    pub fn set_dh(&mut self, v: u8) {
        self.dx = (self.dx & 0x00FF) | ((v as u16) << 8);
    }
}
