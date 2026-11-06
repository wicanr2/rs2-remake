"""單色（CGA、MGA）HD 重繪試稿的共用函式。在 rs2-hdtools 容器內執行。

單色圖庫的幾何（docs/spec/006-display-modes.md、crates/rs2-gfx/src/mono_hd.rs）：
  每列位元組數 stride，一個位元組 8 個單色像素 = 4 個邏輯像素 = 16 個 HD 像素。
  圖 HD 尺寸 = (stride x 16) x (高 x 4)。單色像素在 HD 是 2 寬 4 高。
  遮罩庫（MYB.BDT、SHT*.BDH）1 = 保留背景，0 = 圖在此處。
  HD 圖讀 R 通道，>= 128 是亮點。
"""
import hashlib
import os
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_lib import shelf_pack  # noqa: E402
from rs2_common import mono_to_rows, parse_bank, unrle  # noqa: E402

SCALE = 4
CHROMA = (255, 0, 255)
CHROMA_HEX = "FF00FF"
CHROMA_NAME = "magenta"
BLK_W = 16  # 一個位元組的 HD 寬


def sha256(path):
    return hashlib.sha256(open(path, "rb").read()).hexdigest()


class MonoSource:
    """原版單色圖庫的唯讀存取。src 是 original/rs-2。"""

    def __init__(self, src):
        self.src = src
        self._b = {}

    def bank(self, name):
        if name not in self._b:
            raw = unrle(open(os.path.join(self.src, name), "rb").read(), 0)[0]
            self._b[name] = {it["index"]: it for it in parse_bank(raw)}
        return self._b[name]

    def item(self, name, i):
        return self.bank(name)[i]

    def plane(self, name, i):
        """(高, stride x 8) 的 0/1 陣列，1 是亮點（遮罩庫則 1 是保留背景）。"""
        it = self.item(name, i)
        return np.array(mono_to_rows(it["data"], it["height"], it["stride"]), np.uint8)

    def hd_size(self, name, i):
        it = self.item(name, i)
        return it["stride"] * BLK_W, it["height"] * SCALE

    def best_dy(self, plane, mask, span=3):
        """圖第 y 列對遮罩第 y + dy 列。取亮點落在遮罩 0 處最多的 dy，平手取小的。"""
        best = None
        for dy in range(span):
            inside = 0
            for y in range(plane.shape[0]):
                my = y + dy
                if 0 <= my < mask.shape[0]:
                    inside += int(((plane[y] == 1) & (mask[my] == 0)).sum())
            if best is None or inside > best[0]:
                best = (inside, dy)
        return best[1], best[0], int(plane.sum())

    def silhouette(self, p):
        """與圖同尺寸的布林陣列，True = 不透明。亮點加上遮罩 0 的區域（含黑邊）。"""
        plane = self.plane(p["bank"], p["index"])
        if p["opaque"]:
            return np.ones(plane.shape, bool)
        sil = plane.astype(bool)
        if p.get("mask"):
            mb, mi = p["mask"].split(":")
            m = self.plane(mb, int(mi))
            if m.shape[1] != plane.shape[1]:
                raise SystemExit(f"遮罩寬度不符 {p['bank']}:{p['index']} {plane.shape} 遮罩 {m.shape}")
            for y in range(plane.shape[0]):
                my = y + p["dy"]
                if 0 <= my < m.shape[0]:
                    sil[y] |= m[my] == 0
        return sil


def even_layout(sizes, gap=32, zmin=4, zmax=16):
    """sizes 是 (邏輯寬, 邏輯高)。單色像素在表上是 z/2 寬，所以倍率取偶數。回傳 (canvas, zoom, 位置) 或 None。"""
    best = None
    for canvas in ((1536, 1024), (1024, 1536), (1024, 1024)):
        for z in range(zmax, zmin - 1, -2):
            pos = shelf_pack(sizes, canvas, z, gap)
            if pos is not None:
                if best is None or z > best[1]:
                    best = (canvas, z, pos)
                break
    return best


def render_ref_piece(canvas, p, z, S):
    """把單色圖放大畫進參考表：亮點白、其他黑，輪廓外保持底色。每個單色像素 z/2 寬 z 高。"""
    plane = S.plane(p["bank"], p["index"])
    sil = S.silhouette(p)
    pw, ph = z // 2, z
    big = np.repeat(np.repeat(plane * 255, ph, axis=0), pw, axis=1)
    bigm = np.repeat(np.repeat(sil, ph, axis=0), pw, axis=1)
    h, w = big.shape
    region = canvas[p["y"]:p["y"] + h, p["x"]:p["x"] + w]
    region[bigm] = big[bigm][:, None]


def grey_to_rgb(a):
    return np.stack([a, a, a], axis=-1)
