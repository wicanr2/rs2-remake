# cuts.sh：每段用哪支錄影、從幾秒開始。時間是該錄影檔的秒數（x11grab 從遊戲啟動起錄）。
# 先看錄影抽幀（frames.sh、sheet.sh、scenes.sh）再填，改這裡不用動合成腳本。
# T0_* 是第一個遊戲畫面的時間（record_inner.sh 寫的 .t0，載入中畫面結束）。合成腳本在讀完 .t0 後才 source 本檔。

# 6 至 14 秒：標題入口。2.6 秒起：地球開場、握搖桿的手、RS-2 標誌（約 7 至 8 秒）、署名畫面（約 8 至 10 秒）、ACT 字樣
TITLE_CLIP=title-1;      TITLE_SS=2.6
# 14 至 22 秒：第一關遊玩。第一關頭兩個畫面的調色盤還沒套上（藍紫色），跳過 0.3 秒
PLAY_CLIP=s1modes-1;     PLAY_SS="$(awk -v t="$T0_s1modes" 'BEGIN{printf "%.3f", t + 0.3}')"
# 22 至 30 秒：同一支錄影接著播。F6 的實際切換時間（scenes.sh 量到 EGA 10.57、CGA 12.57，MGA 照 2 秒間隔 14.57）
MODES_SS="$(awk -v t="$PLAY_SS" 'BEGIN{printf "%.3f", t + 8}')"
MODE_EGA_AT="$(awk -v s="$MODES_SS" 'BEGIN{printf "%.2f", 10.57 - s}')"
MODE_CGA_AT="$(awk -v s="$MODES_SS" 'BEGIN{printf "%.2f", 12.57 - s}')"
MODE_MGA_AT="$(awk -v s="$MODES_SS" 'BEGIN{printf "%.2f", 14.57 - s}')"
# 36 至 44 秒：左右對照。原版圖在左（sbsorig），HD 在右（sbshd），各自從遊戲啟動後 SBS_OFF 秒起（兩邊同輸入，用 t0 對齊）
SBS_OFF=24
# 44 至 52 秒：第二、三關快切，每段 2 秒，錄影檔 flow-1 的絕對秒數（第二關 24 至 44 秒，第三關 51 至 71 秒）
CUT1_SS=37.5; CUT2_SS=58.0; CUT3_SS=41.0; CUT4_SS=66.5
# 52 至 58 秒：第一關頭目，錄影檔 boss-1 的絕對秒數（核心被打倒、連爆約在 126 至 130 秒）
BOSS_SS=124.0
# 58 至 64 秒：戰場 150%，錄影檔 wide150-1 的絕對秒數與裁切（寬:高:x:y，視窗 1280x800 內的內容區）
WIDE_SS=10.5
WIDE_CROP="960:400:160:200"
# 64 至 72 秒：片尾字卡的背景，錄影檔 flow-1 的絕對秒數（沙漠與天空 85 至 90 秒，之後是地球與人物）
END_SS=86.0
