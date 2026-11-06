#!/usr/bin/env python3
"""單色 HD 畫風試稿的參考表計畫，輸出 <批次>/plan.json。不呼叫任何外部服務。

用法：hd_mono_plan.py <original/rs-2> <workspace> <批次名>

試稿範圍（第一關示範畫面與第三關一個面板，見 workspace/hd/pilot-mono-review/）：
  BH1 背景 5 項（含遮罩圖第 4 項）、MY 玩家機與子彈、SHT1 敵機、第三關 MAP.BDB 的一個面板。
單色圖庫與彩色圖庫的索引不一定一一對應（尺寸不同的項是另一張圖），所以每項都記 colour 欄：
只有寬度相同、高度差在 2 列以內的才有彩色 HD 圖可當第二張參考圖，否則為 null。
"""
import json
import os
import struct
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_mono_lib import MonoSource, even_layout  # noqa: E402
from rs2_common import unrle  # noqa: E402

src, ws, batch = sys.argv[1:4]
root = os.path.join(ws, "hd", batch)
os.makedirs(root, exist_ok=True)
S = MonoSource(src)
USAGE = {(e["lib"], e["index"]): e for e in json.load(open(os.path.join(ws, "extracted/sprite-usage.json"), encoding="utf-8"))["entries"]}

MASK_BANK = {"MY.BDB": "MYB.BDT", "SHT1.BDB": "SHT1.BDH"}
LIB = {"MY.BDB": "MY", "SHT1.BDB": "SHT1", "BH1.BDB": "BH1"}

# 名稱、說明、圖庫、索引（不透明的以 ! 結尾，用遮罩項的以 =N 標明）
SHEETS = [
    ("mono-SHT1-enemy2", "stage 1 enemy objects: four animation frames of the same small armored gunship that points left, a tiny round energy bullet, a larger glowing round plasma ball, and a glass-domed power-up carrier capsule",
     "SHT1.BDB", [2, 3, 4, 5, 1, 7, 8]),
    ("mono-SHT1-worm", "four poses of one long segmented serpent made of glowing beads, with a spiked sensor head at the top; the poses are animation frames of the same enemy",
     "SHT1.BDB", [25, 26, 27, 28]),
    ("mono-MY-ship", "the player's small futuristic fighter craft seen from the side and flying to the right, in three animation frames of the same craft, plus two sizes of engine exhaust flame trailing to the left and two small muzzle spark flashes",
     "MY.BDB", [7, 9, 11, 12, 13, 10, 17]),
    ("mono-MY-shots", "the player's weapon projectiles: a glowing energy orb, two thin laser segments, two crescent wave shots and a tall flame burst; pieces of one kind are animation frames or variants of the same shot",
     "MY.BDB", [2, 4, 25, 5, 60, 55]),
    ("mono-MY-pod", "five animation frames of the same small round gun pod attached to the player's craft, rotating; keep them identical except for the parts that differ in image 1",
     "MY.BDB", [30, 31, 32, 33, 34]),
]
OPAQUE = {("MY.BDB", 4), ("MY.BDB", 25)}


def colour_match(bank, i):
    """彩色圖庫同索引的圖寬度相同、高度差在 2 列以內才當同一張圖。"""
    lib = LIB.get(bank)
    e = USAGE.get((lib, i)) if lib else None
    it = S.item(bank, i)
    if not e or e["size"][0] != it["stride"] * 4 or abs(e["size"][1] - it["height"]) > 2:
        return None
    return {"lib": lib, "index": i, "size": e["size"]}


def make_piece(bank, i, opaque=False, mask=None):
    it = S.item(bank, i)
    p = {"bank": bank, "index": i, "stride": it["stride"], "w": it["stride"] * 4, "h": it["height"], "opaque": opaque, "mask": mask, "dy": 0,
         "hd": [it["stride"] * 16, it["height"] * 4], "colour": colour_match(bank, i)}
    if mask:
        mb, mi = mask.split(":")
        plane, m = S.plane(bank, i), S.plane(mb, int(mi))
        dy, inside, total = S.best_dy(plane, m)
        p["dy"], p["mask_cover"] = dy, [inside, total]
        p["mask_hd"] = [m.shape[1] * 2, m.shape[0] * 4]
    return p


plan = {"batch": batch, "scale": 4, "sheets": [], "map": None}
for name, desc, bank, idxs in SHEETS:
    pieces = []
    for i in idxs:
        op = (bank, i) in OPAQUE
        pieces.append(make_piece(bank, i, opaque=op, mask=None if op else f"{MASK_BANK[bank]}:{i}"))
    lay = even_layout([(p["w"], p["h"]) for p in pieces], gap=32)
    if lay is None:
        raise SystemExit(f"{name} 放不下")
    canvas, z, pos = lay
    for k, p in enumerate(pieces):
        p["x"], p["y"] = pos[k]
    plan["sheets"].append({"name": name, "canvas": list(canvas), "zoom": z, "desc": desc, "pieces": pieces, "kind": "sprites"})

# BH1：第 0、2 項整張不透明，第 3 項以第 4 項為遮罩（山影），第 1 項是窄條。z=4，1536x1024 內手排。
bh = [make_piece("BH1.BDB", 0, opaque=True), make_piece("BH1.BDB", 2, opaque=True), make_piece("BH1.BDB", 3, mask="BH1.BDB:4"),
      make_piece("BH1.BDB", 1, opaque=True)]
y = 32
for p in bh[:3]:
    p["x"], p["y"] = 32, y
    y += p["h"] * 4 + 32
bh[3]["x"], bh[3]["y"] = 32 + 1280 + 32, 32
assert y <= 1024 and bh[3]["x"] + bh[3]["w"] * 4 <= 1536, (y, bh[3]["x"])
plan["sheets"].append({"name": "mono-BH1", "canvas": [1536, 1024], "zoom": 4, "kind": "sprites",
                       "desc": "the scrolling background layers of stage 1, one landscape: a wide band of ruined alien ground and rubble at the horizon (first strip), a wide curtain of huge organic root-like stalactites hanging over a misty floor (second strip), a snow-lit mountain with a bright rim light on a black sky (third strip), and one narrow vertical strip of drifting smoke that fades from dark at the top to white at the bottom (right). The left and right ends of each wide strip join when the scenery scrolls, so keep the shapes and tones at both ends continuous",
                       "pieces": bh})

# 第三關面板：SHT.SCR（單色用的地形格）的第 45 至 88 欄、第 0 至 19 列，含兩個物件：懸在上方的構造與下方的砲管穹頂。
raw = unrle(open(os.path.join(src, "SHT.SCR"), "rb").read(), 0)[0]
G = np.array(struct.unpack_from("<12000H", raw, 0), np.int32).reshape(20, 600)
c0, c1, r0, r1 = 45, 88, 0, 19
W, H = (c1 - c0 + 1) * 8, (r1 - r0 + 1) * 10
z = 4
cells = G[r0:r1 + 1, c0:c1 + 1]
first = {}
for r in range(cells.shape[0]):
    for c in range(cells.shape[1]):
        t = int(cells[r, c])
        if t and t not in first:
            first[t] = [r0 + r, c0 + c]
plan["map"] = {"name": "mono-MAP-panel", "canvas": [1536, 1024], "zoom": z, "x": 64, "y": 112, "c0": c0, "c1": c1, "r0": r0, "r1": r1, "w": W, "h": H,
               "grid": "SHT.SCR", "tiles_first": {str(k): v for k, v in sorted(first.items())},
               "desc": "one section of a huge biomechanical alien fortress on a black void background: a hanging machine structure at the top and a mound of large cannon barrels and coiled engine domes at the bottom; the structures continue beyond the left and right edges of this section, so keep shapes running straight off those edges"}
json.dump(plan, open(os.path.join(root, "plan.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
for s in plan["sheets"]:
    print(s["name"], s["canvas"], "z", s["zoom"], [(p["index"], p["w"], p["h"], p["dy"], p.get("mask_cover"), bool(p["colour"])) for p in s["pieces"]])
print("map", plan["map"]["name"], (W, H), "tiles", len(first))
