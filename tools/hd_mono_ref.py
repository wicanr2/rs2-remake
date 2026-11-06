#!/usr/bin/env python3
"""依 <批次>/plan.json 產生單色 HD 試稿的參考表與提示詞。不呼叫任何外部服務。

用法：hd_mono_ref.py <original/rs-2> <workspace> <批次名> <風格檔> [名稱子字串...]
輸出：<批次>/ref/<名>.png        第一張參考圖：原版單色圖最近鄰放大（白亮點、黑背景），洋紅底色
      <批次>/ref/<名>.extra.png  第二張參考圖：同位置的彩色 HD 圖轉灰階（只有尺寸相符的項才有）
      <批次>/prompts/<名>.txt、sheets.json
風格檔可含 {minline}、{pitch}、{reduce}，依每張表的倍率代入：表會縮到 4/z 才轉 1 位元，線寬與網點要夠粗才活得下來。
"""
import hashlib
import json
import math
import os
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_mono_lib import CHROMA, CHROMA_HEX, CHROMA_NAME, MonoSource, render_ref_piece, sha256  # noqa: E402

src, ws, batch, style_path = sys.argv[1:5]
filters = sys.argv[5:]
root = os.path.join(ws, "hd", batch)
assets = os.path.join(ws, "hd", "assets")
os.makedirs(os.path.join(root, "ref"), exist_ok=True)
os.makedirs(os.path.join(root, "prompts"), exist_ok=True)
S = MonoSource(src)
plan = json.load(open(os.path.join(root, "plan.json"), encoding="utf-8"))
style_raw = open(style_path, encoding="utf-8").read().strip()

PROMPT = """Use your built-in image generation tool exactly once to create one {W}x{H} image. Use the attached image{plural} as reference. Do not run shell commands and do not edit files.

Image 1 is a {W}x{H} reference sheet of game art from RS-2, a 1990 side-scrolling science-fiction shooter for DOS by the Taiwanese developer Softstar. It shows the original 1-bit monochrome (CGA and Hercules) version of the art, enlarged with nearest-neighbor on a flat {chroma_name} #{chroma} background: white pixels on black, where regular dither patterns stand for mid-grey. Pieces: {desc}.{extra_para}

This is NOT an upscale and NOT a dither cleanup. Repaint every piece by hand as a brand-new high-resolution illustration of the same subject, inventing convincing fine detail where the low-resolution reference only hints at it.

Hard constraints, because each piece is cut back out at its original position:
- Keep every piece at exactly the same position, size and outer silhouette as in image 1. Do not move, resize, rotate, mirror, crop or reorder anything, and do not let anything spill outside its outline.
- Where the outline in image 1 is a blocky stair-step caused by the enlargement, draw a clean smooth curve that stays within one enlarged pixel of it.
- Keep the same pose, proportions and composition of each piece. Bright areas of image 1 stay bright and dark areas stay dark; a dither pattern means a mid-tone area. Black areas inside a piece are part of the art: keep them dark and painted, not transparent.
- Pieces that repeat with small differences are animation frames or size variants of the same thing: keep them identical except for the parts that differ in image 1.
- Keep the flat {chroma_name} #{chroma} background everywhere outside the pieces, with no shadow, glow or texture on it, and do not use that colour inside the pieces.
- No text, no signature, no new objects, no frames around the sheet.

{style}
"""
EXTRA_SPRITES = (" Image 2 is the same sheet with the colour version's redraws of the same pieces, converted to grey and placed at identical positions. "
                 "Use image 2 only for design detail such as panel lines, materials and highlights. Where image 2 and image 1 disagree about the silhouette "
                 "or about which areas are light and dark, follow image 1. A piece that is blank in image 2 has no counterpart there.")
EXTRA_MAP = (" Image 2 is a crop of the colour version's redraw of the same fortress at the same place, converted to grey. Its layout differs slightly. "
             "Use it only for design detail such as panel lines, materials and highlights, and follow image 1 for every shape and every light or dark area.")

sheets = []


def style_for(z):
    return style_raw.format(minline=max(2, round(0.4 * z)), pitch=max(4, round(0.8 * z)), reduce=f"{round(400 / z)}%")


def grey_of(rgb):
    return (rgb[..., 0] * 0.299 + rgb[..., 1] * 0.587 + rgb[..., 2] * 0.114)


def extra_piece(canvas, p, z):
    c = p["colour"]
    im = Image.open(os.path.join(assets, c["lib"], f"{c['index']:04d}.png")).convert("RGBA")
    tw, th = p["w"] * z, p["h"] * z
    im = im.resize((tw, th), Image.LANCZOS)
    a = np.array(im, np.float32)
    g = grey_of(a[..., :3])
    al = a[..., 3:4] / 255
    out = g[..., None] * al + np.array(CHROMA, np.float32) * (1 - al)
    canvas[p["y"]:p["y"] + th, p["x"]:p["x"] + tw] = np.clip(out, 0, 255).astype(np.uint8)


def write(s, canvas, extra, desc, extra_para):
    W, H = s["canvas"]
    z = s["zoom"]
    path = os.path.join(root, "ref", s["name"] + ".png")
    Image.fromarray(canvas).save(path, optimize=True)
    rec = {"name": s["name"], "ref_sha256": sha256(path)}
    if extra is not None:
        ep = os.path.join(root, "ref", s["name"] + ".extra.png")
        Image.fromarray(extra).save(ep, optimize=True)
        rec["extra_sha256"] = sha256(ep)
    txt = PROMPT.format(W=W, H=H, plural="s" if extra is not None else "", chroma=CHROMA_HEX, chroma_name=CHROMA_NAME, desc=desc,
                        extra_para=extra_para if extra is not None else "", style=style_for(z))
    pp = os.path.join(root, "prompts", s["name"] + ".txt")
    open(pp, "w", encoding="utf-8").write(txt)
    rec["prompt_sha256"] = sha256(pp)
    sheets.append(rec)


for s in plan["sheets"]:
    if filters and not any(f in s["name"] for f in filters):
        continue
    W, H = s["canvas"]
    z = s["zoom"]
    canvas = np.zeros((H, W, 3), np.uint8)
    canvas[:] = CHROMA
    extra = np.zeros((H, W, 3), np.uint8)
    extra[:] = CHROMA
    any_extra = False
    for p in s["pieces"]:
        render_ref_piece(canvas, p, z, S)
        if p["colour"]:
            extra_piece(extra, p, z)
            any_extra = True
    write(s, canvas, extra if any_extra else None, s["desc"], EXTRA_SPRITES)

m = plan["map"]
if m and (not filters or any(f in m["name"] for f in filters)):
    W, H = m["canvas"]
    z = m["zoom"]
    raw_grid = None
    import struct
    from rs2_common import unrle
    raw_grid = unrle(open(os.path.join(src, "SHT.SCR"), "rb").read(), 0)[0]
    G = np.array(struct.unpack_from("<12000H", raw_grid, 0), np.int32).reshape(20, 600)
    canvas = np.zeros((H, W, 3), np.uint8)
    canvas[:] = CHROMA
    pw, ph = z // 2, z
    for r in range(m["r0"], m["r1"] + 1):
        for c in range(m["c0"], m["c1"] + 1):
            t = int(G[r, c])
            pl = S.plane("MAP.BDB", t) * 255
            big = np.repeat(np.repeat(pl, ph, axis=0), pw, axis=1)
            yy = m["y"] + (r - m["r0"]) * 10 * z
            xx = m["x"] + (c - m["c0"]) * 8 * z
            canvas[yy:yy + big.shape[0], xx:xx + big.shape[1]] = big[..., None]
    full = Image.open(os.path.join(assets, "maps", "SHT.SCN.hd.png")).convert("RGB")
    tw, th = 8 * 4, 10 * 4
    crop = full.crop((m["c0"] * tw, m["r0"] * th, (m["c1"] + 1) * tw, (m["r1"] + 1) * th))
    crop = crop.resize((m["w"] * z, m["h"] * z), Image.LANCZOS)
    g = grey_of(np.array(crop, np.float32))
    extra = np.zeros((H, W, 3), np.uint8)
    extra[:] = CHROMA
    extra[m["y"]:m["y"] + g.shape[0], m["x"]:m["x"] + g.shape[1]] = np.clip(g, 0, 255).astype(np.uint8)[..., None]
    write({"name": m["name"], "canvas": m["canvas"], "zoom": z}, canvas, extra,
          m["desc"] + ". Black areas inside the rectangle are the empty void of space: keep them pure black", EXTRA_MAP)

json.dump({"batch": batch, "style_file": os.path.basename(style_path), "style_sha256": hashlib.sha256(style_raw.encode()).hexdigest(), "sheets": sheets},
          open(os.path.join(root, "sheets.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print("參考表", len(sheets))
