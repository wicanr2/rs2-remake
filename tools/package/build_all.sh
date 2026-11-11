#!/usr/bin/env bash
# RS-2 remake 完整版發行包（內含原版資料與 HD，只留本機，不得散布）。
#
#   tools/package/build_all.sh [版本] [選項]
#
# 版本格式 v.1.0.2-20261111，預設 v.1.0.2-20261111。
# 產出：dist-all/<版本>/full-local/
#   RS-2-remake-<版本>-full-linux-x86_64.AppImage
#   RS-2-remake-<版本>-full-windows-x86_64.zip
#   RS-2-remake-<版本>-full-macos-universal.zip      （universal .app，無簽章，未在實機驗過）
#   LICENSE、README.txt、THIRD-PARTY-NOTICES.txt、SDL2-LICENSE.txt、SHA256SUMS、manifest.json
# 驗證輸出（截圖、日誌）：dist-all/<版本>/smoke/
#
# 選項：
#   --list           列出步驟後結束
#   --from 步驟      從這個步驟開始（前面的結果沿用 workspace/pkg 內既有的）
#   --only 步驟      只跑這個步驟
#   --skip-tests     略過單元測試
#   --skip-verify    略過驗證（靜態、HD 資料子集、Linux、Windows）
#
# 每一步都在 Docker 內跑（一律 --rm、帶 UID/GID、--memory、--cpus 最多 4、--pids-limit、log 輪替、timeout、
# 預設 --network none，容器名稱前綴 rs2-pkg-）。可重跑：外部輸入有 SHA-256 鎖定（inputs.lock），
# 原始碼先做快照，SDL2 的 Linux 版編譯結果有快取。不上傳、不建 Release、不 commit、不 push。
# 進度筆記在 workspace/progress/package.md。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

STEPS=(fetch_inputs icons snapshot_src notices payload unit_tests unit_tests_windows
       build_linux build_windows build_macos pack appimage finalize verify_static verify_payload verify_linux verify_windows)
TEST_STEPS=" unit_tests unit_tests_windows "
VERIFY_STEPS=" verify_static verify_payload verify_linux verify_windows "
NO_VERSION=" fetch_inputs snapshot_src notices payload unit_tests unit_tests_windows "

VERSION="$DEFAULT_VERSION"; FROM=""; ONLY=""; SKIP_TESTS=0; SKIP_VERIFY=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --list) printf '%s\n' "${STEPS[@]}"; exit 0 ;;
    --from) FROM="${2:?--from 要給步驟名}"; shift ;;
    --only) ONLY="${2:?--only 要給步驟名}"; shift ;;
    --skip-tests) SKIP_TESTS=1 ;;
    --skip-verify) SKIP_VERIFY=1 ;;
    -h|--help) sed -n '2,28p' "$0"; exit 0 ;;
    v.*) VERSION="$1" ;;
    *) die "不認得的參數 $1" ;;
  esac
  shift
done
check_version "$VERSION"
for s in "$FROM" "$ONLY"; do
  [ -z "$s" ] || [[ " ${STEPS[*]} " == *" $s "* ]] || die "沒有這個步驟：$s（--list 看清單）"
done

for img in "$IMG_CROSS" "$IMG_LINUX" "$IMG_SQUASH" "$IMG_VERIFY"; do need_image "$img"; done
[ -f "$ORIG/S.EXE" ] || die "缺原版目錄 $ORIG"
[ -d "$HD_SRC" ] || die "缺 HD 素材 $HD_SRC"
ensure_dir "$PKG" "$PKG/logs"
LOG="$PKG/logs/build_all-$VERSION.log"
if [ -z "$FROM$ONLY" ]; then : > "$LOG"; fi

started=0; [ -z "$FROM" ] && started=1
summary=()
t_all=$SECONDS
for s in "${STEPS[@]}"; do
  [ "$started" -eq 1 ] || { [ "$s" = "$FROM" ] && started=1; }
  [ "$started" -eq 1 ] || continue
  [ -z "$ONLY" ] || [ "$s" = "$ONLY" ] || continue
  if [ "$SKIP_TESTS" -eq 1 ] && [[ "$TEST_STEPS" == *" $s "* ]]; then continue; fi
  if [ "$SKIP_VERIFY" -eq 1 ] && [[ "$VERIFY_STEPS" == *" $s "* ]]; then continue; fi
  t0=$SECONDS
  say "=== $s ==="
  if [[ "$NO_VERSION" == *" $s "* ]]; then args=(); else args=("$VERSION"); fi
  if ! "$PKG_TOOLS/$s.sh" "${args[@]}" 2>&1 | tee -a "$LOG"; then
    die "步驟 $s 失敗，日誌 $LOG"
  fi
  summary+=("$(printf '%-18s %4ds' "$s" $((SECONDS - t0)))")
done

check_no_root_files
say "完成，共 $((SECONDS - t_all)) 秒"
printf '  %s\n' "${summary[@]}"
if [ -d "$ROOT/dist-all/$VERSION/full-local" ]; then
  echo "--- dist-all/$VERSION/full-local"
  (cd "$ROOT/dist-all/$VERSION/full-local" && ls -l && cat SHA256SUMS 2>/dev/null)
fi
