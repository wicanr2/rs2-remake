#!/usr/bin/env python3
"""把單色 HD 全量重繪（m2）的 Codex 原稿切回每個圖庫項，轉成嚴格 1 位元，輸出候選成品根目錄。不呼叫外部服務。

用法：hd_mono_full_cut.py <original/rs-2> <workspace> <輸出資料夾名> <批次> [<批次> ...]
  批次依序列出，後面的批次覆蓋前面的（第一個是 full-mono-m2-r1，提供 plan.json 與 classes.json）。
  輸出在 workspace/hd/<輸出資料夾名>/：
    mono/<圖庫檔名>/<索引>.png  與 workspace/hd/assets/mono 同結構，共 1,526 個檔
    mono-manifest.json          每個檔的方法、來源與提示詞雜湊、Codex 版本與 session、輸出雜湊、量測
    review/                     每張表的原稿與結果對照
轉 1 位元：藝術圖用 4x4 Bayer 有序抖動（週期 4 整除位元組寬 16 與掃描線高 4，圖塊擺在任何位置晶格都對齊），
文字類（TEXT_GROUPS）用閾值 128，因為抖動會把字的筆畫打成點線。
亮度規則：子彈、道具、爆炸與閃光（BRIGHT_ROLES）轉完後，亮點比例若低於原版的 0.8 倍，就對灰階提高 gamma 直到達標，記在 manifest 的 tone_gamma。
"""
import hashlib
import json
import os
import struct
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_mono_full_lib import BRIGHT_ROLES, CHROMA, CREDITS, IMG_LIB, MASK_BANK, TEXT_GROUPS, Full, mask_best_offset, nearest_hd, sha256  # noqa: E402
from rs2_common import unrle  # noqa: E402

TOOL = "hd_mono_full_cut.py r1"
src, ws, outname = sys.argv[1:4]
batches = sys.argv[4:]
hd = os.path.join(ws, "hd")
root0 = os.path.join(hd, batches[0])
out_root = os.path.join(hd, outname)
out = os.path.join(out_root, "mono")
S = Full(src)
plan = json.load(open(os.path.join(root0, "plan.json"), encoding="utf-8"))
cls = json.load(open(os.path.join(root0, "classes.json"), encoding="utf-8"))
style_names = {}

# 每張表取最後一個有原稿的批次
master_of, meta_of, receipt_of, style_of = {}, {}, {}, {}
for b in batches:
    rb = os.path.join(hd, b)
    sj = os.path.join(rb, "sheets.json")
    sm = {r["name"]: r for r in json.load(open(sj, encoding="utf-8"))["sheets"]} if os.path.exists(sj) else {}
    sjd = json.load(open(sj, encoding="utf-8")) if os.path.exists(sj) else {}
    rec = {}
    rp = os.path.join(rb, "logs", "_receipts.tsv")
    if os.path.exists(rp):
        for ln in open(rp, encoding="utf-8"):
            f = ln.rstrip("\n").split("\t")
            if len(f) >= 4:
                rec[f[0]] = {"session": f[1], "master_sha256": f[2], "codex": f[3]}
    for fn in os.listdir(os.path.join(rb, "masters")) if os.path.isdir(os.path.join(rb, "masters")) else []:
        if fn.endswith(".png"):
            n = fn[:-4]
            master_of[n] = os.path.join(rb, "masters", fn)
            meta_of[n] = sm.get(n, {})
            receipt_of[n] = rec.get(n)
            style_of[n] = {"batch": b, "style_file": sjd.get("style_file"), "style_sha256": sjd.get("style_sha256")}

BAYER4 = (np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]], np.float32) + 0.5) / 16
LUMA = np.array([0.299, 0.587, 0.114], np.float32)
# BH 循環背景的接縫修補（BH1 同試稿 --seams，BH2、BH4 同彩色版 hd_postfix.py）：(level_wrap 欄數, 頂端漸暗列數, 底端漸暗列數, blend_wrap 欄數)
SEAM = {("BH1.BDB", 0): (40, 0, 0, 64), ("BH1.BDB", 2): (64, 20, 0, 96), ("BH1.BDB", 3): (40, 0, 20, 0),
        ("BH2.BDB", 0): (40, 0, 0, 48), ("BH2.BDB", 1): (40, 0, 0, 48), ("BH4.BDB", 0): (40, 0, 0, 64)}
SEAMLESS_X = {("BH1.BDB", 1)}   # 32 像素寬的窄條，橫向重複：用半幅位移交叉淡化做成無縫
CRUSH = {("SHT1.BDB", 33), ("SHT1.BDB", 34), ("SHT1.BDB", 35)}  # 頭目本體：接近黑的壓成純黑（同 hd_postfix.py）


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
    o = b.copy()
    for _ in range(n):
        p = o.copy()
        p[1:] |= o[:-1]
        p[:-1] |= o[1:]
        p[:, 1:] |= o[:, :-1]
        p[:, :-1] |= o[:, 1:]
        o = p
    return o


def dilate_xy(b, dx, dy):
    """水平 dx 個像素、垂直 dy 個像素的矩形膨脹。"""
    o = b.copy()
    for _ in range(dx):
        p = o.copy()
        p[:, 1:] |= o[:, :-1]
        p[:, :-1] |= o[:, 1:]
        o = p
    for _ in range(dy):
        p = o.copy()
        p[1:] |= o[:-1]
        p[:-1] |= o[1:]
        o = p
    return o


def luma(rgb):
    return rgb[..., 0] * 0.299 + rgb[..., 1] * 0.587 + rgb[..., 2] * 0.114


def resize_f(a, w, h):
    """float 陣列縮放：縮小用面積平均，放大用雙三次。"""
    im = Image.fromarray(a.astype(np.float32))
    up = w > a.shape[1] or h > a.shape[0]
    return np.array(im.resize((w, h), Image.BICUBIC if up else Image.BOX), np.float32)


def idx_up(a, oh, ow):
    """最近鄰把陣列對應到 oh x ow（任意倍率）。"""
    h, w = a.shape[:2]
    return a[(np.arange(oh) * h // oh)][:, (np.arange(ow) * w // ow)]


# ---- 接縫修補（複製自 hd_postfix.py，在灰階上跑；a 是 (h, w, 4)，前三個通道相同） ----
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


def seamless_x(g):
    """橫向無縫且重複時看不出圖樣：每一列取整列的平均（Codex 畫的煙有直條與頂緣起伏，半幅位移交叉淡化後在每個圖塊中央留下規律的凸起，
    實機重複時像一排城垛，所以只留亮度隨高度變化的漸層），再沿垂直方向以 9 列的方塊平均弄平。結果只隨列變化，左右一定接得上。"""
    h, w = g.shape
    m = g.mean(axis=1)
    k = 9
    pad = np.pad(m, (k // 2, k // 2), mode="edge")
    m = np.convolve(pad, np.ones(k) / k, mode="valid")
    return np.repeat(m[:, None], w, axis=1)


def crush_black(g, limit):
    k = np.clip((g - limit) / limit, 0, 1)
    return g * k


def bayer_bits(gray):
    h, w = gray.shape
    t = BAYER4[(np.arange(h) % 4)[:, None], (np.arange(w) % 4)[None, :]]
    return gray / 255.0 > t


def thr_bits(gray):
    return gray >= 128


def save_plane(bank, i, bits, vis=None):
    d = os.path.join(out, bank)
    os.makedirs(d, exist_ok=True)
    v = bits.astype(np.uint8) * 255
    al = np.full(bits.shape, 255, np.uint8) if vis is None else np.where(vis, 255, 0).astype(np.uint8)
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
    """HD 結果縮回單色像素格（2x4 面積平均）後與原版圖比：輪廓 IoU、亮度相關、亮點比例。"""
    h, w = plane.shape
    lo = resize_f(bits.astype(np.float32) * 255, w, h)
    al = resize_f(vis.astype(np.float32) * 255, w, h) >= 128
    union = (al | sil).sum()
    iou = float((al & sil).sum() / union) if union else 1.0
    a, b = lo[sil], plane[sil].astype(np.float32) * 255
    corr = None if a.size < 4 or a.std() < 1e-3 or b.std() < 1e-3 else float(np.corrcoef(a, b)[0, 1])
    fa, fb = float(a.mean() / 255) if a.size else 0.0, float(b.mean() / 255) if b.size else 0.0
    return {"iou": round(iou, 4), "luma_corr": None if corr is None else round(corr, 3), "white_hd": round(fa, 3), "white_sd": round(fb, 3)}


man = {"tool": TOOL, "batches": batches, "plan": os.path.join(batches[0], "plan.json"), "files": {}, "sheets": {}, "notes": {}}
os.makedirs(os.path.join(out_root, "review"), exist_ok=True)
files = man["files"]
res = {}  # (bank, i) -> (bits, vis) HD 陣列，給相位與遮罩用
sd_sha = {}


def item_sha(bank, i):
    k = (bank, i)
    if k not in sd_sha:
        sd_sha[k] = hashlib.sha256(bytes(S.item(bank, i)["data"])).hexdigest()
    return sd_sha[k]


def rec_for(sheet_name):
    m = meta_of.get(sheet_name, {})
    return {"sheet": sheet_name, "batch": style_of.get(sheet_name, {}).get("batch"), "style_file": style_of.get(sheet_name, {}).get("style_file"),
            "ref_sha256": m.get("ref_sha256"), "extra_ref_sha256": m.get("extra_sha256"), "prompt_sha256": m.get("prompt_sha256"),
            "master_sha256": sha256(master_of[sheet_name]), "codex": (receipt_of.get(sheet_name) or {}).get("codex"),
            "session": (receipt_of.get(sheet_name) or {}).get("session")}


def put(bank, i, fp, method, **kw):
    r = {"method": method, "path": os.path.relpath(fp, out), "sd_item_sha256": item_sha(bank, i), "sha256": sha256(fp)}
    r.update(kw)
    files[f"{bank}:{i}"] = r


# ---------- 1. 參考表上的圖 ----------
todo_missing = []
for s in plan["sheets"]:
    name = s["name"]
    if name not in master_of:
        todo_missing.append(name)
        continue
    im = Image.open(master_of[name]).convert("RGB")
    if list(im.size) != s["canvas"]:
        man["sheets"][name] = {"error": f"尺寸 {list(im.size)} != {s['canvas']}"}
        todo_missing.append(name)
        continue
    master = np.array(im)
    z = s["zoom"]
    man["sheets"][name] = {**rec_for(name), "zoom": z}
    for p in s["pieces"]:
        bank, i = p["bank"], p["index"]
        w, h = p["w"], p["h"]
        hdw, hdh = p["hd"]
        crop = master[p["y"]:p["y"] + h * z, p["x"]:p["x"] + w * z]
        plane = S.plane(bank, i)
        sil = S.silhouette(p)
        if p["opaque"]:
            rgb = fix_opaque(crop, CHROMA, z)
            alpha = np.ones(crop.shape[:2], np.float32)
        else:
            alpha = key_alpha(crop, CHROMA)
            rgb = despill(crop, alpha, CHROMA)
            if not p["free"]:
                allow = idx_up(dilate(sil, 1), h * z, w * z)
                alpha = alpha * allow
        g = luma(rgb)
        gp = resize_f(g * alpha, hdw, hdh)
        ap = resize_f(alpha, hdw, hdh)
        gray = np.where(ap > 0.003, gp / np.maximum(ap, 0.003), 0)
        extra = {}
        if (bank, i) in SEAM or (bank, i) in SEAMLESS_X or (bank, i) in CRUSH:
            if (bank, i) in SEAM:
                n, top, bot, blend = SEAM[(bank, i)]
                a4 = np.dstack([gray, gray, gray, ap * 255])
                a4 = level_wrap(a4, n)
                if blend:
                    a4 = blend_wrap(a4, blend)
                if top:
                    a4 = fade_rows(a4, top, True)
                if bot:
                    a4 = fade_rows(a4, bot, False)
                gray = np.clip(a4[..., 0], 0, 255)
                extra["seam"] = {"level_wrap": n, "blend_wrap": blend, "fade_top": top, "fade_bottom": bot}
            if (bank, i) in SEAMLESS_X:
                gray = np.clip(seamless_x(gray), 0, 255)
                extra["seam"] = {"seamless_x": "row-mean vertical gradient (box 9 rows); the half-width cross-fade left a visible periodic bump"}
            if (bank, i) in CRUSH:
                gray = crush_black(gray, 18)
                extra["crush_black"] = 18
        vis = ap >= 0.5
        text = p.get("group") in TEXT_GROUPS
        conv = thr_bits if text else bayer_bits
        bits = conv(gray) & vis
        gamma = None
        if p.get("bright") and not text:
            sd_frac = float(plane[sil].mean()) if sil.any() else 0.0
            target = 0.8 * sd_frac
            frac = float(bits[vis].mean()) if vis.any() else 0.0
            if frac < target:
                for gm in (0.9, 0.8, 0.7, 0.6, 0.5, 0.42, 0.35):
                    g2 = 255.0 * (np.clip(gray, 0, 255) / 255.0) ** gm
                    b2 = conv(g2) & vis
                    gamma = gm
                    bits = b2
                    if float(b2[vis].mean()) >= target:
                        break
        if p["free"]:
            # 開場飛船與噴焰：物件以外維持原版矩形內容（雲層）
            sdb = nearest_hd(plane) >= 128
            bits = np.where(vis, bits, sdb)
            vis_out = np.ones(bits.shape, bool)
        else:
            vis_out = vis if not p["opaque"] else np.ones(bits.shape, bool)
        res[(bank, i)] = (bits, vis_out)
        fp = save_plane(bank, i, bits, vis_out)
        m = sd_metrics(bits, vis_out, plane, sil)
        m["bright_fraction"] = round(float(bits.mean()), 3)
        if gamma:
            m["tone_gamma"] = gamma
        method = "mono-codex-redraw+" + ("thr128" if text else "bayer4") + ("+sd-overlay" if p["free"] else "")
        put(bank, i, fp, method, **rec_for(name), zoom=z, metrics={**m, **extra})

# ---------- 2. 相位：BH2 像素位移 ----------
for k, v in sorted(cls.items()):
    if v["class"] != "derived-shift":
        continue
    bank, i = v["bank"], v["index"]
    base, shift, circ = v["phase_base"], v["phase_shift"], v["phase_circular"]
    if (bank, base) not in res:
        continue
    bb, bv = res[(bank, base)]
    kh = shift * 2
    if circ:
        bits, vis = np.roll(bb, kh, axis=1), np.roll(bv, kh, axis=1)
    else:
        W = v["hd"][0]
        bits = np.zeros((bb.shape[0], W), bool)
        vis = np.zeros((bb.shape[0], W), bool)
        ww = min(bb.shape[1], W - kh)
        bits[:, kh:kh + ww] = bb[:, :ww]
        vis[:, kh:kh + ww] = bv[:, :ww]
    res[(bank, i)] = (bits, vis)
    fp = save_plane(bank, i, bits, vis)
    put(bank, i, fp, "mono-derived-shift", base=f"{bank}:{base}", shift_mono_px=shift, shift_hd_px=kh, circular=circ, match_sd=v["phase_match"])

# 吻合率低於門檻、但仍要與基礎圖保持同一個相位圖樣的項：BH2 第 18 項是第 17 項的第二個相位，單色資料對基礎圖循環位移 8 個像素的吻合率只有 0.861，
# 差異都在柱邊的 8 欄（原版手工修過）。分開重繪會讓兩個相位的雜訊不同，捲動時閃爍，所以改成由第 17 項在 HD 位移 16 像素衍生。
for (bank, i), (base, shift) in {("BH2.BDB", 18): (17, 8)}.items():
    if (bank, base) not in res:
        continue
    bb, bv = res[(bank, base)]
    bits, vis = np.roll(bb, shift * 2, axis=1), np.roll(bv, shift * 2, axis=1)
    res[(bank, i)] = (bits, vis)
    fp = save_plane(bank, i, bits, vis)
    put(bank, i, fp, "mono-derived-shift", base=f"{bank}:{base}", shift_mono_px=shift, shift_hd_px=shift * 2, circular=True, match_sd=0.861,
        note="吻合率低於 0.93 門檻，為保持相位一致仍由基礎圖衍生；Codex 對這項的重繪不採用")

# ---------- 3. 遮罩 ----------
n_mask_alpha = 0
mask_done = set()


def mask_from_vis(bank, i, bits, vis, mask_spec, dy, kind):
    mb, mi = mask_spec.split(":")
    mi = int(mi)
    it = S.item(mb, mi)
    mh, mw = it["height"] * 4, it["stride"] * 16
    canvas = np.zeros((mh, mw), bool)
    yo = dy * 4
    hh, ww = vis.shape
    y0, y1 = max(0, yo), min(mh, yo + hh)
    x1 = min(mw, ww)
    if y1 > y0 and x1 > 0:
        canvas[y0:y1, :x1] = vis[y0 - yo:y1 - yo, :x1]
    cov = canvas if kind == "item" else dilate_xy(canvas, 2, 4)
    mk = np.where(cov, 0, 255).astype(np.uint8)
    if kind == "item":
        fp = save_plane(mb, mi, mk >= 128)
    else:
        fp = save_gray(mb, mi, mk)
    return fp, mb, mi, mk


for s in plan["sheets"]:
    for p in s["pieces"]:
        if not p.get("mask") or (p["bank"], p["index"]) not in res or s["name"] not in master_of:
            continue
        bits, vis = res[(p["bank"], p["index"])]
        fp, mb, mi, mk = mask_from_vis(p["bank"], p["index"], bits, vis, p["mask"], p["dy"], p["mask_kind"])
        put(mb, mi, fp, "mono-mask-from-hd-alpha", **rec_for(s["name"]), from_item=f"{p['bank']}:{p['index']}", dy=p["dy"], mask_kind=p["mask_kind"],
            metrics={"clear_fraction": round(float((mk == 0).mean()), 3)})
        mask_done.add((mb, mi))
# 相位的遮罩（BH2 第 24 至 26 項，由第 20 至 22 項反推）
for k, v in sorted(cls.items()):
    if v["class"] == "item-mask" and v["bank"] == "BH2.BDB" and v["index"] in (24, 25, 26):
        pi = v["mask_of"]
        if ("BH2.BDB", pi) not in res:
            continue
        bits, vis = res[("BH2.BDB", pi)]
        plane = S.plane("BH2.BDB", pi)
        m = S.plane("BH2.BDB", v["index"])
        dy, inside, total = mask_best_offset(plane, m)
        fp, mb, mi, mk = mask_from_vis("BH2.BDB", pi, bits, vis, f"BH2.BDB:{v['index']}", dy, "item")
        put(mb, mi, fp, "mono-mask-from-hd-alpha", from_item=f"BH2.BDB:{pi}", dy=dy, mask_kind="item", note="由相位圖的 alpha 反推",
            metrics={"clear_fraction": round(float((mk == 0).mean()), 3), "sd_cover": [inside, total]})
        mask_done.add((mb, mi))

# ---------- 4. 第三關地圖 ----------
ms = plan["map"]
raw = unrle(open(os.path.join(src, "SHT.SCR"), "rb").read(), 0)[0]
G = np.array(struct.unpack_from("<12000H", raw, 0), np.int32).reshape(20, 600)
OVER = ms["overlap_columns"]
panel_by = {p["name"]: p for p in ms["panels"]}
tile_choice = {}
n_tiles = 0
map_missing = []
for o in ms["objects"]:
    if not all(pn in master_of for pn in o["panels"]):
        map_missing += [pn for pn in o["panels"] if pn not in master_of]
        continue
    cols = o["c1"] - o["c0"] + 1
    rows = o["r1"] - o["r0"] + 1
    acc = np.zeros((rows * 40, cols * 32), np.float32)
    wsum = np.zeros((cols * 32,), np.float32)
    for k, pn in enumerate(o["panels"]):
        pl = panel_by[pn]
        z = pl["zoom"]
        man["sheets"][pn] = {**rec_for(pn), "zoom": z}
        im = np.array(Image.open(master_of[pn]).convert("RGB"))
        if list(im.shape[1::-1]) != pl["canvas"]:
            map_missing.append(pn + "（尺寸不符）")
            continue
        crop = im[pl["y"]:pl["y"] + pl["h"] * z, pl["x"]:pl["x"] + pl["w"] * z]
        crop = np.clip(fix_opaque(crop, CHROMA, z), 0, 255)
        gp = luma(crop)
        gp = resize_f(gp, pl["w"] * 4, pl["h"] * 4)
        pc = pl["c1"] - pl["c0"] + 1
        wt = np.ones(pc * 32, np.float32)
        ov = OVER * 32
        if k > 0:
            wt[:ov] = (np.arange(ov) + 0.5) / ov
        if k < len(o["panels"]) - 1:
            wt[-ov:] = np.minimum(wt[-ov:], 1 - (np.arange(ov) + 0.5) / ov)
        x0 = (pl["c0"] - o["c0"]) * 32
        acc[:, x0:x0 + pc * 32] += gp * wt[None, :]
        wsum[x0:x0 + pc * 32] += wt
    gray = acc / np.maximum(wsum, 1e-3)[None, :]
    bits_obj = bayer_bits(gray)
    Image.fromarray(bits_obj.astype(np.uint8) * 255).save(os.path.join(out_root, "review", f"map-obj{o['id']:02d}.png"), optimize=True)
    # 每種圖塊在這個物件裡的所有出現位置，挑縮回單色像素格後與原版最像的
    for r in range(o["r0"], o["r1"] + 1):
        for c in range(o["c0"], o["c1"] + 1):
            t = int(G[r, c])
            if not t:
                continue
            y0, x0 = (r - o["r0"]) * 40, (c - o["c0"]) * 32
            tile = bits_obj[y0:y0 + 40, x0:x0 + 32]
            sd = S.plane("MAP.BDB", t).astype(np.float32) * 255
            lo = resize_f(tile.astype(np.float32) * 255, 16, 10)
            if lo.std() > 1e-3 and sd.std() > 1e-3:
                score = float(np.corrcoef(lo.ravel(), sd.ravel())[0, 1])
            else:
                score = -abs(float(lo.mean() - sd.mean())) / 255.0 - 1.0
            if t not in tile_choice or score > tile_choice[t][0]:
                tile_choice[t] = (score, tile.copy(), (r, c), o["id"], 1)
            else:
                tile_choice[t] = (tile_choice[t][0], tile_choice[t][1], tile_choice[t][2], tile_choice[t][3], tile_choice[t][4] + 1)
corrs = []
for t, (score, tile, rc, oid, nocc) in sorted(tile_choice.items()):
    fp = save_plane("MAP.BDB", t, tile)
    sd = S.plane("MAP.BDB", t).astype(np.float32) * 255
    lo = resize_f(tile.astype(np.float32) * 255, 16, 10)
    cc = float(np.corrcoef(lo.ravel(), sd.ravel())[0, 1]) if lo.std() > 1e-3 and sd.std() > 1e-3 else None
    if cc is not None:
        corrs.append(cc)
    pn = [pn for pn in ms["objects"][oid]["panels"]][0]
    put("MAP.BDB", t, fp, "mono-map-tile-codex+bayer4", **rec_for(pn), from_cell=list(rc), object=oid, occurrences=nocc, tile_corr=None if cc is None else round(cc, 3))
    n_tiles += 1
man["notes"]["map"] = {"tiles": n_tiles, "tile_corr_median": round(float(np.median(corrs)), 3) if corrs else None, "missing_panels": map_missing}

# ---------- 5. 其餘：最近鄰 ----------
for k, v in sorted(cls.items()):
    bank, i = v["bank"], v["index"]
    if f"{bank}:{i}" in files:
        continue
    c = v["class"]
    plane = S.plane(bank, i)
    if c in ("credits", "placeholder", "unused-nearest", "blank"):
        fp = save_plane(bank, i, nearest_hd(plane) >= 128)
        put(bank, i, fp, {"credits": "mono-credits-nearest", "placeholder": "mono-nearest-placeholder", "unused-nearest": "mono-nearest-unused", "blank": "mono-nearest-blank"}[c],
            reason=v.get("reason"))
    elif c == "mask-lib":
        fp = save_gray(bank, i, nearest_hd(plane))
        put(bank, i, fp, "mono-mask-nearest", reason="沒有重繪配對的遮罩（不透明圖、佔位或從未使用）")
    elif c == "item-mask" and (bank, i) not in mask_done:
        fp = save_plane(bank, i, nearest_hd(plane) >= 128)
        put(bank, i, fp, "mono-mask-nearest", reason="配對圖尚未重繪")

# ---------- 6. 完整性與嚴格 1 位元檢查 ----------
checked, bad = 0, []
expected = 0
for bank in sorted(os.listdir(os.path.join(hd, "assets", "mono"))):
    for f in sorted(os.listdir(os.path.join(hd, "assets", "mono", bank))):
        expected += 1
        a = os.path.join(out, bank, f)
        if not os.path.exists(a):
            bad.append(f"{bank}/{f} 缺檔")
            continue
        cur = Image.open(os.path.join(hd, "assets", "mono", bank, f))
        im = Image.open(a)
        arr = np.array(im.convert("RGBA"))
        rgb = arr[..., :3]
        ok = bool(((rgb == 0) | (rgb == 255)).all() and (rgb[..., 0] == rgb[..., 1]).all() and (rgb[..., 1] == rgb[..., 2]).all())
        if im.size != cur.size:
            bad.append(f"{bank}/{f} 尺寸 {im.size} != 現況 {cur.size}")
        elif not ok:
            bad.append(f"{bank}/{f} 非 0/255")
        checked += 1
extra_files = []
for bank in os.listdir(out):
    for f in os.listdir(os.path.join(out, bank)):
        if not os.path.exists(os.path.join(hd, "assets", "mono", bank, f)):
            extra_files.append(f"{bank}/{f}")
by_method = {}
for k, v in files.items():
    by_method[v["method"]] = by_method.get(v["method"], 0) + 1
man["check"] = {"expected": expected, "checked": checked, "manifest_entries": len(files), "violations": bad, "extra_files": extra_files, "by_method": dict(sorted(by_method.items())),
                "missing_sheets": todo_missing}
json.dump(man, open(os.path.join(out_root, "mono-manifest.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print("檔案", checked, "/", expected, "manifest", len(files), "違規", len(bad), bad[:6], "多出", extra_files[:5])
print("缺原稿的表", len(todo_missing), todo_missing[:10], "缺面板", map_missing[:5])
print("各方法", dict(sorted(by_method.items())))
print("地圖圖塊", n_tiles, "相關中位數", man["notes"]["map"]["tile_corr_median"])
low = sorted((v["metrics"]["luma_corr"], k) for k, v in files.items() if v.get("metrics", {}).get("luma_corr") is not None)
print("最低亮度相關", low[:10])
print("最低 IoU", sorted((v["metrics"]["iou"], k) for k, v in files.items() if "iou" in v.get("metrics", {}))[:8])
