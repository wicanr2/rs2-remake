#!/usr/bin/env bash
# 對批次目錄裡還沒有原稿的參考表呼叫 Codex。只有這支在主機跑，因為 Codex 本身是外部服務入口。
#   tools/hd_run.sh workspace/hd/<批次> [同時幾張]
# 批次目錄要有 ref/<名>.png 與 prompts/<名>.txt。已有 masters/<名>.png 的不再呼叫，不覆寫、不刪任何檔。
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ ${1:-} == one ]]; then
  b="$2" name="$3"
  log="$b/logs/$name.log"
  rc=0
  images=(-i "$b/ref/$name.png")
  # 第二張參考圖（已畫好的相鄰面板）
  [[ -f $b/ref/$name.extra.png ]] && images+=(-i "$b/ref/$name.extra.png")
  timeout 900 codex exec --skip-git-repo-check -s read-only -C "$b/ref" "${images[@]}" \
    -o "$b/logs/$name.last.txt" - < "$b/prompts/$name.txt" > "$log" 2>&1 || rc=$?
  sid=$(grep -m1 -o 'session id: [0-9a-f-]*' "$log" | cut -d' ' -f3 || true)
  shopt -s nullglob
  imgs=("$HOME/.codex/generated_images/$sid"/*.png)
  if [[ -z ${sid:-} || ${#imgs[@]} -eq 0 ]]; then
    echo "FAIL $name rc=$rc session=${sid:-無}" | tee -a "$b/logs/_status.txt"
    exit 0
  fi
  cp --update=none "${imgs[0]}" "$b/masters/$name.png"
  printf '%s\t%s\t%s\t%s\n' "$name" "$sid" "$(sha256sum "$b/masters/$name.png" | cut -d' ' -f1)" "$(codex --version)" >> "$b/logs/_receipts.tsv"
  echo "OK $name $(sha256sum "$b/masters/$name.png" | cut -c1-16)" | tee -a "$b/logs/_status.txt"
  exit 0
fi

rel=${1:?批次目錄}
jobs=${2:-2}
[[ $rel == workspace/hd/* && $rel != *..* ]] || { echo "批次目錄要在 workspace/hd/ 底下" >&2; exit 1; }
b="$root/$rel"
test -d "$b/ref" && test -d "$b/prompts" || { echo "缺 ref/ 或 prompts/" >&2; exit 1; }
mkdir -p "$b/logs" "$b/masters"
todo=()
for p in "$b"/prompts/*.txt; do
  name=$(basename "$p" .txt)
  [[ -f $b/masters/$name.png ]] || todo+=("$name")
done
echo "要生成 ${#todo[@]} 張"
if (( ${#todo[@]} > 0 )); then
  printf '%s\n' "${todo[@]}" | xargs -P "$jobs" -I{} "$root/tools/hd_run.sh" one "$b" {}
fi
echo "完成：$(find "$b/masters" -name '*.png' | wc -l) 張原稿"
