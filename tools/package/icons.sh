#!/usr/bin/env bash
# 步驟：產生圖示（PNG、ICO、ICNS）與 Windows 資源檔。自己畫的簡單圖案，不用原版美術。
# 用法：icons.sh <版本>
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"

need_image "$IMG_CROSS"
ensure_dir "$PKG/icons"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$PKG/icons" /out rw

say "icons：產生圖示"
dk icons 300 "$IMG_CROSS" --memory 1g --cpus 1 "${m[@]}" -- python3 -I /pkgtools/pkg_icons.py /out "$VERSION"
