#!/bin/sh
# 用法見 tools/q.py。唯讀掛載 workspace/ida。
ROOT=$(cd "$(dirname "$0")/.." && pwd)
test -f "$ROOT/workspace/ida/s_disasm.txt" || { echo "先跑 tools/ida_export.sh" >&2; exit 1; }
exec docker run --rm --network none -u "$(id -u):$(id -g)" --memory 1g --cpus 1 --pids-limit 64 \
  --log-opt max-size=10m --log-opt max-file=3 -v "$ROOT/workspace/ida:/work:ro" -v "$ROOT/tools:/tools:ro" \
  python:3.12-slim timeout 120 python -I /tools/q.py "$@"
