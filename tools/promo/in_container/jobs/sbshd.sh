# 左右對照的 HD 半邊：單一顯示器、倍率 3（視窗 960x600），與 sbsorig 同樣的輸入。
# 兩個顯示器同時錄在 4 核上會丟幀（ffmpeg 回報 More than 1000 frames duplicated），所以分兩次錄，合成時用 .t0 對齊。
NAME=sbshd
SECS=44
NDISP=1
W=960; H=600
CFG1='toolbar=off;scale=3;tween=on;hd=on;opt.invincible=on'
ARGS1='--start newgame --replay /rpl/stage1-move.jsonl'
