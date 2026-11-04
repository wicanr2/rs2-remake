#!/usr/bin/env bash
# 收據雜湊：workspace/dosrun 下的 PNG、JSON、TSV、TXT 逐檔 sha256，寫到 workspace/dosrun/MANIFEST.sha256。
# 另印原版輸入與 rs2dyn 的雜湊。events.tsv 的第一欄是步數，同一組輸入重跑應得到相同的雜湊。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT/workspace/dosrun"
find . -type f \( -name '*.png' -o -name '*.json' -o -name 'events.tsv' -o -name 'trace-*.tsv' -o -name '*.txt' \) ! -path './scratch*' ! -path './sheets/*' ! -name 'stdout.txt' | LC_ALL=C sort | xargs sha256sum > MANIFEST.sha256
echo "MANIFEST.sha256：$(wc -l < MANIFEST.sha256) 檔，sha256 $(sha256sum MANIFEST.sha256 | cut -d' ' -f1)"
echo "S.EXE $(sha256sum "$ROOT/original/rs-2/S.EXE" | cut -d' ' -f1)"
echo "ASMDRV.EXE $(sha256sum "$ROOT/original/rs-2/ASMDRV.EXE" | cut -d' ' -f1)"
echo "rs2dyn $(sha256sum "$ROOT/workspace/dosgolem/workplace/bin/rs2dyn" | cut -d' ' -f1)"
