#!/usr/bin/env python3
"""用完整逐幀收據驗證敵彈槽號 >= 15 的進位缺陷（bullet_slot_carry）。

  python -I analyze_slot_carry.py TRACE.jsonl [--image /ida/S.unpacked.exe] [--show N]

做法：玩家 1 受擊的那一幀（p1.hit = 59h）依 sub_17855 從該幀的敵彈池重建 G2 格網
（格值 = 形狀格值 + 槽號，8 位元相加），再依 sub_14C85 讀玩家 1 的判定帶（形狀 0F0Dh，5x1），
對每個非 0 格依 sub_14D54 推出程式碼會刪除的槽：((格值 and 0Fh) - 1) and FFh。
寫入該格的槽與程式碼刪除的槽不同，就是進位造成的錯刪。再與收據比對：
被錯刪的槽在前一幀存在、這一幀不存在，而真正擊中的槽留在池裡。

只用標準函式庫。形狀與列偏移表取自解包映像（dseg 位移 X = 映像位移 0CBC0h + X），
映像路徑用 --image 指定（預設 /ida/S.unpacked.exe，tools/dos/analyze_slot_carry.sh 會掛載）。
"""
import json
import struct
import sys

DS_IN_IMG = 0xCBC0
G2 = 0x2C1C


def load_image(path):
    b = open(path, "rb").read()
    return b[struct.unpack("<H", b[8:10])[0] * 16:]


class Img:
    def __init__(self, img):
        self.img = img

    def w(self, off):
        return struct.unpack_from("<H", self.img, DS_IN_IMG + off)[0]

    def b(self, off):
        return self.img[DS_IN_IMG + off]

    def shape(self, p):
        w, h = self.w(p), self.w(p + 2)
        return w, h, [self.b(p + 4 + i) for i in range(w * h)]


def row_addr(im, bx):
    bx &= 0xFFFF
    if bx >= 0x14:  # sub_17898 與 sub_14D68：列 >= 14h（含負列）都夾到 14h，該項在表中為 0
        bx = 0x14
    return im.w(0x1B0A + 2 * bx) + G2


def sim_g2(im, eb):
    g = {}
    for s in sorted(eb):
        w = eb[s]
        x, y, sp = w[9], w[10], w[14]
        sw, sh, cells = im.shape(sp)
        for r in range(sh):
            ra = row_addr(im, (y + r) & 0xFFFF)
            dx = ra + 0x27
            for c in range(sw):
                di = (ra + x + c) & 0xFFFF
                if di > 0x2F3C or di < G2 or di >= dx or di <= ra:  # sub_17855 的四個範圍檢查
                    continue
                v = cells[r * sw + c]
                if v:
                    g[di] = ((v + s) & 0xFF, s)  # add al, byte_1F7DB
    return g


def strip_reads(im, g, col, row):
    sw, sh, _ = im.shape(0xF0D)
    out = []
    for r in range(sh):
        ra = row_addr(im, row + 1 + r)
        dx = ra + 0x45
        for c in range(sw):
            di = (ra + col + c) & 0xFFFF
            if di >= 0x3266 or di <= G2 or di >= dx or di <= ra:  # sub_14C85 的範圍檢查
                continue
            if di in g:
                out.append((di, g[di][0], g[di][1]))
    return out


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    path, image, show = argv[0], "/ida/S.unpacked.exe", 8
    i = 1
    while i < len(argv):
        if argv[i] == "--image":
            image = argv[i + 1]
        elif argv[i] == "--show":
            show = int(argv[i + 1])
        else:
            print("不認得的參數", argv[i], file=sys.stderr)
            return 2
        i += 2
    im = Img(load_image(image))
    rows = [json.loads(l) for l in open(path)]
    hit_rows = ge15 = mis = verified = shown = 0
    for t in range(1, len(rows)):
        o, p = rows[t], rows[t - 1]
        if o["p1"]["hit"] != 0x59:
            continue
        hit_rows += 1
        eb = {r["s"]: r["w"] for r in o["eb"]}
        prev = {r["s"]: r["w"] for r in p["eb"]}
        reads = strip_reads(im, sim_g2(im, eb), o["p1"]["col"], o["p1"]["row"])
        if any(s >= 15 for _, _, s in reads):
            ge15 += 1
        bad = [(a, v, s) for a, v, s in reads
               if ((v & 0xF) - 1) & 0xFF != s and (v & 0xF0) not in (0x80, 0x40, 0xC0, 0x20)]
        if not bad:
            continue
        mis += 1
        a, v, s = bad[0]
        k = ((v & 0xF) - 1) & 0xFF
        ok = k in prev and k not in eb and s in eb
        verified += ok
        if shown < show:
            shown += 1
            pos = (prev[k][9], prev[k][10]) if k in prev else None
            print("n=%d frame=%d player=(%d,%d) cell=%02X written_by_slot=%d code_frees_slot=%d "
                  "slot_%d_before=%s after=%s pos_before=%s; slot_%d_after=%s" % (
                      o["n"], o["frame"], o["p1"]["col"], o["p1"]["row"], v, s, k, k, k in prev, k in eb, pos, s, s in eb))
    print("hit_rows=%d rows_reading_slot_ge15=%d rows_with_cell_slot_mismatch=%d "
          "mismatch_with_wrong_slot_freed_and_hitting_slot_alive=%d" % (hit_rows, ge15, mis, verified))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
