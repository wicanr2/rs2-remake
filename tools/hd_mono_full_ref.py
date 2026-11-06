#!/usr/bin/env python3
"""依 plan.json 產生單色 HD 全量重繪的參考表與提示詞。不呼叫任何外部服務。

用法：hd_mono_full_ref.py <original/rs-2> <workspace> <批次名> <風格檔> [--plan-from <批次>] [--notes <JSON 檔名>] [名稱...]
  名稱 給了就只做這幾張（完整名稱，例如 mono-MY-item-balls、mono-MAP-obj00-p0），沒給就全做。
  --plan-from  從另一個批次讀 plan.json（重畫批次用）。重畫批次的 plan.json 只留指定的幾張。
  --notes      <批次>/ 底下的 JSON：{表名: 補充文字}，附在該表提示詞最後（重畫時針對上一輪的問題加指示）。
輸出：<批次>/ref/<名>.png        第一張參考圖：原版單色圖放大（白亮點、黑背景），洋紅底色
      <批次>/ref/<名>.extra.png  第二張參考圖：同位置的彩色 HD 圖轉灰階（只有尺寸相符的項才有）
      <批次>/prompts/<名>.txt、sheets.json、plan.json（重畫批次）
"""
import hashlib
import json
import os
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_mono_full_lib import BRIGHT_ROLES, CHROMA, CHROMA_HEX, CHROMA_NAME, TEXT_GROUPS, Full, render_ref_piece_z, sha256  # noqa: E402

argv = sys.argv[1:]
src, ws, batch, style_path = argv[:4]
rest = argv[4:]
plan_from, notes_file, names = batch, None, []
k = 0
while k < len(rest):
    if rest[k] == "--plan-from":
        plan_from = rest[k + 1]
        k += 2
    elif rest[k] == "--notes":
        notes_file = rest[k + 1]
        k += 2
    else:
        names.append(rest[k])
        k += 1
root = os.path.join(ws, "hd", batch)
assets = os.path.join(ws, "hd", "assets")
os.makedirs(os.path.join(root, "ref"), exist_ok=True)
os.makedirs(os.path.join(root, "prompts"), exist_ok=True)
S = Full(src)
plan = json.load(open(os.path.join(ws, "hd", plan_from, "plan.json"), encoding="utf-8"))
style_raw = open(style_path, encoding="utf-8").read().strip()
extra_notes = json.load(open(os.path.join(root, notes_file), encoding="utf-8")) if notes_file else {}

PROMPT = """Use your built-in image generation tool exactly once to create one {W}x{H} image. Use the attached image{plural} as reference. Do not run shell commands and do not edit files.

Image 1 is a {W}x{H} reference sheet of game art from RS-2, a 1990 side-scrolling science-fiction shooter for DOS by the Taiwanese developer Softstar. It shows the original 1-bit monochrome (CGA and Hercules) version of the art, enlarged with nearest-neighbor on a flat {chroma_name} #{chroma} background: white pixels on black, where regular dither patterns stand for mid-grey. Pieces: {desc}.{extra_para}

This is NOT an upscale and NOT a dither cleanup. Repaint every piece by hand as a brand-new high-resolution illustration of the same subject, inventing convincing fine detail where the low-resolution reference only hints at it.

Hard constraints, because each piece is cut back out at its original position:
- Keep every piece at exactly the same position, size and outer silhouette as in image 1. Do not move, resize, rotate, mirror, crop or reorder anything, and do not let anything spill outside its outline.
- Where the outline in image 1 is a blocky stair-step caused by the enlargement, draw a clean smooth curve that stays within one enlarged pixel of it.
- Keep the same pose, proportions and composition of each piece. Bright areas of image 1 stay bright and dark areas stay dark; a dither pattern means a mid-tone area. Black areas inside a piece are part of the art: keep them dark and painted, not transparent.
- Pieces that repeat with small differences are animation frames or size variants of the same thing: keep them identical except for the parts that differ in image 1.
- Keep the flat {chroma_name} #{chroma} background everywhere outside the pieces, with no shadow, glow or texture on it, and do not use that colour inside the pieces.
- No text except the text already in image 1, no signature, no new objects, no frames around the sheet.{notes}

{style}
"""
EXTRA_SPRITES = (" Image 2 is the same sheet with the colour version's redraws of the same pieces, converted to grey and placed at identical positions. "
                 "Use image 2 only for design detail such as panel lines, materials and highlights. Where image 2 and image 1 disagree about the silhouette "
                 "or about which areas are light and dark, follow image 1. A piece that is blank in image 2 has no counterpart there.")
EXTRA_MAP = (" Image 2 is a crop of the colour version's redraw of the same fortress at the same place, converted to grey. Its layout differs slightly. "
             "Use it only for design detail such as panel lines, materials and highlights, and follow image 1 for every shape and every light or dark area.")
MAP_DESC = ("one section of a huge biomechanical alien fortress on a black void background, built from a grid of wall tiles; the structure continues beyond the left and right edges of this "
            "section, so keep shapes running straight off those edges. Black areas inside the rectangle are the empty void of space: keep them pure black")

NOTE_TEXT = ("For lettering, paint solid pure white letterforms on pure black with crisp clean edges: flat fill, no gradient, no glow, no texture, no drop shadow. "
             "The characters must be exactly the characters of image 1, in the same order.")
NOTE_BRIGHT_ALL = ("Every piece on this sheet follows the brightness rule below.")
NOTE_BRIGHT_SOME = ("Pieces that are small projectiles, power-up balls, explosion frames or flashes follow the brightness rule below; every other piece keeps the normal tonal range.")
SPECIAL = {
    "mono-MY-item-balls": "Each ball carries its letter as a solid pure white glyph, large and unmistakable, centred on a dark disc inside the ball: L, B, K, S in this order. The B has two bowls, the K has two diagonal arms, the S is one smooth curve. The ring of the ball is painted light.",
    "mono-MY-pod-right": "Each frame is one seamless object: paint it as a single continuous shape with no vertical joins or cuts.",
    "mono-MY-pod-left": "Each frame is one seamless object: paint it as a single continuous shape with no vertical joins or cuts.",
    "mono-BH1-smoke": ("This strip is repeated side by side horizontally, so it must look like a seamless field when copies are placed next to each other. "
                       "Paint only continuous, soft, fine-grained smoke noise: no distinct shapes, no clouds with outlines, no wisps, no streaks, no highlights that stand out, "
                       "nothing that would be recognisable as a repeated motif. Keep the vertical gradient: nearly black at the top, brightening steadily to near white at the bottom, "
                       "and keep the left and right edges the same tone as the middle."),
    "mono-BH1": "The three strips are separate layers that are drawn on top of each other and scroll sideways in a loop: make the left edge and the right edge of every strip show the same shapes and tones so the loop joins invisibly.",
    "mono-BH2-surface": "Both strips are layers of a wall of fire seen edge-on, scrolling sideways in a loop: make the left and right edges match.",
}

sheets = []


STYLE_PARTS = style_raw.split("\n\n")  # 第一段是基本畫風，之後每段是只在有適用圖時才附上的規則（亮度規則）


def style_for(bright=False):
    return STYLE_PARTS[0] + (("\n\n" + "\n\n".join(STYLE_PARTS[1:])) if bright and len(STYLE_PARTS) > 1 else "")


def grey_of(rgb):
    return rgb[..., 0] * 0.299 + rgb[..., 1] * 0.587 + rgb[..., 2] * 0.114


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


def sheet_notes(s):
    notes = []
    ps = s["pieces"]
    groups = {p.get("group") for p in ps}
    if groups & TEXT_GROUPS:
        notes.append(NOTE_TEXT)
    br = [p.get("bright") for p in ps]
    if br and all(br):
        notes.append(NOTE_BRIGHT_ALL)
    elif br and any(br):
        notes.append(NOTE_BRIGHT_SOME)
    if s["name"] in SPECIAL:
        notes.append(SPECIAL[s["name"]])
    if s["name"] in extra_notes:
        notes.append(extra_notes[s["name"]])
    return "".join("\n- " + n for n in notes)


def write(name, W, H, canvas, extra, desc, extra_para, notes, bright=False):
    path = os.path.join(root, "ref", name + ".png")
    Image.fromarray(canvas).save(path, optimize=True)
    rec = {"name": name, "ref_sha256": sha256(path), "canvas": [W, H]}
    ep = os.path.join(root, "ref", name + ".extra.png")
    if os.path.exists(ep):
        os.unlink(ep)
    if extra is not None:
        Image.fromarray(extra).save(ep, optimize=True)
        rec["extra_sha256"] = sha256(ep)
    txt = PROMPT.format(W=W, H=H, plural="s" if extra is not None else "", chroma=CHROMA_HEX, chroma_name=CHROMA_NAME, desc=desc,
                        extra_para=extra_para if extra is not None else "", notes=notes, style=style_for(bright))
    pp = os.path.join(root, "prompts", name + ".txt")
    open(pp, "w", encoding="utf-8").write(txt)
    rec["prompt_sha256"] = sha256(pp)
    sheets.append(rec)


want = set(names)
kept_sheets, kept_panels = [], []
for s in plan["sheets"]:
    if want and s["name"] not in want:
        continue
    kept_sheets.append(s)
    W, H = s["canvas"]
    z = s["zoom"]
    canvas = np.zeros((H, W, 3), np.uint8)
    canvas[:] = CHROMA
    extra = np.zeros((H, W, 3), np.uint8)
    extra[:] = CHROMA
    any_extra = False
    for p in s["pieces"]:
        render_ref_piece_z(canvas, p, z, S)
        if p["colour"]:
            extra_piece(extra, p, z)
            any_extra = True
    write(s["name"], W, H, canvas, extra if any_extra else None, s["desc"], EXTRA_SPRITES, sheet_notes(s), bright=any(p.get("bright") for p in s["pieces"]))

# 第三關面板：依 SHT.SCR 的圖塊配置
import struct  # noqa: E402

from rs2_common import unrle  # noqa: E402

raw = unrle(open(os.path.join(src, "SHT.SCR"), "rb").read(), 0)[0]
G = np.array(struct.unpack_from("<12000H", raw, 0), np.int32).reshape(20, 600)
full = None
for pn in plan["map"]["panels"]:
    if want and pn["name"] not in want:
        continue
    kept_panels.append(pn)
    if full is None:
        full = Image.open(os.path.join(assets, "maps", "SHT.SCN.hd.png")).convert("RGB")
    W, H = pn["canvas"]
    z = pn["zoom"]
    canvas = np.zeros((H, W, 3), np.uint8)
    canvas[:] = CHROMA
    for r in range(pn["r0"], pn["r1"] + 1):
        for c in range(pn["c0"], pn["c1"] + 1):
            t = int(G[r, c])
            yy = pn["y"] + (r - pn["r0"]) * 10 * z
            xx = pn["x"] + (c - pn["c0"]) * 8 * z
            if t == 0:
                canvas[yy:yy + 10 * z, xx:xx + 8 * z] = 0
                continue
            pl = S.plane("MAP.BDB", t) * 255  # 10 x 16 單色像素
            oh, ow = 10 * z, 8 * z
            ys = np.arange(oh) * 10 // oh
            xs = np.arange(ow) * 16 // ow
            canvas[yy:yy + oh, xx:xx + ow] = pl[ys][:, xs][..., None]
    crop = full.crop((pn["c0"] * 32, pn["r0"] * 40, (pn["c1"] + 1) * 32, (pn["r1"] + 1) * 40)).resize((pn["w"] * z, pn["h"] * z), Image.LANCZOS)
    g = grey_of(np.array(crop, np.float32))
    extra = np.zeros((H, W, 3), np.uint8)
    extra[:] = CHROMA
    extra[pn["y"]:pn["y"] + g.shape[0], pn["x"]:pn["x"] + g.shape[1]] = np.clip(g, 0, 255).astype(np.uint8)[..., None]
    nt = "".join("\n- " + n for n in [extra_notes.get(pn["name"], "")] if n)
    write(pn["name"], W, H, canvas, extra, MAP_DESC, EXTRA_MAP, nt)

if batch != plan_from:
    sub = {"batch": batch, "scale": plan["scale"], "sheets": kept_sheets, "map": {**plan["map"], "panels": kept_panels}}
    json.dump(sub, open(os.path.join(root, "plan.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
sj = os.path.join(root, "sheets.json")
old = {}
if os.path.exists(sj):
    old = {r["name"]: r for r in json.load(open(sj, encoding="utf-8"))["sheets"]}
for r in sheets:
    old[r["name"]] = r
json.dump({"batch": batch, "style_file": os.path.basename(style_path), "style_sha256": hashlib.sha256(style_raw.encode()).hexdigest(), "sheets": list(old.values())},
          open(sj, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print("參考表", len(sheets), "（sheets.json 共", len(old), "）")
