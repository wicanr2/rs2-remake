//! 第一關的視差景物：遠、中、近三層往左飄的背景小物。
//!
//! 池有 10 槽，每槽一個景物：圖庫索引、x、y 與每幀往左的欄數。圖庫索引同時是狀態，0 是空槽。
//! 生成（原版 `sub_15C3C`、`sub_15C5F`）每幀掃一張群組表，幀計數相符的列把一筆景物記錄放進第一個空槽。
//! 繪製（原版 `sub_15C92`）依索引 10h、0Fh、0Eh 的順序畫，也就是由遠到近；每畫一個，x 減去它的步長，
//! 新的 x 小於等於 -7（有號）就離開，槽清成空槽、x 寫 0。景物不寫格網，不參與碰撞。
//! 只有第一關有群組表。語意見 `docs/re/11-entities-combat.md` 第 4.3 節。
//!
//! 狀態與資料段位移的對應在 `mirror.rs`，這個檔案不出現原版位移。

use crate::draw::Pool;
use crate::game::Core;

/// 池的槽數。
pub(crate) const POOL_SIZE: usize = 10;

/// 三層的圖庫索引，依繪製順序：遠、中、近。
const LAYERS: [u16; 3] = [0x10, 0x0F, 0x0E];

/// 新的 x（有號）小於等於這個值就離開畫面。
const EXIT_COL: i16 = -7;

/// 一個景物。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Prop {
    /// 圖庫索引，也是狀態：0 是空槽。
    pub image: u16,
    /// 欄，位元組（8 像素），可為負。
    pub x: u16,
    /// 像素列。
    pub y: u16,
    /// 每幀往左的欄數。
    pub dx: u16,
}

impl Prop {
    fn is_active(&self) -> bool {
        self.image != 0
    }
}

/// 景物領域的全部狀態。
#[derive(Clone, Debug, Default)]
pub(crate) struct Scenery {
    pub(crate) props: [Prop; POOL_SIZE],
}

impl Scenery {
    fn first_free(&self) -> Option<usize> {
        self.props.iter().position(|p| !p.is_active())
    }
}

impl Core {
    /// 掃群組表，幀計數相符的每一列放一個景物（原版 `sub_15C3C`）。表不依幀排序，幀為 0 結束。
    pub(crate) fn spawn_scenery(&mut self) {
        let mut row = 0u16;
        loop {
            let (frame, record) = self.scenery_group(row);
            if frame == 0 {
                return;
            }
            if frame == self.stage_frame() {
                let prop = self.scenery_record(record);
                self.place_prop(prop);
            }
            row = row.wrapping_add(1);
        }
    }

    /// 把景物放進第一個空槽（原版 `sub_15C5F`）。10 槽全滿就放棄。
    fn place_prop(&mut self, prop: Prop) {
        let Some(slot) = self.sc.first_free() else { return };
        self.sc.props[slot] = prop;
        self.bump_gen(Pool::Scenery, slot as u8);
    }

    /// 畫全部景物（原版 `sub_15C92`，各層是 `sub_15C9C`、`sub_15CB8`、`sub_15CD4`）：由遠到近，同一層依槽號。
    pub(crate) fn draw_scenery(&mut self) {
        for layer in LAYERS {
            for slot in 0..POOL_SIZE {
                if self.sc.props[slot].image == layer {
                    self.set_obj(Pool::Scenery, slot as u8);
                    self.draw_prop(slot);
                    self.clear_obj();
                }
            }
        }
    }

    /// 畫一個景物（原版 `sub_15CF0`）：先畫遮罩再畫平面，位置是移動之前的 x。
    /// 之後 x 減去步長；新的 x 小於等於 -7 就清掉狀態，x 寫 0。
    fn draw_prop(&mut self, slot: usize) {
        let Prop { image, x, y, dx } = self.sc.props[slot];
        let next = x.wrapping_sub(dx);
        if (next as i16) <= EXIT_COL {
            self.sc.props[slot].image = 0;
            self.sc.props[slot].x = 0;
        } else {
            self.sc.props[slot].x = next;
        }
        self.set_blit_params(x, y, image);
        self.cut_stage();
        self.overlay_stage();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{DrawCmd, Op};
    use crate::game::{Fixes, Start};

    fn core() -> Core {
        let image = vec![0u8; rs2_formats::DSEG_IMAGE_OFFSET + 0x10000];
        let mut c = Core::new(&image, &[], Start::Demo, Fixes::NONE);
        c.draws.clear();
        c
    }

    fn blits(c: &Core) -> Vec<crate::draw::BlitCmd> {
        c.draws.iter().filter_map(|d| if let DrawCmd::Blit(b) = d { Some(*b) } else { None }).collect()
    }

    /// 在群組表第 `row` 列放 `(幀, 記錄位址)`，記錄放在 `0x7000 + 8 * row`。
    fn put_group(c: &mut Core, row: u16, frame: u16, prop: Prop) {
        let rec = 0x7000 + 8 * row;
        for (k, w) in [prop.image, prop.x, prop.y, prop.dx].into_iter().enumerate() {
            c.ds.w16(rec + 2 * k as u16, w);
        }
        let at = 0x19DC + 4 * row;
        c.ds.w16(at, frame);
        c.ds.w16(at + 2, rec);
    }

    #[test]
    fn spawn_places_matching_groups_in_the_first_free_slots() {
        let mut c = core();
        let (far, near) = (Prop { image: 0x10, x: 40, y: 20, dx: 1 }, Prop { image: 0x0E, x: 40, y: 90, dx: 4 });
        put_group(&mut c, 0, 385, far);
        put_group(&mut c, 1, 386, near);
        put_group(&mut c, 2, 385, near);
        put_group(&mut c, 3, 0, near);
        c.set_stage_frame(385);
        c.spawn_scenery();
        assert_eq!((c.sc.props[0], c.sc.props[1]), (far, near));
        assert!(!c.sc.props[2].is_active());
        c.set_stage_frame(386);
        c.spawn_scenery();
        assert_eq!(c.sc.props[2], near);
        assert_eq!(c.gens[Pool::Scenery as usize][2], 1, "配到的槽世代加 1");
    }

    #[test]
    fn spawn_with_a_full_pool_drops_the_prop() {
        let mut c = core();
        for p in c.sc.props.iter_mut() {
            p.image = 0x0F;
        }
        let before = c.sc.props;
        put_group(&mut c, 0, 7, Prop { image: 0x10, x: 1, y: 2, dx: 3 });
        c.set_stage_frame(7);
        c.spawn_scenery();
        assert_eq!(c.sc.props, before);
    }

    #[test]
    fn draw_goes_far_to_near_then_by_slot() {
        let mut c = core();
        c.sc.props[0] = Prop { image: 0x0E, x: 10, y: 1, dx: 4 };
        c.sc.props[1] = Prop { image: 0x10, x: 11, y: 2, dx: 1 };
        c.sc.props[2] = Prop { image: 0x0F, x: 12, y: 3, dx: 2 };
        c.sc.props[3] = Prop { image: 0x10, x: 13, y: 4, dx: 1 };
        c.draw_scenery();
        let seen: Vec<(Op, u16)> = blits(&c).iter().map(|b| (b.op, b.index)).collect();
        let want = [0x10, 0x10, 0x0F, 0x0E].into_iter().flat_map(|i| [(Op::S18F1A, i), (Op::S18F5D, i)]).collect::<Vec<_>>();
        assert_eq!(seen, want, "每個景物遮罩加平面，層序 10h、0Fh、0Eh");
        assert_eq!(blits(&c).iter().map(|b| b.x).step_by(2).collect::<Vec<_>>(), [11, 13, 12, 10]);
    }

    #[test]
    fn draw_uses_the_old_x_and_then_moves_left() {
        let mut c = core();
        c.sc.props[4] = Prop { image: 0x0F, x: 20, y: 30, dx: 2 };
        c.draw_scenery();
        let b = blits(&c);
        assert_eq!((b[0].x, b[0].y, b[0].index), (20, 30, 0x0F), "畫在舊的 x");
        assert!(b.iter().all(|c| c.obj.map(|o| (o.pool, o.slot)) == Some((Pool::Scenery, 4))));
        assert_eq!(c.sc.props[4].x, 18);
        assert_eq!(c.sc.props[4].image, 0x0F);
    }

    #[test]
    fn a_prop_leaves_when_the_new_x_reaches_minus_7() {
        let mut c = core();
        // -6 仍留著，-7 離開（有號，小於等於）。
        c.sc.props[0] = Prop { image: 0x0F, x: 1, y: 0, dx: 7 };
        c.sc.props[1] = Prop { image: 0x0F, x: 1, y: 0, dx: 8 };
        c.draw_scenery();
        assert_eq!(c.sc.props[0], Prop { image: 0x0F, x: (-6i16) as u16, y: 0, dx: 7 });
        assert_eq!(c.sc.props[1], Prop { image: 0, x: 0, y: 0, dx: 8 }, "狀態清掉，x 寫 0，步長留著");
        assert_eq!(blits(&c).len(), 4, "離開的那一幀還是畫了");
        c.draw_scenery();
        assert_eq!(blits(&c).len(), 6, "空槽不畫");
    }

    #[test]
    fn a_prop_that_starts_off_screen_to_the_left_is_drawn_once_then_removed() {
        let mut c = core();
        c.sc.props[0] = Prop { image: 0x10, x: (-30i16) as u16, y: 0, dx: 1 };
        c.draw_scenery();
        assert!(!c.sc.props[0].is_active());
        assert_eq!(blits(&c)[0].x, (-30i16) as u16);
    }

    #[test]
    fn an_unknown_state_is_never_drawn_and_never_leaves() {
        let mut c = core();
        c.sc.props[0] = Prop { image: 0x11, x: 3, y: 4, dx: 1 };
        c.draw_scenery();
        c.draw_scenery();
        assert!(c.draws.is_empty());
        assert_eq!(c.sc.props[0].x, 3);
    }
}
