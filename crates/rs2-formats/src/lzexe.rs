//! 解開 `S.EXE` 的兩層 LZEXE，規則同 `tools/unpack_exe.py`。
//! 外層表頭簽章被改成四個空白，內層沒有 MZ 表頭，中間有 `0969:0100` 的包裝碼。

use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum UnpackError {
    Truncated,
    UnknownWrapper,
}

impl fmt::Display for UnpackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnpackError::Truncated => write!(f, "S.EXE 資料不足，串流提前結束"),
            UnpackError::UnknownWrapper => {
                write!(f, "第一層進入點不是預期的 FA E9 包裝碼，這份 S.EXE 不是已知版本")
            }
        }
    }
}

impl std::error::Error for UnpackError {}

struct Bits<'a> {
    src: &'a [u8],
    pos: usize,
    buf: u16,
    cnt: u32,
}

impl<'a> Bits<'a> {
    fn new(src: &'a [u8]) -> Result<Self, UnpackError> {
        let mut b = Bits { src, pos: 0, buf: 0, cnt: 16 };
        b.buf = b.word()?;
        Ok(b)
    }

    fn word(&mut self) -> Result<u16, UnpackError> {
        let lo = self.byte()?;
        let hi = self.byte()?;
        Ok(u16::from_le_bytes([lo, hi]))
    }

    fn byte(&mut self) -> Result<u8, UnpackError> {
        let v = *self.src.get(self.pos).ok_or(UnpackError::Truncated)?;
        self.pos += 1;
        Ok(v)
    }

    /// 第 16 位取完後要立刻補字，順序錯了後面的位元組全部錯位。
    fn bit(&mut self) -> Result<u16, UnpackError> {
        let r = self.buf & 1;
        self.cnt -= 1;
        if self.cnt == 0 {
            self.buf = self.word()?;
            self.cnt = 16;
        } else {
            self.buf >>= 1;
        }
        Ok(r)
    }
}

fn lz91(src: &[u8]) -> Result<Vec<u8>, UnpackError> {
    let mut b = Bits::new(src)?;
    let mut out: Vec<u8> = Vec::with_capacity(src.len() * 2);
    loop {
        if b.bit()? == 1 {
            let v = b.byte()?;
            out.push(v);
            continue;
        }
        let (mut n, off): (usize, isize);
        if b.bit()? == 0 {
            let hi = b.bit()?;
            let lo = b.bit()?;
            n = ((hi << 1) | lo) as usize + 2;
            off = b.byte()? as isize - 256;
        } else {
            let a = b.byte()? as isize;
            let c = b.byte()? as isize;
            off = (a | ((c & 0xF8) << 5)) - 0x2000;
            n = (c & 7) as usize + 2;
            if n == 2 {
                let x = b.byte()?;
                if x == 0 {
                    break;
                }
                if x == 1 {
                    continue;
                }
                n = x as usize + 1;
            }
        }
        for _ in 0..n {
            let at = out.len() as isize + off;
            if at < 0 {
                return Err(UnpackError::Truncated);
            }
            let v = out[at as usize];
            out.push(v);
        }
    }
    Ok(out)
}

fn le16(b: &[u8], at: usize) -> Result<u16, UnpackError> {
    let s = b.get(at..at + 2).ok_or(UnpackError::Truncated)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

/// 回傳內層解包映像（`tools/unpack_exe.py` 的 `S.unpacked.bin`）。
pub fn unpack_s_exe(exe: &[u8]) -> Result<Vec<u8>, UnpackError> {
    let hdr = le16(exe, 8)? as usize * 16;
    let cs = le16(exe, 0x16)? as usize;
    let entry = hdr + cs * 16;
    let orig_ip = le16(exe, entry)? as usize;
    let orig_cs = le16(exe, entry + 2)? as usize;
    let layer1 = lz91(exe.get(hdr..).ok_or(UnpackError::Truncated)?)?;
    let wrap = (orig_cs << 4) + orig_ip;
    if layer1.get(wrap..wrap + 2) != Some(&[0xFA, 0xE9][..]) {
        return Err(UnpackError::UnknownWrapper);
    }
    lz91(&layer1)
}
