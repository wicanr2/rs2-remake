#!/usr/bin/env python3
"""產生 README 用的原版與 HD 對照圖，輸出到 docs/images/。
這批圖含原版美術，進版控是 AGENTS.md 硬規則 4 的例外，repo 要維持私人。

用法：readme_images.py <workspace 目錄> <輸出目錄>
來源：workspace/rs2/shots（重製實際合成的畫面）、workspace/hd/assets/overview（各素材庫總覽）、
workspace/extracted/maps 與 workspace/hd/assets/maps（第三關地圖）。
"""
import os
import shutil
import sys

from PIL import Image

W, OUT = sys.argv[1], sys.argv[2]

# 畫面對照：(輸出名, shots 內的檔名前綴)。原版 320x200 放大 3 倍，HD 1280x800 縮成同尺寸。
SHOTS = [
    ("stage1-scroll", "layer_dm1_n0300"),
    ("stage1-boss-enter", "layer_dm1_n0639"),
    ("stage1-boss", "layer_boss1_kill_n0700"),
]
SIZE = (960, 600)

# 第三關地圖的對照窗，單位是原版像素。地圖 4800x200，HD 是 4 倍。
MAP_X, MAP_W = 1200, 480

LIBS = ["MY", "SHT1", "SHT2", "SHT3", "SHT4", "SHT5", "BH1", "BH2", "BH4", "BEG1", "J1", "J2", "J3", "J4"]


def main():
    os.makedirs(os.path.join(OUT, "game"), exist_ok=True)
    os.makedirs(os.path.join(OUT, "overview"), exist_ok=True)
    for name, prefix in SHOTS:
        shots = os.path.join(W, "rs2", "shots")
        orig = Image.open(os.path.join(shots, prefix + "_orig.png")).convert("RGB")
        hd = Image.open(os.path.join(shots, prefix + "_hd.png")).convert("RGB")
        if orig.size != (320, 200) or hd.size != (1280, 800):
            raise SystemExit(f"{prefix} 尺寸不符：{orig.size} {hd.size}")
        orig.resize(SIZE, Image.NEAREST).save(os.path.join(OUT, "game", name + "-orig.png"), optimize=True)
        hd.resize(SIZE, Image.LANCZOS).save(os.path.join(OUT, "game", name + "-hd.png"), optimize=True)
    for lib in LIBS:
        src = os.path.join(W, "hd", "assets", "overview", lib + ".png")
        shutil.copyfile(src, os.path.join(OUT, "overview", lib + ".png"))
    orig = Image.open(os.path.join(W, "extracted", "maps", "SHT.SCN.vga.png")).convert("RGB")
    hd = Image.open(os.path.join(W, "hd", "assets", "maps", "SHT.SCN.hd.png")).convert("RGB")
    if hd.size != (orig.width * 4, orig.height * 4):
        raise SystemExit(f"地圖尺寸不符：{orig.size} {hd.size}")
    a = orig.crop((MAP_X, 0, MAP_X + MAP_W, orig.height)).resize((MAP_W * 3, orig.height * 3), Image.NEAREST)
    b = hd.crop((MAP_X * 4, 0, (MAP_X + MAP_W) * 4, hd.height)).resize((MAP_W * 3, orig.height * 3), Image.LANCZOS)
    sheet = Image.new("RGB", (a.width, a.height + b.height + 12), (40, 40, 40))
    sheet.paste(a, (0, 0))
    sheet.paste(b, (0, a.height + 12))
    sheet.save(os.path.join(OUT, "game", "stage3-map.png"), optimize=True)
    for root, _, files in os.walk(OUT):
        for f in sorted(files):
            p = os.path.join(root, f)
            print(os.path.relpath(p, OUT), os.path.getsize(p) // 1024, "KB")


main()
