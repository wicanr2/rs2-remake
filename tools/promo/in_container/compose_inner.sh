#!/bin/bash
# 容器內：字卡與字幕素材、配樂、各段影片、串接、驗收抽幀。環境由 compose.sh 帶入：
#   /w（可寫，workspace/promo）、/ptools（唯讀，tools/promo）、/rsaudio（唯讀，原版 AdLib 合成的 PCM）、
#   /android（唯讀，Android 實機截圖）、/outd（可寫，dist-all/<版本>/promo）、VERSION。
# 用法：compose_inner.sh 階段...    階段：assets audio segs final review
set -eu
. /ptools/theme.sh
export LC_ALL=C.UTF-8 HOME=/tmp XDG_CACHE_HOME=/tmp/cache
mkdir -p /tmp/cache
W=/w; A=$W/assets; S=$W/segs; C=$W/clips
mkdir -p "$A" "$S" "$W/review"
FF="ffmpeg -nostdin -hide_banner -loglevel error -y"
ENC="-c:v libx264 -preset veryfast -crf 17 -pix_fmt yuv420p -r $FPS -threads 2 -an"
hex() { echo "0x${1#\#}"; }
ACC=$(hex "$ACCENT"); DEEP=$(hex "$BG_DEEP")

t0() { cat "$C/$1.t0" 2>/dev/null || echo 0; }
T0_title=$(t0 title-1)
T0_s1modes=$(t0 s1modes-1)
T0_sbshd=$(t0 sbshd-1)
T0_sbsorig=$(t0 sbsorig-1)
. /ptools/cuts.sh

add() { awk -v a="$1" -v b="$2" 'BEGIN{printf "%.3f", a + b}'; }
png_h() { identify -format %h "$1"; }
png_w() { identify -format %w "$1"; }

# 淡入秒 淡出秒 總秒 -> fade 濾鏡串（結尾含 format=yuv420p）
fadestr() {
  local fin="$1" fout="$2" dur="$3" parts=() st
  if awk -v x="$fin" 'BEGIN{exit !(x>0)}'; then parts+=("fade=t=in:st=0:d=$fin"); fi
  if awk -v x="$fout" 'BEGIN{exit !(x>0)}'; then
    st=$(awk -v d="$dur" -v f="$fout" 'BEGIN{printf "%.3f", d - f}')
    parts+=("fade=t=out:st=$st:d=$fout")
  fi
  parts+=("format=yuv420p")
  local IFS=,
  echo "${parts[*]}"
}

# ---------------------------------------------------------------- 素材
txt() { # 輸出 字型 字級 顏色 文字
  convert -background none -fill "$4" -font "$2" -pointsize "$3" -interline-spacing 10 -gravity center label:"$5" \
    \( +clone -background black -shadow 75x4+0+3 \) +swap -background none -layers merge +repage "$1"
}
stack() { # 輸出 間距 PNG...（垂直置中排列）
  local out="$1" gap="$2"; shift 2
  local args=() p first=1
  for p in "$@"; do
    if [ "$first" = 1 ]; then first=0; else args+=("(" -size "1x$gap" xc:none ")"); fi
    args+=("$p")
  done
  convert "${args[@]}" -background none -gravity center -append "$out"
}
label_plate() { # 輸出 文字 顏色 寬
  txt "$A/_lbl.png" "$FONT_BLACK" 76 "$3" "$2"
  convert -size "${4}x112" xc:none -fill '#130404D0' -stroke "$ACCENT" -strokewidth 3 \
    -draw "roundrectangle 2,2 $(($4 - 3)),109 12,12" "$A/_lbl.png" -gravity center -composite "$1"
}

assets() {
  echo "[assets] 底圖與字幕 PNG"
  # 掃描線：1x4 的灰階圖塊（兩列變暗），用 multiply 疊上去。tile: 會丟掉 alpha，所以不用透明圖塊。
  convert -size 1x4 xc:white -fill '#A8A8A8' -draw 'rectangle 0,0 0,1' "$A/_scan.png"
  scan() { convert "$1" \( -size ${VW}x${VH} tile:"$A/_scan.png" \) -compose multiply -composite "$2"; }
  # 字卡 1：熔岩，深紅放射加底部琥珀光
  convert -size ${VW}x${VH} radial-gradient:'#6a160c-#130404' \
    \( -size ${VW}x380 gradient:'#F0992300-#F0992358' \) -gravity south -compose over -composite "$A/_bg1.png"
  scan "$A/_bg1.png" "$A/bg_card1.png"
  # 字卡 2：標題畫面的藍紫夜空，底部同色琥珀細光
  convert -size ${VW}x${VH} radial-gradient:'#26357f-#0b1030' \
    \( -size ${VW}x300 gradient:'#F0992300-#F0992330' \) -gravity south -compose over -composite "$A/_bg2.png"
  scan "$A/_bg2.png" "$A/bg_card2.png"
  # 遊玩版面的側邊與背景：深紅黑加掃描線
  convert -size ${VW}x${VH} radial-gradient:'#3c0c07-#130404' "$A/_bg3.png"
  scan "$A/_bg3.png" "$A/bg_side.png"

  # 字卡
  txt "$A/c1_main.png" "$FONT_BLACK" 132 "$ACCENT_HI" "1990 年，大宇資訊"
  txt "$A/c1_small.png" "$FONT_BOLD" 50 "$TEXT" "一款台灣自製的橫向捲軸射擊遊戲"
  txt "$A/c2_main.png" "$FONT_BLACK" 104 "$TEXT" "三十多年後，把它完整保存下來"
  txt "$A/c2_small.png" "$FONT_BOLD" 48 "$COOL" "逆向分析每張圖、每首曲子、每條規則"
  # 標題段的字幕（疊在遊戲畫面上）
  txt "$A/t_rs2.png" "$FONT_BLACK" 140 "$ACCENT_HI" "RS-2"
  txt "$A/t_prog.png" "$FONT_BOLD" 54 "$TEXT" "程式設計　施文馮"
  stack "$A/cap_title.png" 22 "$A/t_rs2.png" "$A/t_prog.png"
  # 遊玩、模式
  txt "$A/p_main.png" "$FONT_BOLD" 56 "$TEXT" "支援 VGA 顯示模式和音效卡"
  txt "$A/p_small.png" "$FONT_MED" 32 "$DIM" "《電腦玩家》1992 年 5 月"
  stack "$A/cap_play.png" 4 "$A/p_main.png" "$A/p_small.png"
  txt "$A/cap_modes.png" "$FONT_BOLD" 62 "$TEXT" "VGA、EGA、CGA、MGA 四種顯示模式"
  for m in VGA EGA CGA MGA; do label_plate "$A/mode_$m.png" "$m" "$ACCENT_HI" 250; done
  # 對照
  txt "$A/cap_sbs.png" "$FONT_BOLD" 54 "$TEXT" "1,337 個圖庫索引全部重繪成 HD，原版畫面一併保留"
  txt "$A/lab_orig.png" "$FONT_BLACK" 64 "$COOL" "原版"
  txt "$A/lab_hd.png" "$FONT_BLACK" 64 "$ACCENT_HI" "HD"
  # 快切（一行與直排兩種）
  txt "$A/cap_cut1.png" "$FONT_BOLD" 58 "$TEXT" "Rust 重寫，與原版實跑逐筆對拍，19,284 筆相同"
  txt "$A/cap_cut2.png" "$FONT_BOLD" 50 "$TEXT" "Rust 重寫，
與原版實跑逐筆對拍，
19,284 筆相同"
  # 頭目、四平台
  txt "$A/cap_boss.png" "$FONT_BOLD" 58 "$TEXT" "雙人同時、四段難度、開發者熱鍵，原樣保留"
  txt "$A/cap_plat.png" "$FONT_BOLD" 62 "$TEXT" "Linux、Windows、macOS、Android 四平台"
  txt "$A/lab_wide.png" "$FONT_BLACK" 52 "$ACCENT_HI" "戰場 150%"
  txt "$A/lab_android.png" "$FONT_BLACK" 52 "$COOL" "Android"
  # 片尾
  txt "$A/e_rs2.png" "$FONT_BLACK" 230 "$ACCENT_HI" "RS-2"
  txt "$A/e_orig.png" "$FONT_BOLD" 56 "$TEXT" "原作　大宇資訊 1990　程式設計　施文馮"
  txt "$A/e_remake.png" "$FONT_MED" 42 "$DIM" "保存與重製　RS-2 remake v.0.9.0"
  txt "$A/e_note.png" "$FONT_MED" 30 "$DIM" "本片含原版美術與音樂，只供本機保存"
  rm -f "$A"/_*.png
  ls "$A" | tr '\n' ' '; echo
}

# ---------------------------------------------------------------- 配樂
audio() {
  echo "[audio] 原版 AdLib 曲（Nuked OPL3 合成）volumedetect"
  for n in A B; do
    echo "-- $n.nuked.pcm"
    ffmpeg -nostdin -hide_banner -f s16le -ar 49716 -ac 1 -i "/rsaudio/$n.nuked.pcm" -af volumedetect -f null - 2>&1 \
      | grep -E "Duration|mean_volume|max_volume|n_samples" || true
  done
  # 開頭 A（標題曲），30 秒左右的「三十多年後」字卡處交叉淡變到 B（關卡曲），B 不夠長就循環。
  # A 取到 30.8 秒、淡變 1.6 秒，所以 B 在 29.2 秒進來、30.8 秒完全取代，淡變中心在 30.0 秒。
  # B 的平均音量比 A 低約 8 dB（volumedetect），只加 6.5 dB 的增益讓兩曲相近，加完最大音量 -4.6 dB，沒有削波。
  local total=72
  $FF -f s16le -ar 49716 -ac 1 -i /rsaudio/A.nuked.pcm -f s16le -ar 49716 -ac 1 -i /rsaudio/B.nuked.pcm -filter_complex \
    "[0:a]aresample=48000,aloop=loop=-1:size=2000000000,atrim=0:30.8,asetpts=N/SR/TB[a];
     [1:a]aresample=48000,volume=6.5dB,aloop=loop=-1:size=2000000000,atrim=0:50,asetpts=N/SR/TB[b];
     [a][b]acrossfade=d=1.6:c1=tri:c2=tri[m];
     [m]atrim=0:$total,afade=t=in:st=0:d=0.8,afade=t=out:st=$((total - 3)):d=3,pan=stereo|c0=c0|c1=c0,aformat=sample_fmts=s16:sample_rates=48000[o]" \
    -map "[o]" "$W/music.wav"
  echo "-- music.wav（混好的配樂）"
  ffmpeg -nostdin -hide_banner -i "$W/music.wav" -af volumedetect -f null - 2>&1 | grep -E "Duration|mean_volume|max_volume" || true
}

# ---------------------------------------------------------------- 各段
# 字卡：card 輸出 秒 底圖 大字 小字
seg_card() {
  local out="$1" dur="$2" bg="$3" main="$4" small="$5"
  local mh sh block top
  mh=$(png_h "$main"); sh=$(png_h "$small")
  block=$((mh + 34 + sh)); top=$(((VH - block) / 2))
  $FF -loop 1 -t "$dur" -i "$bg" -loop 1 -t "$dur" -i "$main" -loop 1 -t "$dur" -i "$small" -filter_complex \
    "[1:v]format=rgba,fade=t=in:st=0.6:d=0.9:alpha=1[m];
     [2:v]format=rgba,fade=t=in:st=2.0:d=0.9:alpha=1[s];
     [0:v][m]overlay=x=(W-w)/2:y=$top[a];
     [a][s]overlay=x=(W-w)/2:y=$((top + mh + 34))[b];
     [b]drawbox=x=(iw-460)/2:y=$((top - 40)):w=460:h=5:color=$ACC@0.95:t=fill,drawbox=x=(iw-460)/2:y=$((top + block + 36)):w=460:h=5:color=$ACC@0.95:t=fill,$(fadestr 0.35 0.35 "$dur")[v]" \
    -map "[v]" $ENC -t "$dur" "$out"
}

# 版面 A：遊戲畫面 1456x910 靠上置中，下方 170 像素是字幕條。遊戲畫面（含 HUD 與右下角的署名）完全不被蓋住。
# 輸入 0 是錄影，1 是底圖，2 是字幕 PNG；輸出標籤 [d]。
LAYOUT_A="[0:v]scale=1456:910:flags=lanczos,setsar=1[g];
     [1:v][g]overlay=232:0[a];
     [a]drawbox=x=226:y=0:w=6:h=910:color=$ACC:t=fill,drawbox=x=1688:y=0:w=6:h=910:color=$ACC:t=fill,drawbox=x=0:y=910:w=1920:h=170:color=$DEEP@0.92:t=fill,drawbox=x=0:y=906:w=1920:h=5:color=$ACC:t=fill[b];
     [2:v]format=rgba,fade=t=in:st=0.3:d=0.5:alpha=1[c];
     [b][c]overlay=x=(W-w)/2:y=910+(170-h)/2[d];"

# 遊玩加下方字幕條：seg_full 輸出 錄影 起點 秒 字幕PNG 淡入 淡出
seg_full() {
  local out="$1" clip="$2" ss="$3" dur="$4" cap="$5" fin="$6" fout="$7"
  $FF -ss "$ss" -t "$dur" -i "$clip" -loop 1 -t "$dur" -i "$A/bg_side.png" -loop 1 -t "$dur" -i "$cap" -filter_complex \
    "$LAYOUT_A
     [d]$(fadestr "$fin" "$fout" "$dur")[v]" \
    -map "[v]" $ENC -t "$dur" "$out"
}

# 顯示模式段：滿版加 VGA、EGA、CGA、MGA 標籤，各標籤依序出現
seg_modes() {
  local out="$1" clip="$2" ss="$3" dur="$4" cap="$5" fin="$6" fout="$7"
  local e="$MODE_EGA_AT" c="$MODE_CGA_AT" m="$MODE_MGA_AT"
  $FF -ss "$ss" -t "$dur" -i "$clip" -loop 1 -t "$dur" -i "$A/bg_side.png" -loop 1 -t "$dur" -i "$cap" \
    -loop 1 -t "$dur" -i "$A/mode_VGA.png" -loop 1 -t "$dur" -i "$A/mode_EGA.png" -loop 1 -t "$dur" -i "$A/mode_CGA.png" -loop 1 -t "$dur" -i "$A/mode_MGA.png" -filter_complex \
    "$LAYOUT_A
     [d][3:v]overlay=x=262:y=26:enable='lt(t,$e)'[l1];
     [l1][4:v]overlay=x=262:y=26:enable='between(t,$e,$c)'[l2];
     [l2][5:v]overlay=x=262:y=26:enable='between(t,$c,$m)'[l3];
     [l3][6:v]overlay=x=262:y=26:enable='gte(t,$m)'[l4];
     [l4]$(fadestr "$fin" "$fout" "$dur")[v]" \
    -map "[v]" $ENC -t "$dur" "$out"
}

# 框內遊玩加右側直排字：seg_framed 輸出 錄影 起點 秒 字幕PNG 淡入 淡出
seg_framed() {
  local out="$1" clip="$2" ss="$3" dur="$4" cap="$5" fin="$6" fout="$7"
  $FF -ss "$ss" -t "$dur" -i "$clip" -loop 1 -t "$dur" -i "$A/bg_side.png" -loop 1 -t "$dur" -i "$cap" -filter_complex \
    "[0:v]scale=1240:775:flags=lanczos,setsar=1[g];
     [1:v][g]overlay=70:152[a];
     [a]drawbox=x=64:y=146:w=1252:h=787:color=$ACC:t=5,drawbox=x=50:y=132:w=1280:h=815:color=$ACC@0.35:t=2[b];
     [2:v]format=rgba,fade=t=in:st=0.3:d=0.5:alpha=1[c];
     [b][c]overlay=x=1330+(560-w)/2:y=(1080-h)/2[d];
     [d]$(fadestr "$fin" "$fout" "$dur")[v]" \
    -map "[v]" $ENC -t "$dur" "$out"
}

# 左右對照：左原版（clipL）右 HD（clipR）
seg_sbs() {
  local out="$1" clipL="$2" ssL="$3" clipR="$4" ssR="$5" dur="$6" fin="$7" fout="$8"
  $FF -ss "$ssL" -t "$dur" -i "$clipL" -ss "$ssR" -t "$dur" -i "$clipR" -loop 1 -t "$dur" -i "$A/bg_side.png" \
    -loop 1 -t "$dur" -i "$A/cap_sbs.png" -loop 1 -t "$dur" -i "$A/lab_orig.png" -loop 1 -t "$dur" -i "$A/lab_hd.png" -filter_complex \
    "[0:v]scale=960:600:flags=lanczos,setsar=1[l];
     [1:v]scale=960:600:flags=lanczos,setsar=1[r];
     [2:v][l]overlay=0:240[x];
     [x][r]overlay=960:240[y];
     [y]drawbox=x=0:y=235:w=1920:h=5:color=$ACC:t=fill,drawbox=x=0:y=840:w=1920:h=5:color=$ACC:t=fill,drawbox=x=957:y=240:w=6:h=600:color=$ACC:t=fill[z];
     [3:v]format=rgba,fade=t=in:st=0.3:d=0.5:alpha=1[c];
     [4:v]format=rgba,fade=t=in:st=0.6:d=0.5:alpha=1[lo];
     [5:v]format=rgba,fade=t=in:st=0.6:d=0.5:alpha=1[lh];
     [z][c]overlay=x=(W-w)/2:y=(235-h)/2[d1];
     [d1][lo]overlay=x=480-w/2:y=845+(235-h)/2[d2];
     [d2][lh]overlay=x=1440-w/2:y=845+(235-h)/2[d3];
     [d3]$(fadestr "$fin" "$fout" "$dur")[v]" \
    -map "[v]" $ENC -t "$dur" "$out"
}

# 戰場 150% 與 Android：左遊玩（裁切內容區），右 Android 靜態圖加安全 zoompan（單幀輸入），下方字幕條
seg_wide() {
  local out="$1" clip="$2" ss="$3" dur="$4" fin="$5" fout="$6"
  local fr=$((dur * FPS)) cw ch cx cy lh ly ry
  IFS=: read -r cw ch cx cy <<< "$WIDE_CROP"
  lh=$((1100 * ch / cw)); lh=$((lh / 2 * 2))      # 左側遊玩畫面縮成寬 1100 的高
  ly=$((500 - lh / 2)); ry=$((500 - 157))          # 兩塊的垂直中心都在 y=500，字幕條在 y=910 以下
  $FF -ss "$ss" -t "$dur" -i "$clip" -loop 1 -i "$ANDROID_IMG" -loop 1 -t "$dur" -i "$A/bg_side.png" \
    -loop 1 -t "$dur" -i "$A/cap_plat.png" -loop 1 -t "$dur" -i "$A/lab_wide.png" -loop 1 -t "$dur" -i "$A/lab_android.png" -filter_complex \
    "[0:v]crop=$cw:$ch:$cx:$cy,scale=1100:$lh:flags=lanczos,setsar=1[g];
     [1:v]format=rgb24,zoompan=z='min(1+0.0011*on,1.22)':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=$fr:s=700x315:fps=$FPS,setsar=1[ad];
     [2:v][g]overlay=44:$ly[a];
     [a][ad]overlay=1176:$ry[b];
     [b]drawbox=x=38:y=$((ly - 6)):w=1112:h=$((lh + 12)):color=$ACC:t=4,drawbox=x=1170:y=$((ry - 6)):w=712:h=327:color=$ACC:t=4,drawbox=x=0:y=910:w=1920:h=170:color=$DEEP@0.92:t=fill,drawbox=x=0:y=906:w=1920:h=5:color=$ACC:t=fill[c];
     [3:v]format=rgba,fade=t=in:st=0.3:d=0.5:alpha=1[cap];
     [4:v]format=rgba,fade=t=in:st=0.5:d=0.5:alpha=1[l1];
     [5:v]format=rgba,fade=t=in:st=0.5:d=0.5:alpha=1[l2];
     [c][cap]overlay=x=(W-w)/2:y=910+(170-h)/2[d];
     [d][l1]overlay=x=44+(1100-w)/2:y=$((ly - 8 - 109)):format=auto[e];
     [e][l2]overlay=x=1176+(700-w)/2:y=$((ry - 8 - 109)):format=auto[f];
     [f]$(fadestr "$fin" "$fout" "$dur")[v]" \
    -map "[v]" $ENC -frames:v "$fr" "$out"
}

# 片尾：遊戲片尾畫面調暗當底，疊字依序淡入
seg_end() {
  local out="$1" clip="$2" ss="$3" dur="$4" fin="$5" fout="$6"
  local h1 h2 h3 h4 block top y2 y3 y4
  h1=$(png_h "$A/e_rs2.png"); h2=$(png_h "$A/e_orig.png"); h3=$(png_h "$A/e_remake.png"); h4=$(png_h "$A/e_note.png")
  block=$((h1 + 20 + h2 + 24 + h3 + 40 + h4)); top=$(((VH - block) / 2))
  y2=$((top + h1 + 20)); y3=$((y2 + h2 + 24)); y4=$((y3 + h3 + 40))
  $FF -ss "$ss" -t "$dur" -i "$clip" -loop 1 -t "$dur" -i "$A/e_rs2.png" -loop 1 -t "$dur" -i "$A/e_orig.png" \
    -loop 1 -t "$dur" -i "$A/e_remake.png" -loop 1 -t "$dur" -i "$A/e_note.png" -filter_complex \
    "[0:v]scale=1920:-2:flags=lanczos,crop=1920:1080,eq=brightness=-0.42:saturation=0.8,gblur=sigma=2.5,setsar=1[bg];
     [bg]drawbox=x=0:y=$((top - 50)):w=1920:h=$((block + 100)):color=$DEEP@0.55:t=fill,drawbox=x=0:y=$((top - 50)):w=1920:h=4:color=$ACC@0.9:t=fill,drawbox=x=0:y=$((top + block + 46)):w=1920:h=4:color=$ACC@0.9:t=fill[a];
     [1:v]format=rgba,fade=t=in:st=0.6:d=1.0:alpha=1[t1];
     [2:v]format=rgba,fade=t=in:st=1.6:d=0.9:alpha=1[t2];
     [3:v]format=rgba,fade=t=in:st=2.4:d=0.9:alpha=1[t3];
     [4:v]format=rgba,fade=t=in:st=3.2:d=0.9:alpha=1[t4];
     [a][t1]overlay=x=(W-w)/2:y=$top[b1];
     [b1][t2]overlay=x=(W-w)/2:y=$y2[b2];
     [b2][t3]overlay=x=(W-w)/2:y=$y3[b3];
     [b3][t4]overlay=x=(W-w)/2:y=$y4[b4];
     [b4]$(fadestr "$fin" "$fout" "$dur")[v]" \
    -map "[v]" $ENC -t "$dur" "$out"
}

segs() {
  ANDROID_IMG=/android/06-moved.png
  [ -f "$ANDROID_IMG" ] || { echo "缺 $ANDROID_IMG"; exit 1; }
  echo "[segs] 各段影片${SEGS:+（只重做 $SEGS）}"
  # 環境變數 SEGS（例如 "02 07a"）只重做指定的段，空的就全做。
  w() { [ -z "${SEGS:-}" ] && return 0; case " $SEGS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }
  w 01  && seg_card   "$S/01-card1.mp4" "$PACE_CARD1" "$A/bg_card1.png" "$A/c1_main.png" "$A/c1_small.png"
  w 02  && seg_framed "$S/02-title.mp4" "$C/$TITLE_CLIP.mp4" "$TITLE_SS" "$PACE_TITLE" "$A/cap_title.png" 0.3 0
  w 03  && seg_full   "$S/03-play.mp4"  "$C/$PLAY_CLIP.mp4"  "$PLAY_SS"  "$PACE_PLAY"  "$A/cap_play.png" 0.3 0
  w 04  && seg_modes  "$S/04-modes.mp4" "$C/$PLAY_CLIP.mp4"  "$MODES_SS" "$PACE_MODES" "$A/cap_modes.png" 0 0.3
  w 05  && seg_card   "$S/05-card2.mp4" "$PACE_CARD2" "$A/bg_card2.png" "$A/c2_main.png" "$A/c2_small.png"
  w 06  && seg_sbs    "$S/06-sbs.mp4"   "$C/sbsorig-1.mp4" "$(add "$T0_sbsorig" "$SBS_OFF")" "$C/sbshd-1.mp4" "$(add "$T0_sbshd" "$SBS_OFF")" "$PACE_SBS" 0.3 0
  w 07a && seg_full   "$S/07a-cut.mp4"  "$C/flow-1.mp4" "$CUT1_SS" 2 "$A/cap_cut1.png" 0 0
  w 07b && seg_framed "$S/07b-cut.mp4"  "$C/flow-1.mp4" "$CUT2_SS" 2 "$A/cap_cut2.png" 0 0
  w 07c && seg_full   "$S/07c-cut.mp4"  "$C/flow-1.mp4" "$CUT3_SS" 2 "$A/cap_cut1.png" 0 0
  w 07d && seg_framed "$S/07d-cut.mp4"  "$C/flow-1.mp4" "$CUT4_SS" 2 "$A/cap_cut2.png" 0 0
  w 08  && seg_full   "$S/08-boss.mp4"  "$C/boss-1.mp4" "$BOSS_SS" "$PACE_BOSS" "$A/cap_boss.png" 0 0
  w 09  && seg_wide   "$S/09-wide.mp4"  "$C/wide150-1.mp4" "$WIDE_SS" "$PACE_WIDE" 0 0.3
  w 10  && seg_end    "$S/10-end.mp4"   "$C/flow-1.mp4" "$END_SS" "$PACE_END" 0.4 1.2
  ls -la "$S"
}

final() {
  echo "[final] 串接與混音"
  : > "$S/list.txt"
  local f
  for f in 01-card1 02-title 03-play 04-modes 05-card2 06-sbs 07a-cut 07b-cut 07c-cut 07d-cut 08-boss 09-wide 10-end; do
    echo "file '$S/$f.mp4'" >> "$S/list.txt"
  done
  mkdir -p /outd
  $FF -f concat -safe 0 -i "$S/list.txt" -i "$W/music.wav" -map 0:v -map 1:a -c:v copy -c:a aac -b:a 192k -ar 48000 \
    -movflags +faststart "$W/final.tmp.mp4"
  mv "$W/final.tmp.mp4" "/outd/RS-2-remake-promo-$VERSION.mp4"
  ls -la /outd
}

review() {
  echo "[review] 驗收抽幀"
  local f="/outd/RS-2-remake-promo-$VERSION.mp4"
  echo "-- ffprobe"
  ffprobe -v error -show_entries format=duration,size -of default=nw=1 "$f"
  ffprobe -v error -select_streams v -show_entries stream=codec_name,width,height,r_frame_rate,duration -of default=nw=1 "$f"
  ffprobe -v error -select_streams a -show_entries stream=codec_name,sample_rate,channels,duration -of default=nw=1 "$f"
  rm -f "$W/review"/*.png
  local i=0 name t
  # 名稱:秒數。前 12 張進 4x3 拼圖（分鏡每個區間一張，另加 CGA 與第二個快切），x 開頭的補看用
  for spec in "01-card1:4.5" "02-title:12" "03-play:18" "04-modes-vga:22.8" "04-modes-cga:26.5" "05-card2:34" "06-sbs:40" \
              "07-cut-a:45" "07-cut-b:47" "08-boss:55" "09-wide:61" "10-end:69"; do
    name="${spec%%:*}"; t="${spec##*:}"; i=$((i + 1))
    $FF -ss "$t" -i "$f" -frames:v 1 "$W/review/$(printf %02d $i)-$name.png"
  done
  for spec in "x1-07-cut-c:49" "x2-07-cut-d:51" "x3-04-modes-ega:24.8" "x4-04-modes-mga:28.8" "x5-02-title-early:7" "x6-02-title-late:13.5" "x7-02-title-logo:11"; do
    name="${spec%%:*}"; t="${spec##*:}"
    $FF -ss "$t" -i "$f" -frames:v 1 "$W/review/$name.png"
  done
  cd "$W/review"
  montage $(ls [0-9]*.png) -font /usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc -pointsize 20 -label '%f' \
    -tile 4x3 -geometry 480x270+4+4 -background '#101018' -fill white montage.png
  ls -la "$W/review"
}

# 成片檢查：靜止畫面與全黑畫面的時段。字卡（0 至 6、30 至 36、64 至 72 秒）有靜止段是預期的，遊玩段不該有。
verify() {
  local f="/outd/RS-2-remake-promo-$VERSION.mp4"
  echo "[verify] freezedetect（n=0.003、至少 0.6 秒）"
  ffmpeg -nostdin -hide_banner -i "$f" -an -vf "freezedetect=n=0.003:d=0.6" -f null - 2>&1 | grep -E "freeze_start|freeze_end" | sed -E 's/.*(freeze_[a-z]+): ([0-9.]+).*/\1 \2/' | paste - - 2>/dev/null || true
  echo "[verify] blackdetect（至少 0.2 秒）"
  ffmpeg -nostdin -hide_banner -i "$f" -an -vf "blackdetect=d=0.2:pix_th=0.04" -f null - 2>&1 | grep black_start || echo "（沒有全黑段）"
  echo "[verify] ffprobe"
  ffprobe -v error -show_entries format=duration,size,bit_rate -of default=nw=1 "$f"
  ffprobe -v error -select_streams v -show_entries stream=codec_name,width,height,r_frame_rate,nb_frames,duration -of default=nw=1 "$f"
  ffprobe -v error -select_streams a -show_entries stream=codec_name,sample_rate,channels,duration -of default=nw=1 "$f"
  echo "[verify] 成片音訊 volumedetect"
  ffmpeg -nostdin -hide_banner -i "$f" -vn -af volumedetect -f null - 2>&1 | grep -E "mean_volume|max_volume"
}

for st in "$@"; do
  case "$st" in
    assets) assets ;;
    audio) audio ;;
    segs) segs ;;
    final) final ;;
    review) review ;;
    verify) verify ;;
    *) echo "不認得的階段 $st"; exit 1 ;;
  esac
done
