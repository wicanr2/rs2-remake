#!/usr/bin/env bash
# 產生一組模糊測試腳本（容器內 python -I，無網路）。
#   tools/dos/fuzz/gen.sh PROFILE SEED [--frames N]
# 寫入 tools/dos/scenarios/fuzz-PROFILE-SEED.txt 與 tools/dos/fuzz/PROFILE-SEED.pokes。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
for d in tools/dos tools/dos/scenarios tools/dos/fuzz; do test -d "$ROOT/$d" || { echo "gen.sh：缺 $d" >&2; exit 1; }; done
exec docker run --rm --name "rs2-fuzz-gen-$$" --network none -u "$(id -u):$(id -g)" \
  --memory 256m --cpus 1 --pids-limit 32 --log-opt max-size=10m --log-opt max-file=3 \
  -v "$ROOT/tools/dos:/dos:ro" -v "$ROOT/tools/dos/scenarios:/dos/scenarios" -v "$ROOT/tools/dos/fuzz:/dos/fuzz" \
  python:3.12-slim timeout 60 python -I /dos/gen_fuzz.py "$@"
