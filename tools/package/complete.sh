#!/usr/bin/env bash
# 四平台與有旁白影片都驗完後，建立唯一交付索引。不建 tag、不發布、不上傳。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"
DIST="$ROOT/dist-all/$VERSION"
PROMO="$WP/promo/$VERSION"
need_image "$IMG_LINUX"
[ -d "$DIST/promo" ] || die "先產生推廣影片"
[ -d "$PROMO/narration" ] || die "先產生旁白"
"$PKG_TOOLS/finalize.sh" "$VERSION"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$ROOT/tools/promo/in_container" /ptools ro
mount_dir m "$PROMO" /w ro
mount_dir m "$DIST" "/release/$VERSION" rw
dk release-complete 360 "$IMG_LINUX" --memory 2g --cpus 2 "${m[@]}" -e "VERSION=$VERSION" -- sh -ec '
  python3 /ptools/media_verify.py "/release/$VERSION/promo/RS-2-remake-promo-$VERSION.mp4" /w/narration "/release/$VERSION/promo"
  python3 /pkgtools/release_manifest.py "/release/$VERSION" "$VERSION" /w
'
check_no_root_files
