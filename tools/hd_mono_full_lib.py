"""單色 HD 全量重繪（m2 灰階賽璐璐）共用函式。在 rs2-hdtools 容器內執行。

在 hd_mono_lib.py 之上補：任意倍率的參考表繪製、任意位移與寬度的遮罩輪廓、各種處理方法的常數。
幾何同 hd_mono_lib.py：每列位元組數 stride，一個位元組 8 個單色像素 = 4 個邏輯像素 = 16 個 HD 像素，
HD 圖尺寸 = (stride x 16) x (高 x 4)，一個單色像素在 HD 是 2 寬 4 高。
"""
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_mono_lib import CHROMA, CHROMA_HEX, CHROMA_NAME, BLK_W, MonoSource, sha256  # noqa: E402,F401

# 單色圖庫檔名 → 彩色圖庫名（sprite-usage.json 與 assets 的資料夾名）
IMG_LIB = {"BEG1.BDB": "BEG1", "BH1.BDB": "BH1", "BH2.BDB": "BH2", "BH4.BDB": "BH4", "J1.GOB": "J1", "J2.GOB": "J2", "J3.GOB": "J3", "J4.GOB": "J4",
           "MY.BDB": "MY", "SHT1.BDB": "SHT1", "SHT2.BDB": "SHT2", "SHT3.BDB": "SHT3", "SHT4.BDB": "SHT4", "SHT5.BDB": "SHT5"}
LIB_BANK = {v: k for k, v in IMG_LIB.items()}
MASK_BANK = {"MY.BDB": "MYB.BDT", "SHT1.BDB": "SHT1.BDH", "SHT2.BDB": "SHT2.BDH", "SHT3.BDB": "SHT3.BDH", "SHT4.BDB": "SHT4.BDH", "SHT5.BDB": "SHT5.BDH"}
# 署名與商標（AGENTS.md「文化保存」）：不送 Codex，維持原版單色最近鄰放大
CREDITS = {("MY.BDB", 15), ("MY.BDB", 26), ("MY.BDB", 47), ("MY.BDB", 48)}
# 同一個圖庫內以圖形式存放的遮罩：(圖, 遮罩項)
MASKED_BY_ITEM = {("BH1.BDB", 3): 4, ("BH2.BDB", 0): 2, ("BH2.BDB", 1): 3, ("BH2.BDB", 19): 23}
# 道具球：遮罩比圖窄，另用「逐列填滿」的輪廓
HULL = {("MY.BDB", 42), ("MY.BDB", 43), ("MY.BDB", 44), ("MY.BDB", 45)}
# 開場飛船與噴焰：原版是連同地球雲層一起貼上的矩形。只重繪物件本身，其餘維持原版矩形內容
FREE = {("BEG1.BDB", i) for i in (8, 9, 10, 11, 12, 13, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28)}
# 亮度規則適用的角色（見 tools/hd-styles/m2-gray-cel-r2.txt）
BRIGHT_ROLES = {"bullet", "item", "explosion", "effect"}
# 文字類的角色與群組：用閾值轉 1 位元（抖動會把字筆畫打成點線）
TEXT_GROUPS = {"MY:act-card", "MY:disk-prompt", "MY:options-labels", "SHT5:end-letters", "BEG1:title-logo"}


def mask_best_offset(plane, mask, span=range(-2, 4)):
    """圖第 y 列對遮罩第 y + dy 列。取亮點落在遮罩 0 處最多的 dy，平手取絕對值小的。欄位只比兩者共有的寬度。"""
    w = min(plane.shape[1], mask.shape[1])
    best = None
    for dy in span:
        inside = 0
        for y in range(plane.shape[0]):
            my = y + dy
            if 0 <= my < mask.shape[0]:
                inside += int(((plane[y, :w] == 1) & (mask[my, :w] == 0)).sum())
        key = (inside, -abs(dy), dy)
        if best is None or key > best[0]:
            best = (key, dy, inside)
    return best[1], best[2], int(plane.sum())


def row_hull(b):
    """每列把最左與最右的 True 之間填滿。"""
    out = b.copy()
    for y in range(b.shape[0]):
        xs = np.nonzero(b[y])[0]
        if len(xs):
            out[y, xs[0]:xs[-1] + 1] = True
    return out


class Full(MonoSource):
    def silhouette(self, p):
        """與圖同尺寸的布林陣列，True = 不透明。"""
        plane = self.plane(p["bank"], p["index"])
        if p["opaque"] or p.get("free"):
            return np.ones(plane.shape, bool)
        sil = plane.astype(bool)
        if p.get("hull"):
            sil = row_hull(sil)
        if p.get("mask"):
            mb, mi = p["mask"].split(":")
            m = self.plane(mb, int(mi))
            w = min(plane.shape[1], m.shape[1])
            for y in range(plane.shape[0]):
                my = y + p["dy"]
                if 0 <= my < m.shape[0]:
                    sil[y, :w] |= m[my, :w] == 0
        return sil


def render_ref_piece_z(canvas, p, z, S):
    """把單色圖畫進參考表，任意倍率 z（邏輯像素 z 個 HD 像素；z 為奇數時單色像素寬度 1、2 交替）。
    亮點白、其餘黑，輪廓外保持底色。回傳實際畫的區域 (x, y, w, h)。"""
    plane = S.plane(p["bank"], p["index"])
    sil = S.silhouette(p)
    h, wpx = plane.shape
    ow, oh = p["w"] * z, h * z
    ys = (np.arange(oh) * h // oh)
    xs = (np.arange(ow) * wpx // ow)
    big = plane[ys][:, xs] * 255
    bigm = sil[ys][:, xs]
    region = canvas[p["y"]:p["y"] + oh, p["x"]:p["x"] + ow]
    region[bigm] = big[bigm][:, None]
    return p["x"], p["y"], ow, oh


def nearest_hd(plane):
    """單色圖放大成 HD 尺寸：每個單色像素 2 寬 4 高。回傳 0/255 的 uint8。"""
    return np.repeat(np.repeat(plane.astype(np.uint8) * 255, 4, axis=0), 2, axis=1)
