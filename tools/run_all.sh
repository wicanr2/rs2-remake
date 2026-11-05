#!/bin/sh
# 從 RS-2_DOS_EN.zip 重建 original/ 與 workspace/（不含 IDA 資料庫）。全部在 Docker 內執行，主機只呼叫 docker。
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
ZIP="$ROOT/RS-2_DOS_EN.zip"
IMG=python:3.12-slim
test -f "$ZIP" || { echo "缺少 $ZIP" >&2; exit 1; }
mkdir -p "$ROOT/original" "$ROOT/workspace/unpacked" "$ROOT/workspace/extracted"
run() {
  docker run --rm --network none -u "$(id -u):$(id -g)" --memory 2g --cpus 4 --pids-limit 128 \
    --log-opt max-size=10m --log-opt max-file=3 -e PYTHONDONTWRITEBYTECODE=1 \
    -v "$ZIP:/zip/RS-2_DOS_EN.zip:ro" -v "$ROOT/tools:/tools:ro" -v "$ROOT/original:/original" \
    -v "$ROOT/workspace:/w" "$IMG" timeout 600 "$@"
}
run python -I -c "
import zipfile
z = zipfile.ZipFile('/zip/RS-2_DOS_EN.zip')
if z.testzip() is not None:
    raise SystemExit('zip 損壞')
for i in z.infolist():
    if i.filename.startswith('/') or '..' in i.filename:
        raise SystemExit('不安全路徑: ' + i.filename)
    z.extract(i, '/original')
"
run python -I /tools/unpack_exe.py /original/rs-2 /w/unpacked > "$ROOT/workspace/unpacked/log.txt"
run python /tools/extract_assets.py /original/rs-2 /w/unpacked /w/extracted
run python /tools/parse_mix.py /original/rs-2 /w/extracted/music
run python /tools/mix_to_vgm.py /original/rs-2 /w/extracted/music
run python /tools/export_tables.py /w/unpacked/S.unpacked.bin /w/extracted/stage-tables.json
run python /tools/export_entities.py /w/unpacked/S.unpacked.bin /w/extracted/entities.json
run python /tools/export_sfx.py /w/unpacked/S.unpacked.bin /original/rs-2 /w/extracted/sfx
run python /tools/extract_intro.py /w/unpacked/S.layer1.bin /w/extracted/intro
run python /tools/extract_textmode.py /w/unpacked/S.unpacked.bin /w/extracted/textmode
mkdir -p "$ROOT/workspace/rs2"
run python /tools/export_pixel_hashes.py /original/rs-2 /w/rs2/pixel-hashes.tsv
# 用途表要讀 IDA 匯出，先跑過 tools/ida_export.sh 才有
if test -f "$ROOT/workspace/ida/s_disasm.txt"; then
  run python /tools/export_sprite_usage.py /w /w/extracted/sprite-usage.json
else
  echo "略過用途表：缺 workspace/ida/s_disasm.txt，先跑 tools/ida_export.sh" >&2
fi
if test "$(find "$ROOT/workspace" "$ROOT/original" -user root | wc -l)" != 0; then
  echo "警告：發現 root 擁有的檔案" >&2
  exit 2
fi
