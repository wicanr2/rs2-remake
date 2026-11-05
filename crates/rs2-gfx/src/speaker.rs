//! `sub_18DCC` 的 E 分支畫圖時附帶的喇叭事件（`sub_192EA`，`192EAh`）。
//!
//! 每個平面畫一次，每列結尾看來源指標的下一個位元組 `al`：`al & 2Dh` 非 0 就把除數設成
//! `(列尾位址高位元組 << 8) | al` 並開閘門，否則關閘門；接著把除數設成 `2BCh << 剩餘列數`。
//! 指令數照機器碼數，和 `rs2dyn` 的 `Step` 差值相同（規格 003）。

use crate::assets::Assets;
use crate::sink::disp_row;
use rs2_audio::{SpkEv, SpkKind};
use rs2_core::{BlitCmd, Op};
use rs2_formats::BankItem;

/// `sub_187A6` 從 `call` 到第二個 `out 42h`：`call`、`push`、`mov`、`out`、`pop`、`out`、`mov`、`out`。
const PIT_CALL: u32 = 8;
/// `sub_187A6` 的 `retn`。
const PIT_RET: u32 = 1;
/// `sub_187B3`、`sub_187BB` 從 `call` 到 `out 61h`：`call`、`nop`、`in`、`or`、`out`。
const GATE_CALL: u32 = 5;
/// 這兩個函式的 `retn`。
const GATE_RET: u32 = 1;

fn divisor(ax: u16) -> u32 {
    if ax == 0 { 65536 } else { ax as u32 }
}

struct Burst {
    out: Vec<SpkEv>,
    /// 上一個事件之後累積的指令數。
    steps: u32,
    /// 通道 2 目前的除數與閘門。`rs2dyn` 的 `S` 只在閘門的值改變時記一筆。
    gate: bool,
}

impl Burst {
    fn run(&mut self, n: u32) {
        self.steps += n;
    }

    fn pit(&mut self, ax: u16) {
        self.steps += PIT_CALL;
        self.out.push(SpkEv { steps: self.steps, kind: SpkKind::Divisor(divisor(ax)) });
        self.steps = PIT_RET;
    }

    fn gate(&mut self, on: bool) {
        self.steps += GATE_CALL;
        if self.gate != on {
            self.out.push(SpkEv { steps: self.steps, kind: SpkKind::Gate(on) });
            self.gate = on;
            self.steps = 0;
        }
        self.steps += GATE_RET;
    }
}

/// 一張圖的一個平面。`next` 是最後一列之後那個位元組（下一項的高度低位元組）。
fn plane(b: &mut Burst, it: &BankItem, next: u8, c: &BlitCmd, stride_before: u16) -> u16 {
    b.pit(0x0A);
    // sub_18AA4 到 sub_19683 的固定部分：retn 之後。入口的指令數只影響第一個事件之前，不影響相對時間。
    let stride = it.stride;
    let _ = stride_before;
    // sub_19683 跳過 y < 0 的列。
    let mut bx = c.y;
    let mut cx = it.height;
    let mut si = 0u16;
    let mut skipped = 0u32;
    while (bx as i16) < 0 {
        bx = bx.wrapping_add(1);
        cx = cx.wrapping_sub(1);
        si = si.wrapping_add(stride);
        skipped += 1;
    }
    let mut dx = disp_row(bx).wrapping_add(c.page);
    let mut di = dx.wrapping_add(c.x);
    if (cx as i16) <= 0 {
        cx = 1;
        bx = 0x12C;
    }
    b.run(FIRST_ROW_ENTRY + 6 * skipped);
    let rows = cx;
    for r in 0..rows {
        let remaining = rows - r;
        // 19307h：push cx、di、dx，sub_1958E（call 加 6 道），mov ax,dx、add、cmp、jg。
        b.run(3 + 7 + 4);
        let ax = dx.wrapping_add(0x27);
        if (bx as i16) <= 0xBD {
            for col in 0..stride {
                let d = di.wrapping_add(col);
                b.run(if d > ax {
                    7
                } else if d < dx {
                    9
                } else {
                    11
                });
            }
            si = si.wrapping_add(stride);
            let al = it.data.get(si as usize).copied().unwrap_or(next);
            b.run(3); // mov al,[si]、test、jz
            if al & 0x2D != 0 {
                b.pit((ax & 0xFF00) | al as u16);
                b.gate(true);
                b.run(1); // jmp short loc_1933D
            } else {
                b.gate(false);
            }
        }
        // 1933Dh：inc bx、pop、pop、add、add、pop cx，mov ax、shl。
        b.run(6 + 2);
        let shift = (remaining & 0x1F) as u32;
        b.pit((0x2BCu32 << shift) as u16);
        b.run(1); // loop
        bx = bx.wrapping_add(1);
        dx = dx.wrapping_add(0x28);
        di = di.wrapping_add(0x28);
    }
    b.run(2); // mov ax、mov ds
    b.gate(false);
    stride
}

/// `sub_192EA` 第一個 `sub_187A6` 回來之後到第一列之前的指令數：`call sub_18AA4`（`call` 加 19 道），
/// `mov ax`、`mov ds`、`push ds`、`mov ax`、`mov ds`、`mov ax`、`pop ds`、`mov es`，`call sub_19683`（`call` 加 18 道）。
/// y < 0 每列另加 6 道（`19689h` 的迴圈）。
const FIRST_ROW_ENTRY: u32 = 20 + 8 + 19;

/// 這個命令附帶的喇叭事件。不是 `S18DCC`、或圖號超出 `MY` 就是空的。
pub fn speaker_events(assets: &Assets, c: &BlitCmd) -> Vec<SpkEv> {
    if c.op != Op::S18DCC {
        return Vec::new();
    }
    let go = &assets.my;
    let mut b = Burst { out: Vec::new(), steps: 0, gate: false };
    let mut stride = 0;
    for k in 0..4 {
        let Some(it) = go.planes[k].get(c.index as usize) else {
            return Vec::new();
        };
        let next = go.planes[k].get(c.index as usize + 1).map_or(0, |n| n.height as u8);
        stride = plane(&mut b, it, next, c, stride);
    }
    b.out
}
