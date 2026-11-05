#!/usr/bin/env bash
# 推送前的外洩閘門，掃「會進版控的檔案」（已追蹤加未追蹤且未被忽略）。只呼叫 tools/leak_check.py，不改它的規則。
# dist-all/、workspace/ 已在 .gitignore，不在清單內。用法：leak_check.sh [--selftest]
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

need_image "$IMG_CROSS"
m=()
mount_dir m "$ROOT" /repo ro
mount_dir m "$ORIG" /orig ro
mount_file m "$WP/unpacked/S.unpacked.bin" /unp/S.unpacked.bin ro
if [ "${1:-}" = "--selftest" ]; then
  dk leak 600 "$IMG_CROSS" --memory 1g --cpus 1 "${m[@]}" -- python3 -I /repo/tools/leak_check.py --selftest /orig /unp/S.unpacked.bin
else
  git -C "$ROOT" ls-files -co --exclude-standard | dk leak 600 "$IMG_CROSS" -i --memory 1g --cpus 1 "${m[@]}" -- \
    python3 -I /repo/tools/leak_check.py /repo /orig /unp/S.unpacked.bin
fi
