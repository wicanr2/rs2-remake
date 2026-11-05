//! 核心狀態與入口。規格見 `docs/spec/001-rust-first-stage.md`。

use crate::ai::AiState;
use crate::draw::{AudioCmd, DrawCmd, ObjId, Pool};
use crate::ebullet::EnemyBullets;
use crate::entity::Enemies;
use crate::flow::{Cheat, Controls, DosBook, FireFlags, Flow, Palette, Scenes};
use crate::fx::Effects;
use crate::grid::Grids;
use crate::loader::{Banks, DemoFile, DemoTape};
use crate::pbullet::PlayerBullets;
use crate::player::{Enhanced, Player, Players, Pods, ShootView};
use crate::scenery::Scenery;
use crate::sound::Sound;
use crate::tables::Tables;
use crate::title::Title;
use crate::video::{DrawParams, Page};
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

/// 戰場大小（規格 005「戰場大小」）：水平方向的欄數，一欄 8 像素，垂直維持原版的 19 列遊戲區。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FieldSize {
    /// 100%：320×190，40 欄，原版。
    #[default]
    Normal,
    /// 120%：384×190，48 欄。
    Wide,
    /// 150%：480×190，60 欄。
    Wider,
}

impl FieldSize {
    /// 欄數。
    pub const fn cols(self) -> u16 {
        match self {
            FieldSize::Normal => 40,
            FieldSize::Wide => 48,
            FieldSize::Wider => 60,
        }
    }

    /// 依 `options.rs` 的選項值（0、1、2）；其他值當 100%。
    pub const fn from_index(i: u8) -> FieldSize {
        match i {
            1 => FieldSize::Wide,
            2 => FieldSize::Wider,
            _ => FieldSize::Normal,
        }
    }
}

/// 改變規則的強化選項（規格 005）。全關等於原版，對拍永遠全關。不屬於原版狀態，不進 `State`。
///
/// `field` 是進關時才讀的選項：關內改它不生效，下一次進關才取新值（規格 005「戰場大小」）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    pub field: FieldSize,
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
        field: FieldSize::Normal,
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

/// 遊戲的全部狀態，依領域分組。每個欄位都是有型別的結構，欄位名是遊戲語意，沒有原版位移。
///
/// 對拍與快照靠它：[`Game::state`] 是模擬在上一次交回控制權時的狀態，[`Game::state_mut`] 與 [`Game::from_state`]
/// 是寫回的入口。原版的資料段版面在 `rs2-trace` 的 `Mirror`，遊戲不認識它。
/// 不屬於原版的狀態（強化選項、AI、圖庫組、修正模式的槽號）留在 `Core`，不在這裡。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub fx: Effects,
    pub eb: EnemyBullets,
    pub pb: PlayerBullets,
    pub en: Enemies,
    pub sc: Scenery,
    pub pl: Players,
    pub title: Title,
    pub flow: Flow,
    pub scenes: Scenes,
    pub draw_params: DrawParams,
    pub sound: Sound,
    pub grids: Grids,
    pub tape: DemoTape,
}

impl State {
    /// 程式還沒執行時的狀態：原版映像裡各變數的初值。`rs2-trace` 的測試逐欄對過映像。
    ///
    /// 唯一的例外是 G4 地形格：映像裡是 800 個 `0002h`，每個關卡進場（`reset_stage`）第一件事就清掉它，
    /// 清之前沒有人讀，所以這裡是 0。第二關的淡出工作表一開始是第二關調色盤的複本，取自表格。
    pub fn power_on(tables: &Tables) -> State {
        const NO: u8 = b'N';
        const YES: u8 = b'Y';
        let mut s = State::default();
        s.en.death_flag = u16::from(YES);
        s.en.stray_damage = 0x2300;
        let ship = Player {
            hp: 0xFF,
            col: 20,
            row: 10,
            blit_x: 20,
            blit_y: 100,
            hit: NO,
            flame: NO,
            fired: NO,
            beam_y: 2,
            pods: Pods { first: NO, second: NO, facing: u16::from(b'<'), image: 30, ..Pods::default() },
            ..Player::default()
        };
        s.pl.ship = [ship; 2];
        s.title.menu.level = 1;
        s.flow = Flow {
            stage: b'A',
            clear: NO,
            integrity_mark: NO,
            demo: YES,
            from_title: NO,
            title_entered: NO,
            password_accepted: NO,
            disk_hint: b'1',
            dos: DosBook {
                message_a: *b" 0 ",
                message_b: *b" 0 ",
                segments: [0x5A00, 0x6000, 0x6800, 0x7000, 0x7800, 0x3000, 0x3800, 0x4000, 0x4800, 0x4800],
            },
            controls: Controls {
                fire: FireFlags { new_press: [YES; 2], event: [YES; 2], turn: [NO; 2] },
                cheat: Cheat { unlocked: u16::from(NO), invincible: u16::from(NO), ..Cheat::default() },
                joymouse: NO,
                sound_key: NO,
                ..Controls::default()
            },
            ..Flow::default()
        };
        s.scenes.stage2.flame_bottom = 156;
        s.scenes.stage2.strip_a.phase = 7;
        let fade = tables.palette(Palette::Stage2Fade).dac;
        s.scenes.stage2.fade = std::array::from_fn(|k| fade[k]);
        s.scenes.tunnel.ceiling_phase = 7;
        s.draw_params.shape_width = Some(0);
        s.sound.music = NO;
        s
    }
}

/// 正式遊戲的開機狀態：原版在 `sub_11556` 入口（標題迴圈）的狀態，減去啟動選單、密碼與 DOS 記憶體配置的影響。
/// 原版從 `sub_10CD0` 進來：`10CD0h` 至 `10D11h` 的關卡初始化（`Core::init_stage1`），第一輪 `10D29h` 的 `sub_19A37`，
/// 然後 `sub_10FEA`：`word_1B02` 減 1（10FEAh 至 10FF2h），`byte_1CFAC` 仍是 `N` 才呼叫 `sub_11556`。
/// 所以標題入口的狀態是映像加上這些初始化的寫入。標題之前沒有寫過 `byte_1CFDC`、`byte_1CFDD`、`byte_1CFAC`、
/// `byte_1CFAB`（`Core::new` 為了直接進第一關而設），這四個維持映像值。
/// 差異只在 `rs2-trace` 的 `DS_EXEMPT`（B1，`tests/wait.rs`）。展示錄影 `DM1.LCK` 在這裡載入：標題移植在示範開始前
/// 自己載入（`sub_1BED5`），但 `sub_10CD0` 在 `10CF9h` 已載過一次。
///
/// 另外三個值是啟動時最後按的鍵留下的：密碼畫面最後一個 Enter，`byte_27422` = 0Dh、按鍵表 Enter = 2
/// 已放開；啟動選單按過的 V = 2。`sub_11556` 在 `1157Dh` 先把 `byte_27422` 清 0 才讀，所以這三個值不影響標題，
/// 但全流程對拍逐位元組比，補上它們才不必把標題實際會讀的位元組列進豁免。
pub fn boot_state(tables: Rc<Tables>, dm1: &[u8]) -> State {
    let base = State::power_on(&tables);
    let mut core = Core::new(tables, dm1, Start::NewGame { difficulty: None }, Fixes::NONE);
    core.flow.demo = base.flow.demo;
    core.flow.title_entered = base.flow.title_entered;
    core.flow.from_title = base.flow.from_title;
    core.set_players_present(base.pl.alive);
    core.show_page(Page::Zero);
    core.count_down_tamper_timer();
    core.flow.controls.last_key = 0x0D;
    core.flow.controls.keys.0[0x1C] = 2;
    core.flow.controls.keys.0[0x2F] = 2;
    // byte_1D3A2：密碼檢查通過時寫 Y，映像是 N。第二關 130AFh 不是 Y 就停在無限迴圈。
    core.flow.password_accepted = b'Y';
    core.snapshot()
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
    /// 外面（`Game::state_mut`）改過的狀態，協程恢復時套用。
    pending: Option<State>,
    /// 協程上一次交回控制權時的狀態。
    state: State,
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
    /// 玩家的強化狀態（`Core::enh`）在上一次交回控制權時的複本，給 `Game::player_status`、`Game::hud_status` 讀。
    /// 不屬於原版狀態，不進 `State`。
    pub(crate) shoot: [ShootView; 2],
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
    pub fixes: Fixes,
    /// 唯讀表格，啟動時從 `S.EXE` 解析。
    pub(crate) tables: Rc<Tables>,
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
    /// 玩家的強化狀態（規格 005 的射擊與操控選項）。不屬於原版狀態，不進 `State`。
    pub(crate) enh: Enhanced,
    /// 標題：選單與示範輪播。
    pub(crate) title: Title,
    /// 關卡流程：關卡字母、過關旗標、計時、輸入。
    pub(crate) flow: Flow,
    /// 各關背景的狀態。
    pub(crate) scenes: Scenes,
    /// 繪製參數：位置、來源、頁、暫存區選擇、形狀寬。下一個繪製命令帶走它們。
    pub(crate) draw_params: DrawParams,
    /// 音樂與靜音開關、喇叭參數。
    pub(crate) sound: Sound,
    /// 戰場格網。
    pub(crate) grids: Grids,
    /// 展示錄影區。
    pub(crate) tape: DemoTape,
    /// 完整性檢查（原版 `sub_11375`）通過後把自己改成 `retn`，之後不再檢查。
    pub(crate) integrity_checked: bool,
    /// CRTC 顯示起點，只有 `sub_19A37`、`sub_19A88` 會寫。第一次翻頁之前不知道。
    pub(crate) display_start: Option<u16>,
    pub(crate) audio: Vec<AudioCmd>,
    /// 目前載入的圖庫組與曲目。
    pub(crate) banks: Banks,
    pub(crate) files: Rc<GameFiles>,
    /// 第三關地圖 `SHT.SCN` 解壓後的內容，對應原版遠段 `word_1CF6E`。`sub_13A51` 載入，之後唯讀。
    pub(crate) map_seg: Vec<u8>,
    pub(crate) draws: Vec<DrawCmd>,
    pub(crate) cur_obj: Option<ObjId>,
    pub(crate) gens: [[u32; 40]; POOLS],
    /// `bullet_slot_carry` 開啟時，G2 每格真正的敵彈槽號加 1，0 表示沒有。不屬於原版狀態。
    pub(crate) g2_slots: Vec<u8>,
    /// 敵人 AI 強化的狀態（規格 005）。不屬於原版狀態，不進 `State`，全關時不影響模擬。
    pub(crate) ai: AiState,
    pub(crate) port: Rc<RefCell<Port>>,
}

impl Core {
    /// 目前的強化選項（規格 005）。每次讀 `Port`，所以 [`Game::set_rules`] 立刻生效。
    /// 標題輪播的展示錄影（`byte_1CFDC` 是 `Y`）播的是原版錄下的輸入，強化規則會讓它對不上，所以展示一律是原版規則。
    pub(crate) fn rules(&self) -> Rules {
        if self.demo_playing() { Rules::ORIGINAL } else { self.port.borrow().rules }
    }

    pub(crate) fn new(tables: Rc<Tables>, dm1: &[u8], start: Start, fixes: Fixes) -> Core {
        let initial = State::power_on(&tables);
        Core::from_initial(tables, initial, dm1, start, fixes)
    }

    /// `initial` 是程式還沒執行時的狀態：正式遊戲用 [`State::power_on`]，單元測試用全 0 的狀態。
    fn from_initial(tables: Rc<Tables>, initial: State, dm1: &[u8], start: Start, fixes: Fixes) -> Core {
        let port = Port {
            req: None,
            input: Vec::new(),
            pending: None,
            state: State::default(),
            draws: Vec::new(),
            at_round_end: false,
            integrity_checked: false,
            display_start: None,
            audio: Vec::new(),
            terrain_scr: false,
            rules: Rules::ORIGINAL,
            split_held: [false; 2],
            shoot: [ShootView::default(); 2],
        };
        let State { fx, eb, pb, en, sc, pl, title, flow, scenes, draw_params, sound, grids, tape } = initial;
        let mut g = Core {
            fixes,
            tables,
            fx,
            eb,
            pb,
            en,
            sc,
            pl,
            enh: Enhanced::default(),
            title,
            flow,
            scenes,
            draw_params,
            sound,
            grids,
            tape,
            integrity_checked: false,
            display_start: None,
            audio: Vec::new(),
            banks: Banks::default(),
            files: Rc::new(GameFiles { dm1: dm1.to_vec(), ..GameFiles::default() }),
            map_seg: Vec::new(),
            draws: Vec::new(),
            cur_obj: None,
            gens: [[0; 40]; POOLS],
            g2_slots: vec![0; 0x321],
            ai: AiState::default(),
            port: Rc::new(RefCell::new(port)),
        };
        // 兩個入口都載入展示錄影（10CF9h 的 sub_1BED5）。
        g.load_demo(DemoFile::Dm1);
        // 標題流程設定的值。
        g.set_demo_mode(matches!(start, Start::Demo));
        g.set_players_present(0x01);
        g.mark_title_entered();
        g.request_stage_music();
        if let Start::NewGame { difficulty: Some(d) } = start {
            let (hp, fire) = match d {
                0 => (0, 0),
                1 => (0, 2),
                2 => (1, 2),
                _ => (1, 3),
            };
            g.set_difficulty_bonuses(hp, fire);
        }
        g.init_stage1();
        g
    }

    /// 單元測試用：沒有原版檔，表格是空的、狀態全為 0（測試自己擺需要的項目）。
    #[cfg(test)]
    pub(crate) fn for_test(start: Start, fixes: Fixes) -> Core {
        Core::from_initial(Rc::new(Tables::for_test()), State::default(), &[], start, fixes)
    }

    /// 單元測試用：改表格。
    #[cfg(test)]
    pub(crate) fn tables_mut(&mut self) -> &mut Tables {
        Rc::make_mut(&mut self.tables)
    }

    /// 單元測試用：擺上原版的兩張列表（`列 × 28h`、`列 × 50h + 1`），不用原版檔的測試要它才能算格位址。
    #[cfg(test)]
    pub(crate) fn fill_row_tables(&mut self) {
        let t = std::mem::take(self.tables_mut());
        *self.tables_mut() = t.with_row_tables();
    }

    /// 目前的狀態，給 [`Game::state`] 與對拍。
    pub(crate) fn snapshot(&self) -> State {
        State {
            fx: self.fx.clone(),
            eb: self.eb.clone(),
            pb: self.pb.clone(),
            en: self.en.clone(),
            sc: self.sc.clone(),
            pl: self.pl.clone(),
            title: self.title.clone(),
            flow: self.flow,
            scenes: self.scenes.clone(),
            draw_params: self.draw_params,
            sound: self.sound,
            grids: self.grids.clone(),
            tape: self.tape.clone(),
        }
    }

    /// 以 `state` 取代全部領域的狀態。載入快照、[`Game::state_mut`] 的修改在協程恢復時套用。
    pub(crate) fn apply_state(&mut self, state: State) {
        let State { fx, eb, pb, en, sc, pl, title, flow, scenes, draw_params, sound, grids, tape } = state;
        self.fx = fx;
        self.eb = eb;
        self.pb = pb;
        self.en = en;
        self.sc = sc;
        self.pl = pl;
        self.title = title;
        self.flow = flow;
        self.scenes = scenes;
        self.draw_params = draw_params;
        self.sound = sound;
        self.grids = grids;
        self.tape = tape;
        self.sync_g2_slots();
    }

    /// 把控制權交回外面，恢復時處理這段期間送達的掃描碼。
    pub(crate) async fn suspend(&mut self, req: Req) {
        {
            let mut p = self.port.borrow_mut();
            p.req = Some(req);
            p.state = self.snapshot();
            p.draws.append(&mut self.draws);
            p.integrity_checked = self.integrity_checked;
            p.display_start = self.display_start;
            p.audio.append(&mut self.audio);
            p.shoot = self.enh.view();
        }
        YieldOnce(false).await;
        let (input, pending) = {
            let mut p = self.port.borrow_mut();
            (std::mem::take(&mut p.input), p.pending.take())
        };
        if let Some(state) = pending {
            // `Game::state_mut` 改過狀態：協程從這個狀態接著跑。
            self.apply_state(state);
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

/// HUD 介面要顯示的玩家狀態，見 `Game::hud_status`。
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
    /// 細步移動選項（規格 005）作用中。`can_*` 用細步的界限（欄 1 至 33、y 0 至 160），`y` 可能是半列位置。
    pub fine_step: bool,
    /// 上一個邏輯步實際採用的方向：水平（左 -1、右 +1）、垂直（上 -1、下 +1）。細步移動時，
    /// 方向和它相同的下一步走整步（2 欄、10 像素），不同的走半步（1 欄、5 像素）。`fine_step` 為假時是 (0, 0)。
    pub last_move: (i8, i8),
    /// 砲座已脫離機身（規格 005「砲座像 Force」）。分離的砲座不跟著機身，顯示層不能把機身的預測偏移套在它身上。
    pub pods_detached: bool,
}

/// 對外的遊戲物件：一個跑在協程裡的 `Core`。
pub struct Game {
    /// 上一次交回控制權時的狀態。要改值用 [`Game::state_mut`]。
    state: State,
    /// `state` 被改過，下一次恢復時要套用。
    dirty: bool,
    fixes: Fixes,
    tables: Rc<Tables>,
    port: Rc<RefCell<Port>>,
    fut: Pin<Box<dyn Future<Output = Exit>>>,
    finished: Option<Exit>,
}

fn stage1(mut core: Core) -> Pin<Box<dyn Future<Output = Exit>>> {
    Box::pin(async move { core.run_stage1_rounds().await })
}

/// 跑一個入口，回傳離開碼。關卡入口直接回傳離開碼；標題與片尾回來就是程式結束，回傳 `Quit`。
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

/// 解析表格。映像雜湊不符就拒絕（fail-closed）。
fn parse_tables(image: &[u8]) -> Rc<Tables> {
    match Tables::parse(image) {
        Ok(t) => Rc::new(t),
        Err(e) => panic!("S.EXE 解包映像不是已知的原版：{e}"),
    }
}

impl Game {
    /// 從一份狀態開始跑。`state` 是原版在該入口的狀態（`rs2-trace` 從傾印讀出），或正式遊戲的開機狀態（[`boot_state`]）。
    /// `files.image` 不是已知的原版解包映像就 `panic`。
    pub fn from_state(files: GameFiles, state: State, entry: Entry, fixes: Fixes) -> Game {
        let tables = parse_tables(&files.image);
        let files = Rc::new(files);
        let mut core = Core::new(Rc::clone(&tables), &[], Start::NewGame { difficulty: None }, fixes);
        core.draws.clear();
        core.apply_state(state);
        core.files = files;
        // sub_11375 通過時把 dl（`Y`）寫進 byte_1CFAA，並把自己改成 retn。快照要帶上這個狀態。
        core.integrity_checked = core.flow.integrity_mark != b'N';
        let state = core.snapshot();
        let port = core.port.clone();
        port.borrow_mut().state = state.clone();
        Game { state, dirty: false, fixes, tables, port, fut: Box::pin(run_entry(core, entry)), finished: None }
    }

    /// 正式遊戲從標題開始：開機狀態加上全部原版檔案。
    pub fn boot(files: GameFiles, fixes: Fixes) -> Game {
        let tables = parse_tables(&files.image);
        let state = boot_state(tables, &files.dm1);
        Game::from_state(files, state, Entry::Title, fixes)
    }

    pub fn new(image: &[u8], dm1: &[u8], start: Start, fixes: Fixes) -> Game {
        Game::new_with_rules(image, dm1, start, fixes, Rules::ORIGINAL)
    }

    /// 同 [`Game::new`]，強化選項在第一關進場之前就位。戰場大小在進關時才讀，所以只有 `rules.field` 不是 100%
    /// 時才需要，其他選項隨時用 [`Game::set_rules`] 改。`rules.field` 是 100% 時和 `Game::new` 完全相同。
    pub fn new_with_rules(image: &[u8], dm1: &[u8], start: Start, fixes: Fixes, rules: Rules) -> Game {
        let tables = parse_tables(image);
        let mut core = Core::new(Rc::clone(&tables), dm1, start, fixes);
        core.port.borrow_mut().rules = rules;
        if rules.field != FieldSize::Normal && !matches!(start, Start::Demo) {
            core.draws.clear();
            core.init_stage1();
        }
        let state = core.snapshot();
        let port = core.port.clone();
        port.borrow_mut().draws.append(&mut core.draws);
        Game { state, dirty: false, fixes, tables, port, fut: stage1(core), finished: None }
    }

    /// 重新開始：沿用同一份狀態，只重跑進入關卡的初始化，殘值行為與原版相同。
    pub fn restart(&mut self, start: Start) {
        // 強化選項、分離鍵與第三關地形來源屬於前端的設定，不屬於一局的狀態，重新開始要帶到新的埠。
        let (checked, rules, split_held, terrain_scr) = {
            let p = self.port.borrow();
            (p.integrity_checked, p.rules, p.split_held, p.terrain_scr)
        };
        let mut core = Core::new(Rc::clone(&self.tables), &[], start, self.fixes);
        // 戰場大小在進關時讀選項（`Core::enter_field`），所以選項要在初始化之前就位。
        {
            let mut p = core.port.borrow_mut();
            p.rules = rules;
            p.split_held = split_held;
            p.terrain_scr = terrain_scr;
        }
        core.apply_state(self.state.clone());
        core.integrity_checked = checked;
        core.set_demo_mode(matches!(start, Start::Demo));
        core.set_players_present(0x01);
        core.draws.clear();
        core.init_stage1();
        self.state = core.snapshot();
        self.dirty = false;
        let port = core.port.clone();
        {
            let mut p = port.borrow_mut();
            p.draws.append(&mut core.draws);
            p.rules = rules;
            p.split_held = split_held;
            p.terrain_scr = terrain_scr;
        }
        self.port = port;
        self.fut = stage1(core);
        self.finished = None;
    }

    /// 上一次交回控制權時的狀態。
    pub fn state(&self) -> &State {
        &self.state
    }

    /// 改上一次交回控制權時的狀態：修改在下一次恢復時套用（測試與對拍的 poke、快照用）。
    pub fn state_mut(&mut self) -> &mut State {
        self.dirty = true;
        &mut self.state
    }

    fn take_audio(&mut self) -> Vec<AudioCmd> {
        std::mem::take(&mut self.port.borrow_mut().audio)
    }

    /// 這一步之前累積的繪製命令（進入關卡時的初始化命令也在這裡）。
    pub fn take_draws(&mut self) -> Vec<DrawCmd> {
        std::mem::take(&mut self.port.borrow_mut().draws)
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
        if std::mem::take(&mut self.dirty) {
            self.port.borrow_mut().pending = Some(self.state.clone());
        }
        let mut cx = Context::from_waker(Waker::noop());
        let r = self.fut.as_mut().poll(&mut cx);
        self.state.clone_from(&self.port.borrow().state);
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

    /// 目前這一關的戰場欄數：40 是原版，戰場加大時是 48 或 60（規格 005「戰場大小」）。進關時決定，關內不變；
    /// 標題、過場與片尾是 40。前端用它決定輸出寬度（欄數 × 8 像素）與能不能切到單色模式。
    pub fn field_cols(&self) -> u16 {
        self.state.grids.cols()
    }

    /// HUD 介面要顯示的玩家狀態。`player` 是 0 或 1。只讀，不影響模擬。
    pub fn hud_status(&self, player: usize) -> HudStatus {
        let players = &self.state.pl;
        let p = &players.ship[player];
        // 波動砲充能（規格 005）開著時，充能是強化狀態的計數（0 至 `WAVE_CHARGE_FULL`），不是原版的鍵盤事件計數。
        let port = self.port.borrow();
        let charge = if port.rules.wave_charge { port.shoot[player.min(1)].charge } else { p.charge };
        HudStatus { active: players.is_alive(player), hp: p.hp, weapon: p.weapon, pods: p.pods.installed(), charge }
    }

    /// 無敵選項（規格 005）：沿用原版秘技的旗標，`word_273A5` 是 `Y` 時受傷不減血（`1451Ch`、`1504Ah`，
    /// 玩家領域的 `take_hit` 讀它）。
    /// 原版的 Alt 鍵會把旗標清成 `N`，所以開著的時候呼叫端每步都要呼叫一次。從沒開過就不寫，模擬和原版相同。
    pub fn set_invincible(&mut self, on: bool) {
        self.state_mut().flow.controls.cheat.invincible = if on { 0x59 } else { 0x4E };
    }

    /// 玩家機的狀態，給前端的顯示層預測用。`player` 是 0 或 1。
    ///
    /// 啟用位元與界限來自 `sub_14501`、`sub_1502F` 與各方向的移動函式，位置與列欄是它們更新的變數。
    /// 界限用玩家領域移動函式的同一組條件（`Player::can_move_up` 一族）。
    pub fn player_status(&self, player: usize) -> PlayerStatus {
        let players = &self.state.pl;
        let p = &players.ship[player];
        let (rules, view) = {
            let port = self.port.borrow();
            (port.rules, port.shoot[player.min(1)])
        };
        let demo = self.state.flow.demo == b'Y';
        // 展示錄影不套用強化規則（`Core::rules`）。
        let fine = rules.fine_step && !demo;
        // 右緣的界限跟著這一關的欄數（規格 005「戰場大小」）。
        let right = crate::player::right_col_for(self.state.grids.cols());
        let reach = if fine {
            // 細步移動：同樣的邊界，垂直用繪製位置 y（原版的列界限 0 至 16 換成 0 至 160 像素）。
            (
                (p.blit_y as i16) > 0,
                (p.blit_y as i16) < (crate::player::BOTTOM_PIXELS),
                p.can_move_left(),
                p.can_move_right_to(right),
            )
        } else {
            (p.can_move_up(), p.can_move_down(), p.can_move_left(), p.can_move_right_to(right))
        };
        PlayerStatus {
            x: p.blit_x as i16,
            y: p.blit_y as i16,
            // 玩家位元：還在場上。血量 0：死亡動畫中，`die_step` 自己往下往右走，不接受輸入。
            // `byte_1CFDC` 是 `Y`：示範，方向來自錄影。`byte_1CFAD` 是 `F`：不跑玩家邏輯。
            controllable: players.is_alive(player) && p.hp != 0 && !demo && !self.state.flow.is_cleared(),
            can_up: reach.0,
            can_down: reach.1,
            can_left: reach.2,
            can_right: reach.3,
            fine_step: fine,
            last_move: if fine { view.last_move } else { (0, 0) },
            pods_detached: view.pods_detached && !demo,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 沒有原版檔：表格是空的，只測 `Core` 的接線。
    fn blank_game(start: Start) -> Game {
        let mut core = Core::for_test(start, Fixes::NONE);
        let state = core.snapshot();
        let port = core.port.clone();
        port.borrow_mut().draws.append(&mut core.draws);
        Game { state, dirty: false, fixes: Fixes::NONE, tables: Rc::new(Tables::default()), port, fut: stage1(core), finished: None }
    }

    #[test]
    fn restart_keeps_the_enhanced_rules() {
        let mut g = blank_game(Start::Demo);
        assert_eq!(g.port.borrow().rules, Rules::ORIGINAL);
        let rules = Rules { ai_aim: true, ai_boss: true, ..Rules::ORIGINAL };
        g.set_rules(rules);
        g.restart(Start::NewGame { difficulty: Some(1) });
        assert_eq!(g.port.borrow().rules, rules, "重新開始沿用強化選項，不洗回全關");
    }
}
