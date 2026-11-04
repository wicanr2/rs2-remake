#!/usr/bin/env bash
# 用 dosgolem 自己的 tools/go.sh 建置探針，輸出到 workspace/dosgolem/workplace/bin/。
#   tools/dos/build.sh run probe rs2dyn ...
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT/workspace/dosgolem"
for p in "$@"; do
  DOSGOLEM_TIMEOUT=10m tools/go.sh build -o "/src/workplace/bin/$p" "./cmd/$p"
done
