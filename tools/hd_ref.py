#!/usr/bin/env python3
"""依 plan.json 產生參考表與提示詞。不呼叫任何外部服務。

用法：hd_ref.py <original/rs-2> <workspace> <批次名> <風格檔> [名稱子字串...]
輸出：workspace/hd/<批次>/ref/<名>.png、prompts/<名>.txt、sheets.json
"""
import hashlib
import json
import os
import struct
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_lib import CHROMAS, Source, sha256  # noqa: E402
from rs2_common import unrle  # noqa: E402
from hd_plan_shared import ROLE_DESC, silhouette_for  # noqa: E402

src, ws, batch, style_path = sys.argv[1:5]
filters = sys.argv[5:]
root = os.path.join(ws, "hd", batch)
os.makedirs(os.path.join(root, "ref"), exist_ok=True)
os.makedirs(os.path.join(root, "prompts"), exist_ok=True)
S = Source(src, os.path.join(ws, "unpacked"), os.path.join(ws, "extracted/manifest.json"))
plan = json.load(open(os.path.join(ws, "hd", "plan.json"), encoding="utf-8"))
usage = json.load(open(os.path.join(ws, "extracted/sprite-usage.json"), encoding="utf-8"))
style = open(style_path, encoding="utf-8").read().strip()

PROMPT = """Use your built-in image generation tool exactly once to create one {W}x{H} image. Use the attached image as the reference. Do not run shell commands and do not edit files.

Attached is a {W}x{H} reference sheet of game art from RS-2, a 1990 side-scrolling science-fiction shooter for DOS by the Taiwanese developer Softstar, enlarged with nearest-neighbor on a flat {chroma_name} #{chroma} background. Pieces: {desc}.

This is NOT an upscale and NOT anti-aliasing. Repaint every piece by hand as a brand-new high-resolution illustration of the same subject, inventing convincing fine detail where the low-resolution reference only hints at it.

Hard constraints, because each piece is cut back out at its original position:
- Keep every piece at exactly the same position, size and outer silhouette as in the reference. Do not move, resize, rotate, mirror, crop or reorder anything, and do not let anything spill outside its outline.
- Where the outline in the reference is a blocky stair-step caused by the enlargement, draw a clean smooth curve that stays within one enlarged pixel of it.
- Keep the same pose, proportions, composition and colour scheme of each piece.
- Pieces that repeat with small differences are animation frames or size variants of the same thing: keep them identical except for the parts that differ in the reference.
- Keep the flat #{chroma} background everywhere outside the pieces, with no shadow, glow or texture on it, and do not use that colour inside the pieces.{opaque_note}
- No text except the text already in the reference, no signature, no new objects, no frames around the sheet.

{style}
"""
CH_NAME = {"FF00FF": "magenta", "00FF00": "green", "00FFFF": "cyan"}
# 重畫時補充的指示，以及第二張參考圖（已畫好的相鄰面板）
EXTRA_NOTE = {
    "MAP-obj09-p1": "The rock walls on the left and right of this section are smooth rounded armored boulders, not pointed leaves or scales. Image 2 is the already painted neighbouring section of the same wall: match its boulder material, shading and teal colours exactly.",
    "MY-pod-right": "Each frame is one seamless object: paint it as a single continuous shape with no vertical joins or cuts.",
}
EXTRA_REF = {"MAP-obj09-p1": "MAP-obj09-p0"}
extra_from = os.environ.get("RS2_EXTRA_REF_BATCH")


def extra(name, txt):
    if name in EXTRA_NOTE:
        txt = txt.rstrip() + "\n\n" + EXTRA_NOTE[name] + "\n"
    return txt


def extra_ref(name):
    if name in EXTRA_REF and extra_from:
        src_p = os.path.join(ws, "hd", extra_from, "masters", EXTRA_REF[name] + ".png")
        Image.open(src_p).convert("RGB").save(os.path.join(root, "ref", name + ".extra.png"), optimize=True)
sheets = []


def render_piece(canvas, p, z, chroma):
    rgb = S.rgb(p["lib"], p["index"], p["palette"])
    sil = silhouette_for(S, p)
    h, w = rgb.shape[:2]
    big = np.repeat(np.repeat(rgb, z, axis=0), z, axis=1)
    bigm = np.repeat(np.repeat(sil, z, axis=0), z, axis=1)
    x, y = p["x"], p["y"]
    region = canvas[y:y + h * z, x:x + w * z]
    region[bigm] = big[bigm]


for s in plan["sheets"]:
    if filters and not any(f in s["name"] for f in filters):
        continue
    W, H = s["canvas"]
    chroma = CHROMAS[s["chroma"]]
    canvas = np.zeros((H, W, 3), np.uint8)
    canvas[:] = chroma
    for p in s["pieces"]:
        render_piece(canvas, p, s["zoom"], s["chroma"])
    path = os.path.join(root, "ref", s["name"] + ".png")
    Image.fromarray(canvas).save(path, optimize=True)
    desc = s["desc"] or (s["note"] and f"{ROLE_DESC.get(s['role'], 'game sprites')} ({s['note']})") or ROLE_DESC.get(s["role"], "game sprites from one stage")
    any_op = any(p["opaque"] for p in s["pieces"])
    note = " Black areas inside a piece are part of the art: keep them dark and painted, not transparent." if any_op else ""
    txt = PROMPT.format(W=W, H=H, chroma=s["chroma"], chroma_name=CH_NAME[s["chroma"]], desc=desc, style=style, opaque_note=note)
    open(os.path.join(root, "prompts", s["name"] + ".txt"), "w", encoding="utf-8").write(extra(s["name"], txt))
    extra_ref(s["name"])
    sheets.append({"name": s["name"], "ref_sha256": sha256(path)})

# 第三關地圖面板
raw = unrle(open(os.path.join(src, "SHT.SCN"), "rb").read(), 0)[0]
G = np.array(struct.unpack_from("<12000H", raw, 0), dtype=np.int32).reshape(20, 600)
for p in plan["map"]["panels"]:
    if filters and not any(f in p["name"] for f in filters):
        continue
    W, H = p["canvas"]
    z = p["zoom"]
    canvas = np.zeros((H, W, 3), np.uint8)
    canvas[:] = CHROMAS[p["chroma"]]
    pal = S.palette(p["palette"])
    for r in range(p["r0"], p["r1"] + 1):
        for c in range(p["c0"], p["c1"] + 1):
            t = int(G[r, c])
            px = S.indexed("MAP", t) if t else np.zeros((10, 8), np.uint8)
            rgb = np.repeat(np.repeat(pal[px], z, axis=0), z, axis=1)
            yy = p["y"] + (r - p["r0"]) * 10 * z
            xx = p["x"] + (c - p["c0"]) * 8 * z
            canvas[yy:yy + 10 * z, xx:xx + 8 * z] = rgb
    path = os.path.join(root, "ref", p["name"] + ".png")
    Image.fromarray(canvas).save(path, optimize=True)
    desc = "one section of a huge biomechanical alien fortress wall on a black void background; it continues beyond the left and right edges of this section, so keep shapes running straight off those edges"
    note = " Black areas inside the rectangle are the empty void of space: keep them pure black."
    txt = PROMPT.format(W=W, H=H, chroma=p["chroma"], chroma_name="magenta", desc=desc, style=style, opaque_note=note)
    open(os.path.join(root, "prompts", p["name"] + ".txt"), "w", encoding="utf-8").write(extra(p["name"], txt))
    extra_ref(p["name"])
    sheets.append({"name": p["name"], "ref_sha256": sha256(path)})
json.dump({"batch": batch, "style_file": os.path.basename(style_path), "style_sha256": hashlib.sha256(style.encode()).hexdigest(), "sheets": sheets},
          open(os.path.join(root, "sheets.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print("參考表", len(sheets))
