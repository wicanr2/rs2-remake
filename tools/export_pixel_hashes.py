#!/usr/bin/env python3
"""規格 001 A1 的跨實作基準：每個圖庫項目解碼後內容的雜湊。

用法：export_pixel_hashes.py <原版目錄> <輸出 TSV>
每列：檔名、區段、項目索引、每列位元組、高、SHA-256。
雜湊對象是 u16le 寬 || u16le 高 || 解碼後的原始位元組。寬是每列位元組數。
Rust 端（crates/rs2-formats）各自計算，兩邊不共用程式。
"""
import hashlib
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rs2_common import go_sections, parse_bank, unrle  # noqa: E402


def main():
    src, out = sys.argv[1], sys.argv[2]
    lines = []
    for name in ("MY.GO", "SHT1.GO", "BH1.GO"):
        buf = open(os.path.join(src, name), "rb").read()
        for sec, s in enumerate(go_sections(buf)):
            raw, _, _ = unrle(buf, s)
            for it in parse_bank(raw):
                h = hashlib.sha256(struct.pack("<HH", it["stride"], it["height"]) + it["data"]).hexdigest()
                lines.append(f"{name}\t{sec}\t{it['index']}\t{it['stride']}\t{it['height']}\t{h}\n")
    with open(out, "w", encoding="utf-8") as fh:
        fh.writelines(lines)
    print(len(lines), "項")


main()
