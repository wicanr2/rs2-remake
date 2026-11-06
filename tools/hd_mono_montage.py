#!/usr/bin/env python3
"""單色 HD 試稿的總覽 montage。不呼叫外部服務。

用法：hd_mono_montage.py <original/rs-2> <workspace> <輸出資料夾名> <標籤=HD 單色資料夾> ...
  讀 workspace/hd/<輸出資料夾名>/ 底下 hd_mono_shots.sh 產生的 <標籤>-s1-<步>.png、<標籤>-s3-<步>.png（SD 與 A 也要有）。
輸出：<輸出資料夾>/../overview.png      逐項對照（每列一項：SD、A、各風格）加四個步數的實機畫面
      <輸出資料夾>/screens-<關>-<步>.png   同一步各標籤的畫面並排（去掉工具列，只留遊戲區），共六個步數
"""
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_mono_lib import MonoSource  # noqa: E402

src, ws, outname = sys.argv[1:4]
labels = [a.split("=", 1) for a in sys.argv[4:]]
base = os.path.join(ws, "hd", outname)
S = MonoSource(src)
try:
    FONT = ImageFont.load_default(size=20)
except TypeError:
    FONT = ImageFont.load_default()
BG = (38, 38, 70)
order = [("SD", None), ("A", "assets")] + labels


def mono_dir(v):
    return os.path.join(ws, "hd", "assets", "mono") if v == "assets" else os.path.join(ws, "hd", v, "mono")


def plane(lab, v, bank, i):
    if v is None:
        return np.repeat(np.repeat(S.plane(bank, i), 4, axis=0), 2, axis=1) * 255
    p = os.path.join(mono_dir(v), bank, f"{i:04d}.png")
    if not os.path.exists(p):
        return None
    return (np.array(Image.open(p).convert("RGBA"))[..., 0] >= 128).astype(np.uint8) * 255


def fit(a, w):
    """縮放到寬 w（放大用最近鄰，縮小用面積平均）。"""
    im = Image.fromarray(a).convert("RGB")
    h = max(1, round(im.height * w / im.width))
    return im.resize((w, h), Image.NEAREST if w >= im.width else Image.BOX)


# ---- 逐項列 ----
ITEMS = [("BH1.BDB", 0), ("BH1.BDB", 2), ("BH1.BDB", 3), ("SHT1.BDB", 2), ("SHT1.BDB", 8), ("SHT1.BDB", 25), ("MY.BDB", 7), ("MY.BDB", 12), ("MY.BDB", 5), ("MY.BDB", 30)]
TW = 360  # 每格寬
rows = []
for bank, i in ITEMS:
    tiles = []
    for lab, v in order:
        a = plane(lab, v, bank, i)
        tiles.append((lab, fit(a, TW) if a is not None else None))
    rows.append((f"{bank} #{i}", tiles))
n = len(order)
W = n * TW + (n + 1) * 8
y = 8
canv_rows = []
for title, tiles in rows:
    h = max(t.height for _, t in tiles if t is not None)
    canv_rows.append((title, tiles, y, h))
    y += h + 30
cv = Image.new("RGB", (W, y + 8), BG)
d = ImageDraw.Draw(cv)
for title, tiles, yy, h in canv_rows:
    d.text((8, yy), title, fill=(255, 255, 0), font=FONT)
    for k, (lab, t) in enumerate(tiles):
        x = 8 + k * (TW + 8)
        d.text((x + 150, yy), lab, fill=(180, 255, 180), font=FONT)
        if t is not None:
            cv.paste(t, (x, yy + 24))
items_img = cv

# ---- 實機畫面 ----
def game_area(path):
    im = Image.open(path).convert("RGB")
    return im.crop((0, 80, im.width, im.height - 40))  # 去掉上方工具列與下方狀態列


steps = [("s1", "0120"), ("s1", "0200"), ("s1", "0300"), ("s1", "0450"), ("s3", "0700"), ("s3", "0760")]
OVERVIEW_STEPS = {("s1", "0200"), ("s1", "0450"), ("s3", "0700"), ("s3", "0760")}
shot_rows = []
for kind, st in steps:
    ims = []
    for lab, v in order:
        p = os.path.join(base, f"{lab}-{kind}-{st}.png")
        if os.path.exists(p):
            ims.append((lab, game_area(p)))
    if not ims:
        continue
    cw = (W - 8 * (len(ims) + 1)) // len(ims)
    row = []
    for lab, im in ims:
        row.append((lab, im.resize((cw, round(im.height * cw / im.width)), Image.BOX)))
    h = max(r[1].height for r in row)
    sc = Image.new("RGB", (W, h + 34), BG)
    dd = ImageDraw.Draw(sc)
    dd.text((8, 2), f"{kind} step {st}", fill=(255, 255, 0), font=FONT)
    for k, (lab, im) in enumerate(row):
        x = 8 + k * (cw + 8)
        dd.text((x + (170 if k == 0 else 8), 2), lab, fill=(180, 255, 180), font=FONT)
        sc.paste(im, (x, 28))
    if (kind, st) in OVERVIEW_STEPS:
        shot_rows.append(sc)
    # 單步對照圖：3 欄排列，每格約 620 寬，看得到細節
    cols = 3
    gw = (1900 - 8 * (cols + 1)) // cols
    tiles = [(lab, im.resize((gw, round(im.height * gw / im.width)), Image.BOX)) for lab, im in ims]
    th = max(t[1].height for t in tiles)
    nrow = -(-len(tiles) // cols)
    grid = Image.new("RGB", (1900, nrow * (th + 28) + 36), BG)
    gd = ImageDraw.Draw(grid)
    gd.text((8, 4), f"{kind} step {st}", fill=(255, 255, 0), font=FONT)
    for k, (lab, im) in enumerate(tiles):
        x, y = 8 + (k % cols) * (gw + 8), 32 + (k // cols) * (th + 28)
        gd.text((x, y), lab, fill=(180, 255, 180), font=FONT)
        grid.paste(im, (x, y + 22))
    grid.save(os.path.join(base, f"screens-{kind}-{st}.png"), optimize=True)

H = items_img.height + sum(r.height for r in shot_rows)
final = Image.new("RGB", (W, H), BG)
final.paste(items_img, (0, 0))
yy = items_img.height
for r in shot_rows:
    final.paste(r, (0, yy))
    yy += r.height
final.save(os.path.join(os.path.dirname(base), "overview.png"), optimize=True)
print("overview", final.size, "screens", len(shot_rows))
