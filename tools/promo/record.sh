#!/usr/bin/env bash
# 錄影：在容器內用發行包的 Linux AppImage 跑遊戲，x11grab 即時錄成 workspace/promo/clips/<工作>-<n>.mp4。
# 工作定義在 in_container/jobs/<名稱>.sh。用法：record.sh 工作名稱...（可給多個，依序跑）
# 每次只跑一個容器，CPU 上限 4。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
[ "$#" -ge 1 ] || die "用法：record.sh 工作名稱..."
need_image "$IMG_REC"
[ -f "$DIST/RS-2-remake-$VERSION-full-linux-x86_64.AppImage" ] || die "缺 AppImage：$DIST"
ensure_dir "$WP" "$WP/clips" "$WP/replays"
for job in "$@"; do
  [ -f "$PROMO_TOOLS/in_container/jobs/$job.sh" ] || die "沒有工作檔 in_container/jobs/$job.sh"
  say "錄影工作：$job（uptime：$(uptime | sed 's/.*load average: //')）"
  m=()
  mount_dir m "$PROMO_TOOLS/in_container" /tools ro
  mount_dir m "$DIST" /dist ro
  mount_dir m "$WP/replays" /rpl ro
  mount_dir m "$WP/clips" /out rw
  DK_CPUS=4 DK_MEM=6g dk "rec-$job" "${REC_TIMEOUT:-900}" "$IMG_REC" --entrypoint bash --tmpfs /tmp:rw,exec,size=3g,mode=1777 -w /tmp "${m[@]}" \
    -e "VERSION=$VERSION" -- /tools/record_inner.sh "$job"
done
check_no_root_files
