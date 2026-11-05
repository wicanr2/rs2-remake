//! 特效池：命中火花、爆炸、尾跡與過關連爆。
//!
//! 池有 30 槽，每槽記一個位置與腳本讀取位置。腳本是一串「圖、dx、dy、形狀」，每幀走一格；
//! 圖號為 [`END_OF_SCRIPT`] 時槽釋放。每格先畫遮罩與四平面（圖庫 `MY.GO`），再把形狀以 `or` 寫進格網 G3。
//! 原版是 `sub_17A3A` 一族，語意見 `docs/re/11-entities-combat.md` 第 8.1 節。
//! 狀態與資料段位移的對應在 `mirror.rs`，這個檔案不出現原版位移。

use crate::draw::Pool;
use crate::game::Core;

/// 特效池的槽數。
pub(crate) const POOL_SIZE: usize = 30;

/// 腳本結束的圖號。
const END_OF_SCRIPT: u16 = 0x0F;

/// 腳本一格的位元組數：圖、dx、dy、形狀指標各一個字組。
const FRAME_BYTES: u16 = 8;

/// y 的單位是格列，乘 10 為像素列。
const ROW_PIXELS: u16 = 10;

/// 一個特效槽。原版是三個平行陣列（x、y、腳本指標）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Effect {
    /// 水平位置，單位是位元組（8 像素）。
    pub x: u16,
    /// 垂直位置，單位是格列。釋放的槽保留舊的 `x`、`y`。
    pub y: u16,
    /// 腳本下一格的讀取位置，原版為資料段指標。0 表示空槽。
    pub script_pos: u16,
}

impl Effect {
    pub(crate) fn is_active(&self) -> bool {
        self.script_pos != 0
    }
}

/// 特效領域的全部狀態。
#[derive(Clone, Debug, Default)]
pub(crate) struct Effects {
    pub(crate) slots: [Effect; POOL_SIZE],
    /// 位置游標（原版 `word_20B7C`、`word_20B7E`）。生成時當作新特效的位置，更新每一格時寫成該格的新位置。
    /// 單顆 `E` 的呼叫端不給位置，沿用游標上一次的值（見 [`Core::spawn_stage_clear_burst`]），所以它是狀態。
    pub(crate) cursor: (u16, u16),
    /// 過關連爆的起點欄（原版 `word_20B7A`）。關卡初始化歸零，過關連爆每輪往左 3 欄。
    pub(crate) chain_base_x: u16,
}

impl Effects {
    fn first_free(&self) -> Option<usize> {
        self.slots.iter().position(|e| !e.is_active())
    }

    /// 關卡初始化：連爆起點歸零。
    pub(crate) fn reset_chain(&mut self) {
        self.chain_base_x = 0;
    }

    /// 過關連爆每輪把起點往左移。
    pub(crate) fn shift_chain_left(&mut self, cols: u16) {
        self.chain_base_x = self.chain_base_x.wrapping_sub(cols);
    }
}

/// 動畫腳本。原版稱動畫表，圖號序列見 `docs/re/11-entities-combat.md` 第 8.1 節。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Script {
    /// 火花（右）。
    SparkRight,
    /// 火花（左）。
    SparkLeft,
    /// 大爆炸。
    BigExplosion,
    /// 一團爆炸。
    Blast,
    /// 尾跡與小爆。
    Trail,
    /// 追蹤彈尾跡。
    SeekerTrail,
    /// 玩家死亡爆炸。
    PlayerDeath,
}

/// 呼叫端要的特效。多半對應一個腳本，`DoubleBlast` 起兩團，`ClearChain` 起十個。
/// 原版以字母指定（`word_20B78`），[`EffectKind::from_letter`] 轉換表格裡存的字母。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectKind {
    /// `S`、`>` 與不認得的字母：玩家子彈命中火花（右）、實體被打爆、玩家被命中。
    SparkRight,
    /// `<`：玩家子彈命中火花（左）。
    SparkLeft,
    /// `B`：大爆炸。
    BigExplosion,
    /// `V`：兩團爆炸，第二團在 x + 6。
    DoubleBlast,
    /// `E`：尾跡與小爆。
    Trail,
    /// `A`：追蹤彈尾跡。
    SeekerTrail,
    /// `G`：玩家死亡爆炸。
    PlayerDeath,
    /// `e`：過關連爆，以 [`Effects::chain_base_x`] 為基底的 10 個特效。
    ClearChain,
}

impl EffectKind {
    /// 原版的字母（`sub_17A3A` 的比較順序）。不認得的字母與 `S`、`>` 同為火花（右）。
    pub(crate) fn from_letter(letter: u16) -> EffectKind {
        match letter {
            0x42 => EffectKind::BigExplosion,
            0x56 => EffectKind::DoubleBlast,
            0x45 => EffectKind::Trail,
            0x41 => EffectKind::SeekerTrail,
            0x47 => EffectKind::PlayerDeath,
            0x3C => EffectKind::SparkLeft,
            0x65 => EffectKind::ClearChain,
            _ => EffectKind::SparkRight,
        }
    }
}

/// 過關連爆的 10 個特效：(x 相對起點的位移, y, 腳本)，依生成順序。原版 `17B99h` 起。
const CLEAR_CHAIN: [(u16, u16, Script); 10] = [
    (0x02, 0x07, Script::BigExplosion),
    (0x0F, 0x05, Script::PlayerDeath),
    (0x11, 0x0A, Script::Blast),
    (0x1C, 0x0B, Script::BigExplosion),
    (0x21, 0x00, Script::Trail),
    (0x03, 0x0C, Script::SeekerTrail),
    (0x22, 0x10, Script::PlayerDeath),
    (0x24, 0x10, Script::Blast),
    (0x21, 0x02, Script::Trail),
    (0x19, 0x07, Script::BigExplosion),
];

/// 腳本的一格。
#[derive(Clone, Copy)]
struct ScriptFrame {
    image: u16,
    dx: u16,
    dy: u16,
    /// 形狀指標，指向 `寬, 高, 寬 × 高 位元組` 的格值。
    shape: u16,
}

/// 8086 `loop` 先減後判，計數為 0 時跑 65536 圈。形狀的寬高來自表格，不會是 0，但照原版行為寫。
pub(crate) fn loop_count(n: u16) -> u32 {
    if n == 0 { 0x1_0000 } else { n as u32 }
}

impl Core {
    /// 在 (`x`, `y`) 生成特效。位置同時寫進游標（原版呼叫端先寫 `word_20B7C`、`word_20B7E` 再呼叫 `sub_17A3A`）。
    pub(crate) fn spawn_effect(&mut self, kind: EffectKind, x: u16, y: u16) {
        self.fx.cursor = (x, y);
        self.spawn_effect_at_cursor(kind);
    }

    /// 在游標位置生成特效（原版 `sub_17A3A`）。池滿了就放棄，不報錯。
    ///
    /// `17A64h` 的第二個 `E` 比較被 `17A4Fh` 攔截，其後的另一組 10 個連爆（`17AA3h`）不可達，不移植。
    pub(crate) fn spawn_effect_at_cursor(&mut self, kind: EffectKind) {
        match kind {
            EffectKind::SparkRight => self.start_script(Script::SparkRight),
            EffectKind::SparkLeft => self.start_script(Script::SparkLeft),
            EffectKind::BigExplosion => self.start_script(Script::BigExplosion),
            EffectKind::DoubleBlast => {
                self.start_script(Script::Blast);
                self.fx.cursor.0 = self.fx.cursor.0.wrapping_add(6);
                self.start_script(Script::Blast);
            }
            EffectKind::Trail => self.start_script(Script::Trail),
            EffectKind::SeekerTrail => self.start_script(Script::SeekerTrail),
            EffectKind::PlayerDeath => self.start_script(Script::PlayerDeath),
            EffectKind::ClearChain => self.spawn_clear_chain(),
        }
    }

    /// 過關連爆的一輪（原版各關主迴圈的 `e`／`E` 選擇，例如第一關 `10D75h`、`10D82h`）。
    /// 繪製頁為 0 時起 10 個連爆；否則在游標位置起一個 `E`，位置沿用游標的殘值。
    /// 第一關剛執行完 `sub_19A88`，頁恆為 0，所以永遠是連爆；其他關見各關主迴圈。
    pub(crate) fn spawn_stage_clear_burst(&mut self, draw_page: u16) {
        let kind = if draw_page != 0 { EffectKind::Trail } else { EffectKind::ClearChain };
        self.spawn_effect_at_cursor(kind);
    }

    /// 過關連爆（原版 `17B99h` 起）：每個特效的位置是 (起點 + 位移, y)。
    fn spawn_clear_chain(&mut self) {
        for (dx, y, script) in CLEAR_CHAIN {
            self.fx.cursor = (self.fx.chain_base_x.wrapping_add(dx), y);
            self.start_script(script);
        }
    }

    /// 把腳本放進第一個空槽，位置取游標（原版 `sub_17C6C`）。30 槽全滿就放棄。
    fn start_script(&mut self, script: Script) {
        let Some(slot) = self.fx.first_free() else { return };
        let (x, y) = self.fx.cursor;
        self.fx.slots[slot] = Effect { x, y, script_pos: script.table_offset() };
        self.bump_gen(Pool::Effect, slot as u8);
    }

    /// 每幀更新並繪製特效池（原版 `sub_17C9A`）：依槽號走每個使用中的槽一格。
    pub(crate) fn update_effects(&mut self) {
        for slot in 0..POOL_SIZE {
            if self.fx.slots[slot].is_active() {
                self.set_obj(Pool::Effect, slot as u8);
                self.step_effect(slot);
                self.clear_obj();
            }
        }
    }

    /// 第 `slot` 槽走一格（原版 `sub_17CB6`）。讀腳本目前這格，讀取位置先前進；
    /// 圖號為結束就釋放槽。否則位置加上 (dx, dy)，畫遮罩與四平面，再把形狀寫進 G3。
    /// 游標在讀格之前就設成這一槽的位置，所以結束的那一格也會留下游標。
    fn step_effect(&mut self, slot: usize) {
        let Effect { x, y, script_pos } = self.fx.slots[slot];
        self.fx.cursor = (x, y);
        let frame = self.script_frame(script_pos);
        self.fx.slots[slot].script_pos = script_pos.wrapping_add(FRAME_BYTES);
        if frame.image == END_OF_SCRIPT {
            self.fx.slots[slot].script_pos = 0;
            return;
        }
        let (x, y) = (x.wrapping_add(frame.dx), y.wrapping_add(frame.dy));
        self.fx.slots[slot].x = x;
        self.fx.slots[slot].y = y;
        self.fx.cursor = (x, y);
        self.set_blit_params(x, y.wrapping_mul(ROW_PIXELS), frame.image);
        self.s18e30();
        self.s18dcc();
        self.stamp_shape(frame.shape, x, y);
    }

    /// 把形狀以 `or` 寫進 G3，左上角在 (`x`, `y`)（原版 `sub_17D2C`）。
    /// 形狀寬同時寫進繪製的每列位元組殘值（`word_2687E`），後面的繪製命令會記下它。
    /// 位址落在 G3 範圍之外的格略過，列號超過 13h 的列都落在越界列。
    /// 玩家子彈（原版 `sub_1698C`）用同一套規則，共用這個函式。
    pub(crate) fn stamp_shape(&mut self, shape: u16, x: u16, y: u16) {
        let width = self.table_word(shape);
        let height = self.table_word(shape.wrapping_add(2));
        self.set_stride_reg(width);
        let mut src = shape.wrapping_add(4);
        let mut row = y;
        for _ in 0..loop_count(height) {
            let mut cell = self.grid3_cell_addr(row, x);
            for _ in 0..loop_count(width) {
                let v = self.table_byte(src);
                self.grid3_or(cell, v);
                cell = cell.wrapping_add(2);
                src = src.wrapping_add(1);
            }
            row = row.wrapping_add(1);
        }
    }

    // ---- 唯讀表格：腳本與形狀。表格階段搬走，現在仍從資料段讀（`Core::table_word`、`Core::table_byte` 在 `mirror.rs`）----

    fn script_frame(&self, pos: u16) -> ScriptFrame {
        ScriptFrame {
            image: self.table_word(pos),
            dx: self.table_word(pos.wrapping_add(2)),
            dy: self.table_word(pos.wrapping_add(4)),
            shape: self.table_word(pos.wrapping_add(6)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dseg::Dseg;
    use crate::game::{Fixes, Start};
    use crate::mirror::Mirror;

    fn core() -> Core {
        let image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
        Core::new(&image, &[], Start::Demo, Fixes::NONE)
    }

    /// 在 `ptr` 擺一格腳本。
    fn put_frame(c: &mut Core, ptr: u16, image: u16, dx: u16, dy: u16, shape: u16) {
        for (i, v) in [image, dx, dy, shape].into_iter().enumerate() {
            c.ds.w16(ptr + 2 * i as u16, v);
        }
    }

    /// 在 `ptr` 擺一個 1 x 1 的形狀。
    fn put_shape(c: &mut Core, ptr: u16, cell: u8) {
        c.ds.w16(ptr, 1);
        c.ds.w16(ptr + 2, 1);
        c.ds.w8(ptr + 4, cell);
    }

    #[test]
    fn letters_map_to_kinds() {
        for (letter, kind) in [
            (b'S', EffectKind::SparkRight),
            (b'>', EffectKind::SparkRight),
            (b'?', EffectKind::SparkRight),
            (b'<', EffectKind::SparkLeft),
            (b'B', EffectKind::BigExplosion),
            (b'V', EffectKind::DoubleBlast),
            (b'E', EffectKind::Trail),
            (b'A', EffectKind::SeekerTrail),
            (b'G', EffectKind::PlayerDeath),
            (b'e', EffectKind::ClearChain),
        ] {
            assert_eq!(EffectKind::from_letter(letter as u16), kind, "字母 {}", letter as char);
        }
    }

    #[test]
    fn spawn_takes_first_free_slot_and_sets_cursor() {
        let mut c = core();
        c.spawn_effect(EffectKind::Trail, 7, 9);
        c.spawn_effect(EffectKind::SparkLeft, 1, 2);
        assert_eq!(c.fx.slots[0], Effect { x: 7, y: 9, script_pos: Script::Trail.table_offset() });
        assert_eq!(c.fx.slots[1], Effect { x: 1, y: 2, script_pos: Script::SparkLeft.table_offset() });
        assert_eq!(c.fx.cursor, (1, 2));
        c.fx.slots[0].script_pos = 0;
        c.spawn_effect(EffectKind::BigExplosion, 3, 4);
        assert_eq!(c.fx.slots[0].x, 3, "釋放的槽優先重用");
    }

    #[test]
    fn full_pool_drops_the_new_effect() {
        let mut c = core();
        for i in 0..POOL_SIZE as u16 {
            c.spawn_effect(EffectKind::SparkRight, i, 0);
        }
        let before = c.fx.slots;
        c.spawn_effect(EffectKind::BigExplosion, 99, 99);
        assert_eq!(c.fx.slots, before);
        assert_eq!(c.fx.cursor, (99, 99), "池滿了游標仍然改變");
    }

    #[test]
    fn double_blast_places_second_six_columns_right_and_leaves_cursor_there() {
        let mut c = core();
        c.spawn_effect(EffectKind::DoubleBlast, 10, 5);
        assert_eq!((c.fx.slots[0].x, c.fx.slots[1].x), (10, 16));
        assert_eq!((c.fx.slots[0].y, c.fx.slots[1].y), (5, 5));
        assert_eq!(c.fx.cursor, (16, 5));
    }

    #[test]
    fn clear_chain_uses_ten_slots_from_chain_base() {
        let mut c = core();
        c.fx.chain_base_x = 100;
        c.spawn_effect_at_cursor(EffectKind::ClearChain);
        for (i, (dx, y, script)) in CLEAR_CHAIN.into_iter().enumerate() {
            assert_eq!(c.fx.slots[i], Effect { x: 100 + dx, y, script_pos: script.table_offset() });
        }
        assert!(!c.fx.slots[10].is_active());
        assert_eq!(c.fx.cursor, (100 + 0x19, 7));
        // 往左移再歸零。
        c.fx.shift_chain_left(3);
        assert_eq!(c.fx.chain_base_x, 97);
        c.fx.reset_chain();
        assert_eq!(c.fx.chain_base_x, 0);
    }

    #[test]
    fn stage_clear_burst_picks_chain_or_trail_by_page() {
        let mut c = core();
        c.fx.cursor = (4, 6);
        c.spawn_stage_clear_burst(0x8000);
        assert_eq!(c.fx.slots[0], Effect { x: 4, y: 6, script_pos: Script::Trail.table_offset() }, "頁不為 0：單顆 E，位置沿用游標");
        assert!(!c.fx.slots[1].is_active());
        c.spawn_stage_clear_burst(0);
        assert!(c.fx.slots[10].is_active(), "頁為 0：連爆");
    }

    #[test]
    fn step_advances_draws_and_stamps_shape() {
        let mut c = core();
        let (script, shape) = (Script::Trail.table_offset(), 0x7000u16);
        put_frame(&mut c, script, 24, 2, 3, shape);
        put_frame(&mut c, script + FRAME_BYTES, END_OF_SCRIPT, 0, 0, 0);
        put_shape(&mut c, shape, 0x05);
        c.draws.clear();
        c.spawn_effect(EffectKind::Trail, 4, 6);

        c.update_effects();
        assert_eq!(c.fx.slots[0], Effect { x: 6, y: 9, script_pos: script + FRAME_BYTES });
        assert_eq!(c.fx.cursor, (6, 9));
        let blits: Vec<_> = c
            .draws
            .iter()
            .filter_map(|d| if let crate::draw::DrawCmd::Blit(b) = d { Some(*b) } else { None })
            .collect();
        assert_eq!(blits.len(), 2, "遮罩與四平面各一次");
        assert!(blits.iter().all(|b| b.x == 6 && b.y == 90 && b.index == 24));
        let cell = c.grid3_cell_addr(9, 6);
        assert_eq!(c.ds.r8(cell), 0x05, "形狀寫進 G3");

        c.update_effects();
        assert!(!c.fx.slots[0].is_active(), "結束格釋放槽");
        assert_eq!(c.fx.slots[0].x, 6, "釋放的槽保留位置");
        assert_eq!(c.draws.len(), 2, "結束格不畫");
    }

    #[test]
    fn cursor_is_left_on_the_last_stepped_slot() {
        // 更新一輪後，游標是最後一個使用中的槽的新位置；單顆 `E` 沿用它。
        let mut c = core();
        let (a, b) = (Script::SparkRight.table_offset(), Script::SparkLeft.table_offset());
        put_shape(&mut c, 0x7000, 1);
        put_frame(&mut c, a, 2, 1, 1, 0x7000);
        put_frame(&mut c, b, 2, 5, 7, 0x7000);
        c.spawn_effect(EffectKind::SparkRight, 10, 10);
        c.spawn_effect(EffectKind::SparkLeft, 20, 20);
        c.update_effects();
        assert_eq!(c.fx.cursor, (25, 27));
        c.spawn_stage_clear_burst(8000);
        assert_eq!(c.fx.slots[2], Effect { x: 25, y: 27, script_pos: Script::Trail.table_offset() });
    }

    #[test]
    fn mirror_is_the_inverse_of_load_for_any_dump() {
        // 偽亂數填滿資料段：讀進再寫回，位元組不變。
        let mut image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
        let mut s = 0x1234_5678u32;
        for b in image[rs2_formats::DSEG_IMAGE_OFFSET..].iter_mut() {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *b = (s >> 24) as u8;
        }
        let ds = Dseg::from_image(&image);
        let mut fx = Effects::default();
        fx.load(&ds);
        let mut same = ds.clone();
        fx.store(&mut same);
        assert_eq!(same.bytes(), ds.bytes(), "store(load(ds)) 要等於 ds");
        // 寫進全 0 的資料段再讀回，結構相同：store 真的寫了每個欄位。
        let mut blank = Dseg::from_image(&vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET]);
        fx.store(&mut blank);
        let mut back = Effects::default();
        back.load(&blank);
        assert_eq!((back.slots, back.cursor, back.chain_base_x), (fx.slots, fx.cursor, fx.chain_base_x));
        assert_ne!(fx.slots, Effects::default().slots, "測試資料不該全 0");
    }
}
