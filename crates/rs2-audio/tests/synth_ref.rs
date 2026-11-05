//! 規格 003 C4：OPL2 合成與參考實作的比對，標 `#[ignore]`。參考 PCM 不進版控：
//! `audio_write_ops` 寫出每首曲子兩輪的暫存器序列，`tools/audio/nuked_ref.sh A B An Bn` 用 Nuked OPL3 合成，
//! `audio_compare_with_reference` 比較同一份序列的輸出。波形對拍只算品質，行為的斷言來自 `opl_receipt.rs`。

use rs2_audio::{Engine, Opl2, Track};
use std::path::PathBuf;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 執行")))
}

fn engine() -> Engine {
    let o = env_path("RS2_ORIG");
    Engine::new(&std::fs::read(o.join("A.MIX")).unwrap(), &std::fs::read(o.join("B.MIX")).unwrap()).unwrap()
}

/// `n` 結尾的版本遮掉小鼓與踩鈸的 key-on。這兩個聲部的相位含雜訊，Nuked 的雜訊每個運算子走一步，
/// 這裡每個取樣走一步，所以不能逐取樣比。其餘聲部（旋律、大鼓）要逐取樣接近。
const CASES: [(&str, Track, bool, usize); 4] = [
    ("A", Track::A, false, 37 * 49716),
    ("B", Track::B, false, 52 * 49716),
    ("An", Track::A, true, 37 * 49716),
    ("Bn", Track::B, true, 52 * 49716),
];

/// 回傳暫存器序列文字（第一行總取樣數，每行「取樣序 暫存器 值」）與合成結果。
fn render(track: Track, no_noise: bool, n: usize) -> (String, Vec<i16>) {
    let mut e = engine();
    let mut o = Opl2::new();
    let mut ops = format!("{n}\n");
    let mut pcm = Vec::with_capacity(n);
    let mut apply = |i: usize, r: u8, v: u8, o: &mut Opl2| {
        let v = if no_noise && r == 0xBD { v & !0x09 } else { v };
        ops.push_str(&format!("{i} {r} {v}\n"));
        o.write(r, v);
    };
    let mut first = Vec::new();
    e.play(track, &mut |r, v| first.push((r, v)));
    for (r, v) in first {
        apply(0, r, v, &mut o);
    }
    for i in 0..n {
        let mut w = Vec::new();
        e.sample(&mut |r, v| w.push((r, v)));
        for (r, v) in w {
            apply(i, r, v, &mut o);
        }
        pcm.push(o.sample());
    }
    (ops, pcm)
}

#[test]
#[ignore]
fn audio_write_ops() {
    let dir = env_path("RS2_OUT").join("audio");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, track, no_noise, n) in CASES {
        let (ops, _) = render(track, no_noise, n);
        std::fs::write(dir.join(format!("{name}.ops")), ops).unwrap();
    }
}

fn read_pcm(p: &std::path::Path) -> Vec<i16> {
    std::fs::read(p).unwrap_or_else(|e| panic!("讀不到 {}：{e}。先跑 tools/audio/nuked_ref.sh", p.display())).chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
}

#[test]
#[ignore]
fn audio_compare_with_reference() {
    let dir = env_path("RS2_OUT").join("audio");
    for (name, track, no_noise, n) in CASES {
        let (_, a) = render(track, no_noise, n);
        let b = read_pcm(&dir.join(format!("{name}.nuked.pcm")));
        assert_eq!(a.len(), b.len());
        let same = a.iter().zip(&b).filter(|(x, y)| x == y).count();
        let (mut ab, mut aa, mut bb, mut dd) = (0f64, 0f64, 0f64, 0f64);
        for (x, y) in a.iter().zip(&b) {
            let (x, y) = (*x as f64, *y as f64);
            ab += x * y;
            aa += x * x;
            bb += y * y;
            dd += (x - y) * (x - y);
        }
        let corr = ab / (aa * bb).sqrt();
        let snr = 10.0 * (bb / dd.max(1.0)).log10();
        // 每 100 毫秒的能量（RMS）比，含雜訊的整首也要在這個範圍內。
        let w = 4972;
        let mut worst = 0f64;
        for (x, y) in a.chunks(w).zip(b.chunks(w)) {
            let e = |s: &[i16]| (s.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / s.len() as f64).sqrt();
            let (ex, ey) = (e(x), e(y));
            if ey > 100.0 {
                worst = worst.max((20.0 * (ex / ey).log10()).abs());
            }
        }
        println!(
            "{name}：{} 取樣，相同 {:.1}%，相關係數 {corr:.6}，訊雜比 {snr:.1} dB，RMS {:.1} 與 {:.1}，每 100 毫秒能量最大差 {worst:.2} dB",
            a.len(),
            100.0 * same as f64 / a.len() as f64,
            (aa / a.len() as f64).sqrt(),
            (bb / a.len() as f64).sqrt()
        );
        if no_noise || track == Track::A {
            assert!(corr > 0.999, "{name} 相關係數 {corr}");
        }
        assert!(worst < 3.0, "{name} 能量差 {worst}");
    }
}
