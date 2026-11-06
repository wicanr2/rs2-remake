#!/bin/sh
# 容器內：依 hd_mono_shots.sh 帶入的環境（VERSION、LABELS，/dist、/hd、/replays 唯讀，/out 可寫）逐標籤截圖。
set -u
AI="/dist/RS-2-remake-$VERSION-full-linux-x86_64.AppImage"
export SDL_RENDER_DRIVER=software SDL_VIDEODRIVER=x11 SDL_AUDIODRIVER=dummy
export APPIMAGE_EXTRACT_AND_RUN=1 TMPDIR=/tmp
cd /tmp
mkdir -p -m 1777 /tmp/.X11-unix 2>/dev/null || true
Xvfb :99 -screen 0 2560x1440x24 -nolisten tcp > /tmp/xvfb.log 2>&1 &
i=0
while [ ! -S /tmp/.X11-unix/X99 ] && [ "$i" -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
[ -S /tmp/.X11-unix/X99 ] || { echo "FAIL xvfb 沒有起來"; cat /tmp/xvfb.log; exit 1; }
export DISPLAY=:99

hdarg() {
  case "$1" in
    SD) echo "--no-hd" ;;
    A) echo "--hd /hd/assets" ;;
    *) echo "--hd /hd/pilot-mono-review/root-$1" ;;
  esac
}

for l in $LABELS; do
  H=$(hdarg "$l")
  # 第一關示範錄影
  cfg="/tmp/cfg-$l-s1"; mkdir -p "$cfg"; : > "$cfg/settings.txt"
  # shellcheck disable=SC2086
  HOME="/tmp/home-$l-s1" "$AI" --appimage-extract-and-run --config "$cfg" --max-secs 80 --mode cga --scale 4 $H --start demo \
    --shot-at "120:/out/$l-s1-0120.png" --shot-at "200:/out/$l-s1-0200.png" --shot-at "300:/out/$l-s1-0300.png" --shot-at "450:/out/$l-s1-0450.png" \
    > "/out/$l-s1.log" 2>&1
  echo "-- $l s1 結束碼 $?"
  # 第三關：輸入紀錄從標題起，無敵避免撞地形死亡。
  cfg="/tmp/cfg-$l-s3"; mkdir -p "$cfg"; printf 'opt.invincible=on\n' > "$cfg/settings.txt"
  # shellcheck disable=SC2086
  HOME="/tmp/home-$l-s3" "$AI" --appimage-extract-and-run --config "$cfg" --max-secs 150 --mode cga --scale 4 $H --start title --replay /replays/flow-play.jsonl \
    --shot-at "700:/out/$l-s3-0700.png" --shot-at "760:/out/$l-s3-0760.png" \
    > "/out/$l-s3.log" 2>&1
  echo "-- $l s3 結束碼 $?"
done
for f in /out/*.png; do
  [ -s "$f" ] && echo "$(basename "$f") $(identify -format '%wx%h' "$f")" || echo "缺 $f"
done
