# RS-2（大宇資訊，1990）：保存、逆向與 remake 準備

本檔是 Codex、Claude Code 與其他代理在本專案工作的共同入口。回覆與文件用繁體中文，程式碼本體保留原語言。

## 目標

保存這款台灣早期 DOS 射擊遊戲的素材、邏輯與製作者資訊，為之後的 clean-room remake 與 decomp 備妥證據。

- 判準是完整性。每張圖、每個顯示模式、每首曲子都要收齊，不以投報率放棄。做不完時寫明未完成項目與下一步方法。
- remake 在 `crates/`，已發行 v1.0.0。現況只看 [CONTEXT.md](CONTEXT.md)，本檔不記進度。

## 文件入口

| 檔案 | 職責 |
|---|---|
| [CONTEXT.md](CONTEXT.md) | 目前狀態、有效決策、術語表 |
| [docs/manual.md](docs/manual.md) | 原版說明書整理、掃描插圖、來源與雜湊；密碼表不收錄 |
| [WORKLIST.md](WORKLIST.md) | 待辦與完成條件 |
| [WORKLOG.md](WORKLOG.md) | 按時間追加的操作、驗證與勘誤 |
| [source-manifest.json](source-manifest.json) | 輸入 zip 與 55 個檔案的 SHA-256、角色分類 |
| [docs/re/00-identity-and-provenance.md](docs/re/00-identity-and-provenance.md) | 作品身分、製作者、外部史料與矛盾 |
| [docs/re/01-boot-chain.md](docs/re/01-boot-chain.md) | 啟動順序、加殼層、破解層、密碼檢查 |
| [docs/re/02-file-formats.md](docs/re/02-file-formats.md) | 圖庫、地圖、音樂、展示錄影格式與檔案載入對照 |
| [docs/re/03-game-logic.md](docs/re/03-game-logic.md) | 遊戲邏輯入口：主流程、時間基準、規則摘要、輸入、熱鍵 |
| [docs/re/10-dynamic-receipts.md](docs/re/10-dynamic-receipts.md) | dosgolem 實跑收據 |
| [docs/re/11-entities-combat.md](docs/re/11-entities-combat.md) | 實體、移動腳本、碰撞、武器、敵彈、亂數 |
| [docs/re/12-stages-presentation.md](docs/re/12-stages-presentation.md) | blit、調色盤、各關流程、背景、頭目、過場、介面 |
| [docs/re/13-audio-and-misc.md](docs/re/13-audio-and-misc.md) | AdLib、MIX、PC 喇叭、intro、文字模式、JOYMOUSE、編譯器指紋 |
| [docs/hd/codex-handoff.md](docs/hd/codex-handoff.md) | HD 素材交給 Codex 的範圍、規則與驗收 |
| [docs/spec/007-manual-posters.md](docs/spec/007-manual-posters.md) | 兩組紀念海報、設定、資產雜湊與前端驗收 |
| [tools/run_all.sh](tools/run_all.sh) | 從 zip 重建 `original/` 與 `workspace/`。先跑過 `tools/ida_export.sh` 才會產生用途表 |
| [tools/ida_export.sh](tools/ida_export.sh) | 用 IDA 匯出 `workspace/ida/s_export.json` 與 `s_disasm.txt` |
| [tools/q.sh](tools/q.sh) | 查反組譯與資料段，用法見 `tools/q.py` |
| [tools/dos/](tools/dos/) | 重跑動態收據。`tools/dosgolem-patches/` 是套在 `workspace/dosgolem` 上的修改 |
| [crates/](crates/) | Rust 重製。`rs2-formats` 解碼、`rs2-game` 遊戲本體、`rs2-gfx` 繪製、`rs2-audio` 音樂與喇叭、`rs2-trace` 對拍、`rs2-play` 視窗。建置與測試走 [tools/rust.sh](tools/rust.sh) |

## 硬規則

1. 分析、搜尋、轉檔、建置、測試、IDA、模擬器一律在 Docker 內執行。主機只做 docker／git 控制、狀態檢查與檔案編輯。Codex 生圖例外，因為它本身是外部服務入口，見「HD 素材」。
2. 一次性工作用 `docker run --rm`，帶目前 UID/GID、`--memory`、`--cpus`、`--pids-limit`、`--log-opt max-size=10m --log-opt max-file=3` 與 timeout，預設 `--network none`。原始輸入唯讀掛載。掛載前確認 host 來源存在。不對整庫或 `$HOME` 遞迴 chown。
3. 不碰共用 docker 資源。禁止任何 prune 與 `docker rmi`，只清自己建立的 container。不動 `~/.cache/`。
4. 原版位元組與其衍生物不進版控，也不進公開發行包：`RS-2_DOS_EN.zip`、`original/`、`workspace/`、`.i64`、解包後的 EXE、抽出的 PNG、MIDI、表格 JSON。`source-manifest.json` 只有雜湊與檔名，可以進版控。例外為 `docs/images/` 的 README 對照圖（使用者 2026-11-05 決定），以及 `docs/images/manual/` 的八張說明書閱讀圖（使用者 2026-11-11 要求整理到私人 repo，密碼表排除）；由 `tools/manual_images.py` 重建，索引見 `docs/manual.md`。`tools/leak_check.py` 只放行列明的路徑。repo 要維持私人，這批圖不進公開發行包。改 repo 可見度前要先問使用者，並從 git 歷史移除這批圖。
5. git 身分一律 `wicanr2@gmail.com`，不符就設 repo-local，並查 `git log --format=%ae | sort -u`。commit message 不放 `Claude-Session:`。私人 repo 是 `wicanr2/rs2-remake`。每次推送前跑 `tools/leak_check.py`，有命中就不推，閘門規則改動後要重跑 `--selftest`。建 PR、改 repo 可見度、外部發訊息先問。
6. 只在本專案目錄寫檔。其他 repo、`~/.claude`、`~/.codex` 與客戶資料不因位於家目錄就能寫入。
7. 不寫出密碼畫面的查表內容或答案。判定邏輯的位址與證據照常記錄，查表資料位移可寫，內容不展開。不散布改過的原版執行檔與破解工具，`SEN.*` 也一樣，只在文件中記錄其行為。

## 證據紀律

每項語意標等級：

| 等級 | 意思 |
|---|---|
| 靜態已證實 | 讀到機器碼或資料結構，且有可重跑的工具或位元組佐證 |
| 動態已證實 | 在受控執行下量到，收據記錄工具版本、輸入與結果 |
| 強推論 | 多個獨立線索一致，但缺直接證明 |
| 假說 | 只有單一線索或尚未驗證 |
| 未知 | 沒有證據 |

- IDA 的自動名稱與導覽別名不是證據。每項語意同時保留原始位址、operand、bytes 與來源。
- 位址慣例：IDA 映像基底 `10000h`，資料段 `dseg:X` 等於線性位址 `1CBC0h + X`，等於解包映像位移 `0CBC0h + X`。引用時寫清楚用哪一種。
- 反組譯查 `.i64` 的 xref，或查 `workspace/ida/s_disasm.txt` 的完整上下文。不用 `grep -v` 濾指令，濾掉的常是索引計算。
- IDA 的線性位址函式歸屬可能把 `retn` 之後的死碼算進前一個函式。判定可達性要看實際控制流，不能只看 proc 邊界。
- 外部資料庫彼此矛盾時，以原版位元組與執行結果為準，矛盾要記下來，不要默默挑一個。
- 「沒有」的結論前先換查法做正對照。要推翻既有斷言前，先找出當初支持它的證據。
- 正文只寫目前正確的內容。推翻紀錄追加到 `WORKLOG.md` 勘誤段。

## 原版來源的判讀

這份 zip 是破解組 Future Brain Inc. 的 BBS 發行版。以它為分析來源時要分開三層：

| 層 | 內容 | remake 處理 |
|---|---|---|
| 大宇原版 | 內層 `S.EXE` 邏輯、資料檔、`ASMDRV`、`JGAME` | 規則與素材的來源 |
| 破解修改 | `12F03h` 的 7 位元組靜態補丁、`SEN.EXE`、`SEN.PAS` | 只作史料，不當原版行為 |
| 破解組 intro | `0969:0100` 的包裝碼與捲動字串、`FILE_ID.DIZ` | 只作史料，不進 remake |

找到未破解的原版磁片映像時，要重跑全部雜湊與靜態比對，並把差異寫進 `docs/re/01-boot-chain.md`。

## 重製技術棧

Rust 1.99.1，edition 2024，建置與測試一律在 Docker（`rust:1.99.1-slim-bookworm`）。模擬核心不碰時間、檔案與繪圖，才能和原版逐幀對拍。規格在 [docs/spec/](docs/spec/)，001 第一關、002 全流程、003 音訊。

## 工作流程

```text
RE 證據 → DRAFT 規格 → 證據審查 → READY 規格 → 實作 → 同狀態驗證 → CONFORMED
```

- 只有 `READY` 規格能授權正式的行為或格式。實作遇到缺案時退回 `DRAFT`，不把假說寫進正式碼或 golden test。
- `READY` 規格放 `docs/spec/`，最少記錄：範圍、輸入雜湊、工具與地址空間、原始位址與 bytes、推論等級、typed behavior、失敗模式、玩家路徑、內部測試、原版 oracle、已知差異、權利邊界。
- 已知差異分兩區：「刻意不照做」與「remake-owned」。後者解出原版程序的那一刻就要改掉。
- 規則文件的語意要釘位元組簽章，工具要求簽章在映像中恰好出現一次，否則 fail-closed。換版本要重釘。
- 用 dosgolem 跑原版對拍，入口見 `tools/dos/` 與 `docs/re/10-dynamic-receipts.md`。修改放在 `workspace/dosgolem` 這份複本與 `tools/dosgolem-patches/`，不寫入 `~/cht/dosgolem`。缺的能力依 `~/cht/DOSBox-X-MCP-Debugger/dosbox-src/` 補。先讀 `~/cht/dosgolem/CLAUDE.md`。
- 本作的敵人開火只用固定亂數表，沒有全域種子，幀計數決定生成，所以固定輸入就能重播。
- 模擬器的位址、快照與 VRAM 只用來證實或推翻關於原版檔案的斷言，不能成為 remake 的資料模型。

動手前依序讀 `~/.claude/knowledge-base/retro/` 的 `retro-remake-source-selection-and-byte-signatures.md`、`compiler-runtime-helper-fingerprints.md`、`retro-remake-spec-gated-workflow.md`；IDA 另讀 `ida-pro-9.4.md`。Codex 走 `~/.codex/knowledge-base/knowledge-router.md` 的對應入口。

## IDA

本機目前只有 `ida-pro-9.4-idapython:py312-v1`，文件列為現行的 `locked-v1` 不在這台機器上。入口是 `tools/ida_export.sh`，匯出腳本在 `tools/ida/`。exit code 不算證據，要驗輸出檔存在、非空、輸入 SHA-256 正確。對同一個 `.i64` 不要連續快速開關，多個查詢合成一次執行。

## HD 素材

使用者指定 HD 素材由 Codex 處理。Claude Code 負責抽取、對照表與驗收工具，不自行生成 HD 美術。

- 流程、成品位置、方法與已知限制見 [docs/hd/codex-handoff.md](docs/hd/codex-handoff.md)。工具是 `tools/hd_*`，映像 `rs2-hdtools:r1`，定義在 `docker/hdtools.Dockerfile`。
- 把原版衍生圖送進 Codex 的 `image_generation` 等於送到 OpenAI。範圍要有使用者明確授權，授權記入 `CONTEXT.md`。不擴張解讀，被權限機制擋下時停下來問，不找繞過路徑。
- 畫風與輸出尺寸由使用者用代表性試稿定案，代理不代為選定。現行定案見 `CONTEXT.md`。
- 原稿與候選圖只留 `workspace/hd/`，不覆寫舊批次。要改就開新批次，`hd_finalize.py` 以後面的批次為準。每張記錄來源 SHA-256、提示詞、工具版本與輸出 SHA-256。
- 同組動畫影格要放同一張參考表，頭目本體要依遊戲內座標合成後整張畫。用途與分組來自 `workspace/extracted/sprite-usage.json`。
- 單純放大或插值只能叫縮放，不能稱為 HD 重繪完成。圖集完成、接入完成與玩家畫面驗收分開記錄。

## 文化保存

下列項目屬於這款遊戲的歷史，remake 與 HD 都要保留或明確記錄處置：

- 製作者署名：`MY.GO` 第 48 項的程式設計與美術工程名單，第 26、47 項的大宇資訊字樣，第 15 項的 SOFTSTAR 標誌。HD 重繪不得改字或改名。
- 四種顯示模式（VGA、EGA、CGA、MGA）的素材。單色版的圖塊數與部分尺寸都和彩色版不同，要一起保存。
- AdLib 音樂與音色表、PC 喇叭音效、JOYMOUSE 周邊工具與按鍵對照。
- 雙人模式、四段難度、展示模式錄影、開發者留下的熱鍵。
- 破解組 intro 與 `SEN.PAS` 是 BBS 時代的史料，留在文件裡，不混入遊戲本體。

## 授權

remake 與工具採已定案的 RRSAL-1.0，不重問。範本在 `~/.claude/knowledge-base/retro/rrsal-1.0.md`，規則見 `~/.claude/rulebook/85-retro-remake-licensing.md`。條款不涵蓋原版素材。大宇資訊仍在營運，本作的權利現況未查證，不把 abandonware 網站的收錄當成可散布的依據。
