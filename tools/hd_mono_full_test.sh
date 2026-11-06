#!/usr/bin/env bash
# 以暫存 HD 根目錄跑 rs2-gfx 的測試（含 #[ignore]），其中 real_hd_mono_images_match_the_sd_item_dimensions 稽核單色 HD 的 1,526 個檔。
# 本腳本不改 tools/rust.sh：照 rust.sh test 的 docker 參數與掛載，只是把 RS2_HD 指到 hd_mono_root.py 組出的暫存根（mono 指向候選，其他連到 assets）。
#   tools/hd_mono_full_test.sh <根目錄名> [cargo test 參數...]
#   根目錄名 = workspace/hd/mono-m2-review/root-<名>，預設參數 -p rs2-gfx -- --include-ignored --nocapture
# 規則同 AGENTS.md 硬規則 1 到 3：docker run --rm、目前 UID/GID、--memory、--cpus（4）、--pids-limit、log 輪替、--network none、外層 timeout、
# 容器名 rs2-mono- 前綴、專案唯讀掛載、掛載前確認來源存在。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="rust:1.99.1-slim-bookworm@sha256:24707e70a4f1e23d9a7b25202ec43126d28a52294b96aad5c670df1fd1f1d400"
WP="$ROOT/workspace"
ORIG="$ROOT/original/rs-2"
DOSRUN="$WP/dosrun"
NAME="${1:?根目錄名（root-<名>）}"
shift
RT="$WP/hd/mono-m2-review/root-$NAME"
[ -d "$RT/mono" ] || { echo "缺 $RT/mono（先跑 hd_mono_root.py）" >&2; exit 1; }
mkdir -p "$WP/target" "$WP/rs2"
for d in "$ORIG" "$DOSRUN" "$WP/cargo" "$WP/target" "$WP/rs2" "$WP/hd"; do
  [ -d "$d" ] || { echo "缺目錄 $d" >&2; exit 1; }
done
docker image inspect "$IMAGE" > /dev/null 2>&1 || { echo "找不到映像 $IMAGE" >&2; exit 1; }
args=("$@")
[ "${#args[@]}" -gt 0 ] || args=(-p rs2-gfx -- --include-ignored --nocapture)
extra=()
if [ -d "$WP/dosrun-modes" ]; then extra+=(-v "$WP/dosrun-modes:/dosrun-modes:ro" -e RS2_DOSRUN_MODES=/dosrun-modes); fi
timeout 3000 docker run --rm -u "$(id -u):$(id -g)" --name "rs2-mono-test-$$" \
  --memory 4g --cpus 4 --pids-limit 256 --network none \
  --log-opt max-size=10m --log-opt max-file=3 \
  -e CARGO_HOME=/cache -e CARGO_TARGET_DIR=/target -e HOME=/tmp \
  -e RS2_ORIG=/orig -e RS2_DOSRUN=/dosrun -e RS2_OUT=/out \
  -v "$WP/cargo:/cache" -v "$WP/target:/target" -v "$WP/rs2:/out" \
  -v "$ORIG:/orig:ro" -v "$DOSRUN:/dosrun:ro" \
  -v "$WP/hd:/hdall:ro" -e "RS2_HD=/hdall/mono-m2-review/root-$NAME" "${extra[@]}" \
  -w /project -v "$ROOT:/project:ro" "$IMAGE" \
  cargo test --release --locked --offline "${args[@]}"
