//! 平面式 VGA 的寫入引擎：4 個 64 KB 平面、閂鎖、序列器 map mask 與圖形控制器暫存器。
//!
//! 原版 E 模式的 blit 靠這些暫存器的實際效果決定結果（讀取平面、寫入模式 0／1／2、
//! 邏輯功能、閂鎖），所以這裡不把每個函式簡化成「某平面某位元組 OR」，而是照 VGA 的
//! 規則逐位元組模擬 CPU 的讀與寫，blit 端再照機器碼的讀寫順序呼叫。
//!
//! 只實作原版 E 模式會用到的部分：讀取模式 0、寫入模式 0 至 3、位元遮罩、設定／重設、
//! 旋轉。位址是 `A000` 段內的 16 位元偏移，64 KB 孔徑。

pub const PLANE_SIZE: usize = 0x10000;

#[derive(Clone)]
pub struct Vga {
    planes: [Vec<u8>; 4],
    latch: [u8; 4],
    /// 序列器索引 2：寫入的平面。
    map_mask: u8,
    /// GC 索引 0、1。
    set_reset: u8,
    enable_sr: u8,
    /// GC 索引 3：位元 0 至 2 旋轉，位元 3 至 4 邏輯功能。
    rotate: u8,
    func: u8,
    /// GC 索引 4：讀取模式 0 讀的平面。
    read_map: u8,
    /// GC 索引 5 位元 0 至 1：寫入模式。
    wmode: u8,
    /// GC 索引 8。
    bit_mask: u8,
    watch: Option<(usize, u16)>,
    watch_hit: bool,
}

impl Default for Vga {
    fn default() -> Self {
        Self::new()
    }
}

impl Vga {
    /// 平面全 0，暫存器是 BIOS 設定圖形模式後的預設值：map mask 0Fh、位元遮罩 FFh、其餘 0。
    pub fn new() -> Vga {
        Vga {
            planes: [vec![0; PLANE_SIZE], vec![0; PLANE_SIZE], vec![0; PLANE_SIZE], vec![0; PLANE_SIZE]],
            latch: [0; 4],
            map_mask: 0x0F,
            set_reset: 0,
            enable_sr: 0,
            rotate: 0,
            func: 0,
            read_map: 0,
            wmode: 0,
            bit_mask: 0xFF,
            watch: None,
            watch_hit: false,
        }
    }

    pub fn plane(&self, p: usize) -> &[u8] {
        &self.planes[p]
    }

    /// 整個平面換成 `data`（64 KB），給從快照起跑用。
    pub fn load_plane(&mut self, p: usize, data: &[u8]) {
        assert_eq!(data.len(), PLANE_SIZE);
        self.planes[p].copy_from_slice(data);
    }

    /// 序列器索引 2（3C4h／3C5h）。
    pub fn seq_map_mask(&mut self, v: u8) {
        self.map_mask = v & 0x0F;
    }

    /// 圖形控制器（3CEh／3CFh）。`out dx, ax` 的 `al` 是索引、`ah` 是資料。
    pub fn gc(&mut self, index: u8, v: u8) {
        match index {
            0 => self.set_reset = v & 0x0F,
            1 => self.enable_sr = v & 0x0F,
            3 => {
                self.rotate = v & 7;
                self.func = (v >> 3) & 3;
            }
            4 => self.read_map = v & 3,
            5 => {
                self.wmode = v & 3;
                debug_assert!(v & 0x08 == 0, "讀取模式 1 不在 E 模式 blit 使用");
            }
            8 => self.bit_mask = v,
            _ => {}
        }
    }

    pub fn map_mask(&self) -> u8 {
        self.map_mask
    }

    pub fn read_map(&self) -> u8 {
        self.read_map
    }

    pub fn write_mode(&self) -> u8 {
        self.wmode
    }

    /// 除錯用：監看某平面某位元組是否被寫入。
    pub fn set_watch(&mut self, plane: usize, off: u16) {
        self.watch = Some((plane, off));
        self.watch_hit = false;
    }

    pub fn take_watch_hit(&mut self) -> bool {
        std::mem::take(&mut self.watch_hit)
    }

    /// CPU 讀一個位元組：4 個平面的該位元組載入閂鎖，回傳 read map 選的平面。
    #[inline]
    pub fn read(&mut self, off: u16) -> u8 {
        let o = off as usize;
        for p in 0..4 {
            self.latch[p] = self.planes[p][o];
        }
        self.latch[self.read_map as usize]
    }

    #[inline]
    fn alu(&self, p: usize, d: u8) -> u8 {
        let l = self.latch[p];
        let r = match self.func {
            0 => d,
            1 => d & l,
            2 => d | l,
            _ => d ^ l,
        };
        (r & self.bit_mask) | (l & !self.bit_mask)
    }

    /// CPU 寫一個位元組。
    #[inline]
    pub fn write(&mut self, off: u16, v: u8) {
        let o = off as usize;
        match self.wmode {
            0 => {
                let data = v.rotate_right(self.rotate as u32);
                for p in 0..4 {
                    if self.map_mask & (1 << p) == 0 {
                        continue;
                    }
                    let d = if self.enable_sr & (1 << p) != 0 {
                        if self.set_reset & (1 << p) != 0 { 0xFF } else { 0 }
                    } else {
                        data
                    };
                    let r = self.alu(p, d);
                    self.put(p, o, r);
                }
            }
            1 => {
                for p in 0..4 {
                    if self.map_mask & (1 << p) != 0 {
                        let l = self.latch[p];
                        self.put(p, o, l);
                    }
                }
            }
            2 => {
                for p in 0..4 {
                    if self.map_mask & (1 << p) == 0 {
                        continue;
                    }
                    let d = if v & (1 << p) != 0 { 0xFF } else { 0 };
                    let r = self.alu(p, d);
                    self.put(p, o, r);
                }
            }
            _ => {
                // 寫入模式 3：CPU 資料（旋轉後）與位元遮罩相與當作遮罩，資料來自設定／重設。
                let m = v.rotate_right(self.rotate as u32) & self.bit_mask;
                for p in 0..4 {
                    if self.map_mask & (1 << p) == 0 {
                        continue;
                    }
                    let d = if self.set_reset & (1 << p) != 0 { 0xFF } else { 0 };
                    let l = self.latch[p];
                    let f = match self.func {
                        0 => d,
                        1 => d & l,
                        2 => d | l,
                        _ => d ^ l,
                    };
                    self.put(p, o, (f & m) | (l & !m));
                }
            }
        }
    }

    #[inline]
    fn put(&mut self, p: usize, o: usize, v: u8) {
        self.planes[p][o] = v;
        if let Some((wp, wo)) = self.watch {
            if wp == p && wo as usize == o {
                self.watch_hit = true;
            }
        }
    }

    /// 繞過暫存器直接寫某平面。給顯示前端以外的自繪內容（幀計數）使用。
    pub fn poke(&mut self, p: usize, off: u16, v: u8) {
        self.put(p, off as usize, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode0_replace_writes_enabled_planes() {
        let mut v = Vga::new();
        v.seq_map_mask(0b0101);
        v.write(10, 0xA5);
        assert_eq!([v.plane(0)[10], v.plane(1)[10], v.plane(2)[10], v.plane(3)[10]], [0xA5, 0, 0xA5, 0]);
    }

    #[test]
    fn mode0_and_uses_latch() {
        let mut v = Vga::new();
        v.seq_map_mask(0x0F);
        v.write(3, 0xFF);
        v.gc(3, 0x08); // AND
        let _ = v.read(3);
        v.write(3, 0x0F);
        assert_eq!(v.plane(2)[3], 0x0F);
        v.gc(3, 0x10); // OR
        let _ = v.read(3);
        v.write(3, 0xF0);
        assert_eq!(v.plane(0)[3], 0xFF);
        v.gc(3, 0x00);
    }

    #[test]
    fn mode1_copies_latches_across_planes() {
        let mut v = Vga::new();
        v.seq_map_mask(0b0001);
        v.write(5, 0x11);
        v.seq_map_mask(0b0010);
        v.write(5, 0x22);
        v.seq_map_mask(0b1000);
        v.write(5, 0x88);
        v.seq_map_mask(0x0F);
        let _ = v.read(5);
        v.gc(5, 1);
        v.write(900, 0x00);
        assert_eq!([v.plane(0)[900], v.plane(1)[900], v.plane(2)[900], v.plane(3)[900]], [0x11, 0x22, 0, 0x88]);
    }

    #[test]
    fn mode2_expands_color_bits() {
        let mut v = Vga::new();
        v.gc(5, 2);
        v.write(7, 0b0101);
        assert_eq!([v.plane(0)[7], v.plane(1)[7], v.plane(2)[7], v.plane(3)[7]], [0xFF, 0, 0xFF, 0]);
    }

    #[test]
    fn read_map_selects_returned_plane() {
        let mut v = Vga::new();
        v.seq_map_mask(0b0100);
        v.write(1, 0x5A);
        v.seq_map_mask(0x0F);
        v.gc(4, 1);
        assert_eq!(v.read(1), 0);
        v.gc(4, 2);
        assert_eq!(v.read(1), 0x5A);
    }
}
