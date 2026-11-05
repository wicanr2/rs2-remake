#!/usr/bin/env python3
"""第三方授權聲明。在 docker 內跑，離線。

用法：pkg_notices.py <rs2-play 的 Cargo.toml> <SDL2 原始碼 tar.gz> <輸出目錄>
輸出：THIRD-PARTY-NOTICES.txt（Rust crate 清單與授權全文）、SDL2-LICENSE.txt（取自 SDL2 原始碼）。
crate 清單來自 `cargo metadata --locked --offline`，授權檔取自 crate 原始碼目錄，所以和實際連結進去的版本一致。
"""
import hashlib
import json
import subprocess
import sys
import tarfile
from pathlib import Path


def main():
    manifest, sdl_tar, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    out.mkdir(parents=True, exist_ok=True)

    with tarfile.open(sdl_tar) as t:
        lic = t.extractfile("SDL2-2.26.5/LICENSE.txt").read()
    (out / "SDL2-LICENSE.txt").write_bytes(lic)

    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--offline", "--manifest-path", str(manifest)], text=True))
    by_id = {p["id"]: p for p in meta["packages"]}
    reach = {n["id"] for n in meta["resolve"]["nodes"]}
    ext = sorted((by_id[i] for i in reach if by_id[i].get("source")), key=lambda p: (p["name"], p["version"]))

    texts = {}  # sha -> (text, [crate])
    lines = []
    for p in ext:
        d = Path(p["manifest_path"]).parent
        files = sorted(f for f in d.iterdir() if f.is_file() and f.name.upper().startswith(("LICENSE", "COPYING", "UNLICENSE")))
        crate = f'{p["name"]} {p["version"]}'
        lines.append(f'  {crate}  授權：{p.get("license") or "（未標示）"}  來源：{p.get("repository") or "crates.io"}')
        if not files:
            lines.append("    （套件內沒有授權檔，依上列授權標示）")
        for f in files:
            data = f.read_bytes().decode("utf-8", "replace").strip() + "\n"
            h = hashlib.sha256(data.encode()).hexdigest()
            texts.setdefault(h, (data, []))[1].append(f"{crate}/{f.name}")

    body = []
    body.append("RS-2 remake 第三方元件聲明\n")
    body.append("本程式的原始碼授權見 LICENSE（RRSAL-1.0）。下列元件各依自己的授權散布。\n")
    body.append("1. SDL2 2.26.5（zlib 授權，全文在 SDL2-LICENSE.txt）\n"
                "   Linux 版：依官方原始碼 SDL2-2.26.5.tar.gz 在 Docker 內編譯，隨包附 libSDL2-2.0.so.0。\n"
                "   Windows 版：官方 mingw 開發包的 SDL2.dll。\n"
                "   macOS 版：官方 SDL2.framework。\n"
                "   原始碼：https://github.com/libsdl-org/SDL/releases/tag/release-2.26.5\n")
    body.append("2. Rust 標準函式庫（MIT 或 Apache-2.0，擇一），靜態連結進執行檔。\n")
    body.append(f"3. Rust crate（靜態連結進執行檔，共 {len(ext)} 個）\n" + "\n".join(lines) + "\n")
    body.append("4. 以上 crate 的授權檔全文\n")
    for h, (text, who) in texts.items():
        body.append("-" * 72 + "\n" + "\n".join(who) + "\n" + "-" * 72 + "\n" + text)
    (out / "THIRD-PARTY-NOTICES.txt").write_text("\n".join(body), encoding="utf-8")
    print(f"notices：{len(ext)} 個 crate，{len(texts)} 份不同的授權全文")


if __name__ == "__main__":
    main()
