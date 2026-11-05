#!/usr/bin/env python3
"""解開後的目錄樹對包內 MANIFEST.json：每個檔案的 SHA-256 與大小、符號連結、沒有多出來的檔案。
用法：pkg_check_tree.py <根目錄> <必備路徑...>"""
import hashlib
import json
import os
import sys
from pathlib import Path

root = Path(sys.argv[1])
must = sys.argv[2:]
m = json.loads((root / "MANIFEST.json").read_text(encoding="utf-8"))["files"]
bad = []
for rel, v in m.items():
    p = root / rel
    if "symlink" in v:
        if not p.is_symlink() or os.readlink(p) != v["symlink"]:
            bad.append(f"連結 {rel}")
        continue
    if not p.is_file():
        bad.append(f"缺 {rel}")
        continue
    h = hashlib.sha256()
    with p.open("rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    if h.hexdigest() != v["sha256"] or p.stat().st_size != v["bytes"]:
        bad.append(f"雜湊 {rel}")
on_disk = {p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file() or p.is_symlink()}
extra = sorted(on_disk - set(m) - {"MANIFEST.json"})
miss = [x for x in must if not (root / x).exists()]
exe = [x for x in ("AppRun", "usr/bin/rs2-play") if not os.access(root / x, os.X_OK)]
print(f"{len(m)} 項對 MANIFEST，不符 {len(bad)}，多出 {len(extra)}，缺必備 {len(miss)}，缺執行位元 {len(exe)}")
for x in (bad + extra + miss + exe)[:10]:
    print("  ", x)
sys.exit(1 if bad or extra or miss or exe else 0)
