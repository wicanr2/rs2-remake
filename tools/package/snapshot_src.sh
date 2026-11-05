#!/usr/bin/env bash
# 步驟：把目前的原始碼（Cargo.toml、Cargo.lock、crates/）複製成快照，之後所有建置與測試都從快照做。
# 原因：建置要跑好幾分鐘，期間別的人可能還在改 working tree。快照讓同一版發行包的三個平台用同一份程式，
# 並把來源雜湊記進 manifest。不含 target 目錄與 .git。
# 快照放 workspace/pkg/snapshots/<時間戳>/，workspace/pkg/src 是指向最新一份的符號連結（原子切換），
# 舊快照不立刻刪（別的腳本可能正掛載著它，例如 android_apk.sh），只保留最新 4 份。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

need_image "$IMG_CROSS"
SNAPS="$PKG/snapshots"
ts="$(date +%Y%m%d-%H%M%S)"
ensure_dir "$PKG" "$SNAPS" "$SNAPS/$ts"
# 舊版本的腳本把 src 當真目錄用：搬進 snapshots 再換成連結。
if [ -d "$PKG/src" ] && [ ! -L "$PKG/src" ]; then
  mv "$PKG/src" "$SNAPS/legacy-$ts"
fi
m=()
mount_dir m "$ROOT" /project ro
mount_dir m "$SNAPS/$ts" /snap rw

say "snapshot_src：複製原始碼到 snapshots/$ts"
dk snap 300 "$IMG_CROSS" --memory 1g --cpus 1 "${m[@]}" -- sh -eu -c '
  cd /project
  tar --exclude=target --exclude=.git -cf - Cargo.toml Cargo.lock crates | tar -C /snap -xf -
  cd /snap
  find . -type f \( -name "*.rs" -o -name "Cargo.toml" -o -name "Cargo.lock" \) | LC_ALL=C sort | xargs sha256sum > /snap/SOURCE.sha256
  echo "檔案 $(wc -l < /snap/SOURCE.sha256) 個，彙總 $(sha256sum /snap/SOURCE.sha256 | cut -d" " -f1)"
'
ln -sfn "snapshots/$ts" "$PKG/src.tmp"
mv -T "$PKG/src.tmp" "$PKG/src"
# 保留最新 4 份。
ls -1dt "$SNAPS"/*/ | tail -n +5 | while read -r old; do rm -rf "$old"; done
