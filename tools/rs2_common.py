"""RS-2 原版資料的共用解碼函式。只用標準函式庫，固定在 python:3.12-slim 容器內執行。

格式依據見 docs/re/02-file-formats.md：
  解壓對照 S.EXE 解包後 sub_1C757（1C757h）
  副檔名修補對照 sub_1C738（1C738h）
"""
import struct
import zlib


def unrle(buf, pos=0):
    """解開一段 [u16 壓縮長度][控制碼串流]。

    對照 sub_1C757：控制碼 <0x80 表示「下一位元組重複 n 次」，>=0x80 表示「複製 n&0x7F 個原值」。
    結束條件只看來源指標是否到達 pos+2+size，所以最後一段原值可能越界讀到區段之外。
    回傳 (解開的資料, 實際讀到的來源位置, 宣告結束位置)。
    """
    size = struct.unpack_from("<H", buf, pos)[0]
    p = pos + 2
    end = p + size
    out = bytearray()
    # 單獨成檔時 size 含表頭 2 位元組，原版會多讀 2 位元組到檔尾之外（緩衝區殘值）。
    # 這些殘值落在哨兵之後，不影響圖片，所以這裡遇到檔尾就停，不去猜殘值內容。
    limit = min(end, len(buf))
    while p < limit:
        n = buf[p]
        p += 1
        if n < 0x80:
            if p >= len(buf):
                break
            out += bytes([buf[p]]) * n
            p += 1
        else:
            k = n & 0x7F
            out += buf[p:p + k]
            p += k
    return bytes(out), p, end


def parse_bank(raw):
    """解析解壓後的圖庫。第一個 u16 同時是第一張圖的位移與「項目數 x 2」。

    最後一項一定是 FF FF 00 00 哨兵，不算圖片。
    每張圖：u16 高、u16 每列位元組數，後接 高x寬 位元組。
    """
    first = struct.unpack_from("<H", raw, 0)[0]
    count = first // 2
    offs = [struct.unpack_from("<H", raw, 2 * i)[0] for i in range(count)]
    items = []
    for i, o in enumerate(offs):
        h, w = struct.unpack_from("<HH", raw, o)
        if h == 0xFFFF:
            if i != count - 1:
                raise ValueError(f"哨兵出現在第 {i} 項，總數 {count}")
            break
        data = raw[o + 4:o + 4 + h * w]
        if len(data) != h * w:
            raise ValueError(f"第 {i} 項資料不足：{len(data)} < {h * w}")
        nxt = offs[i + 1]
        if nxt - o != 4 + h * w:
            raise ValueError(f"第 {i} 項長度不符：{nxt - o} != {4 + h * w}")
        items.append({"index": i, "offset": o, "height": h, "stride": w, "data": data})
    return items


def go_sections(buf):
    """讀 .GO 的 5 字組表頭。第 5 字組等於檔長時只有 4 區段，否則是第 5 區段起點。"""
    tab = struct.unpack_from("<5H", buf, 0)
    if tab[4] >= len(buf):
        starts = list(tab[:4])
    else:
        starts = list(tab)
    return starts


def png_bytes(width, height, rows, palette=None, rgba=False):
    """rows 是逐列位元組。palette 給定時輸出索引色 PNG，rgba=True 時輸出 RGBA。"""
    def chunk(t, d):
        c = zlib.crc32(t + d) & 0xFFFFFFFF
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", c)
    if rgba:
        ct = 6
    elif palette is not None:
        ct = 3
    else:
        ct = 0
    ihdr = struct.pack(">IIBBBBB", width, height, 8, ct, 0, 0, 0)
    raw = b"".join(b"\0" + bytes(r) for r in rows)
    out = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
    if palette is not None and not rgba:
        out += chunk(b"PLTE", bytes(palette))
    out += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    return out


def png_rgb(width, height, rgb):
    """rgb 是連續的 RGB 位元組。"""
    def chunk(t, d):
        c = zlib.crc32(t + d) & 0xFFFFFFFF
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", c)
    stride = width * 3
    raw = b"".join(b"\0" + bytes(rgb[y * stride:(y + 1) * stride]) for y in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)) +
            chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def planar_to_indexed(planes, height, stride):
    """4 個 1bpp 平面 → 每像素一個位元組的索引值。

    planes 依檔案區段順序傳入。第一個區段是最高位：sub_18C0A 先以 bl=1 呼叫 sub_1882E，
    而 sub_1882E 把 bl=1 對應到 map mask 8（plane 3），依序到 bl=8 對應 map mask 1（plane 0）。
    """
    w = stride * 8
    rows = []
    for y in range(height):
        row = bytearray(w)
        off = y * stride
        for x in range(w):
            byte = off + (x >> 3)
            bit = 7 - (x & 7)
            v = 0
            for k in range(4):
                v |= ((planes[k][byte] >> bit) & 1) << (3 - k)
            row[x] = v
        rows.append(row)
    return rows


def mono_to_rows(data, height, stride):
    w = stride * 8
    rows = []
    for y in range(height):
        row = bytearray(w)
        off = y * stride
        for x in range(w):
            row[x] = (data[off + (x >> 3)] >> (7 - (x & 7))) & 1
        rows.append(row)
    return rows


def dac6_to_rgb(table):
    """VGA DAC 6-bit RGB → 8-bit。"""
    out = bytearray()
    for v in table:
        out.append((v * 255 + 31) // 63)
    return bytes(out)


def ega6_to_rgb(attr):
    """EGA 屬性值 (rgbRGB 六位元) → 8-bit RGB。"""
    out = bytearray()
    for c in attr:
        r = ((c >> 2) & 1) * 2 + ((c >> 5) & 1)
        g = ((c >> 1) & 1) * 2 + ((c >> 4) & 1)
        b = (c & 1) * 2 + ((c >> 3) & 1)
        out += bytes([r * 85, g * 85, b * 85])
    return bytes(out)
