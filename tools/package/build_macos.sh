#!/usr/bin/env bash
# 步驟：macOS（x86_64 加 arm64，lipo 成 universal）。工具鏈是映像內的 osxcross（SDK 15.5）。
# 產物：workspace/pkg/cargo-target/macos/universal/rs2-play
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"

need_image "$IMG_CROSS"
ensure_dir "$PKG/cargo-target/macos" "$PKG/cargo-target/macos/universal" "$PKG/logs"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$PKG/src" /project ro
mount_dir m "$PKG_CARGO" /cache rw
mount_dir m "$PKG/cargo-target/macos" /target rw
mount_dir m "$PKG/cargo-target/macos/universal" /out rw

say "build_macos：cargo build（x86_64 與 arm64）加 lipo"
dk build-mac 2400 "$IMG_CROSS" "${m[@]}" --tmpfs /tmp:rw,exec,size=512m \
  -e CARGO_HOME=/cache -e CARGO_TARGET_DIR=/target -e HOME=/tmp -e "RS2_VERSION=$VERSION" -- \
  sh /pkgtools/build_rs2_macos.sh 2>&1 | tee "$PKG/logs/build-macos.log"
test -f "$PKG/cargo-target/macos/universal/rs2-play" || die "沒有產生 macOS universal 執行檔"
