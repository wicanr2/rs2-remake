#!/usr/bin/env bash
# 步驟：把產物收進 dist-all/<版本>/full-local/，寫 LICENSE、README.txt、SHA256SUMS、manifest.json。
# 用法：finalize.sh <版本>
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"

need_image "$IMG_CROSS"
STAGE="$PKG/stage/$VERSION"
[ -f "$STAGE/linux/RS-2-remake-$VERSION-full-linux-x86_64.AppImage" ] || die "先跑 appimage.sh"
DIST="$ROOT/dist-all/$VERSION/full-local"
ensure_dir "$ROOT/dist-all" "$ROOT/dist-all/$VERSION"
ensure_dir "$DIST"

images="{"
for k in IMG_CROSS IMG_LINUX IMG_SQUASH IMG_VERIFY; do
  n="${!k}"
  images+="\"$n\":\"$(docker image inspect --format '{{.Id}}' "$n")\","
done
images="${images%,}}"

m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$STAGE" /stage ro
mount_dir m "$PKG/payload" /payload ro
mount_dir m "$PKG/notices" /notices ro
mount_dir m "$PKG/src" /src ro
mount_file m "$ROOT/LICENSE" /LICENSE ro
mount_dir m "$DIST" /dist rw

say "finalize：收進 dist-all/$VERSION/full-local"
dk finalize 600 "$IMG_CROSS" --memory 1g --cpus 1 "${m[@]}" -e "RS2_IMAGES=$images" -- python3 -I /pkgtools/pkg_finalize.py "$VERSION"
ls -l "$DIST"
