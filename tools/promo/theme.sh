# theme.sh：RS-2 推廣片的設計 token。色票取自第一關實機畫面的直方圖：
#   convert shot.png -crop 960x540+0+68 +repage -resize 160x90 -colors 10 -depth 8 -format %c histogram:info:-
#   最暗 #130404、深紅 #640D06、紅 #A01503、橙紅 #CF2B00、橙 #E35A0E、琥珀 #F09923、黃 #E9C43A、
#   灰藍 #6C7FB0（第一關畫面裡最冷的一色，標題畫面的夜空與 ACT 字樣同一色系）。
# 主題「熔岩赤焰」：深紅黑底、琥珀與黃的 HUD 風格細線、鋼藍紫只用在「原版／HD」這類對照標籤與第二張字卡。
THEME_NAME="熔岩赤焰"
BG_DEEP='#130404'      # 遊戲畫面最暗色
BG_LITE='#5a120a'      # 深紅（#640D06 稍暗）
ACCENT='#F09923'       # 琥珀：框線、標題
ACCENT_HI='#E9C43A'    # 黃：大標高光
ACCENT_RED='#CF2B00'   # 橙紅：小裝飾
TEXT='#FFF0D2'         # 字幕主色（暖白）
DIM='#CDB08A'          # 次要小字
COOL='#8FA2D8'         # 鋼藍紫（提亮）：對照標籤、第二張字卡
COOL_DEEP='#0b1030'    # 第二張字卡底色，取標題畫面的藍紫夜空

FONT_BLACK="Noto-Sans-CJK-TC-Black"    # 大標，硬派、近 HUD 的粗黑
FONT_BOLD="Noto-Sans-CJK-TC-Bold"
FONT_MED="Noto-Sans-CJK-TC-Medium"

VW=1920; VH=1080; FPS=60
MOTIF="scanline"       # 字卡與側邊底圖疊 CRT 掃描線；框線用 HUD 風格的琥珀細線

# 節奏（秒），總長 80：字卡 6、標題 8、遊玩 8、模式 8、字卡 6、對照 8、快切 8、海報 8、頭目 6、四平台 6、片尾 8。
PACE_CARD1=6; PACE_TITLE=8; PACE_PLAY=8; PACE_MODES=8; PACE_CARD2=6
PACE_SBS=8; PACE_CUTS=8; PACE_BOSS=6; PACE_WIDE=6; PACE_END=8
PACE_POSTERS=8
