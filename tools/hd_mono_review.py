#!/usr/bin/env python3
"""單色 HD 試稿的逐項對照圖。不呼叫外部服務。

用法：hd_mono_review.py <original/rs-2> <workspace> <輸出資料夾名> <標籤=HD 單色資料夾> ...
  輸出資料夾在 workspace/hd/<名>/。標籤對應資料夾，A 是 assets/mono，其他是 <批次>/mono。例如
    hd_mono_review.py /in /w pilot-mono-review/items A=assets m1=pilot-mono-m1 m2=pilot-mono-m2
每個試稿項目一張 <圖庫>-<索引>.png，從左到右（或排不下時由上到下）：原版單色 SD（最近鄰 2x4 放大）、各標籤的 HD 圖。
有遮罩的圖下方再畫一排：套用遮罩後疊在 2x2 棋盤抖動背景上的樣子，看遮罩與黑邊對不對。第三關面板另存 MAP-panel.png。
"""
import json
import os
import struct
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_mono_lib import MonoSource  # noqa: E402
from rs2_common import unrle  # noqa: E402

src, ws, outname = sys.argv[1:4]
labels = [a.split("=", 1) for a in sys.argv[4:]]
out = os.path.join(ws, "hd", outname)
os.makedirs(out, exist_ok=True)
S = MonoSource(src)
plan = json.load(open(os.path.join(ws, "hd", labels[1][1] if labels[0][1] == "assets" else labels[0][1], "plan.json"), encoding="utf-8"))
try:
    FONT = ImageFont.load_default(size=18)
except TypeError:
    FONT = ImageFont.load_default()
BG = (38, 38, 70)


def mono_dir(v):
    return os.path.join(ws, "hd", "assets", "mono") if v == "assets" else os.path.join(ws, "hd", v, "mono")


def load_plane(d, bank, i):
    p = os.path.join(d, bank, f"{i:04d}.png")
    if not os.path.exists(p):
        return None
    return (np.array(Image.open(p).convert("RGBA"))[..., 0] >= 128).astype(np.uint8)


def sd_plane(bank, i):
    return np.repeat(np.repeat(S.plane(bank, i), 4, axis=0), 2, axis=1)


def checker(h, w):
    yy, xx = np.mgrid[0:h, 0:w]
    return (((yy // 2) + (xx // 2)) % 2).astype(np.uint8)


def compose(plane, mask, dy):
    """AND 遮罩（1 保留背景）再 OR 圖。mask 的第 y + dy*4 列對圖的第 y 列。"""
    mh, w = mask.shape
    H = max(mh, plane.shape[0] + dy * 4)
    bg = checker(H, w)
    mk = np.ones((H, w), np.uint8)
    mk[:mh] = mask
    res = bg & mk
    pl = np.zeros((H, w), np.uint8)
    pl[dy * 4:dy * 4 + plane.shape[0]] = plane[:H - dy * 4]
    return res | pl


def to_img(a, s):
    im = Image.fromarray((a * 255).astype(np.uint8)).convert("RGB")
    return im if s == 1 else im.resize((im.width * s, im.height * s), Image.NEAREST)


def sheet(title, tiles, maxw=1900, maxh=1900, gap=10):
    """tiles: [(標籤, 圖列表（由上而下）)]。選最大整數倍率，水平排得下就水平，否則 2x2，再不行就垂直。"""
    base_w = max(a.shape[1] for _, arrs in tiles for a in arrs if a is not None)
    base_h = sum(max(a.shape[0] for a in arrs if a is not None) + 0 for _, arrs in tiles[:1]) if False else None
    best = None
    for s in range(8, 0, -1):
        for cols in (len(tiles), 2, 1):
            rows = -(-len(tiles) // cols)
            cw = base_w * s
            rh = [max((sum(a.shape[0] for a in arrs if a is not None) * s + (len([a for a in arrs if a is not None]) - 1) * 4 + 24) for _, arrs in tiles[r * cols:(r + 1) * cols]) for r in range(rows)]
            W = cols * cw + (cols + 1) * gap
            H = sum(rh) + (rows + 1) * gap + 28
            if W <= maxw and H <= maxh:
                best = (s, cols, cw, rh, W, H)
                break
        if best:
            break
    if best is None:
        s, cols = 1, 1
        rows = len(tiles)
        cw = base_w
        rh = [sum(a.shape[0] for a in arrs if a is not None) + 28 for _, arrs in tiles]
        best = (1, 1, cw, rh, cw + 2 * gap, sum(rh) + (rows + 1) * gap + 28)
    s, cols, cw, rh, W, H = best
    cv = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(cv)
    d.text((gap, 4), title, fill=(255, 255, 0), font=FONT)
    y = 28 + gap
    for r in range(-(-len(tiles) // cols)):
        x = gap
        for lab, arrs in tiles[r * cols:(r + 1) * cols]:
            d.text((x, y), lab, fill=(180, 255, 180), font=FONT)
            yy = y + 22
            for a in arrs:
                if a is None:
                    continue
                cv.paste(to_img(a, s), (x, yy))
                yy += a.shape[0] * s + 4
            x += cw + gap
        y += rh[r] + gap
    return cv


items = []
for sh in plan["sheets"]:
    for p in sh["pieces"]:
        items.append(p)
n = 0
for p in items:
    bank, i = p["bank"], p["index"]
    tiles = []
    srcs = [("SD original", None)] + [(lab, v) for lab, v in labels]
    for lab, v in srcs:
        if v is None:
            pl = sd_plane(bank, i)
            mk = None
            if p.get("mask"):
                mb, mi = p["mask"].split(":")
                mk = sd_plane(mb, int(mi))
        else:
            d = mono_dir(v)
            pl = load_plane(d, bank, i)
            mk = None
            if p.get("mask") and pl is not None:
                mb, mi = p["mask"].split(":")
                mk = load_plane(d, mb, int(mi))
        if pl is None:
            # 沒有這項的批次（例如只重畫一部分的試稿）不畫空格；A 一定要有
            if lab == "A":
                tiles.append((lab + " (missing)", [np.zeros((p["hd"][1], p["hd"][0]), np.uint8)]))
            continue
        arrs = [pl]
        if mk is not None:
            arrs.append(compose(pl, mk, p["dy"]))
        tiles.append((lab, arrs))
    title = f"{bank} #{i}  {p['hd'][0]}x{p['hd'][1]}  mask={p.get('mask')}  colour_ref={'yes' if p.get('colour') else 'no'}"
    cv = sheet(title, tiles)
    cv.save(os.path.join(out, f"{bank}-{i:04d}.png"), optimize=True)
    n += 1
# BH1 第 4 項（遮罩圖）單獨一張
if any(p["bank"] == "BH1.BDB" and p["index"] == 3 for p in items):
    p4 = {"bank": "BH1.BDB", "index": 4, "hd": S.hd_size("BH1.BDB", 4), "mask": None, "dy": 0}
    tiles = []
    for lab, v in [("SD original", None)] + labels:
        pl = sd_plane("BH1.BDB", 4) if v is None else load_plane(mono_dir(v), "BH1.BDB", 4)
        if pl is not None:
            tiles.append((lab, [pl]))
    sheet(f"BH1.BDB #4 (mountain mask image) {p4['hd'][0]}x{p4['hd'][1]}", tiles).save(os.path.join(out, "BH1.BDB-0004.png"), optimize=True)
    n += 1

# BH1 第 0 至 3 項是 320 像素循環捲動：把圖左右接起來，看接縫（中央就是接縫）。第 1 項只有 128 寬，會出現兩個接縫。
seam_imgs = []
for bi in (0, 1, 2, 3):
    if not any(p["bank"] == "BH1.BDB" and p["index"] == bi for p in items):
        continue
    tl = []
    for lab, v in [("SD original", None)] + labels:
        pl = sd_plane("BH1.BDB", bi) if v is None else load_plane(mono_dir(v), "BH1.BDB", bi)
        if pl is None:
            continue
        w = pl.shape[1]
        t = np.tile(pl, (1, 5))
        tl.append((lab, [t[:, 2 * w - 240:2 * w + 240]]))
    seam_imgs.append(sheet(f"BH1.BDB #{bi} wrap seam (the join is at the centre of each strip)", tl, maxw=2000))
if seam_imgs:
    H = sum(i.height for i in seam_imgs)
    cv = Image.new("RGB", (max(i.width for i in seam_imgs), H), BG)
    y = 0
    for i in seam_imgs:
        cv.paste(i, (0, y))
        y += i.height
    cv.save(os.path.join(out, "BH1-seams.png"), optimize=True)
    n += 1

# 第三關面板
m = plan["map"]
raw = unrle(open(os.path.join(src, "SHT.SCR"), "rb").read(), 0)[0]
G = np.array(struct.unpack_from("<12000H", raw, 0), np.int32).reshape(20, 600)
cells = [(r, c) for r in range(m["r0"], m["r1"] + 1) for c in range(m["c0"], m["c1"] + 1)]


def panel_from(d):
    H, W = (m["r1"] - m["r0"] + 1) * 40, (m["c1"] - m["c0"] + 1) * 32
    a = np.zeros((H, W), np.uint8)
    cache = {}
    for r, c in cells:
        t = int(G[r, c])
        if t == 0:
            continue
        if t not in cache:
            p = os.path.join(d, "MAP.BDB", f"{t:04d}.png")
            cache[t] = (np.array(Image.open(p).convert("RGBA"))[..., 0] >= 128).astype(np.uint8) if os.path.exists(p) else None
        if cache[t] is not None:
            a[(r - m["r0"]) * 40:(r - m["r0"] + 1) * 40, (c - m["c0"]) * 32:(c - m["c0"] + 1) * 32] = cache[t]
    return a


def half(a):
    return (np.array(Image.fromarray(a * 255).resize((a.shape[1] // 2, a.shape[0] // 2), Image.BOX)) >= 128).astype(np.uint8) if False else a


sd = np.zeros(((m["r1"] - m["r0"] + 1) * 40, (m["c1"] - m["c0"] + 1) * 32), np.uint8)
for r, c in cells:
    sd[(r - m["r0"]) * 40:(r - m["r0"] + 1) * 40, (c - m["c0"]) * 32:(c - m["c0"] + 1) * 32] = sd_plane("MAP.BDB", int(G[r, c]))
tiles = [("SD original", [sd])] + [(lab, [panel_from(mono_dir(v))]) for lab, v in labels]
# 1408x800 的面板兩欄排不下 1900，縮一半（面積平均後閾值）看整體，細節看下面的原尺寸裁切
def shrink(a):
    return np.array(Image.fromarray(a.astype(np.uint8) * 255).resize((a.shape[1] // 2, a.shape[0] // 2), Image.BOX))


nrows = -(-len(tiles) // 2)
cv = Image.new("RGB", (2 * (sd.shape[1] // 2) + 30, nrows * (sd.shape[0] // 2 + 30) + 40), BG)
d = ImageDraw.Draw(cv)
d.text((10, 4), "MAP.BDB stage 3 panel (SHT.SCR cols 45-88, rows 0-19), composed from tiles, 50% view", fill=(255, 255, 0), font=FONT)
for k, (lab, arrs) in enumerate(tiles):
    x = 10 + (k % 2) * (sd.shape[1] // 2 + 10)
    y = 30 + (k // 2) * (sd.shape[0] // 2 + 30)
    d.text((x, y), lab, fill=(180, 255, 180), font=FONT)
    cv.paste(Image.fromarray(shrink(arrs[0])).convert("RGB"), (x, y + 22))
cv.save(os.path.join(out, "MAP-panel.png"), optimize=True)
n += 1
# 原尺寸裁切：面板右半（砲管穹頂），每個來源一張，水平 800 寬由上而下
x0 = 18 * 32
crop_w = 24 * 32
tiles = [(lab, [a[0][400:800, x0:x0 + crop_w]]) for lab, a in tiles]
sheet("MAP.BDB stage 3 panel, right part at 100% (cols 63-86, rows 10-19)", tiles, maxw=1900, maxh=1900).save(os.path.join(out, "MAP-panel-detail.png"), optimize=True)
n += 1
print("對照圖", n, "張 →", out)
