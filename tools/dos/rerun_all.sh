#!/usr/bin/env bash
# 重跑 docs/re/10-dynamic-receipts.md 引用的全部收據，輸出到 workspace/dosrun/<輸出名>/。
#   tools/dos/rerun_all.sh build          建置 workspace/dosgolem 的 rs2dyn（要先套 tools/dosgolem-patches）
#   tools/dos/rerun_all.sh g1|g2|g3|g4|g5  每組最多 4 個容器並行（每個 1 核、2 GB），組內前景等待
# 腳本、配置（A/B/C，見 run_scenario.sh）與逾時依下表。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
R="$ROOT/tools/dos/run_scenario.sh"
export RS2_CPUS=1 RS2_MEM=2g
case "${1:-}" in
  build) "$ROOT/tools/dos/build.sh" rs2dyn ;;
  g1) # 冷啟動、第一關、第二關、第三關
    "$R" cold cold A 560 & "$R" stage1 stage1 A 570 & "$R" stage2 stage2 A 570 & "$R" stage3 stage3 A 570 & wait ;;
  g2) # 第四關與片尾、標題調色盤（V、E）
    "$R" stage4 stage4 A 590 & "$R" title-shots-v title_v A 300 & "$R" title-shots-e title_e A 300 & wait ;;
  g3) # 幀速率三種配置、片尾、死亡
    "$R" fps fpsA A 280 & "$R" fps fpsB B 280 & "$R" fps fpsC C 280 & "$R" ending ending A 400 & wait
    "$R" death death A 400 & "$R" menu menu A 300 & "$R" facing facing A 300 & "$R" facing-pods facing_pods A 300 & wait ;;
  g4) # 熱鍵、密碼、顯示模式、文字選單
    "$R" hot-cheat hot_cheat A 200 & "$R" hot-keys hot_keys A 200 & "$R" hot-ctrld hot_ctrld A 200 & "$R" hot-ctrld-noasm hot_ctrld_noasm B 200 & wait
    "$R" hot-quit hot_quit A 200 & "$R" hot-misc hot_misc A 200 & "$R" hot-capture hot_capture A 200 & "$R" pw-variants pw_variants A 200 & wait
    "$R" mode-v mode_v A 200 & "$R" mode-e mode_e A 300 & "$R" mode-c mode_c A 300 & "$R" mode-h mode_h A 300 & wait
    "$R" mode-c-full mode_c_full A 400 & "$R" menu-text menutext_A A 100 & "$R" menu-text menutext_B B 100 & "$R" menu-text menutext_C C 100 & wait
    ;;
  g5) # 展示模式、S 鍵與喇叭
    "$R" demo demo A 590 & "$R" sound-toggle sound_toggle A 200 & wait ;;
  *) echo "用法：rerun_all.sh build|g1|g2|g3|g4|g5" >&2; exit 1 ;;
esac
