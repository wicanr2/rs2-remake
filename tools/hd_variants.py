#!/usr/bin/env python3
"""補齊「每個素材」的另外兩塊，不呼叫 Codex：

1. 調色盤變體：原版同一張圖在不同關卡用不同調色盤（例如主角機）。以色號對應把 HD 圖重新上色，
   寫到 assets/<庫>/variants/<調色盤>/<索引>.png。
2. 單色版：CGA／MGA 用的 1bpp 圖庫。由對應的 HD 彩色圖轉灰階後以 Bayer 抖動成 1bpp，
   寫到 assets/mono/<檔名>/<索引>.png，寬度依單色版邏輯尺寸 x2、高度 x4（單色版寬度單位本來就是 2 倍）。
   這是推導結果，不是 Codex 重繪，manifest 會標明。

用法：hd_variants.py <original/rs-2> <workspace>
"""
import json
import os
import struct
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_lib import SCALE, Source, sha256  # noqa: E402
from rs2_common import mono_to_rows, parse_bank, unrle  # noqa: E402

src, ws = sys.argv[1:3]
out = os.path.join(ws, "hd", "assets")
S = Source(src, os.path.join(ws, "unpacked"), os.path.join(ws, "extracted/manifest.json"))
usage = json.load(open(os.path.join(ws, "extracted/sprite-usage.json"), encoding="utf-8"))
man = json.load(open(os.path.join(out, "hd-manifest.json"), encoding="utf-8"))
SKIP = {"stage2_d", "loader"}
Image.MAX_IMAGE_PIXELS = None

# ---------- 調色盤變體 ----------


def build_lut(pa, pb):
    """32^3 查表：RGB → 依最近的 3 個來源色加權，把差量帶到目的色。保留重繪補上的明暗細節。"""
    grid = np.linspace(4, 252, 32, dtype=np.float32)
    r, g, b = np.meshgrid(grid, grid, grid, indexing="ij")
    c = np.stack([r, g, b], axis=-1).reshape(-1, 3)
    pa = pa.astype(np.float32)
    pb = pb.astype(np.float32)
    d = np.sqrt(((c[:, None, :] - pa[None, :, :]) ** 2).sum(axis=2)) + 1e-3
    idx = np.argsort(d, axis=1)[:, :3]
    w = 1 / np.take_along_axis(d, idx, axis=1) ** 2
    w /= w.sum(axis=1, keepdims=True)
    res = np.zeros_like(c)
    for k in range(3):
        i = idx[:, k]
        res += w[:, k, None] * (pb[i] + (c - pa[i]))
    return np.clip(res, 0, 255).astype(np.uint8).reshape(32, 32, 32, 3)


luts = {}
variants = {}
for e in usage["entries"]:
    if e["lib"] == "MAP" or e["status"] == "placeholder":
        continue
    k = f"{e['lib']}:{e['index']}"
    a = man["assets"].get(k)
    if not a or a["method"] not in ("codex-redraw", "derived-shift"):
        continue
    base = a.get("palette")
    if a["method"] == "derived-shift":
        base = S.manifest["banks"][e["lib"]]["palette"]
    ps = [p for p in e["palettes"] if p not in SKIP and p != base]
    if not ps:
        continue
    im = np.array(Image.open(os.path.join(out, a["path"])).convert("RGBA"))
    for p in ps:
        key = (base, p)
        if key not in luts:
            luts[key] = build_lut(S.palette(base), S.palette(p))
        lut = luts[key]
        q = (im[..., :3] >> 3).astype(np.intp)
        rgb = lut[q[..., 0], q[..., 1], q[..., 2]]
        arr = np.dstack([rgb, im[..., 3]])
        d = os.path.join(out, e["lib"], "variants", p)
        os.makedirs(d, exist_ok=True)
        fp = os.path.join(d, f"{e['index']:04d}.png")
        Image.fromarray(arr).save(fp, optimize=True)
        variants[f"{k}@{p}"] = {"method": "palette-recolor", "from_palette": base, "path": os.path.relpath(fp, out), "sha256": sha256(fp)}

# ---------- 單色版 ----------
BAYER = np.array([[0, 32, 8, 40, 2, 34, 10, 42], [48, 16, 56, 24, 50, 18, 58, 26], [12, 44, 4, 36, 14, 46, 6, 38], [60, 28, 52, 20, 62, 30, 54, 22],
                  [3, 35, 11, 43, 1, 33, 9, 41], [51, 19, 59, 27, 49, 17, 57, 25], [15, 47, 7, 39, 13, 45, 5, 37], [63, 31, 55, 23, 61, 29, 53, 21]], np.float32)
BAYER = (BAYER + 0.5) / 64


def dither(gray):
    h, w = gray.shape
    # 每 2x2 一個抖動單元，看起來才像 1bpp 的網點而不是細雜訊
    t = BAYER[(np.arange(h) // 2 % 8)[:, None], (np.arange(w) // 2 % 8)[None, :]]
    return (gray / 255.0 > t).astype(np.uint8) * 255


def mono_bank(name):
    b = open(os.path.join(src, name), "rb").read()
    return parse_bank(unrle(b, 0)[0])


MONO_IMG = {"BEG1.BDB": "BEG1", "BH1.BDB": "BH1", "BH2.BDB": "BH2", "BH4.BDB": "BH4", "J1.GOB": "J1", "J2.GOB": "J2", "J3.GOB": "J3", "J4.GOB": "J4",
            "MY.BDB": "MY", "SHT1.BDB": "SHT1", "SHT2.BDB": "SHT2", "SHT3.BDB": "SHT3", "SHT4.BDB": "SHT4", "SHT5.BDB": "SHT5"}
MONO_MASK = {"MYB.BDT": "MY", "SHT1.BDH": "SHT1", "SHT2.BDH": "SHT2", "SHT3.BDH": "SHT3", "SHT4.BDH": "SHT4", "SHT5.BDH": "SHT5"}
# 署名與商標（AGENTS.md「文化保存」）：抖動會把字弄糊，直接放大原版單色圖。
MONO_CREDITS = {("MY.BDB", 15), ("MY.BDB", 26), ("MY.BDB", 47), ("MY.BDB", 48)}
mono = {}
report = []


def corr(a, b):
    if a.std() < 1e-3 or b.std() < 1e-3:
        return None
    return float(np.corrcoef(a.ravel(), b.ravel())[0, 1])


for fname, lib in MONO_IMG.items():
    d = os.path.join(out, "mono", fname)
    os.makedirs(d, exist_ok=True)
    for it in mono_bank(fname):
        i = it["index"]
        tw, th = it["stride"] * 8 * 2, it["height"] * SCALE
        a = man["assets"].get(f"{lib}:{i}")
        orig = np.array(mono_to_rows(it["data"], it["height"], it["stride"]), dtype=np.float32) if it["height"] else None
        if a is None or it["height"] <= 1 or (fname, i) in MONO_CREDITS:
            arr = np.repeat(np.repeat((orig * 255).astype(np.uint8), SCALE, axis=0), 2, axis=1)
            al = np.full(arr.shape, 255, np.uint8)
            method = "mono-nearest"
            c = None
        else:
            im = Image.open(os.path.join(out, a["path"])).convert("RGBA").resize((tw, th), Image.LANCZOS)
            n = np.array(im, np.float32)
            gray = n[..., :3].mean(axis=2)
            al = n[..., 3].astype(np.uint8)
            arr = dither(gray)
            method = "mono-derived-dither"
            lo = np.array(Image.fromarray(arr).resize((it["stride"] * 8, it["height"]), Image.BOX), np.float32)
            c = corr(lo, orig * 255)
        fp = os.path.join(d, f"{i:04d}.png")
        Image.fromarray(np.dstack([arr, arr, arr, al])).save(fp, optimize=True)
        mono[f"{fname}:{i}"] = {"method": method, "from": f"{lib}:{i}" if a else None, "path": os.path.relpath(fp, out), "sha256": sha256(fp),
                                "corr_to_original": None if c is None else round(c, 3)}
        if c is not None:
            report.append((c, f"{fname}:{i}"))
for fname, lib in MONO_MASK.items():
    d = os.path.join(out, "mono", fname)
    os.makedirs(d, exist_ok=True)
    for it in mono_bank(fname):
        i = it["index"]
        tw, th = it["stride"] * 8 * 2, it["height"] * SCALE
        # 原版單色遮罩的放大，HD 版遮罩可由上面單色圖的 alpha 反推
        orig = np.array(mono_to_rows(it["data"], it["height"], it["stride"]), dtype=np.uint8) * 255
        arr = np.repeat(np.repeat(orig, SCALE, axis=0), 2, axis=1)
        fp = os.path.join(d, f"{i:04d}.png")
        Image.fromarray(arr).save(fp, optimize=True)
        mono[f"{fname}:{i}"] = {"method": "mono-mask-nearest", "path": os.path.relpath(fp, out), "sha256": sha256(fp)}

# MAP.BDB：依 SHT.SCR 與 SHT.SCN 同格對應，從 HD 地圖取圖
scn = np.array(struct.unpack_from("<12000H", unrle(open(os.path.join(src, "SHT.SCN"), "rb").read(), 0)[0], 0), np.int32).reshape(20, 600)
scr = np.array(struct.unpack_from("<12000H", unrle(open(os.path.join(src, "SHT.SCR"), "rb").read(), 0)[0], 0), np.int32).reshape(20, 600)
same_geo = float(((scn != 0) == (scr != 0)).mean())
full = np.array(Image.open(os.path.join(out, "maps", "SHT.SCN.hd.png")).convert("RGB"), np.float32).mean(axis=2)
bank = mono_bank("MAP.BDB")
d = os.path.join(out, "mono", "MAP.BDB")
os.makedirs(d, exist_ok=True)
TW, TH = 8 * SCALE, 10 * SCALE
first = {}
for r in range(20):
    for c in range(600):
        t = int(scr[r, c])
        # 優先取彩色地圖也有圖的格；兩邊都空的格沒有內容可取
        if t not in first and scn[r, c] != 0:
            first[t] = (r, c)
map_rep = []
for it in bank:
    i = it["index"]
    fw, fh = it["stride"] * 8 * 2, it["height"] * SCALE
    orig = np.array(mono_to_rows(it["data"], it["height"], it["stride"]), dtype=np.float32) * 255
    if i in first and i != 0:
        r, c = first[i]
        cell = full[r * TH:(r + 1) * TH, c * TW:(c + 1) * TW]
        g = np.array(Image.fromarray(cell.astype(np.uint8)).resize((fw, fh), Image.LANCZOS), np.float32)
        arr = dither(g)
        lo = np.array(Image.fromarray(arr).resize((it["stride"] * 8, it["height"]), Image.BOX), np.float32)
        cc = corr(lo, orig)
        if cc is not None:
            map_rep.append(cc)
        method = "mono-derived-dither"
    else:
        arr = np.repeat(np.repeat(orig.astype(np.uint8), SCALE, axis=0), 2, axis=1)
        method = "mono-nearest"
    fp = os.path.join(d, f"{i:04d}.png")
    Image.fromarray(arr).save(fp, optimize=True)
    mono[f"MAP.BDB:{i}"] = {"method": method, "path": os.path.relpath(fp, out), "sha256": sha256(fp)}

res = {"variants": variants, "mono": mono, "map_geometry_match": round(same_geo, 4),
       "map_corr_median": round(float(np.median(map_rep)), 3) if map_rep else None}
json.dump(res, open(os.path.join(out, "hd-variants-manifest.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
report.sort()
print("調色盤變體", len(variants), "涉及調色盤對", len(luts))
cnt = {}
for v in mono.values():
    cnt[v["method"]] = cnt.get(v["method"], 0) + 1
print("單色", len(mono), cnt)
print("SCN/SCR 幾何一致", res["map_geometry_match"], "地圖相關中位數", res["map_corr_median"])
print("單色相關最低", [(round(a, 3), b) for a, b in report[:10]])
print("中位數", round(float(np.median([a for a, b in report])), 3))
