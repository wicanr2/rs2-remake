//! 最小的 PNG 解碼與編碼，不依賴外部 crate。
//!
//! 解碼：8 位元的 RGBA、RGB、灰階、灰階加 alpha、索引色（含 `tRNS`），非交錯。含 inflate
//! （stored、固定與動態 Huffman）、zlib 的 Adler-32、區塊 CRC-32 與五種濾波。其他格式回傳錯誤。
//! 編碼：RGBA，zlib 用 stored 區塊，不壓縮，給驗收截圖用。

/// 解碼結果，一律是 8 位元 RGBA。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

const CRC_TABLE: [u32; 256] = make_crc_table();

const fn make_crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[n] = c;
        n += 1;
    }
    t
}

fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc = CRC_TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc
}

/// 區塊的 CRC：涵蓋型別與資料。
fn chunk_crc(typ: &[u8], data: &[u8]) -> u32 {
    !crc32_update(crc32_update(0xFFFF_FFFF, typ), data)
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

// ---------------------------------------------------------------------------
// inflate
// ---------------------------------------------------------------------------

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    buf: u64,
    cnt: u32,
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Self {
        Bits { d, pos: 0, buf: 0, cnt: 0 }
    }

    #[inline]
    fn refill(&mut self) {
        while self.cnt <= 56 && self.pos < self.d.len() {
            self.buf |= (self.d[self.pos] as u64) << self.cnt;
            self.pos += 1;
            self.cnt += 8;
        }
    }

    #[inline]
    fn bits(&mut self, n: u32) -> Result<u32, String> {
        if self.cnt < n {
            self.refill();
            if self.cnt < n {
                return Err("deflate 資料提前結束".into());
            }
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.cnt -= n;
        Ok(v)
    }

    fn align(&mut self) {
        let r = self.cnt % 8;
        self.buf >>= r;
        self.cnt -= r;
    }

    /// 對齊後複製 `n` 個位元組，先取位元緩衝區裡已預讀的，再從來源直接複製。
    fn take_bytes(&mut self, mut n: usize, out: &mut Vec<u8>) -> Result<(), String> {
        while n > 0 && self.cnt >= 8 {
            out.push(self.buf as u8);
            self.buf >>= 8;
            self.cnt -= 8;
            n -= 1;
        }
        if n > 0 {
            let end = self.pos.checked_add(n).filter(|&e| e <= self.d.len()).ok_or("stored 區塊資料不足")?;
            out.extend_from_slice(&self.d[self.pos..end]);
            self.pos = end;
        }
        Ok(())
    }

    /// 位元流結束後（已對齊）尚未使用的位元組從這裡起算。
    fn byte_pos(&self) -> usize {
        self.pos - (self.cnt / 8) as usize
    }
}

const TABLE_BITS: u32 = 15;

/// 單層查表的 Huffman 解碼表。項目是 `符號 << 4 | 碼長`，碼長 0 表示無效。
struct Huff {
    table: Vec<u16>,
}

impl Huff {
    fn build(lengths: &[u8]) -> Result<Huff, String> {
        let mut count = [0u32; 16];
        for &l in lengths {
            if l > 15 {
                return Err("碼長超過 15".into());
            }
            count[l as usize] += 1;
        }
        count[0] = 0;
        let mut left = 1i32;
        for c in count.iter().skip(1) {
            left = (left << 1) - *c as i32;
            if left < 0 {
                return Err("Huffman 碼過度訂閱".into());
            }
        }
        let mut next = [0u32; 16];
        let mut code = 0u32;
        for len in 1..16 {
            code = (code + count[len - 1]) << 1;
            next[len] = code;
        }
        let mut table = vec![0u16; 1 << TABLE_BITS];
        for (sym, &l) in lengths.iter().enumerate() {
            if l == 0 {
                continue;
            }
            let l = l as u32;
            let c = next[l as usize];
            next[l as usize] += 1;
            let rev = c.reverse_bits() >> (32 - l);
            let entry = ((sym as u16) << 4) | l as u16;
            let mut i = rev as usize;
            while i < (1 << TABLE_BITS) {
                table[i] = entry;
                i += 1 << l;
            }
        }
        Ok(Huff { table })
    }

    #[inline]
    fn decode(&self, br: &mut Bits) -> Result<usize, String> {
        if br.cnt < TABLE_BITS {
            br.refill();
        }
        let e = self.table[(br.buf & ((1 << TABLE_BITS) - 1)) as usize];
        let l = (e & 15) as u32;
        if l == 0 || l > br.cnt {
            return Err("無效的 Huffman 碼或資料提前結束".into());
        }
        br.buf >>= l;
        br.cnt -= l;
        Ok((e >> 4) as usize)
    }
}

const LBASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEXT: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DBASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DEXT: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

fn inflate_codes(br: &mut Bits, lit: &Huff, dist: &Huff, out: &mut Vec<u8>, max_out: usize) -> Result<(), String> {
    loop {
        let sym = lit.decode(br)?;
        if sym < 256 {
            if out.len() >= max_out {
                return Err("解壓縮結果超過預期大小".into());
            }
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let li = sym - 257;
            if li >= 29 {
                return Err("無效的長度符號".into());
            }
            let len = LBASE[li] as usize + br.bits(LEXT[li] as u32)? as usize;
            let ds = dist.decode(br)?;
            if ds >= 30 {
                return Err("無效的距離符號".into());
            }
            let d = DBASE[ds] as usize + br.bits(DEXT[ds] as u32)? as usize;
            if d > out.len() {
                return Err("距離超出已輸出的資料".into());
            }
            if out.len() + len > max_out {
                return Err("解壓縮結果超過預期大小".into());
            }
            let start = out.len() - d;
            if d >= len {
                out.extend_from_within(start..start + len);
            } else {
                for i in 0..len {
                    let b = out[start + i];
                    out.push(b);
                }
            }
        }
    }
}

fn fixed_tables() -> Result<(Huff, Huff), String> {
    let mut l = [0u8; 288];
    for (i, v) in l.iter_mut().enumerate() {
        *v = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    Ok((Huff::build(&l)?, Huff::build(&[5u8; 30])?))
}

/// 解 raw deflate。回傳輸出與位元流結束的位元組位置。`max_out` 防止解壓縮炸彈。
fn inflate(data: &[u8], max_out: usize) -> Result<(Vec<u8>, usize), String> {
    let mut br = Bits::new(data);
    let mut out: Vec<u8> = Vec::with_capacity(max_out.min(1 << 26));
    let mut fixed: Option<(Huff, Huff)> = None;
    loop {
        let last = br.bits(1)?;
        match br.bits(2)? {
            0 => {
                br.align();
                let len = br.bits(16)? as usize;
                let nlen = br.bits(16)? as usize;
                if len != (!nlen & 0xFFFF) {
                    return Err("stored 區塊的 LEN 與 NLEN 不符".into());
                }
                if out.len() + len > max_out {
                    return Err("解壓縮結果超過預期大小".into());
                }
                br.take_bytes(len, &mut out)?;
            }
            1 => {
                if fixed.is_none() {
                    fixed = Some(fixed_tables()?);
                }
                let (l, d) = fixed.as_ref().unwrap();
                inflate_codes(&mut br, l, d, &mut out, max_out)?;
            }
            2 => {
                let hlit = br.bits(5)? as usize + 257;
                let hdist = br.bits(5)? as usize + 1;
                let hclen = br.bits(4)? as usize + 4;
                if hlit > 286 || hdist > 30 {
                    return Err("動態區塊的碼數超過上限".into());
                }
                let mut cl = [0u8; 19];
                for &o in CL_ORDER.iter().take(hclen) {
                    cl[o] = br.bits(3)? as u8;
                }
                let clh = Huff::build(&cl)?;
                let mut lens = [0u8; 286 + 30];
                let mut i = 0;
                while i < hlit + hdist {
                    let sym = clh.decode(&mut br)?;
                    match sym {
                        0..=15 => {
                            lens[i] = sym as u8;
                            i += 1;
                        }
                        16 => {
                            if i == 0 {
                                return Err("重複碼長 16 前面沒有碼長".into());
                            }
                            let prev = lens[i - 1];
                            let n = 3 + br.bits(2)? as usize;
                            if i + n > hlit + hdist {
                                return Err("碼長重複超出範圍".into());
                            }
                            for _ in 0..n {
                                lens[i] = prev;
                                i += 1;
                            }
                        }
                        _ => {
                            let n = if sym == 17 { 3 + br.bits(3)? as usize } else { 11 + br.bits(7)? as usize };
                            if i + n > hlit + hdist {
                                return Err("碼長重複超出範圍".into());
                            }
                            i += n; // 已是 0
                        }
                    }
                }
                if lens[256] == 0 {
                    return Err("動態區塊沒有結束符號".into());
                }
                let lit = Huff::build(&lens[..hlit])?;
                let dist = Huff::build(&lens[hlit..hlit + hdist])?;
                inflate_codes(&mut br, &lit, &dist, &mut out, max_out)?;
            }
            _ => return Err("無效的 deflate 區塊型別".into()),
        }
        if last == 1 {
            break;
        }
    }
    br.align();
    Ok((out, br.byte_pos()))
}

/// 解 zlib 容器：檢查標頭與 Adler-32。
fn zlib_decompress(data: &[u8], max_out: usize) -> Result<Vec<u8>, String> {
    if data.len() < 6 {
        return Err("zlib 資料太短".into());
    }
    let (cmf, flg) = (data[0], data[1]);
    if cmf & 0x0F != 8 {
        return Err("zlib 壓縮方法不是 deflate".into());
    }
    if (((cmf as u32) << 8) | flg as u32) % 31 != 0 {
        return Err("zlib 標頭檢查碼不符".into());
    }
    if flg & 0x20 != 0 {
        return Err("zlib 預設字典不支援".into());
    }
    let (out, end) = inflate(&data[2..], max_out)?;
    let at = 2 + end;
    let tail = data.get(at..at + 4).ok_or("zlib 缺少 Adler-32")?;
    let want = u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]);
    if adler32(&out) != want {
        return Err("zlib Adler-32 不符".into());
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// PNG
// ---------------------------------------------------------------------------

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (a as i32, b as i32, c as i32);
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// 就地還原濾波。`raw` 每列 `1 + rowbytes`，結果寫進 `out`（每列 `rowbytes`）。
fn unfilter(raw: &[u8], h: usize, rowbytes: usize, bpp: usize) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; h * rowbytes];
    for y in 0..h {
        let ft = raw[y * (rowbytes + 1)];
        let src = &raw[y * (rowbytes + 1) + 1..(y + 1) * (rowbytes + 1)];
        let (before, rest) = out.split_at_mut(y * rowbytes);
        let cur = &mut rest[..rowbytes];
        let prev: &[u8] = if y == 0 { &[] } else { &before[(y - 1) * rowbytes..] };
        match ft {
            0 => cur.copy_from_slice(src),
            1 => {
                for i in 0..rowbytes {
                    let a = if i >= bpp { cur[i - bpp] } else { 0 };
                    cur[i] = src[i].wrapping_add(a);
                }
            }
            2 => {
                for i in 0..rowbytes {
                    let b = if y > 0 { prev[i] } else { 0 };
                    cur[i] = src[i].wrapping_add(b);
                }
            }
            3 => {
                for i in 0..rowbytes {
                    let a = if i >= bpp { cur[i - bpp] as u32 } else { 0 };
                    let b = if y > 0 { prev[i] as u32 } else { 0 };
                    cur[i] = src[i].wrapping_add(((a + b) / 2) as u8);
                }
            }
            4 => {
                for i in 0..rowbytes {
                    let a = if i >= bpp { cur[i - bpp] } else { 0 };
                    let b = if y > 0 { prev[i] } else { 0 };
                    let c = if y > 0 && i >= bpp { prev[i - bpp] } else { 0 };
                    cur[i] = src[i].wrapping_add(paeth(a, b, c));
                }
            }
            _ => return Err(format!("第 {y} 列的濾波型別 {ft} 無效")),
        }
    }
    Ok(out)
}

/// 解碼 PNG。不支援的格式與損毀的資料回傳錯誤，不 panic。
pub fn decode(data: &[u8]) -> Result<Image, String> {
    if data.len() < 8 || data[..8] != SIG {
        return Err("不是 PNG（簽章不符）".into());
    }
    let mut pos = 8usize;
    let mut ihdr: Option<(usize, usize, u8)> = None;
    let mut plte: Vec<u8> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();
    let mut seen_end = false;
    while pos + 12 <= data.len() {
        let len = be32(&data[pos..]) as usize;
        let typ = &data[pos + 4..pos + 8];
        let body_end = pos.checked_add(8).and_then(|v| v.checked_add(len)).ok_or("區塊長度溢位")?;
        if body_end + 4 > data.len() {
            return Err("區塊超出檔案尾端".into());
        }
        let body = &data[pos + 8..body_end];
        if chunk_crc(typ, body) != be32(&data[body_end..]) {
            return Err(format!("區塊 {} 的 CRC 不符", String::from_utf8_lossy(typ)));
        }
        match typ {
            b"IHDR" => {
                if body.len() != 13 {
                    return Err("IHDR 長度不是 13".into());
                }
                let (w, h) = (be32(body) as usize, be32(&body[4..]) as usize);
                let (depth, ct, comp, filt, inter) = (body[8], body[9], body[10], body[11], body[12]);
                if w == 0 || h == 0 || w > 1 << 16 || h > 1 << 16 || w * h > 1 << 28 {
                    return Err(format!("影像尺寸 {w}x{h} 不合理"));
                }
                if depth != 8 {
                    return Err(format!("只支援 8 位元，檔案是 {depth} 位元"));
                }
                if !matches!(ct, 0 | 2 | 3 | 4 | 6) {
                    return Err(format!("無效的顏色型別 {ct}"));
                }
                if comp != 0 || filt != 0 {
                    return Err("壓縮或濾波方法不是 0".into());
                }
                if inter != 0 {
                    return Err("不支援交錯".into());
                }
                ihdr = Some((w, h, ct));
            }
            b"PLTE" => {
                if body.len() % 3 != 0 || body.len() > 768 {
                    return Err("PLTE 長度無效".into());
                }
                plte = body.to_vec();
            }
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => {
                seen_end = true;
                break;
            }
            _ => {}
        }
        pos = body_end + 4;
    }
    let (w, h, ct) = ihdr.ok_or("缺 IHDR")?;
    if !seen_end {
        return Err("缺 IEND".into());
    }
    if idat.is_empty() {
        return Err("缺 IDAT".into());
    }
    let bpp = match ct {
        0 | 3 => 1,
        4 => 2,
        2 => 3,
        _ => 4,
    };
    let rowbytes = w * bpp;
    let expect = h * (rowbytes + 1);
    let raw = zlib_decompress(&idat, expect)?;
    if raw.len() != expect {
        return Err(format!("解壓縮後 {} 位元組，預期 {expect}", raw.len()));
    }
    let px = unfilter(&raw, h, rowbytes, bpp)?;
    let mut rgba = vec![255u8; w * h * 4];
    match ct {
        6 => rgba.copy_from_slice(&px),
        2 => {
            // tRNS 的色鍵是三個 16 位元值，8 位元影像比對低位元組。
            let key = (trns.len() >= 6).then(|| [trns[1], trns[3], trns[5]]);
            for i in 0..w * h {
                let (r, g, b) = (px[3 * i], px[3 * i + 1], px[3 * i + 2]);
                rgba[4 * i..4 * i + 3].copy_from_slice(&[r, g, b]);
                if key == Some([r, g, b]) {
                    rgba[4 * i + 3] = 0;
                }
            }
        }
        0 => {
            let key = (trns.len() >= 2).then(|| trns[1]);
            for i in 0..w * h {
                let v = px[i];
                rgba[4 * i..4 * i + 3].copy_from_slice(&[v, v, v]);
                if key == Some(v) {
                    rgba[4 * i + 3] = 0;
                }
            }
        }
        4 => {
            for i in 0..w * h {
                let v = px[2 * i];
                rgba[4 * i..4 * i + 3].copy_from_slice(&[v, v, v]);
                rgba[4 * i + 3] = px[2 * i + 1];
            }
        }
        _ => {
            if plte.is_empty() {
                return Err("索引色影像缺 PLTE".into());
            }
            let n = plte.len() / 3;
            for i in 0..w * h {
                let k = px[i] as usize;
                if k >= n {
                    return Err(format!("索引 {k} 超出調色盤（{n} 項）"));
                }
                rgba[4 * i..4 * i + 3].copy_from_slice(&plte[3 * k..3 * k + 3]);
                rgba[4 * i + 3] = trns.get(k).copied().unwrap_or(255);
            }
        }
    }
    Ok(Image { w, h, rgba })
}

fn push_chunk(out: &mut Vec<u8>, typ: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(typ);
    out.extend_from_slice(body);
    out.extend_from_slice(&chunk_crc(typ, body).to_be_bytes());
}

/// 編碼 RGBA 成 PNG。zlib 用 stored 區塊，所以檔案約等於原始大小。
pub fn encode_rgba(w: usize, h: usize, rgba: &[u8]) -> Vec<u8> {
    assert_eq!(rgba.len(), w * h * 4, "RGBA 長度與尺寸不符");
    let rowbytes = w * 4;
    let mut raw = Vec::with_capacity(h * (rowbytes + 1));
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&rgba[y * rowbytes..(y + 1) * rowbytes]);
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    if chunks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(c) = chunks.next() {
        z.push(if chunks.peek().is_none() { 1 } else { 0 });
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = Vec::with_capacity(z.len() + 64);
    out.extend_from_slice(&SIG);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", &z);
    push_chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    /// 手寫的 PNG：一個 zlib 容器（stored 區塊）包 `raw`，濾波位元組已含在 `raw` 內。
    fn handmade(w: u32, h: u32, ct: u8, extra: &[(&[u8; 4], Vec<u8>)], raw: &[u8]) -> Vec<u8> {
        let mut z = vec![0x78, 0x01, 1];
        z.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
        z.extend_from_slice(raw);
        z.extend_from_slice(&adler32(raw).to_be_bytes());
        let mut out = SIG.to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, ct, 0, 0, 0]);
        push_chunk(&mut out, b"IHDR", &ihdr);
        for (t, b) in extra {
            push_chunk(&mut out, t, b);
        }
        push_chunk(&mut out, b"IDAT", &z);
        push_chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn crc_and_adler_known_values() {
        // "123456789" 的 CRC-32 是 CBF43926，Adler-32 是 091E01DE。
        assert_eq!(!crc32_update(0xFFFF_FFFF, b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"123456789"), 0x091E_01DE);
        // 空 IEND 區塊的 CRC 是固定值。
        assert_eq!(chunk_crc(b"IEND", &[]), 0xAE42_6082);
    }

    #[test]
    fn inflate_fixed_huffman() {
        // zlib.compress(b"a")
        let z = unhex("789c4b040000620062");
        assert_eq!(zlib_decompress(&z, 16).unwrap(), b"a");
    }

    const DYN_Z: &str = "78da4d528111c340085ac5d504dd7f85829a2669aff71145e09b097dfcd5533a34b27dc86462b0c3a91714851a4910a502b2da6d683e2c3ca6a069a25dc34022f78400e6d3bd4f9d80deaa06dbbbd10310ea6717c99b428bb8f71805e15a2785549b664d1fb434f76b6b648d274f876732b09a6827390c5e04468e483982e574ad42036a949d2b6287d75c7b6dde5a730f1e62adcde2b5fb58f0d6f09b6354e4fca4574f73cf75ec8e7f5e120967dbe6282de1dc4c217031622c2810ff0e297b4ce2ab1a1747474e6c76e6642922697fff155a16b23f97b295f5d3a1569ba495a8882132ef0fedf8dd41";
    const DYN_RAW: &str = "616162616261616261616161616461616265626165616461616161636162626162616161626161626161636261626264636165616163616261626362646162626261646562626162626563616162616161616163616161646161622063646161612065616161616262616161616265626262626120636463616161626161616161616161616161646261616161616465616261616161646161656261626162656462616161636263616163656463636361626161616161626561206565616161616162206462626361622063636261636163656161656361616120636164656261626161656262206164646161616162616161206161622061206262626161616163616263626162626361626161636262632061626262626162626562646561626564616161616161626320616362612065616561626564616162616161636162616161626261656365616161636161612063616120626261616261612062636164616461616220616162616161616161616361626161616161636261622061636162646162626561646362616161616163616161646462616161616161616565626165616161616262616261616161616161616262636263646161656163626164206263636162626463646220626261616161616462626262626163636262626163616161636163656261626263626261616163616261616162626261626161612061652061616365616161656162616265616362206261206261616261616161616461636461206320616161656261616161616461206161626161652063622020626361636163626161206162616163656162616261";

    /// 動態 Huffman 區塊：向量由 zlib 壓縮 600 位元組（BTYPE = 2）產生。完整原文以 hex 存放。
    #[test]
    fn inflate_dynamic_huffman_vector() {
        let z = unhex(DYN_Z);
        assert_eq!((z[2] >> 1) & 3, 2, "向量應該是動態區塊");
        let out = zlib_decompress(&z, 600).unwrap();
        assert_eq!(out, unhex(DYN_RAW));
    }

    #[test]
    fn inflate_rejects_corruption() {
        let mut z = unhex(DYN_Z);
        let n = z.len();
        z[n - 1] ^= 1; // Adler-32
        assert!(zlib_decompress(&z, 600).is_err());
        let mut z = unhex(DYN_Z);
        z.truncate(z.len() / 2);
        assert!(zlib_decompress(&z, 600).is_err());
        // 輸出上限比實際小
        assert!(zlib_decompress(&unhex(DYN_Z), 100).is_err());
    }

    #[test]
    fn rgba_all_filters() {
        // 2x5 的 RGBA，每列用不同濾波；期望值逐一手算。
        let px: [[u8; 8]; 5] = [
            [10, 20, 30, 255, 40, 50, 60, 128],
            [11, 22, 33, 250, 44, 55, 66, 120],
            [200, 100, 50, 25, 5, 15, 25, 35],
            [1, 2, 3, 4, 250, 251, 252, 253],
            [90, 91, 92, 93, 94, 95, 96, 97],
        ];
        let mut raw = Vec::new();
        // 0 none
        raw.push(0);
        raw.extend_from_slice(&px[0]);
        // 1 sub
        raw.push(1);
        let r = &px[1];
        raw.extend_from_slice(&r[..4]);
        for i in 4..8 {
            raw.push(r[i].wrapping_sub(r[i - 4]));
        }
        // 2 up
        raw.push(2);
        for i in 0..8 {
            raw.push(px[2][i].wrapping_sub(px[1][i]));
        }
        // 3 average
        raw.push(3);
        for i in 0..8 {
            let a = if i >= 4 { px[3][i - 4] as u32 } else { 0 };
            let b = px[2][i] as u32;
            raw.push(px[3][i].wrapping_sub(((a + b) / 2) as u8));
        }
        // 4 paeth
        raw.push(4);
        for i in 0..8 {
            let a = if i >= 4 { px[4][i - 4] } else { 0 };
            let b = px[3][i];
            let c = if i >= 4 { px[3][i - 4] } else { 0 };
            raw.push(px[4][i].wrapping_sub(paeth(a, b, c)));
        }
        let png = handmade(2, 5, 6, &[], &raw);
        let img = decode(&png).unwrap();
        assert_eq!((img.w, img.h), (2, 5));
        let want: Vec<u8> = px.iter().flatten().copied().collect();
        assert_eq!(img.rgba, want);
    }

    #[test]
    fn rgb_gray_graya_indexed() {
        // RGB
        let img = decode(&handmade(2, 1, 2, &[], &[0, 1, 2, 3, 4, 5, 6])).unwrap();
        assert_eq!(img.rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        // 灰階
        let img = decode(&handmade(3, 1, 0, &[], &[0, 7, 8, 9])).unwrap();
        assert_eq!(img.rgba, vec![7, 7, 7, 255, 8, 8, 8, 255, 9, 9, 9, 255]);
        // 灰階加 alpha
        let img = decode(&handmade(2, 1, 4, &[], &[0, 50, 100, 60, 0])).unwrap();
        assert_eq!(img.rgba, vec![50, 50, 50, 100, 60, 60, 60, 0]);
        // 索引色加 tRNS（只給第 0 項 alpha）
        let plte = (b"PLTE", vec![255, 0, 0, 0, 255, 0]);
        let trns = (b"tRNS", vec![77]);
        let img = decode(&handmade(2, 1, 3, &[(plte.0, plte.1), (trns.0, trns.1)], &[0, 0, 1])).unwrap();
        assert_eq!(img.rgba, vec![255, 0, 0, 77, 0, 255, 0, 255]);
    }

    #[test]
    fn rejects_unsupported_and_damaged() {
        assert!(decode(b"not a png").is_err());
        let good = handmade(1, 1, 6, &[], &[0, 1, 2, 3, 4]);
        assert!(decode(&good).is_ok());
        // 位元深度 16
        let mut bad = good.clone();
        bad[24] = 16;
        assert!(decode(&bad).is_err(), "CRC 或位元深度檢查要擋下");
        // 交錯
        let mut inter = SIG.to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 1]);
        push_chunk(&mut inter, b"IHDR", &ihdr);
        push_chunk(&mut inter, b"IEND", &[]);
        assert!(decode(&inter).unwrap_err().contains("交錯"));
        // 截斷
        assert!(decode(&good[..good.len() - 5]).is_err());
        // CRC 損毀
        let mut crc = good.clone();
        let n = crc.len();
        crc[n - 20] ^= 0xFF;
        assert!(decode(&crc).is_err());
        // 非法濾波
        assert!(decode(&handmade(1, 1, 6, &[], &[9, 1, 2, 3, 4])).is_err());
        // 資料長度不足
        assert!(decode(&handmade(2, 2, 6, &[], &[0, 1, 2, 3, 4])).is_err());
    }

    #[test]
    fn encode_roundtrip() {
        let (w, h) = (37usize, 23usize);
        let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i * 7 + i / 5) as u8).collect();
        let png = encode_rgba(w, h, &rgba);
        let img = decode(&png).unwrap();
        assert_eq!((img.w, img.h, img.rgba), (w, h, rgba));
        // 超過一個 stored 區塊（65535）
        let (w, h) = (300usize, 120usize);
        let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
        let img = decode(&encode_rgba(w, h, &rgba)).unwrap();
        assert_eq!(img.rgba, rgba);
    }
}
