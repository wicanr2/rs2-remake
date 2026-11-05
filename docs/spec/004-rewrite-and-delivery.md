# 004 完整版：重寫、顯示模式與四平台交付

狀態：DRAFT（2026-11-05）

## 目標

使用者 2026-11-05 指定：

- 遊戲本體要重寫，不是模擬器。
- CGA、MGA 要做，在重製內可以切換顯示模式，也要有 HD。
- 完整版要有 Linux（AppImage）、Windows（Zip）、macOS、Android。
- GitHub 的 README 要有截圖。

## 現況與落差

遊戲沒有模擬 x86，也不含 dosgolem。dosgolem 只用來量原版行為，不進遊戲。

`rs2-core` 也還不算重寫，它是照機器碼逐函式翻成 Rust：

| 量測（`crates/rs2-core/src`） | 數量 |
|---|---|
| 程式行數 | 10,742 |
| 以原版位址命名的函式（`s11556` 這類） | 451 |
| 對暫存器欄位的存取（`self.r.`） | 2,503 |
| 對 64 KB 平坦資料段的存取（`self.ds.`） | 2,223 |
| 寫死的原版位址（`a(0x...)`） | 1,982 |

啟動時還會把 `S.EXE` 解包後的資料段整段載入記憶體，表格都從那裡讀。[AGENTS.md](../../AGENTS.md) 規定模擬器的位址與快照不能成為 remake 的資料模型，平坦資料段違反這條的精神。

## 重寫的完成條件

| 項目 | 條件 |
|---|---|
| 資料模型 | 新 crate `rs2-game`，用有型別的 `Player`、`Enemy`、`Bullet`、`Effect`、`Stage`、`TitleFlow` 等描述遊戲。沒有 `Dseg`、暫存器欄位、位址命名 |
| 表格 | 時間表、樣板、移動腳本、調色盤由 `S.EXE` 解析成有名字的結構，雜湊不符就拒絕。解析完不留原始記憶體 |
| 程序 | 依 `docs/re/` 與規格寫，依功能切模組。機器碼只在文件不夠時查，查到的結論回寫文件 |
| 行為 | 同一份輸入，等待長度、幀計數、四個平面、DAC、顯示起點、音訊事件都和原版收據相同 |
| 正式產物 | `rs2-play` 只連 `rs2-game`，不連 `rs2-core` |

## 驗證

- 既有 20 份收據繼續用。資料段逐位元組比對改成投影：`rs2-trace` 持有「遊戲狀態到原版位移」的對照，只在測試裡存在。
- `rs2-core` 降為測試參考，用來和 `rs2-game` 跑大量隨機輸入的差分。原版收據仍是最終依據。
- 原版有靠表外讀寫才成立的行為（敵彈槽進位、裝甲格誤寫、`word_2687E` 殘值等）。有型別的模型要逐項明確實作，或列進「已知差異」。這是重寫最大的風險，每項都要有收據。

## 顯示模式

- `DisplayMode`：VGA、EGA、CGA、MGA。工具列與快捷鍵切換，切換不影響模擬。
- 先盤點 101 個 `byte_1CF74` 判斷點，分出只影響繪製的與影響邏輯的。影響邏輯的要另外決定如何不影響切換。
- 原版對拍：`rs2dyn` 要補 CGA 模式 6 與 Hercules 的畫面記錄。
- HD：先用 `workspace/hd/assets/mono/` 的 1,526 張推導結果，看實際畫面後由使用者決定要不要重畫。

## 交付

| 平台 | 產物 | 工具鏈（本機既有） | 驗證 |
|---|---|---|---|
| Linux x86_64 | AppImage | `dosgolem-rust-cross`、`fd2-build-appimage` | 解開後在乾淨容器跑，Xvfb 截圖 |
| Windows x86_64 | Zip | `dosgolem-rust-cross`（mingw、`/opt/sdl2/windows`） | Wine 跑實際產物 |
| macOS | universal `.app` 的 Zip | `dosgolem-rust-cross`（osxcross、SDK 15.5） | `lipo`、Mach-O 靜態檢查。沒有 Mac，不能宣稱實機驗過 |
| Android | APK | NDK 27.2（`rich2-go-android`），另裝 Rust 的 Android 目標，SDL 的 `android-project` 在 `sdl2-sys` 內 | `rich2-android-emulator` 跑實際產物 |

- 依 `rulebook/82`，驗的是打包後的產物，不是開發機的二進位檔。
- 每個包都帶 `LICENSE`。APK 看不到檔案，要在遊戲內的關於頁顯示（`rulebook/85`）。
- Android 是移植：要有觸控按鈕、生命週期、可寫目錄，先出設計稿再實作。

## 權利邊界

未決定。見 [CONTEXT.md](../../CONTEXT.md) 的決策紀錄。原版位元組與衍生物不進公開發行包（`AGENTS.md` 硬規則 4）。
