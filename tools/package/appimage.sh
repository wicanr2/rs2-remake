#!/usr/bin/env bash
# 步驟：把 AppDir 壓成 squashfs，接在 AppImage type2 runtime（提交 75849dc，SHA-256 已驗）後面，得到 AppImage。
# 用法：appimage.sh <版本>
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"

need_image "$IMG_SQUASH"
STAGE="$PKG/stage/$VERSION"
[ -d "$STAGE/linux/AppDir" ] || die "先跑 pack.sh"
NAME="RS-2-remake-$VERSION-full-linux-x86_64.AppImage"
rm -f "$STAGE/linux/$NAME"
m=()
mount_dir m "$STAGE/linux" /stage rw
mount_dir m "$INPUTS/appimage-runtime" /runtime ro

say "appimage：mksquashfs 與接上 runtime"
dk appimage 1800 "$IMG_SQUASH" --entrypoint sh --memory 3g --cpus 4 --tmpfs /tmp:rw,exec,size=1g "${m[@]}" -- -eu -c '
  echo "1cc49bcf1e2ccd593c379adb17c9f85a36d619088296504de95b1d06215aebbf  /runtime/runtime-x86_64" | sha256sum -c -
  mksquashfs /stage/AppDir /tmp/rs2.squashfs -noappend -comp zstd -b 1048576 -processors 4 \
    -all-time 0 -mkfs-time 0 -root-owned -no-xattrs >/tmp/mksquashfs.log 2>&1 || { tail -30 /tmp/mksquashfs.log; exit 1; }
  tail -3 /tmp/mksquashfs.log
  cat /runtime/runtime-x86_64 /tmp/rs2.squashfs > "/stage/'"$NAME"'"
  chmod 755 "/stage/'"$NAME"'"
  ls -l "/stage/'"$NAME"'"
'
