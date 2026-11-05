//! `State::power_on` 的每個欄位都要等於原版映像裡該變數的初值：從映像的資料段讀出狀態，和 `power_on` 逐領域比。
//! 需要原版檔：`tools/rust.sh test -p rs2-trace --test power_on -- --ignored --nocapture`

use rs2_game::{State, Tables};
use rs2_trace::load_inputs;
use std::path::PathBuf;

/// 兩段 Debug 文字第一個不同處的前後文。
fn first_difference(a: &str, b: &str) -> String {
    let i = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let lo = i.saturating_sub(120);
    let cut = |s: &str| s.chars().skip(lo).take(300).collect::<String>();
    format!("  從第 {i} 個字元起不同\n  映像：…{}\n  初值：…{}", cut(a), cut(b))
}

#[test]
#[ignore]
fn power_on_matches_the_image() {
    let dir = PathBuf::from(std::env::var("RS2_ORIG").expect("缺 RS2_ORIG，用 tools/rust.sh 跑"));
    let inputs = load_inputs(&dir).expect("載入原版");
    let mirror = inputs.mirror();
    let loaded = mirror.load(mirror.base());
    let tables = Tables::parse(&inputs.image).expect("解析表格");
    let want = State::power_on(&tables);
    let mut bad = Vec::new();
    macro_rules! domain {
        ($($f:ident),*) => {$(
            if loaded.$f != want.$f {
                bad.push(format!("{}:\n{}", stringify!($f), first_difference(&format!("{:?}", loaded.$f), &format!("{:?}", want.$f))));
            }
        )*};
    }
    domain!(fx, eb, pb, en, sc, pl, title, flow, scenes, draw_params, sound, tape);
    // 格網：G4 在映像裡是 800 個 0002h，初值是 0（每個關卡進場第一件事就清掉，見 `State::power_on`）；其餘完全相同。
    let (got, exp) = (&loaded.grids, &want.grids);
    assert_eq!(got.g1, exp.g1, "G1");
    assert_eq!(got.g2, exp.g2, "G2");
    let g4 = 0x640;
    assert!(got.terrain[..g4].chunks(2).all(|w| w == [2, 0]), "映像的 G4 是 800 個 0002h");
    assert!(exp.terrain[..g4].iter().all(|&b| b == 0));
    assert_eq!(got.terrain[g4..], exp.terrain[g4..], "G3 與新欄寫入游標");
    assert!(bad.is_empty(), "初值與映像不同：\n{}", bad.join("\n"));
}
