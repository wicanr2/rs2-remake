#!/usr/bin/env python3
"""匯出破解組 intro（S.EXE 第一層的包裝碼，cs:ip = 0969:0100）的點陣、調色盤與字串。史料，不進 remake。

用法：extract_intro.py <S.layer1.bin> <輸出目錄>
位址：包裝碼的 cs 段在 S.layer1.bin 的位移 9690h，cs:X = 9690h + X（IDA 線性位址再加 10000h）。
規則見 docs/re/13-audio-and-misc.md。只用標準函式庫。

輸出：
  logo.png          280x95，16 色，EGA 調色盤取自 cs:0319（sub_1A15C 之後的載入）
  logo-ega-default.png  同一份資料套預設 EGA 調色盤，供比對
  scroll.txt        cs:0415 起的捲動字串，(byte - 6) xor 35h 解碼，0 結束
  banner.txt        cs:032C、cs:0354 兩段各 40 字元的橫幅字串（原樣明碼）
  intro.json        位移、大小、雜湊、調色盤、顏色循環表與解碼規則
"""
import hashlib
import json
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rs2_common import png_bytes, ega6_to_rgb  # noqa: E402

CS = 0x9690
LOGO_CS = 0x0E24        # sub_1A0AA 之後的 si 起點
LOGO_W_BYTES = 35       # rep movsb cx=23h
LOGO_H = 95             # cx=5Fh
PLANE_STRIDE = 0x0CFD   # = 35 * 95，1A0EE / 1A11A / 1A146 的 add si
PALETTE_CS = 0x0319     # 1A208: mov ah, [si+319h]，16 個 EGA 屬性值
CYCLE_CS = 0x037F       # 1A25B: mov si, 37Fh，每步取 4 個值給屬性 1、2、4、8，k = 0..31，共需 35 個位元組
BANNER1_CS = 0x032C     # 1A1D6: mov dx, 32Ch
BANNER2_CS = 0x0354     # 1A1DF: mov dx, 354h
SCROLL_CS = 0x0415      # 19EF8: mov al, [si+415h]


def sha(b):
    return hashlib.sha256(b).hexdigest()


def decode_scroll(img):
    out = bytearray()
    i = SCROLL_CS
    while True:
        v = ((img[CS + i] - 6) & 0xFF) ^ 0x35
        if v == 0:
            break
        out.append(v)
        i += 1
    return bytes(out), SCROLL_CS, i


def main(argv):
    src, dst = argv[1], argv[2]
    img = open(src, "rb").read()
    if len(img) != 55464:
        raise SystemExit("S.layer1.bin 大小不符，預期 55464")
    # 包裝碼入口特徵：cs:0100 是 cli; jmp，位元組 FA E9 BD 02
    if img[CS + 0x100:CS + 0x104] != bytes.fromhex("fae9bd02"):
        raise SystemExit("0969:0100 的位元組不符，映像版本可能不同")
    os.makedirs(dst, exist_ok=True)

    # 點陣：4 個平面連續存放，第 k 個區塊對應 sequencer map mask 1 << k（1A0C0..1A154 的順序）
    base = CS + LOGO_CS
    need = 4 * PLANE_STRIDE
    if base + need != len(img):
        raise SystemExit("點陣區塊沒有剛好貼齊檔尾：%d != %d" % (base + need, len(img)))
    planes = [img[base + k * PLANE_STRIDE: base + (k + 1) * PLANE_STRIDE] for k in range(4)]
    rows = []
    for y in range(LOGO_H):
        row = bytearray(LOGO_W_BYTES * 8)
        for x in range(LOGO_W_BYTES * 8):
            byte = y * LOGO_W_BYTES + (x >> 3)
            bit = 7 - (x & 7)
            v = 0
            for k in range(4):
                v |= ((planes[k][byte] >> bit) & 1) << k
            row[x] = v
        rows.append(row)

    pal_ega = list(img[CS + PALETTE_CS: CS + PALETTE_CS + 16])
    pal_rgb = ega6_to_rgb(pal_ega)
    with open(os.path.join(dst, "logo.png"), "wb") as fh:
        fh.write(png_bytes(LOGO_W_BYTES * 8, LOGO_H, rows, palette=pal_rgb))
    default_ega = [0, 1, 2, 3, 4, 5, 20, 7, 56, 57, 58, 59, 60, 61, 62, 63]   # BIOS 預設屬性暫存器
    with open(os.path.join(dst, "logo-ega-default.png"), "wb") as fh:
        fh.write(png_bytes(LOGO_W_BYTES * 8, LOGO_H, rows, palette=ega6_to_rgb(default_ega)))

    scroll, s0, s1 = decode_scroll(img)
    with open(os.path.join(dst, "scroll.txt"), "w", encoding="utf-8") as fh:
        fh.write(scroll.decode("ascii") + "\n")

    b1 = img[CS + BANNER1_CS: CS + BANNER1_CS + 40]
    b2 = img[CS + BANNER2_CS: CS + BANNER2_CS + 40]
    with open(os.path.join(dst, "banner.txt"), "w", encoding="utf-8") as fh:
        fh.write(b1.decode("ascii") + "\n" + b2.decode("ascii") + "\n")

    cycle = list(img[CS + CYCLE_CS: CS + CYCLE_CS + 35])
    info = {
        "input": {"file": os.path.basename(src), "sha256": sha(img), "size": len(img)},
        "address_note": "cs = 0969，cs:X 在 S.layer1.bin 位移 9690h + X；IDA 線性位址 = 10000h + 9690h + X",
        "logo": {
            "cs_offset": "cs:%04X" % LOGO_CS, "file_offset": base, "ida_linear": 0x10000 + base,
            "size_bytes": need, "sha256": sha(img[base:base + need]),
            "width_px": LOGO_W_BYTES * 8, "height_px": LOGO_H,
            "format": "4 個連續平面，每平面 35 位元組 x 95 列，間隔 0CFDh；平面 k 對應 sequencer map mask 1<<k；"
                      "每位元組高位元在左；像素值 = p0 | p1<<1 | p2<<2 | p3<<3",
            "evidence": "1A0AA..1A154 四段 rep movsb：map mask 102h/202h/402h/802h，si 起點 E24h、+0CFDh、+19FAh、+26F7h",
        },
        "palette_ega6": {"cs_offset": "cs:%04X" % PALETTE_CS, "values": pal_ega,
                         "rgb": [list(pal_rgb[i * 3:i * 3 + 3]) for i in range(16)],
                         "evidence": "1A1FD..1A21B 迴圈把 16 個值依序寫進屬性暫存器 0..15"},
        "color_cycle": {"cs_offset": "cs:%04X" % CYCLE_CS, "values": cycle,
                        "rule": "每 5 圈主迴圈步進一格（19A0E 計到 5，每圈等一次垂直回掃），一格從 cs:037F+k 取 4 個值寫屬性暫存器 1、2、4、8，k = 0..31 循環",
                        "evidence": "1A241..1A282"},
        "banners": {"cs_offsets": ["cs:%04X" % BANNER1_CS, "cs:%04X" % BANNER2_CS],
                    "text": [b1.decode("ascii"), b2.decode("ascii")],
                    "note": "各 40 字元，sub_19DDA 用 F000:FA6E 的 BIOS 8x8 ROM 字型逐字元取點陣，橫幅畫面依賴機器 ROM，這裡不重繪"},
        "scroll": {"cs_offset": "cs:%04X" % s0, "end_cs_offset": "cs:%04X" % s1, "length": len(scroll),
                   "decode": "plain = ((byte - 6) mod 256) xor 35h，plain == 0 結束；sub_19ED9 與 sub_1A35C 各有一份相同解碼",
                   "sha256": sha(scroll)},
        "draw_routines": {
            "sub_19D68": "以 map mask 依序對 4 個平面填入 bl、bh、bl(si 低位元組)、bh(si 高位元組) 的 1 位元組圖樣，cx 個位元組；用來畫背景橫條",
            "sub_19E12": "把 cs:0E20h 的 40x8 位元組橫幅緩衝區用 XOR 寫入平面 1、2，每列間隔 0A0h，共 8 列",
            "sub_19DDA": "由 40 字元字串與 BIOS ROM 字型（F000:FA6E + 字元*8）建出 cs:0E20h 的 40x8 緩衝區",
            "sub_19ED9": "捲動字串的逐字元取點陣與位元左移，右進左出",
        },
    }
    with open(os.path.join(dst, "intro.json"), "w", encoding="utf-8") as fh:
        json.dump(info, fh, ensure_ascii=False, indent=1)
    print("logo", LOGO_W_BYTES * 8, "x", LOGO_H, "scroll", len(scroll), "chars", "end cs:%04X" % s1)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
