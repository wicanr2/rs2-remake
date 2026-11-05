//! 核心狀態與入口。規格見 `docs/spec/001-rust-first-stage.md`。

use crate::draw::{AudioCmd, BegGroup, DrawCmd, MixFile, ObjId, Pool, ShtGroup};
use crate::dseg::{Dseg, a};
use crate::ebullet::EnemyBullets;
use crate::entity::Enemies;
use crate::flow::{Flow, Scenes};
use crate::fx::Effects;
use crate::mirror::{demo_playing, stage_cleared};
use crate::pbullet::PlayerBullets;
use crate::player::Players;
use crate::regs::Regs;
use crate::scenery::Scenery;
use crate::title::Title;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

/// 原版缺陷的修正開關。遊戲預設全開，對拍全關。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fixes {
    pub bullet_slot_carry: bool,
    pub armor_cell_index: bool,
    pub seeker_spawn_leak: bool,
    /// 第二關 `131C9h`：到幀 260 為止一次都沒被打中時，原版永遠卡在 `sub_1316C`。
    pub no_hit_hang: bool,
}

impl Fixes {
    pub const NONE: Fixes = Fixes { bullet_slot_carry: false, armor_cell_index: false, seeker_spawn_leak: false, no_hit_hang: false };
    pub const ALL: Fixes = Fixes { bullet_slot_carry: true, armor_cell_index: true, seeker_spawn_leak: true, no_hit_hang: true };
}

/// 改變規則的強化選項（規格 005）。全關等於原版，對拍永遠全關。不屬於原版狀態，不進資料段。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    pub wave_charge: bool,
    pub auto_fire: bool,
    pub force_pod: bool,
    pub fine_step: bool,
    pub ai_aim: bool,
    pub ai_fire: bool,
    pub ai_react: bool,
    pub ai_boss: bool,
}

impl Rules {
    pub const ORIGINAL: Rules = Rules {
        wave_charge: false,
        auto_fire: false,
        force_pod: false,
        fine_step: false,
        ai_aim: false,
        ai_fire: false,
        ai_react: false,
        ai_boss: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Demo,
    /// `difficulty` 是選單的四段（0～3）。`None` 是不經選單，難度字維持映像值 0、0。
    NewGame { difficulty: Option<u8> },
}

/// 全遊戲流程的入口，對應原版函式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// `sub_11556`：標題迴圈，正式遊戲從這裡開始。
    Title,
    Stage1,
    Stage2,
    Stage3,
    Stage4,
    Ending,
}

/// 遊戲讀的原版資料。圖庫在 `rs2-gfx`，這裡只有會進模擬的部分。
#[derive(Clone, Debug, Default)]
pub struct GameFiles {
    pub image: Vec<u8>,
    pub dm1: Vec<u8>,
    pub dm2: Vec<u8>,
    pub scn: Vec<u8>,
    pub scr: Vec<u8>,
}

/// 離開關卡的原因，等於原版主迴圈 `dx` 的字母。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// `E`：玩家全滅、Ctrl+R 或示範被按鍵打斷。
    End,
    /// `Q`：Ctrl+Q。
    Quit,
    /// `C`：過關或 Ctrl+C。
    Clear,
}

/// 上一步到這一步之間送達的 set 1 掃描碼，含放開碼與自動重複。
#[derive(Clone, Copy, Debug, Default)]
pub struct Input<'a> {
    pub scancodes: &'a [u8],
}

#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub draws: Vec<DrawCmd>,
    /// 這一步的音訊命令。
    pub audio: Vec<AudioCmd>,
    pub exit: Option<Exit>,
    /// 暫停中，這一步沒有推進模擬。
    pub paused: bool,
    /// 到下一步之前要等幾個 18.2 Hz 的 tick。
    pub ticks: u16,
}

/// 第一幀的 `dseg` 差異豁免範圍（規格 001「初始狀態」），閉區間。
pub const A0_EXEMPT: &[(u16, u16)] = &[
    (0x000E, 0x000E),
    (0x0018, 0x0020),
    (0x0336, 0x034A),
    (0x0386, 0x03AF),
    (0x03D5, 0x03E3),
    (0x0586, 0x0592),
    (0x07E2, 0x07EE),
    (0x0EED, 0x0EFC),
    (0x9CB0, 0x9CB3),
    (0x9CBC, 0x9CC8),
    (0x9CE1, 0x9CE5),
    (0xA7DA, 0xA7DC),
    (0xA7F7, 0xA7F8),
    (0xA852, 0xA858),
    (0xA92A, 0xA931),
    (0xA94B, 0xA94C),
    (0xA9AB, 0xA9AB),
    (0xA8E7, 0xA8E7),
    (0xA8FA, 0xA8FA),
    (0xA904, 0xA904),
    (0xE340, 0xFFFF),
];

/// 全流程對拍（規格 002）的資料段豁免範圍，閉區間。起始內容是規格 001 的 `A0_EXEMPT`、
/// 差分模糊測試另外發現的三處、等待計時器。每一項寫明哪些函式讀寫它；新增的項目也要寫理由。
/// 收據（`wt_*`）與 B1 都沒有出現差異的 A0 項目已刪（`000Eh`、`0586h`～`0592h` 標題動畫計數、按鍵表 `A8E7h`、`A8FAh`、`A904h`），
/// `0336h`～`034Ah` 收成實際會變的兩段。`RS2_DS_REPORT=1` 跑 `tests/wait.rs` 會列出含豁免的全部差異，可重新檢查。
pub const DS_EXEMPT: &[(u16, u16, &str)] = &[
    // ---- A0_EXEMPT（規格 001「初始狀態」）----
    (0x0018, 0x0020, "文字畫面的游標與屬性；啟動函式專用"),
    (0x0336, 0x0338, "sub_113B2、sub_1149F 的工作變數；載入專用"),
    (0x0348, 0x034A, "sub_113B2、sub_1149F 的工作變數；載入專用"),
    (0x0386, 0x03AF, "sub_10162 的列位址；只有 MGA 分支 sub_1117F 讀"),
    (0x03D5, 0x03E3, "屬性暫存器工作表；調色盤由 vram／dac 雜湊比對涵蓋"),
    (0x07E2, 0x07EE, "密碼函式與 INT 8 的 word_1D3A8、word_1D3AE；byte_1D3A2 第二關才讀（130AFh），其餘屬密碼"),
    (0x0EED, 0x0EFC, "JOYMOUSE 輪詢值；byte_1DAAC = N 時不進按鍵表"),
    (0x9CB0, 0x9CB3, "圖庫指標與頁；繪製路徑先寫再讀"),
    (0x9CBC, 0x9CC8, "繪製參數；繪製路徑先寫再讀"),
    (0x9CE1, 0x9CE5, "數字緩衝；sub_1890D 先寫再讀"),
    (0xA7DA, 0xA7DC, "音樂驅動與 INT 8 的 word_2739B；音樂不在範圍"),
    (0xA7F7, 0xA7F8, "word_273B7，見下方計時器"),
    (0xA852, 0xA858, "舊中斷向量；啟動與離開專用"),
    (0xA92A, 0xA931, "檔名工作區；載入專用"),
    (0xA94B, 0xA94C, "檔名工作區；載入專用"),
    (0xA9AB, 0xA9AB, "檔名工作區；載入專用"),
    (0xE340, 0xFFFF, "堆疊殘值；映像外，邏輯不讀"),
    // ---- 差分模糊測試另外發現的（tests/fuzz.rs DS_EXTRA_EXEMPT）----
    (0x9C9E, 0x9CA3, "word_2685E、26860、26862 blit 暫存；只有 sub_19DA8、19E66、19F3B、19FF9、1A09F、1A144、1A1E6 讀寫"),
    (0x9CCA, 0x9CCA, "word_2688A 繪製參數；sub_1100A、1117F、162B1 等寫，sub_18993 與 sub_19A0B（blit）讀"),
    (0x9CD0, 0x9CD0, "byte_26890 數字繪製暫存；sub_1890D、18993、189C1、162B1 寫，sub_18A1F、18A61（數字繪製）讀"),
    // ---- 全流程對拍發現的（規格 002）----
    (0xAA2C, 0xAA2C, "檔名緩衝 SHT4.GO（AA25h，7 個字元加結尾）的第 8 個位元組。sub_1AEBC 經 sub_1C738（1C738h 起，加 3 後 mov [bx], dl）\
                      把它改成 'T' 以開 SHT4.GOT；只有檔案載入（sub_1C82C）讀。重製不碰檔名緩衝（檔案在啟動時已驗過）"),
    // ---- 等待計時器（規格 002）----
    (0xA7F7, 0xA7FA, "word_273B7、word_273B9 的 tick 計數。INT 8（1A7CEh、1A7D2h）加 1，只有 sub_11097（11097h、1109Dh）讀寫。\
                      原版在 1109Dh 取樣時 273B9 還沒歸零（值 >= cx），重製在 WaitDone 已歸零；輪詢迴圈裡 tick 數也不同"),
    // ---- 特效領域重寫（規格 004）：函式內先寫後讀的工作變數與參數，改成區域變數與函式參數，不再寫回資料段 ----
    // 查證：tools/q.sh refs word_20B74／20B76／20B78／20B80，另 grep 反組譯裡所有 3FB4h 至 3FC1h 的位移形式，沒有其他讀寫。
    (0x3FB4, 0x3FB5, "word_20B74 腳本指標暫存；只有 sub_17C6C 寫（17C6Ch）後讀（17C92h）"),
    (0x3FB6, 0x3FB7, "word_20B76 形狀指標；只有 sub_17CB6 寫（17D01h）、sub_17D2C 讀（17D2Ch）"),
    (0x3FB8, 0x3FB9, "word_20B78 特效字母；呼叫端（sub_10CD0、13205、13A51、13DAF、14679、147E1、151A7、1530F、15F8D、167E9、16946、173E1）寫，\
                      只有 sub_17A3A 讀（17A3Ah 起的比較）。重製改成 EffectKind 參數"),
    (0x3FC0, 0x3FC1, "word_20B80 只有 sub_17CB6 寫（17CC8h），沒有任何指令讀"),
    // ---- 敵彈領域重寫（規格 004）：函式內先寫後讀的工作變數與呼叫參數，改成區域變數與函式參數，不再寫回資料段 ----
    // 查證：tools/q.sh refs 與 dsrefs 列出每個變數的全部讀寫（結果記在 workspace/progress/rewrite-ebullet.md）。
    // 池指標表 32B3h、記錄區 32DBh 是狀態，走 Mirror，不在這裡。
    (0x2C02, 0x2C11, "word_1F7C2 起 8 個字組的轉向表；sub_1744A、1747E、174B2、174E6、1751A、1754E、17582、175B6 寫，只有 sub_175EA 讀。\
                      重製改成 TURN_TABLES 常數與 turn_toward 參數"),
    (0x2C12, 0x2C12, "byte_1F7D2 移動碼；sub_17631 寫（17634h），只有 sub_1779B 讀（177A1h）。重製改成 step_bullet 的區域變數"),
    (0x2C13, 0x2C16, "word_1F7D3、word_1F7D5 瞄準目標的欄與列；sub_172DE 寫，只有 sub_16F9F 讀（sub_171E5 的 1724Ch 起是 retn 之後無跳入的碼）。\
                      重製改成 aim_target 的回傳值"),
    (0x2C1B, 0x2C1B, "byte_1F7DB 槽號；sub_17361 歸零並每槽加 1（17367h、17394h），只有 sub_17855 讀（17888h）。重製改成 stamp_grid2 的參數"),
    (0x32AD, 0x32B2, "word_1FE6D、word_1FE6F 開火起點的欄與列：呼叫端（sub_15DBE、15E68、15FD9、1606A、1617A）寫，只有 sub_16D3B 一族（sub_16CEA、16F9F、172DE）\
                      讀寫。word_1FE71 本槽釋放旗標：sub_17631 一族（sub_176D0、176F6、1771C、17742、17955、1798E、179C7、17A00）寫，\
                      只有它們與 sub_17361 讀。重製改成 fire_enemy_bullets 的參數與 move_bullet 的回傳值"),
    (0x3533, 0x3552, "word_200F3 與工作區 3535h 起 15 個字組（word_200F5 至 word_20111）。sub_1739B 寫滿 15 字組後才由 sub_173C5 一族讀（17631、176B5、\
                      176D0、176F6、1771C、17742、17768、1779B、177D2、17821、17855、17898、173E1、17955、1798E、179C7、17A00），sub_173B0 複製回記錄；\
                      word_200F3 另是 sub_16D3B 一族寫、sub_16CEA 讀的樣板位址。重製改成 step_bullet 的區域 Bullet 與 spawn_bullet 的 Template 參數"),
    // ---- 玩家彈領域重寫（規格 004）：函式內先寫後讀的工作變數與呼叫參數，改成區域變數與函式參數，不再寫回資料段 ----
    // 查證：tools/q.sh dsrefs 列出每個變數的全部讀寫（結果記在 workspace/progress/rewrite-pbullet.md）。
    // 池指標表 2607h、記錄區 262Fh、追蹤彈計數 25E8h 是狀態，走 Mirror，不在這裡。
    // 命中計數器 word_1F1B8、1F1BA（25F8h 至 25FBh）只在不可達的碼裡寫，內容不變，所以不豁免。
    (0x25EA, 0x25F7, "word_1F1AA、1F1AC、1F1AE、1F1B0 四向格距與 word_1F1B2、1F1B4、1F1B6 掃描起點與範圍；sub_16BA8 先寫 1F1B2、1F1B4、1F1B6，\
                      sub_16BF5、16C18、16C3B、16C5C 讀它們並各寫一個格距，格距只有 sub_16BA8 讀。重製改成 distance_to_wall 的參數與回傳值"),
    (0x25FC, 0x2606, "word_1F1BC 武器字母、1F1BE 朝向、byte_1F1C0 命中格值、word_1F1C1 欄、1F1C3 列、1F1C5 釋放旗標。\
                      1F1BC、1F1BE、1F1C1、1F1C3 由呼叫端（sub_14501、1480A、1502F、15338）寫，只有 sub_16578、165CF、1669F 讀，\
                      165CF 一族就地改 1F1C1、1F1C3。1F1C0 由 sub_16B49 歸零、169BF 與雷射 16A4E、16ACA 設值，只有 sub_167A3、1691A、16946 讀。\
                      1F1C5 由 sub_16817 歸 N、移動與碰撞設 Y，只有 sub_1673E 讀。重製改成 fire_player_bullets 的參數、Impact 與回傳值"),
    (0x28AF, 0x28D0, "word_1F46F 記錄位址與 28B1h 起 16 個字組的工作區（word_1F471 至 word_1F48F）。sub_16779 把記錄複製進工作區並寫 1F46F，\
                      之後才由 sub_167A3 一族讀寫，sub_1678E 複製回 1F46F 指的記錄；sub_165CF 一族寫樣板位址進 1F46F，只有 sub_16578 讀。\
                      重製改成 update_player_bullets 的區域 PlayerBullet 與 spawn_player_bullet 的 Shot 參數"),
    // ---- 敵人領域重寫（規格 004）：函式內先寫後讀的工作變數與呼叫參數，改成區域變數與函式參數，不再寫回資料段 ----
    // 查證：tools/q.sh refs 與 dsrefs 列出每個變數的全部讀寫（結果記在 workspace/progress/rewrite-entity.md）。
    // 池指標表 2294h、記錄區 22B4h、每槽繪製值 21ECh 至 228Bh、最近繪製值 228Ch、景物池 198Ch 至 19DBh 是狀態，走 Mirror，不在這裡。
    // 死亡旗標 word_1EDAA（21EAh）看似工作變數，其實不能豁免：它的位址是 G3、G4 列起點表的越界列，格網位址函式與玩家領域
    // （尚未搬走）會從資料段直接讀，所以留在資料段，由 `Core::set_enemy_death_flag`（mirror.rs）寫。
    (0x042A, 0x042B, "word_1CFEA 亂數索引的傳遞字：sub_10E33 寫 0（10E4Eh、10E8Dh），sub_1617A 先寫（1617Dh）後讀（16183h），\
                      sub_110A4 讀寫（110A4h、110B0h）。重製改成 rolls_to_fire 的區域變數"),
    (0x1AF6, 0x1AF7, "word_1E6B6 閃白旗標：sub_15F8D 寫（15F8Dh、15FB6h），sub_15E44 讀（15E60h），sub_164F3 寫（1650Ch）後讀（1652Eh），\
                      每次使用前都在同一個敵人的處理裡寫過。重製改成 Step::flashing 與 Slot::flash"),
    (0x1B06, 0x1B09, "word_1E6C6 圖框、byte_1E6C8 移動碼、byte_1E6C9 槽號。1E6C6：sub_15F4C 寫（15F63h、15F6Bh）並讀（15F7Bh），sub_16167 讀（16173h）。\
                      1E6C8：sub_15E68（15E84h、15EB4h）與 sub_15ECD（15EE4h、15F2Dh）寫，只有 sub_15F4C 讀（15F52h）。\
                      1E6C9：sub_15D8C 歸零並每槽加 1（15D92h、15DB7h），只有 sub_16233 讀（16278h）。\
                      重製改成 select_frame 的回傳值、step_enemy 的 code 與 stamp_footprint_g1 的 slot 參數"),
    (0x25B4, 0x25E5, "word_1F174 記錄位址與 25B6h 起 24 個字組的工作區（word_1F176 至 word_1F1A4）。sub_15E1A 與 sub_15ECD 的變身（15F06h）\
                      用 rep movsw 寫滿 24 字組，之後才由 sub_15E68 一族與 sub_15DBE 讀寫，sub_15E2F 複製回 1F174 指的記錄；\
                      sub_15D1C 寫 1F174 的樣板位址，只有 sub_15D41 讀；sub_164F3、16541 另把渲染陣列的 kind 寫進 w18（1F19A），之後沒有人讀。\
                      重製改成 update_enemies 的區域 Enemy 與 spawn_enemy 的樣板參數"),
    // ---- 玩家領域重寫（規格 004）：函式內先寫後讀的工作變數與呼叫參數，改成區域變數與函式參數，不再寫回資料段 ----
    // 查證：tools/q.sh refs 與 dsrefs 列出每個變數的全部讀寫，並用 tools/q.sh callers 確認讀取端只在同一個呼叫鏈裡
    // （結果記在 workspace/progress/rewrite-player.md）。其餘玩家變數（血量、位置、圖、旗標、砲座、光柱、輸入位元、存活位元組）
    // 是狀態，走 Mirror，不在這裡。
    (0x0ECA, 0x0ECA, "byte_1DA8A 玩家 1 砲座接觸的格值：sub_14B63 先歸零（14B63h）、掃描中寫（14B9Fh），只有它呼叫的 sub_14C18 讀（14C18h）。\
                      重製改成 pod_touches_entities 的區域變數"),
    (0x0F2C, 0x0F2C, "byte_1DAEC 玩家 2 砲座接觸的格值：sub_15691 先歸零（15691h）、掃描中寫（156CDh），只有它呼叫的 sub_15746 讀（15746h）。\
                      重製改成 pod_touches_entities 的區域變數"),
    (0x0EDC, 0x0EDF, "word_1DA9C、word_1DA9E 玩家 1 砲座掃描的欄與列：sub_1480A 每次掃描前寫（14815h、1481Bh、14855h、1485Eh），\
                      只有 sub_14B63（14B78h）、sub_14BAE（14BBFh）、sub_14BF4（14C0Bh）、sub_14C3D（14C54h）讀。重製改成 pod_collide 的參數"),
    (0x0F3E, 0x0F41, "word_1DAFE、word_1DB00 玩家 2 砲座掃描的欄與列：sub_15338 每次掃描前寫（15343h、15349h、15383h、1538Ch），\
                      只有 sub_15691（156A6h）、sub_156DC（156EDh）、sub_15722（15739h）、sub_1576B（15782h）讀。重製改成 pod_collide 的參數"),
    (0x0EE8, 0x0EE8, "byte_1DAA8 玩家 1 本幀撿到的道具：sub_14C85 先設 N（14C8Ch）、sub_14CD4 寫（14CF9h），只有 sub_14C85 呼叫的 sub_14D04 讀（14D04h）。\
                      重製改成 body_touches_bullets 的區域變數與 upgrade 的參數"),
    (0x0F4A, 0x0F4A, "byte_1DB0A 玩家 2 本幀撿到的道具：sub_157B3 先設 N（157BAh）、sub_15802 寫（15827h），只有 sub_157B3 呼叫的 sub_15832 讀（15832h）。\
                      重製改成 body_touches_bullets 的區域變數與 upgrade 的參數"),
    // ---- 標題與片尾領域重寫（規格 004）：過場字樣的位置暫存，改成每張卡的區域結構 ----
    // 查證：tools/q.sh refs word_1D148 至 word_1D156（十個符號）列出全部讀寫，另 grep 反組譯裡所有 580h 至 597h 的位移形式，
    // 除了 word_1D140 至 word_1D146（走 Mirror）沒有其他存取，也沒有 [bx+位移] 的表格式存取。
    // 選單的列、難度、角色（580h 至 585h）與示範輪播（586h）是狀態，走 Mirror，不在這裡。
    (0x0588, 0x0597, "word_1D148 至 word_1D156 過場與開場的字樣位置。每個寫入它們的函式都先寫初值再讀：sub_117BF 寫 1D148、1D14A（117BFh、117C5h）後讀；\
                      sub_11C6B 寫 1D148、1D152、1D14C（11C71h、11C77h、11C7Dh）與 1D14E（11CD0h）；sub_11E46 寫 1D148 至 1D14E、1D156（11E4Ch 至 11E64h）\
                      與 1D152、1D154（11EB7h、11EE2h）；sub_121DD 寫 1D150 至 1D156（121E0h 至 121F2h）；sub_12379 寫 1D148 至 1D14E（1237Ch 至 1238Eh）。\
                      讀者只有各自卡片的步驟函式（sub_11CFC、11D58、11DAC、11F58、11FB3、1200A、12062、120B6、12132、1221E、123BD）。\
                      重製改成 cards.rs 的 Stage1Letters 至 Stage4Letters 與 intro.rs 的區域變數"),
    // ---- 流程與關卡領域重寫（規格 004）：函式內先寫後讀的工作變數，改成區域變數與函式參數，不再寫回資料段 ----
    // 查證：tools/q.sh refs 列出每個變數的全部讀寫（結果記在 workspace/progress/rewrite-flow.md）。
    // 各關的背景位置、頭目合成圖的三層位置、流程旗標、輸入與計時是狀態，走 Mirror，不在這裡。
    (0x0F9A, 0x0F9E, "byte_1DB5A 腳本位元組、word_1DB5B 與 word_1DB5D 頭目合成圖的位置：第一、二、三關的 sub_15930、15983、159D6、15A24、15A72 \
                      先從該層的位置（1DB5F 一族）複製進來才呼叫 sub_15BB0（直譯器）與 sub_15AC5、15AF4、15B23、15B52、15B81（繪製），\
                      byte_1DB5A 只在 sub_15930、15983、15A72 呼叫 sub_15BB0 之後立刻讀。重製改成 BossLayer::step 直接更新那一層"),
    (0x0FAB, 0x0FAC, "word_1DB6B 腳本讀取位置：同上，每次由 sub_15930、15983、159D6、15A24、15A72 複製進來，sub_15BB0 就地前進後複製回去。\
                      重製改成 BossLayer::script"),
    (0x900A, 0x900B, "word_25BCA 第三關地圖的索引游標：sub_17DF9 開頭先寫（17E13h）、迴圈內讀寫（17E3Ah、17E4Eh、17E5Eh），sub_182CD 同樣先寫（182F4h）\
                      再讀寫（18310h、18325h）。其他寫入者都不在 E 模式的可達路徑：sub_17DB2 的 17DBEh 起（`17DBDh` retn 之後）、\
                      sub_1840E 的 18430h 起（retn 之後）、單色分支 sub_185AA、sub_18613、sub_1812E、sub_181E1。\
                      重製改成 draw_initial_map 與 scroll_map_second_page 的區域變數"),
];

/// `off` 是否落在 [`DS_EXEMPT`]。
pub fn ds_exempt(off: u16) -> bool {
    DS_EXEMPT.iter().any(|&(lo, hi, _)| lo <= off && off <= hi)
}

/// 正式遊戲的開機資料段：原版在 `sub_11556` 入口（標題迴圈）的狀態，減去啟動選單、密碼與 DOS 記憶體配置的影響。
/// 原版從 `sub_10CD0` 進來：`10CD0h` 至 `10D11h` 的關卡初始化（`Core::init_stage1`），第一輪 `10D29h` 的 `sub_19A37`，
/// 然後 `sub_10FEA`：`word_1B02` 減 1（10FEAh 至 10FF2h），`byte_1CFAC` 仍是 `N` 才呼叫 `sub_11556`。
/// 所以標題入口的資料段是映像加上這些初始化的寫入。標題之前沒有寫過 `byte_1CFDC`、`byte_1CFDD`、`byte_1CFAC`、
/// `byte_1CFAB`（`Core::new` 為了直接進第一關而設），這四個維持映像值。
/// 差異只在 [`DS_EXEMPT`]（B1，`rs2-trace` 的 `tests/wait.rs`）。展示錄影 `DM1.LCK` 不在這裡：映像只有程式，
/// 資料由 `GameFiles` 提供，標題移植在示範開始前自己載入（`sub_1BED5`）。
pub fn boot_ds(image: &[u8]) -> Dseg {
    boot_ds_with_dm1(image, &[])
}

/// [`boot_ds`] 加上 `sub_10CD0` 在 `10CF9h` 用 `sub_1BED5` 載入的 `DM1.LCK`（`dseg:AAE2h` 起 3000 位元組）。
/// 另外三個值是啟動時最後按的鍵留下的：密碼畫面最後一個 Enter，`byte_27422`（`A862h`）= 0Dh、按鍵表 Enter（`A8E7h`）= 2
/// 已放開；啟動選單按過的 V（`A8FAh`）= 2。`sub_11556` 在 `1157Dh` 先把 `byte_27422` 清 0 才讀，所以這三個值不影響標題，
/// 但全流程對拍逐位元組比，補上它們才不必把 `A862h` 這種標題實際會讀的位元組列進 [`DS_EXEMPT`]。
pub fn boot_ds_with_dm1(image: &[u8], dm1: &[u8]) -> Dseg {
    let base = Dseg::from_image(image);
    let mut core = Core::new(image, dm1, Start::NewGame { difficulty: None }, Fixes::NONE);
    for o in [0x041Cu16, 0x03EC, 0x03EB] {
        core.ds.w8(o, base.r8(o));
    }
    // 存活位元組（041Dh，原版 byte_1CFDD）在玩家領域的結構裡。
    core.set_players_present(base.r8(0x041D));
    core.show_first_page();
    core.count_down_tamper_timer();
    core.ds.w8(0xA862, 0x0D);
    core.ds.w8(0xA8E7, 2);
    core.ds.w8(0xA8FA, 2);
    // byte_1D3A2：密碼檢查通過時寫 Y，映像是 N。第二關 130AFh 不是 Y 就停在無限迴圈。
    core.ds.w8(0x07E2, b'Y');
    core.store_mirrors();
    core.ds
}

const POOLS: usize = 9;

/// 對拍的取樣點：各關一幀的取樣位置，值是原版的程式位址（收據的 `pt` 欄用它對照，不能改）。
pub(crate) mod sample {
    /// 第一關普通段與頭目段的 `sub_11097` 之前（`10F98h`、`10FE3h`）。
    pub(crate) const STAGE1_NORMAL: u32 = 0x10F98;
    pub(crate) const STAGE1_BOSS: u32 = 0x10FE3;
    /// 第二關第一段與第二段的幀計數加 1 之後（`1333Ch`、`13390h`）。
    pub(crate) const STAGE2_OPENING: u32 = 0x1333C;
    pub(crate) const STAGE2_BOSS: u32 = 0x13390;
    /// 第三關（`13B5Fh`、`13BBBh`）與第四關（`13EF4h`、`13F42h`）。
    pub(crate) const STAGE3_NORMAL: u32 = 0x13B5F;
    pub(crate) const STAGE3_BOSS: u32 = 0x13BBB;
    pub(crate) const STAGE4_NORMAL: u32 = 0x13EF4;
    pub(crate) const STAGE4_BOSS: u32 = 0x13F42;
}

/// 協程交回控制權的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Req {
    /// 取樣點，對拍工具在這裡讀狀態，不結束這一步。
    Sample(u32),
    /// `sub_11097` 的等待，`ticks` 個 18.2 Hz 的 tick。一步在這裡結束。
    Wait { ticks: u16 },
    /// 等待結束、這段期間的掃描碼處理完，相當於原版 `1109Dh`。對拍工具在這裡比對。
    WaitDone { cx: u16 },
    /// 原版的輪詢迴圈（等按鍵等），每個 tick 檢查一次。一步在這裡結束。
    Poll { paused: bool },
    /// 一輪結束，`settle` 停在這裡。不結束這一步。
    Checkpoint,
}

/// 協程與外面交換資料的地方。
pub(crate) struct Port {
    req: Option<Req>,
    input: Vec<u8>,
    pokes: Vec<(u16, u16, bool)>,
    ds: Dseg,
    draws: Vec<DrawCmd>,
    /// 下一次恢復會走到一輪結尾的檢查（第二頁的等待之後）。
    pub(crate) at_round_end: bool,
    integrity_checked: bool,
    display_start: Option<u16>,
    audio: Vec<AudioCmd>,
    /// 第三關進場時讀 `SHT.SCR`（單色，C、H）而不是 `SHT.SCN`（E）。不屬於原版狀態，由前端在進第三關之前設定。
    pub(crate) terrain_scr: bool,
    /// 強化選項，見 [`Rules`]。
    pub(crate) rules: Rules,
    /// 砲座分離鍵按著（規格 005「砲座像 Force」）。沒有原版掃描碼，不走按鍵表。
    pub(crate) split_held: [bool; 2],
}

struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

/// 模擬狀態。逐函式移植的程式都掛在這裡，只有協程摸得到。
pub struct Core {
    /// 還沒搬走的領域的狀態。已搬走的領域住在下面有型別的欄位，這裡對應的位元組只在
    /// `store_mirrors`（交回控制權）與 `load_mirrors`（載入、poke）時同步，見 `mirror.rs`。
    pub ds: Dseg,
    pub r: Regs,
    pub fixes: Fixes,
    /// 特效池。
    pub(crate) fx: Effects,
    /// 敵方子彈池。
    pub(crate) eb: EnemyBullets,
    /// 玩家子彈池。
    pub(crate) pb: PlayerBullets,
    /// 敵人池。
    pub(crate) en: Enemies,
    /// 第一關的視差景物。
    pub(crate) sc: Scenery,
    /// 兩位玩家與存活位元組。
    pub(crate) pl: Players,
    /// 標題：選單與示範輪播。
    pub(crate) title: Title,
    /// 關卡流程：關卡字母、過關旗標、計時、輸入。
    pub(crate) flow: Flow,
    /// 各關背景的狀態。
    pub(crate) scenes: Scenes,
    /// 完整性檢查（原版 `sub_11375`）通過後把自己改成 `retn`，之後不再檢查。
    pub(crate) integrity_checked: bool,
    /// CRTC 顯示起點，只有 `sub_19A37`、`sub_19A88` 會寫。第一次翻頁之前不知道。
    pub(crate) display_start: Option<u16>,
    /// 音樂緩衝區裡的曲目。
    pub(crate) mix: MixFile,
    pub(crate) audio: Vec<AudioCmd>,
    pub(crate) sht_group: ShtGroup,
    pub(crate) mask_group: ShtGroup,
    pub(crate) beg_group: BegGroup,
    pub(crate) files: Rc<GameFiles>,
    /// 第三關地圖 `SHT.SCN` 解壓後的內容，對應原版遠段 `word_1CF6E`。`sub_13A51` 載入，之後唯讀。
    pub(crate) map_seg: Vec<u8>,
    pub(crate) draws: Vec<DrawCmd>,
    pub(crate) cur_obj: Option<ObjId>,
    pub(crate) gens: [[u32; 40]; POOLS],
    /// `bullet_slot_carry` 開啟時，G2 每格真正的敵彈槽號加 1，0 表示沒有。不屬於原版狀態。
    pub(crate) g2_slots: Vec<u8>,
    pub(crate) port: Rc<RefCell<Port>>,
}

impl Core {
    pub(crate) fn new(image: &[u8], dm1: &[u8], start: Start, fixes: Fixes) -> Core {
        let ds = Dseg::from_image(image);
        let port = Port {
            req: None,
            input: Vec::new(),
            pokes: Vec::new(),
            ds: ds.clone(),
            draws: Vec::new(),
            at_round_end: false,
            integrity_checked: false,
            display_start: None,
            audio: Vec::new(),
            terrain_scr: false,
            rules: Rules::ORIGINAL,
            split_held: [false; 2],
        };
        let mut g = Core {
            ds,
            r: Regs::default(),
            fixes,
            fx: Effects::default(),
            eb: EnemyBullets::default(),
            pb: PlayerBullets::default(),
            en: Enemies::default(),
            sc: Scenery::default(),
            pl: Players::default(),
            title: Title::default(),
            flow: Flow::default(),
            scenes: Scenes::default(),
            integrity_checked: false,
            display_start: None,
            mix: MixFile::A,
            audio: Vec::new(),
            sht_group: ShtGroup::Bh1,
            mask_group: ShtGroup::Sht1,
            beg_group: BegGroup::Beg1,
            files: Rc::new(GameFiles { dm1: dm1.to_vec(), ..GameFiles::default() }),
            map_seg: Vec::new(),
            draws: Vec::new(),
            cur_obj: None,
            gens: [[0; 40]; POOLS],
            g2_slots: vec![0; 0x321],
            port: Rc::new(RefCell::new(port)),
        };
        g.load_mirrors();
        // 兩個入口都載入展示錄影（10CF9h 的 sub_1BED5）。
        for (i, &b) in dm1.iter().enumerate().take(3000) {
            g.ds.w8(0xAAE2u16.wrapping_add(i as u16), b);
        }
        // 標題流程設定的值。
        let demo = matches!(start, Start::Demo);
        g.ds.w8(0x041C, if demo { b'Y' } else { b'N' });
        g.set_players_present(0x01);
        g.ds.w8(0x03EC, b'Y');
        g.ds.w8(0x03EB, b'Y');
        if let Start::NewGame { difficulty: Some(d) } = start {
            let (hp, fire) = match d {
                0 => (0, 0),
                1 => (0, 2),
                2 => (1, 2),
                _ => (1, 3),
            };
            g.ds.w16(a(0x1E6B2), hp);
            g.ds.w16(a(0x1E6B4), fire);
        }
        g.init_stage1();
        g
    }

    /// 把控制權交回外面，恢復時處理這段期間送達的掃描碼。
    pub(crate) async fn suspend(&mut self, req: Req) {
        // 已搬走的領域先寫回 `self.ds`，外面看到的資料段才是完整的，之後的 poke 也套在完整的資料段上。
        self.store_mirrors();
        {
            let mut p = self.port.borrow_mut();
            p.req = Some(req);
            p.ds.copy_from(&self.ds);
            p.draws.append(&mut self.draws);
            p.integrity_checked = self.integrity_checked;
            p.display_start = self.display_start;
            p.audio.append(&mut self.audio);
        }
        YieldOnce(false).await;
        let (input, pokes) = {
            let mut p = self.port.borrow_mut();
            (std::mem::take(&mut p.input), std::mem::take(&mut p.pokes))
        };
        let poked = !pokes.is_empty();
        for (o, v, word) in pokes {
            if word {
                self.ds.w16(o, v);
            } else {
                self.ds.w8(o, v as u8);
            }
        }
        if poked {
            // poke 可能落在已搬走的領域，讓結構看到新值。
            self.load_mirrors();
        }
        for sc in input {
            self.handle_scancode(sc);
        }
    }

    /// 繪製迴圈裡標記目前的物件，之後的 blit 帶這個編號。
    pub(crate) fn set_obj(&mut self, pool: Pool, slot: u8) {
        let generation = self.gens[pool as usize][slot as usize];
        self.cur_obj = Some(ObjId { pool, slot, generation });
    }

    pub(crate) fn clear_obj(&mut self) {
        self.cur_obj = None;
    }

    /// 生成函式配置槽時呼叫。
    pub(crate) fn bump_gen(&mut self, pool: Pool, slot: u8) {
        let g = &mut self.gens[pool as usize][slot as usize];
        *g = g.wrapping_add(1);
    }
}

/// 對外的遊戲物件：一個跑在協程裡的 `Core`。
/// HD 介面要顯示的玩家狀態，見 `Game::hud_status`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HudStatus {
    /// 在場上。
    pub active: bool,
    /// 血量，FFh 起每次命中右移一位，所以亮的位元數就是剩下的格數。
    pub hp: u16,
    /// 武器字母：0、`L`、`B`、`K`、`S`。
    pub weapon: u16,
    /// 已裝上的砲座數，0 到 2。
    pub pods: u8,
    /// 充能計數。
    pub charge: u16,
}

/// 玩家機的狀態，見 `Game::player_status`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerStatus {
    /// 左緣，單位是位元組（8 像素）。
    pub x: i16,
    /// 上緣，單位是列。
    pub y: i16,
    /// 玩家的按鍵會移動它：在場上、血量不是 0、不是示範、玩家邏輯有在跑。
    pub controllable: bool,
    pub can_up: bool,
    pub can_down: bool,
    pub can_left: bool,
    pub can_right: bool,
}

pub struct Game {
    /// 上一次交回控制權時的資料段複本。要改值用 `poke8`、`poke16`。
    pub ds: Dseg,
    fixes: Fixes,
    port: Rc<RefCell<Port>>,
    fut: Pin<Box<dyn Future<Output = Exit>>>,
    finished: Option<Exit>,
}

fn stage1(mut core: Core) -> Pin<Box<dyn Future<Output = Exit>>> {
    Box::pin(async move { core.run_stage1_rounds().await })
}

/// 跑一個入口，回傳離開碼。關卡入口直接回傳離開碼；標題與片尾還在用暫存器，離開碼留在 `dx`。
async fn run_entry(mut core: Core, entry: Entry) -> Exit {
    match entry {
        // 標題與片尾回來就是程式結束。
        Entry::Title => {
            core.run_title().await;
            Exit::Quit
        }
        Entry::Ending => {
            core.run_ending().await;
            Exit::Quit
        }
        Entry::Stage1 => core.play_stage1().await,
        Entry::Stage2 => core.play_stage2().await,
        Entry::Stage3 => core.play_stage3().await,
        Entry::Stage4 => core.play_stage4().await,
    }
}

impl Game {
    /// 從一份資料段快照開始跑。`ds` 是原版在該入口的傾印，或正式遊戲的開機狀態。
    pub fn from_snapshot(files: GameFiles, ds_snapshot: &[u8], entry: Entry, fixes: Fixes) -> Game {
        let files = Rc::new(files);
        let mut core = Core::new(&files.image, &[], Start::NewGame { difficulty: None }, fixes);
        core.draws.clear();
        core.ds.load(ds_snapshot);
        core.load_mirrors();
        core.files = files;
        // sub_11375 通過時把 dl（`Y`）寫進 byte_1CFAA，並把自己改成 retn。快照要帶上這個狀態。
        core.integrity_checked = core.flow.integrity_mark != b'N';
        let ds = core.ds.clone();
        let port = core.port.clone();
        port.borrow_mut().ds.copy_from(&ds);
        Game { ds, fixes, port, fut: Box::pin(run_entry(core, entry)), finished: None }
    }

    pub fn new(image: &[u8], dm1: &[u8], start: Start, fixes: Fixes) -> Game {
        let mut core = Core::new(image, dm1, start, fixes);
        core.store_mirrors();
        let ds = core.ds.clone();
        let port = core.port.clone();
        port.borrow_mut().draws.append(&mut core.draws);
        Game { ds, fixes, port, fut: stage1(core), finished: None }
    }

    /// 重新開始：沿用同一份 `Dseg`，只重跑進入關卡的初始化，殘值行為與原版相同。
    pub fn restart(&mut self, start: Start) {
        let checked = self.port.borrow().integrity_checked;
        let image_free = self.ds.clone();
        let mut core = Core::new(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET], &[], start, self.fixes);
        core.ds = image_free;
        core.load_mirrors();
        core.integrity_checked = checked;
        core.ds.w8(0x041C, if matches!(start, Start::Demo) { b'Y' } else { b'N' });
        core.set_players_present(0x01);
        core.draws.clear();
        core.init_stage1();
        core.store_mirrors();
        self.ds.copy_from(&core.ds);
        let port = core.port.clone();
        port.borrow_mut().draws.append(&mut core.draws);
        self.port = port;
        self.fut = stage1(core);
        self.finished = None;
    }

    /// 這一步之前累積的繪製命令（進入關卡時的初始化命令也在這裡）。
    fn take_audio(&mut self) -> Vec<AudioCmd> {
        std::mem::take(&mut self.port.borrow_mut().audio)
    }

    pub fn take_draws(&mut self) -> Vec<DrawCmd> {
        std::mem::take(&mut self.port.borrow_mut().draws)
    }

    /// 在下一次恢復時寫入資料段，測試用。
    pub fn poke8(&mut self, offset: u16, value: u8) {
        self.port.borrow_mut().pokes.push((offset, value as u16, false));
    }

    pub fn poke16(&mut self, offset: u16, value: u16) {
        self.port.borrow_mut().pokes.push((offset, value, true));
    }

    /// 第三關的地形資料來源（`docs/spec/006-display-modes.md`「第三關地形」）：原版 E（VGA、EGA）讀 `SHT.SCN`，
    /// 單色（CGA、MGA）讀 `SHT.SCR`，兩份在 514 格的零與非零不同，而這份陣列就是碰撞格 G4。
    /// 第三關進場（`sub_13A51`）時讀取，所以進場之後再設不生效：一局裡每個第三關各用進場當下的選擇。
    pub fn set_terrain_scr(&mut self, on: bool) {
        self.port.borrow_mut().terrain_scr = on;
    }

    /// 強化選項（規格 005）。開著時立刻生效，每次變動呼叫一次。全關時模擬與原版逐位元組相同。
    pub fn set_rules(&mut self, rules: Rules) {
        self.port.borrow_mut().rules = rules;
    }

    /// 砲座分離鍵按著（規格 005「砲座像 Force」）。`player` 是 0 或 1，其他值忽略。
    pub fn set_split_held(&mut self, player: usize, held: bool) {
        if let Some(h) = self.port.borrow_mut().split_held.get_mut(player) {
            *h = held;
        }
    }

    fn poll_once(&mut self) -> Poll<Exit> {
        let mut cx = Context::from_waker(Waker::noop());
        let r = self.fut.as_mut().poll(&mut cx);
        self.ds.copy_from(&self.port.borrow().ds);
        r
    }

    /// 跑到下一個等待點。掃描碼在協程恢復時才處理，所以和原版一樣落在上一幀的等待裡。
    pub fn step(&mut self, input: &Input, on_sample: &mut dyn FnMut(&Game, u32)) -> Frame {
        self.step_full(input, on_sample, &mut |_g: &Game, _cx: u16| {})
    }

    /// 同 `step`，另外在每次等待結束（對應原版 `1109Dh`）呼叫 `on_wait`，給全流程對拍用。
    pub fn step_full(
        &mut self,
        input: &Input,
        on_sample: &mut dyn FnMut(&Game, u32),
        on_wait: &mut dyn FnMut(&Game, u16),
    ) -> Frame {
        if let Some(e) = self.finished {
            return Frame { draws: Vec::new(), audio: Vec::new(), exit: Some(e), paused: false, ticks: 0 };
        }
        self.port.borrow_mut().input.extend_from_slice(input.scancodes);
        loop {
            match self.poll_once() {
                Poll::Ready(e) => {
                    self.finished = Some(e);
                    return Frame { draws: self.take_draws(), audio: self.take_audio(), exit: Some(e), paused: false, ticks: 0 };
                }
                Poll::Pending => {
                    let req = self.port.borrow_mut().req.take();
                    match req {
                        Some(Req::Sample(pt)) => on_sample(self, pt),
                        Some(Req::WaitDone { cx }) => on_wait(self, cx),
                        Some(Req::Checkpoint) => {}
                        Some(Req::Wait { ticks }) => {
                            return Frame { draws: self.take_draws(), audio: self.take_audio(), exit: None, paused: false, ticks };
                        }
                        Some(Req::Poll { paused }) => {
                            return Frame { draws: self.take_draws(), audio: self.take_audio(), exit: None, paused, ticks: 1 };
                        }
                        None => unreachable!("協程暫停時一定留下原因"),
                    }
                }
            }
        }
    }

    /// 原版這時的 CRTC 顯示起點（0 或 8000h）。第一次翻頁之前是 `None`。
    pub fn display_start(&self) -> Option<u16> {
        self.port.borrow().display_start
    }

    pub fn state_hash_input(&self) -> &[u8] {
        self.ds.bytes()
    }

    /// HD 介面要顯示的玩家狀態。`player` 是 0 或 1。只讀，不影響模擬。
    pub fn hud_status(&self, player: usize) -> HudStatus {
        let players = Players::from_dseg(&self.ds);
        let p = &players.ship[player];
        HudStatus { active: players.is_alive(player), hp: p.hp, weapon: p.weapon, pods: p.pods.installed(), charge: p.charge }
    }

    /// 無敵選項（規格 005）：沿用原版秘技的旗標，`word_273A5` 是 `Y` 時受傷不減血（`1451Ch`、`1504Ah`，
    /// 玩家領域的 `take_hit` 讀它）。
    /// 原版的 Alt 鍵會把旗標清成 `N`，所以開著的時候呼叫端每步都要呼叫一次。從沒開過就不寫，模擬和原版相同。
    pub fn set_invincible(&mut self, on: bool) {
        self.poke16(a(0x273A5), if on { 0x59 } else { 0x4E });
    }

    /// 玩家機的狀態，給前端的顯示層預測用。`player` 是 0 或 1。
    ///
    /// 啟用位元與界限來自 `sub_14501`、`sub_1502F` 與各方向的移動函式，位置與列欄是它們更新的變數。
    /// 界限用玩家領域移動函式的同一組條件（`Player::can_move_up` 一族）。
    pub fn player_status(&self, player: usize) -> PlayerStatus {
        let players = Players::from_dseg(&self.ds);
        let p = &players.ship[player];
        PlayerStatus {
            x: p.blit_x as i16,
            y: p.blit_y as i16,
            // 玩家位元：還在場上。血量 0：死亡動畫中，`die_step` 自己往下往右走，不接受輸入。
            // `byte_1CFDC` 是 `Y`：示範，方向來自錄影。`byte_1CFAD` 是 `F`：不跑玩家邏輯。
            controllable: players.is_alive(player) && p.hp != 0 && !demo_playing(&self.ds) && !stage_cleared(&self.ds),
            can_up: p.can_move_up(),
            can_down: p.can_move_down(),
            can_left: p.can_move_left(),
            can_right: p.can_move_right(),
        }
    }

    /// 下一次恢復會走到一輪結尾的檢查（Esc 暫停、熱鍵）。
    pub fn at_round_end(&self) -> bool {
        self.port.borrow().at_round_end
    }

    /// 對拍用：取樣之後的等待如果接著一輪結尾的檢查，就走到那裡，回傳離開碼。第一頁的取樣之後不動。
    pub fn settle(&mut self) -> Option<Exit> {
        if self.finished.is_some() {
            return self.finished;
        }
        if !self.port.borrow().at_round_end {
            return None;
        }
        loop {
            match self.poll_once() {
                Poll::Ready(e) => {
                    self.finished = Some(e);
                    return Some(e);
                }
                Poll::Pending => {
                    let req = self.port.borrow_mut().req.take();
                    match req {
                        Some(Req::WaitDone { .. }) => {}
                        Some(Req::Checkpoint) => return None,
                        Some(Req::Poll { .. }) => return None,
                        other => unreachable!("一輪結尾前不該有 {other:?}"),
                    }
                }
            }
        }
    }
}
