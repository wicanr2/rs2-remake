#!/usr/bin/env bash
# README 用的畫面，從實際發行包的 Linux AppImage 截取，不用開發機的二進位檔。
# 乾淨容器（同 verify_linux.sh）、Xvfb、軟體繪圖、SDL_AUDIODRIVER=disk、發行目錄唯讀。
# 輸出在 workspace/readme-shots/<版本>/，挑好的圖再由人複製到 docs/images/game/（原版衍生圖，見 AGENTS.md 硬規則 4）。
# 用法：readme_shots.sh <版本>
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:?版本，例如 v.0.9.0-20261106}"; check_version "$VERSION"

need_image "$IMG_VERIFY"
DIST="$ROOT/dist-all/$VERSION/full-local"
[ -f "$DIST/RS-2-remake-$VERSION-full-linux-x86_64.AppImage" ] || die "缺 AppImage：$DIST"
OUT="$WP/readme-shots/$VERSION"
rm -rf "$OUT"
ensure_dir "$WP/readme-shots" "$OUT"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$DIST" /dist ro
mount_dir m "$OUT" /out rw

say "readme_shots：在乾淨容器跑 AppImage 截圖"
dk readme-shots 900 "$IMG_VERIFY" --entrypoint sh --memory 4g --cpus 4 --tmpfs /tmp:rw,exec,size=2g,mode=1777 -w /tmp "${m[@]}" \
  -e "VERSION=$VERSION" -- /pkgtools/readme_shots_inner.sh 2>&1 | tee "$OUT/shots.log"
