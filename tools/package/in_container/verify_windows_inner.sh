#!/bin/sh
# 乾淨容器內用 Wine 9.0 跑打包後的 Windows 版：解開 zip、目錄改唯讀、沒有 --orig。
# 環境由 verify_windows.sh 帶入：/dist（發行目錄，唯讀）、/out（輸出）、VERSION。
set -u
. /pkgtools/verify_common.sh
NAME="RS-2-remake-$VERSION-full-windows-x86_64"
export HOME=/tmp/home WINEPREFIX=/tmp/home/.wine WINEARCH=win64 WINEDEBUG=-all WINEDLLOVERRIDES="mscoree,mshtml="
mkdir -p "$HOME"
export SDL_RENDER_DRIVER=software SDL_VIDEODRIVER=windows SDL_AUDIODRIVER=disk
cd /tmp
start_xvfb

echo "== 解開 zip（用 Python zipfile，等同玩家解壓）並把目錄改成唯讀"
python3 -I - <<PY
import zipfile, os, stat
z = zipfile.ZipFile("/dist/$NAME.zip")
z.extractall("/tmp/win")
for zi in z.infolist():
    p = os.path.join("/tmp/win", zi.filename)
    mode = (zi.external_attr >> 16) & 0o777
    if mode and not zi.filename.endswith("/"):
        os.chmod(p, mode)
print("解開", len(z.infolist()), "項")
PY
find /tmp/win -type d -exec chmod 555 {} +
find /tmp/win -type f -exec chmod a-w {} +
GAME="/tmp/win/$NAME"
ls "$GAME"
echo "解開位置可寫：$(touch "$GAME/x" 2>/dev/null && echo 是 || echo 否)"
WGAME="Z:\\tmp\\win\\$NAME"

echo "== 建立 Wine prefix"
timeout 120 wineboot --init >/out/wineboot.log 2>&1; echo "wineboot 結束碼 $?"
echo "Wine：$(wine --version)"

echo "== --version"
v="$(timeout 60 wine "$WGAME\\rs2-play.exe" --version 2>&1 | tr -d '\r')"; echo "$v"
echo "$v" | grep -q "rs2-play $VERSION" && pass "exe --version = rs2-play $VERSION" || fail "版本不符：$v"

run() { # 名稱 秒數 檔名前綴 參數...
  name="$1"; secs="$2"; pre="$3"; shift 3
  SDL_DISKAUDIOFILE="Z:\\out\\$pre.raw" timeout $((secs + 90)) wine "$@" > "/out/$pre.log" 2>&1
  rc=$?
  echo "-- $name 結束碼 $rc"
  return $rc
}

echo "== A：標題畫面，預設參數（沒有 --orig、沒有 --hd），30 秒"
run A 30 title "$WGAME\\rs2-play.exe" --max-secs 30 --shot-at "80:Z:\\out\\title-0080.png" --shot-at "200:Z:\\out\\title-0200.png"
[ $? -eq 0 ] && pass "A 結束碼 0" || fail "A 結束碼非 0"
tr -d '\r' < /out/title.log | grep -E "\[資源\]|\[init\]|\[hd\]|\[audio\]|\[window\]|\[shot\]|\[設定\]|panic" | cut -c1-220
tr -d '\r' < /out/title.log | grep -q "原版目錄：.*\\\\data\\\\original（內建位置）" && pass "A 原版目錄是 exe 旁的 data\\original" || fail "A 原版目錄不是內建位置"
tr -d '\r' < /out/title.log | grep -q "HD 資料夾：.*\\\\data\\\\hd（內建位置），預設使用" && pass "A HD 預設開啟（內建位置）" || fail "A HD 沒有自動啟用"
tr -d '\r' < /out/title.log | grep -E "\[hd\] 預載 [0-9]+ 檔.*缺圖 0 項" >/dev/null && pass "A HD 預載成功、缺圖 0" || fail "A HD 預載有缺圖或沒有預載"
shot_check /out/title-0080.png "A 第 80 步"
shot_check /out/title-0200.png "A 第 200 步"
python3 -I /pkgtools/pkg_audio_check.py /out/title.raw 20 49716 && pass "A 音訊非靜音" || fail "A 音訊靜音或太短"
cfg="$(find "$WINEPREFIX/drive_c/users" -path '*rs2-remake/settings.txt' 2>/dev/null | head -1)"
[ -n "$cfg" ] && pass "A 設定寫到 %APPDATA%：${cfg#$WINEPREFIX/}" || fail "A 沒有在 %APPDATA% 找到 settings.txt"

echo "== B：第一關展示，HD 預設開，30 秒"
run B 30 demo-hd "$WGAME\\rs2-play.exe" --start demo --max-secs 30 --shot-at "120:Z:\\out\\demo-hd-0120.png" --shot-at "260:Z:\\out\\demo-hd-0260.png"
[ $? -eq 0 ] && pass "B 結束碼 0" || fail "B 結束碼非 0"
shot_check /out/demo-hd-0260.png "B 第 260 步（HD）"
python3 -I /pkgtools/pkg_audio_check.py /out/demo-hd.raw 20 49716 && pass "B 音訊非靜音" || fail "B 音訊靜音或太短"

echo "== C：第一關展示，--no-hd（對照）"
run C 30 demo-orig "$WGAME\\rs2-play.exe" --start demo --no-hd --max-secs 30 --shot-at "260:Z:\\out\\demo-orig-0260.png"
[ $? -eq 0 ] && pass "C 結束碼 0" || fail "C 結束碼非 0"
ae="$(compare -metric AE /out/demo-hd-0260.png /out/demo-orig-0260.png null: 2>&1)"
echo "HD 與原版圖同一步的差異像素數：$ae"
[ "${ae%%.*}" -gt 1000 ] 2>/dev/null && pass "HD 與原版圖的畫面確實不同（差 $ae 像素）" || fail "HD 與原版圖的畫面沒有差異（$ae）"

echo "== C2：CGA、MGA，單色圖庫來自發行包"
for m in cga mga; do
  run C2 15 "demo-$m" "$WGAME\\rs2-play.exe" --start demo --mode "$m" --max-secs 15 --shot-at "60:Z:\\out\\demo-$m-0060.png"
  [ $? -eq 0 ] && pass "C2 $m 結束碼 0" || fail "C2 $m 結束碼非 0"
  grep -q "\[單色\] 單色圖庫雜湊驗證通過" "/out/demo-$m.log" && pass "C2 $m 單色圖庫載入成功" || fail "C2 $m 單色圖庫載入失敗"
  shot_check "/out/demo-$m-0060.png" "C2 $m 第 60 步"
done

echo "== D：預設繪圖器（不指定 SDL_RENDER_DRIVER，Wine 的 Direct3D），15 秒，只看能不能起來"
SDL_RENDER_DRIVER= run D 15 default-renderer "$WGAME\\rs2-play.exe" --start demo --max-secs 15 --shot-at "120:Z:\\out\\default-renderer-0120.png"
rc=$?
tr -d '\r' < /out/default-renderer.log | grep -E "\[window\]|\[shot\]" | cut -c1-200
[ $rc -eq 0 ] && shot_check /out/default-renderer-0120.png "D 預設繪圖器" || echo "NOTE D 預設繪圖器在 Wine/Xvfb 下結束碼 $rc（Wine 無 GPU 的環境限制，不計入失敗）"

echo "== E：找不到資料時的錯誤訊息（把 data 搬走的副本）"
mkdir -p /tmp/bare && cp "$GAME/rs2-play.exe" "$GAME/SDL2.dll" /tmp/bare/
timeout 60 wine "Z:\\tmp\\bare\\rs2-play.exe" --no-audio --max-secs 3 > /out/missing.log 2>&1; rc=$?
tr -d '\r' < /out/missing.log | head -12
[ "$rc" -ne 0 ] && tr -d '\r' < /out/missing.log | grep -q "找不到原版資料。已找過" && tr -d '\r' < /out/missing.log | grep -q "bare\\\\data\\\\original" && pass "E 沒有資料時結束碼 $rc，訊息列出找過的路徑" || fail "E 錯誤訊息不完整"

wineserver -k 2>/dev/null
stop_xvfb
echo "== 結果：FAIL $FAILS 項"
[ "$FAILS" -eq 0 ]
