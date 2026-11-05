//! 規格 003 C2、C3：全流程收據裡，重製的音樂開關與喇叭事件和原版落在同一次等待。
//! 需要原版檔與收據，標 `#[ignore]`。收據同時有 `waittrace` 與 `audiotrace`。

use rs2_audio::oracle::{Seg, SpkLine, parse_tsv, replay_driver};
use rs2_audio::{Mix, SpkEv, SpkKind, Track};
use rs2_core::{AudioCmd, DrawCmd, MixFile};
use rs2_gfx::Assets;
use rs2_gfx::speaker::speaker_events;
use rs2_trace::wait::{ReplayEvent, ReplayOpts, Verdict, load_receipt, replay_observed};
use rs2_trace::{Inputs, load_inputs};
use std::path::PathBuf;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 執行")))
}

fn setup() -> (Inputs, Assets, Mix, Mix) {
    let dir = env_path("RS2_ORIG");
    let inputs = load_inputs(&dir).expect("載入原版");
    let assets = Assets::load(&dir).expect("載入圖庫");
    let a = Mix::parse(&inputs.files["A.MIX"]).unwrap();
    let b = Mix::parse(&inputs.files["B.MIX"]).unwrap();
    (inputs, assets, a, b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Music {
    Play(Track),
    Stop,
}

fn check(name: &str, inputs: &Inputs, assets: &Assets, a: &Mix, b: &Mix) -> (usize, usize) {
    let dir = env_path("RS2_DOSRUN").join(name);
    let rc = load_receipt(&dir).unwrap_or_else(|e| panic!("{name}：{e}"));
    let steps: Vec<u64> = rc.rows.iter().map(|r| r.step.unwrap_or_else(|| panic!("{name} 的收據沒有 step 欄位"))).collect();
    let last = *steps.last().unwrap();
    let row_of = |s: u64| steps.partition_point(|&x| x < s);
    let text = std::fs::read_to_string(dir.join(format!("audio-{name}.tsv"))).unwrap_or_else(|e| panic!("{name}：{e}"));
    let (opl, spk) = parse_tsv(&text).unwrap();

    // 原版：每個驅動呼叫的位置。replay_driver 同時比對全部寫入（C1）。
    let segs = replay_driver(&opl, a, b).unwrap_or_else(|e| panic!("{name}：{e}"));
    let real_music: Vec<(usize, Music)> = segs
        .iter()
        .filter(|(at, _)| opl[*at].step <= last)
        .filter_map(|&(at, s)| {
            let m = match s {
                Seg::Play(t) => Music::Play(t),
                Seg::Stop => Music::Stop,
                Seg::Cold => return None,
            };
            Some((row_of(opl[at].step), m))
        })
        .collect();
    let real_spk: Vec<SpkLine> = spk.iter().copied().filter(|s| s.step() <= last).collect();

    // 重製：同一份輸入，每一步的音訊命令與喇叭事件。
    let mut music: Vec<(usize, Music)> = Vec::new();
    let mut bursts: Vec<(usize, bool, Vec<SpkEv>, String)> = Vec::new();
    let mut on = true;
    let opts = ReplayOpts { ds_report: false, locate_root: None, boot: false };
    let v = replay_observed(inputs, assets, &rc, &opts, &mut |ev| {
        if let ReplayEvent::StepAt { draws, audio, next_row, .. } = ev {
            for c in audio {
                match *c {
                    AudioCmd::Play(MixFile::A) => music.push((next_row, Music::Play(Track::A))),
                    AudioCmd::Play(MixFile::B) => music.push((next_row, Music::Play(Track::B))),
                    AudioCmd::Stop => music.push((next_row, Music::Stop)),
                    AudioCmd::Speaker(b) => on = b,
                }
            }
            for d in draws {
                if let DrawCmd::Blit(c) = d {
                    let e = speaker_events(assets, c);
                    if !e.is_empty() {
                        let what = format!("index={} x={} y={} page={:04X}", c.index, c.x, c.y, c.page);
                        bursts.push((next_row, on, e, what));
                    }
                }
            }
        }
    });
    assert!(matches!(v, Verdict::Pass { .. }), "{name} 的畫面對拍沒有通過");

    music.retain(|(r, _)| *r < steps.len());
    assert_eq!(music, real_music, "{name} 的音樂開關不同");

    // 喇叭：把重製的事件攤平，靜音時 sub_187B3、sub_187BB 是 retn，只有除數。
    let mut i = 0usize;
    let mut same_steps = 0usize;
    for (row, on, ev, what) in &bursts {
        if *row >= steps.len() {
            break;
        }
        for (k, e) in ev.iter().enumerate() {
            if !*on && matches!(e.kind, SpkKind::Gate(_)) {
                continue;
            }
            let r = real_spk.get(i).unwrap_or_else(|| panic!("{name}：實錄喇叭事件在第 {i} 筆結束，重製還有（第 {row} 筆等待）"));
            let same = match (e.kind, *r) {
                (SpkKind::Divisor(d), SpkLine::Divisor { div, .. }) => d == div,
                (SpkKind::Gate(g), SpkLine::Gate { on, .. }) => g == on,
                _ => false,
            };
            assert!(same, "{name} 第 {i} 筆喇叭事件：重製 {:?}，實錄 {r:?}（第 {row} 筆等待）", e.kind);
            assert_eq!(row_of(r.step()), *row, "{name} 第 {i} 筆喇叭事件落在不同的等待");
            // 每個平面第一個事件之前有平面之間的固定碼，不比；靜音時閘門函式的指令數也不同；
            // 兩筆之間有 IRQ 0 時多了中斷處理常式的指令。
            if k > 0 && *on && !matches!(e.kind, SpkKind::Divisor(10)) && real_spk[i - 1].tick() == r.tick() {
                let d = r.step() - real_spk[i - 1].step();
                assert_eq!(
                    d,
                    e.steps as u64,
                    "{name} 第 {i} 筆喇叭事件的指令數：實錄 {d}，重製 {}；{what}，這段第 {k} 個 {:?}，前一個 {:?}",
                    e.steps,
                    e.kind,
                    ev[k - 1].kind
                );
                same_steps += 1;
            }
            i += 1;
        }
    }
    assert_eq!(i, real_spk.len(), "{name}：實錄有 {} 筆喇叭事件，重製 {i} 筆", real_spk.len());
    println!("{name}：音樂開關 {} 次、OPL 寫入 {} 筆、喇叭事件 {i} 筆（指令數相同 {same_steps} 筆）", music.len(), opl.len());
    (music.len(), i)
}

#[test]
#[ignore]
fn c2_c3_music_and_speaker_match() {
    let (inputs, assets, a, b) = setup();
    let (mut m, mut s) = (0, 0);
    for name in ["wt_title", "wt_menu", "wt_p2", "wt_flow", "wt_mute"] {
        let (x, y) = check(name, &inputs, &assets, &a, &b);
        m += x;
        s += y;
    }
    assert!(m > 0 && s > 0, "比對筆數為 0");
}
