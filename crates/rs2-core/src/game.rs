//! 核心狀態與入口。規格見 `docs/spec/001-rust-first-stage.md`。

use crate::draw::{DrawCmd, ObjId, Pool, ShtGroup};
use crate::dseg::{Dseg, a};
use crate::regs::Regs;

/// 原版缺陷的修正開關。遊戲預設全開，對拍全關。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fixes {
    pub bullet_slot_carry: bool,
    pub armor_cell_index: bool,
    pub seeker_spawn_leak: bool,
}

impl Fixes {
    pub const NONE: Fixes = Fixes { bullet_slot_carry: false, armor_cell_index: false, seeker_spawn_leak: false };
    pub const ALL: Fixes = Fixes { bullet_slot_carry: true, armor_cell_index: true, seeker_spawn_leak: true };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Demo,
    /// `difficulty` 是選單的四段（0～3）。`None` 是不經選單，難度字維持映像值 0、0。
    NewGame { difficulty: Option<u8> },
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
    pub exit: Option<Exit>,
    /// 暫停中，這一步沒有推進模擬。
    pub paused: bool,
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

const POOLS: usize = 9;

pub struct Game {
    pub ds: Dseg,
    pub r: Regs,
    pub fixes: Fixes,
    /// `sub_10CD0` 主迴圈的相位：下一步是第二頁。
    pub(crate) second_page: bool,
    /// `sub_10ECB` 的暫停，原版是阻塞迴圈。
    pub(crate) paused: bool,
    /// `sub_11375` 通過後把自己改成 `retn`。
    pub(crate) s11375_patched: bool,
    pub(crate) sht_group: ShtGroup,
    pub(crate) draws: Vec<DrawCmd>,
    pub(crate) cur_obj: Option<ObjId>,
    pub(crate) gens: [[u32; 40]; POOLS],
    /// `bullet_slot_carry` 開啟時，G2 每格真正的敵彈槽號加 1，0 表示沒有。不屬於原版狀態。
    pub(crate) g2_slots: Vec<u8>,
    /// `sub_16578` 配到槽的次數，`seeker_spawn_leak` 用。不屬於原版狀態。
    pub(crate) pb_spawned: u16,
    pub(crate) finished: Option<Exit>,
}

impl Game {
    pub fn new(image: &[u8], dm1: &[u8], start: Start, fixes: Fixes) -> Game {
        let mut g = Game {
            ds: Dseg::from_image(image),
            r: Regs::default(),
            fixes,
            second_page: false,
            paused: false,
            s11375_patched: false,
            sht_group: ShtGroup::Bh1,
            draws: Vec::new(),
            cur_obj: None,
            gens: [[0; 40]; POOLS],
            g2_slots: vec![0; 0x321],
            pb_spawned: 0,
            finished: None,
        };
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

    /// 重新開始：沿用同一份 `Dseg`，只重跑進入關卡的初始化，殘值行為與原版相同。
    pub fn restart(&mut self, start: Start) {
        self.ds.w8(0x041C, if matches!(start, Start::Demo) { b'Y' } else { b'N' });
        self.ds.w8(0x041D, 1);
        self.second_page = false;
        self.paused = false;
        self.finished = None;
        self.enter_stage();
    }

    /// 這一幀之後才要處理的繪製命令（進入關卡時累積的初始化命令也在這裡）。
    pub fn take_draws(&mut self) -> Vec<DrawCmd> {
        std::mem::take(&mut self.draws)
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

    pub fn state_hash_input(&self) -> &[u8] {
        self.ds.bytes()
    }
}
