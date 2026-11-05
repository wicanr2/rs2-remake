#!/bin/sh
# 同一個執行檔，分別讀完整的 workspace/hd/assets 與發行包的資料子集（payload），同一步的畫面必須逐位元組相同。
# 證明被排除的檔案（overview、_final、maps、MAP/objects、各批次紀錄）執行時不會被讀。
set -u
export LD_LIBRARY_PATH=/sdl/lib SDL_VIDEODRIVER=dummy SDL_RENDER_DRIVER=software HOME=/tmp
BIN=/bin-rs2/rs2-play
fails=0
run() { # 標籤 資料來源(full|pay) 起點 步數 截圖步...
  tag="$1"; src="$2"; start="$3"; steps="$4"; shift 4
  if [ "$src" = full ]; then o="--orig /orig --hd /hd"; else o="--orig /payload/original --hd /payload/hd"; fi
  shots=""; for s in "$@"; do shots="$shots --shot-at $s:/out/$tag-$src-$s.png"; done
  $BIN $o --start "$start" --headless-steps "$steps" --no-audio --config /tmp/cfg-$tag-$src $shots >/out/$tag-$src.log 2>&1 || { echo "FAIL $tag/$src 結束碼非 0"; tail -5 /out/$tag-$src.log; fails=$((fails + 1)); }
  grep -E "\[hd\] 預載" /out/$tag-$src.log | cut -c1-120
  grep -c "缺圖：" /out/$tag-$src.log | xargs -I{} echo "$tag/$src 缺圖行數 {}"
}
cmp_set() { # 標籤 截圖步...
  tag="$1"; shift
  for s in "$@"; do
    a="$(sha256sum < /out/$tag-full-$s.png | cut -c1-16)"; b="$(sha256sum < /out/$tag-pay-$s.png | cut -c1-16)"
    if [ "$a" = "$b" ]; then echo "PASS $tag 第 $s 步：完整 HD 與發行包資料的畫面相同（$a）"; else echo "FAIL $tag 第 $s 步：畫面不同（$a / $b）"; fails=$((fails + 1)); fi
  done
}
for src in full pay; do run demo $src demo 700 100 300 500 690; done
cmp_set demo 100 300 500 690
for src in full pay; do run title $src title 700 120 250 400 650; done
cmp_set title 120 250 400 650
[ "$fails" -eq 0 ] && echo "== 結果：FAIL 0 項" || echo "== 結果：FAIL $fails 項"
[ "$fails" -eq 0 ]
