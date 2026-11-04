#!/usr/bin/env python3
"""把 Codex 原稿切回每張圖，去背，量輪廓吻合度，產生對照圖。

用法：hd_cut.py <original/rs-2> <workspace> <批次名>
輸出：workspace/hd/<批次>/frames/<庫>/<索引>.png（邏輯尺寸 x4 的 RGBA）
      panels/<面板>.png、review/<名>.png、metrics.json
原稿不存在就跳過並記錄，尺寸不符就記錄為失敗。
"""
import json
import os
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_lib import CHROMAS, SCALE, Source, sha256  # noqa: E402
from hd_plan_shared import silhouette_for  # noqa: E402

src, ws, batch = sys.argv[1:4]
root = os.path.join(ws, "hd", batch)
S = Source(src, os.path.join(ws, "unpacked"), os.path.join(ws, "extracted/manifest.json"))
plan = json.load(open(os.path.join(ws, "hd", "plan.json"), encoding="utf-8"))
os.makedirs(os.path.join(root, "review"), exist_ok=True)


def key_alpha(rgb, chroma):
    """依與底色的距離估 alpha。距離 <60 全透明，>140 全不透明，中間線性。"""
    d = np.sqrt(((rgb.astype(np.float32) - np.array(chroma, np.float32)) ** 2).sum(axis=2))
    return np.clip((d - 60) / 80, 0, 1)


def despill(rgb, alpha, chroma):
    """半透明邊緣把底色成分扣掉，避免邊緣發色。"""
    c = np.array(chroma, np.float32)
    a = np.maximum(alpha, 0.05)[..., None]
    out = (rgb.astype(np.float32) - (1 - a) * c) / a
    return np.clip(out, 0, 255)


def fix_opaque(crop, chroma, z):
    """不透明的圖不該有底色。外圈殘留用內側像素補，內部整片殘留視為黑底。"""
    a = key_alpha(crop, chroma)
    rgb = despill(crop, a, chroma) * a[..., None]
    h, w = a.shape
    r = max(2, z)
    bad = a < 0.5
    for k in range(r - 1, -1, -1):
        src_row = k + 1
        m = bad[k]
        rgb[k, m] = rgb[src_row, m]
        m = bad[h - 1 - k]
        rgb[h - 1 - k, m] = rgb[h - 2 - k, m]
    for k in range(r - 1, -1, -1):
        m = bad[:, k]
        rgb[m, k] = rgb[m, k + 1]
        m = bad[:, w - 1 - k]
        rgb[m, w - 1 - k] = rgb[m, w - 2 - k]
    return rgb


def dilate(b, n):
    out = b.copy()
    for _ in range(n):
        o = out.copy()
        o[1:] |= out[:-1]
        o[:-1] |= out[1:]
        o[:, 1:] |= out[:, :-1]
        o[:, :-1] |= out[:, 1:]
        out = o
    return out


def downscale_bool(a, w, h):
    return np.array(Image.fromarray((a * 255).astype(np.uint8)).resize((w, h), Image.BOX)) >= 128


def cut_piece(master, p, z, chroma):
    w, h = p["w"], p["h"]
    x, y = p["x"], p["y"]
    crop = master[y:y + h * z, x:x + w * z]
    tw, th = w * SCALE, h * SCALE
    sil = silhouette_for(S, p)
    if p["opaque"]:
        alpha = np.ones(crop.shape[:2], np.float32)
        rgb = fix_opaque(crop, chroma, z)
    elif p.get("free"):
        # 只畫物件、四周去背：不受原版矩形輪廓限制
        alpha = key_alpha(crop, chroma)
        rgb = despill(crop, alpha, chroma)
    else:
        alpha = key_alpha(crop, chroma)
        rgb = despill(crop, alpha, chroma)
        allow = dilate(sil, 1)
        allow_big = np.repeat(np.repeat(allow, z, axis=0), z, axis=1)
        alpha = alpha * allow_big
    a8 = Image.fromarray((alpha * 255).astype(np.uint8))
    premul = np.clip(rgb * alpha[..., None], 0, 255).astype(np.uint8)
    pm = Image.fromarray(premul).resize((tw, th), Image.LANCZOS)
    a_big = a8.resize((tw, th), Image.LANCZOS)
    pa = np.array(pm, np.float32)
    aa = np.array(a_big, np.float32) / 255
    unp = np.where(aa[..., None] > 0.003, pa / np.maximum(aa[..., None], 0.003), 0)
    out = np.dstack([np.clip(unp, 0, 255).astype(np.uint8), (aa * 255).astype(np.uint8)])
    low = downscale_bool(alpha >= 0.5, w, h)
    inter = int((low & sil).sum())
    union = int((low | sil).sum())
    iou = inter / union if union else 1.0
    # 色彩差：HD 縮回原尺寸，與原圖在輪廓內的平均 RGB 差
    lr = np.array(Image.fromarray(np.clip(rgb, 0, 255).astype(np.uint8)).resize((w, h), Image.BOX), np.float32)
    orig = S.rgb(p["lib"], p["index"], p["palette"]).astype(np.float32)
    cdiff = float(np.abs(lr - orig)[sil].mean()) if sil.any() else 0.0
    # 亮度相關：不透明的圖 IoU 恆為 1，要靠這個抓位移或內容跑掉
    la = lr[sil].mean(axis=1) if sil.any() else np.zeros(1)
    lb = orig[sil].mean(axis=1) if sil.any() else np.zeros(1)
    corr = float(np.corrcoef(la, lb)[0, 1]) if la.size > 3 and la.std() > 1e-3 and lb.std() > 1e-3 else 1.0
    if p.get("free"):
        iou = None
    else:
        iou = round(iou, 4)
    return out, {"iou": iou, "color_diff": round(cdiff, 1), "luma_corr": round(corr, 3), "opaque": p["opaque"], "free": bool(p.get("free"))}


metrics = {"batch": batch, "sheets": {}, "pieces": {}}
checker = None


def board(w, h, n=16):
    yy, xx = np.mgrid[0:h, 0:w]
    v = (((yy // n) + (xx // n)) % 2 * 24 + 40).astype(np.uint8)
    return np.dstack([v, v, v])


for s in plan["sheets"]:
    mp = os.path.join(root, "masters", s["name"] + ".png")
    if not os.path.exists(mp):
        continue
    im = Image.open(mp).convert("RGB")
    if list(im.size) != s["canvas"]:
        metrics["sheets"][s["name"]] = {"error": f"尺寸 {im.size} != {s['canvas']}"}
        continue
    master = np.array(im)
    chroma = CHROMAS[s["chroma"]]
    rows = []
    for p in s["pieces"]:
        rgba, m = cut_piece(master, p, s["zoom"], chroma)
        d = os.path.join(root, "frames", p["lib"])
        os.makedirs(d, exist_ok=True)
        fp = os.path.join(d, f"{p['index']:04d}.png")
        Image.fromarray(rgba).save(fp, optimize=True)
        m.update({"sheet": s["name"], "sha256": sha256(fp), "size": [p["w"] * SCALE, p["h"] * SCALE], "zoom": s["zoom"]})
        metrics["pieces"][f"{p['lib']}:{p['index']}"] = m
        rows.append((p, rgba, m))
    metrics["sheets"][s["name"]] = {"master_sha256": sha256(mp), "pieces": len(rows),
                                    "min_iou": min((r[2]["iou"] for r in rows if r[2]["iou"] is not None), default=None), "max_color_diff": max(r[2]["color_diff"] for r in rows)}
    # 對照圖：左原版 x4，右 HD
    W = 1600
    H = 0
    x = y = 8
    lh = 0
    place = []
    for p, rgba, m in rows:
        tw, th = rgba.shape[1], rgba.shape[0]
        need = tw * 2 + 8
        if x + need > W:
            x = 8
            y += lh + 8
            lh = 0
        place.append((p, rgba, x, y))
        x += need + 16
        lh = max(lh, th)
    H = y + lh + 8
    canvas = board(max(W, max(r[1].shape[1] * 2 + 24 for r in rows)), H)
    for p, rgba, xx, yy in place:
        orig = S.rgb(p["lib"], p["index"], p["palette"])
        sil = silhouette_for(S, p)
        o = np.repeat(np.repeat(orig, SCALE, axis=0), SCALE, axis=1)
        om = np.repeat(np.repeat(sil, SCALE, axis=0), SCALE, axis=1)
        reg = canvas[yy:yy + o.shape[0], xx:xx + o.shape[1]]
        reg[om] = o[om]
        tw = rgba.shape[1]
        reg2 = canvas[yy:yy + rgba.shape[0], xx + tw + 8:xx + tw + 8 + tw]
        a = rgba[..., 3:4].astype(np.float32) / 255
        reg2[:] = np.clip(rgba[..., :3] * a + reg2 * (1 - a), 0, 255).astype(np.uint8)
    Image.fromarray(canvas).save(os.path.join(root, "review", s["name"] + ".png"), optimize=True)

for p in plan["map"]["panels"]:
    mp = os.path.join(root, "masters", p["name"] + ".png")
    if not os.path.exists(mp):
        continue
    im = Image.open(mp).convert("RGB")
    if list(im.size) != p["canvas"]:
        metrics["sheets"][p["name"]] = {"error": f"尺寸 {im.size} != {p['canvas']}"}
        continue
    master = np.array(im)
    z = p["zoom"]
    crop = master[p["y"]:p["y"] + p["h"] * z, p["x"]:p["x"] + p["w"] * z]
    crop = np.clip(fix_opaque(crop, CHROMAS[p["chroma"]], z), 0, 255).astype(np.uint8)
    out = Image.fromarray(crop).resize((p["w"] * SCALE, p["h"] * SCALE), Image.LANCZOS)
    os.makedirs(os.path.join(root, "panels"), exist_ok=True)
    out.save(os.path.join(root, "panels", p["name"] + ".png"), optimize=True)
    low = np.array(Image.fromarray(crop).resize((p["w"], p["h"]), Image.BOX), np.float32).mean(axis=2)
    ref = np.array(Image.open(os.path.join(root, "ref", p["name"] + ".png")).convert("RGB"), np.float32)
    ref = ref[p["y"]:p["y"] + p["h"] * z:z, p["x"]:p["x"] + p["w"] * z:z].mean(axis=2)
    corr = float(np.corrcoef(low.ravel(), ref.ravel())[0, 1])
    metrics["sheets"][p["name"]] = {"master_sha256": sha256(mp), "zoom": z, "luma_corr": round(corr, 3)}
    side = np.vstack([np.repeat(np.repeat(np.array(Image.open(os.path.join(root, "ref", p["name"] + ".png")).convert("RGB"))[p["y"]:p["y"] + p["h"] * z, p["x"]:p["x"] + p["w"] * z], 1, axis=0), 1, axis=1),
                      crop])
    Image.fromarray(side).save(os.path.join(root, "review", p["name"] + ".png"), optimize=True)
json.dump(metrics, open(os.path.join(root, "metrics.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
iou = sorted((v["iou"], k) for k, v in metrics["pieces"].items() if not v["opaque"] and v["iou"] is not None)
print("切出", len(metrics["pieces"]), "張，表", len(metrics["sheets"]))
print("最低 IoU", iou[:8])
print("錯誤", [k for k, v in metrics["sheets"].items() if "error" in v])
corr = sorted((v["luma_corr"], k) for k, v in metrics["pieces"].items())
print("最低亮度相關", corr[:12])
print("地圖面板相關", sorted((v["luma_corr"], k) for k, v in metrics["sheets"].items() if "luma_corr" in v)[:6])
