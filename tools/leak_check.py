#!/usr/bin/env python3
"""推送前的外洩閘門：確認要進版控的檔案沒有夾帶原版素材或密碼表。

用法（在容器內）：
  leak_check.py <repo> <original/rs-2> <S.unpacked.bin> < 檔案清單（git ls-files，每行一個）
  leak_check.py --selftest <original/rs-2> <S.unpacked.bin>
有命中就以 1 結束。例外列在 tools/leak_allow.txt，每行「完整字串<TAB>原因」。只放行那段字串本身，檔案其他內容照查，
密碼表不受例外影響。

README 對照圖與指定的手冊閱讀圖是放行的圖片，規則見 README_IMAGE、MANUAL_IMAGE 與 png_outside_idat。
"""
import hashlib
import os
import re
import struct
import sys
import zlib

DENY_EXT = {".png", ".jpg", ".rar", ".exe", ".zip", ".mid", ".vgm", ".wav", ".bin", ".i64", ".go", ".gob", ".got", ".bdb", ".bdh", ".bdt",
            ".bd1", ".bd2", ".bd3", ".bd4", ".mix", ".lck", ".scn", ".scr", ".joy", ".cfg", ".diz", ".bat", ".pas"}
# 工具本身的 Go 原始碼（dosgolem patch 內）副檔名是 .patch，不受影響。原版的 .GO 與 Go 原始碼同副檔名，以大小寫與所在目錄區分。
SOURCE_GO_OK = re.compile(r"^(tools/|crates/).*\.go$")
MAX_SIZE = 300 * 1024
# 唯一放行的圖片：README 的對照圖（AGENTS.md 硬規則 4 的例外）。路徑、PNG 結構、大小都要符合。
# 整個檔案照樣掃原版位元組。密碼表只比對 IDAT 以外的部分：表的比對窗是低熵的短字串，
# 在壓縮串流裡會碰巧對上，實測 26 MB 的對照圖有 9 張命中，全部落在 IDAT。
# IDAT 以外只允許必要區塊，所以文字區塊與檔尾都夾帶不了東西。
README_IMAGE = re.compile(r"^docs/images/(?:game|overview)/[A-Za-z0-9-]+\.png$")
# 使用者 2026-11-11 授權手冊含截圖進私人 repo。第 5 張只保留左頁，密碼表不輸出。
# 固定檔名由 tools/manual_images.py 產生，來源索引在 docs/manual.md。
MANUAL_IMAGE = re.compile(r"^docs/images/manual/(?:packaging|cover-spread|contents-story|setup-controls|player2-weapons|disk-[abc])\.png$")
PNG_SIG = b"\x89PNG\r\n\x1a\n"
MAX_IMAGE = 8 * 1024 * 1024
PNG_CHUNKS_OK = {b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND"}
# 介面字型（規格 005）：tools/ui_font_bake.py 從 Noto Sans CJK TC（OFL）烘出來的點陣，不含原版素材。
# 只放行這個路徑加確切的雜湊，內容一變就重新擋下。重烘之後要更新雜湊並重跑 --selftest。原版位元組與密碼表照樣掃。
FONT_ASSET = "crates/rs2-play/assets/ui-font.bin"
FONT_SHA256 = "7d18cea7ce8a3422c16dd458ef177bdeaf102bf0aad3a3bd4a58287c14c1afc2"
WIN = 32
HEXWIN = 24
HEXRUN = re.compile(rb"(?:\b(?:0x)?[0-9A-Fa-f]{2}h?\b[ ,\t]*){%d,}" % HEXWIN)
DS = 0xCBC0
PW_OFF = DS + 0x8BD
PW_LEN = 26 * 26


def png_outside_idat(data):
    """檢查 PNG 結構。回傳 (錯誤訊息或 None, IDAT 以外的區塊內容)。"""
    o = len(PNG_SIG)
    rest = bytearray()
    while o + 12 <= len(data):
        n = struct.unpack(">I", data[o:o + 4])[0]
        t = data[o + 4:o + 8]
        if o + 12 + n > len(data):
            return "區塊超出檔尾", b""
        if zlib.crc32(data[o + 4:o + 8 + n]) != struct.unpack(">I", data[o + 8 + n:o + 12 + n])[0]:
            return "CRC 不符", b""
        if t not in PNG_CHUNKS_OK:
            return "不允許的區塊 " + t.decode("latin-1"), b""
        if t != b"IDAT":
            rest += data[o + 8:o + 8 + n]
        o += 12 + n
        if t == b"IEND":
            return (None, bytes(rest)) if o == len(data) else ("IEND 之後還有資料", b"")
    return "缺 IEND", b""


def load_windows(paths):
    win = set()
    hexwin = set()
    for p in paths:
        d = open(p, "rb").read()
        for i in range(0, max(0, len(d) - WIN + 1)):
            w = d[i:i + WIN]
            if len(set(w)) > 4:  # 全 0 或單調填充不算內容
                win.add(w)
        for i in range(0, max(0, len(d) - HEXWIN + 1)):
            w = d[i:i + HEXWIN]
            if len(set(w)) > 4:
                hexwin.add(w)
    return win, hexwin


def scan(name, data, win, hexwin, pw_raw, pw_letters, allow):
    """allow 是可放行的完整字串（bytes）集合。"""
    hits = []
    ext = os.path.splitext(name)[1].lower()
    readme_image = bool(README_IMAGE.fullmatch(name) or MANUAL_IMAGE.fullmatch(name)) and data.startswith(PNG_SIG)
    font = name == FONT_ASSET and hashlib.sha256(data).hexdigest() == FONT_SHA256
    if ext in DENY_EXT and not (ext == ".go" and SOURCE_GO_OK.match(name)) and not readme_image and not font:
        hits.append(("副檔名", ext))
    if len(data) > (MAX_IMAGE if readme_image else MAX_SIZE) and not font:
        hits.append(("過大", len(data)))
    pw_data = data
    if readme_image:
        err, pw_data = png_outside_idat(data)
        if err:
            hits.append(("PNG 結構", err))
    # 密碼表無論如何都不放行
    for i in range(0, len(pw_data) - 11):
        if pw_data[i:i + 12] in pw_raw:
            hits.append(("密碼表原始位元組", i))
            break
    for w in pw_letters:
        if w in pw_data:
            hits.append(("密碼表字母形式", len(w)))
            break
    for m in HEXRUN.finditer(data):
        vals = bytes(int(x.replace(b"0x", b"").rstrip(b"hH"), 16) for x in re.findall(rb"(?:0x)?[0-9A-Fa-f]{2}h?\b", m.group()))
        for i in range(0, len(vals) - 11):
            if vals[i:i + 12] in pw_raw:
                hits.append(("密碼表十六進位", m.start()))
                break
        for i in range(0, len(vals) - HEXWIN + 1):
            if vals[i:i + HEXWIN] in hexwin:
                hits.append(("原版位元組十六進位", m.start()))
                break
    rest = data
    for a in allow:
        rest = rest.replace(a, b" ")
    for i in range(0, max(0, len(rest) - WIN + 1)):
        if rest[i:i + WIN] in win:
            hits.append(("原版位元組", i))
            break
    return hits


def setup(orig, unp):
    paths = [os.path.join(orig, f) for f in sorted(os.listdir(orig))
             if not f.upper().endswith((".PAS", ".DIZ", ".BAT", ".JOY", ".CFG"))]
    paths.append(unp)
    win, hexwin = load_windows(paths)
    exe = open(unp, "rb").read()
    table = exe[PW_OFF:PW_OFF + PW_LEN]
    pw_raw = {table[i:i + 12] for i in range(0, len(table) - 11)}
    pw_letters = {bytes((v + 0x41) & 0xFF for v in table[i:i + 16]) for i in range(0, len(table) - 15)}
    return win, hexwin, pw_raw, pw_letters, exe, table


def selftest(orig, unp):
    win, hexwin, pw_raw, pw_letters, exe, table = setup(orig, unp)
    cases = []
    clean = "這是乾淨的說明文件。cmp word ptr ds:7F0h, 0 位址 12F03h，RLE 控制碼 80h。\n".encode()
    cases.append(("clean.md", clean, False))
    cases.append(("a.md", clean + exe[0x2000:0x2040], True))
    cases.append(("b.md", clean + exe[0x2000:0x2030].hex(" ").encode(), True))
    cases.append(("c.md", clean + ", ".join(f"0x{v:02X}" for v in exe[0x3000:0x3030]).encode(), True))
    cases.append(("d.md", clean + table[10:40].hex(" ").encode(), True))
    cases.append(("e.md", clean + bytes((v + 0x41) & 0xFF for v in table[5:40]), True))
    cases.append(("f.png", b"x", True))
    cases.append(("g.md", b"x" * (MAX_SIZE + 1), True))
    cases.append(("tools/x.go", b"package x\n", False))
    orig_file = sorted(f for f in os.listdir(orig) if f.upper().endswith(".GO"))[0]
    cases.append(("h.md", clean + open(os.path.join(orig, orig_file), "rb").read()[400:460], True))
    # README 對照圖的例外：只放行這個路徑下結構正確的 PNG，原版位元組照樣掃。
    def png(*chunks):
        out = PNG_SIG
        for t, d in chunks:
            out += struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
        return out

    def ok_png(idat=b"x", extra=()):
        return png((b"IHDR", struct.pack(">IIBBBBB", 960, 600, 8, 2, 0, 0, 0)), *extra, (b"IDAT", idat), (b"IEND", b""))

    good = ok_png()
    secret = table[10:40]
    cases.append(("docs/images/game/a-hd.png", good, False))
    cases.append(("docs/images/manual/cover-spread.png", good, False))
    cases.append(("docs/images/manual/player2-weapons.png", good, False))
    cases.append(("docs/images/manual/disk-c.png", good, False))
    cases.append(("docs/images/manual/password.png", good, True))
    cases.append(("docs/images/manual/cover-spread.jpg", good, True))
    cases.append(("docs/images/manual/cover-spread.png", good + secret, True))
    cases.append(("docs/images/manual/cover-spread.png", ok_png(extra=[(b"tEXt", secret)]), True))
    cases.append(("manual.rar", b"archive", True))
    cases.append(("docs/images/overview/SHT1.png", ok_png(b"x" * (MAX_SIZE + 1)), False))
    # 壓縮串流裡的低熵短字串不算命中，這是例外的代價，條件寫在 README_IMAGE 上方。
    cases.append(("docs/images/game/a.png", ok_png(secret), False))
    cases.append(("docs/images/game/a-hd.png", b"not a png", True))
    cases.append(("docs/images/game/a.png", PNG_SIG + b"x", True))
    cases.append(("docs/images/other/a.png", good, True))
    cases.append(("docs/images/game/../../a.png", good, True))
    cases.append(("docs/images/a.png", good, True))
    cases.append(("workspace/hd/assets/MY/0001.png", good, True))
    cases.append(("docs/images/game/a.jpg", good, True))
    cases.append(("docs/images/game/a.png", ok_png(b"x" * (MAX_IMAGE + 1)), True))
    cases.append(("docs/images/game/a.png", ok_png(exe[0x2000:0x2040]), True))
    cases.append(("docs/images/game/a.png", good + exe[0x2000:0x2040], True))
    cases.append(("docs/images/game/a.png", good + secret, True))
    cases.append(("docs/images/game/a.png", ok_png(extra=[(b"tEXt", b"note")]), True))
    cases.append(("docs/images/game/a.png", ok_png(extra=[(b"PLTE", secret)]), True))
    cases.append(("docs/images/game/a.png", ok_png(extra=[(b"PLTE", bytes((v + 0x41) & 0xFF for v in table[5:40]))]), True))
    cases.append(("docs/images/game/a.png", good[:-1] + bytes([good[-1] ^ 1]), True))
    # 介面字型的例外：路徑與雜湊都要對。
    cases.append((FONT_ASSET, b"x", True))
    cases.append((FONT_ASSET, b"x" * (MAX_SIZE + 1), True))
    cases.append(("crates/rs2-play/assets/other.bin", b"x", True))
    bad = 0
    for name, data, expect in cases:
        hit = bool(scan(name, data, win, hexwin, pw_raw, pw_letters, set()))
        print("OK " if hit == expect else "FAIL", name, "命中" if hit else "未命中", "預期", "命中" if expect else "未命中")
        bad += hit != expect
    # 例外清單不能放行密碼表
    hit = scan("a.md", clean + table[10:40].hex(" ").encode(), win, hexwin, pw_raw, pw_letters, {table[10:40].hex(" ").encode()})
    print("OK " if hit else "FAIL", "例外清單不放行密碼表")
    bad += not hit
    # 放行一段字串時，同檔其他原版內容仍要命中
    planted = exe[0x2000:0x2040]
    hit = scan("a.md", clean + b"allowed string" + planted, win, hexwin, pw_raw, pw_letters, {b"allowed string"})
    print("OK " if hit else "FAIL", "放行字串不放行同檔其他內容")
    bad += not hit
    # 放行的字串本身不命中
    s_exact = exe[0x2000:0x2040]
    hit = scan("a.md", clean + s_exact, win, hexwin, pw_raw, pw_letters, {s_exact})
    print("OK " if not hit else "FAIL", "放行字串本身不命中")
    bad += bool(hit)
    sys.exit(1 if bad else 0)


if sys.argv[1] == "--selftest":
    selftest(sys.argv[2], sys.argv[3])
repo, orig, unp = sys.argv[1:4]
win, hexwin, pw_raw, pw_letters, exe, table = setup(orig, unp)
allow = set()
ap = os.path.join(repo, "tools", "leak_allow.txt")
if os.path.exists(ap):
    for line in open(ap, encoding="utf-8"):
        if line.strip() and not line.startswith("#"):
            a, _reason = line.rstrip("\n").split("\t", 1)
            allow.add(a.encode())
files = [x.strip() for x in sys.stdin if x.strip()]
total = 0
for f in files:
    p = os.path.join(repo, f)
    if not os.path.isfile(p):
        continue
    h = scan(f, open(p, "rb").read(), win, hexwin, pw_raw, pw_letters, allow)
    if h:
        total += 1
        print("命中", f, h[:4])
print("掃描", len(files), "檔，命中", total, "檔；放行字串", len(allow), "條")
sys.exit(1 if total else 0)
