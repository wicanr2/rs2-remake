#!/usr/bin/env python3
"""推送前的外洩閘門：確認要進版控的檔案沒有夾帶原版素材或密碼表。

用法（在容器內）：
  leak_check.py <repo> <original/rs-2> <S.unpacked.bin> < 檔案清單（git ls-files，每行一個）
  leak_check.py --selftest <original/rs-2> <S.unpacked.bin>
有命中就以 1 結束。例外列在 tools/leak_allow.txt，每行「完整字串<TAB>原因」。只放行那段字串本身，檔案其他內容照查，
密碼表不受例外影響。
"""
import os
import re
import sys

DENY_EXT = {".png", ".jpg", ".exe", ".zip", ".mid", ".vgm", ".wav", ".bin", ".i64", ".go", ".gob", ".got", ".bdb", ".bdh", ".bdt",
            ".bd1", ".bd2", ".bd3", ".bd4", ".mix", ".lck", ".scn", ".scr", ".joy", ".cfg", ".diz", ".bat", ".pas"}
# 工具本身的 Go 原始碼（dosgolem patch 內）副檔名是 .patch，不受影響。原版的 .GO 與 Go 原始碼同副檔名，以大小寫與所在目錄區分。
SOURCE_GO_OK = re.compile(r"^(tools/|crates/).*\.go$")
MAX_SIZE = 300 * 1024
WIN = 32
HEXWIN = 24
HEXRUN = re.compile(rb"(?:\b(?:0x)?[0-9A-Fa-f]{2}h?\b[ ,\t]*){%d,}" % HEXWIN)
DS = 0xCBC0
PW_OFF = DS + 0x8BD
PW_LEN = 26 * 26


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
    if ext in DENY_EXT and not (ext == ".go" and SOURCE_GO_OK.match(name)):
        hits.append(("副檔名", ext))
    if len(data) > MAX_SIZE:
        hits.append(("過大", len(data)))
    # 密碼表無論如何都不放行
    for i in range(0, len(data) - 11):
        if data[i:i + 12] in pw_raw:
            hits.append(("密碼表原始位元組", i))
            break
    for w in pw_letters:
        if w in data:
            hits.append(("密碼表字母形式", w.decode()))
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
