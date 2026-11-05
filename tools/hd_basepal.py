#!/usr/bin/env python3
"""寫 HD 資料夾的 base-palette.tsv：每張 HD 圖是在哪個調色盤下畫的，以及各調色盤的 16 色。
rs2-gfx 用它在執行時把 HD 圖換成當時的調色盤（靜態變體沒有涵蓋的組合）。不呼叫生成模型。

用法：hd_basepal.py <original/rs-2> <workspace>
格式：`P 名稱 96 個十六進位字元`、`I 圖庫 索引 名稱`。
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hd_lib import Source  # noqa: E402

src, ws = sys.argv[1:3]
out = os.path.join(ws, "hd", "assets")
S = Source(src, os.path.join(ws, "unpacked"), os.path.join(ws, "extracted/manifest.json"))
man = json.load(open(os.path.join(out, "hd-manifest.json"), encoding="utf-8"))
lines = []
for name, p in sorted(S.manifest["palettes"].items()):
    lines.append(f"P\t{name}\t{p['vga_rgb_hex']}")
n = 0
for k, a in sorted(man["assets"].items()):
    lib, index = k.rsplit(":", 1)
    base = a.get("palette")
    if a.get("method") == "derived-shift" or not base:
        bank = S.manifest["banks"].get(lib)
        base = bank.get("palette") if bank else None
    if not base or base not in S.manifest["palettes"]:
        continue
    lines.append(f"I\t{lib}\t{index}\t{base}")
    n += 1
with open(os.path.join(out, "base-palette.tsv"), "w", encoding="utf-8") as f:
    f.write("\n".join(lines) + "\n")
print(f"base-palette.tsv：{len(S.manifest['palettes'])} 個調色盤，{n} 張圖")
