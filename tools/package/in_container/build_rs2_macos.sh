#!/bin/sh
# 在 osxcross 容器內建 rs2-play 的 macOS x86_64 與 arm64，再用 lipo 合成 universal。
# 連結設定來自映像的 CARGO_TARGET_*_RUSTFLAGS：SDL2 連到 /opt/sdl2/macos/lib/libSDL2.dylib（指向官方 SDL2.framework），
# 執行檔 rpath 是 @executable_path/../Frameworks，所以 .app 的 Contents/Frameworks/SDL2.framework 會被找到。
set -eu
: "${RS2_VERSION:?}"
eval "$(osxcross-conf)"
T="x86_64-apple-$OSXCROSS_TARGET"
cd /project/crates/rs2-play
for t in x86_64-apple-darwin aarch64-apple-darwin; do
  cargo build --release --locked --offline --target "$t"
done
mkdir -p /out
lipo -create -output /out/rs2-play \
  /target/x86_64-apple-darwin/release/rs2-play /target/aarch64-apple-darwin/release/rs2-play
chmod 755 /out/rs2-play
echo "--- lipo"
lipo -info /out/rs2-play
for a in x86_64 arm64; do
  lipo -thin "$a" /out/rs2-play -output "/tmp/rs2-$a"
  echo "--- $a"
  "$T-otool" -l "/tmp/rs2-$a" | grep -E "minos|LC_CODE_SIGNATURE|LC_RPATH" -A2 | grep -E "minos|LC_CODE_SIGNATURE|path " || true
  "$T-otool" -L "/tmp/rs2-$a" | tail -n +2
done
