#!/bin/sh
# 乾淨容器內用 Linux AppImage 截 README 用的畫面。環境由 readme_shots.sh 帶入：/dist（唯讀）、/out、VERSION。
# --shot-at 存的是視窗內容，含工具列。
set -u
. /pkgtools/verify_common.sh
AI="/dist/RS-2-remake-$VERSION-full-linux-x86_64.AppImage"
export SDL_RENDER_DRIVER=software SDL_VIDEODRIVER=x11 SDL_AUDIODRIVER=dummy
export APPIMAGE_EXTRACT_AND_RUN=1 TMPDIR=/tmp
cd /tmp
mkdir -p -m 1777 /tmp/.X11-unix 2>/dev/null || true
Xvfb :99 -screen 0 2560x1440x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
i=0
while [ ! -S /tmp/.X11-unix/X99 ] && [ "$i" -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
[ -S /tmp/.X11-unix/X99 ] || { echo "FAIL xvfb 沒有起來"; cat /tmp/xvfb.log; exit 1; }
export DISPLAY=:99

# shot 名稱 秒數 設定行（以 ; 分隔，可空） 參數...
shot() {
  name="$1"; secs="$2"; cfg="$3"; shift 3
  dir="/tmp/cfg-$name"
  mkdir -p "$dir"
  printf '%s\n' "$cfg" | tr ';' '\n' > "$dir/settings.txt"
  HOME="/tmp/home-$name" "$AI" --appimage-extract-and-run --config "$dir" --max-secs "$secs" "$@" > "/out/$name.log" 2>&1
  echo "-- $name 結束碼 $?"
}

shot hero 40 "" --start demo --shot-at 420:/out/hero.png
shot cga 40 "" --start demo --mode cga --shot-at 420:/out/cga.png
shot mga 40 "" --start demo --mode mga --shot-at 420:/out/mga.png
shot orig 40 "" --start demo --no-hd --shot-at 420:/out/orig.png
shot settings 40 "" --start demo --shot-at 300:/out/settings-game.png --open-panel 遊戲
shot pad 40 "" --start demo --fake-pads --shot-at 300:/out/settings-pad.png --open-panel 手把
shot wide 60 "opt.field_size=150;opt.invincible=on" --start newgame \
  --shot-at 250:/out/wide150-0250.png --shot-at 400:/out/wide150-0400.png --shot-at 550:/out/wide150-0550.png
shot wide120 60 "opt.field_size=120;opt.invincible=on" --start newgame --shot-at 400:/out/wide120-0400.png

for f in /out/*.png; do
  [ -s "$f" ] && echo "$(basename "$f") $(identify -format '%wx%h' "$f")" || echo "缺 $f"
done
