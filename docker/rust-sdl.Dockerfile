# rs2-play 的工具鏈：Rust 1.99.1 加 SDL2 開發檔。其他 crate 用基底映像就夠。
# 建置：tools/rust.sh image-sdl（需要網路）
FROM rust:1.99.1-slim-bookworm@sha256:24707e70a4f1e23d9a7b25202ec43126d28a52294b96aad5c670df1fd1f1d400
RUN apt-get update \
 && apt-get install -y --no-install-recommends libsdl2-dev=2.26.5+dfsg-1 \
 && rm -rf /var/lib/apt/lists/*
