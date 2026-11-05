# dosgolem 修改

基底：`/home/anr2/cht/dosgolem` commit `f79eab6db8c64a524a06535e53e5d32b0bbcf7c8`，複製到 `workspace/dosgolem/`（`git clone --local`）。原 repo 未改動。

套用順序：

```sh
git clone --local /home/anr2/cht/dosgolem workspace/dosgolem
cd workspace/dosgolem
git apply ../../tools/dosgolem-patches/0001-dos-resolve-ignore-spaces.patch
git apply ../../tools/dosgolem-patches/0002-cmd-rs2dyn.patch
git apply ../../tools/dosgolem-patches/0003-rs2dyn-full-trace.patch
git apply ../../tools/dosgolem-patches/0004-rs2dyn-dump-state.patch
git apply ../../tools/dosgolem-patches/0005-rs2dyn-fuzz-support.patch
git apply ../../tools/dosgolem-patches/0006-rs2dyn-wait-trace.patch
git apply ../../tools/dosgolem-patches/0007-rs2dyn-audio-and-banks.patch
git apply ../../tools/dosgolem-patches/0008-rs2dyn-mono-vram.patch
```

| patch | 內容 | 原因 |
|---|---|---|
| 0001 | `internal/dos/files.go` 的 `resolve` 先略過檔名內的空白，附 `resolve_spaces_test.go` | DOS 的路徑正規化把空白當分隔略過（DOSBox-X `src/dos/dos_files.cpp` 的 `DOS_MakeName`，短檔名模式 `else if (c==' ') continue;`，結尾空白另有 `while (r>0 && name_int[r-1]==' ') r--;`）。RS-2 的字串表有 `MY.GO ` 這樣尾端帶空白的名字，第 3 個副檔名位置留給 `sub_1C738` 改寫成 `B`（單色）。彩色模式不改寫，原版 dosgolem 找不到 `MY.GO `，遊戲在 `sub_1C82C` 無限重試開檔，畫面停在密碼之後。 |
| 0002 | 新增 `cmd/rs2dyn`：腳本化驅動（送鍵、存畫面與調色盤比對、逐幀追蹤、時基量測、開檔與向量日誌） | 本作需要「依遊戲事件等待」「串接 ASMDRV TSR 再跑 S.EXE」「以 IDA 位址讀寫遊戲變數」「虛擬秒時基」，`cmd/probe` 與 `cmd/run` 以步數為主，這些要靠步數猜。通用化沒有做，位址與調色盤表寫在 `cmd/rs2dyn/rt.go`，屬本作專屬。 |
| 0003 | `cmd/rs2dyn` 新增：`trace NAME full`（完整逐幀狀態，`trace-NAME.jsonl`，新檔 `trace_full.go`）、`play`（依逐幀輸入位元組送鍵與存畫面，`play.go`）、`tracepc`、`watchw`（`watch.go`）、`untiltrace`、`tracerows`、`until8`、`savestate`／`loadstate`；`trapint` 的輸出格式不變 | 規格 001 的驗收 A2 要逐幀比對玩家欄位、15 個實體槽與三個池的原始字組，舊的 `trace`（TSV，幀計數一變就記）只有 20 個欄位。取樣點固定在每個邏輯幀的指令位址（第一關 `10F98h`、`10FE3h`；第二至四關在 `inc word_1CFE2` 之後），機器碼簽章不符就拒絕開始。`play` 讓同一組輸入位元組可以逐幀重播，`tracepc` 與 `watchw` 用來確認缺陷候選。舊的 `trace` 與既有收據的輸出不變 |
| 0004 | `cmd/rs2dyn` 新增 `untilpc`（跑到指定 IDA 位址）與 `dumpds`（存整個 64 KB 資料段，`NAME.ds`），新檔 `dump.go` | 規格 001 的 A0 要比對第一幀前的資料段，A5 要在固定幀傾印平面。第一幀的入口是 `sub_10CD0` 的主迴圈頭 `10D29h` |
| 0005 | `cmd/rs2dyn` 差分模糊測試支援。`play` 新增兩個輸入位元：`40` 右 Shift 持續按住（只在位元改變時送按下／放開），`80` 該幀多送一個右 Shift 按下碼（模擬鍵盤自動重複）。追蹤新增主迴圈幀尾觀測（`10D32h`、`10D29h`、`10D59h`，機器碼簽章不符就拒絕追蹤）：`play` 遇到主迴圈離開（`10D59h`，DX 是離開碼）就停，不再等不會來的筆數。新命令 `settle` 讓最後一筆所在的半步走完並寫 `trace-NAME.end`（`rows=N exit=none\|E\|Q\|C`）。新命令 `dsdump EVERY` 讓筆數是 EVERY 倍數的取樣點另存整個資料段 `ds-NAME-n.ds` | 差分模糊測試（`tools/dos/gen_fuzz.py`）要涵蓋充能與自動重複（`byte_273A9` 只在右 Shift 按住期間有 INT 9 事件時才設）、玩家死亡（離開碼 `E`）與頭目擊殺（`C`）。原本 `play` 在主迴圈離開後會空等到逾時，且收據沒有「之後原版是否離開」的結論，Rust 端無法判斷最後一筆之後該不該離開。JSONL 只涵蓋選定的欄位，`dsdump` 讓重製核心在同一個取樣點（`on_sample` 回呼）逐位元組核對整個資料段。低 6 位的行為與時序、`trace-NAME.jsonl` 的內容都不變；`.end` 與 `ds-*.ds` 只有腳本呼叫 `settle`、`dsdump` 才產生 |
| 0006 | `cmd/rs2dyn` 全流程對拍支援（規格 002）。新檔 `wait.go`：`waittrace NAME [STOPN]` 在 `sub_11097` 離開等待（`1109Dh`，`jb` 沒有跳回之後、歸零之前）每次記一筆，輸出 `wait-NAME.jsonl`（`n`、`ret`、`cx`、`frame`、`sc`、`sct`、`vram`、`dac`、`start`）、`wait-NAME.dsd`（資料段差異，第 0 筆完整、其後每筆只存變動的位元組）與 `wait-NAME.pokes`；`waitstop N`／旗標 `-waitstop`（第 N 筆寫完後傾印 `NAME-wN.ds`、`.p0～p3`、`.json`、`.png` 並正常結束）、`waitsync on\|off`、`untilwait N`、`fontdump`（`F000:FA6E` 起 2048 位元組存成 `biosfont.bin`）。`script.go` 新增 `skipif COND LABEL` 與 `label`，`play` 新增 `!NAME` 結束條件（`play.go`、`trace_full.go`），`rt.go` 的 `down`／`up` 與 `poke`、`pokew` 在 waittrace 開著時先等到等待迴圈裡才動手，`main.go` 處理 `errWaitStop` | 規格 002 要在每個等待點比對完整狀態（資料段、四個平面、DAC），不只是第一關幀尾的取樣。取樣點簽章（`sub_11097` 整個函式 13 位元組）不符就拒絕開始。`sc` 記的是 INT 9 實際送達的掃描碼（`m.KeyIRQs` 增加的那一步），不是排程的那一刻；`sct` 是每個鍵送達時距上一筆的 INT 8 數，輪詢迴圈（片尾、選單、過場等沒有 `sub_11097` 的地方）裡的鍵靠它排進對的 tick。`waitsync` 讓鍵與 poke 落在等待裡，對應重製「恢復時處理掃描碼」的模型；輪詢迴圈裡等不到等待就在 `IRQ0Every x 6` 步後放棄，記一次 `sync_timeouts`。第二至四關沒有 `10D59h` 可偵測離開，`play` 的 `!NAME`（例如 `!J3.GO`）以過場開檔當關卡結束。`waittrace` 只讀狀態，不改變執行；`waitsync` 只在 waittrace 開著時啟動 |
| 0007 | `cmd/rs2dyn`：`audiotrace`（OPL、喇叭閘門、PIT 通道 2）、`waitbanks` 與 `-waitstop` 時傾印圖庫記憶體，等待紀錄加 `step` | 規格 003 的音訊對拍，與越界圖號的證據 |
| 0008 | `cmd/rs2dyn` 單色顯示模式（規格 006）。新檔 `mono.go`：`waittrace` 在 `byte_1CF74`（`dseg:3B4h`）為 `C`（CGA 模式 6）或 `H`（Hercules）時，每筆多記 `mm`（模式字母）、`mono`（顯示記憶體整塊的 SHA-256，C 是 `B8000..BFFFF` 32 KB，H 是 `B0000..BFFFF` 64 KB）、`hpage`（只有 H，3B8h bit 7 選的顯示頁起點）、`msa`、`msb`（背景暫存區 A、B，段值在 `word_1CF70`、`word_1CF72`，各 `0D000h` 位元組的 SHA-256）；新命令 `dumpmono NAME`（`NAME.mono` 是 `B0000..BFFFF` 64 KB，`.msa`、`.msb`，`.mono.json` 是 CRTC 暫存器、3B8h、畫頁偏移）與 `shotmono NAME [page0\|page1]`（C 640×200；H 依 CRTC 的 R1、R6、R9 解碼，本作是 640×300）；`-waitstop` 在單色模式傾印 `.mono` 等而不是平面。`internal/machine/hercules.go` 加 `HerculesCRTC`、`HerculesModeReg` 兩個存取函式 | 規格 006 要對拍 CGA 與 Hercules 的顯示記憶體。dosgolem 的 `shot` 不支援 CGA 模式 6，`Hercules()` 的解碼假設每字元列 4 條掃描線（R9=3），本作寫的是 R9=2（3 條），所以另寫解碼。C、H 的畫面是平坦記憶體，不走 `A000h` 的平面路徑，`vram`、`dac`、`start` 欄位在單色收據裡沒有意義，照舊保留。E 的收據欄位與行為不變（`M` 不是 `C`、`H` 時不執行新程式碼） |

`cmd/rs2dyn` 不改動 dosgolem 既有套件；只用 `machine`、`dos`、`cpu` 的公開欄位與方法。

驗證：`tools/go.sh test ./internal/dos` 通過（含新測試）。0003 以乾淨的基底 commit 依序套用 0001、0002、0003，`cmd/rs2dyn` 與 `internal/dos/files.go` 逐檔與 `workspace/dosgolem/` 相同，`gofmt -l`、`go vet`、`go build` 通過。

0003 不含單元測試：`cmd/rs2dyn` 是驅動程式，驗證靠收據（`docs/re/10-dynamic-receipts.md` 第 8 至 10 節，同一組輸入在兩個獨立容器的追蹤雜湊相同）。

0005 的驗證：乾淨的基底 commit 依序套用 0001 到 0005，`cmd/rs2dyn` 與 `internal/dos/files.go` 逐檔與 `workspace/dosgolem/` 相同，`gofmt -l`、`go vet ./cmd/rs2dyn` 無輸出。

既有收據用新建的 `rs2dyn`（sha256 `67378d194da4894642bedfd02b3984b80770e257b768c82f45130ac48caef5ca`，0004 時是 `414a98ff…`）重跑，輸出不變：

| 重跑 | 指令 | 結果 |
|---|---|---|
| `chk_dm1` | `tools/dos/run_scenario.sh trace-dm1 chk_dm1 A 400` | `trace-dm1.jsonl` 與 `trace_dm1` 逐位元組相同（含 `emu` 欄位），sha256 `932e3caa165fd2684233a08baf3ef3af467b2f045c1ea93789404f3ae2c30ec0` |
| `chk_boss1` | `tools/dos/run_scenario.sh boss1-kill chk_boss1 A 590` | `trace-s1.jsonl`（1192 筆，sha256 `0542c70cd869ccca9f3e9490494123e1fd4c80435dee636afac7d89fd18adaa8`）、`events.tsv` 與 17 張畫面的 png、json 都與 `boss1_kill` 相同 |

`40`、`80` 位元不與 `10`（右 Shift 按放一次）併用，併用時 `10` 的放開會中斷按住。`play` 在取樣之後依序送鍵，每個事件隔 `KeyEvery + 1` 步，事件太多時最後幾個會晚於下一幀讀輸入。這一點 `play` 自己只擋得住「下一筆取樣已經寫出」的情況，所以產生器的收據要用 `tools/dos/fuzz/check.sh` 再驗一次：每一列 `in1` 的方向位元必須等於腳本在那一列送出的方向鍵狀態。

按鍵歸屬的限制：`sc` 記的是「上一筆取樣之後送出的鍵」，重製的 `step` 在該步開頭才收到。玩家輸入（`sub_14458`）在下一步開頭讀，兩邊一致。第二半步尾端的 Esc 暫停（`sub_10ECB`）與熱鍵（`sub_10D99`）在取樣之後才檢查，原版會看到取樣後送出的鍵，重製要到下一步才看到，所以這類鍵只能在第一半步取樣之後送（`gen_fuzz.py` 的 `hot` profile 只選奇數筆的邊界）。

0006 的驗證：乾淨的基底 commit 依序套用 0001 到 0006，`cmd/rs2dyn` 與 `internal/dos` 逐檔與 `workspace/dosgolem/` 相同，`gofmt -l`、`go vet ./cmd/rs2dyn` 無輸出。`~/cht/dosgolem` 沒有改動。

既有收據用新建的 `rs2dyn`（sha256 `448685208755fd9de3e850f174c7c0d43173b5eca17cd942f0051a6d6cf77ac3`）重跑，輸出不變；`waittrace` 沒開時不啟動任何新程式碼，開著時也只讀狀態：

| 重跑 | 指令 | 結果 |
|---|---|---|
| `chk8_dm1` | `tools/dos/run_scenario.sh trace-dm1 chk8_dm1 A 590` | `trace-dm1.jsonl`（含 `emu` 欄位）、`dm1_end.png`、`dm1_end.json`、`events.tsv` 與 `trace_dm1` 逐位元組相同，sha256 `932e3caa165fd2684233a08baf3ef3af467b2f045c1ea93789404f3ae2c30ec0` |
| `chk8_boss1` | `tools/dos/run_scenario.sh boss1-kill chk8_boss1 A 590` | `trace-s1.jsonl`（1192 筆，sha256 `0542c70cd869ccca9f3e9490494123e1fd4c80435dee636afac7d89fd18adaa8`）、`events.tsv` 與 17 張畫面的 png、json（34 檔）都與 `boss1_kill` 相同 |
| `chk8_dm1w` | `tools/dos/run_scenario.sh chk-dm1-wait chk8_dm1w A 590`（`trace-dm1` 加一行 `waittrace chk`） | `trace-dm1.jsonl` 與 `trace_dm1` 逐位元組相同，含 `emu` 的步數，所以 `waittrace` 沒有改變執行；`wait-chk.jsonl` 639 筆 |

負對照：`chk-wait-sig`（`sub_11097` 的 `jb`（`1109Bh`）改成 `nop`）讓 `waittrace` 拒絕開始，訊息是「sub_11097 的機器碼不符（第 4 位元組 90，預期 72）」。決定性：`wt_menu`、`wt_p2` 在兩個獨立容器各跑一次，`wait-*.jsonl`、`.dsd`、`.pokes`、`entry.ds`、`entry.p0`、`events.tsv` 逐位元組相同。

`waittrace` 開著時 `down`、`up`、`poke`、`pokew` 會先等到 `sub_11097` 的等待迴圈裡，所以同一份腳本在有無 `waittrace` 時的送鍵時刻不同。要重現不含 `waittrace` 的舊收據（`trace_dm1`、`boss1_kill`、`fuzz_*`）就不要加 `waittrace`，那些腳本都沒有。
