//! 核心狀態與入口。規格見 `docs/spec/001-rust-first-stage.md`。

use crate::draw::{AudioCmd, BegGroup, DrawCmd, MixFile, ObjId, Pool, ShtGroup};
use crate::dseg::{Dseg, a};
use crate::ebullet::EnemyBullets;
use crate::fx::Effects;
use crate::regs::Regs;
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
];

/// `off` 是否落在 [`DS_EXEMPT`]。
pub fn ds_exempt(off: u16) -> bool {
    DS_EXEMPT.iter().any(|&(lo, hi, _)| lo <= off && off <= hi)
}

/// 正式遊戲的開機資料段：原版在 `sub_11556` 入口（標題迴圈）的狀態，減去啟動選單、密碼與 DOS 記憶體配置的影響。
/// 原版從 `sub_10CD0` 進來：`10CD0h` 至 `10D11h` 的關卡初始化（`enter_stage`），第一輪 `10D29h` 的 `sub_19A37`，
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
    for o in [0x041Cu16, 0x041D, 0x03EC, 0x03EB] {
        core.ds.w8(o, base.r8(o));
    }
    core.s19a37();
    let v = core.ds.r16(0x1B02);
    if v >= 0xD2 {
        core.ds.w16(0x1B02, v - 1);
    }
    core.ds.w8(0xA862, 0x0D);
    core.ds.w8(0xA8E7, 2);
    core.ds.w8(0xA8FA, 2);
    // byte_1D3A2：密碼檢查通過時寫 Y，映像是 N。第二關 130AFh 不是 Y 就停在無限迴圈。
    core.ds.w8(0x07E2, b'Y');
    core.store_mirrors();
    core.ds
}

const POOLS: usize = 9;

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
    s11375_patched: bool,
    display_start: Option<u16>,
    audio: Vec<AudioCmd>,
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
    /// `sub_11375` 通過後把自己改成 `retn`。
    pub(crate) s11375_patched: bool,
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
    /// `sub_16578` 配到槽的次數，`seeker_spawn_leak` 用。不屬於原版狀態。
    pub(crate) pb_spawned: u16,
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
            s11375_patched: false,
            display_start: None,
            audio: Vec::new(),
        };
        let mut g = Core {
            ds,
            r: Regs::default(),
            fixes,
            fx: Effects::default(),
            eb: EnemyBullets::default(),
            s11375_patched: false,
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
            pb_spawned: 0,
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
        g.ds.w8(0x041D, 1);
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
        g.enter_stage();
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
            p.s11375_patched = self.s11375_patched;
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
            self.int9(sc);
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
    Box::pin(async move { core.s10d29().await })
}

/// 跑一個入口，回傳離開碼。
async fn run_entry(mut core: Core, entry: Entry) -> Exit {
    match entry {
        Entry::Title => core.s11556().await,
        Entry::Stage1 => core.s10cd0().await,
        Entry::Stage2 => core.s13205().await,
        Entry::Stage3 => core.s13a51().await,
        Entry::Stage4 => core.s13daf().await,
        Entry::Ending => core.s141d0().await,
    }
    match core.r.dx as u8 {
        b'E' => Exit::End,
        b'C' => Exit::Clear,
        _ => Exit::Quit,
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
        core.s11375_patched = core.ds.r8(a(0x1CFAA)) != b'N';
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
        let patched = self.port.borrow().s11375_patched;
        let image_free = self.ds.clone();
        let mut core = Core::new(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET], &[], start, self.fixes);
        core.ds = image_free;
        core.load_mirrors();
        core.s11375_patched = patched;
        core.ds.w8(0x041C, if matches!(start, Start::Demo) { b'Y' } else { b'N' });
        core.ds.w8(0x041D, 1);
        core.draws.clear();
        core.enter_stage();
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

    /// 無敵選項（規格 005）：沿用原版秘技的旗標，`word_273A5` 是 `Y` 時受傷不減血（`1451Ch`、`1504Ah`）。
    /// 原版的 Alt 鍵會把旗標清成 `N`，所以開著的時候呼叫端每步都要呼叫一次。從沒開過就不寫，模擬和原版相同。
    pub fn set_invincible(&mut self, on: bool) {
        self.poke16(a(0x273A5), if on { 0x59 } else { 0x4E });
    }

    /// 玩家機的狀態，給前端的顯示層預測用。`player` 是 0 或 1。
    ///
    /// 啟用位元與界限來自 `sub_14501`、`sub_1502F` 與各方向的移動函式，位置與列欄是它們更新的變數。
    pub fn player_status(&self, player: usize) -> PlayerStatus {
        let r16 = |l: u32| self.ds.r16(a(l));
        let r8 = |l: u32| self.ds.r8(a(l));
        let (bit, y, x, row, col, hp) = if player == 0 {
            (0x01, 0x1DAC5, 0x1DAC7, 0x1DAC9, 0x1DACB, 0x1CFDE)
        } else {
            (0x10, 0x1DB1F, 0x1DB21, 0x1DB37, 0x1DB39, 0x1CFE0)
        };
        let (row, col) = (r16(row), r16(col));
        PlayerStatus {
            x: r16(x) as i16,
            y: r16(y) as i16,
            // `byte_1CFDD` 的玩家位元：還在場上。血量 0：死亡動畫中，`sub_14679` 自己往下往右走，不接受輸入。
            // `byte_1CFDC` 是 `Y`：示範，方向來自錄影。`byte_1CFAD` 是 `F`：不跑玩家邏輯。
            controllable: r8(0x1CFDD) & bit != 0 && r16(hp) != 0 && r8(0x1CFDC) != b'Y' && r8(0x1CFAD) != b'F',
            can_up: (row as i16) > 0,
            can_down: row < 0x10,
            can_left: (col as i16) > 1,
            can_right: col < 0x21,
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
