#!/usr/bin/env python3
"""把發行包的資料（original/ 與 hd/）打成 Android 用的 rs2-data.pak。在 docker 內跑，來源唯讀。

用法：pkg_android_pak.py <payload 目錄> <輸出檔>
格式見 crates/rs2-play/src/pak.rs。路徑排序，所以同樣的內容產生同樣的檔案，識別碼也相同。
"""
import hashlib
import struct
import sys
from pathlib import Path

MAGIC = b"RS2PAK1\n"


def main():
    root, out = Path(sys.argv[1]), Path(sys.argv[2])
    files = []
    for top in ("original", "hd"):
        base = root / top
        if not base.is_dir():
            sys.exit(f"pak：缺 {base}")
        files.extend(sorted(p for p in base.rglob("*") if p.is_file()))
    if not files:
        sys.exit("pak：沒有檔案")
    blobs = [p.read_bytes() for p in files]
    with out.open("wb") as f:
        f.write(MAGIC)
        f.write(struct.pack("<I", len(files)))
        for p, data in zip(files, blobs):
            rel = p.relative_to(root).as_posix().encode("utf-8")
            f.write(struct.pack("<H", len(rel)))
            f.write(rel)
            f.write(struct.pack("<Q", len(data)))
            f.write(hashlib.sha256(data).digest())
        for data in blobs:
            f.write(data)
    print(f"pak：{len(files)} 檔，{out.stat().st_size / (1 << 20):.1f} MiB")


main()
