#!/usr/bin/env bash
# 步驟：驗發行包的 HD 子集夠用。同一個 Linux 執行檔讀完整 HD 與讀 payload，headless 跑第一關展示與標題流程，
# 同一步的 PNG 必須逐位元組相同，且兩邊都沒有缺圖。用法：verify_payload.sh <版本>
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
VERSION="${1:-$DEFAULT_VERSION}"; check_version "$VERSION"

need_image "$IMG_LINUX"
OUT="$ROOT/dist-all/$VERSION/smoke/payload"
rm -rf "$OUT"
ensure_dir "$ROOT/dist-all/$VERSION/smoke" "$OUT"
m=()
mount_dir m "$PKG_TOOLS/in_container" /pkgtools ro
mount_dir m "$PKG/cargo-target/linux/release" /bin-rs2 ro
mount_dir m "$PKG/cache/sdl2-linux/lib" /sdl/lib ro
mount_dir m "$ORIG" /orig ro
mount_dir m "$HD_SRC" /hd ro
mount_dir m "$PKG/payload" /payload ro
mount_dir m "$OUT" /out rw
say "verify_payload：完整 HD 與發行包資料子集的畫面比對"
dk verify-payload 900 "$IMG_LINUX" --entrypoint sh --memory 4g --cpus 4 --tmpfs /tmp:rw,exec,size=512m,mode=1777 "${m[@]}" -- /pkgtools/verify_payload_inner.sh 2>&1 | tee "$OUT/verify.log"
