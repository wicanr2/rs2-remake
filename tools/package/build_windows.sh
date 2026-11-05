#!/usr/bin/env bash
# 步驟：Windows x86_64（x86_64-pc-windows-gnu，SDL2 用映像內的官方 mingw 開發包 /opt/sdl2/windows）。
# 產物：workspace/pkg/cargo-target/windows/x86_64-pc-windows-gnu/release/rs2-play.exe
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"

need_image "$IMG_CROSS"
ensure_dir "$PKG/cargo-target/windows" "$PKG/logs"
test -f "$PKG/icons/rs2.rc" || die "先跑 icons.sh"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$PKG/src" /project ro
mount_dir m "$PKG_CARGO" /cache rw
mount_dir m "$PKG/cargo-target/windows" /target rw
mount_dir m "$PKG/icons" /icons ro

say "build_windows：cargo build（x86_64-pc-windows-gnu）"
dk build-win 2400 "$IMG_CROSS" "${m[@]}" --tmpfs /tmp:rw,exec,size=512m \
  -e CARGO_HOME=/cache -e CARGO_TARGET_DIR=/target -e HOME=/tmp -e "RS2_VERSION=$VERSION" -- \
  sh /pkgtools/build_rs2_windows.sh 2>&1 | tee "$PKG/logs/build-windows.log"
test -f "$PKG/cargo-target/windows/x86_64-pc-windows-gnu/release/rs2-play.exe" || die "沒有產生 Windows 執行檔"
