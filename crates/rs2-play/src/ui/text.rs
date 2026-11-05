//! 介面字串集中在這個檔。
//!
//! - 工具列、狀態列、設定畫面、觸控鈕的中文只寫在這裡，其他檔不放非 ASCII 的字串常值（`ui::font` 的測試會掃）。
//! - `tools/ui_font_bake.py` 讀這個檔裡的全部字串常值，把用到的字烘成 `assets/ui-font.bin`。
//!   改了字串就要重跑烘字，否則 `ui::font` 的測試會指出缺哪個字。
//! - 執行時才知道的字（手把裝置名稱）可能含字型沒有的字，畫成 `?`。

use sdl2::keyboard::Scancode as K;

/// 烘字時一定要有、但字串裡不一定出現的字（截斷用的刪節號）。
pub const EXTRA_CHARS: &str = "…";

// ---------------------------------------------------------------------------
// 工具列與狀態列
// ---------------------------------------------------------------------------

pub const BTN_START: &str = "開始／繼續";
pub const BTN_PAUSE: &str = "暫停";
pub const BTN_CONTROLS: &str = "控制設定";
pub const BTN_GAME: &str = "遊戲設定";
pub const BTN_FULLSCREEN: &str = "全螢幕";
pub const BTN_GFX: &str = "原貌／HD";
pub const BTN_MODE_PREFIX: &str = "顯示模式：";
/// 四種顯示模式的名稱，接在 `BTN_MODE_PREFIX` 後面。
pub const MODE_VGA: &str = "VGA";
pub const MODE_EGA: &str = "EGA";
pub const MODE_CGA: &str = "CGA";
pub const MODE_MGA: &str = "MGA";
pub const BTN_ABOUT: &str = "關於";
pub const BTN_QUIT: &str = "離開";
pub const BTN_HIDE: &str = "收起";

pub const ST_PAUSED: &str = "已暫停";
pub const ST_RUNNING: &str = "執行中";
pub const ST_ORIGINAL: &str = "原版規則";
pub const ST_RULES: &str = "強化規則";
pub const ST_GOD: &str = "無敵";
/// 單色圖庫（CGA、MGA 用的 21 個檔）載入失敗時，狀態列常駐的說明，完整原因在標準錯誤輸出。
/// 內附點陣沒有「圖庫」「缺檔」的字，所以用現有的字措辭。
pub const ST_MONO_MISSING: &str = "CGA、MGA 不能選：畫面沒有取到";
/// 戰場加大的關卡（規格 005「戰場大小」）裡，顯示模式的循環跳過 CGA、MGA，狀態列說明。
pub const ST_MONO_WIDE: &str = "戰場加大時 CGA、MGA 不能選";
/// 顯示模式是 CGA、MGA，戰場大小選項不是 100% 時的說明：單色進關一律 100%。
pub const ST_FIELD_MONO: &str = "CGA、MGA 的戰場固定 100%";
/// 狀態列各段之間的間隔（全形空白）。
pub const ST_SEP: &str = "　";

// ---------------------------------------------------------------------------
// HD 介面的 HUD 與載入畫面（規格 005「HD 介面」）
// ---------------------------------------------------------------------------

pub const HUD_P1: &str = "1P";
pub const HUD_P2: &str = "2P";
/// 改變規則的選項開著時，HUD 右端的小標籤。
pub const HUD_RULES: &str = "強化";
pub const HUD_GOD: &str = "無敵";

pub const LOAD_TITLE: &str = "載入中";
pub const LOAD_HD: &str = "正在準備 HD 素材";
pub const LOAD_INSTALL_TITLE: &str = "首次啟動";
pub const LOAD_INSTALL: &str = "正在解開遊戲資料";

pub fn pad_added(name: &str) -> String {
    format!("偵測到手把：{name}，請到控制設定指派。")
}

pub fn pad_unsupported_added(name: &str) -> String {
    format!("偵測到手把：{name}，沒有標準配置，不支援。")
}

pub fn pad_removed(name: &str, player: Option<usize>) -> String {
    match player {
        Some(p) => format!("手把已拔除：{name}，已取消第 {} 人的指派。", p + 1),
        None => format!("手把已拔除：{name}"),
    }
}

// ---------------------------------------------------------------------------
// 設定畫面：共通
// ---------------------------------------------------------------------------

pub const TITLE_CONTROLS: &str = "控制設定";
pub const TITLE_GAME: &str = "遊戲設定";
pub const PANEL_HINT: &str = "F8 換頁，Tab 選欄位，Enter 啟動，Esc 關閉";
pub const PANEL_HINT_TOUCH: &str = "點欄位操作，上下拖曳捲動";
pub const PANEL_CLOSE: &str = "關閉";

pub fn msg_in_development(label: &str) -> String {
    format!("「{label}」開發中，這一版還不能切換。")
}

pub const TAB_PAD: &str = "手把";
pub const TAB_PADKEYS: &str = "手把按鍵";
pub const TAB_KEYBOARD: &str = "鍵盤";
pub const TAB_GAME: &str = "遊戲";
pub const TAB_DISPLAY: &str = "顯示";

pub const ON: &str = "開";
pub const OFF: &str = "關";
pub const UNBOUND: &str = "未綁定";
pub const IN_DEVELOPMENT: &str = "開發中";

pub fn player_n(n: usize) -> String {
    format!("第 {n} 人")
}

// ---------------------------------------------------------------------------
// 手把頁
// ---------------------------------------------------------------------------

pub const PAD_NONE: &str = "尚未偵測到手把。USB／藍牙配對請由作業系統完成。";
pub const PAD_LIST_HEAD: &str = "偵測到的手把";
pub const PAD_ASSIGN_HEAD: &str = "目前指派";
pub const PAD_NOTE: &str = "USB／藍牙配對請由作業系統完成。再按一次已指派的欄位就取消指派。";
pub const PAD_UNSUPPORTED: &str = "不支援";
pub const PAD_UNASSIGNED: &str = "未指派";

pub fn pad_assign_button(player: usize, assigned: bool) -> String {
    if assigned { format!("第 {} 人：已指派", player + 1) } else { format!("指派給第 {} 人", player + 1) }
}

pub fn pad_unsupported_name(name: &str) -> String {
    format!("{name}（沒有標準配置，不支援）")
}

pub fn pad_player_line(player: usize, name: Option<&str>) -> String {
    format!("第 {} 人：{}", player + 1, name.unwrap_or(PAD_UNASSIGNED))
}

pub fn msg_pad_assigned(dev: &str, player: usize) -> String {
    format!("已把 {dev} 指派給第 {} 人。", player + 1)
}

pub fn msg_pad_taken(dev: &str, player: usize) -> String {
    format!("{dev} 已經指派給第 {} 人，不能重複指派。", player + 1)
}

pub fn msg_pad_unassigned(player: usize) -> String {
    format!("已取消第 {} 人的手把指派。", player + 1)
}

pub fn msg_pad_unsupported(dev: &str) -> String {
    format!("{dev} 沒有標準配置，目前不支援。")
}

pub const MSG_PAD_GONE: &str = "這支手把已經拔除。";

// ---------------------------------------------------------------------------
// 手把按鍵頁
// ---------------------------------------------------------------------------

pub const PADFN_COL_FN: &str = "功能";
pub const PADFN_COL_1: &str = "按鈕 1";
pub const PADFN_COL_2: &str = "按鈕 2";
pub const PADFN_FIRE: &str = "開火";
pub const PADFN_TURN: &str = "轉向";
pub const PADFN_SPLIT: &str = "分離";
pub const PADFN_PAUSE: &str = "暫停";
pub const PADFN_CONFIRM: &str = "確定";
pub const PADFN_MENU: &str = "選單";
pub const PADKEY_RESET: &str = "還原手把預設";
pub const PADKEY_NOTE1: &str = "方向固定為十字鍵與左搖桿。暫停是前端暫停，不會送進遊戲。選單是標題畫面的選單（原版的 C 鍵）。";
pub const PADKEY_NOTE2: &str = "點欄位或按 Enter 後，按手把上的按鈕。Delete 清空，Esc 取消。同一個按鈕只能用在一個功能。";
pub const PADKEY_PROMPT: &str = "請按按鈕";
pub const PADKEY_CANCELLED: &str = "已取消，保留原按鈕。";
pub const PADKEY_RESET_DONE: &str = "已還原手把按鍵預設。";

pub fn msg_padkey_taken(btn: &str, func: &str) -> String {
    format!("{btn} 已經用在「{func}」，沒有覆寫。")
}

pub fn msg_padkey_set(func: &str, slot: usize, btn: &str) -> String {
    format!("「{func}」按鈕 {}：{btn}", slot + 1)
}

pub fn msg_padkey_cleared(func: &str, slot: usize) -> String {
    format!("已清空「{func}」按鈕 {}。", slot + 1)
}

// ---------------------------------------------------------------------------
// 鍵盤頁
// ---------------------------------------------------------------------------

pub const ACT_UP: &str = "上";
pub const ACT_DOWN: &str = "下";
pub const ACT_LEFT: &str = "左";
pub const ACT_RIGHT: &str = "右";
pub const ACT_UPLEFT: &str = "左上";
pub const ACT_UPRIGHT: &str = "右上";
pub const ACT_DOWNLEFT: &str = "左下";
pub const ACT_DOWNRIGHT: &str = "右下";
pub const ACT_FIRE: &str = "開火";
pub const ACT_TURN: &str = "轉向";
pub const ACT_SPLIT: &str = "分離";

pub const KB_RESET: &str = "兩人鍵盤還原預設";
pub const KB_NOTE: &str = "點欄位或按 Enter 後按新鍵，Delete 清空，Esc 取消。衝突時不覆寫。";
pub const KB_NOTE_SPLIT: &str = "分離鍵要等「砲座像 Force」完成才有作用，現在只是先存起來。";
pub const KB_NOTE_KEYS: &str = "遊戲內不能綁 F1 到 F11、Esc、Tab、Ctrl、Alt。送進遊戲的永遠是原版的掃描碼。";
pub const KB_PROMPT: &str = "請按新鍵";
pub const KB_CANCELLED: &str = "已取消，保留原鍵位。";
pub const KB_RESET_DONE: &str = "已還原兩人鍵盤預設。";
pub const KB_LOST_FOCUS: &str = "失去焦點，已取消重綁。";

pub fn kb_head(player: usize) -> String {
    format!("第 {} 人鍵盤", player + 1)
}

pub fn msg_kb_conflict(key: &str, player: usize, act: &str) -> String {
    format!("{key} 已綁定第 {} 人{act}，沒有覆寫。", player + 1)
}

pub fn msg_kb_reserved(key: &str) -> String {
    format!("{key} 不能綁定。")
}

pub fn msg_kb_set(player: usize, act: &str, key: &str) -> String {
    format!("第 {} 人{act}：{key}", player + 1)
}

pub fn msg_kb_cleared(player: usize, act: &str) -> String {
    format!("已清空第 {} 人{act}。", player + 1)
}

// ---------------------------------------------------------------------------
// 遊戲頁
// ---------------------------------------------------------------------------

pub const GROUP_VIEW: &str = "視野";
pub const GROUP_SHOOT: &str = "射擊";
pub const GROUP_PRACTICE: &str = "練習";
pub const GROUP_CONTROL: &str = "操控";
pub const GROUP_AI: &str = "AI";

pub const OPT_FIELD_SIZE: &str = "戰場大小";
pub const OPT_WAVE_CHARGE: &str = "波動砲充能";
pub const OPT_AUTO_FIRE: &str = "按住連射";
pub const OPT_FORCE_POD: &str = "砲座像 Force";
pub const OPT_HIT_FEEDBACK: &str = "打擊回饋";
pub const OPT_INVINCIBLE: &str = "無敵";
pub const OPT_FEEL: &str = "手感（玩家機預測、短按保留）";
pub const OPT_FINE_STEP: &str = "細步移動";
pub const OPT_AI_AIM: &str = "瞄準更準";
pub const OPT_AI_FIRE: &str = "火力加強";
pub const OPT_AI_REACT: &str = "敵人會反應";
pub const OPT_AI_BOSS: &str = "頭目新招式";

pub const VAL_NEW: &str = "新";
pub const VAL_ORIGINAL_TIMING: &str = "原版時序";
pub const VAL_SIZE_100: &str = "100%";
pub const VAL_SIZE_120: &str = "120%";
pub const VAL_SIZE_150: &str = "150%";

pub const GAME_NOTE1: &str = "戰場大小在下一次進關才生效。CGA、MGA 沒有加大的畫面，戰場固定 100%。";
pub const GAME_NOTE2: &str = "改變規則的選項開著時，狀態列顯示「強化規則」。全部關閉就是原版規則。";

// ---------------------------------------------------------------------------
// 顯示頁
// ---------------------------------------------------------------------------

pub const DISP_MODE: &str = "顯示模式";
pub const DISP_TWEEN: &str = "補間";
pub const DISP_COUNTER: &str = "幀計數";
pub const DISP_SCALE: &str = "倍率";
pub const DISP_TOOLBAR: &str = "工具列";
pub const DISP_SHOWN: &str = "顯示";
pub const DISP_HIDDEN: &str = "隱藏";
pub const DISP_NOTE: &str = "倍率只改視窗大小，手機版沒有倍率。工具列隱藏後，狀態列也一起隱藏。";
pub const DISP_KEYS: &str = "快捷鍵：F1 補間，F2 幀計數，F3 原貌／HD，F4 倍率，F5 手感，F6 顯示模式，F10 設定畫面，F11 全螢幕。";

pub fn scale_value(n: u32) -> String {
    format!("{n} 倍")
}

// ---------------------------------------------------------------------------
// 觸控鈕
// ---------------------------------------------------------------------------

pub const TOUCH_FIRE: &str = "開火";
pub const TOUCH_OK: &str = "確定";
pub const TOUCH_TURN: &str = "轉向";
pub const TOUCH_SPLIT: &str = "分離";
pub const TOUCH_MENU: &str = "選單";

// ---------------------------------------------------------------------------
// 鍵盤的鍵：存檔名稱與顯示名稱
// ---------------------------------------------------------------------------

/// 可以綁定的鍵：`(掃描碼, 存檔名稱, 顯示名稱)`。F1 到 F12、Esc、Tab、Delete、Ctrl、Alt、CapsLock 等不在表內。
pub const KEYS: &[(K, &str, &str)] = &[
    (K::A, "A", "A"),
    (K::B, "B", "B"),
    (K::C, "C", "C"),
    (K::D, "D", "D"),
    (K::E, "E", "E"),
    (K::F, "F", "F"),
    (K::G, "G", "G"),
    (K::H, "H", "H"),
    (K::I, "I", "I"),
    (K::J, "J", "J"),
    (K::K, "K", "K"),
    (K::L, "L", "L"),
    (K::M, "M", "M"),
    (K::N, "N", "N"),
    (K::O, "O", "O"),
    (K::P, "P", "P"),
    (K::Q, "Q", "Q"),
    (K::R, "R", "R"),
    (K::S, "S", "S"),
    (K::T, "T", "T"),
    (K::U, "U", "U"),
    (K::V, "V", "V"),
    (K::W, "W", "W"),
    (K::X, "X", "X"),
    (K::Y, "Y", "Y"),
    (K::Z, "Z", "Z"),
    (K::Num0, "0", "0"),
    (K::Num1, "1", "1"),
    (K::Num2, "2", "2"),
    (K::Num3, "3", "3"),
    (K::Num4, "4", "4"),
    (K::Num5, "5", "5"),
    (K::Num6, "6", "6"),
    (K::Num7, "7", "7"),
    (K::Num8, "8", "8"),
    (K::Num9, "9", "9"),
    (K::Minus, "Minus", "-"),
    (K::Equals, "Equals", "="),
    (K::LeftBracket, "LeftBracket", "["),
    (K::RightBracket, "RightBracket", "]"),
    (K::Backslash, "Backslash", "\\"),
    (K::Semicolon, "Semicolon", ";"),
    (K::Apostrophe, "Apostrophe", "'"),
    (K::Grave, "Grave", "`"),
    (K::Comma, "Comma", ","),
    (K::Period, "Period", "."),
    (K::Slash, "Slash", "/"),
    (K::Up, "Up", "↑"),
    (K::Down, "Down", "↓"),
    (K::Left, "Left", "←"),
    (K::Right, "Right", "→"),
    (K::Home, "Home", "Home"),
    (K::End, "End", "End"),
    (K::PageUp, "PageUp", "PgUp"),
    (K::PageDown, "PageDown", "PgDn"),
    (K::Insert, "Insert", "Insert"),
    (K::Space, "Space", "空白鍵"),
    (K::Return, "Enter", "Enter"),
    (K::LShift, "LShift", "左 Shift"),
    (K::RShift, "RShift", "右 Shift"),
    (K::Kp0, "Kp0", "小鍵盤 0"),
    (K::Kp1, "Kp1", "小鍵盤 1"),
    (K::Kp2, "Kp2", "小鍵盤 2"),
    (K::Kp3, "Kp3", "小鍵盤 3"),
    (K::Kp4, "Kp4", "小鍵盤 4"),
    (K::Kp5, "Kp5", "小鍵盤 5"),
    (K::Kp6, "Kp6", "小鍵盤 6"),
    (K::Kp7, "Kp7", "小鍵盤 7"),
    (K::Kp8, "Kp8", "小鍵盤 8"),
    (K::Kp9, "Kp9", "小鍵盤 9"),
    (K::KpPlus, "KpPlus", "小鍵盤 +"),
    (K::KpMinus, "KpMinus", "小鍵盤 -"),
    (K::KpMultiply, "KpMultiply", "小鍵盤 *"),
    (K::KpDivide, "KpDivide", "小鍵盤 /"),
    (K::KpPeriod, "KpPeriod", "小鍵盤 ."),
    (K::KpEnter, "KpEnter", "小鍵盤 Enter"),
];

/// 可以綁定的鍵。
pub fn key_bindable(k: K) -> bool {
    KEYS.iter().any(|(c, _, _)| *c == k)
}

/// 鍵的顯示名稱。不在表內的鍵用 SDL 的英文名稱（ASCII）。
pub fn key_label(k: K) -> String {
    KEYS.iter().find(|(c, _, _)| *c == k).map(|(_, _, l)| (*l).to_string()).unwrap_or_else(|| k.name().to_string())
}

/// 存檔名稱轉鍵。
pub fn key_from_name(name: &str) -> Option<K> {
    KEYS.iter().find(|(_, n, _)| *n == name).map(|(c, _, _)| *c)
}

/// 鍵轉存檔名稱。不在表內回傳 `None`。
pub fn key_name(k: K) -> Option<&'static str> {
    KEYS.iter().find(|(c, _, _)| *c == k).map(|(_, n, _)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_table_has_unique_names_and_keys() {
        for (i, (k, n, _)) in KEYS.iter().enumerate() {
            for (k2, n2, _) in &KEYS[i + 1..] {
                assert!(k != k2, "鍵重複：{n}");
                assert!(n != n2, "存檔名稱重複：{n}");
            }
        }
        assert_eq!(key_from_name("RShift"), Some(K::RShift));
        assert_eq!(key_name(K::Slash), Some("Slash"));
        assert_eq!(key_label(K::LShift), "左 Shift");
        assert!(!key_bindable(K::F1) && !key_bindable(K::Escape) && !key_bindable(K::Tab) && !key_bindable(K::LCtrl));
    }
}
