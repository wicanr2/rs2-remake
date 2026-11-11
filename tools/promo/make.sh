#!/usr/bin/env bash
# RS-2 remake 推廣片（80 秒、1920x1080、60 fps、H.264 加 AAC）：輸入腳本、錄影、合成、驗收一條龍。
# 成品只留本機（含原版美術與原曲，權利屬大宇資訊）：dist-all/<版本>/promo/，中間檔在 workspace/promo/。
# 全程 docker --rm，錄影最多 4 核、合成 2 核，容器名前綴 rs2-promo-，沒有任何 prune 或 rmi。
#
# 用法：make.sh [步驟...]    步驟：replays narrate record compose（不給就全跑）
#   replays  產生輸入腳本（workspace/promo/replays/）
#   record   用發行包的 AppImage 在 Xvfb 裡 x11grab 錄影（workspace/promo/clips/），約 8 分鐘
#   compose  字卡素材、配樂、各段、串接、驗收抽幀
# 錄影前提：dist-all/<版本>/full-local/ 有 AppImage；workspace/dosrun/boss1_kill/trace-s1.jsonl 與
#   workspace/rs2/audio/{A,B}.nuked.pcm、dist-all/<版本>/smoke/android/06-moved.png 已存在。
# 每段用哪一段錄影、從幾秒開始，寫在 cuts.sh；設計 token 在 theme.sh。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

steps=("$@")
[ "${#steps[@]}" -gt 0 ] || steps=(replays narrate record compose)

for f in "$ROOT/workspace/dosrun/boss1_kill/trace-s1.jsonl" "$ROOT/workspace/rs2/audio/A.nuked.pcm" \
         "$ROOT/workspace/rs2/audio/B.nuked.pcm" "$ROOT/dist-all/$VERSION/smoke/android/06-moved.png" \
         "$DIST/RS-2-remake-$VERSION-full-linux-x86_64.AppImage"; do
  [ -f "$f" ] || die "缺素材：$f"
done
ensure_dir "$WP" "$WP/replays" "$WP/clips" "$OUT_DIR"

for s in "${steps[@]}"; do
  case "$s" in
    replays)
      "$PROMO_TOOLS/gen_replays.sh"
      cp -f "$ROOT/workspace/dosrun/boss1_kill/trace-s1.jsonl" "$WP/replays/boss1_kill.jsonl"   # 原版實跑收據的真人輸入
      ;;
    record)
      # 兩個顯示器同時錄會丟幀，所以對照的兩半各錄一次（sbshd、sbsorig）。
      "$PROMO_TOOLS/record.sh" title s1modes sbshd sbsorig wide150 flow boss posterclassic postermodern
      ;;
    narrate)
      bash "$PROMO_TOOLS/narrate.sh"
      ;;
    compose)
      "$PROMO_TOOLS/compose.sh" assets audio
      "$PROMO_TOOLS/compose.sh" narration
      "$PROMO_TOOLS/compose.sh" segs final review verify
      ;;
    *) die "不認得的步驟 $s（replays narrate record compose）" ;;
  esac
done
say "完成：$FINAL"
