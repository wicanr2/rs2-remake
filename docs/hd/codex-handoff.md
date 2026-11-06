# HD 素材

HD 素材由 Codex 重繪，Claude Code 負責抽取、排版、切圖、衍生與驗收。使用者選定的畫風是賽璐璐加噴槍（`tools/hd-styles/a-cel-airbrush.txt`），輸出是邏輯尺寸 4 倍，全螢幕 1280×800。成品在 `workspace/hd/assets/`，不進版控。

## 看成品

| 路徑 | 內容 |
|---|---|
| `workspace/hd/assets/overview/<庫>.png` | 原版與 HD 並排總覽，每組左原版右 HD |
| `workspace/hd/assets/<庫>/<索引>.png` | HD 單張，RGBA |
| `workspace/hd/assets/<庫>/masks/<索引>.png` | HD 遮罩，255 保留背景，0 清掉背景 |
| `workspace/hd/assets/<庫>/variants/<調色盤>/<索引>.png` | 同一張圖在其他關卡的配色 |
| `workspace/hd/assets/mono/<檔名>/<索引>.png` | CGA／MGA 單色版 |
| `workspace/hd/assets/MAP/objects/objNN.png` | 第三關物件整圖，remake 優先用這些 |
| `workspace/hd/assets/maps/SHT.SCN.hd.png` | 第三關完整地圖，19200×800 |
| `workspace/hd/assets/hd-manifest.json`、`hd-variants-manifest.json` | 每張的方法、來源、雜湊、量測 |

## 每種素材怎麼處理

| 方法 | 數量 | 說明 |
|---|---|---|
| `codex-redraw` | 278 | 進參考表給 Codex 重繪，切回單張 |
| `map-object-slice` | 1,013 | 第三關圖塊。先把 13 種連通物件切成 16 個面板重繪，接合後切回 |
| `derived-shift` | 14 | `BH2` 像素位移相位，由基礎磚循環位移 |
| `derived-mask-from-alpha` | 7 | 以圖形式存放的遮罩（`BH1` 第 4 項、`BH2` 第 2、3、23～26 項），由 HD alpha 反推 |
| `protected-epx-scale` | 4 | `MY` 第 15、26、47、48 項，公司標誌與署名，不交給生成模型，用 EPX 對索引圖放大 |
| `placeholder-nearest` | 20 | 高 1 列的佔位圖，直接放大 |
| `map-blank` | 1 | 全黑的空白圖塊 |
| 調色盤變體 `palette-recolor` | 326 | 依色號對應重新上色，未經生成模型。沒有涵蓋的組合由 `rs2-gfx` 在執行時換色 |
單色 1,526 張（`workspace/hd/assets/mono/`，每張的方法與雜湊在 `assets/mono-manifest.json`）。使用者 2026-11-06 選定 m2 灰階賽璐璐（`tools/hd-styles/m2-gray-cel-r2.txt`），Codex 畫灰階，工具以 4×4 Bayer 有序抖動轉 1-bit。批次 `full-mono-m2-r1`（89 張表）與 `full-mono-m2-r2`（3 張重畫）：

| 單色方法 | 數量 | 說明 |
|---|---|---|
| `mono-map-tile-codex+bayer4` | 1,006 | 第三關圖塊。單色用 `SHT.SCR` 的配置，排成 12 個物件、15 個面板重繪，每種圖塊取縮回單色格後最像原版的出現位置 |
| `mono-codex-redraw+bayer4` | 224 | 一般圖 |
| `mono-codex-redraw+thr128` | 28 | 文字類，抖動會把筆畫打成點線，改用門檻 128 |
| `mono-codex-redraw+bayer4+sd-overlay` | 17 | 開場飛船與噴焰，只重繪物件，矩形其餘部分維持原版 |
| `mono-mask-from-hd-alpha` | 173 | 遮罩，由重繪圖的 alpha 反推 |
| `mono-mask-nearest` | 35 | 沒有重繪配對的遮罩，直接放大 |
| `mono-derived-shift` | 14 | `BH2` 像素位移相位，由基礎磚位移 |
| `mono-nearest-placeholder`、`mono-nearest-unused`、`mono-nearest-blank` | 25 | 佔位圖、遊戲不畫的項、全黑圖塊 |
| `mono-credits-nearest` | 4 | `MY` 第 15、26、47、48 項署名與商標，不送 Codex，原版單色最近鄰放大 |

- 子彈、道具、爆炸：提示詞要求淺色本體、純白核心加一圈純黑細輪廓，在黑太空與亮地面上都看得見。轉 1-bit 後亮點比例低於原版 0.8 倍的再提高 gamma（4 項）。
- `BH1` 第 1 項是橫向重複的 32 像素窄條，取 Codex 稿每列平均只留垂直漸層再抖動，左右必接得上，代價是沒有煙霧紋理。
- 先前由彩色 HD 推導的抖動版備份在 `workspace/hd/mono-derived-v1/`。`hd_variants.py` 看到 `assets/mono-manifest.json` 就不再推導單色。

遮罩庫的 201 張中，有配對使用的由 HD alpha 反推並外擴一個邏輯像素，重現原版的黑邊；沒配對的殘留遮罩直接放大。

## 重建流程

```sh
tools/run_all.sh                       # 抽取
tools/ida_export.sh && tools/run_all.sh  # 第一次要有 IDA 匯出才有用途表
tools/hd_docker.sh hd_plan.py /in /w /w/hd/plan.json
tools/hd_docker.sh hd_ref.py /in /w <批次> /tools/hd-styles/a-cel-airbrush.txt [名稱子字串...]
tools/hd_run.sh workspace/hd/<批次> 4    # 唯一在主機跑的步驟，呼叫 codex exec
tools/hd_docker.sh hd_cut.py /in /w <批次>
tools/hd_docker.sh hd_finalize.py /in /w full-a-r1 full-a-r2   # 後面的批次覆蓋前面
tools/hd_docker.sh hd_postfix.py /w                            # 接縫與邊線的確定性修補
tools/hd_docker.sh hd_variants.py /in /w
tools/hd_docker.sh hd_basepal.py /in /w                        # 每張圖的基準調色盤，執行時換色用
tools/hd_docker.sh hd_overview.py /in /w
```

單色 HD 的重繪（m2）另走一條：

```sh
tools/hd_docker.sh hd_mono_full_plan.py /in /w full-mono-m2-r1
tools/hd_docker.sh hd_mono_full_ref.py /in /w full-mono-m2-r1 /tools/hd-styles/m2-gray-cel-r2.txt
tools/hd_run.sh workspace/hd/full-mono-m2-r1 4
tools/hd_docker.sh hd_mono_full_cut.py /in /w mono-m2-candidate full-mono-m2-r1 full-mono-m2-r2
# 候選 workspace/hd/mono-m2-candidate/ 看過實機對照、使用者同意後，複製 mono/ 與 mono-manifest.json 進 assets/
```

`hd_mono_full_*` 的參數細節見各檔開頭；`r2` 的參考表用 `--plan-from full-mono-m2-r1` 取同一份計畫。

`hd_run.sh` 的實際呼叫：

```sh
codex exec --skip-git-repo-check -s read-only -C <ref> -i <ref.png> [-i <extra.png>] -o <last.txt> - < <prompt.txt>
```

生成的圖在 `~/.codex/generated_images/<session id>/`，工具複製成 `masters/<名>.png`。已有原稿的不再呼叫，不覆寫。每批的 `logs/_receipts.tsv` 記錄表名、session id、原稿 SHA-256 與 Codex 版本，`prompts/` 與 `ref/` 保留每次實際送出的內容。

輸出批次 `full-a-r1`（102 張表，其中 3 張沿用試稿）與 `full-a-r2`（6 張重畫）。r2 帶 `RS2_EXTRA_REF_BATCH=full-a-r1` 環境變數產生參考表，才會取到 `MAP-obj09-p0` 當第二張參考圖。

## 排版規則

- 每張參考表 1536×1024、1024×1536 或 1024×1024，倍率取可放下的最大整數。一般最低 4 倍。
- 同組的動畫影格放同一張表，分組來自 `workspace/extracted/sprite-usage.json`。放不下就對半切。64×64 以下的單張小圖，同庫同調色盤併表，每表最多 8 張。
- 頭目本體依遊戲內座標合成後整張重繪，切回才不會有接縫：`SHT1` 第 33～35 項、`SHT2` 第 32、33 項、`SHT2` 第 34、35 項。這三張倍率只有 3，切回時放大 1.33 倍。
- 地圖面板最寬 46 欄，相鄰面板重疊 4 欄，切回後線性混合。
- 底色取離圖內顏色最遠的洋紅、綠或青。
- 覆寫方式繪製的圖（連黑色也蓋掉底下）整個矩形不透明。外圈殘留的底色用內側像素補，內部整片底色視為黑色。
- 開場飛船與噴焰（`BEG1` 第 8～13、18～28 項）原版是連同地球雲層一起貼上的矩形。HD 地球不可能與補丁雲層逐像素吻合，所以改成只畫物件、四周透明，由 remake 疊在 HD 地球上。這是刻意的差異。

## 驗收紀錄

| 項目 | 結果 |
|---|---|
| 工具正負對照 | 把參考表原樣當原稿切回，IoU 為 1.0；整張位移後最低降到 0.22 |
| 透明件輪廓 IoU | 見 `metrics.json`，最低約 0.82（`SHT3` 第 37 項，一張小弧形）。工具允許外擴 1 個邏輯像素，讓階梯邊畫成順滑曲線 |
| 不透明件 | IoU 恆為 1，改看 `luma_corr`。數值低的都人工看過，來自暗圖或畫風差異 |
| 文字 | `ACT`、數字、選單字樣、換片提示、`RS-S` 變 `RS-2`、`END`、字母圓球全部目視正確 |
| 全批目視 | sonnet 代理看完第一批 102 張，部分是合併縮圖，小圖細節可能漏看。重畫項與配色修正進第二批，第二批 6 張中主代理親看 4 張（飛船、噴焰、眼睛、岩壁），手部與砲座只看量測值 |
| 完整性 | `hd_finalize.py` 檢查全部 1,337 個索引都有輸出，地圖沒有未填的格，否則中止 |

接入遊戲的驗收見下面「接入驗收」。

## 已知限制

- 第三關岩壁在不同物件有風格差異（稜角巨石與裝甲圓石），只重畫了最離群的 `MAP-obj09-p1`。
- 地圖圖塊單張取自第一次出現的位置，同一塊重複使用時接縫可能明顯。remake 應優先用物件整圖。
- 頭目損壞貼片（`SHT1` 第 19～24 項等）和本體分開畫，貼上時可能有小色差。
- `MY` 第 31 項重畫後，砲座影格的輪廓 IoU 約 0.98。
- 調色盤變體是色彩映射，同色階的細節會比基礎版扁平，爆炸類最明顯。第二關頭目在 `stage2_c` 下，石塊邊緣略帶紅色。
- 原版循環少數幾個色號做火焰閃爍，HD 圖沒有色號，所以 HD 的火焰是靜止的。
- 調色盤變體是推導結果。單色版已由 Codex 重繪；開場飛船的矩形邊緣仍看得出細微痕跡，`MY` 第 30 至 40 項砲座的輪廓 IoU 約 0.72 至 0.82，約 16 張地圖圖塊和原版單色的相關為負（最低 `MAP` 第 241 項 -0.17）。
- 第 48 項署名的中間字未確認，放大後也是原點陣。
- 破解組 intro 不做 HD，它不是大宇的素材。音樂沒做 HD，本文件只涵蓋圖像。

## 接入驗收

`LayerSink` 用同一批繪製命令合成 HD。`l9_full_flow_hd_coverage` 走 20 份收據，缺圖 0。`hd_tour` 用全流程按鍵走過四關與片尾，每關多跑一段，存原版與 HD 對照到 `workspace/rs2/shots/tour/`，四關都目視過，位置、遮罩與配色和原版一致。

HD 畫布是 RGBA，沒有色號，原版改 DAC 會連已畫好的像素一起變色，這裡這樣對應：

| 情況 | 處理 |
|---|---|
| 換成另一張有名字的調色盤 | 兩個顯示頁整塊換色，血條跟著變 |
| 圖在目前的調色盤沒有靜態變體 | 依 `base-palette.tsv` 從基本圖的調色盤換色（`layer/recolor.rs`） |
| 背景寫進暫存區 | 保留圖原本的調色盤，第一次被複製到顯示頁才換。原版在載入用的調色盤下寫背景，提早換色會來回轉換失真 |
| 整體淡入淡出、DAC 不在清單內 | 畫布不動，輸出時換色 |
| 只有少數色號在變（少於 6 個） | 不換。這些色號只影響用它畫的像素，HD 沒有色號，換色會染到相近的顏色 |

第一關的接縫與邊線由 `tools/hd_postfix.py` 修補，不需要生成模型：

| 問題 | 修法 |
|---|---|
| `BH1` 第 0 至 3 項循環接縫 | 把接縫兩側的顏色差平攤到附近的欄，形狀不動，遮罩不受影響 |
| 山影與倒懸岩交界的水平色帶 | 第 3 項底部與第 2 項頂部漸暗成黑，原版這兩處都是黑色 |
| 頭目本體的矩形邊線（`SHT1` 第 33 至 35 項） | 接近黑的暗綠壓成純黑 |

## 規則

- 把原版衍生圖送進 Codex 等於送到 OpenAI。使用者已授權本作全部圖像素材的 HD 用途，見 `CONTEXT.md`。再擴大用途（公開、發行、進版控）要另外問。
- 候選圖與原稿只留 `workspace/hd/`，不覆寫舊批次。單張成品也只留本機，進版控的只有 `docs/images/` 的 README 對照圖（使用者 2026-11-05 決定）。
- 圖片內的文字要逐字核對。署名與商標不交給生成模型。
