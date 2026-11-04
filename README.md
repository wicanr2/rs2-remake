# RS-2 保存與重製

大宇資訊（Softstar）1990 年 DOS 橫向捲軸射擊遊戲《RS-2》的逆向分析、素材解析工具與 Rust 重製。目的是保存台灣早期遊戲的製作成果與技術細節。

## 原版素材不在這裡

本儲存庫不含也不散布原版的執行檔、資料檔、美術、音樂，也不含由它們衍生的圖片與音訊。要跑工具，請自備原版，放在 `RS-2_DOS_EN.zip`，工具會核對雜湊。`tools/leak_check.py` 在推送前檢查有沒有夾帶。

分析用的這一份是破解組 Future Brain Inc. 的 BBS 發行版。破解補丁與 intro 只當史料記錄，不當原版行為，也不進重製。

## 這款遊戲

玩家駕駛戰機橫向推進四關，每關結尾打頭目。可單人（兩種機體）或雙人同時遊玩，有四段難度。沒有分數、生命數或接關，血量每次被打中減半，而且跨關保留。

它支援 VGA、EGA、CGA 與 MGA 四種顯示模式，CGA／MGA 有獨立的單色圖庫，音樂走 AdLib，音效走 PC 喇叭，還有大宇自己的 JOYMOUSE 搖桿介面工具。

《電腦玩家》1992 年 5 月的大宇專題把它列為施文馮的作品，稱其在技術上有長足進步。遊戲內的製作名單也寫著程式設計施文馮。

## 現況

| 部分 | 狀態 |
|---|---|
| 檔案格式 | 全部解出，工具重建結果可重現 |
| 遊戲邏輯 | 靜態讀碼完成，未解項列在 `WORKLIST.md` |
| 原版實跑 | dosgolem 走通冷啟動、四關與片尾。頭目未實際擊殺 |
| HD 圖像 | 全部索引都有輸出，只留本機，尚未接入遊戲 |
| Rust 重製 | 尚未開始，第一份規格是 DRAFT |

唯一現況入口是 [CONTEXT.md](CONTEXT.md)。

## 重要發現

| 發現 | 文件 |
|---|---|
| `S.EXE` 是兩層 LZEXE 夾一段破解組 intro，密碼檢查已被 7 位元組補丁固化 | [01](docs/re/01-boot-chain.md) |
| 圖庫是 RLE 壓縮的索引表，彩色圖第一個平面區段是最高位，遮罩獨立成庫 | [02](docs/re/02-file-formats.md) |
| 邏輯更新率是每秒 9.10 次，AdLib 驅動改寫 PIT 但以累加器轉發原頻率 | [10](docs/re/10-dynamic-receipts.md) |
| 敵人由幀計數時間表生成，移動腳本是位元組碼，亂數是固定表 | [11](docs/re/11-entities-combat.md) |
| 頭目由合成圖、隱藏碰撞體與可見分件組成 | [12](docs/re/12-stages-presentation.md) |
| 音樂格式與 OPL2 暫存器逐欄對照，可轉成 VGM（尚未與原版逐筆比對） | [13](docs/re/13-audio-and-misc.md) |

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

## 授權

採 RRSAL-1.0（復古重製 source-available 授權條款），全文見 [LICENSE](LICENSE)。非商業用途免費，實況與影片明示允許，商業用途請洽談。條款不涵蓋原版素材。

本專案與大宇資訊沒有隸屬、合作、贊助或授權關係。原版的權利屬於各自的權利人，權利現況未查證。
