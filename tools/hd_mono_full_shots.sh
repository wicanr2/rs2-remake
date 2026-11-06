#!/usr/bin/env bash
# 單色 HD 全量候選的實機畫面：用發行包的 Linux AppImage，在乾淨容器的 Xvfb 內，對每個 HD 根目錄、每個顯示模式截同一步的畫面。
#   tools/hd_mono_full_shots.sh "<模式...>" <標籤...>
#   標籤 SD   = --no-hd（原版單色圖）
#   標籤 A    = --hd /hd/assets（現況單色 HD）
#   其他標籤  = --hd /hd/mono-m2-review/root-<標籤>（hd_mono_root.py 組的暫存根目錄：mono 指向候選，其他連到 assets）
# 例：tools/hd_mono_full_shots.sh "cga mga" SD A cand
# 輸出 workspace/hd/mono-m2-review/shots/<模式>-<標籤>-<場景>-<步數>.png，場景 flow（標題、四關、片尾）與 open（開場動畫與標題）。
# 規則同 AGENTS.md 硬規則 1 到 3：docker run --rm、目前 UID/GID、--memory、--cpus（4，同一時間只跑一個容器）、--pids-limit、log 輪替、--network none、
# 外層 timeout、容器名 rs2-mono- 前綴、只讀掛載原始輸入、掛載前確認來源存在。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-v.1.0.0-20261106}"
IMG="yuan-wine-verify:noble-r2"
MODES="${1:?顯示模式，例如 \"cga mga\"}"
shift
[ "$#" -gt 0 ] || { echo "要至少一個標籤" >&2; exit 1; }
DIST="$ROOT/dist-all/$VERSION/full"
HD="$ROOT/workspace/hd"
REPLAYS="$ROOT/workspace/promo/replays"
OUT="$HD/mono-m2-review/shots"
INNER="$ROOT/tools/hd_mono_full_shots_inner.sh"
[ -f "$DIST/RS-2-remake-$VERSION-full-linux-x86_64.AppImage" ] || { echo "缺 AppImage：$DIST" >&2; exit 1; }
[ -f "$REPLAYS/flow-play.jsonl" ] || { echo "缺 $REPLAYS/flow-play.jsonl（tools/promo/gen_replays.sh 產生）" >&2; exit 1; }
[ -d "$HD/assets/mono" ] && [ -f "$INNER" ] || { echo "缺 assets/mono 或 inner 腳本" >&2; exit 1; }
for l in "$@"; do
  case "$l" in SD | A) ;; *) [ -d "$HD/mono-m2-review/root-$l/mono" ] || { echo "缺 root-$l（先跑 hd_mono_root.py）" >&2; exit 1; } ;; esac
done
docker image inspect "$IMG" > /dev/null 2>&1 || { echo "找不到映像 $IMG" >&2; exit 1; }
mkdir -p "$OUT"
[ "$(stat -c %u "$OUT")" = "$(id -u)" ] || { echo "$OUT 不是目前使用者所有" >&2; exit 1; }
timeout --kill-after=30 5400 docker run --rm --init --name "rs2-mono-fshots-$$" -u "$(id -u):$(id -g)" \
  --memory 4g --cpus 4 --pids-limit 1024 --network none --log-opt max-size=10m --log-opt max-file=3 \
  --entrypoint sh --tmpfs /tmp:rw,exec,size=2g,mode=1777 -w /tmp \
  -v "$DIST:/dist:ro" -v "$HD:/hd:ro" -v "$REPLAYS:/replays:ro" -v "$INNER:/inner.sh:ro" -v "$OUT:/out" \
  -e "VERSION=$VERSION" -e "LABELS=$*" -e "MODES=$MODES" -e "SCENES=${SCENES:-flow open}" \
  "$IMG" /inner.sh 2>&1 | tee "$OUT/shots-$(date +%H%M%S).log"
find "$OUT" -user root | head -3
