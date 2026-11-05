# 驗證用的共用函式。在驗證容器內 source。
# 啟動 Xvfb 並等到 socket 出現。
start_xvfb() {
  mkdir -p -m 1777 /tmp/.X11-unix 2>/dev/null || true
  Xvfb :99 -screen 0 1280x800x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
  XVFB_PID=$!
  i=0
  while [ ! -S /tmp/.X11-unix/X99 ] && [ "$i" -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
  [ -S /tmp/.X11-unix/X99 ] || { echo "FAIL xvfb 沒有起來"; cat /tmp/xvfb.log; exit 1; }
  export DISPLAY=:99
}
stop_xvfb() { kill "$XVFB_PID" 2>/dev/null || true; }

FAILS=0
pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; FAILS=$((FAILS + 1)); }

# nonblack 檔案：用 ImageMagick 取平均亮度與尺寸。
shot_check() {
  f="$1"; label="$2"
  [ -s "$f" ] || { fail "$label：沒有截圖 $f"; return; }
  mean="$(convert "$f" -colorspace Gray -format '%[fx:mean]' info:)"
  dim="$(identify -format '%wx%h' "$f")"
  if awk -v m="$mean" 'BEGIN{exit !(m > 0.02)}'; then pass "$label：截圖 $dim，平均亮度 $mean，非黑"; else fail "$label：截圖 $dim 平均亮度 $mean，近乎全黑"; fi
}
