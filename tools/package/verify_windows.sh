#!/usr/bin/env bash
# 步驟：Windows 驗證。用 Wine 9.0 跑 dist-all/<版本>/full 裡 zip 解開的 rs2-play.exe（rulebook/82），
# 乾淨容器、Xvfb、SDL_AUDIODRIVER=disk、解開位置唯讀、沒有 --orig。Wine 不是 Windows，只能證明 PE 能載入與執行、
# 路徑邏輯對；真機行為（Direct3D、主控台視窗等）沒有驗到。
# 用法：verify_windows.sh <版本>
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"
IMG_V="$IMG_VERIFY"

need_image "$IMG_V"
DIST="$ROOT/dist-all/$VERSION/full"
OUT="$ROOT/dist-all/$VERSION/verify/windows"
rm -rf "$OUT"
ensure_dir "$ROOT/dist-all/$VERSION/verify" "$OUT"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$DIST" /dist ro
mount_dir m "$OUT" /out rw

say "verify_windows：Wine 跑 exe（約 4 分鐘）"
dk verify-win 1200 "$IMG_V" --entrypoint sh --memory 4g --cpus 4 --tmpfs /tmp:rw,exec,size=3g,mode=1777 -w /tmp "${m[@]}" \
  -e "VERSION=$VERSION" -- /pkgtools/verify_windows_inner.sh 2>&1 | tee "$OUT/verify.log"
