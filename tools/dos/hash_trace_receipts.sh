#!/usr/bin/env bash
# 逐幀收據清單：docs/re/10-dynamic-receipts.md 第 8 至 10 節引用的收據逐檔 sha256，
# 寫到 workspace/dosrun/MANIFEST-trace.sha256（不動 MANIFEST.sha256）。
# 同一組輸入重跑應得到相同的雜湊（trace-*.jsonl 含模擬器步數 emu.step，也相同）。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT/workspace/dosrun"
DIRS=(trace_dm1 trace_dm1_run2 boss1_kill boss1_kill_run2 trace_stage1 trace_stage2 trace_stage3 trace_stage4 defect_armor defect_seeker_leak defect_seeker_ctrl defect_slot_carry)
for d in "${DIRS[@]}"; do test -d "$d" || { echo "hash_trace_receipts.sh：缺 $d" >&2; exit 1; }; done
{
  for d in "${DIRS[@]}"; do find "$d" -type f \( -name '*.png' -o -name '*.json' -o -name '*.jsonl' -o -name 'events.tsv' -o -name 'stdout.txt' \); done
  ls sheets/boss1_kill_*.png
} | LC_ALL=C sort | xargs sha256sum > MANIFEST-trace.sha256
echo "MANIFEST-trace.sha256：$(wc -l < MANIFEST-trace.sha256) 檔，sha256 $(sha256sum MANIFEST-trace.sha256 | cut -d' ' -f1)"
echo "S.EXE $(sha256sum "$ROOT/original/rs-2/S.EXE" | cut -d' ' -f1)"
echo "ASMDRV.EXE $(sha256sum "$ROOT/original/rs-2/ASMDRV.EXE" | cut -d' ' -f1)"
echo "rs2dyn $(sha256sum "$ROOT/workspace/dosgolem/workplace/bin/rs2dyn" | cut -d' ' -f1)"
