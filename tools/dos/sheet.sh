#!/usr/bin/env bash
# 把 workspace/dosrun 下的多張 PNG 拼成一張總覽。
#   tools/dos/sheet.sh <輸出名> <欄數> <相對 workspace/dosrun 的 PNG 路徑（不含 .png）>...
# 輸出到 workspace/dosrun/sheets/<輸出名>.png。（rs2dyn 的 sheet 命令；不跑任何指令）
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
NAME="$1"; COLS="$2"; shift 2
mkdir -p "$ROOT/workspace/dosrun/sheets"
TMP="$ROOT/workspace/dosrun/sheets/.sheet-$NAME.txt"
{ printf 'sheet %s %s' "$NAME" "$COLS"; for f in "$@"; do printf ' ../%s' "$f"; done; printf '\n'; } > "$TMP"
RS2_EXTRA_MOUNT="" "$ROOT/tools/dos/dd.sh" sheet 120 rs2dyn -root /orig -out /out/sheets -script "/out/sheets/.sheet-$NAME.txt" > /dev/null
rm -f "$TMP"
echo "$ROOT/workspace/dosrun/sheets/$NAME.png"
