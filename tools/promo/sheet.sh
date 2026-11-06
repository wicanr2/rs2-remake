#!/usr/bin/env bash
# 把 workspace/promo/ 底下某個資料夾的 PNG 排成一張 contact sheet（含檔名標籤）。
# 用法：sheet.sh 資料夾（相對於 workspace/promo）輸出檔名（放在同一資料夾）[欄數 [縮圖寬]]
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
D="${1:?資料夾（相對於 workspace/promo）}"; OUT="${2:?輸出檔名}"; COLS="${3:-4}"; TW="${4:-480}"
need_image "$IMG_MIX"
ensure_dir "$WP/$D"
m=()
mount_dir m "$WP/$D" /d rw
DK_CPUS=1 dk "sheet" 300 "$IMG_MIX" --entrypoint sh --tmpfs /tmp:rw,exec,size=512m,mode=1777 -w /d "${m[@]}" -- \
  -c "montage -font /usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc -pointsize 18 -label '%f' -tile ${COLS}x -geometry ${TW}x+4+4 -background '#101018' -fill white \$(ls *.png | grep -v -x '$OUT') '$OUT'"
echo "$WP/$D/$OUT"
