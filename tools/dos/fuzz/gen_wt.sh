#!/usr/bin/env bash
# 產生全流程對拍用的隨機輸入腳本（容器內 python -I，無網路）。
#   tools/dos/fuzz/gen_wt.sh wt STAGE PROFILE SEED     → tools/dos/scenarios/wt-fzSTAGE-PROFILE-SEED.txt
#   tools/dos/fuzz/gen_wt.sh duo SEED                  → tools/dos/scenarios/wt-p2.txt
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
for d in tools/dos tools/dos/scenarios; do test -d "$ROOT/$d" || { echo "gen_wt.sh：缺 $d" >&2; exit 1; }; done
exec docker run --rm --name "rs2-wt-gen-$$" --network none -u "$(id -u):$(id -g)" \
  --memory 256m --cpus 1 --pids-limit 32 --log-opt max-size=10m --log-opt max-file=3 \
  -v "$ROOT/tools/dos:/dos:ro" -v "$ROOT/tools/dos/scenarios:/dos/scenarios" \
  python:3.12-slim timeout 60 python -I /dos/gen_fuzz.py "$@"
