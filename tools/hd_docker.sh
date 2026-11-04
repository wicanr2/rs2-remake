#!/bin/sh
# 在 rs2-hdtools 映像內跑 HD 工具。用法：tools/hd_docker.sh <工具檔名> <參數...>，路徑一律用 /in 與 /w。
ROOT=$(cd "$(dirname "$0")/.." && pwd)
T=$1; shift
exec docker run --rm --network none -u "$(id -u):$(id -g)" --memory 4g --cpus 4 --pids-limit 128 \
  --log-opt max-size=10m --log-opt max-file=3 -e PYTHONDONTWRITEBYTECODE=1 \
  -v "$ROOT/original/rs-2:/in:ro" -v "$ROOT/tools:/tools:ro" -v "$ROOT/workspace:/w" \
  rs2-hdtools:r1 timeout 1500 python "/tools/$T" "$@"
