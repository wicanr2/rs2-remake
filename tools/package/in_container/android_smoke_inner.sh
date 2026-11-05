#!/usr/bin/env bash
# 在模擬器映像內執行，見 tools/package/android_smoke.sh。
set -euo pipefail
export PATH="$ANDROID_HOME/emulator:$ANDROID_HOME/platform-tools:$ANDROID_HOME/cmdline-tools/latest/bin:$PATH"
PKG=com.wicanr2.rs2remake
export ANDROID_AVD_HOME=/tmp/avd
mkdir -p "$ANDROID_AVD_HOME"
echo no | avdmanager create avd -n rs2 -k 'system-images;android-35;default;x86_64' -f >/dev/null
cfg="$ANDROID_AVD_HOME/rs2.avd/config.ini"
{
  echo hw.lcd.width=1080; echo hw.lcd.height=2400; echo hw.lcd.density=400
  echo hw.ramSize=3072; echo vm.heapSize=256; echo disk.dataPartition.size=2G; echo hw.keyboard=yes
} >> "$cfg"

emulator -avd rs2 -no-window -no-audio -no-boot-anim -no-snapshot -gpu swiftshader_indirect \
  -camera-back none -camera-front none >/out/emulator.log 2>&1 &
adb wait-for-device
for _ in $(seq 1 240); do
  [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ] && break
  sleep 2
done
[ "$(adb shell getprop sys.boot_completed | tr -d '\r')" = 1 ] || { echo "模擬器沒開起來"; tail -20 /out/emulator.log; exit 1; }
adb shell settings put global window_animation_scale 0 || true
adb shell settings put global transition_animation_scale 0 || true
adb shell settings put global hide_error_dialogs 1 || true
adb shell svc power stayon true || true
adb shell settings put system accelerometer_rotation 0 || true
# 第一次全螢幕的系統提示（ImmersiveModeConfirmation）會吃掉觸控。真機上玩家按一次就沒了，測試先略過。
adb shell settings put secure immersive_mode_confirmations confirmed || true
# 開機完成後系統還在套用資源覆蓋，這時啟動的 Activity 會被重建（Config changes=80000000）。等它穩定。
sleep 25
adb logcat -c

fail() { echo "失敗：$*"; adb logcat -d > /out/logcat-full.txt || true; adb logcat -d -s rs2:* SDL:* AndroidRuntime:* > /out/logcat.txt || true; exit 1; }
shot() { adb exec-out screencap -p > "/out/$1.png"; ls -l "/out/$1.png" | awk '{print "截圖 " $NF " " $5 " 位元組"}'; }
alive() { adb shell pidof "$PKG" >/dev/null 2>&1 || fail "$1：程序不在了"; }
wait_log() {
  # wait_log 字串 秒數
  for _ in $(seq 1 "$2"); do
    adb logcat -d -s rs2:* 2>/dev/null | grep -q "$1" && return 0
    sleep 1
  done
  fail "等不到 logcat：$1"
}

echo "安裝"
adb install -r "$APK" >/dev/null
adb shell dumpsys package "$PKG" | grep -E 'versionName|versionCode' | head -2
echo "第一次啟動"
adb shell am start -W -n "$PKG/.MainActivity" | grep -E 'Status|TotalTime'
sleep 3
shot 01-install
wait_log '\[install\]' 240
adb logcat -d -s rs2:* | grep '\[install\]'
wait_log '\[window\]' 240
sleep 8
alive "進遊戲"
for _ in $(seq 1 30); do
  adb shell dumpsys window | grep -E 'mCurrentFocus' | head -1 | grep -q MainActivity && break
  sleep 1
done
adb shell dumpsys window | grep -E 'mCurrentFocus' | head -1 | grep -q MainActivity || fail "遊戲不在前景，觸控會被別的視窗吃掉"
adb logcat -d -s rs2:* | grep -E '\[資源\]|\[init\]|\[hd\]|\[window\]|\[audio\]' | cut -d: -f4- | cut -c1-160 | head -14
adb shell wm size
adb shell dumpsys window | grep -E 'mCurrentFocus|mCurrentRotation' | head -3
shot 02-title

# 座標是橫向 2400×1080。尺寸見 crates/rs2-play/src/touch.rs，dp = 2.5。
D=2.5
xy() { python3 -c "print(int($1), int($2))"; }
OK_XY=$(xy "2400-62*$D" "1080-62*$D")
TURN_XY=$(xy "2400-62*$D" "1080-146*$D")
DPAD_X=$(python3 -c "print(int(68*$D))"); DPAD_Y=$(python3 -c "print(int(1080-78*$D))")
MENU_XY=$(xy "38*$D" "38*$D")
PAUSE_XY=$(xy "2400-38*$D" "38*$D")

# 最新的玩家機位置，不能控制（標題、過場、死亡動畫）時是空的。
pos() { adb logcat -d -s rs2:* 2>/dev/null | grep '\[play\]' | tail -1 | grep -oE '\([0-9-]+,[0-9-]+\)' | tr -d '()' | tr ',' ' ' || true; }
echo "按確定，直到玩家機出現"
reached=0
for i in $(seq 1 40); do
  adb shell input tap $OK_XY
  sleep 5
  alive "開場"
  [ -n "$(pos)" ] && { reached=1; echo "第 $i 次按確定後進關"; break; }
done
[ "$reached" = 1 ] || fail "按了 40 次確定都沒有進關"
echo "進關後不碰觸控 25 秒：不該有任何按鍵事件"
n0=$(adb logcat -d -s rs2:* | grep -cE '\[輸入\]|\[手把事件\]' || true)
sleep 25
alive "閒置"
adb logcat -d -s rs2:* | grep -E '\[輸入\]|\[手把事件\]|\[play\]' | tail -24 | cut -d: -f3- | cut -c1-70
n1=$(adb logcat -d -s rs2:* | grep -cE '\[輸入\]|\[手把事件\]' || true)
echo "閒置前後事件數：$n0 -> $n1"
[ "$n0" = "$n1" ] || fail "沒有碰觸控卻有輸入事件（$n0 -> $n1）"
# 原版放著不動約在幀 129 被打死，走完死亡動畫才回標題。再按確定開新的一局。
for i in $(seq 1 40); do
  [ -n "$(pos)" ] && break
  adb shell input tap $OK_XY
  sleep 5
done
[ -n "$(pos)" ] || fail "閒置之後回不到可以操作"
echo "方向盤往右按住，然後開火"
read -r x0 y0 <<<"$(pos)"
# 原版放著不動約 14 秒會被打死，測試跑慢一點讀到的就是「不能控制」，所以取最後一個有效位置。
last_pos() { adb logcat -d -s rs2:* 2>/dev/null | grep '\[play\] 玩家 1 在' | tail -1 | grep -oE '\([0-9-]+,[0-9-]+\)' | tr -d '()' | tr ',' ' ' || true; }
# adb input 模擬不了多指：同時送第二個點擊會取消還在進行的滑動。多指同時操作只有 touch.rs 的單元測試涵蓋，要真機才驗得到。
adb shell input swipe $DPAD_X $DPAD_Y $((DPAD_X+120)) $DPAD_Y 2500
sleep 1
adb shell input tap $OK_XY
shot 05-move
alive "移動"
read -r x1 y1 <<<"$(last_pos)"
echo "玩家機：($x0,$y0) -> ($x1,$y1)"
# 死亡動畫也會往右走，但同時往下。只往右才算方向盤造成的。
[ "$x1" -gt "$x0" ] && [ "$y1" = "$y0" ] || fail "方向盤往右，玩家機沒有只往右（($x0,$y0) -> ($x1,$y1)）"
# 按住約 1.7 秒，一秒 9.1 步，每步 16 像素。放寬到 5 步，模擬器的觸控事件時間不準。
[ $((x1 - x0)) -ge 80 ] || fail "按住方向盤卻走不遠（($x0,$y0) -> ($x1,$y1)）"
shot 06-moved
adb logcat -d -s rs2:* | grep -E '\[輸入\]' | head -30 | cut -d: -f4- | cut -c1-80
adb logcat -d -s rs2:* | grep '\[perf\]' | tail -4 | cut -d: -f4- | cut -c1-90
echo "暫停與恢復"
adb shell input tap $PAUSE_XY
sleep 1
shot 07-paused
adb shell input tap $PAUSE_XY
sleep 1
echo "工具列"
adb shell input tap $MENU_XY
sleep 2
shot 08-toolbar
alive "工具列"

echo "關於頁"
# 按鈕的位置由 app.rs 寫進 logcat，不在測試裡重算版面。
about=$(adb logcat -d -s rs2:* | grep '\[工具列\] About ' | tail -1 | grep -oE '[0-9]+ [0-9]+$' || true)
[ -n "$about" ] || fail "logcat 沒有關於鈕的位置"
ABOUT_X=${about% *}; ABOUT_Y=${about#* }
adb shell input tap "$ABOUT_X" "$ABOUT_Y"
# 切換視窗的途中 mCurrentFocus 會是 null，等到穩定才判定。
for _ in $(seq 1 20); do
  adb shell dumpsys window | grep -E 'mCurrentFocus' | head -1 > /out/about-focus.txt
  grep -q AboutActivity /out/about-focus.txt && break
  sleep 1
done
cut -c1-110 /out/about-focus.txt
grep -q AboutActivity /out/about-focus.txt || fail "關於頁沒有在前景"
sleep 1
shot 09-about
adb shell input keyevent BACK
sleep 2
alive "從關於頁回來"

echo "切到背景再回來"
adb shell input keyevent HOME
sleep 3
adb shell am start -W -n "$PKG/.MainActivity" | grep -E 'Status'
sleep 5
alive "回前景"
shot 10-resumed

echo "第二次啟動不再解資料"
adb shell am force-stop "$PKG"
adb logcat -c
adb shell am start -W -n "$PKG/.MainActivity" | grep -E 'Status|TotalTime'
wait_log '\[install\]' 120
adb logcat -d -s rs2:* | grep '\[install\]' | cut -d: -f4-
wait_log '\[window\]' 240
sleep 5
alive "第二次啟動"
shot 11-second-launch

adb logcat -d > /out/logcat-full.txt
adb logcat -d -s rs2:* SDL:* AndroidRuntime:* > /out/logcat.txt
if grep -qE 'FATAL EXCEPTION|當掉|SIGSEGV|signal 11' /out/logcat-full.txt; then
  fail "logcat 有崩潰紀錄"
fi
echo "PSS（KB）："; adb shell dumpsys meminfo "$PKG" | grep -E 'TOTAL PSS|TOTAL:' | head -2
echo "完成"
