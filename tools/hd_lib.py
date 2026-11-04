"""HD 工具共用：載入原版圖庫、輪廓、參考表排版。在 rs2-hdtools 容器內執行。"""
import hashlib
import json
import os
import struct
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rs2_common import dac6_to_rgb, go_sections, parse_bank, planar_to_indexed, unrle  # noqa: E402

DS = 0xCBC0
SCALE = 4
CHROMAS = {"FF00FF": (255, 0, 255), "00FF00": (0, 255, 0), "00FFFF": (0, 255, 255)}


class Source:
    """原版圖庫的唯讀存取。src=original/rs-2，unp=workspace/unpacked。"""

    def __init__(self, src, unp, manifest):
        self.src = src
        self.exe = open(os.path.join(unp, "S.unpacked.bin"), "rb").read()
        self.manifest = json.load(open(manifest, encoding="utf-8"))
        self.banks = {}
        self.masks = {}
        for name in ("MY", "SHT1", "SHT2", "SHT3", "SHT4", "SHT5", "BH1", "BH2", "BH4", "J1", "J2", "J3", "J4", "MAP"):
            b = open(os.path.join(src, name + ".GO"), "rb").read()
            st = go_sections(b)
            sec = [parse_bank(unrle(b, s)[0]) for s in st]
            if len(st) == 5:
                self.masks[name] = sec[0]
                sec = sec[1:]
            self.banks[name] = sec
        self.masks["SHT4"] = parse_bank(unrle(open(os.path.join(src, "SHT4.GOT"), "rb").read(), 0)[0])
        self.banks["BEG1"] = [parse_bank(unrle(open(os.path.join(src, f"BEG1.BD{i}"), "rb").read(), 0)[0]) for i in (1, 2, 3, 4)]

    def size(self, bank, i):
        it = self.banks[bank][0][i]
        return it["stride"] * 8, it["height"]

    def indexed(self, bank, i):
        it = [self.banks[bank][k][i] for k in range(4)]
        rows = planar_to_indexed([x["data"] for x in it], it[0]["height"], it[0]["stride"])
        return np.array([list(r) for r in rows], dtype=np.uint8)

    def palette(self, name):
        p = self.manifest["palettes"][name]
        return np.frombuffer(bytes.fromhex(p["vga_rgb_hex"]), dtype=np.uint8).reshape(16, 3)

    def mask_bits(self, bank, i):
        m = self.masks.get(bank)
        if not m or i >= len(m):
            return None
        it = m[i]
        h, st = it["height"], it["stride"]
        a = np.frombuffer(it["data"], dtype=np.uint8).reshape(h, st)
        return np.unpackbits(a, axis=1)  # 1 = 保留背景

    def silhouette(self, bank, i, mask_index=None, dx=0, dy=0):
        """回傳與圖同尺寸的布林陣列，True = 不透明。

        有配對遮罩時以遮罩為準（可含內部黑色），平面相對遮罩的位移為 (dx, dy)。
        沒有遮罩就以非 0 色號為準。
        """
        px = self.indexed(bank, i)
        opaque = px != 0
        if mask_index is None:
            return opaque
        mb = self.mask_bits(bank, mask_index)
        if mb is None:
            return opaque
        h, w = px.shape
        out = opaque.copy()
        for y in range(h):
            for x in range(w):
                my, mx = y + dy, x + dx
                if 0 <= my < mb.shape[0] and 0 <= mx < mb.shape[1] and mb[my, mx] == 0:
                    out[y, x] = True
        return out

    def rgb(self, bank, i, pal):
        return self.palette(pal)[self.indexed(bank, i)]


def pick_chroma(colors):
    """挑離圖內顏色最遠的底色。colors: Nx3 陣列。"""
    best, bd = "FF00FF", -1
    for name, c in CHROMAS.items():
        d = np.sqrt(((colors.astype(int) - np.array(c)) ** 2).sum(axis=1)).min() if len(colors) else 999
        if d > bd:
            best, bd = name, d
    return best, float(bd)


def shelf_pack(sizes, canvas, zoom, gap):
    """把 (w,h) 依高度排序放進畫布，回傳每塊左上角；放不下回 None。"""
    order = sorted(range(len(sizes)), key=lambda k: (-sizes[k][1], k))
    W, H = canvas
    x = gap
    y = gap
    lh = 0
    pos = {}
    for k in order:
        w, h = sizes[k][0] * zoom, sizes[k][1] * zoom
        if w + 2 * gap > W or h + 2 * gap > H:
            return None
        if x + w + gap > W:
            x = gap
            y += lh + gap
            lh = 0
        if y + h + gap > H:
            return None
        pos[k] = (x, y)
        x += w + gap
        lh = max(lh, h)
    return pos


def best_layout(sizes, gap=32, zmin=1, zmax=16):
    """回傳 (canvas, zoom, 位置)，取可放下的最大整數倍率。"""
    best = None
    for canvas in ((1536, 1024), (1024, 1536), (1024, 1024)):
        for z in range(zmax, zmin - 1, -1):
            pos = shelf_pack(sizes, canvas, z, gap)
            if pos is not None:
                if best is None or z > best[1]:
                    best = (canvas, z, pos)
                break
    return best


def sha256(path):
    return hashlib.sha256(open(path, "rb").read()).hexdigest()
