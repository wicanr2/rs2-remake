#!/usr/bin/env bash
# 步驟：組裝三個平台的發行包內容（AppDir、Windows zip、macOS zip）。AppImage 的 squashfs 另在 appimage.sh 做。
# 用法：pack.sh <版本>
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"

need_image "$IMG_CROSS"
STAGE="$PKG/stage/$VERSION"
rm -rf "$STAGE"
ensure_dir "$STAGE"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$PKG/payload" /payload ro
mount_dir m "$PKG/icons" /icons ro
mount_dir m "$PKG/notices" /notices ro
mount_dir m "$INPUTS" /inputs ro
mount_dir m "$PKG/cargo-target/linux/release" /bins/linux ro
mount_dir m "$PKG/cache/sdl2-linux/lib" /bins/linux-sdl ro
mount_dir m "$PKG/cargo-target/windows" /bins/windows ro
mount_dir m "$PKG/cargo-target/macos" /bins/macos ro
mount_file m "$ROOT/LICENSE" /LICENSE ro
mount_dir m "$PKG/src" /src ro
mount_dir m "$STAGE" /out rw

say "pack：組裝內容（$VERSION）"
dk pack 1800 "$IMG_CROSS" --memory 2g --cpus 2 "${m[@]}" -- python3 -I /pkgtools/pkg_assemble.py "$VERSION"
