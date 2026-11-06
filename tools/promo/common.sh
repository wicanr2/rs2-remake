#!/usr/bin/env bash
# 推廣影片腳本共用的變數與 docker 包裝。由 make.sh 與各步驟腳本 source。
#
# 規則（AGENTS.md 硬規則 1 到 3 與使用者對本片的邊界）：
#   - 錄影與合成都在 docker 內，一律 docker run --rm，帶目前 UID/GID、--memory、--cpus、--pids-limit、
#     log 輪替、--network none，外層 timeout。容器名稱前綴 rs2-promo-。
#   - 不 prune、不 rmi、不 commit、不建新映像，只清自己建立的容器（--rm）。
#   - 每個 -v 掛載前確認 host 來源存在且形態正確；原始輸入唯讀掛載。
#   - 成品含原版美術與原曲，只留本機，不進版控（dist-all/<版本>/promo/ 與 workspace/promo/）。
set -euo pipefail

PROMO_TOOLS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$PROMO_TOOLS/../.." && pwd)"
WP="$ROOT/workspace/promo"
VERSION="${VERSION:-v.1.0.0-20261106}"
DIST="$ROOT/dist-all/$VERSION/full"
OUT_DIR="$ROOT/dist-all/$VERSION/promo"
FINAL="$OUT_DIR/RS-2-remake-promo-$VERSION.mp4"

# 映像（本機既有，唯讀使用）。
IMG_REC="mm45-rust-desktop:1.99.1-r3"   # Xvfb、xdotool、帶 x11grab 的 ffmpeg，可跑 AppImage
IMG_MIX="kgb-cht-video:1"               # ffmpeg、ImageMagick、Noto Sans CJK 全字重

die() { echo "promo: $*" >&2; exit 1; }
say() { echo "[$(date +%H:%M:%S)] $*" >&2; }

need_image() {
  docker image inspect "$1" >/dev/null 2>&1 || die "找不到映像 $1（本機既有映像，腳本不下載也不重建）"
}

# mount_dir 陣列名 host目錄 容器路徑 [ro|rw]：host 來源不存在就中止，不讓 dockerd 以 root 建空目錄頂替。
mount_dir() {
  local -n _arr="$1"
  local src="$2" dst="$3" mode="${4:-ro}"
  [ -d "$src" ] || die "掛載來源不是目錄或不存在：$src"
  _arr+=(-v "$src:$dst:$mode")
}

ensure_dir() {
  mkdir -p "$@"
  local d
  for d in "$@"; do
    [ "$(stat -c %u "$d")" = "$(id -u)" ] || die "$d 不是目前使用者所有（可能被 dockerd 以 root 建立）"
  done
}

# dk 名稱 逾時秒數 映像 [docker run 選項...] -- 指令...
# 預設 2 核、4g 記憶體；DK_CPUS 可調（錄影最多 4、合成 2），DK_MEM 可調。
dk() {
  local name="$1" tmo="$2" image="$3"
  shift 3
  local opts=()
  while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do
    opts+=("$1")
    shift
  done
  [ "${1:-}" = "--" ] || die "dk：缺少 --"
  shift
  local cpus="${DK_CPUS:-2}"
  [ "$cpus" -le 4 ] || die "CPU 上限是 4"
  timeout --kill-after=30 "$tmo" docker run --rm --init \
    --name "rs2-promo-${name}-$$" \
    -u "$(id -u):$(id -g)" \
    --memory "${DK_MEM:-4g}" --cpus "$cpus" --pids-limit "${DK_PIDS:-512}" \
    --network none \
    --log-opt max-size=10m --log-opt max-file=3 \
    "${opts[@]}" "$image" "$@"
}

# 收尾自檢：工作根底下不該有 root 擁有的東西。只列出，不動。
check_no_root_files() {
  local n
  n="$(find "$WP" "$OUT_DIR" -user root 2>/dev/null | head -5)"
  [ -z "$n" ] || die "發現 root 擁有的檔案：$n"
}
