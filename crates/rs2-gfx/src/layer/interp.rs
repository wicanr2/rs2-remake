//! 補間：把目前幀的命令配對到前一幀，算出每個命令的位移量，再依進度換成偏移。
//!
//! 配對鍵是 `(obj, op, 該組合在這一幀第幾次出現)`。`obj` 為 `None`、前一幀沒有同鍵、
//! 世代不同（`ObjId` 相等才算同一物件）、位移超過 `INTERP_MAX_PX` 的命令都不內插，直接畫目前位置。

use super::Off;
use rs2_core::{BlitCmd, DrawCmd, ObjId, Op, Pool};
use std::collections::HashMap;

/// 一幀內位移超過這個像素數就不內插，視為瞬移（生成、消失、換位）。
/// 腳本碼 0Ah 一幀走 6 欄（48 像素）是合法上限。試玩後定案（規格 001「補間門檻」）。
pub const INTERP_MAX_PX: i32 = 48;

/// 同一物件在前一幀沒有相同 op 的命令時（受擊閃白的幀：前一幀是遮罩加平面，這一幀只有 `S19025`；閃白後的下一幀相反），
/// 改用「同一物件第一個圖形命令」的位移。沒有這條，閃白期間（玩家被擊中後約 6 幀、每隔一幀）物件會每幀瞬移。
/// 規格 001 的「閃爍不內插、只有位置內插」要求閃白幀也內插位置，只靠 `(obj, op, 次序)` 配對做不到。
/// 設成 `false` 就退回只配對相同 op。
pub const INTERP_CROSS_OP: bool = true;

/// 背景循環長度（位元組）：暫存區每列 80 位元組，同一張圖在 x = 0 與 x = 40 各畫一份，所以水平循環是 40 位元組。
pub const BG_PERIOD_BYTES: i32 = 40;

/// 目前幀相對前一幀的位移，單位是邏輯像素。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Delta {
    pub dx: i32,
    pub dy: i32,
    /// 來源欄（`src_x + shift`）的位移，只有暫存區複製類命令有。
    pub ds: i32,
    /// `S1A287` 的列數變化。
    pub dcx: i32,
    /// 背景層的來源欄在循環內時，這是目前的來源欄（位元組），偏移要依循環長度環繞。
    pub wrap_col: Option<i32>,
}

/// 可內插位置的命令：畫在顯示頁上的圖、複製與填色。暫存區寫入、清頁、血條、數字都不內插。
pub fn interpolable(op: Op) -> bool {
    matches!(
        op,
        Op::S18C0A
            | Op::S18CE2
            | Op::S18D46
            | Op::S18D89
            | Op::S18DCC
            | Op::S18E30
            | Op::S18E73
            | Op::S18EB6
            | Op::S18F1A
            | Op::S18F5D
            | Op::S18FC1
            | Op::S19025
            | Op::S19BA4
            | Op::S19D08
            | Op::S19DA8
            | Op::S1A09F
            | Op::S1A287
    )
}

/// 畫圖庫圖的命令（遮罩、平面、閃白），同一物件的這些命令共用同一組位移。
fn is_image_op(op: Op) -> bool {
    interpolable(op) && !matches!(op, Op::S19BA4 | Op::S19D08 | Op::S19DA8 | Op::S1A09F | Op::S1A287)
}

/// 從暫存區讀來源欄的命令。其他命令的 `src_x`、`shift` 是上一次背景繪製留下的殘值，不能拿來內插。
fn is_copy(op: Op) -> bool {
    matches!(op, Op::S19BA4 | Op::S19D08 | Op::S19DA8 | Op::S1A09F)
}

/// 一對命令的位移。超過門檻或完全沒動回傳 `None`。
pub fn delta(p: &BlitCmd, c: &BlitCmd, pool: Pool) -> Option<Delta> {
    let dx = (c.x as i16 as i32 - p.x as i16 as i32) * 8;
    let dy = c.y as i16 as i32 - p.y as i16 as i32;
    let (mut ds, mut wrap_col) = (0, None);
    if is_copy(c.op) {
        let pc = p.src_x.wrapping_add(p.shift) as i16 as i32;
        let cc = c.src_x.wrapping_add(c.shift) as i16 as i32;
        if pool == Pool::Background && (0..BG_PERIOD_BYTES).contains(&pc) && (0..BG_PERIOD_BYTES).contains(&cc) {
            // 取最短的環繞差：38 → 0 是 +2，不是 -38。
            let half = BG_PERIOD_BYTES / 2;
            ds = ((cc - pc + half).rem_euclid(BG_PERIOD_BYTES) - half) * 8;
            wrap_col = Some(cc);
        } else {
            ds = (cc - pc) * 8;
        }
    }
    let dcx = if c.op == Op::S1A287 { c.cx as i16 as i32 - p.cx as i16 as i32 } else { 0 };
    if dx.abs().max(dy.abs()).max(ds.abs()).max(dcx.abs()) > INTERP_MAX_PX {
        return None;
    }
    if dx == 0 && dy == 0 && ds == 0 && dcx == 0 {
        return None;
    }
    Some(Delta { dx, dy, ds, dcx, wrap_col })
}

/// 每個物件在這一幀的第一個圖形命令（遮罩在平面之前，所以是遮罩或閃白）。
fn first_image_cmds(cmds: &[DrawCmd]) -> HashMap<ObjId, &BlitCmd> {
    let mut m = HashMap::new();
    for d in cmds {
        if let DrawCmd::Blit(b) = d {
            if let (Some(o), true) = (b.obj, is_image_op(b.op)) {
                m.entry(o).or_insert(b);
            }
        }
    }
    m
}

fn finish(dx: i32, dy: i32) -> Option<Delta> {
    if dx.abs().max(dy.abs()) > INTERP_MAX_PX || (dx == 0 && dy == 0) {
        return None;
    }
    Some(Delta { dx, dy, ..Default::default() })
}

/// 目前幀每個命令的位移，與 `cur` 等長。
pub fn pair(prev: &[DrawCmd], cur: &[DrawCmd]) -> Vec<Option<Delta>> {
    pair_with(prev, cur, INTERP_CROSS_OP)
}

/// `cross_op` 為真時，同一物件找不到相同 op 的前一幀命令，改用物件第一個圖形命令的位移。
pub fn pair_with(prev: &[DrawCmd], cur: &[DrawCmd], cross_op: bool) -> Vec<Option<Delta>> {
    let mut by_key: HashMap<(ObjId, Op), Vec<&BlitCmd>> = HashMap::new();
    for d in prev {
        if let DrawCmd::Blit(b) = d {
            if let Some(o) = b.obj {
                by_key.entry((o, b.op)).or_default().push(b);
            }
        }
    }
    let (prev_ref, cur_ref) = if cross_op { (first_image_cmds(prev), first_image_cmds(cur)) } else { Default::default() };
    let mut seen: HashMap<(ObjId, Op), usize> = HashMap::new();
    cur.iter()
        .map(|d| match d {
            DrawCmd::Blit(c) if interpolable(c.op) => {
                let o = c.obj?;
                let key = (o, c.op);
                let n = seen.entry(key).or_insert(0);
                let nth = *n;
                *n += 1;
                match by_key.get(&key).and_then(|v| v.get(nth)) {
                    Some(p) => delta(p, c, o.pool),
                    None if cross_op && is_image_op(c.op) => {
                        let (p, q) = (prev_ref.get(&o)?, cur_ref.get(&o)?);
                        finish((q.x as i16 as i32 - p.x as i16 as i32) * 8, q.y as i16 as i32 - p.y as i16 as i32)
                    }
                    None => None,
                }
            }
            _ => None,
        })
        .collect()
}

/// 進度 `t`（0 為前一幀，1 為目前幀）下的偏移，`scale` 是輸出對邏輯像素的倍率。
/// `t = 1` 時一律是 0，所以輸出與不內插完全相同。
pub fn offsets(d: &Delta, t: f32, scale: i32) -> Off {
    let u = 1.0 - t;
    let s = scale as f32;
    let r = |v: i32| (-(v as f32) * u * s).round() as i32;
    let sox = match d.wrap_col {
        Some(cc) => {
            let per = 8 * BG_PERIOD_BYTES * scale;
            let cur = cc * 8 * scale;
            let new = ((cc * 8) as f32 * s - d.ds as f32 * u * s).round() as i32;
            new.rem_euclid(per) - cur
        }
        None => r(d.ds),
    };
    Off { ox: r(d.dx), oy: r(d.dy), sox, ocx: r(d.dcx) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rs2_core::ShtGroup;

    fn cmd(op: Op, x: u16, y: u16, obj: Option<ObjId>) -> BlitCmd {
        BlitCmd {
            op,
            x,
            y,
            index: 0,
            page: 0,
            scratch: 0,
            src_x: 0,
            src_y: 0,
            shift: 0,
            cx: 0,
            dx: 0,
            text_x: 0,
            text_y: 0,
            sht_group: ShtGroup::Sht1,
            mask_group: ShtGroup::Sht1,
            beg_group: rs2_core::BegGroup::Beg1,
            stride_reg: None,
            obj,
        }
    }

    fn oid(pool: Pool, slot: u8, generation: u32) -> ObjId {
        ObjId { pool, slot, generation }
    }

    fn blit(c: BlitCmd) -> DrawCmd {
        DrawCmd::Blit(c)
    }

    #[test]
    fn pairs_same_object_and_converts_bytes_to_pixels() {
        let o = oid(Pool::Entity, 3, 1);
        let prev = [blit(cmd(Op::S18F5D, 10, 50, Some(o)))];
        let cur = [blit(cmd(Op::S18F5D, 12, 47, Some(o)))];
        let d = pair(&prev, &cur);
        assert_eq!(d[0], Some(Delta { dx: 16, dy: -3, ..Default::default() }));
    }

    #[test]
    fn different_generation_is_not_interpolated() {
        let a = oid(Pool::EnemyBullet, 5, 1);
        let b = oid(Pool::EnemyBullet, 5, 2);
        let prev = [blit(cmd(Op::S18F5D, 10, 50, Some(a)))];
        let cur = [blit(cmd(Op::S18F5D, 11, 51, Some(b)))];
        assert_eq!(pair(&prev, &cur)[0], None, "同槽不同世代是另一個物件");
        let same = [blit(cmd(Op::S18F5D, 11, 51, Some(a)))];
        assert!(pair(&prev, &same)[0].is_some());
    }

    #[test]
    fn missing_in_previous_or_no_obj_or_other_op_is_not_interpolated() {
        let o = oid(Pool::Entity, 1, 1);
        let other = oid(Pool::Entity, 2, 1);
        let prev = [blit(cmd(Op::S18F5D, 10, 50, Some(o))), blit(cmd(Op::S18F5D, 3, 3, None))];
        let cur = [
            blit(cmd(Op::S18F5D, 11, 50, Some(other))),
            blit(cmd(Op::S18F5D, 4, 3, None)),
            blit(cmd(Op::S18CE2, 11, 50, Some(o))),
        ];
        assert_eq!(pair_with(&prev, &cur, false), vec![None, None, None]);
    }

    #[test]
    fn nth_occurrence_pairs_in_order() {
        let o = oid(Pool::Pod, 0, 1);
        let prev = [blit(cmd(Op::S18F5D, 10, 50, Some(o))), blit(cmd(Op::S18F5D, 20, 80, Some(o)))];
        let cur = [blit(cmd(Op::S18F5D, 11, 50, Some(o))), blit(cmd(Op::S18F5D, 20, 82, Some(o)))];
        let d = pair_with(&prev, &cur, false);
        assert_eq!(d[0].unwrap().dx, 8);
        assert_eq!(d[1].unwrap().dy, 2);
        // 前一幀只有一次出現時，第二次沒有對象。
        let d = pair_with(&prev[..1], &cur, false);
        assert!(d[0].is_some() && d[1].is_none());
    }

    #[test]
    fn flash_frames_borrow_the_objects_motion_when_ops_differ() {
        let o = oid(Pool::Entity, 3, 1);
        let other = oid(Pool::Entity, 4, 1);
        // 前一幀：遮罩 (1,10)、平面 (1,11)。這一幀受擊閃白：只有 S19025 (3,14)。下一幀：遮罩 (5,18)、平面 (5,19)。
        let a = [blit(cmd(Op::S18F1A, 1, 10, Some(o))), blit(cmd(Op::S18F5D, 1, 11, Some(o)))];
        let b = [blit(cmd(Op::S19025, 3, 14, Some(o)))];
        let c = [blit(cmd(Op::S18F1A, 5, 18, Some(o))), blit(cmd(Op::S18F5D, 5, 19, Some(o)))];
        // 嚴格配對：閃白幀與閃白後的幀都配不到。
        assert_eq!(pair_with(&a, &b, false), vec![None]);
        assert_eq!(pair_with(&b, &c, false), vec![None, None]);
        // 同物件第一個圖形命令的位移：閃白幀 (16, 4)，下一幀遮罩與平面共用 (16, 4)，兩者相對位置不變。
        assert_eq!(pair_with(&a, &b, true), vec![Some(Delta { dx: 16, dy: 4, ..Default::default() })]);
        let d = pair_with(&b, &c, true);
        assert_eq!(d[0], d[1]);
        assert_eq!(d[0], Some(Delta { dx: 16, dy: 4, ..Default::default() }));
        // 前一幀沒有這個物件，或是別的物件，仍然不內插。
        assert_eq!(pair_with(&[blit(cmd(Op::S19025, 3, 14, Some(other)))], &c, true), vec![None, None]);
        // 位移超過門檻不內插。
        let far = [blit(cmd(Op::S19025, 20, 14, Some(o)))];
        assert_eq!(pair_with(&a, &far, true), vec![None]);
        // 有相同 op 的前一幀命令時，用它自己的位移，不被物件位移取代。
        let a2 = [blit(cmd(Op::S18F1A, 1, 10, Some(o))), blit(cmd(Op::S18F5D, 2, 11, Some(o)))];
        let d = pair_with(&a2, &c, true);
        assert_eq!(d[0].unwrap().dx, 32);
        assert_eq!(d[1].unwrap().dx, 24);
    }

    #[test]
    fn over_threshold_is_not_interpolated() {
        let o = oid(Pool::Entity, 1, 1);
        // 6 欄 = 48 像素在門檻上，7 欄不行。
        let prev = [blit(cmd(Op::S18F5D, 10, 50, Some(o)))];
        assert!(pair(&prev, &[blit(cmd(Op::S18F5D, 16, 50, Some(o)))])[0].is_some());
        assert!(pair(&prev, &[blit(cmd(Op::S18F5D, 17, 50, Some(o)))])[0].is_none());
        assert!(pair(&prev, &[blit(cmd(Op::S18F5D, 10, 99, Some(o)))])[0].is_none());
        // 沒動也不產生位移。
        assert!(pair(&prev, &prev)[0].is_none());
    }

    #[test]
    fn non_display_ops_are_never_interpolated() {
        let o = oid(Pool::Entity, 1, 1);
        for op in [Op::S19ADC, Op::S19CF9, Op::S1929F, Op::S1890D, Op::S1152D] {
            let prev = [blit(cmd(op, 10, 50, Some(o)))];
            let cur = [blit(cmd(op, 11, 50, Some(o)))];
            assert_eq!(pair_with(&prev, &cur, true)[0], None, "{op:?}");
        }
    }

    #[test]
    fn background_shift_wraps_by_period() {
        let o = oid(Pool::Background, 1, 0);
        let mk = |shift: u16| {
            let mut c = cmd(Op::S19BA4, 0, 149, Some(o));
            c.shift = shift;
            blit(c)
        };
        // 波浪每幀 2 位元組（16 像素），38 → 0 環繞。
        let d = pair(&[mk(38)], &[mk(0)])[0].unwrap();
        assert_eq!((d.ds, d.wrap_col), (16, Some(0)));
        let d = pair(&[mk(10)], &[mk(12)])[0].unwrap();
        assert_eq!((d.ds, d.wrap_col), (16, Some(12)));
        // 一般物件的 shift 不環繞。
        let e = oid(Pool::Entity, 1, 1);
        let mut c1 = cmd(Op::S19D08, 0, 0, Some(e));
        c1.shift = 38;
        let c0 = cmd(Op::S19D08, 0, 0, Some(e));
        let d = pair(&[blit(c1)], &[blit(c0)])[0];
        assert_eq!(d, None, "-38 位元組超過門檻");
    }

    #[test]
    fn non_copy_ops_ignore_stale_source_columns() {
        let o = oid(Pool::Entity, 1, 1);
        let mut p = cmd(Op::S18F5D, 10, 50, Some(o));
        p.shift = 7;
        let mut c = cmd(Op::S18F5D, 11, 50, Some(o));
        c.shift = 30;
        let d = pair(&[blit(p)], &[blit(c)])[0].unwrap();
        assert_eq!(d.ds, 0);
    }

    #[test]
    fn offsets_endpoints_and_midpoint() {
        let d = Delta { dx: 16, dy: -3, ..Default::default() };
        let o1 = offsets(&d, 1.0, 1);
        assert_eq!(o1, Off::default(), "t = 1 必須是 0 偏移");
        let o0 = offsets(&d, 0.0, 1);
        assert_eq!((o0.ox, o0.oy), (-16, 3));
        let o5 = offsets(&d, 0.5, 1);
        assert_eq!((o5.ox, o5.oy), (-8, 2)); // 1.5 四捨五入遠離 0
        let h = offsets(&d, 0.5, 4);
        assert_eq!((h.ox, h.oy), (-32, 6));
    }

    #[test]
    fn wrapped_source_offset_stays_inside_period() {
        // 目前欄 0，位移 +16：t = 0 要回到欄 38（304 像素）。
        let d = Delta { ds: 16, wrap_col: Some(0), ..Default::default() };
        assert_eq!(offsets(&d, 0.0, 1).sox, 304);
        assert_eq!(offsets(&d, 1.0, 1).sox, 0);
        assert_eq!(offsets(&d, 0.5, 1).sox, 312);
        assert_eq!(offsets(&d, 0.5, 4).sox, 312 * 4);
        // 欄 12，位移 +16：t = 0 是欄 10（80 像素），偏移 -16。
        let d = Delta { ds: 16, wrap_col: Some(12), ..Default::default() };
        assert_eq!(offsets(&d, 0.0, 1).sox, -16);
    }
}
