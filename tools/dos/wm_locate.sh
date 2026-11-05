#!/usr/bin/env bash
# 單色收據的第一個不同處定位：重跑 wt-NAME 腳本，到 waittrace 的第 N 筆（收據的 n 欄，0 起算）寫完就傾印
# 資料段、顯示記憶體（.mono，B0000..BFFFF）與背景暫存區（.msa、.msb），然後正常結束。
#   tools/dos/wm_locate.sh NAME N [逾時秒，預設 1800]     NAME 例：stage3-h
# 輸出 <收據根>/wl_<NAME>_<N>/：wt_<NAME>-w<N>.ds、.mono、.msa、.msb、.mono.json、.png。
# 測試端用 RS2_MONO_LOCATE=1 讀它（crates/rs2-trace/src/mono.rs 的 locate_detail）。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
NAME="${1:?用法：wm_locate.sh NAME N [逾時秒]}"; N="${2:?缺筆序 N}"; T="${3:-1800}"
NAME="${NAME#wm_}"; NAME="${NAME#wt_}"; NAME="${NAME#wt-}"
[[ "$NAME" =~ ^[a-z0-9-]+$ ]] || { echo "wm_locate.sh：名稱 $NAME 不合格式" >&2; exit 1; }
[[ "$N" =~ ^[0-9]+$ ]] || { echo "wm_locate.sh：N 要是非負整數" >&2; exit 1; }
test -f "$ROOT/tools/dos/scenarios/wt-$NAME.txt" || { echo "wm_locate.sh：找不到腳本 wt-$NAME.txt" >&2; exit 1; }
RUNROOT="${RS2_RUN_OUT:-$ROOT/workspace/dosrun-modes}"
mkdir -p "$RUNROOT"
export RS2_RUN_OUT="$RUNROOT" RS2_CNAME=rs2-modes RS2_CPUS="${RS2_CPUS:-2}" RS2_MEM="${RS2_MEM:-2g}"
OUT="wl_${NAME}_${N}"
rm -rf "${RUNROOT:?}/$OUT"; mkdir -p "$RUNROOT/$OUT"
set +e
"$ROOT/tools/dos/dd.sh" "$OUT" "$T" rs2dyn -root /orig -out "/out/$OUT" -scratch "/out/scratch-$OUT" -chain ASMDRV.EXE -adlib \
  -waitstop "$N" -script "/tools-dos/scenarios/wt-$NAME.txt" > "$RUNROOT/$OUT/stdout.txt" 2>&1
echo "exit=$?" >> "$RUNROOT/$OUT/stdout.txt"
set -e
tail -2 "$RUNROOT/$OUT/stdout.txt"
ls "$RUNROOT/$OUT" | grep -- "-w$N\." | sed "s|^|$OUT/|"
