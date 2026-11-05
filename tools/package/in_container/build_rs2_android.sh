#!/bin/sh
# 在 rs2-android 映像內建 Android 的原生部分與 Java 部分。輸入來源都唯讀。
# 環境變數：ABIS（空白分隔）。輸出到 /out/lib/<abi>/*.so 與 /out/classes.dex。
set -eu
: "${ABIS:?}"
NDK=/opt/android-sdk/ndk/27.2.12479018
API=23
SDK=/opt/android-sdk
JAR="$SDK/platforms/android-35/android.jar"

# SDL2：快取在 /cache/sdl，原始碼雜湊與 ABI 不變就不重編。
stamp="$(sha256sum /inputs/SDL2-2.26.5.tar.gz | cut -d' ' -f1) $ABIS"
if [ "$(cat /cache/sdl/.stamp 2>/dev/null || true)" != "$stamp" ]; then
  rm -rf /cache/sdl
  mkdir -p /cache/sdl/src
  tar -xzf /inputs/SDL2-2.26.5.tar.gz -C /cache/sdl/src --strip-components=1
  cd /cache/sdl/src
  # NDK 27 移除了 ALooper_pollAll，SDL 之後的版本也改用 ALooper_pollOnce。SDL 只在逾時 0 的輪詢用它，兩者行為相同。
  grep -rl ALooper_pollAll src | xargs sed -i 's/ALooper_pollAll/ALooper_pollOnce/g'
  ndk-build -j4 \
    NDK_PROJECT_PATH=. APP_BUILD_SCRIPT=./Android.mk \
    APP_ABI="$ABIS" APP_PLATFORM=android-$API APP_MODULES=SDL2 APP_STL=c++_static \
    NDK_OUT=/cache/sdl/obj NDK_LIBS_OUT=/cache/sdl/libs >/cache/sdl/build.log 2>&1 \
    || { tail -30 /cache/sdl/build.log; exit 1; }
  echo "$stamp" > /cache/sdl/.stamp
fi

# rs2-play 與 libmain.so。
toolbin="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CC_aarch64_linux_android="$toolbin/aarch64-linux-android$API-clang"
export CC_x86_64_linux_android="$toolbin/x86_64-linux-android$API-clang"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$CC_x86_64_linux_android"
cd /project/crates/rs2-android
for abi in $ABIS; do
  case "$abi" in
    arm64-v8a) triple=aarch64-linux-android ;;
    x86_64) triple=x86_64-linux-android ;;
    *) echo "不支援的 ABI：$abi" >&2; exit 2 ;;
  esac
  RUSTFLAGS="-L native=/cache/sdl/libs/$abi" cargo build --release --locked --offline --target "$triple"
  mkdir -p "/out/lib/$abi"
  cp "/target/$triple/release/libmain.so" "/out/lib/$abi/libmain.so"
  cp "/cache/sdl/libs/$abi/libSDL2.so" "/out/lib/$abi/libSDL2.so"
  "$toolbin/llvm-strip" --strip-unneeded "/out/lib/$abi/libmain.so"
  "$toolbin/llvm-readelf" -d "/out/lib/$abi/libmain.so" | grep NEEDED
done

# Java：SDL 的 SDLActivity 等加上我們的兩個 Activity，不用 Gradle。
mkdir -p /out/classes /out/dex
find /cache/sdl/src/android-project/app/src/main/java /android/java -name '*.java' > /out/java-sources.txt
javac -encoding UTF-8 -nowarn -source 8 -target 8 -classpath "$JAR" -d /out/classes @/out/java-sources.txt 2>&1 | grep -v '^Note:\|^warning:\|^[0-9]* warning' || true
find /out/classes -name '*.class' > /out/class-files.txt
d8 --min-api $API --lib "$JAR" --output /out/dex @/out/class-files.txt
ls -l /out/dex
