#!/usr/bin/env bash
# 全流程對拍收據清單：workspace/dosrun/wt_* 的收據檔逐檔 sha256，寫到 workspace/dosrun/MANIFEST-wait.sha256
# （不動 MANIFEST.sha256 與 MANIFEST-trace.sha256）。同一份腳本在兩個獨立容器重跑，雜湊應相同
# （wait-*.jsonl、.dsd、.pokes、entry.*、biosfont.bin、events.tsv 都不含牆鐘）。
#   tools/dos/hash_wait_receipts.sh [目錄前綴，預設 wt_]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT/workspace/dosrun"
PFX="${1:-wt_}"
shopt -s nullglob
dirs=("$PFX"*)
(( ${#dirs[@]} > 0 )) || { echo "hash_wait_receipts.sh：沒有 $PFX* 目錄" >&2; exit 1; }
for d in "${dirs[@]}"; do
  [[ -d "$d" ]] && find "$d" -type f \( -name 'wait-*' -o -name 'entry.*' -o -name 'biosfont.bin' -o -name 'events.tsv' -o -name 'trace-*.jsonl' -o -name 'stdout.txt' \)
done | LC_ALL=C sort | xargs sha256sum > MANIFEST-wait.sha256
echo "MANIFEST-wait.sha256：$(wc -l < MANIFEST-wait.sha256) 檔，sha256 $(sha256sum MANIFEST-wait.sha256 | cut -d' ' -f1)"
echo "rs2dyn $(sha256sum "$ROOT/workspace/dosgolem/workplace/bin/rs2dyn" | cut -d' ' -f1)"
