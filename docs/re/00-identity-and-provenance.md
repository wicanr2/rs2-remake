# 作品身分與來源

## 輸入

| 項目 | 值 |
|---|---|
| 檔案 | `RS-2_DOS_EN.zip`，614,133 位元組，55 個檔案 |
| SHA-256 | `4e57219a90f44515f0e907e1f79a820761a0005be44b281d4450ac9ea2ba5004` |
| 每檔雜湊 | [source-manifest.json](../../source-manifest.json) |

壓縮檔內的檔案日期不可靠。多個檔案落在 1980 年與 2090 年，只有部分是 1990 年，看起來是開發機時鐘未設定造成，這點屬於強推論。日期不能拿來排序版本。

## 遊戲內證據

| 事實 | 來源 | 等級 |
|---|---|---|
| 大宇資訊製作，1990 | `MY.GO` 第 26 項圖片「大宇資訊 1990」 | 靜態已證實 |
| 英文名 SOFTSTAR 標誌 | `MY.GO` 第 15 項 | 靜態已證實 |
| 公司英文全名 | `JGAME.EXE` 字串 `(c) Copyright 1990 SOFTSTAR Information Co.,Ltd` | 靜態已證實 |
| 片名字樣 RS-2 | `BEG1.BD1`～`BD4` 第 14 項 | 靜態已證實 |
| 程式設計：施文馮 | `MY.GO` 第 48 項點陣字，並有《電腦玩家》1992 年專題佐證，見下節 | 靜態已證實，外部來源相符 |
| 美術工程：李□銘 | `MY.GO` 第 48 項點陣字 | 中間字未定，形近「凌」或「淩」 |
| 支援雙人與角色選擇 | `MY.GO` 第 83～85 項 `ROLE 1`、`ROLE 2`、`2 PLAYERS`，並有 `sub_127A5` | 靜態已證實 |
| 四段難度 | `MY.GO` 第 78～81 項，並有 `sub_12754` | 靜態已證實 |
| 原版以磁片 A、B、C 發行 | `MY.GO` 第 68 項「Please insert disk A in drive」，第 70～72 項是 A、B、C 字模。各載入函式先把 `byte_274EC` 設成 `31h`～`33h`，開檔失敗走 `sub_1C896` | 強推論，未找到實體磁片資料 |

美術工程的名字只有 16 像素點陣這一項來源，公開引用前要找手冊、包裝或當事人確認。

## 外部資料

| 來源 | 內容 | 與本作證據的關係 |
|---|---|---|
| [《電腦玩家》第 10 期（1992 年 5 月）大宇專題的部落格備份](http://shortlin.blogspot.com/2026/08/199205010.html) | 「1990 年，施仁馮（原刊如此，應為「施文馮」）又走回了老路，推出射擊遊戲「RS-2」」，並稱其「支援 VGA 顯示模式和音效卡」。歷年列表寫「RS-2　射擊　施文馮」。另列「逆襲　射擊　施文馮」，是他更早的作品 | 作者、年代、VGA 與音效卡都相符。「逆襲」是另一款遊戲 |
| [MyAbandonware](https://www.myabandonware.com/game/rs-2-1hx) | 1990，DOS，Softstar。Perspectives 寫 side view，描述文字又寫 top-down。下載項目標 English version | 年代相符。視角自相矛盾 |
| [Home of the Underdogs](https://homeoftheunderdogs.net/game.php?id=2201) | 1992，vertical shooter，Designer: Unknown | 年代與類型都和本作證據衝突 |
| [LaunchBox](https://gamesdb.launchbox-app.com/games/details/97392-rs-2) | 1990，side scrolling，單人。道具 K 半月、B 追蹤火球、S 散射、L 雷射，只有一條血 | 道具字母與 `14D2Ah` 相符。單人與 `2 PLAYERS` 衝突 |
| cool3c 文章 `article/7507` | 搜尋摘要提到大宇早期單色橫捲射擊「逆襲」與「滅」 | 取回全文得到 HTTP 403。《電腦玩家》專題把「逆襲」與 RS-2 分開列 |
| [露天拍賣搜尋摘要](https://www.ruten.com.tw/item/show?21942401746459) | 標題寫「RS-2 大磁片版（5.2 吋 3 片裝全）附操作手冊」 | 3 片與 disk A、B、C 相符。頁面已是 404，只見搜尋摘要 |

本作屬橫向捲軸的判斷有兩項依據，等級為靜態已證實。`SHT.SCN` 是 600 欄 × 20 列的地圖，顯示時從 `word_25BC6` 起算 42 欄（`18613h`～`18666h`）。敵人與頭目美術也是橫向構圖。

中文片名：《電腦玩家》專題與露天摘要都直接寫「RS-2」，目前沒有證據顯示另有中文片名。RS-2 的全名或含意查不到。《電腦玩家》專題的另一段話可作定位：施文馮在兩款射擊遊戲之後轉做益智遊戲《歡樂接龍》，專題把 RS-2 視為奠定其地位的技術基礎。

## 這份拷貝的身分

這是破解發行版，細節見 [01-boot-chain.md](01-boot-chain.md)。`S.EXE` 外層 intro 的捲動字串解碼後寫著：

> MIRACLE MAN of Future Brain Inc. is proud to present to you: 'RS-2'.

字串另提到「a shoot'em up game from the far east」與「our friend in the far east」。解碼方式：`dseg` 相對 `0969:0415`，每位元組 `(b-6) xor 35h`，結果為 0 時結束。

`SEN.PAS` 另署名 `Program design by Leu S.S.`，與 intro 的署名不同。兩者的關係未知。

## 仍缺的史料

- 盒裝、手冊、磁片映像與發行價格。
- 美術工程人員的完整姓名。
- 當年的遊戲評測與製作人員訪談。
- 未破解的 `S.EXE`，用來確認 `12F03h` 的原始位元組與密碼流程。
- 權利現況。大宇資訊仍在營運，本作是否還有人主張權利未查證。
