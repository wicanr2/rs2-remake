#!/bin/sh
# 在 Debian 12 容器內建 rs2-play（Linux x86_64）。SDL2 連到 /sdl 的自編版本，執行檔的 RUNPATH 是 $ORIGIN/../lib。
set -eu
: "${RS2_VERSION:?}"
export RUSTFLAGS='-L native=/sdl/lib -C link-arg=-Wl,-rpath,$ORIGIN/../lib'
cd /project/crates/rs2-play
cargo build --release --locked --offline
bin=/target/release/rs2-play
echo "--- NEEDED / RUNPATH"
readelf -d "$bin" | grep -E "NEEDED|RUNPATH|RPATH"
echo "--- 最高的 GLIBC 符號版本"
objdump -T "$bin" | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1
