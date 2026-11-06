#!/bin/sh
# 容器內：產生推廣片用的輸入腳本（每步一行 {"sc":[掃描碼...]}，格式同 flow-sc.jsonl）。
# 掃描碼：開火是右 Shift 36h（54）與放開 B6h（182），每步按放一次。方向 ↑48h ↓50h ←4Bh →4Dh，放開碼加 128。
# 熱鍵 Ctrl+C（1Dh、2Eh）直接過關，放開碼 9Dh、AEh。下面的步數偏移取自全流程收據 flow-sc 的相同動作。
# 輸出（寫到 /out）：
#   stage1-move.jsonl   從 --start newgame 起，第一關一路上下左右移動加開火
#   flow-play.jsonl     從 --start title 起，標題進第一關，Ctrl+C 過關，第二、三關移動加開火，第四關過關進片尾
# 環境變數可覆寫各關的起點與長度（探測後微調）：G1 L1 GAP2 L2 GAP3 L3 GAP4 L4 TAIL
set -eu
G1="${G1:-148}"; L1="${L1:-30}"
GAP2="${GAP2:-145}"; L2="${L2:-190}"
GAP3="${GAP3:-105}"; L3="${L3:-190}"
GAP4="${GAP4:-112}"; L4="${L4:-60}"
TAIL="${TAIL:-170}"
STAGE1_STEPS="${STAGE1_STEPS:-1300}"

awk -v G1="$G1" -v L1="$L1" -v GAP2="$GAP2" -v L2="$L2" -v GAP3="$GAP3" -v L3="$L3" \
    -v GAP4="$GAP4" -v L4="$L4" -v TAIL="$TAIL" -v S1="$STAGE1_STEPS" -v OUTDIR=/out '
function add(i, c) { ev[i] = (ev[i] == "" ? c : ev[i] "," c) }
# 第 a 步到第 b-1 步：每步開火，方向依 pat 輪流按住。b 那一步放開最後的方向。
function play(a, b,   pos, k, t, d, len, cur) {
  pos = a; k = 1; cur = 0
  while (pos < b) {
    split(pat[k], t, ":"); d = t[1] + 0; len = t[2] + 0
    if (cur) add(pos, cur + 128)
    add(pos, d); cur = d
    pos += len; k = (k % npat) + 1
  }
  for (i = a; i < b; i++) { add(i, 54); add(i, 182) }
  if (cur) add(b, cur + 128)
}
# Ctrl+C 與放開、換關的空白鍵（偏移同 flow-sc：放開 +12、+18，空白鍵 +82、+90）。
function ctrlc(c) { add(c, 29); add(c, 46); add(c + 12, 174); add(c + 18, 157); add(c + 82, 57); add(c + 90, 185) }
function dump(file, n,   i) {
  for (i = 0; i < n; i++) print "{\"sc\":[" ev[i] "]}" > file
  close(file)
}
BEGIN {
  npat = split("72:10 77:4 80:18 75:4 72:8 80:8 77:3 72:14 75:3 80:6", pat, " ")

  # stage1-move：newgame 直接是第一關，第 0 步就是遊玩。
  play(0, S1)
  dump(OUTDIR "/stage1-move.jsonl", S1 + 2)
  for (i in ev) delete ev[i]

  # flow-play：標題。第 19 步空白鍵略過開場，第 92 步空白鍵開始，放開碼在 +12、+8（同 flow-sc）。
  add(19, 57); add(31, 185); add(92, 57); add(100, 185)
  play(G1, G1 + L1); C1 = G1 + L1; ctrlc(C1)
  G2 = C1 + GAP2; play(G2, G2 + L2); C2 = G2 + L2; ctrlc(C2)
  G3 = C2 + GAP3; play(G3, G3 + L3); C3 = G3 + L3; ctrlc(C3)
  G4 = C3 + GAP4; play(G4, G4 + L4); C4 = G4 + L4; add(C4, 29); add(C4, 46); add(C4 + 12, 174); add(C4 + 18, 157)
  # 片尾：同 flow-sc，第四關 Ctrl+C 後 +98 空白鍵。
  add(C4 + 98, 57); add(C4 + 110, 185)
  total = C4 + TAIL
  dump(OUTDIR "/flow-play.jsonl", total)
  printf "flow-play：G1=%d C1=%d G2=%d C2=%d G3=%d C3=%d G4=%d C4=%d 共 %d 步\n", G1, C1, G2, C2, G3, C3, G4, C4, total
}' < /dev/null
ls -l /out/stage1-move.jsonl /out/flow-play.jsonl
