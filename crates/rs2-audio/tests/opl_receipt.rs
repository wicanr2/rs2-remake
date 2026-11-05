//! 規格 003 C1：`Driver` 與原版 `ASMDRV` 寫進 OPL2 的每一筆相同，tick 也相同。
//! 需要原版檔與收據，標 `#[ignore]`。`tools/rust.sh verify` 會跑。

use rs2_audio::Mix;
use rs2_audio::oracle::{OplW, Seg, parse_tsv, replay_driver};
use std::path::PathBuf;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 執行")))
}

fn load(name: &str) -> Vec<OplW> {
    let p = env_path("RS2_DOSRUN").join(name).join(format!("audio-{name}.tsv"));
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("讀不到 {}：{e}", p.display()));
    let (o, _) = parse_tsv(&text).unwrap();
    assert!(!o.is_empty(), "{name} 沒有 OPL 寫入");
    o
}

fn mixes() -> (Mix, Mix) {
    let orig = env_path("RS2_ORIG");
    (Mix::parse(&std::fs::read(orig.join("A.MIX")).unwrap()).unwrap(), Mix::parse(&std::fs::read(orig.join("B.MIX")).unwrap()).unwrap())
}

#[test]
#[ignore]
fn c1_opl_writes_match_the_original_driver() {
    let (a, b) = mixes();
    let mut total = 0;
    for name in ["au_title", "au_flow"] {
        let real = load(name);
        let segs = replay_driver(&real, &a, &b).unwrap_or_else(|e| panic!("{name}：{e}"));
        let plays = segs.iter().filter(|(_, s)| matches!(s, Seg::Play(_))).count();
        println!("{name}：比對 {} 筆，{plays} 次播放", real.len());
        assert!(plays > 0);
        total += real.len();
    }
    assert!(total > 20000, "比對筆數 {total}");
}

/// 負對照：曲目改一個音符，比對要在播放時失敗。
#[test]
#[ignore]
fn c1_negative_control_changed_note() {
    let (a, _) = mixes();
    let mut m = a.clone();
    let p = a.stream.iter().position(|&x| x & 0xF0 == 0x90).unwrap();
    m.stream[p + 1] ^= 1;
    let e = replay_driver(&load("au_title"), &m, &m).expect_err("改過音符不該相同");
    assert!(e.contains("播放") || e.contains("AX=2"), "{e}");
    println!("負對照：{e}");
}
