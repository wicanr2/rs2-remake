#!/usr/bin/env bash
# 用收據的 in1 驗證送鍵時序（容器內 python -I，無網路）。
#   tools/dos/fuzz/check.sh NAME...        NAME = profile-seed
# 每一列玩家 1 讀到的方向位元要等於腳本在那一列送的方向鍵狀態，不符的列印出來，有不符就回傳 1。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
rc=0
for n in "$@"; do
  [[ "$n" =~ ^[a-z]+-[0-9]+$ ]] || { echo "check.sh：名稱 $n 不合格式 profile-seed" >&2; exit 1; }
  test -f "$ROOT/tools/dos/scenarios/fuzz-$n.txt" || { echo "check.sh：缺 fuzz-$n.txt" >&2; exit 1; }
  test -f "$ROOT/workspace/dosrun/fuzz_$n/trace-fz.jsonl" || { echo "check.sh：缺收據 fuzz_$n" >&2; exit 1; }
  docker run --rm --name "rs2-fuzz-check-$$" --network none -u "$(id -u):$(id -g)" \
    --memory 512m --cpus 1 --pids-limit 32 --log-opt max-size=10m --log-opt max-file=3 \
    -v "$ROOT/tools/dos:/dos:ro" -v "$ROOT/workspace/dosrun/fuzz_$n:/rec:ro" \
    python:3.12-slim timeout 120 python -I /dos/gen_fuzz.py check "/dos/scenarios/fuzz-$n.txt" /rec/trace-fz.jsonl | sed "s/^/$n: /" || rc=1
done
exit $rc
