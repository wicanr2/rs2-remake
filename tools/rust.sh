#!/usr/bin/env bash
# Rust 的建置與測試一律在 docker。主機只呼叫本腳本。
#
#   tools/rust.sh test [參數...]   跑單元測試，參數交給 cargo test
#   tools/rust.sh verify           跑全部測試，含需要原版檔的 #[ignore] 測試
#   tools/rust.sh trace [參數...]  執行 rs2-trace
#   tools/rust.sh cargo <參數...>  執行任意 cargo 子命令
#   tools/rust.sh image-sdl        建置 rs2-play 用的映像（唯一需要網路的一步）
#   tools/rust.sh play-build       建置 rs2-play（SDL2 前端，release），產物在 workspace/target-play
#   tools/rust.sh play [參數...]   在本機 X11 桌面開視窗玩；原版目錄、HD 資料夾、設定目錄由本腳本帶入
#   tools/rust.sh play-test [參數...]  跑 rs2-play 的單元測試與 A6（SDL_VIDEODRIVER=dummy，需要原版檔）
#   tools/rust.sh play-gui-check <名稱> <秒數> [參數...]  Xvfb 下開視窗跑指定秒數，存 PNG 到 workspace/rs2/shots
#
# 專案目錄以唯讀掛載，輸出在 workspace/cargo、workspace/target、workspace/rs2。
# tools/rust.sh lock 產生 Cargo.lock，這是唯一可寫掛載專案根的子命令。
# play-* 另用 workspace/cargo-play（離線 crate 快取）與 workspace/target-play；
# crates/rs2-play/Cargo.lock 不存在時，play-build 會先以可寫掛載該目錄產生它。
# play-* 的映像 SDL_IMAGE：rs2-rust-sdl:1.99.1-r1 存在就用，否則唯讀使用本機的 mm45-rust-desktop:1.99.1-r3
# （Rust 1.99.1、libsdl2-dev 2.26.5、Xvfb）；環境變數 RS2_SDL_IMAGE 可覆寫。本腳本不修改、不重新標記映像。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="rust:1.99.1-slim-bookworm@sha256:24707e70a4f1e23d9a7b25202ec43126d28a52294b96aad5c670df1fd1f1d400"
WP="$ROOT/workspace"
ORIG="$ROOT/original/rs-2"
DOSRUN="$WP/dosrun"

mkdir -p "$WP/cargo" "$WP/target" "$WP/rs2"
test -d "$ORIG" || { echo "缺原版目錄 $ORIG，先跑 tools/run_all.sh" >&2; exit 1; }
test -d "$DOSRUN" || { echo "缺 $DOSRUN" >&2; exit 1; }

base=(docker run --rm -u "$(id -u):$(id -g)"
  --memory 4g --cpus 4 --pids-limit 256 --network none
  --log-opt max-size=10m --log-opt max-file=3
  -e CARGO_HOME=/cache -e CARGO_TARGET_DIR=/target -e HOME=/tmp
  -e RS2_ORIG=/orig -e RS2_DOSRUN=/dosrun -e RS2_OUT=/out
  -v "$WP/cargo:/cache" -v "$WP/target:/target" -v "$WP/rs2:/out"
  -v "$ORIG:/orig:ro" -v "$DOSRUN:/dosrun:ro" -w /project)

# HD 素材資料夾（選用）：存在才唯讀掛載，LayerSink 的 HD 驗收用 RS2_HD 讀它。
HD="$WP/hd/assets"
if test -d "$HD"; then
  base+=(-v "$HD:/hd:ro" -e RS2_HD=/hd)
fi

# rs2-play 用：選映像並準備共用參數。只有 play-* 子命令呼叫，其他子命令不受影響。
SDL_IMAGE=""
PLAY_CARGO="$WP/cargo-play"
PLAY_TARGET="$WP/target-play"
pbase=()
play_prepare() {
  if [ -n "${RS2_SDL_IMAGE:-}" ]; then
    SDL_IMAGE="$RS2_SDL_IMAGE"
  elif docker image inspect "rs2-rust-sdl:1.99.1-r1" >/dev/null 2>&1; then
    SDL_IMAGE="rs2-rust-sdl:1.99.1-r1"
  elif docker image inspect "mm45-rust-desktop:1.99.1-r3" >/dev/null 2>&1; then
    SDL_IMAGE="mm45-rust-desktop:1.99.1-r3"
    echo "play: 沒有 rs2-rust-sdl:1.99.1-r1，唯讀使用本機映像 $SDL_IMAGE" >&2
  else
    echo "找不到可用的 SDL 映像（rs2-rust-sdl:1.99.1-r1、mm45-rust-desktop:1.99.1-r3），可設 RS2_SDL_IMAGE" >&2
    exit 1
  fi
  docker image inspect "$SDL_IMAGE" >/dev/null 2>&1 || { echo "找不到映像 $SDL_IMAGE" >&2; exit 1; }
  test -d "$PLAY_CARGO/registry" || { echo "缺 $PLAY_CARGO/registry（離線 crate 快取）" >&2; exit 1; }
  mkdir -p "$PLAY_TARGET"
  pbase=(docker run --rm --init -u "$(id -u):$(id -g)"
    --memory 6g --cpus 4 --pids-limit 256 --network none
    --log-opt max-size=10m --log-opt max-file=3
    -e CARGO_HOME=/cache -e CARGO_TARGET_DIR=/target -e HOME=/tmp
    -v "$PLAY_CARGO:/cache" -v "$PLAY_TARGET:/target")
}

# 執行 rs2-play 時共用的唯讀素材掛載與環境（原版、收據、HD）。
play_data_mounts() {
  pdata=(-v "$ORIG:/orig:ro" -v "$DOSRUN:/dosrun:ro")
  phd=()
  if test -d "$HD"; then
    pdata+=(-v "$HD:/hd:ro")
    phd=(--hd /hd)
  fi
}

cmd="${1:?用法見檔頭}"; shift || true
case "$cmd" in
  lock)
    timeout 300 "${base[@]}" --name "rs2-lock-$$" -v "$ROOT:/project" "$IMAGE" \
      cargo generate-lockfile --offline
    ;;
  test)
    timeout 1800 "${base[@]}" --name "rs2-test-$$" -v "$ROOT:/project:ro" "$IMAGE" \
      cargo test --release --locked --offline "$@"
    ;;
  verify)
    timeout 1800 "${base[@]}" --name "rs2-verify-$$" -v "$ROOT:/project:ro" "$IMAGE" \
      cargo test --release --locked --offline -- --include-ignored --nocapture
    ;;
  trace)
    timeout 1800 "${base[@]}" --name "rs2-trace-$$" -v "$ROOT:/project:ro" "$IMAGE" \
      cargo run --release --locked --offline -p rs2-trace -- "$@"
    ;;
  cargo)
    timeout 1800 "${base[@]}" --name "rs2-cargo-$$" -v "$ROOT:/project:ro" "$IMAGE" \
      cargo "$@" --locked --offline
    ;;
  image-sdl)
    timeout 900 docker build -q -t rs2-rust-sdl:1.99.1-r1 -f "$ROOT/docker/rust-sdl.Dockerfile" "$ROOT/docker"
    ;;
  play-build)
    play_prepare
    if ! test -f "$ROOT/crates/rs2-play/Cargo.lock"; then
      # 只有 crates/rs2-play 可寫，其餘仍唯讀。
      timeout 300 "${pbase[@]}" --name "rs2-playlock-$$" -w /project/crates/rs2-play \
        -v "$ROOT:/project:ro" -v "$ROOT/crates/rs2-play:/project/crates/rs2-play" "$SDL_IMAGE" \
        cargo generate-lockfile --offline
    fi
    timeout 1800 "${pbase[@]}" --name "rs2-playbuild-$$" -w /project/crates/rs2-play \
      -v "$ROOT:/project:ro" "$SDL_IMAGE" cargo build --release --locked --offline
    ;;
  play-test)
    # 單元測試加 A6。A6 需要原版檔與收據，標 #[ignore]，所以帶 --include-ignored。
    play_prepare; play_data_mounts
    timeout 3000 "${pbase[@]}" --name "rs2-playtest-$$" -w /project/crates/rs2-play \
      -e SDL_VIDEODRIVER=dummy -e SDL_RENDER_DRIVER=software \
      -e RS2_ORIG=/orig -e RS2_DOSRUN=/dosrun -e RS2_OUT=/out ${phd:+-e RS2_HD=/hd} \
      -v "$ROOT:/project:ro" -v "$WP/rs2:/out" "${pdata[@]}" "$SDL_IMAGE" \
      cargo test --release --locked --offline "$@" -- --include-ignored --test-threads=1 --nocapture
    ;;
  play)
    # 在本機 X11 桌面開視窗玩，不設 timeout（玩家自己關）。設定與存檔在 workspace/rs2/play。
    [[ "${DISPLAY:-}" =~ ^:[0-9]+(\.[0-9]+)?$ ]] || { echo "請在已登入 X11／XWayland 桌面的終端執行（需要 DISPLAY）" >&2; exit 2; }
    test -d /tmp/.X11-unix || { echo "找不到 /tmp/.X11-unix" >&2; exit 1; }
    play_prepare; play_data_mounts
    test -x "$PLAY_TARGET/release/rs2-play" || { echo "先跑 tools/rust.sh play-build" >&2; exit 1; }
    mkdir -p "$WP/rs2/play"
    gpu=(); if test -c /dev/dri/renderD128; then gpu=(--device /dev/dri/renderD128 --group-add "$(stat -c %g /dev/dri/renderD128)"); fi
    auth=(); xa="${XAUTHORITY:-$HOME/.Xauthority}"; if test -f "$xa"; then auth=(-v "$xa:/tmp/xauth:ro" -e XAUTHORITY=/tmp/xauth); fi
    docker run --rm --init -u "$(id -u):$(id -g)" --name "rs2-play-$$" \
      --memory 6g --cpus 4 --pids-limit 256 --network none --read-only --tmpfs /tmp:rw,exec,size=256m \
      --cap-drop ALL --security-opt no-new-privileges \
      --log-opt max-size=10m --log-opt max-file=3 \
      -e HOME=/tmp -e DISPLAY -e SDL_VIDEODRIVER=x11 "${gpu[@]}" "${auth[@]}" \
      -v /tmp/.X11-unix:/tmp/.X11-unix:ro -v "$PLAY_TARGET/release:/tool:ro" "${pdata[@]}" \
      -v "$WP/rs2/play:/play" "$SDL_IMAGE" \
      /tool/rs2-play --orig /orig "${phd[@]}" --config /play/config "$@"
    ;;
  play-gui-check)
    # Xvfb 下開視窗跑指定秒數。--shot-at 在第 50、120 步各存一張 PNG（視窗內容含工具列），
    # 其餘參數照傳，例如 --replay /dosrun/boss1_kill/trace-s1.jsonl、--no-hd。
    name="${1:?輸出名}"; secs="${2:?秒數}"; shift 2
    [[ "$name" =~ ^[A-Za-z0-9_-]+$ ]] || { echo "輸出名只能用英數、底線、連字號" >&2; exit 2; }
    play_prepare; play_data_mounts
    test -x "$PLAY_TARGET/release/rs2-play" || { echo "先跑 tools/rust.sh play-build" >&2; exit 1; }
    mkdir -p "$WP/rs2/shots"
    timeout $((secs + 300)) docker run --rm --init -u "$(id -u):$(id -g)" --name "rs2-gui-$name-$$" \
      --memory 6g --cpus 4 --pids-limit 256 --network none --tmpfs /tmp:rw,exec,size=256m,mode=1777 \
      --cap-drop ALL --security-opt no-new-privileges \
      --log-opt max-size=10m --log-opt max-file=3 \
      -e HOME=/tmp -e SDL_VIDEODRIVER=x11 -e SDL_RENDER_DRIVER=software \
      -v "$PLAY_TARGET/release:/tool:ro" "${pdata[@]}" -v "$WP/rs2/shots:/shots" "$SDL_IMAGE" \
      sh -c 'mkdir -p -m 1777 /tmp/.X11-unix; Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp >/tmp/xvfb.log 2>&1 & xp=$!; sleep 2; DISPLAY=:99 "$@"; rc=$?; kill $xp 2>/dev/null; exit $rc' \
      sh /tool/rs2-play --orig /orig "${phd[@]}" --config /tmp/cfg --max-secs "$secs" \
        --shot-at "50:/shots/rs2play_${name}_s0050.png" --shot-at "120:/shots/rs2play_${name}_s0120.png" "$@"
    ;;
  *)
    echo "未知子命令 $cmd，用法見檔頭" >&2
    exit 2
    ;;
esac
