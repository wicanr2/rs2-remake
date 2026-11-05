//! Android 的資料包：原版檔與 HD 素材打成一個檔放進 APK，第一次啟動解到內部儲存。
//! 寫入端是 `tools/package/in_container/pkg_android_pak.py`，格式兩邊要一致。
//!
//! | 欄位 | 內容 |
//! |---|---|
//! | 標頭 | `RS2PAK1\n`，4 位元組檔數（小端） |
//! | 目錄，每檔一筆 | 2 位元組路徑長度、UTF-8 路徑、8 位元組大小、32 位元組 SHA-256 |
//! | 內容 | 依目錄順序接在一起 |
//!
//! 目錄的 SHA-256 當作這個資料包的識別碼，解完寫進 `.installed`。下次啟動識別碼相同就不再解。

use rs2_formats::sha256_hex;
use std::io::Read;
use std::path::{Component, Path};

const MAGIC: &[u8; 8] = b"RS2PAK1\n";
const STAMP: &str = ".installed";
/// 一次讀寫的大小。
const CHUNK: usize = 1 << 20;

struct Entry {
    path: String,
    size: u64,
    sha: [u8; 32],
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn read_exact(r: &mut impl Read, buf: &mut [u8], what: &str) -> Result<(), String> {
    r.read_exact(buf).map_err(|e| format!("資料包讀不到{what}：{e}"))
}

/// 路徑只能是目錄內的相對路徑，不接受 `..` 與絕對路徑。
fn safe_path(p: &str) -> bool {
    !p.is_empty() && Path::new(p).components().all(|c| matches!(c, Component::Normal(_)))
}

/// 把資料包解到 `dest`。已經解過同一份就直接回傳 `Ok(false)`。
/// `progress` 收到 0 到 1 的進度，呼叫端用來畫載入畫面。內容的 SHA-256 不符就中止，不留 `.installed`。
pub fn unpack(r: &mut impl Read, dest: &Path, progress: &mut dyn FnMut(f32)) -> Result<bool, String> {
    let mut magic = [0u8; 8];
    read_exact(r, &mut magic, "標頭")?;
    if &magic != MAGIC {
        return Err("資料包的標頭不對".into());
    }
    let mut n = [0u8; 4];
    read_exact(r, &mut n, "檔數")?;
    let count = u32::from_le_bytes(n);
    let mut index: Vec<u8> = Vec::new();
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut l = [0u8; 2];
        read_exact(r, &mut l, "路徑長度")?;
        let mut path = vec![0u8; u16::from_le_bytes(l) as usize];
        read_exact(r, &mut path, "路徑")?;
        let mut size = [0u8; 8];
        read_exact(r, &mut size, "檔案大小")?;
        let mut sha = [0u8; 32];
        read_exact(r, &mut sha, "雜湊")?;
        index.extend_from_slice(&l);
        index.extend_from_slice(&path);
        index.extend_from_slice(&size);
        index.extend_from_slice(&sha);
        let path = String::from_utf8(path).map_err(|_| "資料包的路徑不是 UTF-8".to_string())?;
        if !safe_path(&path) {
            return Err(format!("資料包的路徑不安全：{path}"));
        }
        entries.push(Entry { path, size: u64::from_le_bytes(size), sha });
    }
    let id = sha256_hex(&index);
    let stamp = dest.join(STAMP);
    if std::fs::read_to_string(&stamp).is_ok_and(|s| s.trim() == id) {
        return Ok(false);
    }
    // 舊版本的檔案不留，換版時素材可能減少。
    if dest.exists() {
        std::fs::remove_dir_all(dest).map_err(|e| format!("清不掉舊資料 {}：{e}", dest.display()))?;
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("建不了 {}：{e}", dest.display()))?;
    let total: u64 = entries.iter().map(|e| e.size).sum::<u64>().max(1);
    let mut done = 0u64;
    let mut buf = vec![0u8; CHUNK];
    for e in &entries {
        let path = dest.join(&e.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| format!("建不了 {}：{err}", parent.display()))?;
        }
        // 檔案都小於 RAM，整個讀進來算雜湊再寫。
        let mut data = Vec::with_capacity(e.size as usize);
        let mut left = e.size;
        while left > 0 {
            let want = left.min(CHUNK as u64) as usize;
            read_exact(r, &mut buf[..want], &format!(" {} 的內容", e.path))?;
            data.extend_from_slice(&buf[..want]);
            left -= want as u64;
            done += want as u64;
            progress(done as f32 / total as f32);
        }
        if sha256_hex(&data) != hex(&e.sha) {
            return Err(format!("{} 的 SHA-256 不符，資料包損壞", e.path));
        }
        std::fs::write(&path, &data).map_err(|err| format!("寫不了 {}：{err}", path.display()))?;
    }
    std::fs::write(&stamp, &id).map_err(|err| format!("寫不了 {}：{err}", stamp.display()))?;
    progress(1.0);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sha(data: &[u8]) -> Vec<u8> {
        let h = sha256_hex(data);
        (0..32).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    fn pak(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&(files.len() as u32).to_le_bytes());
        for (p, d) in files {
            out.extend_from_slice(&(p.len() as u16).to_le_bytes());
            out.extend_from_slice(p.as_bytes());
            out.extend_from_slice(&(d.len() as u64).to_le_bytes());
            out.extend_from_slice(&sha(d));
        }
        for (_, d) in files {
            out.extend_from_slice(d);
        }
        out
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rs2-pak-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn unpacks_verifies_and_skips_next_time() {
        let d = tmp("ok");
        let big = vec![7u8; CHUNK + 123];
        let p = pak(&[("original/S.EXE", b"abc"), ("hd/MY/0001.png", &big), ("empty", b"")]);
        let mut last = 0.0;
        assert_eq!(unpack(&mut &p[..], &d, &mut |f| last = f), Ok(true));
        assert_eq!(last, 1.0);
        assert_eq!(std::fs::read(d.join("original/S.EXE")).unwrap(), b"abc");
        assert_eq!(std::fs::read(d.join("hd/MY/0001.png")).unwrap(), big);
        assert_eq!(std::fs::read(d.join("empty")).unwrap(), b"");
        // 第二次不動任何檔。
        std::fs::write(d.join("original/S.EXE"), b"changed").unwrap();
        assert_eq!(unpack(&mut &p[..], &d, &mut |_| ()), Ok(false));
        assert_eq!(std::fs::read(d.join("original/S.EXE")).unwrap(), b"changed");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_version_replaces_the_old_files() {
        let d = tmp("ver");
        unpack(&mut &pak(&[("a", b"1"), ("old", b"2")])[..], &d, &mut |_| ()).unwrap();
        assert_eq!(unpack(&mut &pak(&[("a", b"9")])[..], &d, &mut |_| ()), Ok(true));
        assert_eq!(std::fs::read(d.join("a")).unwrap(), b"9");
        assert!(!d.join("old").exists(), "舊版本的檔案不留");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn corruption_leaves_no_stamp() {
        let d = tmp("bad");
        let mut p = pak(&[("a", b"hello")]);
        let n = p.len();
        p[n - 1] ^= 1;
        assert!(unpack(&mut &p[..], &d, &mut |_| ()).unwrap_err().contains("SHA-256"));
        assert!(!d.join(STAMP).exists());
        // 修好之後會重解，不會因為殘檔跳過。
        assert_eq!(unpack(&mut &pak(&[("a", b"hello")])[..], &d, &mut |_| ()), Ok(true));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn truncated_and_foreign_files_are_errors() {
        let d = tmp("trunc");
        let p = pak(&[("a", b"hello")]);
        assert!(unpack(&mut &p[..p.len() - 2], &d, &mut |_| ()).is_err());
        assert!(unpack(&mut &b"NOTAPAK\n...."[..], &d, &mut |_| ()).unwrap_err().contains("標頭"));
        assert!(unpack(&mut &p[..4], &d, &mut |_| ()).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn paths_cannot_escape_the_data_directory() {
        let d = tmp("esc");
        for bad in ["../evil", "/abs", "a/../../evil", ""] {
            let p = pak(&[(bad, b"x")]);
            assert!(unpack(&mut &p[..], &d, &mut |_| ()).unwrap_err().contains("不安全"), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
