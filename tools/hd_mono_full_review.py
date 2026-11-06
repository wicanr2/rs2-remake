#!/usr/bin/env python3
"""單色 HD 全量重繪的對照圖。不呼叫外部服務。

用法：hd_mono_full_review.py <original/rs-2> <workspace> <批次名（讀 plan.json 與 ref）> <候選資料夾名> <輸出資料夾名>
輸出在 workspace/hd/<輸出資料夾名>/：
  sheets/<表名>.png        每張表：上 = 原版單色參考表（參考圖裁到圖的範圍），中 = 候選成品，下 = 現況 assets（有的話）。
                           候選與現況以各圖的 alpha 疊在灰底上，透明處（輪廓外）顯示灰色，輪廓內的黑是黑。
  map-strip-<n>.png        第三關地圖一段：上 = 原版單色（2x4 放大），中 = 候選，下 = 現況，都由 SHT.SCR 的圖塊拼成
  montage-<群>.png         幾張表併成一張的總覽（縮到 1800 寬以內）
"""
import json
import os
import struct
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_mono_full_lib import Full, nearest_hd  # noqa: E402
from rs2_common import unrle  # noqa: E402

src, ws, batch, cand, outname = sys.argv[1:6]
hd = os.path.join(ws, "hd")
out = os.path.join(hd, outname)
os.makedirs(os.path.join(out, "sheets"), exist_ok=True)
S = Full(src)
plan = json.load(open(os.path.join(hd, batch, "plan.json"), encoding="utf-8"))
try:
    FONT = ImageFont.load_default(size=18)
except TypeError:
    FONT = ImageFont.load_default()
BG = (38, 38, 70)
GREY = (70, 70, 70)


def load(dirname, bank, i):
    p = os.path.join(hd, dirname, "mono", bank, f"{i:04d}.png")
    return Image.open(p).convert("RGBA") if os.path.exists(p) else None


def render_sheet(s, which):
    """which: 'cand' 或 'assets'。回傳 RGB 影像（整張表大小，灰底）。"""
    W, H = s["canvas"]
    z = s["zoom"]
    cv = Image.new("RGB", (W, H), GREY)
    for p in s["pieces"]:
        im = load(cand if which == "cand" else "assets", p["bank"], p["index"])
        if im is None:
            continue
        tw, th = p["w"] * z, p["h"] * z
        im = im.resize((tw, th), Image.NEAREST if z >= 4 else Image.BOX)
        bg = Image.new("RGB", (tw, th), GREY)
        bg.paste(im.convert("RGB"), (0, 0), im.split()[3])
        cv.paste(bg, (p["x"], p["y"]))
    return cv


def bbox(s):
    z = s["zoom"]
    x0 = min(p["x"] for p in s["pieces"])
    y0 = min(p["y"] for p in s["pieces"])
    x1 = max(p["x"] + p["w"] * z for p in s["pieces"])
    y1 = max(p["y"] + p["h"] * z for p in s["pieces"])
    return max(0, x0 - 8), max(0, y0 - 8), min(s["canvas"][0], x1 + 8), min(s["canvas"][1], y1 + 8)


def stack(title, rows, maxw=1800, gap=6):
    """rows: [(標籤, 影像)]，等比例縮到 maxw 以內，由上而下排。"""
    w = max(im.width for _, im in rows)
    sc = min(1.0, maxw / w)
    ims = [(lab, im.resize((max(1, round(im.width * sc)), max(1, round(im.height * sc))), Image.NEAREST if sc >= 1 else Image.BOX)) for lab, im in rows]
    H = sum(im.height + 22 for _, im in ims) + 28 + gap
    cv = Image.new("RGB", (max(im.width for _, im in ims) + 2 * gap, H), BG)
    d = ImageDraw.Draw(cv)
    d.text((gap, 3), f"{title}  (scale {sc:.2f})", fill=(255, 255, 0), font=FONT)
    y = 28
    for lab, im in ims:
        d.text((gap, y), lab, fill=(180, 255, 180), font=FONT)
        cv.paste(im, (gap, y + 20))
        y += im.height + 22
    return cv


made = {}
for s in plan["sheets"]:
    rp = os.path.join(hd, batch, "ref", s["name"] + ".png")
    if not os.path.exists(rp):
        continue
    x0, y0, x1, y1 = bbox(s)
    ref = Image.open(rp).convert("RGB").crop((x0, y0, x1, y1))
    c = render_sheet(s, "cand").crop((x0, y0, x1, y1))
    a = render_sheet(s, "assets").crop((x0, y0, x1, y1))
    im = stack(s["name"], [("SD reference", ref), ("candidate", c), ("current assets", a)])
    im.save(os.path.join(out, "sheets", s["name"] + ".png"), optimize=True)
    made[s["name"]] = im
print("sheets", len(made))

# 總覽：依前綴併圖
groups = {"BEG1": "mono-BEG1", "BH-J": ("mono-BH", "mono-J", "mono-SHT5-single"), "MY": "mono-MY", "SHT1": "mono-SHT1", "SHT2": "mono-SHT2", "SHT3": "mono-SHT3", "SHT4": "mono-SHT4",
          "SHT5": "mono-SHT5-end"}
for g, pre in groups.items():
    names = [n for n in made if n.startswith(pre)]
    if not names:
        continue
    ims = [made[n] for n in names]
    # 每 4 張一頁，避免圖太長
    for k in range(0, len(ims), 4):
        chunk = ims[k:k + 4]
        W = max(i.width for i in chunk)
        H = sum(i.height for i in chunk)
        cv = Image.new("RGB", (W, H), BG)
        y = 0
        for i in chunk:
            cv.paste(i, (0, y))
            y += i.height
        cv.save(os.path.join(out, f"montage-{g}-{k // 4}.png"), optimize=True)

# 第三關地圖段
raw = unrle(open(os.path.join(src, "SHT.SCR"), "rb").read(), 0)[0]
G = np.array(struct.unpack_from("<12000H", raw, 0), np.int32).reshape(20, 600)


def strip(c0, c1, which):
    w = (c1 - c0) * 32
    a = np.zeros((800, w), np.uint8)
    cache = {}
    for r in range(20):
        for c in range(c0, c1):
            t = int(G[r, c])
            if not t:
                continue
            if t not in cache:
                if which == "sd":
                    cache[t] = nearest_hd(S.plane("MAP.BDB", t))
                else:
                    im = load(cand if which == "cand" else "assets", "MAP.BDB", t)
                    cache[t] = None if im is None else np.array(im)[..., 0]
            if cache[t] is not None:
                a[r * 40:(r + 1) * 40, (c - c0) * 32:(c - c0 + 1) * 32] = cache[t]
    return Image.fromarray(a).convert("RGB")


n = 0
for k, c0 in enumerate((0, 100, 200, 300, 400, 500)):
    c1 = min(600, c0 + 56)
    stack(f"MAP.BDB stage 3 columns {c0}-{c1 - 1} from SHT.SCR", [("SD original", strip(c0, c1, "sd")), ("candidate", strip(c0, c1, "cand")), ("current assets", strip(c0, c1, "assets"))],
          maxw=1790).save(os.path.join(out, f"map-strip-{k}.png"), optimize=True)
    n += 1
print("map strips", n, "->", out)
