#!/usr/bin/env bash
# 在 dosgolem 映像內跑已建好的 workspace/dosgolem/workplace/bin/<程式>。
#   tools/dos/dd.sh <用途> <逾時秒> <程式> [參數...]
# 容器名 rs2-dyn-<用途>-<pid>；原版目錄唯讀掛 /orig，輸出目錄 workspace/dosrun 掛 /out（可寫）。
# 預設 --network none。資源上限：4 核、4 GB、256 pids。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PURPOSE="$1"; TMO="$2"; PROG="$3"; shift 3
IMAGE="${DOSGOLEM_GO_IMAGE:-dosgolem-go-ebiten:1.24-r1}"
ORIG="${RS2_ORIG:-$ROOT/original/rs-2}"
OUT="$ROOT/workspace/dosrun"
BIN="$ROOT/workspace/dosgolem/workplace/bin/$PROG"
test -d "$ORIG" || { echo "dd.sh：缺 $ORIG" >&2; exit 1; }
test -d "$OUT" || { echo "dd.sh：缺 $OUT" >&2; exit 1; }
test -x "$BIN" || { echo "dd.sh：缺 $BIN，先跑 tools/dos/build.sh" >&2; exit 1; }
EXTRA=()
if [[ -n "${RS2_EXTRA_MOUNT:-}" ]]; then
  test -e "${RS2_EXTRA_MOUNT%%:*}" || { echo "dd.sh：掛載來源不存在" >&2; exit 1; }
  EXTRA+=(-v "$RS2_EXTRA_MOUNT")
fi
exec timeout "$TMO" docker run --rm --init --name "${RS2_CNAME:-rs2-dyn}-${PURPOSE}-$$" --network none \
  -u "$(id -u):$(id -g)" --memory "${RS2_MEM:-4g}" --cpus "${RS2_CPUS:-4}" --pids-limit 256 \
  --log-opt max-size=10m --log-opt max-file=3 \
  -v "$ORIG:/orig:ro" -v "$ROOT/tools/dos:/tools-dos:ro" -v "$OUT:/out" -v "$ROOT/workspace/dosgolem/workplace/bin:/bin-dyn:ro" \
  "${EXTRA[@]}" -w /out "$IMAGE" "/bin-dyn/$PROG" "$@"
