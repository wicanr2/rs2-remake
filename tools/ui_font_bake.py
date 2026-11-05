#!/usr/bin/env python3
"""把介面字串用到的字烘成點陣，輸出 crates/rs2-play/assets/ui-font.bin。

在 rs2-hdtools:r1 映像（Pillow 11.3.0）內跑，離線：

  docker run --rm --network none -u "$(id -u):$(id -g)" --memory 1g --cpus 2 --pids-limit 64 \\
    --log-opt max-size=10m --log-opt max-file=3 -e PYTHONDONTWRITEBYTECODE=1 \\
    -v "$PWD/tools:/tools:ro" -v "$PWD/crates/rs2-play:/play" \\
    -v "<NotoSansCJKtc-Regular.otf 所在目錄>:/font:ro" \\
    rs2-hdtools:r1 timeout 300 python -I /tools/ui_font_bake.py \\
      --font /font/NotoSansCJKtc-Regular.otf --text-rs /play/src/ui/text.rs --out /play/assets/ui-font.bin

字集：ASCII 可印字元，加上 `src/ui/text.rs` 裡全部字串與字元常值（含 `EXTRA_CHARS`）的每個字。
`#[cfg(test)]` 之後的內容不掃。字型沒有某個字時中止，不輸出缺字方塊。

來源字型：Noto Sans CJK TC Regular，SIL OFL 1.1，
  https://github.com/notofonts/noto-cjk 的 f8d157532fbfaeda587e826d4cd5b21a49186f7c
  Sans/OTF/TraditionalChinese/NotoSansCJKtc-Regular.otf，SHA-256 dce08bd4fd91aa8aa76ed8fea4b694c2dfb8550f67871e326843212ddbeb88b4。
授權全文在 crates/rs2-play/assets/OFL.txt，發行包的 THIRD-PARTY-NOTICES.txt 會帶。

檔案格式（全部小端）：
  "RS2F" | 版本 u8 = 1 | 字高數 n u8 | 字數 m u16
  m 個 u32：Unicode 碼位，由小到大
  n 段（字高由小到大），每段：
    字高 px u8 | 基線到字框頂端的距離 ascent u8 | 保留 u16 | 點陣資料長度 u32
    m 筆字符資料：進位 adv u8、左側 bx i8、基線上方 by i8、寬 w u8、高 h u8、資料位移 off u32
    點陣資料：每個字符 w*h 個 8 位元 alpha，列優先，以 RLE 壓縮
  RLE：控制位元組 c < 0x80 是 c+1 個 0；c >= 0x80 是接著 (c-0x80+1) 個原樣位元組。
  字框：邊長 px，baseline 在字框頂端往下 ascent。CJK 字符的筆畫落在字框內。
"""
import argparse
import hashlib
import struct
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

SIZES = (16, 20, 24, 32)
# Noto Sans CJK 的表意字框：基線在字框底端往上 0.12 em，頂端在基線上方 0.88 em。烘字時用實測驗證。
ASCENT_RATIO = 0.88
NOTDEF_PROBE = 0x10FFFD


def string_literals(src: str):
    """依序列出 Rust 原始碼裡的字串與字元常值（已還原跳脫）。略過註解。不支援 raw string。"""
    out = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j + 1
        elif src.startswith("/*", i):
            j = src.find("*/", i + 2)
            i = n if j < 0 else j + 2
        elif c == "r" and (src.startswith('r"', i) or src.startswith('r#"', i)) and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            raise SystemExit("text.rs 不能用 raw string（掃描器不支援）")
        elif c == '"':
            s, i = read_escaped(src, i + 1, '"')
            out.append(s)
        elif c == "'":
            if i + 1 < n and src[i + 1] == "\\":
                s, i = read_escaped(src, i + 1, "'")
                out.append(s)
            elif i + 2 < n and src[i + 2] == "'":
                out.append(src[i + 1])
                i += 3
            else:
                i += 1  # 生命週期
        else:
            i += 1
    return out


def read_escaped(src: str, i: int, quote: str):
    buf = []
    n = len(src)
    while i < n:
        c = src[i]
        if c == quote:
            return "".join(buf), i + 1
        if c == "\\":
            i += 1
            e = src[i]
            if e == "n":
                buf.append("\n")
            elif e == "r":
                buf.append("\r")
            elif e == "t":
                buf.append("\t")
            elif e in "\\\"'":
                buf.append(e)
            elif e == "0":
                buf.append("\0")
            elif e == "u":
                j = src.index("}", i)
                buf.append(chr(int(src[i + 2 : j], 16)))
                i = j
            elif e == "\n":
                while i + 1 < n and src[i + 1] in " \t\r\n":
                    i += 1
            else:
                raise SystemExit(f"不認得的跳脫 \\{e}")
            i += 1
        else:
            buf.append(c)
            i += 1
    raise SystemExit("字串沒有結尾")


def charset(text_rs: Path):
    src = text_rs.read_text(encoding="utf-8")
    cut = src.find("#[cfg(test)]")
    if cut >= 0:
        src = src[:cut]
    chars = {chr(c) for c in range(0x20, 0x7F)}
    for s in string_literals(src):
        for ch in s:
            if ch >= " " and ch != "\x7f":
                chars.add(ch)
    return sorted(chars, key=ord)


def draw_glyph(font, ch, size):
    """回傳 (進位, 左側, 基線上方, 寬, 高, alpha 位元組)。空白字符寬高為 0。"""
    pad = size
    base = size * 2
    im = Image.new("L", (size * 3, size * 3), 0)
    ImageDraw.Draw(im).text((pad, base), ch, font=font, fill=255, anchor="ls")
    adv = int(round(font.getlength(ch)))
    box = im.getbbox()
    if box is None:
        return adv, 0, 0, 0, 0, b""
    l, t, r, b = box
    return adv, l - pad, base - t, r - l, b - t, im.crop(box).tobytes()


def rle(data: bytes) -> bytes:
    out = bytearray()
    i, n = 0, len(data)
    lit = bytearray()

    def flush():
        while lit:
            chunk = lit[:128]
            del lit[:128]
            out.append(0x80 + len(chunk) - 1)
            out.extend(chunk)

    while i < n:
        if data[i] == 0:
            j = i
            while j < n and data[j] == 0:
                j += 1
            run = j - i
            if run >= 2 or not lit:
                flush()
                while run > 0:
                    k = min(run, 128)
                    out.append(k - 1)
                    run -= k
                i = j
                continue
        lit.append(data[i])
        i += 1
    flush()
    return bytes(out)


def unrle(data: bytes, total: int) -> bytes:
    out = bytearray()
    i = 0
    while len(out) < total:
        c = data[i]
        i += 1
        if c < 0x80:
            out.extend(b"\0" * (c + 1))
        else:
            k = c - 0x80 + 1
            out.extend(data[i : i + k])
            i += k
    assert len(out) == total, (len(out), total)
    return bytes(out)


def check_em_box(font_path: Path):
    """用常見的方正字量字框：筆畫上下緣離基線的距離要落在 [-0.12 em, 0.88 em] 之內，且上下大致對稱。"""
    size = 200
    font = ImageFont.truetype(str(font_path), size)
    tops, bottoms = [], []
    for ch in "國鑫龜口永田":
        _, _, by, _, h, _ = draw_glyph(font, ch, size)
        tops.append(by)
        bottoms.append(by - h)
    top, bottom = max(tops), min(bottoms)
    print(f"字框實測（200px）：墨跡最高在基線上 {top} px（{top / size:.3f} em），最低在基線上 {bottom} px（{bottom / size:.3f} em）")
    assert 0.80 <= top / size <= 0.90, "字框頂端與預期的 0.88 em 差太多"
    assert -0.14 <= bottom / size <= -0.04, "字框底端與預期的 -0.12 em 差太多"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--font", required=True, type=Path)
    ap.add_argument("--text-rs", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--sizes", default=",".join(map(str, SIZES)))
    a = ap.parse_args()
    sizes = sorted({int(s) for s in a.sizes.split(",")})
    if not a.font.is_file() or not a.text_rs.is_file():
        raise SystemExit("缺字型檔或 text.rs")
    font_sha = hashlib.sha256(a.font.read_bytes()).hexdigest()
    chars = charset(a.text_rs)
    print(f"字型 {a.font.name} sha256 {font_sha[:16]}…，字集 {len(chars)} 字（其中非 ASCII {sum(1 for c in chars if ord(c) > 0x7E)}）")
    check_em_box(a.font)

    sections = []
    for size in sizes:
        font = ImageFont.truetype(str(a.font), size)
        notdef = draw_glyph(font, chr(NOTDEF_PROBE), size)
        recs, blob = [], bytearray()
        missing = []
        for ch in chars:
            adv, bx, by, w, h, alpha = draw_glyph(font, ch, size)
            if ch.strip() and (adv, bx, by, w, h, alpha) == notdef:
                missing.append(ch)
                continue
            assert 0 <= adv < 256 and -128 <= bx < 128 and -128 <= by < 128 and w < 256 and h < 256, (ch, adv, bx, by, w, h)
            recs.append((adv, bx, by, w, h, len(blob)))
            blob.extend(rle(alpha))
            if alpha:
                assert unrle(rle(alpha), len(alpha)) == alpha
        if missing:
            raise SystemExit(f"字型沒有這些字（{size}px）：{''.join(missing)}")
        sections.append((size, int(round(size * ASCENT_RATIO)), recs, bytes(blob)))

    out = bytearray(b"RS2F")
    out += struct.pack("<BBH", 1, len(sections), len(chars))
    for ch in chars:
        out += struct.pack("<I", ord(ch))
    for size, ascent, recs, blob in sections:
        out += struct.pack("<BBHI", size, ascent, 0, len(blob))
        for adv, bx, by, w, h, off in recs:
            out += struct.pack("<BbbBBI", adv, bx, by, w, h, off)
        out += blob
    a.out.parent.mkdir(parents=True, exist_ok=True)
    tmp = a.out.with_suffix(".tmp")
    tmp.write_bytes(bytes(out))
    tmp.replace(a.out)
    print(f"寫出 {a.out}：{len(out)} 位元組，字高 {sizes}，sha256 {hashlib.sha256(out).hexdigest()}")
    for size, _, _, blob in sections:
        print(f"  {size}px 點陣資料 {len(blob)} 位元組")


if __name__ == "__main__":
    sys.exit(main())
