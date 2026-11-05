#!/bin/sh
# 在交叉工具鏈容器內編 rs2-play 的 Windows 單元測試執行檔（不執行），複製到 /out 並附上 SDL2.dll，交給 Wine 驗證容器跑。
set -eu
cd /project/crates/rs2-play
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-Lnative=/opt/sdl2/windows/x86_64-w64-mingw32/lib"
cargo test --release --locked --offline --lib --no-run --target x86_64-pc-windows-gnu --message-format=json 2>/tmp/cargo-test.log > /tmp/cargo-test.json || { tail -30 /tmp/cargo-test.log; exit 1; }
exe="$(python3 -I -c '
import json,sys
for l in open("/tmp/cargo-test.json"):
    try: m=json.loads(l)
    except ValueError: continue
    if m.get("reason")=="compiler-artifact" and m.get("profile",{}).get("test") and m.get("executable") and m["target"]["name"] in ("rs2-play","rs2_play") and "lib" in m["target"]["kind"]:
        print(m["executable"])
')"
test -n "$exe"
cp "$exe" /out/rs2_play_tests.exe
cp /opt/sdl2/windows/x86_64-w64-mingw32/bin/SDL2.dll /out/
ls -l /out
