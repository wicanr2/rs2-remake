#!/bin/sh
# 容器內：依 hd_mono_full_shots.sh 帶入的環境（VERSION、LABELS、MODES、可選的 SCENES，/dist、/hd、/replays 唯讀，/out 可寫）逐模式、逐標籤截圖。
# SCENES 預設 "flow open"，每個模式與標籤的執行：
#   flow     --start title --replay flow-play.jsonl（標題、第一至四關、片尾），無敵避免撞地形死亡
#   open     --start title 不給輸入，截開場動畫（稀疏的步數，到第 300 步的第一關展示）
#   open2    --start title 不給輸入，第 19 至 39 步逐步截，找標誌與過場的畫面
# 輸出 /out/<模式>-<標籤>-<場景>-<步數>.png（含工具列）。
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
    *) echo "--hd /hd/mono-m2-review/root-$1" ;;
  esac
}
want() { case " ${SCENES:-flow open} " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

for m in $MODES; do
  for l in $LABELS; do
    H=$(hdarg "$l")
    if want flow; then
      # 標題、四關、片尾：步數取自 tools/promo/in_container/gen_replays.sh 的預設（G1=148 C1=178 G2=323 C2=513 G3=618 C3=808 G4=920 C4=980 共 1150 步）
      cfg="/tmp/cfg-$m-$l-flow"; mkdir -p "$cfg"; printf 'opt.invincible=on\n' > "$cfg/settings.txt"
      args=""
      for st in 30 60 90 130 170 360 460 700 760 940 975 1040 1100 1140; do args="$args --shot-at $st:/out/$m-$l-flow-$st.png"; done
      # shellcheck disable=SC2086
      HOME="/tmp/home-$m-$l-flow" "$AI" --appimage-extract-and-run --config "$cfg" --max-secs 200 --mode "$m" --scale 4 $H --start title --replay /replays/flow-play.jsonl $args \
        > "/out/$m-$l-flow.log" 2>&1
      echo "-- $m $l flow 結束碼 $?"
    fi
    if want open; then
      cfg="/tmp/cfg-$m-$l-open"; mkdir -p "$cfg"; : > "$cfg/settings.txt"
      args=""
      for st in 1 2 3 4 5 6 8 10 12 14 16 18 20 40 60 80 100 120 140 160 180 200 230 260 300; do args="$args --shot-at $st:/out/$m-$l-open-$st.png"; done
      # shellcheck disable=SC2086
      HOME="/tmp/home-$m-$l-open" "$AI" --appimage-extract-and-run --config "$cfg" --max-secs 60 --mode "$m" --scale 4 $H --start title $args \
        > "/out/$m-$l-open.log" 2>&1
      echo "-- $m $l open 結束碼 $?"
    fi
    if want open2; then
      cfg="/tmp/cfg-$m-$l-open2"; mkdir -p "$cfg"; : > "$cfg/settings.txt"
      args=""
      for st in 19 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39; do args="$args --shot-at $st:/out/$m-$l-open2-$st.png"; done
      # shellcheck disable=SC2086
      HOME="/tmp/home-$m-$l-open2" "$AI" --appimage-extract-and-run --config "$cfg" --max-secs 40 --mode "$m" --scale 4 $H --start title $args \
        > "/out/$m-$l-open2.log" 2>&1
      echo "-- $m $l open2 結束碼 $?"
    fi
  done
done
for f in /out/*.png; do
  [ -s "$f" ] && echo "$(basename "$f") $(identify -format '%wx%h' "$f")" || echo "缺 $f"
done
