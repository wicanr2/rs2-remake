#!/usr/bin/env bash
# 全流程對拍的第一個不同處定位：重跑 wt-NAME 腳本，到 waittrace 的第 N 筆（收據的 n 欄，0 起算）寫完就傾印
# 資料段、四個平面與畫面，然後正常結束。
#   tools/dos/wait_locate.sh NAME N [逾時秒，預設 1500]
# NAME 可寫 stage2、wt_stage2 或 wt-stage2。輸出 workspace/dosrun/wl_<NAME>_<N>/：
#   wt_<NAME>-w<N>.ds、.p0～p3（平面）、.json、.png，與 wait-wt_<NAME>.jsonl、.dsd（到第 N 筆為止）。
# 重製那邊用同一筆的 .ds 與 .p0～p3 對照：資料段用 rs2_trace::wait::ds_diff，平面找偏移，
# 用 rs2_gfx::VgaSink::watch 與 last_write_seq 找最後寫入的命令（做法同 tests/a5.rs 的 last_writer）。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
NAME="${1:?用法：wait_locate.sh NAME N [逾時秒]}"; N="${2:?缺筆序 N}"; T="${3:-1500}"
NAME="${NAME#wt_}"; NAME="${NAME#wt-}"
[[ "$NAME" =~ ^[a-z0-9-]+$ ]] || { echo "wait_locate.sh：名稱 $NAME 不合格式" >&2; exit 1; }
[[ "$N" =~ ^[0-9]+$ ]] || { echo "wait_locate.sh：N 要是非負整數" >&2; exit 1; }
test -f "$ROOT/tools/dos/scenarios/wt-$NAME.txt" || { echo "wait_locate.sh：找不到腳本 wt-$NAME.txt" >&2; exit 1; }
OUT="wl_${NAME}_${N}"
rm -rf "$ROOT/workspace/dosrun/$OUT"; mkdir -p "$ROOT/workspace/dosrun/$OUT"
export RS2_CNAME=rs2-wt RS2_CPUS="${RS2_CPUS:-2}" RS2_MEM="${RS2_MEM:-2g}"
set +e
"$ROOT/tools/dos/dd.sh" "$OUT" "$T" rs2dyn -root /orig -out "/out/$OUT" -scratch "/out/scratch-$OUT" -chain ASMDRV.EXE -adlib \
  -waitstop "$N" -script "/tools-dos/scenarios/wt-$NAME.txt" > "$ROOT/workspace/dosrun/$OUT/stdout.txt" 2>&1
echo "exit=$?" >> "$ROOT/workspace/dosrun/$OUT/stdout.txt"
set -e
tail -2 "$ROOT/workspace/dosrun/$OUT/stdout.txt"
ls "$ROOT/workspace/dosrun/$OUT" | grep -- "-w$N\." | sed "s|^|$OUT/|"
