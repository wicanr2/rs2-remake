#!/usr/bin/env python3
"""單色 HD 全量候選的實機畫面並排與總覽。不呼叫外部服務。

用法：hd_mono_full_montage.py <workspace> <模式（cga 或 mga）> <標籤...>
  讀 workspace/hd/mono-m2-review/shots/<模式>-<標籤>-<場景>-<步數>.png（hd_mono_full_shots.sh 產生），標籤 SD 是原版，A 是現況，cand 是候選。
輸出（workspace/hd/mono-m2-review/）：
  side/<模式>-<場景>-<步數>.png   同一步各標籤並排（去掉工具列），每格寬 620
  overview-<模式>-<n>.png         幾步為一張的小圖總覽，每列一步、每欄一個標籤，每格寬 400
"""
import os
import sys

from PIL import Image, ImageDraw, ImageFont

ws, mode = sys.argv[1:3]
labels = sys.argv[3:]
base = os.path.join(ws, "hd", "mono-m2-review")
shots = os.path.join(base, "shots")
os.makedirs(os.path.join(base, "side"), exist_ok=True)
try:
    FONT = ImageFont.load_default(size=18)
except TypeError:
    FONT = ImageFont.load_default()
BG = (38, 38, 70)
FLOW = [30, 60, 90, 130, 170, 360, 460, 700, 760, 940, 975, 1040, 1100, 1140]
OPEN = [1, 2, 3, 4, 5, 6, 8, 10, 12, 14, 16, 18, 20, 40, 60, 80, 100, 120, 140, 160, 180, 200, 230, 260, 300]
NAMES = {30: "title", 60: "title", 90: "title", 130: "stage 1 card", 170: "stage 1", 360: "stage 2", 460: "stage 2", 700: "stage 3", 760: "stage 3", 940: "stage 4", 975: "stage 4",
         1040: "ending", 1100: "ending", 1140: "ending"}


def game(path):
    im = Image.open(path).convert("RGB")
    return im.crop((0, 80, im.width, im.height))  # 去掉工具列


def row(scene, step, w):
    ims = []
    for lab in labels:
        p = os.path.join(shots, f"{mode}-{lab}-{scene}-{step}.png")
        if os.path.exists(p):
            g = game(p)
            ims.append((lab, g.resize((w, round(g.height * w / g.width)), Image.BOX)))
    return ims


def sheet(rows, w, title):
    h = max(im.height for _, ims in rows for _, im in ims) if rows else 1
    cols = len(labels)
    W = cols * w + (cols + 1) * 6
    H = len(rows) * (h + 24) + 28
    cv = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(cv)
    d.text((6, 3), title, fill=(255, 255, 0), font=FONT)
    y = 28
    for caption, ims in rows:
        for k, (lab, im) in enumerate(ims):
            x = 6 + k * (w + 6)
            d.text((x, y), f"{lab}  {caption}", fill=(180, 255, 180), font=FONT)
            cv.paste(im, (x, y + 20))
        y += h + 24
    return cv


n = 0
for scene, steps in (("flow", FLOW), ("open", OPEN)):
    for step in steps:
        ims = row(scene, step, 620)
        if not ims:
            continue
        sheet([(f"{scene} {step}", ims)], 620, f"{mode} {scene} step {step}").save(os.path.join(base, "side", f"{mode}-{scene}-{step}.png"), optimize=True)
        n += 1
# 總覽
parts = [("flow-1", [(f"{NAMES[s]} {s}", row("flow", s, 400)) for s in FLOW[:7]]),
         ("flow-2", [(f"{NAMES[s]} {s}", row("flow", s, 400)) for s in FLOW[7:]])]
for k in range(0, len(OPEN), 7):
    parts.append((f"open-{k // 7 + 1}", [(f"open {s}", row("open", s, 400)) for s in OPEN[k:k + 7]]))
for name, rows in parts:
    rows = [(c, i) for c, i in rows if i]
    if rows:
        sheet(rows, 400, f"{mode} {name}").save(os.path.join(base, f"overview-{mode}-{name}.png"), optimize=True)
print("side", n, "->", base)

# 總覽一張圖：CGA 與 MGA 的 SD、現況、候選並排，每列一個場景（兩個模式的畫面都在才做，標籤固定 SD、A、cand）
if mode == "mga":
    cols = [(m, lab) for m in ("cga", "mga") for lab in ("SD", "A", "cand")]
    scenes = [("opening 6", "open", 6), ("opening ship 8", "open", 8), ("title 60", "flow", 60), ("stage 1 card 130", "flow", 130), ("stage 1 170", "flow", 170),
              ("stage 2 360", "flow", 360), ("stage 3 760", "flow", 760), ("stage 4 940", "flow", 940), ("ending 1040", "flow", 1040), ("ending 1100", "flow", 1100)]
    w = 300
    rows = []
    for cap, scene, step in scenes:
        ims = []
        for m, lab in cols:
            p = os.path.join(shots, f"{m}-{lab}-{scene}-{step}.png")
            if os.path.exists(p):
                g = game(p)
                ims.append((f"{m} {lab}", g.resize((w, round(g.height * w / g.width)), Image.BOX)))
        if ims:
            rows.append((cap, ims))
    if rows:
        h = max(im.height for _, ims in rows for _, im in ims)
        W = len(cols) * w + (len(cols) + 1) * 6
        cv = Image.new("RGB", (W, len(rows) * (h + 24) + 28), BG)
        d = ImageDraw.Draw(cv)
        d.text((6, 3), "overview: CGA and MGA, SD original | current assets | candidate", fill=(255, 255, 0), font=FONT)
        y = 28
        for cap, ims in rows:
            for k, (lab, im) in enumerate(ims):
                x = 6 + k * (w + 6)
                d.text((x, y), f"{lab}  {cap}", fill=(180, 255, 180), font=FONT)
                cv.paste(im, (x, y + 20))
            y += h + 24
        cv.save(os.path.join(base, "overview.png"), optimize=True)
        print("overview.png", cv.size)
