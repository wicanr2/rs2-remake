# Android 建置映像：rich2-go-android（NDK 27.2、SDK 35、JDK 17）加 Rust 1.99.1 與兩個 Android 目標的標準庫。
# 建 APK 的流程不用 Gradle（tools/package/android_apk.sh），所以建好之後可以斷網跑。
#
# 建置要網路，只為了下載 rust-std：
#   docker build --network host -f docker/android.Dockerfile -t rs2-android:1.99.1-r1 docker
FROM rust:1.99.1-slim-bookworm@sha256:24707e70a4f1e23d9a7b25202ec43126d28a52294b96aad5c670df1fd1f1d400 AS rust

FROM rich2-go-android:20261102-r1
COPY --from=rust /usr/local/cargo /usr/local/cargo
COPY --from=rust /usr/local/rustup /usr/local/rustup
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:/opt/android-sdk/ndk/27.2.12479018:/opt/android-sdk/build-tools/35.0.0:/opt/android-sdk/platform-tools:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

# 雜湊取自 static.rust-lang.org/dist/rust-std-1.99.1-<目標>.tar.xz.sha256。
RUN set -eu; \
    sysroot="$(rustc --print sysroot)"; \
    for pair in \
      "aarch64-linux-android 7463b06bb5f734c02d3bac967538b4de4da910fb90668398bc21b10b5620e35d" \
      "x86_64-linux-android 48132cd1f40a8112345ead45e8483495ab48285f152100f3ae1b1190bed4d8d4"; do \
        set -- $pair; t=$1; h=$2; f="rust-std-1.99.1-$t.tar.xz"; \
        curl -fsSL -o "/tmp/$f" "https://static.rust-lang.org/dist/$f"; \
        echo "$h  /tmp/$f" | sha256sum -c -; \
        mkdir -p "/tmp/$t"; tar -xJf "/tmp/$f" -C "/tmp/$t" --strip-components=1; \
        "/tmp/$t/install.sh" --prefix="$sysroot" --components="rust-std-$t" --disable-ldconfig >/dev/null; \
        rm -rf "/tmp/$f" "/tmp/$t"; \
    done; \
    rustc --version; \
    ls "$sysroot/lib/rustlib"
