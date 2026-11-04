#!/usr/bin/env python3
"""依 sprite-usage.json 排出 HD 參考表計畫與衍生規則，輸出 plan.json。

用法：hd_plan.py <original/rs-2> <workspace> <輸出 plan.json>
每個圖庫索引都要落在下列其中一種處理方式，否則中止：
  sheet        進 Codex 參考表重繪
  derived      由另一張 HD 圖計算（像素位移相位、遮罩）
  protected    署名與標誌，不交給生成模型，只做確定性放大
  placeholder  高 1 列的佔位圖，只做確定性放大
  map          MAP 圖塊，由物件面板重繪後切回
"""
import json
import os
import struct
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_lib import Source, best_layout, pick_chroma  # noqa: E402
from rs2_common import unrle  # noqa: E402

src, ws, out = sys.argv[1], sys.argv[2], sys.argv[3]
S = Source(src, os.path.join(ws, "unpacked"), os.path.join(ws, "extracted/manifest.json"))
U = json.load(open(os.path.join(ws, "extracted/sprite-usage.json"), encoding="utf-8"))
E = {(e["lib"], e["index"]): e for e in U["entries"]}

# 署名與公司標誌：含中文人名或商標，模型可能改字。第 48 項第二行中間字尚未確認。
PROTECTED = {("MY", 15), ("MY", 26), ("MY", 47), ("MY", 48)}

# 依遊戲內座標合成後整張重繪，避免接縫。座標來源：docs/re/12-stages-presentation.md 第 6 節。
COMPOSITES = {
    "SHT1:boss-body": {"lib": "SHT1", "parts": {33: (0, 0), 34: (144, 34), 35: (304, 13)}},
    "SHT2:midboss-body": {"lib": "SHT2", "parts": {32: (0, 0), 33: (320, 7)}},
    "SHT2:boss-body": {"lib": "SHT2", "parts": {34: (0, 0), 35: (272, 34)}},
}
# 開場飛船與噴焰：原版是連同地球雲層一起貼上去的矩形。HD 地球與這些補丁的雲層不可能逐像素吻合，
# 所以只畫物件本身，四周去背，由 remake 疊在 HD 地球上。輪廓不受原版矩形限制。
FREE_ALPHA = {("BEG1", i) for i in (8, 9, 10, 11, 12, 13, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28)}
# 平面與遮罩成對：(圖, 遮罩)。遮罩 0 的部分才有圖，最後由 HD alpha 反推遮罩。
MASKED_BY_ITEM = {("BH1", 3): 4, ("BH2", 0): 2, ("BH2", 1): 3, ("BH2", 19): 23, ("BH2", 20): 24, ("BH2", 21): 25, ("BH2", 22): 26}

ROLE_DESC = {
    "enemy": "enemy craft or creature sprites; pieces in one group are animation frames or growth stages of the same subject",
    "boss_part": "parts of a large biomechanical boss machine",
    "bullet": "weapon projectiles and energy shots",
    "explosion": "explosion and fireball animation frames, ordered from small to large or by frame",
    "player": "the player's small futuristic fighter craft and its attachments",
    "item": "glowing power-up capsules and carrier objects",
    "ui": "interface illustration",
    "text": "stylized lettering",
    "cutscene": "cinematic illustration",
    "background": "scrolling background scenery for a side-scrolling shooter",
    "effect": "a vertical lightning energy effect",
    "unused": "leftover sprites from the same game",
}
GROUP_DESC = {
    "MY:act-card": "large stylized letters A, C, T and digits 1, 2, 3, 4. Keep each glyph readable and exactly the same character",
    "MY:disk-prompt": "text banners that read exactly 'Please insert disk A in drive' and 'And press any key to continue.', and the separate letters A, B, C",
    "MY:options-labels": "menu text labels that read exactly LEVEL:, EASY, NORMAL, HARD, HARDEST, CTRL:, ROLE 1, ROLE 2, 2 PLAYERS. Keep the spelling exactly",
    "MY:options-portraits": "four expressions of the same young woman portrait in a blue frame; only the facial expression differs between frames",
    "MY:item-balls": "four glowing capsules marked with the letters L, B, K, S",
    "BEG1:title-logo": "the game logo letters 'RS-S' with three frames of the last S morphing into the digit 2, so the logo finally reads RS-2",
    "BEG1:title-eyes": "an extreme close-up of an anime hero's eyes and hair",
    "BEG1:title-hand": "a close-up of a gloved hand gripping a control lever",
    "BEG1:ship-front-zoom": "a spaceship seen from the front at five sizes, the same ship",
    "BEG1:ship-approach": "a spaceship with blue and white hull and orange engine rings, seen from behind while it flies toward a planet, at nine sizes, the same ship. The large black shape in its middle is part of the ship and stays black. The noisy white, blue and grey pixels around the ship are only the planet's cloud layer behind it: do NOT paint them and do NOT turn them into machinery. Replace everything around the ship with the flat background colour",
    "BEG1:ship-exhaust": "engine exhaust flares and small glowing sparks, eight frames growing in size. The noisy pixels around the flare are only the planet behind it: do NOT paint them. Paint only the flare and sparks, and replace everything around them with the flat background colour",
    "BEG1:title-earth": "a realistic view of planet Earth from space on black",
    "BEG1:planet-flash": "a planet with a bright flash at three sizes",
    "SHT5:end-letters": "the three large stylized letters E, N, D",
    "SHT1:boss-body": "one huge side-view biomechanical alien battleship drawn as a single continuous machine",
    "SHT2:midboss-body": "one long biomechanical mid-boss warship drawn as a single continuous machine",
    "SHT2:boss-body": "one huge biomechanical lion-headed boss machine drawn as a single continuous machine",
}


def pal_of(lib, i):
    # stage2_d 是第二關淡出的工作表，loader 是載入畫面，兩者都不是這張圖的代表色
    ps = [p for p in E[(lib, i)]["palettes"] if p not in ("stage2_d", "loader")]
    if ps:
        return ps[0]
    return S.manifest["banks"][lib]["palette"]


def is_opaque(lib, i):
    """覆寫方式繪製的圖連黑色也會蓋掉底下，所以整個矩形都不透明。"""
    if (lib, i) in MASKED_BY_ITEM or (lib, i) in FREE_ALPHA:
        return False
    e = E[(lib, i)]
    if e["mask"].get("used"):
        return False
    modes = " ".join(e["draw_modes"])
    return "replace" in modes or "to-bg-store" in modes


def silhouette(lib, i):
    """回傳 (布林陣列, 遮罩來源, dy)。"""
    px = S.indexed(lib, i)
    if is_opaque(lib, i):
        return np.ones(px.shape, bool), None, 0
    if (lib, i) in MASKED_BY_ITEM:
        m = S.indexed(lib, MASKED_BY_ITEM[(lib, i)])
        out = px != 0
        h = min(px.shape[0], m.shape[0])
        out[:h] |= m[:h] == 0
        return out, f"item:{MASKED_BY_ITEM[(lib, i)]}", 0
    e = E[(lib, i)]
    mk = e["mask"]
    if mk.get("used") and mk.get("index") is not None:
        mb = S.mask_bits(lib, mk["index"])
        if mb is not None and mb.shape[1] == px.shape[1]:
            best = None
            for dy in (0, 1):
                inside = 0
                for y in range(px.shape[0]):
                    my = y + dy
                    if 0 <= my < mb.shape[0]:
                        inside += int(((px[y] != 0) & (mb[my] == 0)).sum())
                if best is None or inside > best[0]:
                    best = (inside, dy)
            dy = best[1]
            sil = px != 0
            for y in range(px.shape[0]):
                my = y + dy
                if 0 <= my < mb.shape[0]:
                    sil[y] |= mb[my] == 0
            return sil, f"mask:{mk['index']}", dy
    return px != 0, None, 0


def derive_phases():
    """找出相位圖：與基礎圖為循環位移，或差異很小的近似位移。"""
    out = {}
    px = lambda i: S.indexed("BH2", i)  # noqa: E731
    for base, members in ((5, range(6, 13)), (13, range(14, 17)), (17, [18]), (19, range(20, 23))):
        b = px(base)
        for i in members:
            t = px(i)
            best = max(range(b.shape[1]), key=lambda k: int((np.roll(b, k, axis=1) == t).sum()))
            match = float((np.roll(b, best, axis=1) == t).mean())
            out[i] = {"base": base, "shift": int(best), "match": round(match, 4)}
    return out


plan = {"scale": 4, "sheets": [], "derived": [], "protected": [], "placeholder": [], "map": {}, "phase_report": {}}
handled = set()

for k in sorted(PROTECTED):
    plan["protected"].append({"lib": k[0], "index": k[1], "palette": pal_of(*k)})
    handled.add(k)

for e in U["entries"]:
    k = (e["lib"], e["index"])
    if e["lib"] != "MAP" and e["status"] == "placeholder":
        plan["placeholder"].append({"lib": k[0], "index": k[1]})
        handled.add(k)

phases = derive_phases()
plan["phase_report"] = {str(i): v for i, v in phases.items()}
# 近似度夠高才算相位；第 5～12、19～22 組有手工差異，低於門檻的留給 Codex 成組重繪。
PHASE_MIN = 0.93
for i, v in phases.items():
    if v["match"] >= PHASE_MIN:
        plan["derived"].append({"lib": "BH2", "index": i, "kind": "shift", "base": v["base"], "shift": v["shift"], "match": v["match"]})
        handled.add(("BH2", i))
for (lib, i), m in MASKED_BY_ITEM.items():
    plan["derived"].append({"lib": lib, "index": m, "kind": "mask-from-alpha", "base": i, "rows": S.size(lib, m)[1]})
    handled.add((lib, m))

groups = []


def add_group(name, lib, members, role, note=None):
    groups.append({"name": name, "lib": lib, "members": list(members), "role": role, "note": note})


# 合成體
in_comp = set()
for name, c in COMPOSITES.items():
    in_comp |= {(c["lib"], i) for i in c["parts"]}
for name, g in U["groups"].items():
    if g["lib"] == "MAP":
        continue
    ms = [m for m in g["members"] if (g["lib"], m) not in handled and (g["lib"], m) not in in_comp]
    if not ms:
        continue
    single = ":single-" in name
    role = E[(g["lib"], ms[0])]["role"]
    add_group(name, g["lib"], ms, role, g.get("note"))
# 沒有 group 的索引
grouped = {(g["lib"], m) for g in groups for m in g["members"]}
for e in U["entries"]:
    k = (e["lib"], e["index"])
    if e["lib"] == "MAP" or k in handled or k in in_comp or k in grouped:
        continue
    add_group(f"{e['lib']}:ungrouped-{e['index']}", e["lib"], [e["index"]], e["role"] or "unused")


def sheet_from(name, lib, members, role, palname, note, composite=None):
    """回傳一張或多張參考表，放不下 Z>=4 就切半。"""
    if composite:
        parts = composite["parts"]
        W = max(S.size(lib, i)[0] + x for i, (x, y) in parts.items())
        H = max(S.size(lib, i)[1] + y for i, (x, y) in parts.items())
        lay = best_layout([(W, H)], gap=8, zmin=2)
        canvas, z, pos = lay
        ox, oy = pos[0]
        pieces = []
        colors = []
        for i, (x, y) in parts.items():
            sil, ms, dy = silhouette(lib, i)
            w, h = S.size(lib, i)
            pieces.append({"lib": lib, "index": i, "x": ox + x * z, "y": oy + y * z, "w": w, "h": h, "palette": palname,
                           "opaque": is_opaque(lib, i), "free": (lib, i) in FREE_ALPHA, "mask": ms, "dy": dy})
            colors.append(S.palette(palname)[np.unique(S.indexed(lib, i))])
        return [{"name": name, "canvas": list(canvas), "zoom": z, "pieces": pieces, "role": role, "note": note,
                 "colors": np.vstack(colors), "composite": True}]
    sizes = [S.size(lib, i) for i in members]
    lay = best_layout(sizes, zmin=4)
    if lay is None:
        if len(members) == 1:
            lay = best_layout(sizes, gap=8, zmin=2)
            if lay is None:
                raise SystemExit(f"{name} {members} 放不下")
        else:
            mid = len(members) // 2
            return sheet_from(name + "-a", lib, members[:mid], role, palname, note) + \
                sheet_from(name + "-b", lib, members[mid:], role, palname, note)
    canvas, z, pos = lay
    pieces = []
    colors = []
    for k, i in enumerate(members):
        sil, ms, dy = silhouette(lib, i)
        w, h = sizes[k]
        pal_i = pal_of(lib, i)
        pieces.append({"lib": lib, "index": i, "x": pos[k][0], "y": pos[k][1], "w": w, "h": h, "palette": pal_i,
                       "opaque": is_opaque(lib, i), "free": (lib, i) in FREE_ALPHA, "mask": ms, "dy": dy})
        colors.append(S.palette(pal_i)[np.unique(S.indexed(lib, i))])
    return [{"name": name, "canvas": list(canvas), "zoom": z, "pieces": pieces, "role": role, "note": note,
             "colors": np.vstack(colors), "composite": False}]


sheets = []
for name, c in COMPOSITES.items():
    first = next(iter(c["parts"]))
    sheets += sheet_from(name, c["lib"], list(c["parts"]), "boss_part", pal_of(c["lib"], first), None, composite=c)
# 單張小圖合併成同庫同調色盤的表，每表最多 8 張，減少生成次數且畫風一致
small = {}
for g in groups:
    if ":single-" in g["name"] or ":ungrouped-" in g["name"]:
        i = g["members"][0]
        w, h = S.size(g["lib"], i)
        if w * h <= 64 * 64:
            small.setdefault((g["lib"], pal_of(g["lib"], i)), []).append(g)
            continue
    p = pal_of(g["lib"], g["members"][0])
    sheets += sheet_from(g["name"].replace(":", "-"), g["lib"], g["members"], g["role"], p, g["note"])
for (lib, p), gs in small.items():
    for k in range(0, len(gs), 8):
        chunk = gs[k:k + 8]
        ms = [g["members"][0] for g in chunk]
        sheets += sheet_from(f"{lib}-misc-{p}-{k // 8}", lib, ms, "misc", p, None)

for s in sheets:
    chroma, dist = pick_chroma(s.pop("colors"))
    s["chroma"] = chroma
    s["chroma_distance"] = round(dist, 1)
    s["name"] = s["name"].replace(":", "-")
    s["desc"] = GROUP_DESC.get(s["name"].replace("-", ":", 1)) or GROUP_DESC.get(s["name"]) or None
    for p in s["pieces"]:
        handled.add((p["lib"], p["index"]))
plan["sheets"] = sheets

# MAP：連通物件，超過 Z=4 寬度就切成有重疊的面板
raw = unrle(open(os.path.join(src, "SHT.SCN"), "rb").read(), 0)[0]
COLS, ROWS = 600, 20
g = np.array(struct.unpack_from(f"<{COLS * ROWS}H", raw, 0), dtype=np.int32).reshape(ROWS, COLS)
seen = np.zeros(g.shape, bool)
objs = []
for r in range(ROWS):
    for c in range(COLS):
        if seen[r, c] or g[r, c] == 0:
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
                    if 0 <= n[0] < ROWS and 0 <= n[1] < COLS and not seen[n] and g[n] != 0:
                        seen[n] = True
                        st.append(n)
        r0, r1 = min(x[0] for x in cells), max(x[0] for x in cells)
        c0, c1 = min(x[1] for x in cells), max(x[1] for x in cells)
        objs.append({"r0": r0, "r1": r1, "c0": c0, "c1": c1})
MAXW = 46  # 欄，46x8=368 像素，Z=4 為 1472
OVER = 4   # 重疊欄數
uniq = {}
for o in sorted(objs, key=lambda o: o["c0"]):
    key = (o["r1"] - o["r0"], o["c1"] - o["c0"], g[o["r0"]:o["r1"] + 1, o["c0"]:o["c1"] + 1].tobytes())
    if key in uniq:
        uniq[key]["instances"].append([int(o["r0"]), int(o["c0"])])
        continue
    uniq[key] = {"r0": int(o["r0"]), "r1": int(o["r1"]), "c0": int(o["c0"]), "c1": int(o["c1"]), "instances": [[int(o["r0"]), int(o["c0"])]]}
map_objs = []
panels = []
for n, o in enumerate(uniq.values()):
    o["id"] = n
    w = o["c1"] - o["c0"] + 1
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
        name = f"MAP-obj{n:02d}-p{k}"
        o["panels"].append(name)
        panels.append({"name": name, "obj": n, "r0": o["r0"], "r1": o["r1"], "c0": c0, "c1": c1})
    map_objs.append(o)
plan["map"] = {"columns": COLS, "rows": ROWS, "objects": map_objs, "panels": panels, "overlap_columns": OVER,
               "unused_tiles": [i for i in range(len(S.banks["MAP"][0])) if i not in set(g.ravel().tolist())]}
for p in panels:
    W, H = (p["c1"] - p["c0"] + 1) * 8, (p["r1"] - p["r0"] + 1) * 10
    lay = best_layout([(W, H)], gap=8, zmin=2)
    canvas, z, pos = lay
    p.update({"canvas": list(canvas), "zoom": z, "x": pos[0][0], "y": pos[0][1], "w": W, "h": H, "palette": "stage3", "chroma": "FF00FF"})
# 地圖沒用到的圖塊也要重繪，單獨成一張表
unused_tiles = [i for i in plan["map"]["unused_tiles"]]
extra = sheet_from("MAP-unused-tiles", "MAP", unused_tiles, "unused", "stage3", None)
for s in extra:
    chroma, dist = pick_chroma(s.pop("colors"))
    s["chroma"], s["chroma_distance"], s["desc"] = chroma, round(dist, 1), "small leftover wall tiles of a biomechanical alien fortress"
plan["sheets"] += extra
for i in range(len(S.banks["MAP"][0])):
    handled.add(("MAP", i))

missing = [f"{e['lib']}#{e['index']}" for e in U["entries"] if (e["lib"], e["index"]) not in handled]
if missing:
    raise SystemExit("未處理：" + " ".join(missing))
json.dump(plan, open(out, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
zs = sorted(s["zoom"] for s in plan["sheets"])
print("參考表", len(plan["sheets"]), "倍率", zs[0], "~", zs[-1], "低於 4 倍的", [s["name"] for s in plan["sheets"] if s["zoom"] < 4])
print("地圖物件", len(map_objs), "面板", len(panels), "面板倍率", sorted({p["zoom"] for p in panels}), "未用圖塊", plan["map"]["unused_tiles"])
print("衍生", len(plan["derived"]), "保護", len(plan["protected"]), "佔位", len(plan["placeholder"]))
print("相位吻合", {k: v["match"] for k, v in plan["phase_report"].items()})
