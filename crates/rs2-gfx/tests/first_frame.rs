//! 第一幀前的暫存區 A：`sub_10B65` 的呼叫序列餵給 `VgaSink`，與 dosgolem 的平面傾印逐位元組比對。
//! 需要原版檔與收據，由 `tools/rust.sh test -p rs2-gfx -- --ignored` 跑（環境變數 `RS2_ORIG`、`RS2_DOSRUN`）。

use rs2_core::{BlitCmd, DrawCmd, Op, ShtGroup};
use rs2_gfx::{Assets, VgaSink, locate};
use std::path::PathBuf;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 跑")))
}

fn put(x: u16, y: u16, index: u16) -> DrawCmd {
    DrawCmd::Blit(BlitCmd {
        op: Op::S19ADC,
        x,
        y,
        index,
        page: 0,
        scratch: 0,
        src_x: 0,
        src_y: 0,
        shift: 0,
        cx: 0,
        dx: 0,
        text_x: 0x12,
        text_y: 0xBF,
        sht_group: ShtGroup::Bh1,
        mask_group: ShtGroup::Sht1,
        beg_group: rs2_core::BegGroup::Beg1,
        stride_reg: None,
        obj: None,
    })
}

/// `crates/rs2-core/src/flow.rs` 的 `s10b65`：`BH1` 的五層寫進暫存區 A。
fn s10b65() -> Vec<DrawCmd> {
    let mut v = vec![put(0, 0, 0), put(0x28, 0, 0), put(0, 0x28, 2), put(0x28, 0x28, 2)];
    for i in 0..0x0A {
        v.push(put(4 * i, 0x6E, 1));
    }
    v.extend([put(0, 0xC8, 3), put(0x28, 0xC8, 3), put(0, 0xF3, 4), put(0x28, 0xF3, 4)]);
    v
}

#[test]
#[ignore]
fn scratch_a_matches_dump() {
    let assets = Assets::load(&env_path("RS2_ORIG")).expect("載入圖庫");
    let mut sink = VgaSink::new(assets);
    let cmds = s10b65();
    for c in &cmds {
        sink.apply(c);
    }
    assert_eq!(sink.anomalies(), 0, "{:?}", sink.anomaly_log());
    let dir = env_path("RS2_DOSRUN").join("a0_demo");
    let mut compared = 0usize;
    let mut bad = Vec::new();
    for p in 0..4 {
        let want = std::fs::read(dir.join(format!("demo_entry.p{p}"))).unwrap();
        assert_eq!(want.len(), 0x10000);
        let got = sink.plane(p);
        for o in 0..0x10000usize {
            compared += 1;
            if got[o] != want[o] {
                bad.push(format!("平面 {p} 偏移 {o:04X}（{}）重製 {:02X} 原版 {:02X}", locate(o as u16), got[o], want[o]));
            }
        }
    }
    let nonzero: usize = (0..4).map(|p| sink.plane(p).iter().filter(|&&b| b != 0).count()).sum();
    println!("第一幀暫存區 A 比對 {compared} 位元組，不同 {}，非零 {nonzero}", bad.len());
    assert!(nonzero > 10000, "重建結果幾乎全 0，比對沒有意義");
    assert_eq!(compared, 4 * 0x10000);
    assert!(bad.is_empty(), "不同 {} 位元組，前 20 筆：{:#?}", bad.len(), &bad[..bad.len().min(20)]);
}
