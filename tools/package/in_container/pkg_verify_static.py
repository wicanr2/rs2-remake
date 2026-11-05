#!/usr/bin/env python3
"""靜態驗收：直接讀 dist-all/<版本>/full 裡的成品，不解到磁碟。在 docker 內跑，離線。

用法：pkg_verify_static.py <版本>
讀：/dist（成品，唯讀）、/src（原始碼快照，取 EXPECTED）。

驗什麼：
  共通  SHA256SUMS、zip 的 CRC、包內每個檔案對 MANIFEST.json 的 SHA-256、授權檔、沒有不該帶的檔案、
        原版 26 檔的雜湊對 rs2-formats 的 EXPECTED、HD 有 base-palette.tsv 與 mono/。
  Windows  PE 為 x86_64、主控台子系統、相依 DLL 只有系統與同目錄的 SDL2.dll、版本資源有版本字串。
  macOS    fat 檔恰好 x86_64 加 arm64、相依路徑只有 @rpath 的 SDL2.framework 與系統函式庫、rpath 正確、
           最低系統版本、Info.plist、符號連結、arm64 的 ad-hoc 簽章 CodeDirectory 逐頁雜湊對得上。
  AppImage  ELF 加 type2 魔數、squashfs 緊接在 runtime 後面。
"""
import hashlib
import json
import plistlib
import re
import stat
import struct
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

DIST = Path("/dist")
SRC = Path("/src")
FAILS = 0


def ok(msg):
    print("PASS", msg)


def bad(msg):
    global FAILS
    FAILS += 1
    print("FAIL", msg)


def check(cond, msg):
    (ok if cond else bad)(msg)
    return cond


def sha(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


# ---------------------------------------------------------------------------
# Mach-O
# ---------------------------------------------------------------------------

CPU = {0x01000007: "x86_64", 0x0100000C: "arm64"}
LC_LOAD_DYLIB, LC_ID_DYLIB, LC_LOAD_WEAK_DYLIB, LC_REEXPORT_DYLIB = 0xC, 0xD, 0x80000018, 0x8000001F
LC_RPATH, LC_CODE_SIGNATURE, LC_BUILD_VERSION, LC_VERSION_MIN_MACOSX = 0x8000001C, 0x1D, 0x32, 0x24


def slices(data: bytes):
    magic = struct.unpack(">I", data[:4])[0]
    if magic == 0xCAFEBABE:
        n = struct.unpack(">I", data[4:8])[0]
        out = []
        for i in range(n):
            cpu, _sub, off, size, _al = struct.unpack(">iiIII", data[8 + 20 * i:28 + 20 * i])
            out.append((CPU.get(cpu, hex(cpu)), off, size))
        return out
    if struct.unpack("<I", data[:4])[0] == 0xFEEDFACF:
        cpu = struct.unpack("<i", data[4:8])[0]
        return [(CPU.get(cpu, hex(cpu)), 0, len(data))]
    raise ValueError("不是 Mach-O")


def macho_info(data: bytes, off: int):
    magic, _cpu, _sub, _ft, ncmds, _sz, _fl, _res = struct.unpack("<IiiIIIII", data[off:off + 32])
    assert magic == 0xFEEDFACF, hex(magic)
    p = off + 32
    info = {"dylibs": [], "rpaths": [], "minos": None, "sig": None, "id": None}
    for _ in range(ncmds):
        cmd, size = struct.unpack("<II", data[p:p + 8])
        if cmd in (LC_LOAD_DYLIB, LC_LOAD_WEAK_DYLIB, LC_REEXPORT_DYLIB, LC_ID_DYLIB):
            noff = struct.unpack("<I", data[p + 8:p + 12])[0]
            name = data[p + noff:p + size].split(b"\0")[0].decode()
            if cmd == LC_ID_DYLIB:
                info["id"] = name
            else:
                info["dylibs"].append(name)
        elif cmd == LC_RPATH:
            noff = struct.unpack("<I", data[p + 8:p + 12])[0]
            info["rpaths"].append(data[p + noff:p + size].split(b"\0")[0].decode())
        elif cmd == LC_BUILD_VERSION:
            _plat, minos, _sdk, _nt = struct.unpack("<IIII", data[p + 8:p + 24])
            info["minos"] = (minos >> 16, (minos >> 8) & 0xFF)
        elif cmd == LC_VERSION_MIN_MACOSX:
            ver, _sdk = struct.unpack("<II", data[p + 8:p + 16])
            info["minos"] = (ver >> 16, (ver >> 8) & 0xFF)
        elif cmd == LC_CODE_SIGNATURE:
            info["sig"] = struct.unpack("<II", data[p + 8:p + 16])
        p += size
    return info


def verify_code_directory(data: bytes, off: int, size: int, sig):
    """逐頁驗 CodeDirectory 的雜湊。回傳 (成功, 說明)。"""
    dataoff, datasize = sig
    blob = data[off + dataoff:off + dataoff + datasize]
    magic, _length, count = struct.unpack(">III", blob[:12])
    if magic != 0xFADE0CC0:
        return False, "不是 SuperBlob"
    cd = None
    for i in range(count):
        typ, boff = struct.unpack(">II", blob[12 + 8 * i:20 + 8 * i])
        if typ == 0:
            cd = blob[boff:]
            break
    if cd is None or struct.unpack(">I", cd[:4])[0] != 0xFADE0C02:
        return False, "沒有 CodeDirectory"
    (_m, length, _ver, flags, hash_off, ident_off, nspecial, ncode, limit) = struct.unpack(">IIIIIIIII", cd[:36])
    hsize, htype, _plat, psz = struct.unpack(">BBBB", cd[36:40])
    algo = {1: hashlib.sha1, 2: hashlib.sha256, 3: hashlib.sha256}.get(htype)
    if algo is None:
        return False, f"不認得的雜湊類型 {htype}"
    page = 1 << psz if psz else limit
    ident = cd[ident_off:].split(b"\0")[0].decode(errors="replace")
    body = data[off:off + limit]
    pages = (limit + page - 1) // page if psz else 1
    if pages != ncode:
        return False, f"頁數 {pages} 與 nCodeSlots {ncode} 不符"
    for i in range(ncode):
        chunk = body[i * page:min((i + 1) * page, limit)]
        want = cd[hash_off + i * hsize:hash_off + (i + 1) * hsize]
        if algo(chunk).digest()[:hsize] != want:
            return False, f"第 {i} 頁雜湊不符"
    adhoc = bool(flags & 0x2)
    return True, f"識別字 {ident}，{ncode} 頁 {'SHA-256' if htype in (2, 3) else 'SHA-1'} 全部相符，{'ad-hoc' if adhoc else '非 ad-hoc'}"


ALLOWED_DYLIB = re.compile(r"^(@rpath/SDL2\.framework/Versions/A/SDL2|/usr/lib/[A-Za-z0-9_.+-]+\.dylib|/System/Library/Frameworks/[A-Za-z0-9_.+/-]+)$")


def verify_macho(label, data, want_archs, rpath_expected, min_allowed):
    sl = slices(data)
    archs = sorted(a for a, _, _ in sl)
    check(archs == sorted(want_archs), f"{label}：架構 {archs}")
    for arch, off, size in sl:
        info = macho_info(data, off)
        bad_libs = [d for d in info["dylibs"] if not ALLOWED_DYLIB.match(d)]
        check(not bad_libs, f"{label}[{arch}]：相依 {len(info['dylibs'])} 個，沒有開發機路徑" + (f"（異常：{bad_libs}）" if bad_libs else ""))
        if rpath_expected is not None:
            check(info["rpaths"] == rpath_expected, f"{label}[{arch}]：rpath {info['rpaths']}")
        mo = info["minos"]
        if mo and min_allowed.get(arch):
            check(mo <= min_allowed[arch], f"{label}[{arch}]：最低系統版本 {mo[0]}.{mo[1]}（上限 {min_allowed[arch][0]}.{min_allowed[arch][1]}）")
        if arch == "arm64":
            if check(info["sig"] is not None, f"{label}[arm64]：有 LC_CODE_SIGNATURE（Apple Silicon 必要）"):
                good, msg = verify_code_directory(data, off, size, info["sig"])
                check(good, f"{label}[arm64]：簽章 {msg}")
        elif info["sig"]:
            good, msg = verify_code_directory(data, off, size, info["sig"])
            check(good, f"{label}[x86_64]：簽章 {msg}")
    return sl


# ---------------------------------------------------------------------------
# 共通
# ---------------------------------------------------------------------------

def expected_original():
    text = (SRC / "crates/rs2-formats/src/manifest.rs").read_text(encoding="utf-8")
    body = text.split("pub const EXPECTED", 1)[1].split("];", 1)[0]
    return dict(re.findall(r'\("([^"]+)",\s*"([0-9a-f]{64})"\)', body))


FORBIDDEN = re.compile(r"(?:^|/)(SEN\.EXE|SEN\.PAS|RS2\.BAT|RS2\.JOY|JOYMOUSE\.CFG|FILE_ID\.DIZ|ASMDRV\.EXE|JGAME\.EXE|MYB\.BDT)$|/overview/|/_final/|hd-manifest\.json|hd-postfix\.json|hd-variants-manifest\.json|/maps/|/MAP/objects/", re.I)


def content_checks(label, files: dict, manifest: dict, prefix: str, data_root: str, doc_roots):
    """files：包內路徑 -> bytes 或 ('link', target)。"""
    mf = manifest["files"]
    regular = {k: v for k, v in files.items() if not isinstance(v, tuple)}
    links = {k: v[1] for k, v in files.items() if isinstance(v, tuple)}
    want = {k: v for k, v in mf.items() if "sha256" in v}
    want_links = {k: v["symlink"] for k, v in mf.items() if "symlink" in v}
    check(links == want_links, f"{label}：符號連結 {len(links)} 個與 MANIFEST 相同")
    miss = [k for k in want if k not in regular and k != prefix + "MANIFEST.json"]
    extra = [k for k in regular if k not in want and k != prefix + "MANIFEST.json"]
    check(not miss and not extra, f"{label}：檔案清單與 MANIFEST 相同（{len(want)} 檔）" + (f" 缺 {miss[:3]} 多 {extra[:3]}" if miss or extra else ""))
    wrong = [k for k, v in want.items() if k in regular and sha(regular[k]) != v["sha256"]]
    check(not wrong, f"{label}：每個檔案的 SHA-256 對得上 MANIFEST" + (f"（不符 {wrong[:3]}）" if wrong else ""))
    forb = [k for k in regular if FORBIDDEN.search("/" + k)]
    check(not forb, f"{label}：沒有破解組檔案或執行時用不到的 HD 檔案" + (f"（{forb[:3]}）" if forb else ""))
    for root in doc_roots:
        for n in ("LICENSE", "THIRD-PARTY-NOTICES.txt", "SDL2-LICENSE.txt"):
            check(prefix + root + n in regular, f"{label}：{root}{n} 存在")
    check(prefix + "LICENSE" in regular and prefix + "README.txt" in regular, f"{label}：發行包根目錄有 LICENSE 與 README.txt")
    check(all(b"RRSAL-1.0" in regular[prefix + r + "LICENSE"] for r in doc_roots + [""]), f"{label}：各處的 LICENSE 都是 RRSAL-1.0")
    # 原版 26 檔
    exp = expected_original()
    got = {k.rsplit("/", 1)[1]: sha(v) for k, v in regular.items() if k.startswith(prefix + data_root + "original/")}
    check(got == exp, f"{label}：原版資料 {len(got)} 檔的雜湊與 rs2-formats EXPECTED 完全相同")
    # 資料整份對 payload.json（組裝時驗過原版雜湊、分類過 HD）
    pay = json.loads(Path("/payload/payload.json").read_text())["files"]
    dr = prefix + data_root
    data = {k[len(dr):]: {"sha256": sha(v), "bytes": len(v)} for k, v in regular.items() if k.startswith((dr + "original/", dr + "hd/"))}
    check(data == pay, f"{label}：資料 {len(data)} 檔的路徑、SHA-256、大小與 payload.json 完全相同")
    check((dr + "hd/base-palette.tsv") in regular, f"{label}：HD 有 base-palette.tsv")
    mono = [k for k in regular if k.startswith(dr + "hd/mono/")]
    check(len(mono) > 0, f"{label}：HD 保留 mono/（{len(mono)} 張）")
    return regular


def read_zip(path: Path):
    files = {}
    with zipfile.ZipFile(path) as z:
        check(z.testzip() is None, f"{path.name}：zip CRC 全部通過（{len(z.infolist())} 項）")
        unsafe = [zi.filename for zi in z.infolist() if zi.filename.startswith("/") or ".." in zi.filename.split("/")]
        check(not unsafe, f"{path.name}：沒有絕對路徑或 .. 的項目" + (f"（{unsafe[:3]}）" if unsafe else ""))
        for zi in z.infolist():
            name = zi.filename
            mode = (zi.external_attr >> 16) & 0xFFFF
            if stat.S_ISLNK(mode):
                files[name] = ("link", z.read(zi).decode())
            elif not name.endswith("/"):
                files[name] = z.read(zi)
        modes = {zi.filename: (zi.external_attr >> 16) & 0o777 for zi in z.infolist()}
    return files, modes


def main():
    version = sys.argv[1]
    print("== SHA256SUMS")
    r = subprocess.run(["sha256sum", "-c", "SHA256SUMS"], cwd=DIST, capture_output=True, text=True)
    check(r.returncode == 0, "dist 內所有檔案的 SHA-256 與 SHA256SUMS 相符\n" + r.stdout.strip())

    # --- Windows
    print("== Windows zip")
    wz = DIST / f"RS-2-remake-{version}-full-windows-x86_64.zip"
    files, modes = read_zip(wz)
    top = f"RS-2-remake-{version}-full-windows-x86_64/"
    manifest = json.loads(files[top + "MANIFEST.json"])
    content_checks("Windows", files, manifest, top, "data/", ["licenses/"])
    exe = files[top + "rs2-play.exe"]
    pe = struct.unpack("<I", exe[0x3C:0x40])[0]
    machine = struct.unpack("<H", exe[pe + 4:pe + 6])[0]
    subsystem = struct.unpack("<H", exe[pe + 24 + 68:pe + 24 + 70])[0]
    check(exe[:2] == b"MZ" and exe[pe:pe + 4] == b"PE\0\0" and machine == 0x8664, "Windows：rs2-play.exe 是 x86_64 PE")
    check(subsystem == 3, f"Windows：子系統 {subsystem}（3 = 主控台，啟動錯誤看得到）")
    dll = files[top + "SDL2.dll"]
    pe2 = struct.unpack("<I", dll[0x3C:0x40])[0]
    check(dll[:2] == b"MZ" and struct.unpack("<H", dll[pe2 + 4:pe2 + 6])[0] == 0x8664 and b"SDL_CreateWindow" in dll, "Windows：SDL2.dll 是 x86_64 PE，匯出 SDL_CreateWindow")
    check(version.encode("utf-16le") in exe, f"Windows：exe 的版本資源含 {version}")
    check("RS-2 remake".encode("utf-16le") in exe, "Windows：exe 的版本資源含產品名稱")
    with tempfile.TemporaryDirectory() as t:
        (Path(t) / "rs2-play.exe").write_bytes(exe)
        out = subprocess.run(["x86_64-w64-mingw32-objdump", "-p", str(Path(t) / "rs2-play.exe")], capture_output=True, text=True).stdout
    dlls = sorted(set(re.findall(r"DLL Name: (\S+)", out)))
    system = {"KERNEL32.dll", "USERENV.dll", "WS2_32.dll", "bcryptprimitives.dll", "msvcrt.dll", "ntdll.dll", "SDL2.dll"}
    odd = [d for d in dlls if d not in system and not d.lower().startswith("api-ms-win-")]
    check(not odd, f"Windows：相依 DLL {dlls}，除了同目錄的 SDL2.dll 都是系統 DLL" + (f"（異常 {odd}）" if odd else ""))
    check(any(m & 0o111 for n, m in modes.items() if n.endswith("rs2-play.exe")), "Windows：zip 內 exe 有執行位元")

    # --- macOS
    print("== macOS zip")
    mz = DIST / f"RS-2-remake-{version}-full-macos-universal.zip"
    files, modes = read_zip(mz)
    top = f"RS-2-remake-{version}-full-macos-universal/"
    app = top + "RS-2.app/Contents/"
    manifest = json.loads(files[top + "MANIFEST.json"])
    content_checks("macOS", files, manifest, top, "RS-2.app/Contents/Resources/", ["licenses/", "RS-2.app/Contents/Resources/"])
    check(modes[app + "MacOS/rs2-play"] & 0o111 == 0o111, "macOS：Contents/MacOS/rs2-play 有執行位元")
    check(modes[app + "Frameworks/SDL2.framework/Versions/A/SDL2"] & 0o111 == 0o111, "macOS：SDL2 有執行位元")
    plist = plistlib.loads(files[app + "Info.plist"])
    check(plist["CFBundleExecutable"] == "rs2-play" and (app + "MacOS/" + plist["CFBundleExecutable"]) in files, "macOS：Info.plist 的 CFBundleExecutable 指到存在的檔案")
    check((app + "Resources/" + plist["CFBundleIconFile"] + ".icns") in files, "macOS：圖示檔存在")
    check(files[app + "Frameworks/SDL2.framework/Versions/Current"] == ("link", "A")
          and files[app + "Frameworks/SDL2.framework/SDL2"] == ("link", "Versions/Current/SDL2"), "macOS：SDL2.framework 的符號連結保留（Versions/Current、SDL2）")
    print("   Info.plist：", {k: plist[k] for k in ("CFBundleIdentifier", "CFBundleShortVersionString", "LSMinimumSystemVersion")})
    main_bin = files[app + "MacOS/rs2-play"]
    verify_macho("macOS rs2-play", main_bin, ["x86_64", "arm64"], ["@executable_path/../Frameworks"], {"x86_64": (10, 13), "arm64": (11, 0)})
    fw = files[app + "Frameworks/SDL2.framework/Versions/A/SDL2"]
    sl = verify_macho("macOS SDL2.framework", fw, ["x86_64", "arm64"], None, {"x86_64": (10, 13), "arm64": (11, 0)})
    ids = {a: macho_info(fw, off)["id"] for a, off, _ in sl}
    check(set(ids.values()) == {"@rpath/SDL2.framework/Versions/A/SDL2"}, f"macOS：SDL2 的 install name {set(ids.values())} 與 rs2-play 的連結路徑一致")
    check(("link", "Versions/Current/Resources") == files.get(app + "Frameworks/SDL2.framework/Resources"), "macOS：framework 的 Resources 連結保留")

    # --- AppImage
    print("== AppImage")
    ai = (DIST / f"RS-2-remake-{version}-full-linux-x86_64.AppImage").read_bytes()
    runtime = (Path("/inputs/appimage-runtime/runtime-x86_64")).read_bytes()
    check(ai[:4] == b"\x7fELF" and ai[8:11] == b"AI\x02", "AppImage：ELF 且帶 type2 魔數 AI\\x02")
    check(ai[:len(runtime)] == runtime, f"AppImage：開頭是 runtime 75849dc（{len(runtime)} 位元組，SHA-256 {sha(runtime)[:16]}…）")
    check(ai[len(runtime):len(runtime) + 4] == b"hsqs", "AppImage：squashfs 緊接在 runtime 後面")
    check(os_exec(DIST / f"RS-2-remake-{version}-full-linux-x86_64.AppImage"), "AppImage：有執行位元")

    # --- 三包資料一致
    print("== 三個平台的資料一致")
    def data_map(m, root):
        return {k.split(root, 1)[1]: v["sha256"] for k, v in m["files"].items() if root in k and "sha256" in v}
    win = json.loads(zipfile.ZipFile(wz).read(f"RS-2-remake-{version}-full-windows-x86_64/MANIFEST.json"))
    mac = json.loads(zipfile.ZipFile(mz).read(f"RS-2-remake-{version}-full-macos-universal/MANIFEST.json"))
    w = data_map(win, "/data/")
    mm = {k: v for k, v in data_map(mac, "/Resources/").items() if k.startswith(("original/", "hd/"))}
    check(w == mm and len(w) > 3000, f"Windows 與 macOS 的資料檔（{len(w)} 個）完全相同")

    print(f"== 結果：FAIL {FAILS} 項")
    sys.exit(1 if FAILS else 0)


def os_exec(p: Path) -> bool:
    import os
    return bool(os.stat(p).st_mode & 0o111)


if __name__ == "__main__":
    main()
