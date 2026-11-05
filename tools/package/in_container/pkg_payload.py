#!/usr/bin/env python3
"""組出發行包的資料：原版檔案與 HD 素材。在 docker 內跑，來源唯讀。

用法：pkg_payload.py <原版目錄> <HD 資料夾> <原始碼快照> <輸出目錄>

原版：只放 `rs2-formats` 的 EXPECTED 與 `rs2-gfx` 的 MONO_EXPECTED 列的檔案，逐一驗 SHA-256。其餘檔案
（破解組的 SEN.EXE、SEN.PAS、RS2.BAT 等）不進發行包（AGENTS.md 硬規則 7）。
HD：只放執行時會讀的檔案，依 `rs2-gfx` 的 HdAssets 與 MonoHdSink 讀取路徑：
  <圖庫>/NNNN.png、<圖庫>/masks/NNNN.png、<圖庫>/variants/<調色盤>/NNNN.png、base-palette.tsv、mono/。
沒歸類的檔案一律中止，避免素材資料夾多了新東西卻默默漏帶或夾帶。
"""
import hashlib
import json
import os
import re
import shutil
import sys
from pathlib import Path

LIBS = ["MY", "SHT1", "SHT2", "SHT3", "SHT4", "SHT5", "BH1", "BH2", "BH4", "MAP", "J1", "J2", "J3", "J4", "BEG1"]
L = "|".join(LIBS)
KEEP = [
    ("image", re.compile(rf"^(?:{L})/\d{{4}}\.png$")),
    ("mask", re.compile(rf"^(?:{L})/masks/\d{{4}}\.png$")),
    ("variant", re.compile(rf"^(?:{L})/variants/[a-z0-9_]+/\d{{4}}\.png$")),
    ("palette", re.compile(r"^base-palette\.tsv$")),
    ("mono", re.compile(r"^mono/[A-Za-z0-9_.]+/\d{4}\.png$")),
]
# 執行時用不到的：審圖用的總覽、定稿暫存、第三關整張地圖與物件整圖（HdAssets 不讀）、各批次的紀錄檔。
DROP = [
    ("overview", re.compile(r"^overview/")),
    ("final", re.compile(r"/_final/")),
    ("maps", re.compile(r"^maps/")),
    ("MAP-objects", re.compile(r"^MAP/objects/")),
    ("records", re.compile(r"^hd-[a-z-]+\.json$")),
]


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def table(path: Path, name: str, want: int):
    text = path.read_text(encoding="utf-8")
    body = text.split(f"pub const {name}", 1)[1].split("];", 1)[0]
    pairs = re.findall(r'\("([^"]+)",\s*"([0-9a-f]{64})"\)', body)
    if len(pairs) != want:
        sys.exit(f"payload：{name} 解析到 {len(pairs)} 筆，預期 {want} 筆，{path.name} 格式變了？")
    return pairs


def expected(src: Path):
    color = table(src / "crates/rs2-formats/src/manifest.rs", "EXPECTED", 26)
    mono = table(src / "crates/rs2-gfx/src/mono.rs", "MONO_EXPECTED", 21)
    return color + mono


def put(src: Path, dst: Path):
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, dst)
    dst.chmod(0o644)


def main():
    orig, hd, snap, out = map(Path, sys.argv[1:5])
    if out.exists():
        shutil.rmtree(out)
    (out / "original").mkdir(parents=True)

    files = {}
    pairs = expected(snap)
    for name, want in pairs:
        p = orig / name
        if not p.is_file():
            sys.exit(f"payload：原版目錄缺 {name}")
        got = sha256(p)
        if got != want:
            sys.exit(f"payload：{name} 的雜湊不符（預期 {want}，實際 {got}）")
        put(p, out / "original" / name)
        files[f"original/{name}"] = {"sha256": got, "bytes": p.stat().st_size}
    print(f"原版 {len(pairs)} 檔，雜湊全部相符")

    kept = {k: 0 for k, _ in KEEP}
    dropped = {k: 0 for k, _ in DROP}
    dropped_bytes = 0
    unknown = []
    for root, dirs, names in os.walk(hd):
        dirs.sort()
        for n in sorted(names):
            p = Path(root) / n
            rel = p.relative_to(hd).as_posix()
            kind = next((k for k, r in KEEP if r.match(rel)), None)
            if kind:
                put(p, out / "hd" / rel)
                files[f"hd/{rel}"] = {"sha256": sha256(p), "bytes": p.stat().st_size}
                kept[kind] += 1
                continue
            d = next((k for k, r in DROP if r.search(rel)), None)
            if d:
                dropped[d] += 1
                dropped_bytes += p.stat().st_size
            else:
                unknown.append(rel)
    if unknown:
        sys.exit("payload：HD 資料夾有沒歸類的檔案，先決定要不要帶：\n  " + "\n  ".join(unknown[:30]) + (f"\n  ...共 {len(unknown)} 個" if len(unknown) > 30 else ""))
    for lib in LIBS:
        if not any(k.startswith(f"hd/{lib}/") for k in files):
            sys.exit(f"payload：HD 沒有圖庫 {lib}")
    if not any(k == "hd/base-palette.tsv" for k in files):
        sys.exit("payload：HD 缺 base-palette.tsv（執行時換色要用）")
    if kept["mono"] == 0:
        sys.exit("payload：HD 缺 mono/")
    total = sum(v["bytes"] for v in files.values())
    print("HD 保留", kept, "略過", dropped, f"（{dropped_bytes / 1048576:.1f} MiB）")
    print(f"payload 共 {len(files)} 檔，{total / 1048576:.1f} MiB")
    (out / "payload.json").write_text(
        json.dumps({"files": files, "hd_kept": kept, "hd_dropped": dropped, "hd_dropped_bytes": dropped_bytes}, indent=1, sort_keys=True) + "\n"
    )


if __name__ == "__main__":
    main()
