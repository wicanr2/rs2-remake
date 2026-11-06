#!/usr/bin/env bash
# 列出錄影中大幅換畫面的時間（秒），用來量顯示模式切換、載入畫面等時間點。
# 用法：scenes.sh 錄影名稱（clips/ 內，不含 .mp4）[起點秒 [長度秒 [門檻]]]
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
N="${1:?錄影名稱}"; SS="${2:-0}"; LEN="${3:-30}"; TH="${4:-0.2}"
need_image "$IMG_MIX"
[ -f "$WP/clips/$N.mp4" ] || die "沒有 $WP/clips/$N.mp4"
m=()
mount_dir m "$WP/clips" /c ro
DK_CPUS=1 dk "scenes" 300 "$IMG_MIX" --entrypoint sh --tmpfs /tmp:rw,size=64m,mode=1777 "${m[@]}" -- \
  -c "ffmpeg -nostdin -hide_banner -ss $SS -t $LEN -i /c/$N.mp4 -an -vf \"select='gt(scene,$TH)',showinfo\" -f null - 2>&1 | awk '/pts_time:/{for(i=1;i<=NF;i++) if(\$i ~ /^pts_time:/){split(\$i,a,\":\"); printf \"%.2f\n\", a[2] + $SS}}'"
