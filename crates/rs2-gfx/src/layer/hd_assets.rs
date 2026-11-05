//! HD 圖片資料夾。圖片用到才載入，解碼結果快取；找不到或尺寸不符就退回原版圖的最近鄰放大。
//!
//! 資料夾結構見 `docs/hd/codex-handoff.md`：
//! `<庫>/<索引>.png`、`<庫>/masks/<索引>.png`、`<庫>/variants/<調色盤>/<索引>.png`，索引四位數補零。
//! 調色盤變體只在原生調色盤不同時才存在，所以「變體檔存在」就等於「這張圖在這個調色盤下要換」。

use super::pal::{Pal, PALETTES};
use crate::assets::Assets;
use crate::png;
use rs2_core::{BegGroup, ShtGroup};
use rs2_formats::{BankItem, GoFile};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// HD 對邏輯像素的倍率。
pub const SCALE: usize = 4;

/// HD 資料夾底下的圖庫，目錄名同 `name()`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lib {
    My,
    Sht1,
    Sht2,
    Sht3,
    Sht4,
    Sht5,
    Bh1,
    Bh2,
    Bh4,
    /// 第三關 8x10 圖塊。
    Map,
    J1,
    J2,
    J3,
    J4,
    Beg1,
}

impl Lib {
    pub const ALL: [Lib; 15] = [
        Lib::My,
        Lib::Sht1,
        Lib::Sht2,
        Lib::Sht3,
        Lib::Sht4,
        Lib::Sht5,
        Lib::Bh1,
        Lib::Bh2,
        Lib::Bh4,
        Lib::Map,
        Lib::J1,
        Lib::J2,
        Lib::J3,
        Lib::J4,
        Lib::Beg1,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Lib::My => "MY",
            Lib::Sht1 => "SHT1",
            Lib::Sht2 => "SHT2",
            Lib::Sht3 => "SHT3",
            Lib::Sht4 => "SHT4",
            Lib::Sht5 => "SHT5",
            Lib::Bh1 => "BH1",
            Lib::Bh2 => "BH2",
            Lib::Bh4 => "BH4",
            Lib::Map => "MAP",
            Lib::J1 => "J1",
            Lib::J2 => "J2",
            Lib::J3 => "J3",
            Lib::J4 => "J4",
            Lib::Beg1 => "BEG1",
        }
    }

    pub fn go(self, a: &Assets) -> &GoFile {
        match self {
            Lib::My => &a.my,
            Lib::Sht1 => &a.sht1,
            Lib::Sht2 => &a.sht2,
            Lib::Sht3 => &a.sht3,
            Lib::Sht4 => &a.sht4,
            Lib::Sht5 => &a.sht5,
            Lib::Bh1 => &a.bh1,
            Lib::Bh2 => &a.bh2,
            Lib::Bh4 => &a.bh4,
            Lib::Map => &a.map,
            Lib::J1 => &a.j1,
            Lib::J2 => &a.j2,
            Lib::J3 => &a.j3,
            Lib::J4 => &a.j4,
            Lib::Beg1 => &a.beg1,
        }
    }

    /// `ShtGroup`（`sht_group` 或 `mask_group`）對應的圖庫。
    pub fn from_sht(g: ShtGroup) -> Lib {
        match g {
            ShtGroup::Bh1 => Lib::Bh1,
            ShtGroup::Bh2 => Lib::Bh2,
            ShtGroup::Bh4 => Lib::Bh4,
            ShtGroup::Sht1 => Lib::Sht1,
            ShtGroup::Sht2 => Lib::Sht2,
            ShtGroup::Sht3 => Lib::Sht3,
            ShtGroup::Sht4 => Lib::Sht4,
            ShtGroup::Sht5 => Lib::Sht5,
        }
    }

    pub fn from_beg(g: BegGroup) -> Lib {
        match g {
            BegGroup::Beg1 => Lib::Beg1,
            BegGroup::J1 => Lib::J1,
            BegGroup::J2 => Lib::J2,
            BegGroup::J3 => Lib::J3,
            BegGroup::J4 => Lib::J4,
        }
    }
}

/// 依場景預載的圖庫與調色盤變體。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HdSet {
    /// 標題、開場動畫與選單：`MY`、`BEG1`。
    Title,
    /// 第 1 至 4 關：`MY`、該關的 `SHT`、背景（`BH`，第三關是 `MAP`）。
    Stage(u8),
    /// 關間過場 `J1` 至 `J4`（參數 1 至 4）。
    Cutscene(u8),
    /// 片尾：`MY`、`SHT5`。
    Ending,
}

impl HdSet {
    /// 場景用到的圖庫。不認得的關卡編號回傳空。
    pub fn libs(self) -> Vec<Lib> {
        match self {
            HdSet::Title => vec![Lib::My, Lib::Beg1],
            HdSet::Stage(1) => vec![Lib::My, Lib::Sht1, Lib::Bh1],
            HdSet::Stage(2) => vec![Lib::My, Lib::Sht2, Lib::Bh2],
            HdSet::Stage(3) => vec![Lib::My, Lib::Sht3, Lib::Map],
            HdSet::Stage(4) => vec![Lib::My, Lib::Sht4, Lib::Bh4],
            HdSet::Cutscene(n @ 1..=4) => vec![Lib::My, [Lib::J1, Lib::J2, Lib::J3, Lib::J4][n as usize - 1]],
            HdSet::Ending => vec![Lib::My, Lib::Sht5],
            _ => Vec::new(),
        }
    }

    /// 場景用到的調色盤名稱（`PALETTES`）。第三關的 DAC 表與 `stage2_c` 相同，辨識結果是 `stage2_c`。
    pub fn palettes(self) -> &'static [&'static str] {
        match self {
            HdSet::Title => &["title_d", "title_e", "menu", "beg1_a", "beg1_b"],
            HdSet::Stage(1) => &["stage1", "stage1_boss"],
            HdSet::Stage(2) => &["stage2", "stage2_b", "stage2_c"],
            HdSet::Stage(3) => &["stage2_c"],
            HdSet::Stage(4) => &["stage4"],
            HdSet::Cutscene(1) => &["j1"],
            HdSet::Cutscene(2) => &["j2"],
            HdSet::Cutscene(3) => &["j3"],
            HdSet::Cutscene(4) => &["j4"],
            HdSet::Ending => &["ending_a", "ending_b"],
            _ => &[],
        }
    }
}

/// RGBA 圖，單位是 HD 像素。
pub struct HdImg {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

/// 單通道遮罩，255 保留背景，0 清掉。
pub struct HdMask {
    pub w: usize,
    pub h: usize,
    pub v: Vec<u8>,
}

/// 無調色盤變體的鍵。
const NO_PAL: u16 = 0xFFFF;

pub struct HdAssets {
    dir: PathBuf,
    images: HashMap<(Lib, u16, u16), Option<Arc<HdImg>>>,
    masks: HashMap<(Lib, u16), Option<Arc<HdMask>>>,
    fallback_img: HashMap<(Lib, u16, u64), Option<Arc<HdImg>>>,
    fallback_mask: HashMap<(Lib, u16), Option<Arc<HdMask>>>,
    missing: Vec<String>,
    loaded: Vec<String>,
}

impl HdAssets {
    /// 開啟 HD 資料夾。只檢查它是目錄且至少有一個圖庫子目錄（`Lib::ALL` 的名稱），圖片到用時才讀。
    pub fn open(dir: &Path) -> Result<HdAssets, String> {
        if !dir.is_dir() {
            return Err(format!("HD 資料夾不存在或不是目錄：{}", dir.display()));
        }
        if !Lib::ALL.iter().any(|l| dir.join(l.name()).is_dir()) {
            return Err(format!("{} 底下沒有 MY、SHT1 等任何一個圖庫目錄", dir.display()));
        }
        Ok(HdAssets {
            dir: dir.to_path_buf(),
            images: HashMap::new(),
            masks: HashMap::new(),
            fallback_img: HashMap::new(),
            fallback_mask: HashMap::new(),
            missing: Vec::new(),
            loaded: Vec::new(),
        })
    }

    /// 成功載入的 HD 檔（相對路徑，依載入順序），診斷用，例如確認調色盤變體在切換之後才被用到。
    pub fn loaded(&self) -> &[String] {
        &self.loaded
    }

    /// 缺的 HD 圖（含解碼失敗與尺寸不符），每項只列一次，依發現順序。
    pub fn missing(&self) -> &[String] {
        &self.missing
    }

    fn note_missing(&mut self, s: String) {
        if !self.missing.contains(&s) {
            self.missing.push(s);
        }
    }

    fn load_png(&mut self, rel: &str, want_w: usize, want_h: usize) -> Option<png::Image> {
        let path = self.dir.join(rel);
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.note_missing(rel.to_string());
                return None;
            }
            Err(e) => {
                self.note_missing(format!("{rel}（讀取失敗：{e}）"));
                return None;
            }
        };
        match png::decode(&data) {
            Ok(img) if img.w == want_w && img.h == want_h => {
                self.loaded.push(rel.to_string());
                Some(img)
            }
            Ok(img) => {
                self.note_missing(format!("{rel}（尺寸 {}x{}，預期 {want_w}x{want_h}）", img.w, img.h));
                None
            }
            Err(e) => {
                self.note_missing(format!("{rel}（解碼失敗：{e}）"));
                None
            }
        }
    }

    /// 取圖。先試調色盤變體，再試基本圖，最後用原版圖放大。`it` 是原版圖庫項，決定預期尺寸。
    pub fn image(&mut self, lib: Lib, index: u16, pal: &Pal, assets: &Assets) -> Option<Arc<HdImg>> {
        let go = lib.go(assets);
        let it = go.planes[0].get(index as usize)?;
        let (ew, eh) = (it.stride as usize * 8 * SCALE, it.height as usize * SCALE);
        if let Some(n) = pal.name {
            let key = (lib, index, n as u16);
            if !self.images.contains_key(&key) {
                let rel = format!("{}/variants/{}/{:04}.png", lib.name(), PALETTES[n].0, index);
                // 變體不存在是常態（原生調色盤相同），不算缺圖。
                let exists = self.dir.join(&rel).is_file();
                let v = if exists { self.load_png(&rel, ew, eh).map(|i| Arc::new(HdImg { w: i.w, h: i.h, rgba: i.rgba })) } else { None };
                self.images.insert(key, v);
            }
            if let Some(Some(v)) = self.images.get(&key) {
                return Some(v.clone());
            }
        }
        let key = (lib, index, NO_PAL);
        if !self.images.contains_key(&key) {
            let rel = format!("{}/{:04}.png", lib.name(), index);
            let v = self.load_png(&rel, ew, eh).map(|i| Arc::new(HdImg { w: i.w, h: i.h, rgba: i.rgba }));
            self.images.insert(key, v);
        }
        if let Some(Some(v)) = self.images.get(&key) {
            return Some(v.clone());
        }
        // 退回原版圖，依目前調色盤上色。
        let fkey = (lib, index, pal.visible_key());
        if !self.fallback_img.contains_key(&fkey) {
            let f = fallback_image(go, index as usize, &pal.visible8()).map(Arc::new);
            self.fallback_img.insert(fkey, f);
        }
        self.fallback_img.get(&fkey).cloned().flatten()
    }

    /// 取遮罩。`lib` 要有遮罩區段（`MY`、`SHT1` 至 `SHT5`），沒有的庫回傳 `None`。
    pub fn mask(&mut self, lib: Lib, index: u16, assets: &Assets) -> Option<Arc<HdMask>> {
        let go = lib.go(assets);
        let it = go.mask.as_ref()?.get(index as usize)?;
        let (ew, eh) = (it.stride as usize * 8 * SCALE, it.height as usize * SCALE);
        let key = (lib, index);
        if !self.masks.contains_key(&key) {
            let rel = format!("{}/masks/{:04}.png", lib.name(), index);
            let v = self.load_png(&rel, ew, eh).map(|i| Arc::new(HdMask { w: i.w, h: i.h, v: i.rgba.chunks_exact(4).map(|p| p[0]).collect() }));
            self.masks.insert(key, v);
        }
        if let Some(Some(v)) = self.masks.get(&key) {
            return Some(v.clone());
        }
        if !self.fallback_mask.contains_key(&key) {
            self.fallback_mask.insert(key, fallback_mask(it).map(Arc::new));
        }
        self.fallback_mask.get(&key).cloned().flatten()
    }
}

/// 預載工作的種類。
#[derive(Clone, Copy)]
enum JobKind {
    /// `key.2` 是調色盤下標，或 `NO_PAL`。
    Image((Lib, u16, u16)),
    Mask((Lib, u16)),
}

struct Job {
    kind: JobKind,
    rel: String,
    w: usize,
    h: usize,
}

/// 讀檔並解碼，錯誤訊息的格式與 `HdAssets::load_png` 相同（缺圖清單的文字要一致）。
fn read_png_file(dir: &Path, job: &Job) -> Result<png::Image, String> {
    let data = match std::fs::read(dir.join(&job.rel)) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(job.rel.clone()),
        Err(e) => return Err(format!("{}（讀取失敗：{e}）", job.rel)),
    };
    match png::decode(&data) {
        Ok(img) if img.w == job.w && img.h == job.h => Ok(img),
        Ok(img) => Err(format!("{}（尺寸 {}x{}，預期 {}x{}）", job.rel, img.w, img.h, job.w, job.h)),
        Err(e) => Err(format!("{}（解碼失敗：{e}）", job.rel)),
    }
}

impl HdAssets {
    /// 預載第一關會用到的 HD 檔，等於 `preload_set(HdSet::Stage(1))`：`MY`、`SHT1`、`BH1` 每個圖庫項的基本圖、
    /// `stage1` 與 `stage1_boss` 調色盤變體（檔案存在才算），以及 `MY`、`SHT1` 的遮罩。
    /// 解碼結果進快取，之後 `image`、`mask` 直接命中，缺圖與尺寸不符照常記進 `missing`。已在快取裡的不重做。
    ///
    /// `threads` 個執行緒平行讀檔解碼（至少 1）。回傳 `(成功解碼的檔數, 解碼後 RGBA 與遮罩的位元組數)`。
    /// 可以在背景執行緒呼叫，完成後再把 `HdAssets` 交給 `LayerSink::new`。
    pub fn preload(&mut self, assets: &Assets, threads: usize) -> (usize, usize) {
        self.preload_set(assets, HdSet::Stage(1), threads)
    }

    /// 依場景預載，內容見 `HdSet`。可以對不同場景連續呼叫，已在快取裡的不重做。
    pub fn preload_set(&mut self, assets: &Assets, set: HdSet, threads: usize) -> (usize, usize) {
        self.preload_libs(assets, &set.libs(), set.palettes(), threads)
    }

    /// 預載指定圖庫的基本圖、遮罩與指定調色盤名稱的變體，細節同 `preload`。
    pub fn preload_libs(&mut self, assets: &Assets, libs: &[Lib], palettes: &[&str], threads: usize) -> (usize, usize) {
        let pal_idx: Vec<u16> = palettes
            .iter()
            .filter_map(|n| PALETTES.iter().position(|p| p.0 == *n))
            .map(|i| i as u16)
            .collect();
        // 第一階段：列出要做的檔。
        let mut jobs: Vec<Job> = Vec::new();
        for &lib in libs {
            let go = lib.go(assets);
            for (index, it) in go.planes[0].iter().enumerate() {
                let index = index as u16;
                let (w, h) = (it.stride as usize * 8 * SCALE, it.height as usize * SCALE);
                for &n in &pal_idx {
                    let key = (lib, index, n);
                    if self.images.contains_key(&key) {
                        continue;
                    }
                    let rel = format!("{}/variants/{}/{:04}.png", lib.name(), PALETTES[n as usize].0, index);
                    if self.dir.join(&rel).is_file() {
                        jobs.push(Job { kind: JobKind::Image(key), rel, w, h });
                    } else {
                        // 變體不存在是常態，不算缺圖。
                        self.images.insert(key, None);
                    }
                }
                let key = (lib, index, NO_PAL);
                if !self.images.contains_key(&key) {
                    jobs.push(Job { kind: JobKind::Image(key), rel: format!("{}/{:04}.png", lib.name(), index), w, h });
                }
            }
            if let Some(masks) = go.mask.as_ref() {
                for (index, it) in masks.iter().enumerate() {
                    let key = (lib, index as u16);
                    if !self.masks.contains_key(&key) {
                        let (w, h) = (it.stride as usize * 8 * SCALE, it.height as usize * SCALE);
                        jobs.push(Job { kind: JobKind::Mask(key), rel: format!("{}/masks/{:04}.png", lib.name(), index), w, h });
                    }
                }
            }
        }
        // 第二階段：平行解碼。
        let next = std::sync::atomic::AtomicUsize::new(0);
        let done: std::sync::Mutex<Vec<(usize, Result<png::Image, String>)>> = std::sync::Mutex::new(Vec::with_capacity(jobs.len()));
        let dir = self.dir.as_path();
        std::thread::scope(|s| {
            for _ in 0..threads.max(1) {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(job) = jobs.get(i) else { break };
                        let r = read_png_file(dir, job);
                        done.lock().unwrap_or_else(|e| e.into_inner()).push((i, r));
                    }
                });
            }
        });
        let mut results = done.into_inner().unwrap_or_else(|e| e.into_inner());
        results.sort_by_key(|r| r.0);
        // 第三階段：依工作順序放進快取，缺圖清單的順序固定。
        let (mut files, mut bytes) = (0usize, 0usize);
        for (i, r) in results {
            let job = &jobs[i];
            match (job.kind, r) {
                (JobKind::Image(key), Ok(img)) => {
                    self.loaded.push(job.rel.clone());
                    files += 1;
                    bytes += img.rgba.len();
                    self.images.insert(key, Some(Arc::new(HdImg { w: img.w, h: img.h, rgba: img.rgba })));
                }
                (JobKind::Mask(key), Ok(img)) => {
                    self.loaded.push(job.rel.clone());
                    files += 1;
                    let v: Vec<u8> = img.rgba.chunks_exact(4).map(|p| p[0]).collect();
                    bytes += v.len();
                    self.masks.insert(key, Some(Arc::new(HdMask { w: img.w, h: img.h, v })));
                }
                (JobKind::Image(key), Err(note)) => {
                    self.note_missing(note);
                    self.images.insert(key, None);
                }
                (JobKind::Mask(key), Err(note)) => {
                    self.note_missing(note);
                    self.masks.insert(key, None);
                }
            }
        }
        (files, bytes)
    }
}

/// 原版圖項放大 `SCALE` 倍。色號 0 透明，其餘不透明；外框畫洋紅標記缺圖。
fn fallback_image(go: &GoFile, index: usize, colors: &[[u8; 3]; 16]) -> Option<HdImg> {
    let p0 = go.planes[0].get(index)?;
    let (w, h) = (p0.stride as usize * 8, p0.height as usize);
    let items: [Option<&BankItem>; 4] = [0, 1, 2, 3].map(|k| go.planes[k].get(index));
    let (dw, dh) = (w * SCALE, h * SCALE);
    let mut rgba = vec![0u8; dw * dh * 4];
    for y in 0..h {
        for x in 0..w {
            let mut color = 0usize;
            for (k, it) in items.iter().enumerate() {
                if let Some(it) = it {
                    if it.stride == p0.stride && y < it.height as usize {
                        let b = it.data.get(y * it.stride as usize + x / 8).copied().unwrap_or(0);
                        color |= (((b >> (7 - x % 8)) & 1) as usize) << (3 - k);
                    }
                }
            }
            if color == 0 {
                continue;
            }
            let c = colors[color];
            for dy in 0..SCALE {
                for dx in 0..SCALE {
                    let o = ((y * SCALE + dy) * dw + x * SCALE + dx) * 4;
                    rgba[o..o + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                }
            }
        }
    }
    // 缺圖標記：2 個 HD 像素寬的洋紅外框。
    for y in 0..dh {
        for x in 0..dw {
            if x < 2 || y < 2 || x + 2 >= dw || y + 2 >= dh {
                let o = (y * dw + x) * 4;
                rgba[o..o + 4].copy_from_slice(&[255, 0, 255, 255]);
            }
        }
    }
    Some(HdImg { w: dw, h: dh, rgba })
}

/// 原版 1 位元遮罩放大：位元 1（保留）是 255。
fn fallback_mask(it: &BankItem) -> Option<HdMask> {
    let (w, h) = (it.stride as usize * 8, it.height as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let (dw, dh) = (w * SCALE, h * SCALE);
    let mut v = vec![0u8; dw * dh];
    for y in 0..dh {
        for x in 0..dw {
            let (ly, lx) = (y / SCALE, x / SCALE);
            let b = it.data.get(ly * it.stride as usize + lx / 8).copied().unwrap_or(0);
            if (b >> (7 - lx % 8)) & 1 != 0 {
                v[y * dw + x] = 255;
            }
        }
    }
    Some(HdMask { w: dw, h: dh, v })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(h: u16, stride: u16, data: &[u8]) -> BankItem {
        BankItem { height: h, stride, data: data.to_vec() }
    }

    #[test]
    fn open_rejects_missing_or_empty_dir() {
        assert!(HdAssets::open(Path::new("/nonexistent-hd-dir-for-test")).is_err());
        let d = std::env::temp_dir().join(format!("rs2-hd-open-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert!(HdAssets::open(&d).is_err());
        std::fs::create_dir_all(d.join("MY")).unwrap();
        assert!(HdAssets::open(&d).is_ok());
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn solid_png(w: usize, h: usize, px: [u8; 4]) -> Vec<u8> {
        png::encode_rgba(w, h, &px.repeat(w * h))
    }

    #[test]
    fn preload_fills_the_cache_and_reports_missing() {
        let d = std::env::temp_dir().join(format!("rs2-hd-preload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("MY/masks")).unwrap();
        std::fs::create_dir_all(d.join("MY/variants/stage1_boss")).unwrap();
        std::fs::create_dir_all(d.join("SHT1")).unwrap();
        // 圖項 8x2 邏輯像素，HD 32x8。
        std::fs::write(d.join("MY/0000.png"), solid_png(32, 8, [1, 2, 3, 255])).unwrap();
        std::fs::write(d.join("MY/masks/0000.png"), solid_png(32, 8, [200, 200, 200, 255])).unwrap();
        std::fs::write(d.join("MY/variants/stage1_boss/0000.png"), solid_png(32, 8, [9, 9, 9, 255])).unwrap();
        let bank = || [vec![item(2, 1, &[0, 0])], vec![item(2, 1, &[0, 0])], vec![item(2, 1, &[0, 0])], vec![item(2, 1, &[0, 0])]];
        let assets = Assets {
            my: GoFile { mask: Some(vec![item(2, 1, &[0xFF, 0xFF])]), planes: bank() },
            sht1: GoFile { mask: None, planes: bank() },
            ..Assets::empty()
        };
        let mut hd = HdAssets::open(&d).unwrap();
        // MY 的基本圖、遮罩、stage1_boss 變體各一個；SHT1 的圖沒有檔，列為缺圖；SHT1 沒有 stage1 變體檔是常態。
        assert_eq!(hd.preload(&assets, 3), (3, 32 * 8 * 4 + 32 * 8 + 32 * 8 * 4));
        assert_eq!(hd.missing(), ["SHT1/0000.png".to_string()]);
        assert_eq!(hd.loaded().len(), 3);
        // 已在快取：不再做事。
        assert_eq!(hd.preload(&assets, 1), (0, 0));
        assert_eq!(hd.missing().len(), 1);
        // 預載之後刪檔，image 與 mask 仍從快取取得。
        std::fs::remove_dir_all(d.join("MY")).unwrap();
        let boss = PALETTES.iter().position(|p| p.0 == "stage1_boss").unwrap();
        let mut pal = Pal::new();
        pal.name = Some(boss);
        assert_eq!(&hd.image(Lib::My, 0, &pal, &assets).unwrap().rgba[0..4], &[9, 9, 9, 255]);
        pal.name = None;
        assert_eq!(&hd.image(Lib::My, 0, &pal, &assets).unwrap().rgba[0..4], &[1, 2, 3, 255]);
        assert_eq!(hd.mask(Lib::My, 0, &assets).unwrap().v[0], 200);
        // 缺圖退回原版圖放大，缺圖清單不重複。
        assert!(hd.image(Lib::Sht1, 0, &pal, &assets).is_some());
        assert_eq!(hd.missing().len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn fallback_scales_and_marks() {
        // 8x2 的圖，兩列各一位元組：平面 3 與平面 0 組成色號。
        let go = GoFile {
            mask: None,
            planes: [
                vec![item(2, 1, &[0b1000_0000, 0])],
                vec![item(2, 1, &[0, 0])],
                vec![item(2, 1, &[0, 0])],
                vec![item(2, 1, &[0b1000_0000, 0])],
            ],
        };
        let mut colors = [[0u8; 3]; 16];
        colors[9] = [10, 20, 30];
        let img = fallback_image(&go, 0, &colors).unwrap();
        assert_eq!((img.w, img.h), (32, 8));
        // 左上角被外框蓋住；(4,4) 是第 1 個邏輯像素內部（邏輯 (1,1) 為 0 所以透明）
        assert_eq!(&img.rgba[0..4], &[255, 0, 255, 255]);
        // 邏輯像素 (0,0) 色號 9，內部 HD 像素 (2,2) 不在外框內
        let o = (2 * 32 + 2) * 4;
        assert_eq!(&img.rgba[o..o + 4], &[10, 20, 30, 255]);
        // 色號 0 的位置透明
        let o = (4 * 32 + 12) * 4;
        assert_eq!(img.rgba[o + 3], 0);
    }

    #[test]
    fn fallback_mask_scales_bits() {
        let m = fallback_mask(&item(1, 1, &[0b1010_0000])).unwrap();
        assert_eq!((m.w, m.h), (32, 4));
        assert_eq!(m.v[0], 255);
        assert_eq!(m.v[SCALE], 0);
        assert_eq!(m.v[2 * SCALE], 255);
    }
}
