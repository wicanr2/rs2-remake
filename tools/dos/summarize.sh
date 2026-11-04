#!/usr/bin/env bash
# 摘要一份 events.tsv：畫面對具名調色盤、時基量測、PIT 變化、開檔序列、寫檔與結束。
#   tools/dos/summarize.sh <workspace/dosrun 下的輸出名> [shots|measure|pit|open|misc]...
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
E="$ROOT/workspace/dosrun/$1/events.tsv"; shift
test -f "$E" || { echo "找不到 $E" >&2; exit 1; }
want=("$@"); [[ ${#want[@]} -eq 0 ]] && want=(shots measure pit open misc)
for w in "${want[@]}"; do
  echo "## $w"
  case "$w" in
    shots) awk -F'\t' '$4=="shot"{split($5,a," "); m=$5; d=m; sub(/.*exact_dac:\[/,"",d); sub(/\].*/,"",d); t=m; sub(/.*exact_attr:\[/,"",t); sub(/\].*/,"",t); printf "%9.3fs  %-34s dac=[%s] attr=[%s]\n",$3,a[1],d,t}' "$E" ;;
    measure) awk -F'\t' '$4=="measure"{print $3"s", $5}' "$E" ;;
    pit) awk -F'\t' '$4=="pit"{printf "%9.3fs step %-12s %s\n",$3,$1,$5}' "$E" ;;
    open) awk -F'\t' '$4=="open"||$4=="missing"{printf "%9.3fs step %-12s %-7s %s\n",$3,$1,$4,$5}' "$E" ;;
    misc) awk -F'\t' '$4=="peek"||$4=="wrote"||$4=="note"||$4=="exit"||$4=="trapped"||$4=="summary"&&($5 ~ /^steps|^wrote|^unimpl/){printf "%9.3fs %-8s %s\n",$3,$4,$5}' "$E" ;;
  esac
done
