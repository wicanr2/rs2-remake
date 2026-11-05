#!/usr/bin/env bash
# 步驟：Linux 驗證。驗 dist-all/<版本>/full 裡的 AppImage 本身（rulebook/82），不是開發機的二進位檔。
# 乾淨容器（Ubuntu 24.04，沒有專案目錄、沒有 SDL2、沒有 --orig）、Xvfb、SDL_AUDIODRIVER=disk、發行目錄唯讀。
# 用法：verify_linux.sh <版本>
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"
IMG_V="$IMG_VERIFY"

need_image "$IMG_V"
DIST="$ROOT/dist-all/$VERSION/full"
OUT="$ROOT/dist-all/$VERSION/verify/linux"
rm -rf "$OUT"
ensure_dir "$ROOT/dist-all/$VERSION/verify" "$OUT"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$DIST" /dist ro
mount_dir m "$PKG/stage/$VERSION/linux/AppDir" /appdir ro
mount_dir m "$OUT" /out rw

say "verify_linux：在乾淨容器跑 AppImage（約 3 分鐘）"
dk verify-linux 900 "$IMG_V" --entrypoint sh --memory 4g --cpus 4 --tmpfs /tmp:rw,exec,size=2g,mode=1777 -w /tmp "${m[@]}" \
  -e "VERSION=$VERSION" -- /pkgtools/verify_linux_inner.sh 2>&1 | tee "$OUT/verify.log"
