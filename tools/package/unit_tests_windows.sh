#!/usr/bin/env bash
# 步驟：Windows 路徑處理的單元測試。交叉編成 Windows 的測試執行檔，用 Wine 跑，
# 這樣 #[cfg(windows)] 的測試（反斜線路徑、%APPDATA%）在真正的 Windows 路徑語意下執行。
# 用法：unit_tests_windows.sh
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
IMG_V="$IMG_VERIFY"

need_image "$IMG_CROSS"; need_image "$IMG_V"
ensure_dir "$PKG/cargo-target/windows-test" "$PKG/windows-test-bin" "$PKG/logs"
rm -f "$PKG/windows-test-bin/"*
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$PKG/src" /project ro
mount_dir m "$PKG_CARGO" /cache rw
mount_dir m "$PKG/cargo-target/windows-test" /target rw
mount_dir m "$PKG/windows-test-bin" /out rw
say "unit_tests_windows：交叉編譯測試執行檔"
dk build-wintest 1800 "$IMG_CROSS" "${m[@]}" --tmpfs /tmp:rw,exec,size=512m \
  -e CARGO_HOME=/cache -e CARGO_TARGET_DIR=/target -e HOME=/tmp -- sh /pkgtools/build_tests_windows.sh 2>&1 | tail -5

m=()
mount_dir m "$PKG/windows-test-bin" /wtest ro
say "unit_tests_windows：Wine 執行"
dk run-wintest 600 "$IMG_V" --entrypoint sh --memory 2g --cpus 2 --tmpfs /tmp:rw,exec,size=1g,mode=1777 -w /tmp "${m[@]}" \
  -- -c 'export HOME=/tmp/home WINEPREFIX=/tmp/home/.wine WINEARCH=win64 WINEDEBUG=-all WINEDLLOVERRIDES="mscoree,mshtml="; mkdir -p $HOME
         mkdir -p -m 1777 /tmp/.X11-unix; Xvfb :99 -screen 0 800x600x24 -nolisten tcp >/dev/null 2>&1 & export DISPLAY=:99; sleep 2
         wineboot --init >/dev/null 2>&1
         cp /wtest/* /tmp/ && cd /tmp && wine rs2_play_tests.exe --test-threads=1 2>&1 | tr -d "\r"; rc=$?; wineserver -k; exit $rc' \
  2>&1 | tee "$PKG/logs/unit-tests-windows-wine.log" | grep -E "resources::|settings::|args::|test result|windows_native|panicked|FAILED" | tail -60
test "${PIPESTATUS[0]}" -eq 0 || die "Windows 單元測試失敗"
