# RS-2 保存與重製

1990 年，大宇資訊（Softstar）推出《RS-2》，一款台灣自製、類似 R-Type 的 DOS 橫向捲軸射擊遊戲。程式設計是施文馮。遊戲支援 VGA、EGA、CGA、MGA 四種顯示模式，用 AdLib 音效卡播放音樂，可以兩人同時遊玩，難度分四段。《電腦玩家》1992 年 5 月的大宇專題寫它「支援 VGA 顯示模式和音效卡」。

這個專案把它保存下來，再用現代的方式重製。原版執行檔經過逆向分析，檔案格式全部解出，遊戲規則也讀完並寫成文件，少數未解項列在 `WORKLIST.md`。全部 1,337 個圖庫索引都重繪成 4 倍解析度的 HD 版。遊戲本體用 Rust 重寫，再拿原版實際執行的紀錄逐筆比對：20 份紀錄、19,284 筆，模擬與畫面全部相同。

重製版有 Linux、Windows、macOS 與 Android 四個平台的發行包，macOS 與 Android 版還沒在實機上測試。遊戲中可以隨時切換原版圖與 HD，也能切換四種顯示模式。另外有可選的強化規則，例如加大戰場、波動砲充能、Force 式砲座與 AI 強化，預設都關閉，打開時畫面會標示，原版玩法不受影響。操作支援手把，Android 版有觸控按鈕。

![Linux 發行包的實際視窗，第一關](docs/images/game/window-stage1.png)

Linux 發行包（AppImage）的實際視窗，第一關示範模式，HD 圖。上方是工具列與狀態列。

| | |
|---|---|
| ![第一關頭目，HD](docs/images/game/stage1-boss-hd.png) | ![第二關頭目，HD](docs/images/game/stage2-boss-hd.png) |
| 第一關頭目 | 第二關頭目 |
| ![第三關，HD](docs/images/game/stage3-eyes-hd.png) | ![第四關頭目，HD](docs/images/game/stage4-fish-hd.png) |
| 第三關 | 第四關頭目 |

原版與 HD 的對照見[素材：原版與 HD](#素材原版與-hd)，發行包的其他畫面見[發行包](#發行包)。

## 原版素材

本儲存庫不含原版的執行檔、資料檔與音樂，也不含抽出的單張圖片與音訊。例外是 `docs/images/` 的原版與 HD 對照圖，裡面有各素材庫的完整總覽，所以這個 repo 維持私人，不公開。

要跑工具，請自備原版，放在 `RS-2_DOS_EN.zip`，工具會核對雜湊。`tools/leak_check.py` 在推送前檢查有沒有夾帶，圖片只放行 `docs/images/` 這個路徑。

分析用的這一份是破解組 Future Brain Inc. 的 BBS 發行版。破解補丁與 intro 只當史料記錄，不當原版行為，也不進重製。

## 這款遊戲

玩家駕駛戰機橫向推進四關，每關結尾打頭目。可單人（兩種機體）或雙人同時遊玩，有四段難度。沒有分數、生命數或接關，血量每次被打中減半，而且跨關保留。

它支援 VGA、EGA、CGA 與 MGA 四種顯示模式，CGA／MGA 有獨立的單色圖庫，音樂走 AdLib，音效走 PC 喇叭，還有大宇自己的 JOYMOUSE 搖桿介面工具。

《電腦玩家》1992 年 5 月的大宇專題把它列為施文馮的作品，稱其在技術上有長足進步。遊戲內的製作名單也寫著程式設計施文馮。

## 素材：原版與 HD

全部 1,337 個圖庫索引都有 HD 版，解析度是原版的 4 倍，畫風是賽璐璐加噴槍。其中 278 張與第三關的 1,013 個圖塊由 Codex 重繪。製作者署名與公司標誌共 4 張不交給生成模型，用 EPX 演算法放大。其餘是由重繪結果推導的位移圖、遮罩與佔位圖。

下面的圖放在 `docs/images/`，由 `tools/readme_images.sh` 產生。

### 遊戲畫面

這些是重製實際合成的畫面，左右兩張來自同一批繪製命令。原版圖模式與 DOS 原版的顯示記憶體逐位元組相同。

| 場景 | 原版 | HD |
|---|---|---|
| 開場字卡 | ![開場字卡，原版](docs/images/game/act-card-orig.png) | ![開場字卡，HD](docs/images/game/act-card-hd.png) |
| 第一關 | ![第一關，原版](docs/images/game/stage1-scroll-orig.png) | ![第一關，HD](docs/images/game/stage1-scroll-hd.png) |
| 第一關頭目 | ![第一關頭目，原版](docs/images/game/stage1-boss-orig.png) | ![第一關頭目，HD](docs/images/game/stage1-boss-hd.png) |
| 第二關敵艦 | ![第二關敵艦，原版](docs/images/game/stage2-ship-orig.png) | ![第二關敵艦，HD](docs/images/game/stage2-ship-hd.png) |
| 第二關頭目 | ![第二關頭目，原版](docs/images/game/stage2-boss-orig.png) | ![第二關頭目，HD](docs/images/game/stage2-boss-hd.png) |
| 第三關 | ![第三關，原版](docs/images/game/stage3-eyes-orig.png) | ![第三關，HD](docs/images/game/stage3-eyes-hd.png) |
| 第三關機械基地 | ![第三關機械基地，原版](docs/images/game/stage3-base-orig.png) | ![第三關機械基地，HD](docs/images/game/stage3-base-hd.png) |
| 第四關 | ![第四關，原版](docs/images/game/stage4-fly-orig.png) | ![第四關，HD](docs/images/game/stage4-fly-hd.png) |
| 第四關頭目 | ![第四關頭目，原版](docs/images/game/stage4-fish-orig.png) | ![第四關頭目，HD](docs/images/game/stage4-fish-hd.png) |
| 片尾 | ![片尾，原版](docs/images/game/ending-orig.png) | ![片尾，HD](docs/images/game/ending-hd.png) |

### 各素材庫

每組左邊是原版，右邊是 HD。

<details>
<summary><code>MY</code>：玩家機、子彈、爆炸、介面字樣、標題與署名（86 項）</summary>

<img src="docs/images/overview/MY.png" alt="MY 素材庫，原版與 HD 對照" width="100%">
</details>

<details>
<summary><code>SHT1</code>：第一關的敵人與頭目（36 項）</summary>

<img src="docs/images/overview/SHT1.png" alt="SHT1 素材庫，原版與 HD 對照" width="100%">
</details>

<details>
<summary><code>SHT2</code>：第二關的敵人與頭目（36 項）</summary>

<img src="docs/images/overview/SHT2.png" alt="SHT2 素材庫，原版與 HD 對照" width="100%">
</details>

<details>
<summary><code>SHT3</code>：第三關的敵人與頭目（44 項）</summary>

<img src="docs/images/overview/SHT3.png" alt="SHT3 素材庫，原版與 HD 對照" width="100%">
</details>

<details>
<summary><code>SHT4</code>：第四關的敵人與頭目（35 項）</summary>

<img src="docs/images/overview/SHT4.png" alt="SHT4 素材庫，原版與 HD 對照" width="100%">
</details>

<details>
<summary><code>SHT5</code>：片尾（5 項）</summary>

<img src="docs/images/overview/SHT5.png" alt="SHT5 素材庫，原版與 HD 對照" width="100%">
</details>

<details>
<summary><code>BH1</code>：第一關背景（5 項）</summary>

<img src="docs/images/overview/BH1.png" alt="BH1 素材庫，原版與 HD 對照" width="100%">
</details>

<details>
<summary><code>BH2</code>：第二關背景（28 項）</summary>

<img src="docs/images/overview/BH2.png" alt="BH2 素材庫，原版與 HD 對照" width="100%">
</details>

<details>
<summary><code>BH4</code>：第四關背景（1 項）</summary>

<img src="docs/images/overview/BH4.png" alt="BH4 素材庫，原版與 HD 對照" width="100%">
</details>

<details>
<summary><code>BEG1</code>：開場（32 項）</summary>

<img src="docs/images/overview/BEG1.png" alt="BEG1 素材庫，原版與 HD 對照" width="100%">
</details>

<details>
<summary><code>J1</code> 至 <code>J4</code>：關間過場（共 10 項）</summary>

<img src="docs/images/overview/J1.png" alt="J1 素材庫，原版與 HD 對照" width="100%">
<img src="docs/images/overview/J2.png" alt="J2 素材庫，原版與 HD 對照" width="100%">
<img src="docs/images/overview/J3.png" alt="J3 素材庫，原版與 HD 對照" width="100%">
<img src="docs/images/overview/J4.png" alt="J4 素材庫，原版與 HD 對照" width="100%">
</details>

<details>
<summary><code>MAP</code>：第三關地圖圖塊（1,019 項）。上方是原版，下方是 HD，只取地圖橫向 1200 到 1680 像素這一段</summary>

<img src="docs/images/game/stage3-map.png" alt="第三關地圖一段，上方原版，下方 HD" width="100%">
</details>

CGA 與 MGA 的單色版另有 1,526 張，由 Codex 以灰階賽璐璐重繪，再轉成黑白點陣，署名與公司標誌照舊用原版放大，這裡沒有列出。

## 現況

| 部分 | 狀態 |
|---|---|
| 檔案格式 | 全部解出，工具重建結果可重現 |
| 遊戲邏輯 | 靜態讀碼完成，未解項列在 `WORKLIST.md` |
| 原版實跑 | dosgolem 走通冷啟動、四關與片尾，第一關頭目已實際擊殺 |
| HD 圖像 | 全部索引都有輸出。單張成品只留本機，`docs/images/` 有對照圖。全流程接入無缺圖，配色跟著調色盤走 |
| Rust 重製 | 從標題玩到片尾，含開場動畫、示範、選單、雙人、四關、過場、片尾。模擬與畫面和原版逐筆相同（20 份收據 19,284 筆） |
| 音樂與音效 | 重製自帶 OPL2 合成器。暫存器寫入和原版驅動逐筆相同，喇叭事件也相同。波形只和參考實作比過品質 |
| 顯示模式 | VGA、EGA、CGA、MGA 四種都能在遊戲中切換，各有原版圖與 HD。CGA、MGA 用原版的單色圖庫 |
| 強化選項 | 十二項，在工具列的「遊戲設定」裡，見[強化選項](#強化選項) |
| 發行包 | Linux AppImage、Windows zip、macOS universal（.app）、Android APK。內含原版資料與 HD，只留本機，不公開 |
| 尚未驗證 | 真實桌面的視窗與音效裝置、GPU 繪圖器、全螢幕、macOS 實機、Android 實機與觸控、實體手把都沒測過，只在容器、Wine 與 Android 模擬器跑過。手感與強化選項的平衡沒有真人驗證 |
| 核心的寫法 | 遊戲沒有模擬 x86，dosgolem 只用來量原版行為，不進遊戲。`rs2-game` 是有型別的重寫，表格啟動時從 `S.EXE` 解析，對拍時才把狀態還原成資料段比對（[規格 004](docs/spec/004-rewrite-and-delivery.md)） |

唯一現況入口是 [CONTEXT.md](CONTEXT.md)。

## 試玩

需要 Docker、X11 桌面，以及自備的原版。先跑過 `tools/run_all.sh`。音效走 PulseAudio 或 ALSA，開不起來時遊戲照跑但沒有聲音。

```sh
tools/rust.sh play-build
tools/rust.sh play              # 有 workspace/hd/assets 就用 HD，加 --no-hd 改原版圖
```

從標題開始，按 C 開選單（難度與玩家數），其他鍵開始遊戲。

| 操作 | 玩家 1 | 玩家 2 | 手把 |
|---|---|---|---|
| 移動 | 方向鍵，Home、PgUp、End、PgDn 斜向 | T、B、F、H，R、Y、V、N 斜向 | 十字鍵、左搖桿 |
| 開火 | 右 Shift | 左 Shift | A、RT |
| 砲座轉向 | `/` | Z | B、LT |
| 砲座分離 | `.` | X | RB、LB |

砲座分離只在「砲座像 Force」開著時有作用。按鍵與手把按鈕都能在「控制設定」改。Esc 是原版的暫停，S 切換音效，Ctrl+Q 離開。

| 熱鍵 | 作用 |
|---|---|
| F1 | 畫面補間 |
| F2 | 幀計數 |
| F3 | 原版圖與 HD |
| F4 | 視窗倍率 |
| F5 | 手感（玩家機預測與短按保留，關掉是原版時序） |
| F6 | 顯示模式：VGA、EGA、CGA、MGA |
| F10 | 開關設定畫面，裡面 F8 換頁 |
| F11 | 全螢幕 |

加 `--no-audio` 關聲音，`--mode cga` 指定顯示模式，`--original-bugs` 保留原版缺陷（預設修正）。完整選項見 `--help`。

### 強化選項

工具列的「遊戲設定」有十二個選項。改變規則的預設關，只影響畫面與聲音的預設開。示範錄影一律照原版規則播放，所以重播結果不受影響。

| 分組 | 選項 | 預設 |
|---|---|---|
| 視野 | 戰場大小：100%、120%、150%，只加寬水平方向，下一次進關時生效。加寬時不能選 CGA、MGA | 100% |
| 射擊 | 波動砲充能、按住連射、砲座像 Force | 關 |
| 射擊 | 打擊回饋 | 開 |
| 練習 | 無敵 | 關 |
| 操控 | 手感 | 開 |
| 操控 | 細步移動 | 關 |
| AI | 瞄準更準、火力加強、敵人會反應、頭目新招式 | 關 |

規則與數值見[規格 005](docs/spec/005-enhanced-mode.md)。

### 發行包

```sh
bash tools/package/build_all.sh v.1.0.0-YYYYMMDD
ANDROID_SRC=$PWD tools/package/android_apk.sh v.1.0.0-YYYYMMDD arm64-v8a x86_64
```

成品放在 `dist-all/`，內含原版資料與 HD 圖，只供自己保存，不上傳。步驟與驗證見[規格 004](docs/spec/004-rewrite-and-delivery.md)。目前的版本是 v1.0.0（`v.1.0.0-20261106`），另有一支 72 秒的推廣影片，由 `tools/promo/make.sh` 產生，同樣只留本機。

下面的畫面都取自 v.1.0.0-20261106 的發行包：桌面版由 `tools/package/readme_shots.sh` 在容器裡執行 AppImage 截取，Android 版取自模擬器的煙霧測試（`tools/package/android_smoke.sh`）。

| | |
|---|---|
| ![戰場大小 100%](docs/images/game/field-100.png) | ![戰場大小 150%](docs/images/game/field-150.png) |
| 戰場大小 100%，第一關同一步 | 戰場大小 150%，前方多看到 20 欄 |
| ![遊戲設定頁](docs/images/game/window-settings.png) | ![CGA 單色 HD](docs/images/game/mono-cga-hd.png) |
| 設定畫面的遊戲頁 | CGA 模式，單色 HD |

![Android 版第一關](docs/images/game/android-stage1.png)

Android 版在模擬器上執行第一關，左下是方向鍵，右下是開火與砲座轉向。

## 重要發現

| 發現 | 文件 |
|---|---|
| `S.EXE` 是兩層 LZEXE 夾一段破解組 intro，密碼檢查已被 7 位元組補丁固化 | [01](docs/re/01-boot-chain.md) |
| 圖庫是 RLE 壓縮的索引表，彩色圖第一個平面區段是最高位，遮罩獨立成庫 | [02](docs/re/02-file-formats.md) |
| 邏輯更新率是每秒 9.10 次，AdLib 驅動改寫 PIT 但以累加器轉發原頻率 | [10](docs/re/10-dynamic-receipts.md) |
| 敵人由幀計數時間表生成，移動腳本是位元組碼，亂數是固定表 | [11](docs/re/11-entities-combat.md) |
| 頭目由合成圖、隱藏碰撞體與可見分件組成 | [12](docs/re/12-stages-presentation.md) |
| 音樂格式與 OPL2 暫存器逐欄對照，驅動的移植和原版的寫入逐筆相同 | [13](docs/re/13-audio-and-misc.md) |
| 喇叭聲全來自畫爆炸圖的迴圈，除數取自圖的像素資料 | [13](docs/re/13-audio-and-misc.md) |
| 整局無傷到第二關幀 260 時原版會卡死，第四關 Ctrl+C 進片尾會把表外記憶體當圖畫 | [12](docs/re/12-stages-presentation.md) |

## 重建分析結果

需要 Docker。下列指令只在容器內處理資料，輸出落在不進版控的 `original/` 與 `workspace/`。

```sh
tools/run_all.sh        # 解壓、解包、抽素材、匯出表格
tools/ida_export.sh     # 需要本機 IDA 9.4 映像
tools/run_all.sh        # 有 IDA 匯出才會產生素材用途表
```

動態收據的重跑方式見 [docs/re/10-dynamic-receipts.md](docs/re/10-dynamic-receipts.md)，HD 流程見 [docs/hd/codex-handoff.md](docs/hd/codex-handoff.md)。

## 文件

| 檔案 | 內容 |
|---|---|
| [AGENTS.md](AGENTS.md) | 工作規則、證據等級、邊界 |
| [CONTEXT.md](CONTEXT.md) | 現況、決策、術語 |
| [WORKLIST.md](WORKLIST.md) | 待辦 |
| [docs/re/](docs/re/) | 逆向文件 |
| [docs/spec/](docs/spec/) | 重製規格 |
| [docs/hd/](docs/hd/) | HD 素材 |
| [crates/](crates/) | Rust 重製 |

## 授權

採 RRSAL-1.0（復古重製 source-available 授權條款），全文見 [LICENSE](LICENSE)。非商業用途免費，實況與影片明示允許，商業用途請洽談。條款不涵蓋原版素材，也不涵蓋 `docs/images/` 的對照圖與 HD 圖。

本專案與大宇資訊沒有隸屬、合作、贊助或授權關係。原版的權利屬於各自的權利人，權利現況未查證。
