#!/bin/bash
# 容器內：在 Xvfb 裡跑發行包的 AppImage，用 x11grab 即時錄影（60 fps、veryfast、crf 14）。
# 環境由 record.sh 帶入：/dist（唯讀，AppImage）、/rpl（唯讀，輸入腳本）、/tools（唯讀）、/out（可寫）、VERSION。
# 用法：record_inner.sh 工作名稱   （工作定義在 /tools/jobs/<名稱>.sh）
#
# 工作檔變數：
#   SECS    錄影秒數（從 ffmpeg 起算，含 AppImage 解開與載入的黑畫面）
#   NDISP   顯示器數，1 或 2。每個顯示器一個 Xvfb 與一個遊戲視窗，由同一個 ffmpeg 各錄一支檔
#   W H     每個顯示器（也是視窗）的大小
#   CFG1 ARGS1 [CFG2 ARGS2]   settings.txt 內容（以分號分隔）與遊戲參數
#   post_launch()             視窗出現後呼叫，可用 keys 排程按鍵
# 輸出：/out/<名稱>-<n>.mp4 與 /out/<名稱>-<n>.t0（第一個非黑畫面的時間，秒）
set -u
JOB="${1:?工作名稱}"
. "/tools/jobs/$JOB.sh"
NAME="${NAME:-$JOB}"
AI="/dist/RS-2-remake-$VERSION-full-linux-x86_64.AppImage"
[ -f "$AI" ] || { echo "FAIL 缺 AppImage $AI"; exit 1; }
export SDL_RENDER_DRIVER=software SDL_VIDEODRIVER=x11 SDL_AUDIODRIVER=dummy
export APPIMAGE_EXTRACT_AND_RUN=1 XDG_CACHE_HOME=/tmp/cache
mkdir -p -m 1777 /tmp/.X11-unix /tmp/cache 2>/dev/null || true
cd /tmp

PIDS=""
cleanup() { [ -z "$PIDS" ] || kill $PIDS 2>/dev/null || true; }
trap cleanup EXIT

# 顯示器 :99、:100。
for d in $(seq 1 "$NDISP"); do
  n=$((98 + d))
  Xvfb ":$n" -screen 0 "${W}x${H}x24" -nolisten tcp >"/tmp/xvfb$d.log" 2>&1 &
  PIDS="$PIDS $!"
done
for d in $(seq 1 "$NDISP"); do
  n=$((98 + d)); i=0
  while [ ! -S "/tmp/.X11-unix/X$n" ] && [ "$i" -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
  [ -S "/tmp/.X11-unix/X$n" ] || { echo "FAIL xvfb :$n 沒有起來"; cat "/tmp/xvfb$d.log"; exit 1; }
done

# keys 秒數 鍵名 [秒數 鍵名 ...]：視窗出現後起算，各自再等指定的秒數按一次鍵（XTEST，顯示器 :99）。
keys() {
  (
    export DISPLAY=:99
    xdotool mousemove $((W / 2)) $((H / 2)) 2>/dev/null
    t_prev=0
    while [ "$#" -ge 2 ]; do
      t="$1"; k="$2"; shift 2
      sleep "$(awk -v a="$t" -v b="$t_prev" 'BEGIN{printf "%.3f", a - b}')"
      t_prev="$t"
      xdotool key "$k" && echo "[keys] $t s：$k"
    done
  ) &
  PIDS="$PIDS $!"
}

# 起遊戲。每個實例用自己的 TMPDIR，避免 --appimage-extract-and-run 的解開目錄互相覆蓋。
for d in $(seq 1 "$NDISP"); do
  eval "cfg=\${CFG$d}; args=\${ARGS$d}"
  mkdir -p "/tmp/cfg$d" "/tmp/home$d" "/tmp/tmp$d"
  printf '%s\n' "$cfg" | tr ';' '\n' > "/tmp/cfg$d/settings.txt"
  # shellcheck disable=SC2086
  DISPLAY=":$((98 + d))" HOME="/tmp/home$d" TMPDIR="/tmp/tmp$d" \
    "$AI" --appimage-extract-and-run --config "/tmp/cfg$d" $args --max-secs "$((SECS + 10))" >"/out/$NAME-$d.log" 2>&1 &
  PIDS="$PIDS $!"
done

# 錄影從遊戲啟動就開始，開頭的黑畫面之後用 .t0 修掉。
inputs=""; maps=""
for d in $(seq 1 "$NDISP"); do
  inputs="$inputs -f x11grab -framerate 60 -draw_mouse 0 -video_size ${W}x${H} -i :$((98 + d))"
  maps="$maps -map $((d - 1)):v -c:v libx264 -preset veryfast -crf 14 -pix_fmt yuv420p -g 60 -threads 2 -t $SECS /out/$NAME-$d.mp4"
done
# shellcheck disable=SC2086
ffmpeg -nostdin -hide_banner -loglevel warning -y $inputs $maps >"/out/$NAME-ffmpeg.log" 2>&1 &
FF=$!

# 等第一個視窗出現，再排按鍵。
i=0
while [ "$i" -lt 600 ]; do
  if DISPLAY=:99 xdotool search --name "RS-2 remake" >/dev/null 2>&1; then break; fi
  sleep 0.1; i=$((i + 1))
done
echo "[record] 視窗出現，約 $(awk -v i="$i" 'BEGIN{printf "%.1f", i/10}') 秒（錄影起算）"
type post_launch >/dev/null 2>&1 && post_launch
wait "$FF"
cleanup
sleep 1

# 第一個非黑畫面的時間。
for d in $(seq 1 "$NDISP"); do
  f="/out/$NAME-$d.mp4"
  [ -s "$f" ] || { echo "FAIL 沒有 $f"; continue; }
  t0="$(/tools/t0.sh "$f" 0)"
  echo "${t0:-0}" > "/out/$NAME-$d.t0"
  echo "[record] $f：$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$f") 秒，t0=${t0:-0}"
done
