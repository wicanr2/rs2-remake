#!/bin/sh
# 容器內：找錄影裡第一個遊戲畫面的時間（秒）。
# 錄影從遊戲啟動就開始：Xvfb 的黑底，HD 版接著有深藍的「載入中」畫面，載入完才是遊戲。
# 黑底到載入中的差異太小（scene 分數低於門檻），所以第一次 scene > 0.25（0.05 秒之後）就是載入中到遊戲。
# 用法：t0.sh 影片 要略過的次數（預設 0）
f="${1:?影片}"; skip="${2:-0}"
ffmpeg -nostdin -hide_banner -t 8 -i "$f" -an -vf "select='gt(scene,0.25)',showinfo" -f null - 2>&1 \
  | awk -v skip="$skip" '/pts_time:/{
      for (i = 1; i <= NF; i++) if ($i ~ /^pts_time:/) { split($i, a, ":"); t = a[2] + 0 }
      if (t >= 0.05) { if (n >= skip) { print t; exit } n++ }
    }'
