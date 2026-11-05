# 003 音訊

狀態：CONFORMED（2026-11-05）

證據：`ASMDRV` 的逐函式對照在 [docs/re/13-audio-and-misc.md](../re/13-audio-and-misc.md) 第 1、2 節，喇叭在第 4 節。`tools/mix_to_vgm.py` 的 Python 版已經和原版的實錄寫入比對過前段，差別只有跨曲目保留的聲部狀態，本規格的 Rust 版一併處理。

## 範圍

遊戲內的全部聲音：AdLib 音樂（`A.MIX`、`B.MIX`）與 PC 喇叭的繪圖附帶音。

不含：Sound Blaster（原版沒有）、沒有 AdLib 時的路徑（`sub_10A90` 把 `10AD4h`、`10B12h` 改成 `retn`）、永遠不會執行的參數式喇叭引擎（`sub_18736`，見 `docs/re/13` 4.2）。

## 輸入

`A.MIX`、`B.MIX`。雜湊在 `rs2-formats` 的 `EXPECTED`。喇叭事件來自 `MY.GO` 的圖庫資料。

## 架構

| 層 | 位置 | 內容 |
|---|---|---|
| 命令 | `rs2-game` 的 `Frame.audio` | `AudioCmd::Play(MixFile)`、`Stop`、`Speaker(bool)`，和繪製命令一樣在一步結束時交出 |
| 驅動 | `rs2-audio::driver` | `ASMDRV` 的合成層與事件層逐函式移植，輸出 OPL2 暫存器寫入 |
| 合成 | `rs2-audio::opl2` | YM3812，整數運算，49716 Hz |
| 喇叭事件 | `rs2-gfx::speaker` | `sub_192EA` 依圖庫資料產生的除數與閘門，事件間隔是機器碼的指令數 |
| 喇叭輸出 | `rs2-audio::speaker` | PIT 通道 2 模式 3 的方波，閘門控制 |
| 接合 | `rs2-audio::engine` | 一個取樣是 24 個 PIT 週期，驅動 tick 落點是整數運算，不累積誤差 |
| 裝置 | `rs2-play::audio` | SDL 音效裝置，或 `--wav-out` 依遊戲時間算出 WAV |

模擬不等音訊，音訊也不回頭影響模擬。音樂相關狀態（`Sound`）由 `rs2-game` 照原版維護，裝置開不開都一樣。

## 驅動的行為

`sub_10AD4` 是 `AX=0`、`AX=2`、`AX=4`，`sub_10B12` 是 `AX=5`、`AX=3`、`AX=0`，都只在 `byte_1CBCE` 是 `Y` 時動作。`sub_187C3` 依靜音開關呼叫它們，並把 `sub_187B3`、`sub_187BB` 改成 `retn`，所以靜音時喇叭閘門不動。

合成層的狀態跨曲目保留：新曲開頭的滑音事件會寫出上一首最後的音符，音量與細調在 `AX=0` 重設。

`80h` 事件的驅動缺陷（處理後不讀 delta）在兩首曲子都不會觸發，重製遇到就 `unreachable!`，規則寫在 `docs/re/13` 2.3。

## 全流程對拍

| 收據 | 內容 |
|---|---|
| `audio-NAME.tsv` | `rs2dyn` 的 `audiotrace`：每筆 OPL 寫入（步數、IRQ 0 累計、暫存器、值），每次喇叭閘門改變，每次 PIT 通道 2 重載 |
| `wait-NAME.jsonl` 的 `step` | 這筆等待的步數，用來把音訊事件對到第幾次等待 |

音樂的比對對象是暫存器序列。依 dosgolem 規格的原則，暫存器串是樂譜，波形是演奏，樂譜逐筆比，演奏只比品質。

## 驗收

| 編號 | 項目 | 通過條件 |
|---|---|---|
| C1 | 暫存器 | `au_title`、`au_flow` 全部 OPL 寫入的暫存器與值相同，每次播放內 tick 差固定（`rs2-audio/tests/opl_receipt.rs`） |
| C2 | 開關時機 | 全流程收據裡每次播放與停止落在同一次等待，曲目相同（`rs2-trace/tests/audio.rs`） |
| C3 | 喇叭 | 全流程收據裡每段繪圖附帶音的除數與閘門序列相同，事件之間的指令數相同（同上） |
| C4 | 合成品質 | 與 Nuked OPL3 黑箱比對同一份暫存器序列：遮掉小鼓與踩鈸時相關係數大於 0.999；整首每 100 毫秒能量差小於 3 dB（`rs2-audio/tests/synth_ref.rs`） |
| C5 | 前端 | 音效裝置不影響模擬：`--wav-out` 與否每步資料段雜湊相同。`SDL_AUDIODRIVER=disk` 能輸出非靜音資料 |

## 驗收結果

| 編號 | 結果 |
|---|---|
| C1 | `c1_opl_writes_match_the_original_driver`，29,425 筆相同，負對照會失敗 |
| C2、C3 | `c2_c3_music_and_speaker_match`，`wt_title`、`wt_menu`、`wt_p2`、`wt_flow`、`wt_mute`。音樂開關 32 次落在同一次等待，OPL 36,411 筆相同，喇叭事件 317,133 筆相同，其中 304,753 筆的指令數也相同（其餘夾了 IRQ 0 或是靜音、平面開頭，不比指令數） |
| C4 | `audio_compare_with_reference`。`A` 相關係數 0.9995，`Bn` 0.9996；整首每 100 毫秒能量差最大 1.53 dB |
| C5 | `c5_audio_does_not_touch_the_simulation`。`SDL_AUDIODRIVER=disk` 輸出非靜音資料。實際喇叭沒有驗過 |

## 已知差異

### 刻意不照做

| 原版 | 重製 | 為什麼 |
|---|---|---|
| OPL2 的 DAC 是 10 位元浮點 | 線性 16 位元 | 差異在量化雜訊，換來簡單與決定性 |
| 沒有 AdLib 時印 `NO ADLIB?` 並關音樂 | 永遠有 AdLib | 重製自帶合成器 |

### remake-owned

| 項目 | 現況 | 要改的條件 |
|---|---|---|
| 喇叭的時間軸 | 原版是 CPU 跑繪圖迴圈的速度，靜態無法決定。重製用 4 MIPS 換算指令數（`speaker.rs` 的 `STEP_NUM`、`STEP_DEN`） | 有實機錄音時對照 |
| 喇叭音量與高通 | 振幅 5000，時間常數約 20 毫秒，憑常識選的 | 有實機錄音時對照 |
| 雜訊暫存器的節奏 | 每個取樣走一步。Nuked OPL3 每個運算子走一步，兩者逐取樣不同，能量相近（強推論：OPL2 的實際節奏未知） | 有 OPL2 實機錄音時對照 |
| 取樣率轉換 | 交給 SDL | 音質有問題時換自己的重取樣 |

## 失敗模式

- `A.MIX`、`B.MIX` 雜湊不符：拒絕啟動。
- 音效裝置打不開：印原因，遊戲照跑，沒有聲音。

## 權利邊界

同規格 001。`rs2-audio` 的 `driver` 是 `ASMDRV` 的逐函式移植，repo 維持私人，對外公開前先問使用者。`opl2` 依公開的晶片分析撰寫，Nuked OPL3（LGPL）只編譯成黑箱參考，它的程式碼不進這個 repo，比對工具在 `tools/audio/`。
