#!/bin/sh
# 乾淨容器內驗證 Linux AppImage：沒有專案目錄、沒有 --orig、沒有 SDL2。
# 環境由 verify_linux.sh 帶入：/dist（發行目錄，唯讀）、/appdir（AppDir，唯讀）、/out（輸出）、VERSION。
set -u
. /pkgtools/verify_common.sh
NAME="RS-2-remake-$VERSION-full-linux-x86_64.AppImage"
AI="/dist/$NAME"
export HOME=/tmp/home
mkdir -p "$HOME"
export SDL_RENDER_DRIVER=software SDL_VIDEODRIVER=x11 SDL_AUDIODRIVER=disk
export APPIMAGE_EXTRACT_AND_RUN=1 TMPDIR=/tmp
cd /tmp
start_xvfb

echo "== 環境"
echo "cwd=$(pwd) 使用者=$(id -u) /dist 可寫=$(touch /dist/x 2>/dev/null && echo 是 || echo 否) /appdir 可寫=$(touch /appdir/x 2>/dev/null && echo 是 || echo 否)"
echo "系統有 libSDL2：$(ls /usr/lib/x86_64-linux-gnu/ | grep -c 'libSDL2')；--orig 與 RS2_DATA：沒有（RS2_DATA=${RS2_DATA:-未設}）"
ls -l "$AI"

echo "== --version"
v="$("$AI" --appimage-extract-and-run --version 2>&1)"; echo "$v"
[ "$v" = "rs2-play $VERSION" ] && pass "AppImage --version = $v" || fail "版本不符：$v"

run() { # 名稱 秒數 檔名前綴 參數...
  name="$1"; secs="$2"; pre="$3"; shift 3
  SDL_DISKAUDIOFILE="/out/$pre.raw" "$@" > "/out/$pre.log" 2>&1
  rc=$?
  echo "-- $name 結束碼 $rc"
  return $rc
}

echo "== A：標題畫面，預設參數（沒有 --orig、沒有 --hd），30 秒"
run A 30 title "$AI" --appimage-extract-and-run --max-secs 30 --shot-at 80:/out/title-0080.png --shot-at 200:/out/title-0200.png
[ $? -eq 0 ] && pass "A 結束碼 0" || fail "A 結束碼非 0"
grep -E "\[資源\]|\[init\]|\[hd\]|\[audio\]|\[window\]|\[shot\]" /out/title.log | cut -c1-200
grep -q "原版目錄：/tmp/appimage_extracted_[0-9a-f]*/usr/share/rs2/original（內建位置）" /out/title.log && pass "A 原版目錄來自內建位置" || fail "A 原版目錄不是內建位置"
grep -q "HD 資料夾：.*usr/share/rs2/hd（內建位置），預設使用" /out/title.log && pass "A HD 預設開啟（內建位置）" || fail "A HD 沒有自動啟用"
grep -E "\[hd\] 預載 [0-9]+ 檔.*缺圖 0 項" /out/title.log >/dev/null && pass "A HD 預載成功、缺圖 0" || fail "A HD 預載有缺圖或沒有預載"
shot_check /out/title-0080.png "A 第 80 步"
shot_check /out/title-0200.png "A 第 200 步"
python3 -I /pkgtools/pkg_audio_check.py /out/title.raw 20 49716 && pass "A 音訊非靜音" || fail "A 音訊靜音或太短"

echo "== B：第一關展示，HD 預設開，30 秒"
run B 30 demo-hd "$AI" --appimage-extract-and-run --start demo --max-secs 30 --shot-at 120:/out/demo-hd-0120.png --shot-at 260:/out/demo-hd-0260.png
[ $? -eq 0 ] && pass "B 結束碼 0" || fail "B 結束碼非 0"
shot_check /out/demo-hd-0260.png "B 第 260 步（HD）"
python3 -I /pkgtools/pkg_audio_check.py /out/demo-hd.raw 20 49716 && pass "B 音訊非靜音" || fail "B 音訊靜音或太短"

echo "== C：第一關展示，--no-hd（對照）"
run C 30 demo-orig "$AI" --appimage-extract-and-run --start demo --no-hd --max-secs 30 --shot-at 260:/out/demo-orig-0260.png
[ $? -eq 0 ] && pass "C 結束碼 0" || fail "C 結束碼非 0"
grep -q "HD：--no-hd" /out/demo-orig.log && pass "C --no-hd 生效" || fail "C --no-hd 沒生效"
ae="$(compare -metric AE /out/demo-hd-0260.png /out/demo-orig-0260.png null: 2>&1)"
echo "HD 與原版圖同一步的差異像素數：$ae"
[ "${ae%%.*}" -gt 1000 ] 2>/dev/null && pass "HD 與原版圖的畫面確實不同（差 $ae 像素）" || fail "HD 與原版圖的畫面沒有差異（$ae）"

echo "== C2：CGA、MGA，單色圖庫來自發行包"
for m in cga mga; do
  run C2 15 "demo-$m" "$AI" --appimage-extract-and-run --start demo --mode "$m" --max-secs 15 --shot-at "60:/out/demo-$m-0060.png"
  [ $? -eq 0 ] && pass "C2 $m 結束碼 0" || fail "C2 $m 結束碼非 0"
  grep -q "\[單色\] 單色圖庫雜湊驗證通過" "/out/demo-$m.log" && pass "C2 $m 單色圖庫載入成功" || fail "C2 $m 單色圖庫載入失敗"
  shot_check "/out/demo-$m-0060.png" "C2 $m 第 60 步"
done

echo "== D：唯讀的 AppDir（直接跑 AppRun，不經 AppImage 解開），HOME 指向不可寫的位置"
HOME=/proc run D 8 readonly /appdir/AppRun --start demo --max-secs 8 --shot-at 40:/out/readonly-0040.png
[ $? -eq 0 ] && pass "D 唯讀 AppDir 結束碼 0" || fail "D 唯讀 AppDir 結束碼非 0"
grep -E "\[資源\] 原版目錄" /out/readonly.log
grep -q "原版目錄：/appdir/usr/share/rs2/original（內建位置）" /out/readonly.log && pass "D 資料從唯讀 AppDir 讀到" || fail "D 沒有從 AppDir 讀資料"
grep -E "\[設定\] 警告" /out/readonly.log | head -2
grep -q "\[設定\] 警告：寫不了" /out/readonly.log && pass "D 設定目錄不可寫時只警告，遊戲照跑" || echo "NOTE D 沒有看到設定寫入警告（可能 HOME=/proc 下沒有走到存檔）"
shot_check /out/readonly-0040.png "D 第 40 步"

echo "== E：找不到資料時的錯誤訊息（用 RS2_DATA 指向空目錄、APPDIR 指向空）"
mkdir -p /tmp/empty
RS2_DATA=/tmp/empty "$AI" --appimage-extract-and-run --no-audio --max-secs 3 > /out/missing.log 2>&1; rc=$?
echo "結束碼 $rc（預期非 0，但包內資料仍在，所以這支其實會成功）"
cp /out/missing.log /out/missing-env-fallthrough.log
# 真正找不到：直接跑 AppRun 的執行檔，複製到沒有資料的地方
mkdir -p /tmp/bare/usr/bin /tmp/bare/usr/lib
cp /appdir/usr/bin/rs2-play /tmp/bare/usr/bin/; cp /appdir/usr/lib/libSDL2-2.0.so.0 /tmp/bare/usr/lib/
/tmp/bare/usr/bin/rs2-play --no-audio --max-secs 3 > /out/missing.log 2>&1; rc=$?
cat /out/missing.log
[ "$rc" -ne 0 ] && grep -q "找不到原版資料。已找過" /out/missing.log && grep -q "/tmp/bare/usr/share/rs2/original" /out/missing.log && pass "E 沒有資料時結束碼 $rc，訊息列出找過的路徑" || fail "E 錯誤訊息不完整"

echo "== F：AppImage 內容對 MANIFEST.json（--appimage-extract 解開後逐檔雜湊，含授權檔）"
rm -rf /tmp/squashfs-root
(cd /tmp && "$AI" --appimage-extract >/dev/null 2>&1)
python3 -I /pkgtools/pkg_check_tree.py /tmp/squashfs-root LICENSE README.txt usr/share/doc/rs2-remake/LICENSE usr/share/doc/rs2-remake/THIRD-PARTY-NOTICES.txt \
  usr/share/doc/rs2-remake/SDL2-LICENSE.txt usr/share/licenses/appimage-runtime/runtime-MIT.txt usr/share/rs2/hd/base-palette.tsv usr/lib/libSDL2-2.0.so.0 .DirIcon rs2-remake.desktop \
  && pass "F AppImage 解開後的內容與 MANIFEST 完全相同，授權檔都在" || fail "F AppImage 內容與 MANIFEST 不符"
LD_APP="$(ldd /tmp/squashfs-root/usr/bin/rs2-play 2>&1)"; echo "$LD_APP" | sed "s/ (0x[0-9a-f]*)//"
if echo "$LD_APP" | grep -q "not found"; then fail "F rs2-play 有找不到的函式庫"; elif echo "$LD_APP" | grep -Eq "libSDL2-2.0.so.0 => /tmp/squashfs-root/usr/(bin/\.\./)?lib/libSDL2-2.0.so.0"; then pass "F rs2-play 的 SDL2 解析到包內的 usr/lib（RUNPATH），沒有找不到的函式庫"; else fail "F rs2-play 的 SDL2 沒有解析到包內"; fi
LD_SDL="$(ldd /tmp/squashfs-root/usr/lib/libSDL2-2.0.so.0 2>&1)"
OTHER="$(echo "$LD_SDL" | grep -v -E "linux-vdso|libm\.so|libc\.so|ld-linux" || true)"
if [ -z "$OTHER" ]; then pass "F 內建 libSDL2 只依賴 libm 與 libc（其餘後端以 dlopen 載入）"; else fail "F 內建 libSDL2 還有其他相依：$OTHER"; fi

stop_xvfb
echo "== 結果：FAIL $FAILS 項"
[ "$FAILS" -eq 0 ]
