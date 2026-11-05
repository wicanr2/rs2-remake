#!/usr/bin/env bash
# 步驟：rs2-play 的單元測試（資源尋找、設定目錄、參數）。只跑 bin 內的 #[test]，不跑需要原版檔的 A6。
# 用獨立的 target 目錄，不和別人的 workspace/target-play 搶鎖。
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

need_image "$IMG_LINUX"
ensure_dir "$PKG/cargo-target/test" "$PKG/logs"
m=()
mount_dir m "$PKG/src" /project ro
mount_dir m "$PKG_CARGO" /cache rw
mount_dir m "$PKG/cargo-target/test" /target rw

say "unit_tests：cargo test（Linux，$IMG_LINUX）"
dk unit 1800 "$IMG_LINUX" "${m[@]}" -w /project/crates/rs2-play \
  -e CARGO_HOME=/cache -e CARGO_TARGET_DIR=/target -e HOME=/tmp \
  -e SDL_VIDEODRIVER=dummy -e SDL_RENDER_DRIVER=software \
  -- cargo test --release --locked --offline --lib --bins 2>&1 | tee "$PKG/logs/unit-tests-linux.log"
