#!/usr/bin/env bash
# 跑全流程對拍測試，並把 RS2_WAIT（收據名稱子字串）、RS2_WAIT_NEG、RS2_DS_REPORT、RS2_WAIT_LOCATE 傳進容器。
# tools/rust.sh 不傳這幾個變數，所以過濾收據要用本腳本；不過濾時 `tools/rust.sh test -p rs2-trace --test wait -- --ignored --nocapture` 等價。
#   RS2_WAIT=stage1 tools/dos/wait_test.sh [cargo test 的參數，預設 -p rs2-trace --test wait -- --ignored --nocapture]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE="rust:1.99.1-slim-bookworm@sha256:24707e70a4f1e23d9a7b25202ec43126d28a52294b96aad5c670df1fd1f1d400"
WP="$ROOT/workspace"
for d in "$WP/cargo" "$WP/target" "$WP/rs2" "$ROOT/original/rs-2" "$WP/dosrun"; do test -d "$d" || { echo "wait_test.sh：缺 $d" >&2; exit 1; }; done
ARGS=("$@")
if [[ ${#ARGS[@]} -eq 0 ]]; then ARGS=(-p rs2-trace --test wait -- --ignored --nocapture); fi
exec timeout 1800 docker run --rm -u "$(id -u):$(id -g)" --name "rs2-wtest-$$" \
  --memory 4g --cpus 4 --pids-limit 256 --network none --log-opt max-size=10m --log-opt max-file=3 \
  -e CARGO_HOME=/cache -e CARGO_TARGET_DIR=/target -e HOME=/tmp \
  -e RS2_ORIG=/orig -e RS2_DOSRUN=/dosrun -e RS2_OUT=/out \
  ${RS2_WAIT:+-e RS2_WAIT} ${RS2_WAIT_NEG:+-e RS2_WAIT_NEG} ${RS2_DS_REPORT:+-e RS2_DS_REPORT} ${RS2_WAIT_LOCATE:+-e RS2_WAIT_LOCATE} \
  -v "$WP/cargo:/cache" -v "$WP/target:/target" -v "$WP/rs2:/out" \
  -v "$ROOT/original/rs-2:/orig:ro" -v "$WP/dosrun:/dosrun:ro" -v "$ROOT:/project:ro" -w /project \
  "$IMAGE" cargo test --release --locked --offline "${ARGS[@]}"
