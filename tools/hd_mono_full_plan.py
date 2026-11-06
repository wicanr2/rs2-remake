#!/usr/bin/env python3
"""單色 HD 全量重繪（m2）的參考表計畫，輸出 <批次>/plan.json 與 <批次>/classes.json。不呼叫任何外部服務。

用法：hd_mono_full_plan.py <original/rs-2> <workspace> <批次名>

單色圖庫 21 個，assets/mono 共 1,526 個檔。每個檔落在下列其中一類，否則中止：
  codex-redraw      進 Codex 參考表重繪（含圖形式遮罩的主圖、有遮罩庫配對的圖）
  derived-shift     BH2 的像素位移相位，由基礎圖在 HD 位移（單色資料與基礎圖吻合率 >= 0.93）
  item-mask         同圖庫內以圖形式存放的遮罩（BH1 第 4 項、BH2 第 2、3、23 至 26 項），由配對圖的 alpha 反推
  mask-lib          遮罩庫（MYB.BDT、SHT*.BDH）：有重繪配對的由 alpha 反推，其餘最近鄰放大
  map-tile          MAP.BDB 圖塊：依 SHT.SCR 排成物件面板重繪，每種圖塊取最像原版的出現位置切回
  credits           MY 第 15、26、47、48 項，不送 Codex，最近鄰放大
  placeholder       高 1 列的佔位圖，最近鄰放大
  unused-nearest    彩色版只有佔位或尺寸對不上的從未繪製項，最近鄰放大
  blank             全黑圖，最近鄰放大
"""
import json
import os
import struct
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_lib import best_layout, shelf_pack  # noqa: E402
from hd_mono_full_lib import (BRIGHT_ROLES, CREDITS, FREE, HULL, IMG_LIB, LIB_BANK, MASK_BANK, MASKED_BY_ITEM, TEXT_GROUPS, Full,  # noqa: E402
                              mask_best_offset)
from hd_mono_lib import even_layout  # noqa: E402
from rs2_common import unrle  # noqa: E402

src, ws, batch = sys.argv[1:4]
root = os.path.join(ws, "hd", batch)
os.makedirs(root, exist_ok=True)
S = Full(src)
U = json.load(open(os.path.join(ws, "extracted/sprite-usage.json"), encoding="utf-8"))
E = {(e["lib"], e["index"]): e for e in U["entries"]}
assets = os.path.join(ws, "hd", "assets")

# 依遊戲內座標合成後整張重繪（座標同彩色版，見 docs/re/12-stages-presentation.md 第 6 節；單色圖寬度可能差一個位元組，座標以邏輯像素計）
COMPOSITES = {
    "SHT1:boss-body": {"bank": "SHT1.BDB", "parts": {33: (0, 0), 34: (144, 34), 35: (304, 13)}},
    "SHT2:midboss-body": {"bank": "SHT2.BDB", "parts": {32: (0, 0), 33: (320, 7)}},
    "SHT2:boss-body": {"bank": "SHT2.BDB", "parts": {34: (0, 0), 35: (272, 34)}},
}
PHASE_MIN = 0.93

ROLE_DESC = {
    "enemy": "enemy craft or creature sprites; pieces in one group are animation frames or growth stages of the same subject",
    "boss_part": "parts of a large biomechanical boss machine",
    "bullet": "weapon projectiles and energy shots",
    "explosion": "explosion and fireball animation frames",
    "player": "the player's small futuristic fighter craft and its attachments",
    "item": "glowing power-up capsules and carrier objects",
    "ui": "interface illustration",
    "text": "stylized lettering",
    "cutscene": "cinematic illustration",
    "background": "scrolling background scenery for a side-scrolling shooter",
    "effect": "a vertical lightning energy effect",
    "unused": "leftover sprites from the same game",
    "misc": "small assorted sprites from one stage of the game",
}
GROUP_DESC = {
    "MY:act-card": "large stylized letters A, C, T and digits 1, 2, 3, 4. Keep each glyph readable and exactly the same character",
    "MY:disk-prompt": "text banners that read exactly 'Please insert disk A in drive' and 'And press any key to continue.', and the separate letters A, B, C",
    "MY:options-labels": "menu text labels that read exactly LEVEL:, EASY, NORMAL, HARD, HARDEST, CTRL:, ROLE 1, ROLE 2, 2 PLAYERS. Keep the spelling exactly",
    "MY:options-portraits": "four expressions of the same young woman portrait in a frame; only the facial expression differs between frames",
    "MY:item-balls": "four round glowing power-up balls marked with the letters L, B, K, S, in that order from top-left to bottom-right: ball 1 shows an L, ball 2 a B, ball 3 a K, ball 4 an S",
    "BEG1:title-logo": "the game logo letters 'RS-S' with three frames of the last S morphing into the digit 2, so the logo finally reads RS-2",
    "BEG1:title-eyes": "an extreme close-up of an anime hero's eyes and hair",
    "BEG1:title-hand": "a close-up of a gloved hand gripping a control lever",
    "BEG1:ship-front-zoom": "a spaceship seen from the front at five sizes, the same ship",
    "BEG1:ship-approach": "a spaceship with a pale hull and engine rings, seen from behind while it flies toward a planet, at nine sizes, the same ship. The large black shape in its middle is part of the ship and stays black. The noisy white and grey pixels around the ship are only the planet's cloud layer behind it: do NOT paint them and do NOT turn them into machinery. Replace everything around the ship with the flat background colour",
    "BEG1:ship-exhaust": "engine exhaust flares and small glowing sparks, eight frames growing in size. The noisy pixels around the flare are only the planet behind it: do NOT paint them. Paint only the flare and sparks, and replace everything around them with the flat background colour",
    "BEG1:title-earth": "a realistic view of planet Earth from space on black",
    "BEG1:planet-flash": "a planet with a bright flash at three sizes",
    "SHT5:end-letters": "the three large stylized letters E, N, D",
    "SHT1:boss-body": "one huge side-view biomechanical alien battleship drawn as a single continuous machine",
    "SHT2:midboss-body": "one long biomechanical mid-boss warship drawn as a single continuous machine",
    "SHT2:boss-body": "one huge biomechanical lion-headed boss machine drawn as a single continuous machine",
}

# ---------- 逐項分類 ----------
cls = {}      # "bank:idx" -> {"class":..., ...}
pieces_of = {}  # (bank, idx) -> piece dict（還沒排版）


def key(bank, i):
    return f"{bank}:{i}"


def colour_ref(bank, i, it):
    """彩色圖庫同索引的 HD 圖當第二張參考圖：寬度差 8 邏輯像素內、高度差 3 列內，且檔案存在。"""
    lib = IMG_LIB[bank]
    e = E.get((lib, i))
    if not e:
        return None
    cw, ch = e["size"]
    if abs(cw - it["stride"] * 4) > 8 or abs(ch - it["height"]) > 3:
        return None
    if not os.path.exists(os.path.join(assets, lib, f"{i:04d}.png")):
        return None
    return {"lib": lib, "index": i, "size": [cw, ch]}


def is_opaque(bank, i):
    lib = IMG_LIB[bank]
    e = E.get((lib, i))
    if not e or (bank, i) in MASKED_BY_ITEM or (bank, i) in FREE or (bank, i) in HULL:
        return False
    if e["mask"].get("used"):
        return False
    modes = " ".join(e["draw_modes"])
    return "replace" in modes or "to-bg-store" in modes


# 相位：單色資料與基礎圖的吻合率
PHASES = {}
for base, members, circ in ((5, range(6, 13), True), (13, range(14, 17), True), (17, [18], True), (19, range(20, 23), False)):
    b = S.plane("BH2.BDB", base)
    for i in members:
        t = S.plane("BH2.BDB", i)
        W = t.shape[1]

        def shifted(k):
            if circ:
                return np.roll(b, k, axis=1) if b.shape == t.shape else None
            o = np.zeros((b.shape[0], W), b.dtype)
            ww = min(b.shape[1], W - k)
            o[:, k:k + ww] = b[:, :ww]
            return o if o.shape == t.shape else None

        best = None
        for k in range(W):
            o = shifted(k)
            if o is None:
                continue
            m = float((o == t).mean())
            if best is None or m > best[1]:
                best = (k, m)
        if best:
            PHASES[i] = {"base": base, "shift": best[0], "match": round(best[1], 4), "circular": circ}
ITEM_MASKS = {2, 3, 23, 24, 25, 26}  # BH2 內的圖形式遮罩

for bank, lib in IMG_LIB.items():
    for i, it in sorted(S.bank(bank).items()):
        k = key(bank, i)
        h = it["height"]
        e = E.get((lib, i))
        w_hd, h_hd = it["stride"] * 16, h * 4
        base = {"bank": bank, "index": i, "stride": it["stride"], "w": it["stride"] * 4, "h": h, "hd": [w_hd, h_hd]}
        plane = S.plane(bank, i)
        if (bank, i) in CREDITS:
            cls[k] = {**base, "class": "credits", "reason": "署名與商標，AGENTS.md 文化保存，不送 Codex"}
        elif h <= 1:
            cls[k] = {**base, "class": "placeholder", "reason": "高 1 列的佔位圖"}
        elif bank == "BH2.BDB" and i in ITEM_MASKS:
            cls[k] = {**base, "class": "item-mask", "mask_of": {23: 19, 24: 20, 25: 21, 26: 22, 2: 0, 3: 1}[i]}
        elif bank == "BH1.BDB" and i == 4:
            cls[k] = {**base, "class": "item-mask", "mask_of": 3}
        elif bank == "BH2.BDB" and i in PHASES and PHASES[i]["match"] >= PHASE_MIN:
            cls[k] = {**base, "class": "derived-shift", **{f"phase_{a}": b for a, b in PHASES[i].items()}}
        elif plane.sum() == 0:
            cls[k] = {**base, "class": "blank", "reason": "全黑圖"}
        elif e and e["status"] == "placeholder":
            cls[k] = {**base, "class": "unused-nearest", "reason": "sprite-usage 標為 placeholder（彩色版只有佔位圖，遊戲從未繪製）"}
        elif e and e["status"] == "unused" and not colour_ref(bank, i, it):
            cls[k] = {**base, "class": "unused-nearest", "reason": "sprite-usage 標為 unused，且沒有同形的彩色版可參考"}
        else:
            p = dict(base)
            p["opaque"] = is_opaque(bank, i)
            p["free"] = (bank, i) in FREE
            p["hull"] = (bank, i) in HULL
            p["colour"] = colour_ref(bank, i, it)
            p["role"] = (e or {}).get("role") or "misc"
            p["status"] = (e or {}).get("status")
            p["group"] = (e or {}).get("group")
            p["mask"], p["dy"], p["mask_kind"] = None, 0, None
            if (bank, i) in MASKED_BY_ITEM:
                mi = MASKED_BY_ITEM[(bank, i)]
                p["mask"], p["mask_kind"] = f"{bank}:{mi}", "item"
            elif not p["opaque"] and not p["free"] and e and e["mask"].get("used") and e["mask"].get("index") is not None and bank in MASK_BANK:
                mi = e["mask"]["index"]
                if mi in S.bank(MASK_BANK[bank]):
                    p["mask"], p["mask_kind"] = f"{MASK_BANK[bank]}:{mi}", "lib"
            if p["mask"]:
                mb, mi = p["mask"].split(":")
                m = S.plane(mb, int(mi))
                dy, inside, total = mask_best_offset(plane, m)
                if (bank, i) in {("BH2.BDB", 0), ("BH2.BDB", 1)}:
                    # docs/re/12-stages-presentation.md 5.2：火焰面的遮罩只取前 22 列，與圖上緣對齊
                    dy = 0
                    inside = int(((plane[:m.shape[0]] == 1) & (m[:plane.shape[0]] == 0)).sum())
                p["dy"], p["mask_cover"] = dy, [inside, total]
                p["mask_hd"] = [m.shape[1] * 2, m.shape[0] * 4]
            p["bright"] = p["role"] in BRIGHT_ROLES
            cls[k] = {**base, "class": "codex-redraw"}
            pieces_of[(bank, i)] = p

# 遮罩庫：有重繪配對的由 alpha 反推
for mb in sorted(set(MASK_BANK.values())):
    for i, it in sorted(S.bank(mb).items()):
        k = key(mb, i)
        partner = None
        for bank, m in MASK_BANK.items():
            if m == mb and (bank, i) in pieces_of and pieces_of[(bank, i)]["mask"] == k:
                partner = (bank, i)
            if m == mb and cls.get(key(bank, i), {}).get("class") == "derived-shift":
                pass
        cls[k] = {"bank": mb, "index": i, "stride": it["stride"], "h": it["height"], "hd": [it["stride"] * 16, it["height"] * 4],
                  "class": "mask-lib", "partner": key(*partner) if partner else None}

# ---------- 分組與排版 ----------
groups = []
handled = {(b, i) for (b, i) in pieces_of} ^ set()
in_comp = set()
for name, c in COMPOSITES.items():
    in_comp |= {(c["bank"], i) for i in c["parts"]}


def add_group(name, bank, members, role, note=None):
    groups.append({"name": name, "bank": bank, "members": list(members), "role": role, "note": note})


for name, g in U["groups"].items():
    if g["lib"] == "MAP":
        continue
    bank = LIB_BANK[g["lib"]]
    ms = [m for m in g["members"] if (bank, m) in pieces_of and (bank, m) not in in_comp]
    if not ms:
        continue
    add_group(name, bank, ms, pieces_of[(bank, ms[0])]["role"], g.get("note"))
grouped = {(g["bank"], m) for g in groups for m in g["members"]}
for (bank, i), p in sorted(pieces_of.items()):
    if (bank, i) in grouped or (bank, i) in in_comp:
        continue
    add_group(f"{IMG_LIB[bank]}:ungrouped-{i}", bank, [i], p["role"] or "misc")

NOTES = {}


def desc_of(g):
    return GROUP_DESC.get(g["name"]) or ((g["note"] and f"{ROLE_DESC.get(g['role'], 'game sprites')} ({g['note']})") or ROLE_DESC.get(g["role"], "game sprites from one stage"))


def sheet_from(name, bank, members, desc, composite=None, force_z=None):
    """回傳一張或多張表。放不下單色偶數倍率 4 就切半。"""
    ps = [dict(pieces_of[(bank, i)]) for i in members]
    if composite:
        parts = composite["parts"]
        W = max(pieces_of[(bank, i)]["w"] + x for i, (x, y) in parts.items())
        H = max(pieces_of[(bank, i)]["h"] + y for i, (x, y) in parts.items())
        best = None
        for canvas in ((1536, 1024), (1024, 1536), (1024, 1024)):
            for z in range(8, 1, -1):
                if shelf_pack([(W, H)], canvas, z, 8) is not None:
                    if best is None or z > best[1]:
                        best = (canvas, z, shelf_pack([(W, H)], canvas, z, 8))
                    break
        canvas, z, pos = best
        ox, oy = pos[0]
        for p, (i, (x, y)) in zip(ps, parts.items()):
            p["x"], p["y"] = ox + x * z, oy + y * z
        return [{"name": name, "canvas": list(canvas), "zoom": z, "desc": desc, "pieces": ps, "kind": "composite", "group": name}]
    sizes = [(p["w"], p["h"]) for p in ps]
    lay = even_layout(sizes, gap=32, zmin=4)
    if lay is None:
        if len(members) == 1:
            lay = best_layout(sizes, gap=8, zmin=2)
            if lay is None:
                raise SystemExit(f"{name} {members} 放不下")
        else:
            mid = len(members) // 2
            return sheet_from(name + "-a", bank, members[:mid], desc) + sheet_from(name + "-b", bank, members[mid:], desc)
    canvas, z, pos = lay
    for k, p in enumerate(ps):
        p["x"], p["y"] = pos[k]
    return [{"name": name, "canvas": list(canvas), "zoom": z, "desc": desc, "pieces": ps, "kind": "sprites", "group": name}]


sheets = []
for name, c in COMPOSITES.items():
    sheets += sheet_from("mono-" + name.replace(":", "-"), c["bank"], list(c["parts"]), GROUP_DESC[name], composite=c)

small = {}
for g in groups:
    ms = g["members"]
    if g["name"].startswith("BH1:"):
        continue
    if len(ms) == 1:
        p = pieces_of[(g["bank"], ms[0])]
        if p["w"] * p["h"] <= 64 * 64 and not p["free"]:
            small.setdefault(g["bank"], []).append(g)
            continue
    sheets += sheet_from("mono-" + g["name"].replace(":", "-"), g["bank"], ms, desc_of(g))
for bank, gs in sorted(small.items()):
    for k in range(0, len(gs), 8):
        chunk = gs[k:k + 8]
        ms = [g["members"][0] for g in chunk]
        sheets += sheet_from(f"mono-{IMG_LIB[bank]}-misc-{k // 8}", bank, ms, "small assorted sprites from one stage of the game; they are unrelated to each other, each piece is its own object")

# BH1：第 0、2、3 項（含遮罩圖第 4 項）排成同一張風景，第 1 項（32 像素寬的煙）另成一張。z=4，手排。
bh = [dict(pieces_of[("BH1.BDB", i)]) for i in (0, 2, 3)]
y = 32
for p in bh:
    p["x"], p["y"] = 32, y
    y += p["h"] * 4 + 32
assert y <= 1024
sheets.append({"name": "mono-BH1", "canvas": [1536, 1024], "zoom": 4, "kind": "sprites", "group": "BH1:landscape",
               "desc": "the scrolling background layers of stage 1, one landscape from top to bottom: a wide band of ruined alien ground and rubble at the horizon (first strip), a wide curtain of huge organic root-like stalactites hanging over a misty floor (second strip), and a snow-lit mountain with a bright rim light on a black sky (third strip). The left and right ends of each wide strip join when the scenery scrolls, so keep the shapes and tones at both ends continuous",
               "pieces": bh})
smoke = dict(pieces_of[("BH1.BDB", 1)])
smoke["x"], smoke["y"] = 32, 32
# Codex 的方形輸出是 1254x1254，不是 1024x1024，所以用直式 1024x1536。
sheets.append({"name": "mono-BH1-smoke", "canvas": [1024, 1536], "zoom": 8, "kind": "sprites", "group": "BH1:smoke", "pieces": [smoke],
               "desc": "one narrow tall strip of drifting smoke or mist, a soft continuous field of fine grain that is dark at the top and fades to bright white at the bottom"})

# ---------- MAP：依 SHT.SCR ----------
raw = unrle(open(os.path.join(src, "SHT.SCR"), "rb").read(), 0)[0]
COLS, ROWS = 600, 20
G = np.array(struct.unpack_from(f"<{COLS * ROWS}H", raw, 0), np.int32).reshape(ROWS, COLS)
seen = np.zeros(G.shape, bool)
objs = []
for r in range(ROWS):
    for c in range(COLS):
        if seen[r, c] or G[r, c] == 0:
            continue
        st = [(r, c)]
        seen[r, c] = True
        cells = []
        while st:
            a, b = st.pop()
            cells.append((a, b))
            for da in (-1, 0, 1):
                for db in (-1, 0, 1):
                    n = (a + da, b + db)
                    if 0 <= n[0] < ROWS and 0 <= n[1] < COLS and not seen[n] and G[n] != 0:
                        seen[n] = True
                        st.append(n)
        objs.append({"r0": min(x[0] for x in cells), "r1": max(x[0] for x in cells), "c0": min(x[1] for x in cells), "c1": max(x[1] for x in cells)})
MAXW, OVER = 46, 4
uniq = {}
for o in sorted(objs, key=lambda o: o["c0"]):
    kk = (o["r1"] - o["r0"], o["c1"] - o["c0"], G[o["r0"]:o["r1"] + 1, o["c0"]:o["c1"] + 1].tobytes())
    if kk in uniq:
        uniq[kk]["instances"].append([int(o["r0"]), int(o["c0"])])
        continue
    uniq[kk] = {"r0": int(o["r0"]), "r1": int(o["r1"]), "c0": int(o["c0"]), "c1": int(o["c1"]), "instances": [[int(o["r0"]), int(o["c0"])]]}
map_objs, panels = [], []
for n, o in enumerate(uniq.values()):
    o["id"] = n
    cols = []
    c = o["c0"]
    while True:
        e = min(c + MAXW - 1, o["c1"])
        cols.append((c, e))
        if e >= o["c1"]:
            break
        c = e + 1 - OVER
    o["panels"] = []
    for k, (c0, c1) in enumerate(cols):
        pn = f"mono-MAP-obj{n:02d}-p{k}"
        o["panels"].append(pn)
        panels.append({"name": pn, "obj": n, "r0": o["r0"], "r1": o["r1"], "c0": c0, "c1": c1})
    map_objs.append(o)
for p in panels:
    W, H = (p["c1"] - p["c0"] + 1) * 8, (p["r1"] - p["r0"] + 1) * 10
    lay = even_layout([(W, H)], gap=8, zmin=4)
    if lay is None:
        raise SystemExit(f"面板 {p['name']} {W}x{H} 放不下")
    canvas, z, pos = lay
    p.update({"canvas": list(canvas), "zoom": z, "x": pos[0][0], "y": pos[0][1], "w": W, "h": H})
tiles = {}
for r in range(ROWS):
    for c in range(COLS):
        t = int(G[r, c])
        if t:
            tiles.setdefault(t, 0)
            tiles[t] += 1
for i, it in sorted(S.bank("MAP.BDB").items()):
    k = key("MAP.BDB", i)
    if i == 0 or i not in tiles:
        cls[k] = {"bank": "MAP.BDB", "index": i, "stride": it["stride"], "h": it["height"], "hd": [32, 40], "class": "blank" if i == 0 else "unused-nearest",
                  "reason": "SHT.SCR 的空格，圖塊全黑" if i == 0 else "SHT.SCR 沒有用到的圖塊"}
    else:
        cls[k] = {"bank": "MAP.BDB", "index": i, "stride": it["stride"], "h": it["height"], "hd": [32, 40], "class": "map-tile", "uses": tiles[i]}
mapplan = {"columns": COLS, "rows": ROWS, "grid": "SHT.SCR", "objects": map_objs, "panels": panels, "overlap_columns": OVER}

plan = {"batch": batch, "scale": 4, "sheets": sheets, "map": mapplan}
json.dump(plan, open(os.path.join(root, "plan.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
json.dump(cls, open(os.path.join(root, "classes.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)

# ---------- 摘要 ----------
count = {}
for v in cls.values():
    count[v["class"]] = count.get(v["class"], 0) + 1
print("分類（含遮罩庫與 MAP）", dict(sorted(count.items())), "合計", sum(count.values()))
print("參考表", len(sheets), "面板", len(panels), "地圖物件", len(map_objs), "含重複實例", len(objs), "SCR 使用圖塊", len(tiles))
npieces = sum(len(s["pieces"]) for s in sheets)
print("sheet 內圖", npieces, "與 pieces_of", len(pieces_of))
used = {(p["bank"], p["index"]) for s in sheets for p in s["pieces"]}
print("沒進任何 sheet 的重繪項", sorted(set(pieces_of) - used))
nomask = [(p["bank"], p["index"]) for s in sheets for p in s["pieces"] if not p["opaque"] and not p["mask"] and not p["free"]]
print("非不透明、沒遮罩、非 free 的圖（純 OR 繪製）", nomask)
low = [(p["bank"], p["index"], p["mask_cover"]) for s in sheets for p in s["pieces"] if p.get("mask_cover") and p["mask_cover"][1] and p["mask_cover"][0] / p["mask_cover"][1] < 0.95]
print("遮罩涵蓋率低於 0.95", low)
print("遮罩偏移 dy 分佈", sorted({(p["dy"]) for s in sheets for p in s["pieces"] if p["mask"]}))
print("寬度不符的遮罩", [(p["bank"], p["index"], p["w"] * 4 // 4, p["mask_hd"]) for s in sheets for p in s["pieces"] if p["mask"] and p["mask_hd"][0] != p["hd"][0]])
for s in sheets:
    print(f"{s['name']:34} {s['canvas'][0]}x{s['canvas'][1]} z{s['zoom']:<2} {s['kind'][:4]} " + ",".join(f"{p['bank'][:4]}{p['index']}" for p in s["pieces"]))
print("面板", [(p["name"][-9:], p["zoom"], p["canvas"][0], f"{p['w']}x{p['h']}") for p in panels])
print("derived", {k: (v["phase_base"], v["phase_shift"], v["phase_match"]) for k, v in cls.items() if v["class"] == "derived-shift"})
