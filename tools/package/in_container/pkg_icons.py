#!/usr/bin/env python3
"""發行包圖示：自己畫的簡單戰機圖案，不使用原版美術。只用標準庫，在 docker 內跑。

用法：pkg_icons.py <輸出目錄> <版本>
輸出：rs2-<N>.png（16 到 512）、rs2.ico、rs2.icns、rs2.rc（Windows 資源：圖示與版本資訊）。
"""
import struct
import sys
import zlib
from pathlib import Path

SS = 4  # 超取樣倍率

# 圖案以 0..1 的座標描述，所有尺寸共用。
BG_TOP = (0x0B, 0x12, 0x30)
BG_BOTTOM = (0x1E, 0x2F, 0x6B)
SHIP = [(0.50, 0.14), (0.585, 0.40), (0.86, 0.68), (0.86, 0.77), (0.61, 0.68), (0.565, 0.80), (0.435, 0.80), (0.39, 0.68),
        (0.14, 0.77), (0.14, 0.68), (0.415, 0.40)]
SHIP_SHADE = [(0.50, 0.14), (0.585, 0.40), (0.86, 0.68), (0.86, 0.77), (0.61, 0.68), (0.565, 0.80), (0.50, 0.80)]
COCKPIT = [(0.50, 0.30), (0.54, 0.43), (0.50, 0.50), (0.46, 0.43)]
FLAME = [(0.455, 0.80), (0.545, 0.80), (0.50, 0.95)]
FLAME_CORE = [(0.48, 0.80), (0.52, 0.80), (0.50, 0.88)]
STARS = [(0.14, 0.16, 0.012), (0.82, 0.12, 0.016), (0.88, 0.40, 0.010), (0.10, 0.50, 0.012), (0.24, 0.32, 0.009), (0.74, 0.30, 0.009)]


def fill_poly(buf, n, pts, rgba):
    """掃描線填多邊形（偶奇規則）到 n*n 的 RGBA 緩衝，座標為像素。"""
    ys = [p[1] for p in pts]
    y0, y1 = max(0, int(min(ys))), min(n - 1, int(max(ys)) + 1)
    px = bytes(rgba)
    for y in range(y0, y1 + 1):
        yc = y + 0.5
        xs = []
        for i in range(len(pts)):
            (xa, ya), (xb, yb) = pts[i], pts[(i + 1) % len(pts)]
            if (ya <= yc < yb) or (yb <= yc < ya):
                xs.append(xa + (yc - ya) * (xb - xa) / (yb - ya))
        xs.sort()
        for j in range(0, len(xs) - 1, 2):
            a, b = max(0, int(xs[j] + 0.5)), min(n, int(xs[j + 1] + 0.5))
            if b > a:
                buf[(y * n + a) * 4:(y * n + b) * 4] = px * (b - a)


def fill_circle(buf, n, cx, cy, r, rgba):
    pts = []
    import math
    for k in range(24):
        a = 2 * math.pi * k / 24
        pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    fill_poly(buf, n, pts, rgba)


def rounded_rect_mask(n, radius):
    """回傳每列 (起, 迄) 的像素範圍，圓角矩形。"""
    rows = []
    for y in range(n):
        yc = y + 0.5
        if yc < radius:
            dy = radius - yc
        elif yc > n - radius:
            dy = yc - (n - radius)
        else:
            dy = 0
        dx = (radius * radius - dy * dy) ** 0.5 if dy < radius else 0
        rows.append((int(radius - dx + 0.5), int(n - radius + dx + 0.5)))
    return rows


def render(size):
    n = size * SS
    buf = bytearray(n * n * 4)
    rows = rounded_rect_mask(n, 0.2 * n)
    for y in range(n):
        t = y / (n - 1)
        c = bytes([int(BG_TOP[i] + (BG_BOTTOM[i] - BG_TOP[i]) * t) for i in range(3)] + [255])
        a, b = rows[y]
        if b > a:
            buf[(y * n + a) * 4:(y * n + b) * 4] = c * (b - a)
    sc = lambda pts: [(x * n, y * n) for x, y in pts]
    for x, y, r in STARS:
        fill_circle(buf, n, x * n, y * n, max(r * n, 1.0), (235, 240, 255, 255))
    fill_poly(buf, n, sc(FLAME), (255, 140, 30, 255))
    fill_poly(buf, n, sc(FLAME_CORE), (255, 235, 140, 255))
    fill_poly(buf, n, sc(SHIP), (206, 220, 240, 255))
    fill_poly(buf, n, sc(SHIP_SHADE), (150, 172, 208, 255))
    fill_poly(buf, n, sc(COCKPIT), (40, 190, 230, 255))
    # 箱型縮小。透明度用預乘加總，邊緣不會有黑邊。
    out = bytearray(size * size * 4)
    for y in range(size):
        acc = [[0, 0, 0, 0] for _ in range(size)]
        for sy in range(SS):
            row = buf[((y * SS + sy) * n) * 4:((y * SS + sy + 1) * n) * 4]
            for x in range(size):
                base = x * SS * 4
                a = acc[x]
                for sx in range(SS):
                    o = base + sx * 4
                    al = row[o + 3]
                    a[0] += row[o] * al
                    a[1] += row[o + 1] * al
                    a[2] += row[o + 2] * al
                    a[3] += al
        for x in range(size):
            r, g, b, a = acc[x]
            o = (y * size + x) * 4
            if a:
                out[o] = r // a
                out[o + 1] = g // a
                out[o + 2] = b // a
                out[o + 3] = a // (SS * SS)
    return bytes(out)


def png(size, rgba):
    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    raw = b"".join(b"\x00" + rgba[y * size * 4:(y + 1) * size * 4] for y in range(size))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def ico(entries):
    """entries: [(size, png_bytes, rgba)]。256 以下用 PNG 以外的 DIB 也可，這裡 256 用 PNG，其餘用 32 位元 DIB。"""
    head = struct.pack("<HHH", 0, 1, len(entries))
    body = b""
    dirs = b""
    off = 6 + 16 * len(entries)
    for size, pngb, rgba in entries:
        if size >= 256:
            data = pngb
        else:
            # BITMAPINFOHEADER，高度為兩倍（XOR 加 AND 遮罩），像素由下而上、BGRA。
            hdr = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, 0, 0, 0, 0, 0)
            xor = b"".join(
                b"".join(bytes([rgba[(y * size + x) * 4 + 2], rgba[(y * size + x) * 4 + 1], rgba[(y * size + x) * 4], rgba[(y * size + x) * 4 + 3]])
                         for x in range(size))
                for y in range(size - 1, -1, -1))
            mask_row = ((size + 31) // 32) * 4
            data = hdr + xor + b"\x00" * (mask_row * size)
        dirs += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), off)
        off += len(data)
        body += data
    return head + dirs + body


def icns(pngs):
    types = {16: b"icp4", 32: b"icp5", 64: b"icp6", 128: b"ic07", 256: b"ic08", 512: b"ic09"}
    body = b"".join(types[s] + struct.pack(">I", 8 + len(p)) + p for s, p in sorted(pngs.items()) if s in types)
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def rc(version):
    import re
    m = re.fullmatch(r"v\.(\d+)\.(\d+)\.(\d+)-(\d{8})", version)
    assert m, version
    a, b, c = m.group(1), m.group(2), m.group(3)
    return f'''1 ICON "rs2.ico"
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
FILEOS 0x4
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "ProductName", "RS-2 remake"
      VALUE "FileDescription", "RS-2 remake"
      VALUE "InternalName", "rs2-play"
      VALUE "OriginalFilename", "rs2-play.exe"
      VALUE "ProductVersion", "{version}"
      VALUE "FileVersion", "{version}"
      VALUE "LegalCopyright", "Copyright (c) 2026 Wang Chun-Yu (wicanr2). RRSAL-1.0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
'''


def main():
    out = Path(sys.argv[1])
    version = sys.argv[2]
    out.mkdir(parents=True, exist_ok=True)
    sizes = [16, 32, 48, 64, 128, 256, 512]
    rgba = {s: render(s) for s in sizes}
    pngs = {s: png(s, rgba[s]) for s in sizes}
    for s in sizes:
        (out / f"rs2-{s}.png").write_bytes(pngs[s])
    (out / "rs2.ico").write_bytes(ico([(s, pngs[s], rgba[s]) for s in (16, 32, 48, 256)]))
    (out / "rs2.icns").write_bytes(icns(pngs))
    (out / "rs2.rc").write_text(rc(version), encoding="ascii")
    print("icons:", ", ".join(f"{s}px" for s in sizes))


if __name__ == "__main__":
    main()
