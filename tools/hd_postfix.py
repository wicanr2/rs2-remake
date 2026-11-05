#!/usr/bin/env python3
"""HD 成品的確定性修補，不呼叫生成模型。在 hd_finalize.py 之後、hd_variants.py 之前跑，可以重複執行：
每次都從 hd_finalize 留下的 <庫>/<索引>.png 副本（<庫>/_final/<索引>.png）重算。

用法：hd_postfix.py <workspace>

修補項目（接入遊戲後看到的問題，清單見 docs/hd/codex-handoff.md）：
1. 第一關背景循環接縫：BH1 第 0、1、2、3 項左右兩端接不起來。原版素材可循環。
   沿接縫把兩側的顏色差平攤到附近的欄，形狀不動，遮罩不受影響。
2. 山影與倒懸岩交界：BH1 第 3 項底部與第 2 項頂部在原版都是黑色，HD 重繪後是深灰，有水平色帶。底部與頂部漸暗成黑。
3. 頭目本體的矩形邊線：SHT1 第 33 至 35 項是不透明矩形，黑底部分被重繪成接近黑的暗綠，邊界看得見。接近黑的像素壓成純黑。
"""
import json
import os
import shutil
import sys

import numpy as np
from PIL import Image

ws = sys.argv[1]
out = os.path.join(ws, "hd", "assets")


def load(lib, i):
    """取 hd_finalize 的原始結果。第一次執行時先留副本。"""
    p = os.path.join(out, lib, f"{i:04d}.png")
    keep = os.path.join(out, lib, "_final", f"{i:04d}.png")
    if not os.path.exists(keep):
        os.makedirs(os.path.dirname(keep), exist_ok=True)
        shutil.copyfile(p, keep)
    return np.array(Image.open(keep).convert("RGBA"), np.float32), p


def save(arr, p):
    Image.fromarray(np.clip(arr + 0.5, 0, 255).astype(np.uint8)).save(p, optimize=True)


def level_wrap(a, n):
    """把左右兩端的顏色差平攤到兩側各 n 欄。差值沿列方向平滑，避免出現橫紋。"""
    h, w = a.shape[:2]
    # 接縫兩側取幾欄的平均，比單欄穩定。
    k = 3
    left, right = a[:, :k, :3].mean(axis=1), a[:, w - k :, :3].mean(axis=1)
    both = (a[:, :k, 3].mean(axis=1) > 0) & (a[:, w - k :, 3].mean(axis=1) > 0)
    d = (right - left) * both[:, None]
    r = 6
    box = np.ones(2 * r + 1) / (2 * r + 1)
    d = np.stack([np.convolve(d[:, c], box, mode="same") for c in range(3)], axis=1)
    ramp = 1 - (np.arange(n) + 0.5) / n
    b = a.copy()
    op = (a[..., 3] > 0)[..., None]
    b[:, :n, :3] += (d[:, None, :] / 2) * ramp[None, :, None] * op[:, :n]
    b[:, w - n :, :3] -= (d[:, None, :] / 2) * ramp[::-1][None, :, None] * op[:, w - n :]
    return b


def fade_rows(a, rows, top):
    """最上或最下幾列漸暗成黑，透明度不動。"""
    h = a.shape[0]
    t = (np.arange(rows) + 0.5) / rows
    b = a.copy()
    if top:
        b[:rows, :, :3] *= t[:, None, None]
    else:
        b[h - rows :, :, :3] *= t[::-1][:, None, None]
    return b


def crush_black(a, limit):
    """亮度低於 limit 的像素壓成純黑，limit 到兩倍之間線性過渡，保留暗部層次。"""
    lum = a[..., :3].max(axis=2)
    k = np.clip((lum - limit) / limit, 0, 1)[..., None]
    b = a.copy()
    b[..., :3] *= k
    return b


log = {}
for i, n, top, bottom in [(0, 40, 0, 0), (1, 24, 0, 0), (2, 64, 20, 0), (3, 40, 0, 20)]:
    a, p = load("BH1", i)
    a = level_wrap(a, n)
    if top:
        a = fade_rows(a, top, True)
    if bottom:
        a = fade_rows(a, bottom, False)
    save(a, p)
    log[f"BH1:{i}"] = {"level_wrap": n, "fade_top": top, "fade_bottom": bottom}
for i in (33, 34, 35):
    a, p = load("SHT1", i)
    save(crush_black(a, 18), p)
    log[f"SHT1:{i}"] = {"crush_black": 18}
with open(os.path.join(out, "hd-postfix.json"), "w", encoding="utf-8") as f:
    json.dump(log, f, ensure_ascii=False, indent=1)
print("修補", len(log), "張")
