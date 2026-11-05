#!/usr/bin/env bash
# 錄製全流程對拍收據：每個 NAME 跑 tools/dos/scenarios/wt-NAME.txt，輸出 workspace/dosrun/wt_NAME/。
#   tools/dos/wt_run.sh [-j 並行數，預設 3，上限 3] [-t 逾時秒，預設 1500] NAME...
# NAME 例：title flow menu stage2 stage3 stage4 ending p2 fz2-mix-1。容器名前綴 rs2-wt-，每個容器 2 核、2 GB。
# 腳本要先存在（fz 系列用 tools/dos/fuzz/gen_wt.sh 產生）。既有輸出目錄會被覆蓋，所以只對 wt_* 使用。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
J=3; T=1500
while [[ "${1:-}" == -* ]]; do
  case "$1" in
    -j) J="$2"; shift 2 ;;
    -t) T="$2"; shift 2 ;;
    *) echo "wt_run.sh：不認得 $1" >&2; exit 1 ;;
  esac
done
(( J >= 1 && J <= 3 )) || { echo "wt_run.sh：並行數 1 到 3" >&2; exit 1; }
for n in "$@"; do
  [[ "$n" =~ ^[a-z0-9-]+$ ]] || { echo "wt_run.sh：名稱 $n 不合格式" >&2; exit 1; }
  test -f "$ROOT/tools/dos/scenarios/wt-$n.txt" || { echo "wt_run.sh：缺 wt-$n.txt" >&2; exit 1; }
done
export ROOT T RS2_CPUS="${RS2_CPUS:-2}" RS2_MEM="${RS2_MEM:-2g}" RS2_CNAME="${RS2_CNAME:-rs2-wt}"
printf '%s\n' "$@" | xargs -P "$J" -I{} bash -c '"$ROOT/tools/dos/run_scenario.sh" "wt-{}" "wt_{}" A "$T" 2>&1 | sed "s/^/{}: /"'
