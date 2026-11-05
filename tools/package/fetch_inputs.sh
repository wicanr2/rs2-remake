#!/usr/bin/env bash
# 步驟：把外部輸入（SDL2 原始碼、AppImage runtime 與授權）複製到 workspace/pkg/inputs 並驗 SHA-256。
# 來源 ~/cht/dosgolem-rust/third_party 以唯讀掛載，不改它。已存在且雜湊正確就跳過複製。
# 容器：沒有網路。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

need_image "$IMG_CROSS"
ensure_dir "$INPUTS"
m=()
mount_dir m "$SIBLING_THIRD_PARTY" /src ro
mount_dir m "$INPUTS" /inputs rw
mount_file m "$PKG_TOOLS/inputs.lock" /inputs.lock ro

say "fetch_inputs：複製並驗證外部輸入"
dk fetch 300 "$IMG_CROSS" --memory 1g --cpus 1 "${m[@]}" -- sh -eu -c '
  mkdir -p /inputs/appimage-runtime
  cp -n /src/SDL2-2.26.5/SDL2-2.26.5.tar.gz /inputs/
  cp -n /src/appimage-runtime-75849dc/runtime-x86_64 /inputs/appimage-runtime/
  mkdir -p /inputs/appimage-runtime/licenses
  cp -n /src/appimage-runtime-75849dc/licenses/* /inputs/appimage-runtime/licenses/
  cd /inputs
  grep -v "^#" /inputs.lock | sha256sum -c -
'
