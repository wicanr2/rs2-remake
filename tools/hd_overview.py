#!/usr/bin/env python3
"""把 HD 素材庫排成原版／HD 對照總覽，給人檢視。不進版控。

用法：hd_overview.py <original/rs-2> <workspace>
輸出：workspace/hd/assets/overview/<庫>.png
"""
import json
import os
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_lib import SCALE, Source  # noqa: E402

src, ws = sys.argv[1:3]
S = Source(src, os.path.join(ws, "unpacked"), os.path.join(ws, "extracted/manifest.json"))
out = os.path.join(ws, "hd", "assets")
man = json.load(open(os.path.join(out, "hd-manifest.json"), encoding="utf-8"))
os.makedirs(os.path.join(out, "overview"), exist_ok=True)
bank_pal = {b: v["sprites"] for b, v in S.manifest["banks"].items()}
for lib in S.banks:
    if lib == "MAP":
        continue
    items = sorted(int(k.split(":")[1]) for k in man["assets"] if k.startswith(lib + ":"))
    W = 1800
    x = y = 8
    lh = 0
    place = []
    for i in items:
        w, h = S.size(lib, i)
        if h <= 1:
            continue
        tw, th = w * SCALE, h * SCALE
        s = 1 if tw <= 860 else 860 / tw
        dw, dh = int(tw * s), int(th * s)
        need = dw * 2 + 6
        if x + need + 8 > W:
            x = 8
            y += lh + 10
            lh = 0
        place.append((i, x, y, dw, dh))
        x += need + 18
        lh = max(lh, dh)
    H = y + lh + 8
    yy, xx = np.mgrid[0:H, 0:W]
    v = (((yy // 12) + (xx // 12)) % 2 * 20 + 48).astype(np.uint8)
    c = Image.fromarray(np.dstack([v, v, v]))
    for i, px, py, dw, dh in place:
        pal = bank_pal[lib][i].get("palette") if i < len(bank_pal[lib]) else None
        rgb = S.rgb(lib, i, pal or S.manifest["banks"][lib]["palette"])
        c.paste(Image.fromarray(rgb).resize((dw, dh), Image.NEAREST), (px, py))
        hd = Image.open(os.path.join(out, man["assets"][f"{lib}:{i}"]["path"])).convert("RGBA").resize((dw, dh), Image.LANCZOS)
        c.paste(hd, (px + dw + 6, py), hd)
    c.save(os.path.join(out, "overview", lib + ".png"), optimize=True)
print("總覽", os.listdir(os.path.join(out, "overview")))
