#!/usr/bin/env python3
"""把 RS-2 全部圖形與資料解成可檢視檔案，輸出留在 workspace（不進版控）。

用法：extract_assets.py <原版目錄> <解包目錄> <輸出目錄>
任一區段結構驗證失敗就中止，不輸出部分結果。
"""
import hashlib
import json
import os
import struct
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rs2_common import (dac6_to_rgb, ega6_to_rgb, go_sections, mono_to_rows, parse_bank,
                        planar_to_indexed, png_bytes, png_rgb, unrle)

src, unp, dst = sys.argv[1], sys.argv[2], sys.argv[3]
DS = 0xCBC0  # dseg 段基底相對於載入映像起點的位移（IDA para 1CBCh，映像基底 1000h）
exe = open(os.path.join(unp, "S.unpacked.bin"), "rb").read()


def ds(o, n):
    return exe[DS + o:DS + o + n]


# (名稱, 17 位元組 EGA 屬性表位移)。VGA DAC 表緊接其後 48 位元組；C 表例外，見下。
PALETTES = {
    "stage1": (0x4EC, 0x4FD, "sub_110E3，第一關捲動開始"),
    "stage1_boss": (0x52D, 0x53E, "sub_11141，第一關捲軸計數 26Ch"),
    "loader": (0x56E, 0x5A9, "sub_11112，載入 MY.GO 後"),
    "title_d": (0x598, 0x5A9, "sub_1295B"),
    "title_e": (0x5D9, 0x5EA, "sub_12989"),
    "menu": (0x61A, 0x62B, "sub_129B7"),
    "beg1_a": (0x65B, 0x66C, "sub_129E5"),
    "beg1_b": (0x69C, 0x6AD, "sub_12A13"),
    "j1": (0x6DD, 0x6EE, "sub_12A41"),
    "j2": (0x71E, 0x72F, "sub_12A6F"),
    "j3": (0x75F, 0x770, "sub_12A9D"),
    "j4": (0x7A0, 0x7B1, "sub_12ACB"),
    "stage2": (0xC14, 0xC25, "sub_137F8"),
    "stage2_b": (0xC55, 0xC66, "sub_13827"),
    "stage2_d": (0xC96, 0xCA7, "sub_13894"),
    "stage2_c": (0xCD7, 0xCE8, "sub_13856"),
    "stage3": (0xD18, 0xD29, "sub_13D32"),
    "stage4": (0xD71, 0xD82, "sub_14063"),
    "ending_a": (0xE43, 0xE54, "sub_143EA"),
    "ending_b": (0xE84, 0xE95, "sub_14419"),
}
PAL = {}
for k, (a, d, why) in PALETTES.items():
    PAL[k] = {"vga": dac6_to_rgb(ds(d, 48)), "ega": ega6_to_rgb(ds(a, 16)), "attr_ds": a, "dac_ds": d, "source": why}

# 圖庫 → 預設預覽調色盤（依呼叫端組合，證據見 docs/re/02-file-formats.md）
GO_PAL = {"MY": "stage1", "SHT1": "stage1", "BH1": "stage1", "SHT2": "stage2", "BH2": "stage2",
          "SHT3": "stage3", "MAP": "stage3", "SHT4": "stage4", "BH4": "stage4", "SHT5": "ending_a",
          "J1": "j1", "J2": "j2", "J3": "j3", "J4": "j4", "BEG1": "beg1_a"}

# 同一庫內實際使用不同調色盤的索引。來源：sprite-usage.json 的呼叫點分析，並與實跑 DAC 比對（docs/re/10）。
PAL_OVERRIDE = {
    "beg1_title_e": ("BEG1", "title_e", [2, 3, 4, 5, 6, 14, 15, 16, 17]),
    "beg1_title_d": ("BEG1", "title_d", [0, 1]),
    "my_j1": ("MY", "j1", [61, 62, 63, 64]),
    "my_j2": ("MY", "j2", [65]),
    "my_j3": ("MY", "j3", [66]),
    "my_j4": ("MY", "j4", [67]),
    "my_title": ("MY", "title_d", [15, 47, 48]),
    "my_menu": ("MY", "menu", [73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85]),
    "sht1_boss": ("SHT1", "stage1_boss", [7, 17, 18, 19, 20, 21, 22, 23, 24, 29, 33, 34, 35]),
    "sht2_b": ("SHT2", "stage2_b", [5, 6, 7, 11, 12, 13, 14, 15, 18, 19, 20, 26, 27, 28, 32, 33]),
    "sht2_c": ("SHT2", "stage2_c", [23, 24, 25, 29, 30, 34, 35]),
    "sht5_b": ("SHT5", "ending_b", [0, 1, 2, 4]),
}
PAL_OF = {(lib, i): pal for lib, pal, ids in PAL_OVERRIDE.values() for i in ids}

FONT = {
    "0": "111 101 101 101 111", "1": "010 110 010 010 111", "2": "111 001 111 100 111",
    "3": "111 001 111 001 111", "4": "101 101 111 001 001", "5": "111 100 111 001 111",
    "6": "111 100 111 101 111", "7": "111 001 010 010 010", "8": "111 101 111 101 111",
    "9": "111 101 111 001 111",
}


def write(path, data):
    p = os.path.join(dst, path)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "wb") as fh:
        fh.write(data)
    return {"path": path, "sha256": hashlib.sha256(data).hexdigest(), "size": len(data)}


def label(canvas, W, x, y, text, color):
    for ch in text:
        rows = FONT[ch].split()
        for j, r in enumerate(rows):
            for i, c in enumerate(r):
                if c == "1":
                    canvas[(y + j) * W + x + i] = color
        x += 4


def sheet(items, pal, path, scale=1, maxw=960, bg=None):
    """以圖庫索引標籤排版。items: [(index, w, h, rows)]。用保留色 16=背景，17=標籤。"""
    W = maxw
    x = 4
    y = 4
    lh = 0
    layout = []
    for idx, w, h, rows in items:
        ww = w * scale
        hh = h * scale
        if x + max(ww, 16) + 4 > W:
            x = 4
            y += lh + 12
            lh = 0
        layout.append((idx, x, y, ww, hh, rows))
        x += max(ww, 16) + 6
        lh = max(lh, hh)
    H = y + lh + 14
    c = bytearray([16]) * (W * H)
    for idx, xx, yy, ww, hh, rows in layout:
        label(c, W, xx, yy, str(idx), 17)
        yy += 7
        for j in range(hh):
            r = rows[j // scale]
            for i in range(ww):
                c[(yy + j) * W + xx + i] = r[i // scale]
    pp = bytes(pal) + bytes(3 * 0)
    pp = pp[:48].ljust(48, b"\0") + bytes((64, 0, 64)) + bytes((255, 255, 0))
    return write(path, png_bytes(W, H, [c[i * W:(i + 1) * W] for i in range(H)], palette=pp))


def sheet_rgb(items, kind, path, scale=1, maxw=960):
    """每項可用不同調色盤的總覽圖，輸出 RGB PNG。"""
    W = maxw
    x = 4
    y = 4
    lh = 0
    layout = []
    for idx, w, h, rows, pname in items:
        ww = w * scale
        hh = h * scale
        if x + max(ww, 16) + 4 > W:
            x = 4
            y += lh + 12
            lh = 0
        layout.append((idx, x, y, ww, hh, rows, pname))
        x += max(ww, 16) + 6
        lh = max(lh, hh)
    H = y + lh + 14
    bg = bytes((64, 0, 64))
    fg = bytes((255, 255, 0))
    c = bytearray(bg * (W * H))
    for idx, xx, yy, ww, hh, rows, pname in layout:
        for ch_i, ch in enumerate(str(idx)):
            for j, r in enumerate(FONT[ch].split()):
                for i, v in enumerate(r):
                    if v == "1":
                        p = ((yy + j) * W + xx + ch_i * 4 + i) * 3
                        c[p:p + 3] = fg
        pal = PAL[pname][kind]
        yy += 7
        for j in range(hh):
            r = rows[j // scale]
            for i in range(ww):
                v = r[i // scale] * 3
                p = ((yy + j) * W + xx + i) * 3
                c[p:p + 3] = pal[v:v + 3]
    return write(path, png_rgb(W, H, c))


manifest = {"source_dir": os.path.basename(os.path.normpath(src)), "palettes": {}, "banks": {}, "other": {}, "files": []}
for k, v in PAL.items():
    manifest["palettes"][k] = {"vga_rgb_hex": v["vga"].hex(), "ega_rgb_hex": v["ega"].hex(),
                               "attr_ds": f"{v['attr_ds']:#x}", "dac_ds": f"{v['dac_ds']:#x}", "source": v["source"]}


def read(n):
    return open(os.path.join(src, n), "rb").read()


def decode(buf, pos=0):
    raw, p, e = unrle(buf, pos)
    return raw, parse_bank(raw), p - e


GROUPS = {}
for name in ("MY", "SHT1", "SHT2", "SHT3", "SHT4", "SHT5", "BH1", "BH2", "BH4", "J1", "J2", "J3", "J4", "MAP"):
    b = read(name + ".GO")
    starts = go_sections(b)
    sec = [decode(b, s) for s in starts]
    if len(starts) == 5:
        mask = sec[0]
        planes = sec[1:]
        mask_from = name + ".GO#0"
    else:
        planes = sec
        mask = None
        mask_from = None
    if name == "SHT4":
        mask = decode(read("SHT4.GOT"))
        mask_from = "SHT4.GOT"
    GROUPS[name] = {"planes": planes, "mask": mask, "mask_from": mask_from}
# BEG1：四個平面分別成檔
GROUPS["BEG1"] = {"planes": [decode(read(f"BEG1.BD{i}")) for i in (1, 2, 3, 4)], "mask": None, "mask_from": None}

for name, g in GROUPS.items():
    n = len(g["planes"][0][1])
    for k in range(4):
        if len(g["planes"][k][1]) != n:
            raise SystemExit(f"{name} 平面 {k} 項數不同")
    palname = GO_PAL[name]
    sheet_items = []
    entries = []
    os.makedirs(os.path.join(dst, "sprites", name), exist_ok=True)
    for i in range(n):
        it = [g["planes"][k][1][i] for k in range(4)]
        h, st = it[0]["height"], it[0]["stride"]
        for k in range(1, 4):
            if (it[k]["height"], it[k]["stride"]) != (h, st):
                raise SystemExit(f"{name} 第 {i} 項各平面尺寸不同")
        rows = planar_to_indexed([x["data"] for x in it], h, st)
        w = st * 8
        pname = PAL_OF.get((name, i), palname)
        pal = PAL[pname]["vga"]
        rec = write(f"sprites/{name}/{i:04d}.png", png_bytes(w, h, rows, palette=pal))
        entries.append({"index": i, "width": w, "height": h, "palette": pname, "png": rec["path"], "sha256": rec["sha256"]})
        # 總覽圖一張只能有一組調色盤，所以每項換算成 RGB 後再用 24-bit 排版
        sheet_items.append((i, w, h, rows, pname))
    mask_rec = []
    if g["mask"]:
        mi = g["mask"][1]
        mitems = []
        for it in mi:
            rows = mono_to_rows(it["data"], it["height"], it["stride"])
            mitems.append((it["index"], it["stride"] * 8, it["height"], [bytearray(1 if v else 15 for v in r) for r in rows]))
            rec = write(f"masks/{name}/{it['index']:04d}.png", png_bytes(it["stride"] * 8, it["height"],
                        [bytearray(255 if v else 0 for v in r) for r in rows]))
            mask_rec.append({"index": it["index"], "width": it["stride"] * 8, "height": it["height"], "png": rec["path"]})
        sheet(mitems, bytes((0, 0, 0)) * 16, f"sheets/{name}.mask.png")
    if name == "MAP":
        sc = 1
    else:
        sc = 2 if max(e["width"] for e in entries) <= 200 else 1
    r1 = sheet_rgb(sheet_items, "vga", f"sheets/{name}.vga.png", scale=sc, maxw=1280)
    r2 = sheet_rgb(sheet_items, "ega", f"sheets/{name}.ega.png", scale=sc, maxw=1280)
    manifest["banks"][name] = {"count": n, "palette": palname, "mask_source": g["mask_from"], "sprites": entries, "masks": mask_rec,
                               "sheets": [r1["path"], r2["path"]]}

# 單色版：.BDB/.GOB/.BDH/.GOT/.BDT
mono = ["BEG1.BDB", "BH1.BDB", "BH2.BDB", "BH4.BDB", "J1.GOB", "J2.GOB", "J3.GOB", "J4.GOB", "MAP.BDB", "MY.BDB", "MYB.BDT",
        "SHT1.BDB", "SHT1.BDH", "SHT2.BDB", "SHT2.BDH", "SHT3.BDB", "SHT3.BDH", "SHT4.BDB", "SHT4.BDH", "SHT5.BDB", "SHT5.BDH"]
pal01 = bytes((0, 0, 0, 255, 255, 255)) + bytes(3 * 14)
manifest["mono"] = {}
for n in mono:
    raw, items, over = decode(read(n))
    sh = []
    for it in items:
        sh.append((it["index"], it["stride"] * 8, it["height"], mono_to_rows(it["data"], it["height"], it["stride"])))
    sheet(sh, pal01, f"sheets/mono/{n}.png", maxw=1320)
    manifest["mono"][n] = {"count": len(items), "dims": [[it["stride"] * 8, it["height"]] for it in items]}

# 第三關地圖
for n in ("SHT.SCN", "SHT.SCR"):
    raw, _, _ = unrle(read(n), 0)
    write(f"maps/{n}.bin", raw)
    manifest["other"][n] = {"decoded_size": len(raw)}
raw, _, _ = unrle(read("SHT.SCN"), 0)
tiles = GROUPS["MAP"]["planes"]
tile_rows = {}
COLS = 600
RH = len(raw) // (COLS * 2)
pal = PAL["stage3"]["vga"]
W, H = COLS * 8, RH * 10
canvas = [bytearray(W) for _ in range(H)]
used = set()
for r in range(RH):
    for c in range(COLS):
        t = struct.unpack_from("<H", raw, (r * COLS + c) * 2)[0]
        used.add(t)
        idx = t & 0x7FFF
        if idx >= len(tiles[0][1]):
            continue
        if idx not in tile_rows:
            tile_rows[idx] = planar_to_indexed([tiles[k][1][idx]["data"] for k in range(4)], 10, 1)
        for y, row in enumerate(tile_rows[idx]):
            canvas[r * 10 + y][c * 8:c * 8 + 8] = row
write("maps/SHT.SCN.vga.png", png_bytes(W, H, canvas, palette=pal))
manifest["other"]["SHT.SCN"].update({"columns": COLS, "rows": RH, "tiles_used": len(used), "max_tile": max(used)})

for n in ("A.MIX", "B.MIX", "DM1.LCK", "DM2.LCK"):
    write(f"raw/{n}", read(n))

with open(os.path.join(dst, "manifest.json"), "w", encoding="utf-8") as fh:
    json.dump(manifest, fh, ensure_ascii=False, indent=1)
tot = sum(v["count"] for v in manifest["banks"].values())
print("banks", len(manifest["banks"]), "sprites", tot, "mono", len(manifest["mono"]), "map", manifest["other"]["SHT.SCN"])
