#!/bin/sh
# 在交叉工具鏈容器內建 rs2-play.exe（x86_64-pc-windows-gnu）。圖示與版本資訊用 windres 編成物件後連進去。
# 映像預設的 RUSTFLAGS 只有 SDL2 的庫路徑，這裡補上資源物件。連結成主控台程式：錯誤訊息與 [init] 一類的日誌看得到。
set -eu
: "${RS2_VERSION:?}"
mkdir -p /tmp/res
cp /icons/rs2.ico /icons/rs2.rc /tmp/res/
cd /tmp/res
x86_64-w64-mingw32-windres rs2.rc -O coff -o /tmp/res/rs2.res.o
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-Lnative=/opt/sdl2/windows/x86_64-w64-mingw32/lib -C link-arg=/tmp/res/rs2.res.o"
cd /project/crates/rs2-play
cargo build --release --locked --offline --target x86_64-pc-windows-gnu
exe=/target/x86_64-pc-windows-gnu/release/rs2-play.exe
echo "--- DLL 相依"
x86_64-w64-mingw32-objdump -p "$exe" | grep "DLL Name" | sort -u
echo "--- 子系統與資源"
x86_64-w64-mingw32-objdump -p "$exe" | grep -E "Subsystem|MajorOSVersion" | head -3
x86_64-w64-mingw32-objdump -h "$exe" | grep -E "\.rsrc"
