#!/usr/bin/env bash
# 重算 workspace/promo/clips/ 內錄影的 .t0（第一個遊戲畫面的時間），不重錄。
# 用法：retime.sh 錄影名稱:略過次數 ...   例：retime.sh title-1:1 sbsorig-1:0（HD 版略過 1，--no-hd 略過 0）
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
[ "$#" -ge 1 ] || die "用法：retime.sh 錄影名稱:略過次數..."
need_image "$IMG_MIX"
m=()
mount_dir m "$PROMO_TOOLS/in_container" /tools ro
mount_dir m "$WP/clips" /c rw
for spec in "$@"; do
  n="${spec%%:*}"; s="${spec##*:}"
  [ -f "$WP/clips/$n.mp4" ] || die "沒有 $WP/clips/$n.mp4"
  DK_CPUS=1 dk "retime" 120 "$IMG_MIX" --entrypoint sh --tmpfs /tmp:rw,size=64m,mode=1777 "${m[@]}" -- \
    -c "t=\$(/tools/t0.sh /c/$n.mp4 $s); echo \"\$t\" > /c/$n.t0; echo \"$n t0=\$t\""
done
