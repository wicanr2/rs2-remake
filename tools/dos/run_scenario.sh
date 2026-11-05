#!/usr/bin/env bash
# 跑一個腳本，輸出到 workspace/dosrun/<輸出名>/。
#   tools/dos/run_scenario.sh <腳本名（不含 .txt）> <輸出名> [A|B|C] [逾時秒]
# 配置：A ＝ ASMDRV 以 TSR 常駐＋AdLib（預設，RS2.BAT 的順序）
#       B ＝ 不載 ASMDRV（INT 60h 維持 dosgolem 的預設 stub，遊戲偵測不到 AdLib）
#       C ＝ ASMDRV 常駐但沒有 AdLib 硬體
# 腳本在 tools/dos/scenarios/。可用 RS2_CPUS／RS2_MEM 調整容器資源。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SC="$1"; OUT="$2"; CFG="${3:-A}"; TMO="${4:-570}"
test -f "$ROOT/tools/dos/scenarios/$SC.txt" || { echo "找不到腳本 $SC" >&2; exit 1; }
# RS2_RUN_OUT：收據輸出根目錄，預設 workspace/dosrun。單色模式的收據放 workspace/dosrun-modes（dd.sh 讀同一個變數）。
RUNROOT="${RS2_RUN_OUT:-$ROOT/workspace/dosrun}"
[[ "$OUT" =~ ^[A-Za-z0-9_-]+$ ]] || { echo "輸出名只能用英數、底線、連字號" >&2; exit 1; }
rm -rf "${RUNROOT:?}/$OUT"; mkdir -p "$RUNROOT/$OUT"
case "$CFG" in
  A) ARGS=(-chain ASMDRV.EXE -adlib) ;;
  B) ARGS=() ;;
  C) ARGS=(-chain ASMDRV.EXE) ;;
  *) echo "配置只有 A、B、C" >&2; exit 1 ;;
esac
set +e
"$ROOT/tools/dos/dd.sh" "$OUT" "$TMO" rs2dyn -root /orig -out "/out/$OUT" -scratch "/out/scratch-$OUT" "${ARGS[@]}" \
  -script "/tools-dos/scenarios/$SC.txt" > "$RUNROOT/$OUT/stdout.txt" 2>&1
echo "exit=$?" >> "$RUNROOT/$OUT/stdout.txt"
tail -2 "$RUNROOT/$OUT/stdout.txt"
