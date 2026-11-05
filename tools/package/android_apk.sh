#!/usr/bin/env bash
# Android 完整版 APK。原版資料與 HD 內嵌，只留本機（dist-all/ 不進版控）。
# 前提：跑過 build_all.sh 的前段，workspace/pkg 有 src、inputs、payload、notices、icons。
# 用法：android_apk.sh <版本> [ABI...]，ABI 預設 arm64-v8a x86_64（x86_64 給模擬器驗）。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"
shift || true
ABIS="${*:-arm64-v8a x86_64}"
IMG_ANDROID="rs2-android:1.99.1-r1"
need_image "$IMG_ANDROID"

AND="$PKG/android"
OUT="$AND/out"
rm -rf "$OUT"
ensure_dir "$AND" "$AND/cache" "$AND/cargo-target" "$OUT" "$PKG/logs"
FINAL="$ROOT/dist-all/$VERSION/full"
ensure_dir "$FINAL"
APK="$FINAL/RS-2-remake-$VERSION-android-full.apk"

say "android：資料包"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$PKG/payload" /payload ro
mount_dir m "$AND" /and rw
dk android-pak 600 "$IMG_ANDROID" --memory 2g --cpus 1 "${m[@]}" -- python3 -I /pkgtools/pkg_android_pak.py /payload /and/rs2-data.pak

say "android：原生與 Java（$ABIS）"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "${ANDROID_SRC:-$PKG/src}" /project ro
mount_dir m "$ROOT/android" /android ro
mount_dir m "$INPUTS" /inputs ro
mount_dir m "$PKG_CARGO" /cargo rw
mount_dir m "$AND/cache" /cache rw
mount_dir m "$AND/cargo-target" /target rw
mount_dir m "$OUT" /out rw
dk android-build 3000 "$IMG_ANDROID" "${m[@]}" \
  -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/target -e HOME=/tmp -e "RS2_VERSION=$VERSION" -e "ABIS=$ABIS" -- \
  sh /pkgtools/build_rs2_android.sh 2>&1 | tee "$PKG/logs/build-android.log"

say "android：組 APK"
vcode="$(printf '%s' "$VERSION" | sed -E 's/^v\.([0-9]+)\.([0-9]+)\.([0-9]+)-.*/\1 \2 \3/' | awk '{print $1*1000000+$2*1000+$3}')"
m=()
mount_dir m "$ROOT/android" /android ro
mount_dir m "$PKG/notices" /notices ro
mount_dir m "$PKG/icons" /icons ro
mount_file m "$ROOT/LICENSE" /LICENSE ro
mount_file m "$AND/rs2-data.pak" /rs2-data.pak ro
mount_dir m "$OUT" /out rw
mount_dir m "$AND" /and rw
dk android-apk 900 "$IMG_ANDROID" --memory 2g --cpus 2 "${m[@]}" -e HOME=/tmp -e "VNAME=$VERSION" -e "VCODE=$vcode" -- bash -euc '
  JAR=/opt/android-sdk/platforms/android-35/android.jar
  rm -rf /out/apk /out/res /out/assets
  mkdir -p /out/apk /out/res/drawable /out/res/values /out/assets
  cp /android/res/values/strings.xml /out/res/values/
  cp /icons/rs2-256.png /out/res/drawable/ic_launcher.png
  cp /LICENSE /notices/THIRD-PARTY-NOTICES.txt /notices/SDL2-LICENSE.txt /out/assets/
  ln -s /rs2-data.pak /out/assets/rs2-data.pak
  # .pak 不壓縮：裡面是 PNG，再壓也省不了多少，第一次啟動才讀得快。
  aapt package -f -0 pak -M /android/AndroidManifest.xml -S /out/res -A /out/assets -I "$JAR" \
    --min-sdk-version 23 --target-sdk-version 35 --version-name "$VNAME" --version-code "$VCODE" \
    -F /out/unsigned.apk
  cd /out
  cp -r lib apk/lib
  cp dex/classes.dex apk/classes.dex
  # 原生庫不壓縮並對齊，Android 6 起可以直接從 APK 載入。
  jar u0f unsigned.apk -C apk lib
  jar uf unsigned.apk -C apk classes.dex
  zipalign -f -p 4 unsigned.apk aligned.apk
  ks=/and/debug.keystore
  if [ ! -f "$ks" ]; then
    keytool -genkeypair -keystore "$ks" -storepass android -keypass android -alias rs2 \
      -dname "CN=RS-2 remake local build,O=wicanr2,C=TW" -keyalg RSA -keysize 2048 -validity 10000 >/dev/null 2>&1
  fi
  apksigner sign --ks "$ks" --ks-pass pass:android --key-pass pass:android --out final.apk aligned.apk
  apksigner verify --verbose final.apk | head -6
  aapt dump badging final.apk | sed -n 1,3p
  # 打包後的產物才算數：授權、資料、原生庫都要真的在裡面。
  unzip -l final.apk > list.txt
  for f in assets/LICENSE assets/THIRD-PARTY-NOTICES.txt assets/SDL2-LICENSE.txt assets/rs2-data.pak classes.dex; do
    grep -q " $f\$" list.txt || { echo "APK 缺 $f" >&2; exit 1; }
  done
  for abi in $(ls lib); do
    for so in libmain.so libSDL2.so; do
      grep -q " lib/$abi/$so\$" list.txt || { echo "APK 缺 lib/$abi/$so" >&2; exit 1; }
    done
  done
  unzip -p final.apk classes.dex | strings | grep -q AboutActivity || { echo "dex 缺 AboutActivity" >&2; exit 1; }
  sha256sum final.apk
'
cp "$OUT/final.apk" "$APK"
sha256sum "$APK" | sed "s#$FINAL/##" > "$APK.sha256"
ls -l "$APK"
check_no_root_files
say "android：完成 $APK"
