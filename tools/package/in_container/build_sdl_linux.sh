#!/bin/sh
# 在 Debian 12 容器內從官方原始碼編譯 SDL2 2.26.5。各個後端（X11、Wayland、PulseAudio、ALSA、libdecor、udev）
# 以 dlopen 載入，所以 libSDL2-2.0.so.0 只依賴 libc 一類的基礎函式庫，AppImage 在沒有這些函式庫的系統上也能啟動。
set -eu
rm -rf /tmp/sdl-build
mkdir -p /tmp/sdl-build
cd /tmp/sdl-build
tar xzf /inputs/SDL2-2.26.5.tar.gz
cd SDL2-2.26.5
./configure --prefix=/prefix --disable-static --enable-shared >/tmp/sdl-configure.log 2>&1 || { tail -60 /tmp/sdl-configure.log; exit 1; }
make -j"$(nproc)" >/tmp/sdl-make.log 2>&1 || { tail -60 /tmp/sdl-make.log; exit 1; }
make install >/tmp/sdl-install.log 2>&1 || { tail -60 /tmp/sdl-install.log; exit 1; }
echo "--- configure 摘要"
grep -E "^(Video drivers|Audio drivers|Input drivers)|^checking for (X11|Wayland|libdecor|PulseAudio|ALSA)" /tmp/sdl-configure.log | head -20 || true
grep -E "(x11|wayland|pulse|alsa|libdecor|udev).*(yes|no)$" /tmp/sdl-configure.log | head -20 || true
echo "--- NEEDED"
readelf -d /prefix/lib/libSDL2-2.0.so.0 | grep NEEDED
# 去掉除錯符號（編譯預設帶 -g，約 10 MB 變 2 MB 左右）。
strip --strip-unneeded /prefix/lib/libSDL2-2.0.so.0.2600.5
ls -l /prefix/lib/libSDL2-2.0.so.0.2600.5
