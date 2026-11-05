#!/usr/bin/env bash
# 錄製模糊測試收據：每個 NAME（profile-seed）跑 tools/dos/scenarios/fuzz-NAME.txt，輸出 workspace/dosrun/fuzz_NAME/。
#   tools/dos/fuzz/run.sh [-j 並行數，預設 3，上限 3] NAME...
# 每個容器 2 核、2 GB，名稱前綴 rs2-fuzz-。腳本要先用 tools/dos/fuzz/gen.sh 產生。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
J=3
if [[ "${1:-}" == "-j" ]]; then J="$2"; shift 2; fi
(( J >= 1 && J <= 3 )) || { echo "run.sh：並行數 1 到 3" >&2; exit 1; }
for n in "$@"; do
  [[ "$n" =~ ^[a-z]+-[0-9]+$ ]] || { echo "run.sh：名稱 $n 不合格式 profile-seed" >&2; exit 1; }
  test -f "$ROOT/tools/dos/scenarios/fuzz-$n.txt" || { echo "run.sh：缺 fuzz-$n.txt，先跑 gen.sh" >&2; exit 1; }
done
export ROOT RS2_CPUS=2 RS2_MEM=2g RS2_CNAME=rs2-fuzz
printf '%s\n' "$@" | xargs -P "$J" -I{} bash -c '"$ROOT/tools/dos/run_scenario.sh" "fuzz-{}" "fuzz_{}" A 1500 2>&1 | sed "s/^/{}: /"'
