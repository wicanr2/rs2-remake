#!/usr/bin/env bash
# 在容器內跑 analyze_slot_carry.py。路徑相對 workspace/dosrun。
#   tools/dos/analyze_slot_carry.sh trace_stage4/trace-s4.jsonl [--show N]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
test -f "$ROOT/workspace/ida/S.unpacked.exe" || { echo "analyze_slot_carry.sh：缺 workspace/ida/S.unpacked.exe" >&2; exit 1; }
F="$1"; shift
exec timeout "${CHECK_TIMEOUT:-300}" docker run --rm --name "rs2-trc-slot-$$" --network none \
  -u "$(id -u):$(id -g)" --memory 1g --cpus 1 --pids-limit 64 \
  --log-opt max-size=10m --log-opt max-file=3 \
  -v "$ROOT/workspace/dosrun:/data:ro" -v "$ROOT/tools/dos:/tools-dos:ro" -v "$ROOT/workspace/ida:/ida:ro" \
  python:3.12-slim python -I /tools-dos/analyze_slot_carry.py "/data/$F" "$@"
