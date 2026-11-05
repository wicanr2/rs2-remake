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

echo "按確定開始遊戲"
adb shell input tap $OK_XY
sleep 25
alive "開場"
shot 03-after-start
adb shell input tap $OK_XY
sleep 25
alive "進第一關"
shot 04-stage1
echo "方向盤往右滑、按住，開火"
adb shell input swipe $DPAD_X $DPAD_Y $((DPAD_X+150)) $DPAD_Y 1500 &
swipe_pid=$!
adb shell input tap $OK_XY
sleep 0.7
shot 05-move
# 只等這一個。模擬器也是這個 shell 的背景工作，不帶參數的 wait 會一直等下去。
wait "$swipe_pid"
sleep 1
alive "移動"
shot 06-moved
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

echo "關於頁（工具列第五顆）"
# 工具列的版面算法同 toolbar.rs：字型倍率 6，字寬 6×6，按鈕邊距 18。
ABOUT_X=$(python3 -c "
fs=6;pad=3*fs;x=pad
for c in (9,9,5,9):
    x+=c*6*fs-fs+2*pad+pad
print(x+40)")
adb shell input tap "$ABOUT_X" 70
sleep 3
adb shell dumpsys window | grep -E 'mCurrentFocus' | head -1 | tee /out/about-focus.txt
grep -q AboutActivity /out/about-focus.txt || echo "警告：關於頁沒有在前景"
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
