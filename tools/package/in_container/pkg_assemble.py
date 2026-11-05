#!/usr/bin/env python3
"""組裝三個平台的發行包內容。在 docker 內跑（交叉工具鏈映像，有 python3 與 SDL2 的 Windows、macOS 開發包）。

用法：pkg_assemble.py <版本>
讀：/payload（原版與 HD）、/icons、/notices、/inputs、/bins、/LICENSE、/src/SOURCE.sha256、/opt/sdl2。
寫：/out/<平台>：
  linux/AppDir/              交給 mksquashfs（appimage.sh）
  windows/<名稱>.zip
  macos/<名稱>.zip
  <平台>/MANIFEST.json       每個檔案的 SHA-256，也放在包內

每個包都帶 LICENSE、README.txt、THIRD-PARTY-NOTICES.txt、SDL2-LICENSE.txt（rulebook/85：授權要出現在每個發行包）。
"""
import hashlib
import json
import os
import plistlib
import re
import shutil
import stat
import sys
import zipfile
from pathlib import Path

PAYLOAD = Path("/payload")
ICONS = Path("/icons")
NOTICES = Path("/notices")
INPUTS = Path("/inputs")
BINS = Path("/bins")
OUT = Path("/out")
LICENSE = Path("/LICENSE")
SOURCE_SHA = Path("/src/SOURCE.sha256")
SDL_WIN_DLL = Path("/opt/sdl2/windows/x86_64-w64-mingw32/bin/SDL2.dll")
SDL_MAC_FW = Path("/opt/sdl2/SDL2.framework")

APP = "rs2-remake"
MAC_MIN = "10.13"


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


class Entry:
    """包內的一個項目：檔案（來源檔或位元組）或符號連結。"""

    def __init__(self, path, mode=0o644, src=None, data=None, link=None):
        self.path, self.mode, self.src, self.data, self.link = path, mode, src, data, link

    def read(self) -> bytes:
        return self.data if self.data is not None else self.src.read_bytes()

    def digest(self):
        if self.link is not None:
            return None
        if self.src is not None:
            return sha256(self.src), self.src.stat().st_size
        return hashlib.sha256(self.data).hexdigest(), len(self.data)


def text(path, s, mode=0o644, bom=False, crlf=False):
    if crlf:
        s = s.replace("\r\n", "\n").replace("\n", "\r\n")
    return Entry(path, mode, data=(b"\xef\xbb\xbf" if bom else b"") + s.encode("utf-8"))


def file_entry(path, src, mode=0o644):
    return Entry(path, mode, src=Path(src))


def payload_entries(prefix):
    out = []
    for p in sorted(PAYLOAD.rglob("*")):
        if p.is_file() and p.name != "payload.json":
            out.append(file_entry(f"{prefix}{p.relative_to(PAYLOAD).as_posix()}", p))
    return out


def tree_entries(src_root: Path, prefix: str, exec_names=()):
    """複製一個目錄樹，保留符號連結。exec_names 內的檔名給 0755。"""
    out = []
    for root, dirs, names in os.walk(src_root):
        dirs.sort()
        # 指向目錄的符號連結會出現在 dirs，os.walk 不會走進去，要自己收成連結項目。
        for n in sorted(names + [d for d in dirs if (Path(root) / d).is_symlink()]):
            p = Path(root) / n
            rel = p.relative_to(src_root).as_posix()
            if p.is_symlink():
                out.append(Entry(prefix + rel, 0o755, link=os.readlink(p)))
            elif p.is_file():
                out.append(file_entry(prefix + rel, p, 0o755 if n in exec_names else 0o644))
    return out


START = {
        "linux": (
            "啟動\n"
            "  1. chmod +x RS-2-remake-*-linux-x86_64.AppImage\n"
            "  2. 直接執行 AppImage。沒有 FUSE 的系統加 --appimage-extract-and-run，或先 --appimage-extract 解開再執行 squashfs-root/AppRun。\n"
            "  需要 x86_64、glibc 2.34 以上，以及 X11、XWayland 或 Wayland 桌面。音效走 PulseAudio（含 PipeWire 相容層）或 ALSA；\n"
            "  開不起來時遊戲照跑但沒有聲音。SDL2 內建在包內，不必另外安裝。\n"
        ),
        "windows": (
            "啟動\n"
            "  解開 zip，雙擊 rs2-play.exe。資料在 data\\ 底下，整個資料夾不要拆開。\n"
            "  需要 Windows 10 以上、x86_64。執行時會同時開一個主控台視窗，顯示啟動與錯誤日誌，關掉遊戲視窗它就會結束。\n"
        ),
        "macos": (
            "啟動\n"
            "  解開 zip，雙擊 RS-2.app。這個 app 沒有簽章也沒有公證，第一次開啟要在 Finder 對它按右鍵選「打開」，\n"
            "  或在終端機執行：xattr -dr com.apple.quarantine RS-2.app\n"
            "  Intel 與 Apple Silicon 共用同一個 universal 執行檔。Intel Mac 需要 macOS 10.13 以上，Apple Silicon 需要 macOS 11 以上。\n"
            "  這一版是在 Linux 上交叉編譯，作者沒有 Mac 實機，沒有實際跑過。\n"
        ),
}
CFG = {
    "linux": "~/.config/rs2-remake/settings.txt（有設 XDG_CONFIG_HOME 就在那裡）",
    "windows": "%APPDATA%\\rs2-remake\\settings.txt",
    "macos": "~/Library/Application Support/rs2-remake/settings.txt",
}
LABEL = {"linux": "Linux x86_64（AppImage）", "windows": "Windows x86_64（zip）", "macos": "macOS universal（zip 內的 .app）"}


def readme(version, platform):
    """單一平台的 README；platform 傳 None 時是整個發行目錄用的總覽，三個平台的啟動說明與設定檔位置都列出。"""
    plats = [platform] if platform else ["linux", "windows", "macos"]
    if platform:
        start = START[platform]
        cfg = CFG[platform]
    else:
        start = "啟動\n" + "".join(f"\n【{LABEL[p]}】\n" + START[p].split("啟動\n", 1)[1] for p in plats)
        cfg = "".join(f"\n    {LABEL[p]}：{CFG[p]}" for p in plats)
    return f"""RS-2 remake {version}（完整版）

大宇資訊 1990 年 DOS 射擊遊戲《RS-2》的 Rust 重製。這個包內含原版資料與 HD 素材，解開就能玩。

只限本機保存
  本包含大宇資訊（Softstar）原版的資料檔、美術、音樂，以及依原版重繪的 HD 圖。這些素材的權利屬於各自的權利人。
  只限自己保存與試玩，不得散布、上傳、轉贈或商用。可公開的版本不含這些資料，需要自備原版。

{start}
操作鍵
  標題畫面按 C 開選單（難度與玩家數），其他鍵開始遊戲。
  方向鍵移動；Home、PgUp、End、PgDn 是斜向。右 Shift 開火；/ 砲座轉向；Esc 暫停；S 切換音效；Ctrl+Q 離開。
  雙人時玩家 2 用 T、B、F、H 移動，左 Shift 開火，Z 砲座轉向。
  F1 畫面補間；F2 幀計數；F3 原版圖與 HD；F4 視窗倍率；F5 玩家機預測與短按保留（關掉就是原版時序）；
  F10 隱藏工具列；F11 全螢幕。視窗上方的工具列可以用滑鼠點。
  手把（SDL GameController）：第一支是玩家 1，第二支是玩家 2。十字鍵與左搖桿移動，A 開火，B 砲座轉向，Start 暫停，Y 開選單。

設定
  設定檔：{cfg}
  程式所在的位置可能唯讀，所以設定不寫在那裡。
  命令列選項見 --help，例如 --no-hd 只用原版圖、--no-audio 關聲音、--scale N 視窗倍率。
  環境變數 RS2_DATA 可以改用別處的資料（資料根，底下有 original/ 與 hd/，或直接指向原版目錄）。

授權
  程式、工具與文件依 RRSAL-1.0（見 LICENSE），不涵蓋原版素材。商業用途請先洽談：wicanr2@gmail.com。
  第三方元件見 THIRD-PARTY-NOTICES.txt 與 SDL2-LICENSE.txt。
  來源：https://github.com/wicanr2/rs2-remake（私人儲存庫）
"""


def desktop_file():
    return f"""[Desktop Entry]
Type=Application
Name=RS-2 remake
Name[zh_TW]=RS-2 重製版
Comment=Rust remake of the 1990 DOS shooter RS-2
Exec=rs2-play
Icon={APP}
Terminal=false
Categories=Game;ArcadeGame;
"""


APPRUN = """#!/bin/sh
# AppImage 進入點。APPDIR 由 AppImage runtime 設定；直接執行解開的 AppDir 時自己算。
HERE="$(dirname "$(readlink -f "$0")")"
export APPDIR="${APPDIR:-$HERE}"
exec "$HERE/usr/bin/rs2-play" "$@"
"""


def docs(version, platform, notices_prefix, with_root=True):
    """授權與說明文件。回傳 (路徑, 項目) 清單。"""
    lic = LICENSE.read_text(encoding="utf-8")
    rd = readme(version, platform)
    out = []
    out.append(text(notices_prefix + "LICENSE", lic))
    out.append(text(notices_prefix + "THIRD-PARTY-NOTICES.txt", (NOTICES / "THIRD-PARTY-NOTICES.txt").read_text(encoding="utf-8")))
    out.append(text(notices_prefix + "SDL2-LICENSE.txt", (NOTICES / "SDL2-LICENSE.txt").read_text(encoding="utf-8")))
    return out, lic, rd


# ---------------------------------------------------------------------------
# 寫出：目錄與 zip
# ---------------------------------------------------------------------------

def write_dir(root: Path, entries):
    if root.exists():
        shutil.rmtree(root)
    for e in entries:
        p = root / e.path
        p.parent.mkdir(parents=True, exist_ok=True)
        if e.link is not None:
            os.symlink(e.link, p)
        elif e.src is not None:
            shutil.copyfile(e.src, p)
            p.chmod(e.mode)
        else:
            p.write_bytes(e.data)
            p.chmod(e.mode)
    # 目錄一律 755，沒有 setgid 之類。
    for d in [root, *[p for p in root.rglob("*") if p.is_dir() and not p.is_symlink()]]:
        d.chmod(0o755)


def zip_date(version):
    m = re.search(r"-(\d{4})(\d{2})(\d{2})$", version)
    return (int(m.group(1)), int(m.group(2)), int(m.group(3)), 0, 0, 0)


def write_zip(path: Path, entries, version):
    if path.exists():
        path.unlink()
    date = zip_date(version)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for e in sorted(entries, key=lambda x: x.path):
            zi = zipfile.ZipInfo(e.path, date_time=date)
            zi.create_system = 3
            if e.link is not None:
                zi.external_attr = (stat.S_IFLNK | 0o755) << 16
                zi.compress_type = zipfile.ZIP_STORED
                z.writestr(zi, e.link)
                continue
            zi.external_attr = (stat.S_IFREG | e.mode) << 16
            zi.compress_type = zipfile.ZIP_DEFLATED
            if e.src is not None:
                with e.src.open("rb") as f, z.open(zi, "w") as w:
                    shutil.copyfileobj(f, w, 1 << 20)
            else:
                z.writestr(zi, e.data, compress_type=zipfile.ZIP_DEFLATED, compresslevel=6)


def manifest(version, platform, entries, extra):
    files = {}
    for e in sorted(entries, key=lambda x: x.path):
        if e.link is not None:
            files[e.path] = {"symlink": e.link}
        else:
            h, n = e.digest()
            files[e.path] = {"sha256": h, "bytes": n}
    return {
        "product": "RS-2 remake",
        "edition": "full（內含原版資料與 HD，只限本機保存，不得散布）",
        "version": version,
        "platform": platform,
        "source_sha256": sha256(SOURCE_SHA),
        **extra,
        "files": files,
    }


def with_manifest(entries, version, platform, extra, prefix=""):
    m = manifest(version, platform, entries, extra)
    entries.append(text(prefix + "MANIFEST.json", json.dumps(m, ensure_ascii=False, indent=1, sort_keys=True) + "\n"))
    return m


# ---------------------------------------------------------------------------
# 平台
# ---------------------------------------------------------------------------

def build_linux(version):
    docs_e, lic, rd = docs(version, "linux", f"usr/share/doc/{APP}/")
    e = list(docs_e)
    e.append(text(f"usr/share/doc/{APP}/README.txt", rd))
    e.append(text("LICENSE", lic))
    e.append(text("README.txt", rd))
    e.append(file_entry("usr/bin/rs2-play", BINS / "linux/rs2-play", 0o755))
    e.append(file_entry("usr/lib/libSDL2-2.0.so.0", BINS / "linux-sdl/libSDL2-2.0.so.0.2600.5", 0o755))
    e += payload_entries("usr/share/rs2/")
    e.append(text("AppRun", APPRUN, 0o755))
    e.append(text(f"{APP}.desktop", desktop_file()))
    e.append(text(f"usr/share/applications/{APP}.desktop", desktop_file()))
    e.append(file_entry(f"{APP}.png", ICONS / "rs2-256.png"))
    e.append(Entry(".DirIcon", 0o755, link=f"{APP}.png"))
    e.append(file_entry(f"usr/share/icons/hicolor/256x256/apps/{APP}.png", ICONS / "rs2-256.png"))
    for p in sorted((INPUTS / "appimage-runtime/licenses").iterdir()):
        e.append(file_entry(f"usr/share/licenses/appimage-runtime/{p.name}", p))
    extra = {
        "binary_sha256": sha256(BINS / "linux/rs2-play"),
        "sdl2": {"version": "2.26.5", "build": "官方原始碼在 Debian 12 容器內編譯，後端以 dlopen 載入",
                 "tarball_sha256": sha256(INPUTS / "SDL2-2.26.5.tar.gz"),
                 "sha256": sha256(BINS / "linux-sdl/libSDL2-2.0.so.0.2600.5")},
        "appimage_runtime": {"commit": "75849dc", "sha256": sha256(INPUTS / "appimage-runtime/runtime-x86_64")},
    }
    m = with_manifest(e, version, "linux-x86_64", extra)
    write_dir(OUT / "linux/AppDir", e)
    (OUT / "linux/MANIFEST.json").write_text(json.dumps({k: v for k, v in m.items()}, ensure_ascii=False, indent=1, sort_keys=True) + "\n")
    print(f"linux：AppDir {len(e)} 項")


def build_windows(version):
    name = f"RS-2-remake-{version}-full-windows-x86_64"
    top = name + "/"
    docs_e, lic, rd = docs(version, "windows", top + "licenses/")
    e = list(docs_e)
    e.append(text(top + "LICENSE", lic, crlf=True, bom=True))
    e.append(text(top + "README.txt", rd, crlf=True, bom=True))
    exe = BINS / "windows/x86_64-pc-windows-gnu/release/rs2-play.exe"
    e.append(file_entry(top + "rs2-play.exe", exe, 0o755))
    e.append(file_entry(top + "SDL2.dll", SDL_WIN_DLL, 0o755))
    e += payload_entries(top + "data/")
    extra = {"binary_sha256": sha256(exe),
             "sdl2": {"version": "2.26.5", "build": "官方 mingw 開發包（SDL2-devel-2.26.5-mingw.tar.gz）的 SDL2.dll", "sha256": sha256(SDL_WIN_DLL)}}
    m = with_manifest(e, version, "windows-x86_64", extra, top)
    out = OUT / "windows" / (name + ".zip")
    out.parent.mkdir(parents=True, exist_ok=True)
    write_zip(out, e, version)
    (OUT / "windows/MANIFEST.json").write_text(json.dumps(m, ensure_ascii=False, indent=1, sort_keys=True) + "\n")
    print(f"windows：{out.name}，{len(e)} 項，{out.stat().st_size} 位元組")


def build_macos(version):
    name = f"RS-2-remake-{version}-full-macos-universal"
    top = name + "/"
    app = top + "RS-2.app/Contents/"
    docs_root, lic, rd = docs(version, "macos", top + "licenses/")
    docs_app, _, _ = docs(version, "macos", app + "Resources/")
    e = list(docs_root) + list(docs_app)
    e.append(text(top + "LICENSE", lic))
    e.append(text(top + "README.txt", rd))
    e.append(text(app + "Resources/README.txt", rd))
    binary = BINS / "macos/universal/rs2-play"
    e.append(file_entry(app + "MacOS/rs2-play", binary, 0o755))
    e += tree_entries(SDL_MAC_FW, app + "Frameworks/SDL2.framework/", exec_names={"SDL2"})
    e += payload_entries(app + "Resources/")
    e.append(file_entry(app + "Resources/rs2.icns", ICONS / "rs2.icns"))
    ver_num = re.fullmatch(r"v\.(\d+\.\d+\.\d+)-(\d{8})", version)
    plist = {
        "CFBundleName": "RS-2",
        "CFBundleDisplayName": "RS-2 remake",
        "CFBundleIdentifier": "local.wicanr2.rs2-remake",
        "CFBundleExecutable": "rs2-play",
        "CFBundleIconFile": "rs2",
        "CFBundlePackageType": "APPL",
        "CFBundleSignature": "????",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleShortVersionString": ver_num.group(1),
        "CFBundleVersion": ver_num.group(2),
        "CFBundleGetInfoString": version,
        "LSMinimumSystemVersion": MAC_MIN,
        "LSApplicationCategoryType": "public.app-category.arcade-games",
        "NSHighResolutionCapable": True,
        "NSHumanReadableCopyright": "Copyright (c) 2026 Wang Chun-Yu (wicanr2). RRSAL-1.0",
    }
    e.append(Entry(app + "Info.plist", 0o644, data=plistlib.dumps(plist, fmt=plistlib.FMT_XML)))
    e.append(text(app + "PkgInfo", "APPL????"))
    extra = {"binary_sha256": sha256(binary),
             "sdl2": {"version": "2.26.5", "build": "官方 SDL2.framework（SDL2-2.26.5.dmg），原樣放進 Contents/Frameworks，不改動以保留它的簽章封印",
                      "sha256": sha256(SDL_MAC_FW / "Versions/A/SDL2")},
             "signing": "未簽 bundle、未公證；rs2-play 的 arm64 slice 帶連結器的 ad-hoc 簽章（LC_CODE_SIGNATURE）"}
    m = with_manifest(e, version, "macos-universal", extra, top)
    out = OUT / "macos" / (name + ".zip")
    out.parent.mkdir(parents=True, exist_ok=True)
    write_zip(out, e, version)
    (OUT / "macos/MANIFEST.json").write_text(json.dumps(m, ensure_ascii=False, indent=1, sort_keys=True) + "\n")
    print(f"macos：{out.name}，{len(e)} 項，{out.stat().st_size} 位元組")


def main():
    version = sys.argv[1]
    if not re.fullmatch(r"v\.\d+\.\d+\.\d+-\d{8}", version):
        sys.exit(f"版本格式不對：{version}")
    OUT.mkdir(parents=True, exist_ok=True)
    build_linux(version)
    build_windows(version)
    build_macos(version)


if __name__ == "__main__":
    main()
