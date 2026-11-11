#!/usr/bin/env bash
# 步驟：靜態驗收三個成品（zip CRC、MANIFEST 逐檔雜湊、授權檔、PE 與 Mach-O 結構、arm64 簽章逐頁雜湊、AppImage 結構）。
# 用法：verify_static.sh <版本>
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"

need_image "$IMG_CROSS"
OUT="$ROOT/dist-all/$VERSION/smoke"
ensure_dir "$OUT"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$ROOT/dist-all/$VERSION/full-local" /dist ro
mount_dir m "$PKG/src" /src ro
mount_dir m "$PKG/payload" /payload ro
mount_dir m "$INPUTS" /inputs ro

say "verify_static：靜態驗收"
dk verify-static 900 "$IMG_CROSS" --memory 4g --cpus 2 "${m[@]}" -- python3 -I /pkgtools/pkg_verify_static.py "$VERSION" 2>&1 | tee "$OUT/static.log"
