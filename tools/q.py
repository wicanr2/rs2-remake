#!/usr/bin/env python3
"""查詢 IDA 匯出的反組譯與解包映像。給人與代理共用，在容器內跑（tools/q.sh）。

  func NAME [最多列數]      印函式內容（到下一個 proc 為止）
  range 起 迄               印線性位址範圍，十六進位不帶 h
  grep 正規式 [最多筆]      對去註解後的反組譯做 regex，每筆標所屬函式
  callers NAME              列 call/jmp 到 NAME 的位置
  refs 符號                 列所有提到該符號的行
  bytes 線性位址 長度       印解包映像的原始位元組
  ds 偏移 長度              印 dseg:偏移 的位元組（線性 = 1CBC0h + 偏移）
  dw 偏移 字組數            以 u16 印 dseg:偏移
  str 偏移                  印 dseg:偏移 起的可列印字串

注意：IDA 的 proc 邊界可能把 retn 之後的死碼算進前一個函式，判定可達性要看控制流。
"""
import re
import struct
import sys

W = "/work"
L = open(f"{W}/s_disasm.txt", errors="replace").read().split("\n")
b = open(f"{W}/S.unpacked.exe", "rb").read()
img = b[struct.unpack("<H", b[8:10])[0] * 16:]
BASE = 0x10000
DS = 0x1CBC0


def clean(x):
    return re.sub(r"\s{2,}", " ", x.split(";")[0].rstrip())


idx = {}
owner = []
cur = None
for i, l in enumerate(L):
    if l.endswith(" proc"):
        cur = l.split()[1]
        idx[cur] = i
    owner.append(cur)
c = sys.argv[1]
a = sys.argv[2:]
if c == "func":
    i = idx[a[0]]
    n = int(a[1]) if len(a) > 1 else 10 ** 6
    print(clean(L[i]))
    j = i + 1
    k = 0
    while j < len(L) and k < n and not L[j].endswith(" proc"):
        s = clean(L[j])
        if s:
            print(s)
            k += 1
        j += 1
elif c == "range":
    s, e = int(a[0], 16), int(a[1], 16)
    for l in L:
        m = re.match(r"^([0-9A-F]{5})", l)
        if m and s <= int(m.group(1), 16) <= e and clean(l):
            print(clean(l))
elif c == "grep":
    r = re.compile(a[0])
    n = int(a[1]) if len(a) > 1 else 200
    k = 0
    for i, l in enumerate(L):
        s = clean(l)
        if r.search(s):
            print(f"[{owner[i]}] {s}")
            k += 1
            if k >= n:
                print("... 截斷")
                break
elif c == "callers":
    r = re.compile(r"\b(call|jmp|j[a-z]+|loop)\b.*\b" + re.escape(a[0]) + r"\b")
    for i, l in enumerate(L):
        s = clean(l)
        if r.search(s):
            print(f"[{owner[i]}] {s}")
elif c == "refs":
    r = re.compile(r"\b" + re.escape(a[0]) + r"\b")
    for i, l in enumerate(L):
        s = clean(l)
        if r.search(s):
            print(f"[{owner[i]}] {s}")
elif c == "bytes":
    p = int(a[0], 16) - BASE
    print(img[p:p + int(a[1], 0)].hex(" "))
elif c == "ds":
    p = DS - BASE + int(a[0], 16)
    d = img[p:p + int(a[1], 0)]
    for i in range(0, len(d), 16):
        print(f"{int(a[0], 16) + i:05X}  {d[i:i + 16].hex(' ')}")
elif c == "dw":
    p = DS - BASE + int(a[0], 16)
    n = int(a[1], 0)
    print(" ".join(f"{v:04X}" for v in struct.unpack_from(f"<{n}H", img, p)))
elif c == "str":
    p = DS - BASE + int(a[0], 16)
    e = p
    while 32 <= img[e] < 127:
        e += 1
    print(img[p:e].decode())
else:
    print(__doc__)
