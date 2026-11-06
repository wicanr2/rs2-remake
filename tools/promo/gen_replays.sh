#!/usr/bin/env bash
# 產生推廣片用的輸入腳本到 workspace/promo/replays/：stage1-move.jsonl、flow-play.jsonl。
# 可用環境變數覆寫 G1 L1 GAP2 L2 GAP3 L3 GAP4 L4 TAIL STAGE1_STEPS，說明見 in_container/gen_replays.sh。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
need_image "$IMG_MIX"
ensure_dir "$WP" "$WP/replays"
m=()
mount_dir m "$PROMO_TOOLS/in_container" /tools ro
mount_dir m "$WP/replays" /out rw
e=()
for v in G1 L1 GAP2 L2 GAP3 L3 GAP4 L4 TAIL STAGE1_STEPS; do
  [ -z "${!v:-}" ] || e+=(-e "$v=${!v}")
done
DK_CPUS=1 dk "genrep" 120 "$IMG_MIX" --entrypoint sh --tmpfs /tmp:rw,exec,size=64m,mode=1777 "${m[@]}" "${e[@]}" -- /tools/gen_replays.sh
