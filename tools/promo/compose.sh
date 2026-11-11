#!/usr/bin/env bash
# 合成：字卡與字幕素材、配樂、各段影片、串接成片、驗收抽幀。全在 docker 內，CPU 上限 2。
# 用法：[SEGS="02 07a"] compose.sh 階段...    階段：assets audio segs final review verify
# SEGS 只重做指定的段（說明見 in_container/compose_inner.sh）。
# 成品：dist-all/<版本>/promo/RS-2-remake-promo-<版本>.mp4；中間檔在 workspace/promo/{assets,segs,review}。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
[ "$#" -ge 1 ] || die "用法：compose.sh 階段..."
need_image "$IMG_MIX"
ensure_dir "$WP" "$WP/clips" "$OUT_DIR"
m=()
mount_dir m "$WP" /w rw
mount_dir m "$PROMO_TOOLS" /ptools ro
mount_dir m "$ROOT/workspace/rs2/audio" /rsaudio ro
mount_dir m "$ROOT/dist-all/$VERSION/smoke/android" /android ro
mount_dir m "$OUT_DIR" /outd rw
if [ "$*" = narration ]; then
  need_image "$IMG_REC"
  DK_CPUS=2 DK_MEM=2g dk "narration-mix" 300 "$IMG_REC" "${m[@]}" -- \
    python3 /ptools/in_container/narration_mix.py /w/narration /w/music.wav
else
  DK_CPUS=2 DK_MEM=4g dk "compose" "${COMPOSE_TIMEOUT:-1200}" "$IMG_MIX" --entrypoint bash --tmpfs /tmp:rw,exec,size=1g,mode=1777 -w /tmp "${m[@]}" \
    -e "VERSION=$VERSION" -e "SEGS=${SEGS:-}" -- /ptools/in_container/compose_inner.sh "$@"
fi
check_no_root_files
