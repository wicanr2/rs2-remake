#!/usr/bin/env python3
"""把 Codex 批次切好的圖與衍生規則合成完整 HD 素材庫。

用法：hd_finalize.py <original/rs-2> <workspace> <批次名> [後續批次...]
同一張圖出現在多個批次時，以最後一個批次為準，舊批次不刪。
輸出：workspace/hd/assets/
  <庫>/<索引>.png        邏輯尺寸 x4 的 RGBA
  <庫>/masks/<索引>.png  遮罩，255 保留背景，0 清掉背景，尺寸為原遮罩 x4
  MAP/objects/objNN.png  第三關物件整圖
  maps/SHT.SCN.hd.png    第三關完整地圖
  hd-manifest.json       每張的來源、方法、雜湊、量測
任何一個圖庫索引沒有輸出就中止。
"""
import hashlib
import json
import os
import struct
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_lib import SCALE, Source, sha256  # noqa: E402
from rs2_common import unrle  # noqa: E402

src, ws = sys.argv[1:3]
batches = sys.argv[3:]
roots = {b: os.path.join(ws, "hd", b) for b in batches}
out = os.path.join(ws, "hd", "assets")
S = Source(src, os.path.join(ws, "unpacked"), os.path.join(ws, "extracted/manifest.json"))
plan = json.load(open(os.path.join(ws, "hd", "plan.json"), encoding="utf-8"))
metrics = {b: json.load(open(os.path.join(roots[b], "metrics.json"), encoding="utf-8")) for b in batches}
usage = json.load(open(os.path.join(ws, "extracted/sprite-usage.json"), encoding="utf-8"))
E = {(e["lib"], e["index"]): e for e in usage["entries"]}
metas = {b: json.load(open(os.path.join(roots[b], "sheets.json"), encoding="utf-8")) for b in batches}
ref_sha = {b: {s["name"]: s["ref_sha256"] for s in metas[b]["sheets"]} for b in batches}
prompt_sha = {b: {s["name"]: sha256(os.path.join(roots[b], "prompts", s["name"] + ".txt")) for s in metas[b]["sheets"]} for b in batches}
styles = {metas[b].get("style_sha256") for b in batches}
if len(styles) != 1:
    raise SystemExit(f"批次風格不一致：{styles}")
MAN = {"batches": batches, "scale": SCALE, "style": metas[batches[0]].get("style_file"), "style_sha256": styles.pop(),
       "assets": {}, "masks": {}, "map": {}}


def save(lib, index, arr, method, **extra):
    d = os.path.join(out, lib)
    os.makedirs(d, exist_ok=True)
    fp = os.path.join(d, f"{index:04d}.png")
    Image.fromarray(arr).save(fp, optimize=True)
    MAN["assets"][f"{lib}:{index}"] = dict({"method": method, "path": os.path.relpath(fp, out), "sha256": sha256(fp),
                                            "size": [int(arr.shape[1]), int(arr.shape[0])]}, **extra)
    return fp


def load_frame(batch, lib, index):
    return np.array(Image.open(os.path.join(roots[batch], "frames", lib, f"{index:04d}.png")).convert("RGBA"))


def latest_piece(key):
    for b in reversed(batches):
        if key in metrics[b]["pieces"]:
            return b
    return None


def latest_panel(name):
    for b in reversed(batches):
        if os.path.exists(os.path.join(roots[b], "panels", name + ".png")):
            return b
    return None


def scale2x(a):
    """EPX／AdvMAME2x，對索引圖運算，不產生新顏色。"""
    h, w = a.shape
    o = np.zeros((h * 2, w * 2), a.dtype)
    p = np.pad(a, 1, mode="edge")
    b = p[:-2, 1:-1]
    d = p[1:-1, :-2]
    e = p[1:-1, 1:-1]
    f = p[1:-1, 2:]
    h_ = p[2:, 1:-1]
    cond = (b != h_) & (d != f)
    o[0::2, 0::2] = np.where(cond & (d == b), d, e)
    o[0::2, 1::2] = np.where(cond & (b == f), f, e)
    o[1::2, 0::2] = np.where(cond & (d == h_), d, e)
    o[1::2, 1::2] = np.where(cond & (h_ == f), f, e)
    return o


# 1. Codex 切出的圖
codex = {}
for s in plan["sheets"]:
    for p in s["pieces"]:
        k = f"{p['lib']}:{p['index']}"
        b = latest_piece(k)
        if b is None:
            continue
        arr = load_frame(b, p["lib"], p["index"])
        m = metrics[b]["pieces"][k]
        mst = metrics[b]["sheets"][s["name"]]
        save(p["lib"], p["index"], arr, "codex-redraw", batch=b, sheet=s["name"], ref_sha256=ref_sha[b].get(s["name"]),
             prompt_sha256=prompt_sha[b].get(s["name"]), master_sha256=mst.get("master_sha256"), iou=m["iou"], color_diff=m["color_diff"],
             luma_corr=m.get("luma_corr"), sheet_zoom=s["zoom"], palette=p["palette"])
        codex[(p["lib"], p["index"])] = arr

# 2. 保護項：不交給模型，以 EPX 對索引圖放大兩次
for p in plan["protected"]:
    lib, i = p["lib"], p["index"]
    idx = scale2x(scale2x(S.indexed(lib, i)))
    pal = S.palette(p["palette"])
    rgb = pal[idx]
    arr = np.dstack([rgb, np.full(idx.shape, 255, np.uint8)])
    save(lib, i, arr, "protected-epx-scale", palette=p["palette"], note="署名或商標，未經生成模型")

# 3. 佔位項
for p in plan["placeholder"]:
    lib, i = p["lib"], p["index"]
    px = S.indexed(lib, i)
    pal = S.palette(S.manifest["banks"][lib]["palette"])
    rgb = np.repeat(np.repeat(pal[px], SCALE, axis=0), SCALE, axis=1)
    a = np.repeat(np.repeat((px != 0).astype(np.uint8) * 255, SCALE, axis=0), SCALE, axis=1)
    save(lib, i, np.dstack([rgb, a]), "placeholder-nearest", note="高 1 列的佔位圖")

# 4. 衍生：相位與由 alpha 反推的遮罩項
for d in plan["derived"]:
    lib, i, base = d["lib"], d["index"], d["base"]
    if (lib, base) not in codex:
        continue
    b = codex[(lib, base)]
    if d["kind"] == "shift":
        arr = np.roll(b, d["shift"] * SCALE, axis=1)
        save(lib, i, arr, "derived-shift", base=base, shift_px=d["shift"], match=d["match"])
        # 相位圖也可能是遮罩的來源（BH2 第 20～22 項對應遮罩第 24～26 項）
        codex[(lib, i)] = arr
    else:
        rows = d["rows"] * SCALE
        a = b[..., 3][:rows]
        v = 255 - a
        arr = np.dstack([v, v, v, np.full(v.shape, 255, np.uint8)])
        save(lib, i, arr, "derived-mask-from-alpha", base=base)

# 5. 第三關地圖
raw = unrle(open(os.path.join(src, "SHT.SCN"), "rb").read(), 0)[0]
G = np.array(struct.unpack_from("<12000H", raw, 0), dtype=np.int32).reshape(20, 600)
os.makedirs(os.path.join(out, "MAP", "objects"), exist_ok=True)
os.makedirs(os.path.join(out, "maps"), exist_ok=True)
TW, TH = 8 * SCALE, 10 * SCALE
tile_done = {}
full = np.zeros((20 * TH, 600 * TW, 3), np.uint8)
filled = np.zeros(G.shape, bool)
panels = {p["name"]: p for p in plan["map"]["panels"]}
missing_panels = []
for o in plan["map"]["objects"]:
    W = (o["c1"] - o["c0"] + 1) * TW
    H = (o["r1"] - o["r0"] + 1) * TH
    acc = np.zeros((H, W, 3), np.float32)
    wgt = np.zeros((H, W), np.float32)
    ok = True
    for name in o["panels"]:
        pb = latest_panel(name)
        if pb is None:
            missing_panels.append(name)
            ok = False
            continue
        fp = os.path.join(roots[pb], "panels", name + ".png")
        p = panels[name]
        im = np.array(Image.open(fp).convert("RGB"), np.float32)
        x0 = (p["c0"] - o["c0"]) * TW
        ramp = np.ones(im.shape[1], np.float32)
        ov = plan["map"]["overlap_columns"] * TW
        if p["c0"] > o["c0"]:
            ramp[:ov] = np.linspace(0, 1, ov)
        if p["c1"] < o["c1"]:
            ramp[-ov:] = np.linspace(1, 0, ov)
        acc[:, x0:x0 + im.shape[1]] += im * ramp[None, :, None]
        wgt[:, x0:x0 + im.shape[1]] += ramp[None, :]
    if not ok:
        continue
    img = np.clip(acc / np.maximum(wgt, 1e-3)[..., None], 0, 255).astype(np.uint8)
    fp = os.path.join(out, "MAP", "objects", f"obj{o['id']:02d}.png")
    Image.fromarray(img).save(fp, optimize=True)
    MAN["map"][f"obj{o['id']:02d}"] = {"path": os.path.relpath(fp, out), "sha256": sha256(fp), "panels": o["panels"],
                                       "panel_masters": {n: metrics[latest_panel(n)]["sheets"].get(n, {}).get("master_sha256") for n in o["panels"]},
                                       "instances": o["instances"], "bbox_cells": [o["r0"], o["r1"], o["c0"], o["c1"]]}
    # 每個實例貼進完整地圖，每種圖塊取第一次出現切回單塊
    for inst in o["instances"]:
        ir, ic = inst
        for r in range(o["r1"] - o["r0"] + 1):
            for c in range(o["c1"] - o["c0"] + 1):
                t = int(G[ir + r, ic + c])
                src_t = int(G[o["r0"] + r, o["c0"] + c])
                if t == 0 or src_t != t:
                    continue
                cell = img[r * TH:(r + 1) * TH, c * TW:(c + 1) * TW]
                full[(ir + r) * TH:(ir + r + 1) * TH, (ic + c) * TW:(ic + c + 1) * TW] = cell
                filled[ir + r, ic + c] = True
                if t not in tile_done:
                    tile_done[t] = cell.copy()
for t, cell in tile_done.items():
    arr = np.dstack([cell, np.full(cell.shape[:2], 255, np.uint8)])
    save("MAP", t, arr, "map-object-slice", note="取自該圖塊第一次出現的物件位置")
blank = np.zeros((TH, TW, 4), np.uint8)
blank[..., 3] = 255
if ("MAP", 0) not in {(a.split(":")[0], int(a.split(":")[1])) for a in MAN["assets"]}:
    save("MAP", 0, blank, "map-blank", note="全黑的空白圖塊")
need = (G != 0) & ~filled
fp = os.path.join(out, "maps", "SHT.SCN.hd.png")
Image.fromarray(full).save(fp, optimize=True)
MAN["map"]["full"] = {"path": os.path.relpath(fp, out), "sha256": sha256(fp), "size": [600 * TW, 20 * TH], "unfilled_cells": int(need.sum())}

# 6. 遮罩庫：有配對的由 HD alpha 反推並外擴一圈，沒配對的用原遮罩放大
for lib, bank in S.masks.items():
    d = os.path.join(out, lib, "masks")
    os.makedirs(d, exist_ok=True)
    for it in bank:
        i = it["index"]
        mb = S.mask_bits(lib, i)
        h, w = mb.shape
        k = (lib, i)
        e = E.get(k)
        method = "mask-nearest"
        o = np.repeat(np.repeat((mb * 255).astype(np.uint8), SCALE, axis=0), SCALE, axis=1)
        if e is not None and e["mask"].get("used") and k in codex:
            a = codex[k][..., 3]
            # 原版遮罩比平面多一圈黑邊，用外擴一個邏輯像素重現
            b = a >= 128
            for _ in range(SCALE):
                c = b.copy()
                c[1:] |= b[:-1]
                c[:-1] |= b[1:]
                c[:, 1:] |= b[:, :-1]
                c[:, :-1] |= b[:, 1:]
                b = c
            dy = 0
            for s in plan["sheets"]:
                for p in s["pieces"]:
                    if p["lib"] == lib and p["index"] == i:
                        dy = p.get("dy", 0)
            hd = np.full((h * SCALE, w * SCALE), 255, np.uint8)
            yo = dy * SCALE
            hh = min(b.shape[0], hd.shape[0] - yo)
            ww = min(b.shape[1], hd.shape[1])
            if hh > 0:
                hd[yo:yo + hh, :ww] = np.where(b[:hh, :ww], 0, 255)
            o = hd
            method = "mask-from-hd-alpha"
        fp = os.path.join(d, f"{i:04d}.png")
        Image.fromarray(o).save(fp, optimize=True)
        MAN["masks"][f"{lib}:{i}"] = {"method": method, "path": os.path.relpath(fp, out), "sha256": sha256(fp), "size": [int(o.shape[1]), int(o.shape[0])]}

# 7. 檢查完整性
missing = [f"{e['lib']}:{e['index']}" for e in usage["entries"] if f"{e['lib']}:{e['index']}" not in MAN["assets"]]
counts = {}
for v in MAN["assets"].values():
    counts[v["method"]] = counts.get(v["method"], 0) + 1
MAN["counts"] = counts
MAN["missing"] = missing
MAN["missing_panels"] = sorted(set(missing_panels))
json.dump(MAN, open(os.path.join(out, "hd-manifest.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print("素材", len(MAN["assets"]), "遮罩", len(MAN["masks"]), counts)
print("缺", len(missing), missing[:30], "缺面板", MAN["missing_panels"], "地圖未填", MAN["map"]["full"]["unfilled_cells"])
if missing or MAN["map"]["full"]["unfilled_cells"]:
    sys.exit(3)
