#!/bin/sh
# 產生 README 用的對照圖到 docs/images/。圖含原版美術，進版控是 AGENTS.md 硬規則 4 的例外，repo 要維持私人。
# 前提：tools/run_all.sh、HD 成品（workspace/hd/assets）、
# tools/rust.sh test -p rs2-trace --test hd_tour -- --ignored（產生 workspace/rs2/shots/tour）。
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
for d in workspace/rs2/shots/tour workspace/hd/assets/overview workspace/hd/assets/maps workspace/extracted/maps; do
  test -d "$ROOT/$d" || { echo "缺 $d" >&2; exit 1; }
done
mkdir -p "$ROOT/docs/images"
exec docker run --rm --network none -u "$(id -u):$(id -g)" --memory 3g --cpus 2 --pids-limit 32 \
  --log-opt max-size=10m --log-opt max-file=3 -e PYTHONDONTWRITEBYTECODE=1 \
  -v "$ROOT/tools:/tools:ro" -v "$ROOT/workspace:/w:ro" -v "$ROOT/docs/images:/out" \
  rs2-hdtools:r1 timeout 300 python -I /tools/readme_images.py /w /out
