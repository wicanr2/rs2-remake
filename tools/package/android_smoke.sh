#!/usr/bin/env bash
# Android 煙霧測試：在模擬器裝實際的 APK、跑起來、用合成的觸控玩一段，留下截圖與 logcat。
# 模擬器不等於真機，手感、瀏海、藍牙手把要真機才驗得到。
# 用法：android_smoke.sh <APK> <輸出目錄>
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
APK="${1:?要指定 APK}"; OUT="${2:?要指定輸出目錄}"
[ -f "$APK" ] || die "找不到 $APK"
[ -e /dev/kvm ] || die "沒有 /dev/kvm"
IMG="rich2-android-emulator:api35-20261102-r1"
need_image "$IMG"
mkdir -p "$OUT"
ensure_dir "$OUT"
OUT="$(cd "$OUT" && pwd)"
APK_DIR="$(cd "$(dirname "$APK")" && pwd)"
APK_NAME="$(basename "$APK")"

timeout --kill-after=30 1500 docker run --rm --init --name "rs2-pkg-android-smoke-$$" \
  -u "$(id -u):$(id -g)" --group-add "$(stat -c %g /dev/kvm)" --device /dev/kvm \
  --memory 8g --cpus 4 --pids-limit 1024 --network none \
  --log-opt max-size=10m --log-opt max-file=3 \
  -e HOME=/tmp -e "APK=/apk/$APK_NAME" \
  -v "$APK_DIR:/apk:ro" -v "$OUT:/out" -v "$PKG_TOOLS/in_container:/pkgtools:ro" \
  "$IMG" bash /pkgtools/android_smoke_inner.sh 2>&1 | tee "$OUT/verify.log"
