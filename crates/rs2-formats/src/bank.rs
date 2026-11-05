//! RLE、圖庫與 `.GO` 區段，規則同 `tools/rs2_common.py`，對照 `sub_1C757`、`sub_18AA4`。

/// 解開一段 `[u16 長度][控制碼串流]`。控制碼 `< 80h` 重複下一位元組 n 次，`>= 80h` 複製 `n & 7Fh` 個。
/// 單獨成檔時長度含表頭 2 位元組，原版會多讀到檔尾之外的緩衝區殘值，那些殘值落在哨兵之後，
/// 不影響圖片，所以這裡遇到檔尾就停。
pub fn unrle(buf: &[u8], pos: usize) -> Vec<u8> {
    let size = u16::from_le_bytes([buf[pos], buf[pos + 1]]) as usize;
    let mut p = pos + 2;
    let limit = (p + size).min(buf.len());
    let mut out = Vec::new();
    while p < limit {
        let n = buf[p] as usize;
        p += 1;
        if n < 0x80 {
            if p >= buf.len() {
                break;
            }
            out.resize(out.len() + n, buf[p]);
            p += 1;
        } else {
            let k = n & 0x7F;
            let end = (p + k).min(buf.len());
            out.extend_from_slice(&buf[p..end]);
            p += k;
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BankItem {
    pub height: u16,
    /// 每列位元組數。
    pub stride: u16,
    pub data: Vec<u8>,
}

/// 解析解壓後的圖庫。第一個 u16 同時是第一張圖的位移與「項目數 × 2」，尾端是 `FF FF 00 00` 哨兵。
pub fn parse_bank(raw: &[u8]) -> Result<Vec<BankItem>, String> {
    let rd = |at: usize| -> Result<u16, String> {
        raw.get(at..at + 2)
            .map(|s| u16::from_le_bytes([s[0], s[1]]))
            .ok_or_else(|| format!("圖庫在位移 {at} 資料不足"))
    };
    let count = rd(0)? as usize / 2;
    let mut offs = Vec::with_capacity(count);
    for i in 0..count {
        offs.push(rd(2 * i)? as usize);
    }
    let mut items = Vec::new();
    for (i, &o) in offs.iter().enumerate() {
        let h = rd(o)?;
        let w = rd(o + 2)?;
        if h == 0xFFFF {
            if i != count - 1 {
                return Err(format!("哨兵出現在第 {i} 項，總數 {count}"));
            }
            break;
        }
        let n = h as usize * w as usize;
        let data = raw.get(o + 4..o + 4 + n).ok_or_else(|| format!("第 {i} 項資料不足"))?;
        if i + 1 < count && offs[i + 1] - o != 4 + n {
            return Err(format!("第 {i} 項長度不符"));
        }
        items.push(BankItem { height: h, stride: w, data: data.to_vec() });
    }
    Ok(items)
}

/// `.GO`：4 或 5 個區段。5 個時第 0 段是遮罩。平面區段的第一個是 plane 3（最高位）。
#[derive(Clone, Debug)]
pub struct GoFile {
    pub mask: Option<Vec<BankItem>>,
    /// 依檔案順序：`planes[0]` 是 plane 3，`planes[3]` 是 plane 0。
    pub planes: [Vec<BankItem>; 4],
}

pub fn parse_go(buf: &[u8]) -> Result<GoFile, String> {
    if buf.len() < 10 {
        return Err("`.GO` 表頭不足".into());
    }
    let w = |i: usize| u16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]) as usize;
    let starts: Vec<usize> = if w(4) >= buf.len() { (0..4).map(w).collect() } else { (0..5).map(w).collect() };
    let mut banks = Vec::new();
    for s in &starts {
        banks.push(parse_bank(&unrle(buf, *s))?);
    }
    let mut it = banks.into_iter();
    let mask = if starts.len() == 5 { it.next() } else { None };
    let planes = [it.next().unwrap(), it.next().unwrap(), it.next().unwrap(), it.next().unwrap()];
    Ok(GoFile { mask, planes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_runs_and_literals() {
        // 長度 5：重複 3 次 AA，再複製 2 個原值。
        let buf = [5, 0, 3, 0xAA, 0x82, 1, 2];
        assert_eq!(unrle(&buf, 0), vec![0xAA, 0xAA, 0xAA, 1, 2]);
    }

    #[test]
    fn bank_with_sentinel() {
        // 兩項：一張 1×2 的圖加哨兵。偏移表 2 項（4 位元組）。
        let raw = [4, 0, 10, 0, 1, 0, 2, 0, 0xF0, 0x0F, 0xFF, 0xFF, 0, 0];
        let items = parse_bank(&raw).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].stride, 2);
        assert_eq!(items[0].data, vec![0xF0, 0x0F]);
    }
}
