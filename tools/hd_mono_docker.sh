#!/bin/sh
# 在 rs2-hdtools 映像內跑單色 HD 試稿工具。用法：tools/hd_mono_docker.sh <工具檔名> <參數...>，路徑一律用 /in 與 /w。
# 同 hd_docker.sh，另外給容器 rs2-mono- 前綴的名字。只清自己建立的容器（--rm），不碰其他 docker 資源。
ROOT=$(cd "$(dirname "$0")/.." && pwd)
for d in original/rs-2 tools workspace; do
  test -d "$ROOT/$d" || { echo "缺目錄 $d" >&2; exit 1; }
done
T=$1; shift
exec docker run --rm --name "rs2-mono-tool-$$" --network none -u "$(id -u):$(id -g)" --memory 4g --cpus 4 --pids-limit 128 \
  --log-opt max-size=10m --log-opt max-file=3 -e PYTHONDONTWRITEBYTECODE=1 \
  -v "$ROOT/original/rs-2:/in:ro" -v "$ROOT/tools:/tools:ro" -v "$ROOT/workspace:/w" \
  rs2-hdtools:r1 timeout 1500 python -I "/tools/$T" "$@"
