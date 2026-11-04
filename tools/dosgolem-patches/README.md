# dosgolem 修改

基底：`/home/anr2/cht/dosgolem` commit `f79eab6db8c64a524a06535e53e5d32b0bbcf7c8`，複製到 `workspace/dosgolem/`（`git clone --local`）。原 repo 未改動。

套用順序：

```sh
git clone --local /home/anr2/cht/dosgolem workspace/dosgolem
cd workspace/dosgolem
git apply ../../tools/dosgolem-patches/0001-dos-resolve-ignore-spaces.patch
git apply ../../tools/dosgolem-patches/0002-cmd-rs2dyn.patch
```

| patch | 內容 | 原因 |
|---|---|---|
| 0001 | `internal/dos/files.go` 的 `resolve` 先略過檔名內的空白，附 `resolve_spaces_test.go` | DOS 的路徑正規化把空白當分隔略過（DOSBox-X `src/dos/dos_files.cpp` 的 `DOS_MakeName`，短檔名模式 `else if (c==' ') continue;`，結尾空白另有 `while (r>0 && name_int[r-1]==' ') r--;`）。RS-2 的字串表有 `MY.GO ` 這樣尾端帶空白的名字，第 3 個副檔名位置留給 `sub_1C738` 改寫成 `B`（單色）。彩色模式不改寫，原版 dosgolem 找不到 `MY.GO `，遊戲在 `sub_1C82C` 無限重試開檔，畫面停在密碼之後。 |
| 0002 | 新增 `cmd/rs2dyn`：腳本化驅動（送鍵、存畫面與調色盤比對、逐幀追蹤、時基量測、開檔與向量日誌） | 本作需要「依遊戲事件等待」「串接 ASMDRV TSR 再跑 S.EXE」「以 IDA 位址讀寫遊戲變數」「虛擬秒時基」，`cmd/probe` 與 `cmd/run` 以步數為主，這些要靠步數猜。通用化沒有做，位址與調色盤表寫在 `cmd/rs2dyn/rt.go`，屬本作專屬。 |

`cmd/rs2dyn` 不改動 dosgolem 既有套件；只用 `machine`、`dos`、`cpu` 的公開欄位與方法。

驗證：`tools/go.sh test ./internal/dos` 通過（含新測試）。
