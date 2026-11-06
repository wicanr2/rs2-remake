#!/usr/bin/env bash
# 完整版發行包腳本共用的變數與 docker 包裝。由 build_all.sh 與各步驟腳本 source。
#
# 規則（AGENTS.md 硬規則 1 到 3）：每一步都在 docker 內跑，一律 --rm，帶目前 UID/GID、--memory、--cpus、
# --pids-limit、log 輪替與外層 timeout，預設 --network none。容器名稱前綴 rs2-pkg-。
# 不碰共用 docker 資源：沒有任何 prune、rmi，也不 rm 別人的容器。
# 每個 -v 掛載前都確認 host 來源存在且形態正確（mount_dir、mount_file）。
set -euo pipefail

PKG_TOOLS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$PKG_TOOLS/../.." && pwd)"
WP="$ROOT/workspace"
PKG="$WP/pkg"
PKG_CARGO="$WP/cargo-play"              # 離線 crate 快取（專案自己的）
ORIG="$ROOT/original/rs-2"
HD_SRC="$WP/hd/assets"
INPUTS="$PKG/inputs"
SIBLING_THIRD_PARTY="$HOME/cht/dosgolem-rust/third_party"   # 唯讀來源：SDL2 原始碼與 AppImage runtime

# 映像（全是本機既有，唯讀使用）。
IMG_CROSS="dosgolem-rust-cross:1.99.1-sdk15.5-r1"   # Rust、mingw、osxcross、SDL2 Windows 與 macOS 開發包
IMG_LINUX="mm45-rust-desktop:1.99.1-r3"             # Debian 12（glibc 2.36）、Rust 1.99.1、SDL2 編譯所需標頭
IMG_SQUASH="hr-appimage:runtime-recovery-r1"        # mksquashfs 4.5.1
IMG_VERIFY="yuan-wine-verify:noble-r2"               # Ubuntu 24.04：Xvfb、Wine 9.0、ImageMagick、python3；沒有 SDL2 與專案檔案

DEFAULT_VERSION="v.1.0.0-20261106"

die() { echo "build: $*" >&2; exit 1; }
say() { echo "[$(date +%H:%M:%S)] $*" >&2; }

check_version() {
  [[ "$1" =~ ^v\.[0-9]+\.[0-9]+\.[0-9]+-[0-9]{8}$ ]] || die "版本格式要像 v.0.9.0-20261105，收到 $1"
}

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

mount_file() {
  local -n _arr="$1"
  local src="$2" dst="$3" mode="${4:-ro}"
  [ -f "$src" ] || die "掛載來源不是檔案或不存在：$src"
  _arr+=(-v "$src:$dst:$mode")
}

# dk 名稱 逾時秒數 映像 [docker run 選項...] -- 指令...
# 預設 4 核、6g 記憶體；DK_CPUS、DK_MEM 可調（CPU 上限 4）。
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
  local cpus="${DK_CPUS:-4}"
  [ "$cpus" -le 4 ] || die "CPU 上限是 4"
  timeout --kill-after=30 "$tmo" docker run --rm --init \
    --name "rs2-pkg-${name}-$$" \
    -u "$(id -u):$(id -g)" \
    --memory "${DK_MEM:-6g}" --cpus "$cpus" --pids-limit "${DK_PIDS:-1024}" \
    --network none \
    --log-opt max-size=10m --log-opt max-file=3 \
    "${opts[@]}" "$image" "$@"
}

# 建立並確認工作目錄是自己的，不是 dockerd 以 root 建出來的。
ensure_dir() {
  mkdir -p "$@"
  local d
  for d in "$@"; do
    [ "$(stat -c %u "$d")" = "$(id -u)" ] || die "$d 不是目前使用者所有（可能被 dockerd 以 root 建立）"
  done
}

# 收尾自檢：工作根底下不該有 root 擁有的東西。只列出，不動。
check_no_root_files() {
  local n
  # android/ 是別的腳本的工作目錄，不在這裡查。
  n="$(find "$PKG" -path "$PKG/android" -prune -o -user root -print 2>/dev/null | head -5; find "$ROOT/dist-all" -user root 2>/dev/null | head -5)"
  [ -z "$n" ] || die "發現 root 擁有的檔案：$n"
}
