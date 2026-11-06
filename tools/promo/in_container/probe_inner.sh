#!/bin/sh
# 容器內：不開即時錄影，用 --headless-steps 與 --shot-at 快速取樣，設計輸入腳本與挑片段時使用。
# 環境由 probe.sh 帶入：/dist（唯讀，AppImage）、/rpl（唯讀，輸入腳本）、/out（可寫）、VERSION。
# 用法：probe_inner.sh 名稱 "設定行;以分號分隔" 遊戲參數...
set -u
AI="/dist/RS-2-remake-$VERSION-full-linux-x86_64.AppImage"
export SDL_RENDER_DRIVER=software SDL_VIDEODRIVER=x11 SDL_AUDIODRIVER=dummy
export APPIMAGE_EXTRACT_AND_RUN=1 TMPDIR=/tmp
cd /tmp
mkdir -p -m 1777 /tmp/.X11-unix 2>/dev/null || true
Xvfb :99 -screen 0 1280x800x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
i=0
while [ ! -S /tmp/.X11-unix/X99 ] && [ "$i" -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
[ -S /tmp/.X11-unix/X99 ] || { echo "FAIL xvfb 沒有起來"; cat /tmp/xvfb.log; exit 1; }
export DISPLAY=:99
name="$1"; cfg="$2"; shift 2
dir="/tmp/cfg-$name"
mkdir -p "$dir"
printf '%s\n' "$cfg" | tr ';' '\n' > "$dir/settings.txt"
mkdir -p "/out/$name"
HOME="/tmp/home-$name" "$AI" --appimage-extract-and-run --config "$dir" "$@" > "/out/$name.log" 2>&1
echo "$name 結束碼 $?"
