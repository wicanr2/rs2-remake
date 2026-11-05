#!/usr/bin/env bash
# 步驟：第三方授權聲明（crate 清單與授權全文、SDL2 授權）。從快照讀，離線。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

need_image "$IMG_CROSS"
ensure_dir "$PKG/notices"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$PKG/src" /project ro
mount_dir m "$PKG_CARGO" /cache rw
mount_dir m "$INPUTS" /inputs ro
mount_dir m "$PKG/notices" /out rw

say "notices：授權聲明"
dk notices 300 "$IMG_CROSS" --memory 1g --cpus 1 "${m[@]}" -e CARGO_HOME=/cache -e HOME=/tmp -- \
  python3 -I /pkgtools/pkg_notices.py /project/crates/rs2-play/Cargo.toml /inputs/SDL2-2.26.5.tar.gz /out
