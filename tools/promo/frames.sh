#!/usr/bin/env bash
# 從 workspace/promo/ 內的影片抽幀到 workspace/promo/<輸出資料夾>/，檔名是 f<時間×每秒張數，四位數>.png。
# 用法：frames.sh 影片（相對於 workspace/promo）輸出資料夾 起點秒 長度秒 每秒張數 縮圖寬
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
IN="${1:?影片}"; OUTD="${2:?輸出資料夾}"; SS="${3:-0}"; LEN="${4:-10}"; R="${5:-1}"; TW="${6:-480}"
need_image "$IMG_MIX"
[ -f "$WP/$IN" ] || die "沒有 $WP/$IN"
ensure_dir "$WP/$OUTD"
SN="$(awk -v s="$SS" -v r="$R" 'BEGIN{printf "%d", s*r}')"
m=()
mount_dir m "$WP" /w rw
DK_CPUS=1 dk "frames" 300 "$IMG_MIX" --entrypoint sh --tmpfs /tmp:rw,exec,size=256m,mode=1777 "${m[@]}" -- \
  -c "ffmpeg -nostdin -hide_banner -loglevel error -y -ss $SS -t $LEN -i '/w/$IN' -vf 'fps=$R,scale=$TW:-1' -start_number $SN '/w/$OUTD/f%04d.png'"
ls "$WP/$OUTD" | wc -l
