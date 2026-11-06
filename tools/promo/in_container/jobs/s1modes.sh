# 第一關從頭玩（移動加開火、無敵），HD、補間。視窗出現後 10.3、12.3、14.3 秒各按一次 F6，
# 依序切成 EGA、CGA、MGA（F6 是顯示模式切換，見 --help）。--toggle-at 只在 --headless-steps 生效，視窗模式不用它。
NAME=s1modes
SECS=26
NDISP=1
W=1280; H=800
CFG1='toolbar=off;scale=4;tween=on;hd=on;opt.invincible=on'
ARGS1='--start newgame --replay /rpl/stage1-move.jsonl'
post_launch() { keys 10.3 F6 12.3 F6 14.3 F6; }
