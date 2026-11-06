#!/usr/bin/env bash
# 取樣用：在容器內以 --headless-steps 快速跑遊戲，存指定步數的畫面，不即時錄影。
# 輸出在 workspace/promo/probe/（<名稱>.log 與 --shot-at 指定的 PNG，路徑用 /out/<名稱>/xxx.png）。
# 用法：probe.sh 名稱 "設定行;以分號分隔" 遊戲參數...
#   輸入腳本放在 workspace/promo/replays/，遊戲參數裡用 /rpl/檔名。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
NAME="${1:?名稱}"; CFG="${2:-}"; shift 2
need_image "$IMG_REC"
[ -f "$DIST/RS-2-remake-$VERSION-full-linux-x86_64.AppImage" ] || die "缺 AppImage：$DIST"
ensure_dir "$WP" "$WP/probe" "$WP/replays"
m=()
mount_dir m "$PROMO_TOOLS/in_container" /tools ro
mount_dir m "$DIST" /dist ro
mount_dir m "$WP/replays" /rpl ro
mount_dir m "$WP/probe" /out rw
DK_CPUS="${DK_CPUS:-2}" dk "probe-$NAME" "${PROBE_TIMEOUT:-600}" "$IMG_REC" --entrypoint sh --tmpfs /tmp:rw,exec,size=2g,mode=1777 -w /tmp "${m[@]}" \
  -e "VERSION=$VERSION" -- /tools/probe_inner.sh "$NAME" "$CFG" "$@"
