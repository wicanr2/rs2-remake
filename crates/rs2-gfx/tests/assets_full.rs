//! 全部圖庫的載入與結構核對。需要原版檔，由 `tools/rust.sh test -p rs2-gfx -- --ignored` 跑（環境變數 `RS2_ORIG`）。
//! 預期數字來自 `docs/re/02-file-formats.md`「抽取統計」與 `docs/re/12-stages-presentation.md` 第 1 節：
//! 彩色圖庫 15 組共 1,337 張（含 `MAP` 1,019 塊），彩色遮罩 `MY` 68、`SHT1` 31、`SHT2` 21、`SHT3` 43、`SHT4` 35、`SHT5` 3。

use rs2_formats::GoFile;
use rs2_gfx::Assets;
use std::path::PathBuf;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 跑")))
}

fn libs(a: &Assets) -> [(&'static str, &GoFile); 15] {
    [
        ("MY", &a.my),
        ("SHT1", &a.sht1),
        ("SHT2", &a.sht2),
        ("SHT3", &a.sht3),
        ("SHT4", &a.sht4),
        ("SHT5", &a.sht5),
        ("BH1", &a.bh1),
        ("BH2", &a.bh2),
        ("BH4", &a.bh4),
        ("MAP", &a.map),
        ("J1", &a.j1),
        ("J2", &a.j2),
        ("J3", &a.j3),
        ("J4", &a.j4),
        ("BEG1", &a.beg1),
    ]
}

#[test]
#[ignore]
fn all_libraries_load_with_expected_shape() {
    let a = Assets::load(&env_path("RS2_ORIG")).expect("載入全部圖庫");
    let mut total = 0usize;
    for (name, go) in libs(&a) {
        let n = go.planes[0].len();
        assert!(n > 0, "{name} 沒有圖");
        for (k, p) in go.planes.iter().enumerate() {
            assert_eq!(p.len(), n, "{name} 平面區段 {k} 項數不同");
        }
        total += n;
    }
    assert_eq!(total, 1337, "彩色圖庫總張數");
    for (name, want) in [("MY", 68usize), ("SHT1", 31), ("SHT2", 21), ("SHT3", 43), ("SHT4", 35), ("SHT5", 3)] {
        let go = libs(&a).into_iter().find(|(n, _)| *n == name).unwrap().1;
        assert_eq!(go.mask.as_ref().map(|m| m.len()), Some(want), "{name} 遮罩項數");
    }
    for (name, go) in libs(&a) {
        if !name.starts_with("SHT") && name != "MY" {
            assert!(go.mask.is_none(), "{name} 不該有遮罩");
        }
    }
    assert_eq!(a.map.planes[0].len(), 1019);
    assert_eq!(a.beg1.planes[0].len(), 32);
    // MAP 圖塊都是每列 1 位元組、10 列，`sub_196E5` 固定讀 10 位元組。
    for k in 0..4 {
        for (i, it) in a.map.planes[k].iter().enumerate() {
            assert_eq!((it.height, it.stride), (10, 1), "MAP 平面區段 {k} 第 {i} 項");
        }
    }
    // 載入的 19 個檔（15 組圖庫加 `SHT4.GOT` 與 `BEG1.BD2`～`BD4` 的額外檔案）都在 `EXPECTED` 內，雜湊都驗過。
    assert!(a.unverified.is_empty(), "EXPECTED 沒有預期雜湊、因此沒驗證的檔案：{:?}", a.unverified);
}

/// `workspace/extracted/manifest.json` 的 `banks` 區段（`tools/extract_assets.py` 的 Python 實作，與 Rust 解碼不共用程式）：
/// 回傳每個圖庫的（項數，各項的（寬像素，高））。只掃文字，不解析 JSON。
fn manifest_banks() -> std::collections::BTreeMap<String, Vec<(usize, usize)>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workspace/extracted/manifest.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("讀不到 {}：{e}", path.display()));
    let mut out = std::collections::BTreeMap::new();
    let mut in_banks = false;
    let mut cur: Option<String> = None;
    let mut in_sprites = false;
    let (mut w, mut idx) = (0usize, 0usize);
    let num = |s: &str| -> usize { s.trim().trim_end_matches(',').parse().unwrap() };
    for line in text.lines() {
        if !in_banks {
            in_banks = line == " \"banks\": {";
            continue;
        }
        if line == " }," || line == " }" {
            break;
        }
        if line.starts_with("  \"") && line.ends_with("{") {
            let name = line.trim().trim_end_matches('{').trim().trim_end_matches(':').trim_matches('"').to_string();
            out.insert(name.clone(), Vec::new());
            cur = Some(name);
            in_sprites = false;
        } else if line == "   \"sprites\": [" {
            in_sprites = true;
        } else if line == "   ]," || line == "   ]" {
            in_sprites = false;
        } else if !in_sprites {
            // masks、sheets 區段有同名欄位，不收。
        } else if let Some(v) = line.trim().strip_prefix("\"index\":") {
            idx = num(v);
        } else if let Some(v) = line.trim().strip_prefix("\"width\":") {
            w = num(v);
        } else if let Some(v) = line.trim().strip_prefix("\"height\":") {
            let h = num(v);
            let list = out.get_mut(cur.as_ref().unwrap()).unwrap();
            assert_eq!(list.len(), idx, "manifest 的項目順序");
            list.push((w, h));
        }
    }
    out
}

#[test]
#[ignore]
fn all_libraries_match_the_independent_python_extraction() {
    let a = Assets::load(&env_path("RS2_ORIG")).expect("載入全部圖庫");
    let banks = manifest_banks();
    let mut compared = 0usize;
    for (name, go) in libs(&a) {
        let want = banks.get(name).unwrap_or_else(|| panic!("manifest 沒有 {name}"));
        assert_eq!(want.len(), go.planes[0].len(), "{name} 項數");
        for (i, it) in go.planes[0].iter().enumerate() {
            assert_eq!(want[i], (it.stride as usize * 8, it.height as usize), "{name} 第 {i} 項尺寸");
            compared += 1;
        }
    }
    assert_eq!(compared, 1337);
}
