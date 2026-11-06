#!/usr/bin/env python3
"""把單色 HD 試稿的 Codex 原稿切回每張圖，轉成嚴格 1 位元，輸出 MonoHdSink 讀的目錄結構。不呼叫外部服務。

用法：hd_mono_cut.py <original/rs-2> <workspace> <批次名> <轉換法> [--seams]
  轉換法
    thr:<T>   縮到 HD 尺寸（面積平均）後，灰階 >= T 為亮點。純黑白墨線稿用（T 小於 128 讓細線容易變黑，大於 128 讓細線容易變白）。
    bayer4    縮到 HD 尺寸後做 4x4 Bayer 有序抖動，逐 HD 像素。灰階稿用。
              週期 4 同時整除位元組寬 16 與掃描線高 4，任何擺放位置的抖動晶格都對齊，捲動與移動時圖樣不會漂移。
  --seams     BH1 第 0 至 3 項（循環捲動）在轉 1 位元之前，於灰階上做與彩色版 hd_postfix.py 相同的接縫修補
              （level_wrap、blend_wrap、頂底漸暗）。
輸出（<批次>/mono/，與 <hd_dir>/mono/ 同格式）：
  <圖庫檔名>/<索引>.png   圖：RGBA，R=G=B 為 0 或 255，輪廓外 alpha 0
  <遮罩庫檔名>/<索引>.png  遮罩：灰階，255 保留背景、0 圖在此處。依 alpha 外擴 2 個 HD 像素反推，圖的亮點一律限制在遮罩內
  review/<名>.png         每張表的參考表、原稿、結果對照
  mono-manifest.json      每張的來源 SHA-256、提示詞、工具、輸出 SHA-256 與量測
"""
import json
import os
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_mono_lib import CHROMA, MonoSource, sha256  # noqa: E402

TOOL = "hd_mono_cut.py r1"
src, ws, batch, post = sys.argv[1:5]
seams = "--seams" in sys.argv[5:]
root = os.path.join(ws, "hd", batch)
out = os.path.join(root, "mono")
S = MonoSource(src)
plan = json.load(open(os.path.join(root, "plan.json"), encoding="utf-8"))
sheets_meta = json.load(open(os.path.join(root, "sheets.json"), encoding="utf-8"))
SM = {s["name"]: s for s in sheets_meta["sheets"]}
receipts = {}
rp = os.path.join(root, "logs", "_receipts.tsv")
if os.path.exists(rp):
    for ln in open(rp, encoding="utf-8"):
        f = ln.rstrip("\n").split("\t")
        if len(f) >= 4:
            receipts[f[0]] = {"session": f[1], "master_sha256": f[2], "codex": f[3]}
BAYER4 = (np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]], np.float32) + 0.5) / 16


def key_alpha(rgb, chroma):
    d = np.sqrt(((rgb.astype(np.float32) - np.array(chroma, np.float32)) ** 2).sum(axis=2))
    return np.clip((d - 60) / 80, 0, 1)


def despill(rgb, alpha, chroma):
    c = np.array(chroma, np.float32)
    a = np.maximum(alpha, 0.05)[..., None]
    return np.clip((rgb.astype(np.float32) - (1 - a) * c) / a, 0, 255)


def fix_opaque(crop, chroma, z):
    """不透明的圖不該有底色。外圈殘留用內側像素補，內部整片殘留視為黑底（同 hd_cut.py）。"""
    a = key_alpha(crop, chroma)
    rgb = despill(crop, a, chroma) * a[..., None]
    h, w = a.shape
    r = max(2, z)
    bad = a < 0.5
    for k in range(r - 1, -1, -1):
        m = bad[k]
        rgb[k, m] = rgb[k + 1, m]
        m = bad[h - 1 - k]
        rgb[h - 1 - k, m] = rgb[h - 2 - k, m]
    for k in range(r - 1, -1, -1):
        m = bad[:, k]
        rgb[m, k] = rgb[m, k + 1]
        m = bad[:, w - 1 - k]
        rgb[m, w - 1 - k] = rgb[m, w - 2 - k]
    return rgb


def dilate(b, n):
    out_ = b.copy()
    for _ in range(n):
        o = out_.copy()
        o[1:] |= out_[:-1]
        o[:-1] |= out_[1:]
        o[:, 1:] |= out_[:, :-1]
        o[:, :-1] |= out_[:, 1:]
        out_ = o
    return out_


def luma(rgb):
    return rgb[..., 0] * 0.299 + rgb[..., 1] * 0.587 + rgb[..., 2] * 0.114


def resize_f(a, w, h):
    """float 陣列面積平均縮放。"""
    return np.array(Image.fromarray(a.astype(np.float32)).resize((w, h), Image.BOX), np.float32)


# ---- BH1 接縫修補（複製自 hd_postfix.py，在灰階上跑；a 是 (h, w, 4)，前三個通道相同） ----
def level_wrap(a, n):
    h, w = a.shape[:2]
    k = 3
    left, right = a[:, :k, :3].mean(axis=1), a[:, w - k:, :3].mean(axis=1)
    both = (a[:, :k, 3].mean(axis=1) > 0) & (a[:, w - k:, 3].mean(axis=1) > 0)
    d = (right - left) * both[:, None]
    r = 6
    box = np.ones(2 * r + 1) / (2 * r + 1)
    d = np.stack([np.convolve(d[:, c], box, mode="same") for c in range(3)], axis=1)
    ramp = 1 - (np.arange(n) + 0.5) / n
    b = a.copy()
    op = (a[..., 3] > 0)[..., None]
    b[:, :n, :3] += (d[:, None, :] / 2) * ramp[None, :, None] * op[:, :n]
    b[:, w - n:, :3] -= (d[:, None, :] / 2) * ramp[::-1][None, :, None] * op[:, w - n:]
    return b


def blend_wrap(a, n, inset=2):
    h, w = a.shape[:2]
    b = a.copy()
    for k in range(inset):
        b[:, k] = b[:, inset]
        b[:, w - 1 - k] = b[:, w - 1 - inset]
    idx = np.arange(-n, n)
    weight = 0.5 * (1 + np.cos(np.pi * idx / n))
    outside = 1 - weight
    best, best_cost = None, None
    for c in range(n + inset, w - n - inset, 4):
        cost = (np.abs(b[:, c + idx, :3] - b[:, (w + idx) % w, :3]).sum(axis=(0, 2)) * outside).sum()
        if best_cost is None or cost < best_cost:
            best, best_cost = c, cost
    o = b.copy()
    for k, t in zip(idx, weight):
        x = (w + k) % w
        srcc, dst = b[:, best + k], b[:, x]
        op = ((srcc[:, 3] > 0) & (dst[:, 3] > 0))[:, None]
        o[:, x, :3] = np.where(op, dst[:, :3] * (1 - t) + srcc[:, :3] * t, dst[:, :3])
    return o


def fade_rows(a, rows, top):
    h = a.shape[0]
    t = (np.arange(rows) + 0.5) / rows
    b = a.copy()
    if top:
        b[:rows, :, :3] *= t[:, None, None]
    else:
        b[h - rows:, :, :3] *= t[::-1][:, None, None]
    return b


# BH1 索引：(level_wrap 欄數, 頂端漸暗列數, 底端漸暗列數, blend_wrap 欄數)，同 hd_postfix.py
SEAM = {0: (40, 0, 0, 64), 1: (24, 0, 0, 0), 2: (64, 20, 0, 96), 3: (40, 0, 20, 0)}


def to_bits(gray):
    if post.startswith("thr:"):
        return gray >= float(post[4:])
    if post == "bayer4":
        h, w = gray.shape
        t = BAYER4[(np.arange(h) % 4)[:, None], (np.arange(w) % 4)[None, :]]
        return gray / 255.0 > t
    raise SystemExit(f"不認得的轉換法 {post}")


def save_plane(bank, i, bits, vis):
    d = os.path.join(out, bank)
    os.makedirs(d, exist_ok=True)
    v = bits.astype(np.uint8) * 255
    al = np.where(vis, 255, 0).astype(np.uint8)
    fp = os.path.join(d, f"{i:04d}.png")
    Image.fromarray(np.dstack([v, v, v, al])).save(fp, optimize=True)
    return fp


def save_gray(bank, i, arr):
    d = os.path.join(out, bank)
    os.makedirs(d, exist_ok=True)
    fp = os.path.join(d, f"{i:04d}.png")
    Image.fromarray(arr.astype(np.uint8)).save(fp, optimize=True)
    return fp


def sd_metrics(bits, vis, plane, sil):
    """HD 結果縮回單色像素格（2x4 面積平均）後與原版圖比：輪廓 IoU、亮度相關、亮點比例差。"""
    h, w = plane.shape
    lo = resize_f(bits.astype(np.float32) * 255, w, h)
    al = resize_f(vis.astype(np.float32) * 255, w, h) >= 128
    union = (al | sil).sum()
    iou = float((al & sil).sum() / union) if union else 1.0
    a, b = lo[sil], plane[sil].astype(np.float32) * 255
    corr = None if a.size < 4 or a.std() < 1e-3 or b.std() < 1e-3 else float(np.corrcoef(a, b)[0, 1])
    fa, fb = float(a.mean() / 255) if a.size else 0.0, float(b.mean() / 255) if b.size else 0.0
    return {"iou": round(iou, 4), "luma_corr": None if corr is None else round(corr, 3), "white_hd": round(fa, 3), "white_sd": round(fb, 3)}


man = {"tool": TOOL, "post": post, "seams": seams, "style_file": sheets_meta.get("style_file"), "style_sha256": sheets_meta.get("style_sha256"),
       "items": {}, "masks": {}, "sheets": {}}
os.makedirs(os.path.join(out, "review"), exist_ok=True)
os.makedirs(os.path.join(root, "review"), exist_ok=True)

for s in plan["sheets"]:
    mp = os.path.join(root, "masters", s["name"] + ".png")
    if not os.path.exists(mp):
        print("沒有原稿，跳過", s["name"])
        continue
    im = Image.open(mp).convert("RGB")
    if list(im.size) != s["canvas"]:
        man["sheets"][s["name"]] = {"error": f"尺寸 {list(im.size)} != {s['canvas']}"}
        print("尺寸不符", s["name"], im.size, s["canvas"])
        continue
    master = np.array(im)
    z = s["zoom"]
    man["sheets"][s["name"]] = {"master_sha256": sha256(mp), "ref_sha256": SM[s["name"]]["ref_sha256"], "extra_sha256": SM[s["name"]].get("extra_sha256"),
                                "prompt_sha256": SM[s["name"]]["prompt_sha256"], "zoom": z, "receipt": receipts.get(s["name"])}
    for p in s["pieces"]:
        w, h = p["w"], p["h"]
        crop = master[p["y"]:p["y"] + h * z, p["x"]:p["x"] + w * z]
        hdw, hdh = p["hd"]
        sil = S.silhouette(p)
        plane = S.plane(p["bank"], p["index"])
        if p["opaque"]:
            rgb = fix_opaque(crop, CHROMA, z)
            alpha = np.ones(crop.shape[:2], np.float32)
        else:
            alpha = key_alpha(crop, CHROMA)
            rgb = despill(crop, alpha, CHROMA)
            allow = np.repeat(np.repeat(dilate(sil, 1), z, axis=0), z // 2, axis=1)
            alpha = alpha * allow
        g = luma(rgb)
        gp = resize_f(g * alpha, hdw, hdh)
        ap = resize_f(alpha, hdw, hdh)
        gray = np.where(ap > 0.003, gp / np.maximum(ap, 0.003), 0)
        if seams and p["bank"] == "BH1.BDB" and p["index"] in SEAM:
            n, top, bot, blend = SEAM[p["index"]]
            a4 = np.dstack([gray, gray, gray, ap * 255])
            a4 = level_wrap(a4, n)
            if blend:
                a4 = blend_wrap(a4, blend)
            if top:
                a4 = fade_rows(a4, top, True)
            if bot:
                a4 = fade_rows(a4, bot, False)
            gray = np.clip(a4[..., 0], 0, 255)
        vis = ap >= 0.5
        bits = to_bits(gray) & vis
        rec = {"method": f"mono-codex-redraw+{post}", "sheet": s["name"], "zoom": z, "hd": [hdw, hdh], "dy": p["dy"]}
        # 遮罩
        if p.get("mask"):
            mb, mi = p["mask"].split(":")
            mi = int(mi)
            mh = S.item(mb, mi)["height"] * 4
            r = 0 if mb == p["bank"] else 2
            cov = dilate(vis, r) if r else vis
            mk = np.full((mh, hdw), 255, np.uint8)
            yo = p["dy"] * 4
            hh = min(cov.shape[0], mh - yo)
            if hh > 0:
                mk[yo:yo + hh] = np.where(cov[:hh], 0, 255)
            # 圖的亮點只能在遮罩 0 處（AND 之後才 OR，其餘會被原版遮罩切掉）
            inside = mk[yo:yo + hdh] == 0 if yo + hdh <= mh else np.pad(mk[yo:] == 0, ((0, yo + hdh - mh), (0, 0)))
            bits &= inside[:hdh]
            if mb == p["bank"]:
                # BH1 第 4 項：遮罩本身就是一張圖（0 是山）
                fp = save_plane(mb, mi, mk >= 128, np.ones(mk.shape, bool))
            else:
                fp = save_gray(mb, mi, mk)
            man["masks"][f"{mb}:{mi}"] = {"method": "mono-mask-from-hd-alpha", "path": os.path.relpath(fp, out), "sha256": sha256(fp), "size": [hdw, mh],
                                          "from": f"{p['bank']}:{p['index']}", "sheet": s["name"]}
        fp = save_plane(p["bank"], p["index"], bits, vis if not p["opaque"] else np.ones(bits.shape, bool))
        rec.update(sd_metrics(bits, vis if not p["opaque"] else np.ones(bits.shape, bool), plane, sil))
        rec.update({"path": os.path.relpath(fp, out), "sha256": sha256(fp), "bright_fraction": round(float(bits.mean()), 3)})
        man["items"][f"{p['bank']}:{p['index']}"] = rec

# 第三關面板：整張轉 1 位元後，每種圖塊取第一次出現的位置切回（抖動晶格以面板原點對齊，32x40 的圖塊與 4x4 週期整除，所以接得上）
m = plan["map"]
mp = os.path.join(root, "masters", m["name"] + ".png")
if os.path.exists(mp):
    im = Image.open(mp).convert("RGB")
    if list(im.size) != m["canvas"]:
        man["sheets"][m["name"]] = {"error": f"尺寸 {list(im.size)} != {m['canvas']}"}
        print("尺寸不符", m["name"])
    else:
        z = m["zoom"]
        master = np.array(im)
        crop = master[m["y"]:m["y"] + m["h"] * z, m["x"]:m["x"] + m["w"] * z]
        crop = np.clip(fix_opaque(crop, CHROMA, z), 0, 255)
        hdw, hdh = m["w"] * 4, m["h"] * 4
        gp = resize_f(luma(crop), hdw, hdh) if (hdw, hdh) != crop.shape[1::-1] else luma(crop)
        bits = to_bits(gp)
        man["sheets"][m["name"]] = {"master_sha256": sha256(mp), "ref_sha256": SM[m["name"]]["ref_sha256"], "extra_sha256": SM[m["name"]].get("extra_sha256"),
                                    "prompt_sha256": SM[m["name"]]["prompt_sha256"], "zoom": z, "receipt": receipts.get(m["name"])}
        d = os.path.join(out, "MAP.BDB")
        os.makedirs(d, exist_ok=True)
        full = bits.astype(np.uint8) * 255
        Image.fromarray(full).save(os.path.join(out, "review", m["name"] + ".panel.png"), optimize=True)
        n_tiles = 0
        corr = []
        for t, (r, c) in m["tiles_first"].items():
            t = int(t)
            y0, x0 = (r - m["r0"]) * 40, (c - m["c0"]) * 32
            tile = full[y0:y0 + 40, x0:x0 + 32]
            fp = os.path.join(d, f"{t:04d}.png")
            Image.fromarray(tile).save(fp, optimize=True)
            sd = S.plane("MAP.BDB", t).astype(np.float32) * 255
            lo = resize_f(tile.astype(np.float32), 16, 10)
            if lo.std() > 1e-3 and sd.std() > 1e-3:
                corr.append(float(np.corrcoef(lo.ravel(), sd.ravel())[0, 1]))
            man["items"][f"MAP.BDB:{t}"] = {"method": f"mono-codex-redraw+{post}", "sheet": m["name"], "from_cell": [r, c], "path": os.path.relpath(fp, out), "sha256": sha256(fp)}
            n_tiles += 1
        man["sheets"][m["name"]]["tiles"] = n_tiles
        man["sheets"][m["name"]]["tile_corr_median"] = round(float(np.median(corr)), 3) if corr else None
        print("地圖圖塊", n_tiles, "相關中位數", man["sheets"][m["name"]]["tile_corr_median"])

# 嚴格 1 位元檢查：讀回每個輸出檔，RGB 三通道相同且只有 0 或 255，尺寸等於圖庫項的預期尺寸
checked, bad = 0, []
for dp, dn, fn in os.walk(out):
    if os.path.basename(dp) == "review":
        continue
    bank = os.path.basename(dp)
    for f in fn:
        a = np.array(Image.open(os.path.join(dp, f)).convert("RGBA"))
        rgb = a[..., :3]
        ok = bool(((rgb == 0) | (rgb == 255)).all() and (rgb[..., 0] == rgb[..., 1]).all() and (rgb[..., 1] == rgb[..., 2]).all())
        it = S.item(bank, int(f[:4]))
        ok &= a.shape[1] == it["stride"] * 16 and a.shape[0] in (it["height"] * 4,)
        checked += 1
        if not ok:
            bad.append(f"{bank}/{f}")
man["strict_1bit_check"] = {"files": checked, "violations": bad}
print("嚴格 1 位元與尺寸檢查", checked, "個檔，違規", len(bad), bad[:5])
json.dump(man, open(os.path.join(root, "mono-manifest.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
low = sorted((v["luma_corr"], k) for k, v in man["items"].items() if v.get("luma_corr") is not None)
print("切出", len(man["items"]), "張，遮罩", len(man["masks"]))
print("最低 IoU", sorted((v["iou"], k) for k, v in man["items"].items() if "iou" in v)[:6])
print("最低亮度相關", low[:8])
