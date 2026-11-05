//! 平坦的 64 KB 資料段。實體、子彈、特效、格網、列表、輸入旗標全部住在這裡。
//! 原版的表外讀寫是可觀察行為（規格 001「實作方法」），所以不拆成各池的陣列。

/// IDA 線性位址換成 `dseg` 偏移：`dseg:X` = 線性 `1CBC0h + X`。
pub const fn a(linear: u32) -> u16 {
    (linear - 0x1CBC0) as u16
}

#[derive(Clone)]
pub struct Dseg {
    mem: Box<[u8; 0x10000]>,
}

impl Dseg {
    /// 取解包映像位移 `0CBC0h` 起到檔尾，其餘補零。
    pub fn from_image(image: &[u8]) -> Self {
        let mut mem: Box<[u8; 0x10000]> = vec![0u8; 0x10000].into_boxed_slice().try_into().unwrap();
        let src = &image[rs2_formats::DSEG_IMAGE_OFFSET..];
        let n = src.len().min(0x10000);
        mem[..n].copy_from_slice(&src[..n]);
        Dseg { mem }
    }

    #[inline]
    pub fn r8(&self, o: u16) -> u8 {
        self.mem[o as usize]
    }

    #[inline]
    pub fn w8(&mut self, o: u16, v: u8) {
        self.mem[o as usize] = v;
    }

    /// 小端序，偏移環繞。
    #[inline]
    pub fn r16(&self, o: u16) -> u16 {
        u16::from_le_bytes([self.mem[o as usize], self.mem[o.wrapping_add(1) as usize]])
    }

    #[inline]
    pub fn w16(&mut self, o: u16, v: u16) {
        let [lo, hi] = v.to_le_bytes();
        self.mem[o as usize] = lo;
        self.mem[o.wrapping_add(1) as usize] = hi;
    }

    pub fn bytes(&self) -> &[u8] {
        &self.mem[..]
    }

    /// `rep stosw`：從 `o` 起寫 `n` 個字組。
    pub fn fill16(&mut self, o: u16, n: u16, v: u16) {
        let mut p = o;
        for _ in 0..n {
            self.w16(p, v);
            p = p.wrapping_add(2);
        }
    }

    /// `rep movsw`：逐字組複製，重疊時行為與 8086 相同。
    pub fn copy16(&mut self, dst: u16, src: u16, n: u16) {
        let (mut d, mut s) = (dst, src);
        for _ in 0..n {
            let v = self.r16(s);
            self.w16(d, v);
            d = d.wrapping_add(2);
            s = s.wrapping_add(2);
        }
    }
}
