#!/usr/bin/env bash
# agy CLI 生成的腳本已審查並保存在 in_container/narration_generate.py。
# 此步驟只把已定旁白稿送到 Microsoft Edge TTS，沒有主機憑證或其他掛載。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
IMG_VOICE="rich2-narration:edge-tts7.3.1-20261101-r1"
need_image "$IMG_VOICE"
ensure_dir "$WP" "$WP/narration"
cp "$PROMO_TOOLS/narration.json" "$WP/narration/plan.json"
cp "$PROMO_TOOLS/in_container/narration_generate.py" "$WP/narration/generate.py"
m=()
mount_dir m "$WP/narration" /out rw
DK_NETWORK=bridge DK_CPUS=1 DK_MEM=1g dk narrate 1500 "$IMG_VOICE" "${m[@]}" -w /out -- python3 /out/generate.py
check_no_root_files
