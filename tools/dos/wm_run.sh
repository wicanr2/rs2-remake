#!/usr/bin/env bash
# 錄製單色模式（C、H）的全流程對拍收據：每個 NAME 跑 tools/dos/scenarios/wt-NAME.txt，輸出 <收據根>/wm_NAME/。
#   tools/dos/wm_run.sh [-j 並行數，預設 2，上限 2] [-t 逾時秒，預設 1800] NAME...
# NAME 例：title-c title-h stage1-c stage1-h stage2-c stage2-h stage3-c stage3-h stage4-c stage4-h。
# 收據根預設 workspace/dosrun-modes（RS2_RUN_OUT 可改）。目錄前綴 wm_ 而不是 wt_：E 的測試掃 wt_*，單色收據沒有平面傾印，要分開。
# 容器名前綴 rs2-modes-，每個容器 2 核、2 GB，並行 2 個共 4 核。腳本與欄位見 tools/dosgolem-patches/0008-rs2dyn-mono-vram.patch。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
J=2; T=1800
while [[ "${1:-}" == -* ]]; do
  case "$1" in
    -j) J="$2"; shift 2 ;;
    -t) T="$2"; shift 2 ;;
    *) echo "wm_run.sh：不認得 $1" >&2; exit 1 ;;
  esac
done
(( J >= 1 && J <= 2 )) || { echo "wm_run.sh：並行數 1 到 2" >&2; exit 1; }
for n in "$@"; do
  [[ "$n" =~ ^[a-z0-9-]+$ ]] || { echo "wm_run.sh：名稱 $n 不合格式" >&2; exit 1; }
  test -f "$ROOT/tools/dos/scenarios/wt-$n.txt" || { echo "wm_run.sh：缺 wt-$n.txt" >&2; exit 1; }
done
export ROOT T RS2_RUN_OUT="${RS2_RUN_OUT:-$ROOT/workspace/dosrun-modes}" RS2_CPUS="${RS2_CPUS:-2}" RS2_MEM=2g RS2_CNAME=rs2-modes
mkdir -p "$RS2_RUN_OUT"
printf '%s\n' "$@" | xargs -P "$J" -I{} bash -c '"$ROOT/tools/dos/run_scenario.sh" "wt-{}" "wm_{}" A "$T" 2>&1 | sed "s/^/{}: /"'
