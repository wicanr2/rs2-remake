#!/usr/bin/env bash
# 步驟：Linux x86_64。先在 Debian 12 容器內從官方原始碼編 SDL2（結果快取，原始碼雜湊與映像不變就跳過），
# 再建 rs2-play。產物：workspace/pkg/cargo-target/linux/release/rs2-play、workspace/pkg/cache/sdl2-linux/lib。
# 用法：build_linux.sh <版本>
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"

need_image "$IMG_LINUX"
ensure_dir "$PKG/cache/sdl2-linux" "$PKG/cargo-target/linux" "$PKG/logs"

stamp="$(sha256sum "$INPUTS/SDL2-2.26.5.tar.gz" | cut -d' ' -f1) $(docker image inspect --format '{{.Id}}' "$IMG_LINUX") $(sha256sum "$PKG_TOOLS/in_container/build_sdl_linux.sh" | cut -d' ' -f1)"
if [ -f "$PKG/cache/sdl2-linux/.stamp" ] && [ "$(cat "$PKG/cache/sdl2-linux/.stamp")" = "$stamp" ] && [ -e "$PKG/cache/sdl2-linux/lib/libSDL2-2.0.so.0" ]; then
  say "build_linux：SDL2 快取有效，跳過編譯"
else
  say "build_linux：從原始碼編 SDL2 2.26.5"
  rm -rf "$PKG/cache/sdl2-linux"
  ensure_dir "$PKG/cache/sdl2-linux"
  m=()
  mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
  mount_dir m "$INPUTS" /inputs ro
  mount_dir m "$PKG/cache/sdl2-linux" /prefix rw
  dk sdl-linux 1800 "$IMG_LINUX" "${m[@]}" --tmpfs /tmp:rw,exec,size=2g -- sh /pkgtools/build_sdl_linux.sh 2>&1 | tee "$PKG/logs/sdl-linux.log"
  echo "$stamp" > "$PKG/cache/sdl2-linux/.stamp"
fi

say "build_linux：cargo build rs2-play"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$PKG/src" /project ro
mount_dir m "$PKG_CARGO" /cache rw
mount_dir m "$PKG/cargo-target/linux" /target rw
mount_dir m "$PKG/cache/sdl2-linux" /sdl ro
dk build-linux 2400 "$IMG_LINUX" "${m[@]}" \
  -e CARGO_HOME=/cache -e CARGO_TARGET_DIR=/target -e HOME=/tmp -e "RS2_VERSION=$VERSION" -- \
  sh /pkgtools/build_rs2_linux.sh 2>&1 | tee "$PKG/logs/build-linux.log"
test -x "$PKG/cargo-target/linux/release/rs2-play" || die "沒有產生 Linux 執行檔"
