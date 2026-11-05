#!/usr/bin/env python3
"""把三個平台的產物收進 dist-all/<版本>/full/，並寫總覽文件、SHA256SUMS 與 manifest.json。在 docker 內跑。

用法：pkg_finalize.py <版本>
讀：/stage（pack.sh 與 appimage.sh 的輸出）、/payload/payload.json、/notices、/LICENSE、環境變數 RS2_IMAGES（JSON，映像名稱與 id）。
寫：/dist。
"""
import hashlib
import json
import os
import shutil
import sys
from pathlib import Path

sys.path.insert(0, "/pkgtools")
import pkg_assemble as A  # noqa: E402


def sha(p: Path) -> str:
    return A.sha256(p)


def main():
    version = sys.argv[1]
    stage, dist = Path("/stage"), Path("/dist")

    arts = [
        ("linux-x86_64", "AppImage", next((stage / "linux").glob("*.AppImage")), stage / "linux/MANIFEST.json"),
        ("windows-x86_64", "zip", next((stage / "windows").glob("*.zip")), stage / "windows/MANIFEST.json"),
        ("macos-universal", "zip", next((stage / "macos").glob("*.zip")), stage / "macos/MANIFEST.json"),
    ]
    # 這個目錄可能還有別的腳本放進來的成品（例如 Android 的 APK），不整個清掉，只覆蓋自己管的檔案。
    managed = {s.name for _, _, s, _ in arts} | {"LICENSE", "README.txt", "THIRD-PARTY-NOTICES.txt", "SDL2-LICENSE.txt", "manifest.json", "SHA256SUMS"}
    for n in managed:
        (dist / n).unlink(missing_ok=True)
    entries = []
    for plat, kind, src, mpath in arts:
        dst = dist / src.name
        shutil.copyfile(src, dst)
        dst.chmod(0o755 if kind == "AppImage" else 0o644)
        pm = json.loads(mpath.read_text(encoding="utf-8"))
        entries.append({
            "platform": plat, "format": kind, "file": dst.name, "bytes": dst.stat().st_size, "sha256": sha(dst),
            "binary_sha256": pm["binary_sha256"], "sdl2": pm["sdl2"], "files_in_package": len(pm["files"]),
            **({"appimage_runtime": pm["appimage_runtime"]} if "appimage_runtime" in pm else {}),
            **({"signing": pm["signing"]} if "signing" in pm else {}),
        })

    lic = Path("/LICENSE").read_text(encoding="utf-8")
    (dist / "LICENSE").write_text(lic, encoding="utf-8")
    (dist / "README.txt").write_text(A.readme(version, None), encoding="utf-8")
    for n in ("THIRD-PARTY-NOTICES.txt", "SDL2-LICENSE.txt"):
        shutil.copyfile(Path("/notices") / n, dist / n)

    payload = json.loads(Path("/payload/payload.json").read_text())
    manifest = {
        "product": "RS-2 remake",
        "edition": "full（內含原版資料與 HD，只限本機保存，不得散布、不得上傳）",
        "version": version,
        "source_sha256": A.sha256(A.SOURCE_SHA) if A.SOURCE_SHA.exists() else None,
        "payload": {
            "original_files": sum(1 for k in payload["files"] if k.startswith("original/")),
            "hd_files": sum(1 for k in payload["files"] if k.startswith("hd/")),
            "hd_kept": payload["hd_kept"], "hd_dropped": payload["hd_dropped"],
            "catalog_sha256": hashlib.sha256((Path("/payload/payload.json")).read_bytes()).hexdigest(),
        },
        "toolchain_images": json.loads(os.environ.get("RS2_IMAGES", "{}")),
        "artifacts": entries,
        # 同目錄的其他成品（別的腳本放的）。只列檔名、大小與雜湊，內容由放它的腳本負責。
        "other_files": [{"file": p.name, "bytes": p.stat().st_size, "sha256": sha(p)}
                        for p in sorted(dist.iterdir()) if p.is_file() and p.name not in managed],
    }
    (dist / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=1, sort_keys=True) + "\n", encoding="utf-8")

    names = [e["file"] for e in entries] + ["LICENSE", "README.txt", "THIRD-PARTY-NOTICES.txt", "SDL2-LICENSE.txt", "manifest.json"]
    names += [o["file"] for o in manifest["other_files"]]
    (dist / "SHA256SUMS").write_text("".join(f"{sha(dist / n)}  {n}\n" for n in names), encoding="utf-8")
    for e in entries:
        print(f'{e["file"]}  {e["bytes"]}  {e["sha256"]}')


if __name__ == "__main__":
    main()
