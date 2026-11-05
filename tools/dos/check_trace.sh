#!/usr/bin/env bash
# 在容器內驗證完整逐幀收據。路徑相對 workspace/dosrun。
#   tools/dos/check_trace.sh trace_dm1/trace-dm1.jsonl --expect-rows 640 [--diff 另一份.jsonl] [--lck DM1.LCK]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
test -d "$ROOT/workspace/dosrun" || { echo "check_trace.sh：缺 workspace/dosrun" >&2; exit 1; }
F="$1"; shift
args=()
while [[ $# -gt 0 ]]; do
  if [[ "$1" == "--diff" ]]; then args+=("--diff" "/data/$2"); shift 2;
  elif [[ "$1" == "--lck" ]]; then args+=("--lck" "/orig/$2"); shift 2; else args+=("$1"); shift; fi
done
exec timeout "${CHECK_TIMEOUT:-300}" docker run --rm --name "rs2-trc-check-$$" --network none \
  -u "$(id -u):$(id -g)" --memory 1g --cpus 1 --pids-limit 64 \
  --log-opt max-size=10m --log-opt max-file=3 \
  -v "$ROOT/workspace/dosrun:/data:ro" -v "$ROOT/tools/dos:/tools-dos:ro" -v "$ROOT/original/rs-2:/orig:ro" \
  python:3.12-slim python -I /tools-dos/check_trace.py "/data/$F" "${args[@]}"
