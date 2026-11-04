#!/usr/bin/env python3
"""靜態解開 RS-2 的 LZEXE 外殼，不執行任何原版程式。

用法：unpack_exe.py <原版目錄> <輸出目錄>
S.EXE 是兩層 LZEXE：外層表頭簽章被改成四個空白，內層沒有 MZ 表頭。
第一層解開後，映像內有一段 0969:0100 的包裝碼，再交棒給第二層 stub（0942:000E）。
"""
import hashlib
import json
import os
import struct
import sys


def lz91(src, start=0):
    """LZEXE 0.91 位元串流。回傳解開的位元組與串流結束位置。"""
    pos = start

    def rw():
        nonlocal pos
        v = src[pos] | (src[pos + 1] << 8)
        pos += 2
        return v

    buf = rw()
    cnt = 16

    def bit():
        nonlocal buf, cnt
        r = buf & 1
        cnt -= 1
        if cnt == 0:
            buf = rw()
            cnt = 16
        else:
            buf >>= 1
        return r

    out = bytearray()
    while True:
        if bit():
            out.append(src[pos])
            pos += 1
            continue
        if bit() == 0:
            n = bit() << 1
            n |= bit()
            n += 2
            off = src[pos] - 256
            pos += 1
        else:
            a = src[pos]
            c = src[pos + 1]
            pos += 2
            off = (a | ((c & 0xF8) << 5)) - 0x2000
            n = (c & 7) + 2
            if n == 2:
                x = src[pos]
                pos += 1
                if x == 0:
                    break
                if x == 1:
                    continue
                n = x + 1
        for _ in range(n):
            out.append(out[off])
    return bytes(out), pos


def relocs91(buf, p):
    rs = 0
    ro = 0
    out = []
    while True:
        span = buf[p]
        p += 1
        if span == 0:
            span = buf[p] | (buf[p + 1] << 8)
            p += 2
            if span == 0:
                rs += 0xFFF
                continue
            if span == 1:
                break
        ro += span
        rs += (ro & ~0xF) >> 4
        ro &= 0xF
        out.append((ro, rs))
    return out, p


def relocs90(buf, p):
    out = []
    seg = 0
    while True:
        c = buf[p] | (buf[p + 1] << 8)
        p += 2
        for _ in range(c):
            ro = buf[p] | (buf[p + 1] << 8)
            p += 2
            out.append((ro, seg))
        seg += 0x1000
        if seg > 0xF000:
            break
    return out, p


def mz(image, relocs, ip, cs, sp, ss, minalloc=0x1000):
    hdrsz = (0x1C + len(relocs) * 4 + 15) // 16
    total = hdrsz * 16 + len(image)
    h = struct.pack("<2s13H", b"MZ", total % 512, (total + 511) // 512, len(relocs), hdrsz,
                    minalloc, 0xFFFF, ss, sp, 0, ip, cs, 0x1C, 0)
    h += b"".join(struct.pack("<HH", a, b) for a, b in relocs)
    return h.ljust(hdrsz * 16, b"\0") + image


def unpack_one(b, ver):
    hdr = struct.unpack_from("<H", b, 8)[0] * 16
    ip, cs = struct.unpack_from("<HH", b, 0x14)
    entry = hdr + cs * 16
    o_ip, o_cs, o_sp, o_ss, comp, inc, dec = struct.unpack_from("<7H", b, entry)
    image, _ = lz91(b[hdr:])
    rel, _ = (relocs91(b, entry + 0x158) if ver == 91 else relocs90(b, entry + 0x19D))
    return image, rel, {"ip": o_ip, "cs": o_cs, "sp": o_sp, "ss": o_ss, "signature_bytes": b[0x1C:0x20].hex()}


def main():
    src, dst = sys.argv[1], sys.argv[2]
    os.makedirs(dst, exist_ok=True)
    rec = {}

    def save(name, data):
        with open(os.path.join(dst, name), "wb") as fh:
            fh.write(data)
        rec[name] = {"size": len(data), "sha256": hashlib.sha256(data).hexdigest()}

    b = open(os.path.join(src, "S.EXE"), "rb").read()
    img1, rel1, r1 = unpack_one(b, 91)
    save("S.layer1.bin", img1)
    # 第二層 stub 的段落值由第一層 0969:0100 的包裝碼資料欄 (+4,+6 字組後) 取得
    wrap = (r1["cs"] << 4) + r1["ip"]
    if img1[wrap:wrap + 2] != bytes([0xFA, 0xE9]):
        raise SystemExit("第一層進入點不是預期的 FA E9 包裝碼，這份 S.EXE 不是已知版本")
    layer2_cs = struct.unpack_from("<H", img1, wrap + 6)[0]
    entry2 = layer2_cs * 16
    o_ip, o_cs, o_sp, o_ss, comp, inc, dec = struct.unpack_from("<7H", img1, entry2)
    img2, end2 = lz91(img1)
    rel2, p = relocs91(img1, entry2 + 0x158)
    save("S.unpacked.bin", img2)
    save("S.unpacked.exe", mz(img2, rel2, o_ip, o_cs, o_sp, o_ss))
    rec["S.chain"] = {
        "layer1": r1,
        "wrapper": {"entry": f"{r1['cs']:04X}:{r1['ip']:04X}", "image_offset": wrap,
                    "head_hex": img1[wrap:wrap + 12].hex(" ")},
        "layer2": {"stub": f"{layer2_cs:04X}:000E", "ip": o_ip, "cs": o_cs, "sp": o_sp, "ss": o_ss,
                   "relocs": len(rel2), "image_size": len(img2)},
    }
    for name, ver in (("ASMDRV", 90), ("SEN", 91)):
        b = open(os.path.join(src, name + ".EXE"), "rb").read()
        img, rel, r = unpack_one(b, ver)
        save(name + ".unpacked.exe", mz(img, rel, r["ip"], r["cs"], r["sp"], r["ss"]))
        rec[name + ".chain"] = dict(r, relocs=len(rel), image_size=len(img))
    with open(os.path.join(dst, "unpack-manifest.json"), "w", encoding="utf-8") as fh:
        json.dump(rec, fh, ensure_ascii=False, indent=1)
    print(json.dumps(rec, ensure_ascii=False, indent=1))


main()
