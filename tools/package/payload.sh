#!/usr/bin/env bash
# 步驟：組出發行包的資料（原版檔案 47 個（彩色 26、單色 21）加 HD 執行時素材）到 workspace/pkg/payload。來源唯讀掛載。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

need_image "$IMG_CROSS"
ensure_dir "$PKG"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$ORIG" /orig ro
mount_dir m "$HD_SRC" /hd ro
mount_dir m "$PKG/src" /src ro
mount_dir m "$PKG" /pkg rw

say "payload：組資料"
dk payload 900 "$IMG_CROSS" --memory 1g --cpus 1 "${m[@]}" -- python3 -I /pkgtools/pkg_payload.py /orig /hd /src /pkg/payload
