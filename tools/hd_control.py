#!/usr/bin/env python3
"""切圖工具的正負對照。不呼叫外部服務。

正對照：把參考表直接當原稿切回，輪廓 IoU 必須接近 1。
負對照：把原稿整張往右下移 z*8 像素，不透明小圖的 IoU 必須明顯下降。
用法：hd_control.py <workspace> <批次名>
"""
import json
import os
import sys

import numpy as np
from PIL import Image

ws, batch = sys.argv[1:3]
mode = sys.argv[3]
root = os.path.join(ws, "hd", batch)
os.makedirs(os.path.join(root, "masters"), exist_ok=True)
for f in os.listdir(os.path.join(root, "ref")):
    a = np.array(Image.open(os.path.join(root, "ref", f)).convert("RGB"))
    if mode == "shift":
        bg = a[0, 0].copy()
        s = np.full_like(a, 0)
        s[:] = bg
        s[40:, 40:] = a[:-40, :-40]
        a = s
    Image.fromarray(a).save(os.path.join(root, "masters", f))
print("原稿", mode, len(os.listdir(os.path.join(root, "masters"))))
