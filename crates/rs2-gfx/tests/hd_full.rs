//! 全部場景的 HD 圖庫預載：每個圖庫項、遮罩與調色盤變體都要存在，尺寸是原版的 4 倍。
//! 需要原版檔與 HD 資料夾，由 `tools/rust.sh test -p rs2-gfx -- --ignored` 跑（`RS2_ORIG`、`RS2_HD`，
//! 後者在 `workspace/hd/assets` 存在時由 `tools/rust.sh` 掛入）。

use rs2_gfx::{Assets, HdAssets, HdSet};
use std::path::PathBuf;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 跑")))
}

#[test]
#[ignore]
fn every_scene_preloads_without_missing_or_wrong_sized_files() {
    let assets = Assets::load(&env_path("RS2_ORIG")).expect("載入圖庫");
    let mut hd = HdAssets::open(&env_path("RS2_HD")).expect("開 HD 資料夾");
    let sets = [
        HdSet::Title,
        HdSet::Stage(1),
        HdSet::Stage(2),
        HdSet::Stage(3),
        HdSet::Stage(4),
        HdSet::Cutscene(1),
        HdSet::Cutscene(2),
        HdSet::Cutscene(3),
        HdSet::Cutscene(4),
        HdSet::Ending,
    ];
    let mut total_files = 0usize;
    for set in sets {
        let (files, bytes) = hd.preload_set(&assets, set, 4);
        println!("{set:?}：解碼 {files} 個檔，{} MB", bytes / (1 << 20));
        assert!(files > 0, "{set:?} 沒有解碼任何檔");
        total_files += files;
    }
    println!("共 {total_files} 個檔；缺圖或尺寸不符 {} 個", hd.missing().len());
    assert!(hd.missing().is_empty(), "{:#?}", hd.missing());
}
