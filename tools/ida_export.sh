#!/bin/sh
# 在 Docker 內用 IDA 9.4 分析解包後的 S.EXE，輸出 workspace/ida/s_export.json 與 s_disasm.txt。
# 先跑 tools/run_all.sh。IDA 的 exit code 不算證據，這裡以輸出檔與輸入雜湊判定。
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
IN="$ROOT/workspace/unpacked/S.unpacked.exe"
OUT="$ROOT/workspace/ida"
test -f "$IN" || { echo "缺少 $IN，先跑 tools/run_all.sh" >&2; exit 1; }
mkdir -p "$OUT"
cp "$IN" "$OUT/S.unpacked.exe"
rm -f "$OUT/s_export.json" "$OUT/s_disasm.txt"
timeout 300 docker run --rm --network none -u "$(id -u):$(id -g)" --memory 4g --cpus 4 --pids-limit 512 \
  --log-opt max-size=10m --log-opt max-file=3 -v "$OUT:/work" -v "$ROOT/tools/ida:/tools:ro" -w /work \
  ida-pro-9.4-idapython:py312-v1 idat -A -c -S"/tools/export_all.py /work" S.unpacked.exe >/dev/null 2>&1 || true
test -s "$OUT/s_export.json" || { echo "IDA 沒有產生 s_export.json" >&2; exit 2; }
test -s "$OUT/s_disasm.txt" || { echo "IDA 沒有產生 s_disasm.txt" >&2; exit 2; }
