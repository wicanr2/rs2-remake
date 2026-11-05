#!/usr/bin/env bash
# 用 Nuked OPL3（DOSBox-X 原始碼內的 nukedopl.cpp，唯讀掛載，不進這個 repo）把暫存器序列合成成參考 PCM。
#   tools/audio/nuked_ref.sh NAME...   讀 workspace/rs2/audio/NAME.ops，寫 NAME.nuked.pcm
# 映像沿用本機的 mm45-rust-desktop（內含 gcc），不修改、不重新建置。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
NUKED="${NUKED_DIR:-$HOME/cht/DOSBox-X-MCP-Debugger/dosbox-src/src/hardware}"
test -f "$NUKED/nukedopl.cpp" || { echo "缺 $NUKED/nukedopl.cpp" >&2; exit 1; }
OUT="$ROOT/workspace/rs2/audio"
test -d "$OUT" || { echo "缺 $OUT，先跑 audio_write_ops" >&2; exit 1; }
exec timeout 900 docker run --rm --network none -u "$(id -u):$(id -g)" --name "rs2-nuked-$$" \
  --memory 1g --cpus 2 --pids-limit 32 --log-opt max-size=10m --log-opt max-file=3 \
  -v "$ROOT/tools/audio:/src:ro" -v "$NUKED:/nuked:ro" -v "$OUT:/out" mm45-rust-desktop:1.99.1-r3 sh -c '
    set -e
    gcc -O2 -x c "-DNUKED_C=\"/nuked/nukedopl.cpp\"" -I/nuked /src/nuked_ref.c -lm -o /tmp/nuked_ref
    for n in "$@"; do /tmp/nuked_ref < /out/$n.ops > /out/$n.nuked.pcm; ls -la /out/$n.nuked.pcm; done
  ' sh "$@"
