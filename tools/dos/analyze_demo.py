"""把 rs2dyn 的 trace-demoN.tsv 與 DMn.LCK 比對。在容器內以 python -I 執行。
用法：python -I analyze_demo.py <trace.tsv> <DMn.LCK>
逐幀的 inbyte（byte_1DAA4）應等於 LCK[frame + k]（k 取使吻合最多的偏移）。
同時印出 facing 在哪些幀切換，以及切換幀與 LCK bit5 的對應。"""
import sys, collections

def main(trace, lck):
    d = open(lck, 'rb').read()
    rows = [l.rstrip('\n').split('\t') for l in open(trace)]
    hdr, rows = rows[0], rows[1:]
    ix = {n: i for i, n in enumerate(hdr)}
    fr = [int(r[ix['frame']]) for r in rows]
    inb = [int(r[ix['inbyte']]) for r in rows]
    face = [int(r[ix['facing']]) for r in rows]
    # 只看從 0 起連續遞增的那一段
    seq = []
    cuts = []
    for i, (f, b, fc) in enumerate(zip(fr, inb, face)):
        if i and f <= fr[i - 1] and f != fr[i - 1] + 1:
            cuts.append((i, fr[i - 1], f))
            break  # 只分析第一段連續遞增的幀
        seq.append((f, b, fc))
    print('第一段之後的幀計數回頭點 (列, 前, 後):', cuts)
    best = None
    for k in range(-3, 4):
        ok = tot = 0
        for f, b, _ in seq:
            if 0 <= f + k < len(d):
                tot += 1
                ok += (d[f + k] & 0x3F) == b
        print(f'k={k:+d}: {ok}/{tot}')
        if best is None or ok > best[1]:
            best = (k, ok, tot)
    k = best[0]
    print('best k', best)
    mism = [(f, b, d[f + k] & 0x3F) for f, b, _ in seq if 0 <= f + k < len(d) and (d[f + k] & 0x3F) != b]
    print('mismatch frames (frame, inbyte, lck&3F):', mism[:20], 'count', len(mism))
    # 各位元
    for bit in range(6):
        t = sum(1 for f, b, _ in seq if 0 <= f + k < len(d) and (d[f + k] >> bit & 1) and (b >> bit & 1))
        n = sum(1 for f, b, _ in seq if 0 <= f + k < len(d) and (d[f + k] >> bit & 1))
        print(f'bit{bit}: LCK 有 {n} 幀，inbyte 同幀也有 {t}')
    flips = [(seq[i][0], seq[i-1][2], seq[i][2]) for i in range(1, len(seq)) if seq[i][2] != seq[i-1][2]]
    print('facing 變化 (frame, 前, 後):', flips)
    bit5 = [i for i in range(len(d)) if d[i] & 0x20]
    print('LCK bit5 幀:', bit5[:12])
    print('LCK 長度', len(d), '追蹤列數', len(seq), '追蹤最大幀', max(f for f, _, _ in seq))

main(sys.argv[1], sys.argv[2])

def extra(trace, lck):
    d = open(lck, 'rb').read()
    rows = [l.rstrip('\n').split('\t') for l in open(trace)]
    hdr, rows = rows[0], rows[1:]
    ix = {n: i for i, n in enumerate(hdr)}
    g = lambda r, n: int(r[ix[n]])
    seq = []
    for i, r in enumerate(rows):
        if i and g(r, 'frame') <= g(rows[i - 1], 'frame'):
            break
        seq.append(r)
    names = [('up', 0), ('down', 1), ('right', 2), ('left', 3)]
    for n, bit in names:
        agree = sum(1 for r in seq if (g(r, n) == 1) == bool(d[g(r, 'frame') - 1] >> bit & 1))
        print(f'狀態位元組 {n} == 1 與 LCK bit{bit}：{agree}/{len(seq)} 幀吻合')
    # 位置變化：同一幀有 bit 時，下一列的 yi / xi 變化
    for n, bit, fld, sign in [('up', 0, 'yi', -1), ('down', 1, 'yi', 1), ('right', 2, 'xi', 1), ('left', 3, 'xi', -1)]:
        hit = tot = 0
        for a, b in zip(seq, seq[1:]):
            f = g(b, 'frame') - 1
            if d[f] >> bit & 1 and not (d[f] >> (bit ^ 1) & 1):
                tot += 1
                dv = g(b, fld) - g(a, fld)
                hit += (dv > 0) - (dv < 0) == sign or (dv == 0)
                # dv==0 是撞邊界
        print(f'bit{bit}（{n}）單獨出現 {tot} 幀，下一列 {fld} 往 {"+" if sign > 0 else "-"} 方向或停在邊界：{hit}/{tot}')
    # 開火：LCK bit4 的幀，子彈槽是否增加不易觀察，改看 'fire' 欄位在該幀處理後被清成 N(78)
    f78 = sum(1 for r in seq if g(r, 'fire') == 78)
    print('fire 欄位（byte_273A9）在列尾為 78(N) 的列數', f78, '／', len(seq))

extra(sys.argv[1], sys.argv[2])
