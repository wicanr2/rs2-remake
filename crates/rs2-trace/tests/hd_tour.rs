//! HD 目視巡檢：用全流程收據的按鍵走過四關，每關多跑一段，定期把原版圖與 HD 存成 PNG。
//! 不是驗收，沒有斷言。`tools/rust.sh test -p rs2-trace --test hd_tour -- --ignored --nocapture`。
//! 輸出 `workspace/rs2/shots/tour/`，含原版美術，不進版控。

use rs2_core::{Fixes, Game, Input};
use rs2_gfx::{Assets, HdAssets, LayerSink, RenderOpts, png};
use rs2_trace::load_inputs;
use rs2_trace::wait::{ReplayEvent, ReplayOpts, Verdict, load_receipt, replay_observed};
use std::path::PathBuf;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("缺環境變數 {name}，用 tools/rust.sh 執行")))
}

/// 每關從原本按 Ctrl+C 的時點再多跑幾步。
const STAGE_STEPS: [u16; 4] = [760, 1000, 1500, 1300];
/// 每幾幀存一張。
const EVERY: u16 = 60;

#[test]
#[ignore]
fn hd_tour() {
    let Some(hd_dir) = std::env::var_os("RS2_HD") else {
        println!("沒有 RS2_HD，略過");
        return;
    };
    let dir = env_path("RS2_ORIG");
    let inputs = load_inputs(&dir).unwrap();
    let assets = Assets::load(&dir).unwrap();
    let rc = load_receipt(&env_path("RS2_DOSRUN").join("wt_flow")).unwrap();
    let mut list: Vec<Vec<u8>> = Vec::new();
    let opts = ReplayOpts { ds_report: false, locate_root: None, boot: true };
    let v = replay_observed(&inputs, &assets, &rc, &opts, &mut |ev| {
        if let ReplayEvent::StepAt { sc, .. } = ev {
            list.push(sc.to_vec());
        }
    });
    assert!(matches!(v, Verdict::Pass { .. }));

    let out = env_path("RS2_OUT").join("shots/tour");
    std::fs::create_dir_all(&out).unwrap();
    let hd = HdAssets::open(&PathBuf::from(hd_dir)).unwrap();
    let mut lay = LayerSink::new(assets.clone(), Some(hd));
    let mut g = Game::boot(inputs.game_files(), Fixes::ALL);
    lay.push_frame(&g.take_draws());

    let mut stage = 0usize;
    let mut i = 0usize;
    let mut shots = 0;
    let mut last_shot = (usize::MAX, 0u16);
    let mut step = 0usize;
    let mut held = 0u16;
    let mut extra = 0;
    while (i < list.len() || extra < 260) && step < 40000 {
        step += 1;
        if i >= list.len() {
            // 收據在片尾第一幀就結束，多跑一段看 END 字樣。
            extra += 1;
            let f = g.step(&Input { scancodes: &[] }, &mut |_g: &Game, _pt: u32| {});
            lay.push_frame(&f.draws);
            if f.exit.is_some() {
                break;
            }
            if extra % 40 == 0 && !f.draws.is_empty() {
                let name = |kind: &str| out.join(format!("s5_{extra:05}_{kind}.png"));
                let (a, w, h) = {
                    let (img, w, h) = lay.render(1.0, RenderOpts { interpolate: false, hd: false, show_counter: false, player: [None, None] });
                    (img.to_vec(), w, h)
                };
                std::fs::write(name("o"), png::encode_rgba(w, h, &a)).unwrap();
                let (b, w, h) = {
                    let (img, w, h) = lay.render(1.0, RenderOpts { interpolate: false, hd: true, show_counter: false, player: [None, None] });
                    (img.to_vec(), w, h)
                };
                std::fs::write(name("h"), png::encode_rgba(w, h, &b)).unwrap();
            }
            continue;
        }
        // Ctrl+C 的 C（2Eh）先留著，這一關多跑一段再送。頭目段幀計數會來回，所以用步數算。
        let want_c = list[i].contains(&0x2E);
        let hold = want_c && stage < 4 && held < STAGE_STEPS[stage];
        if hold {
            held += 1;
        }
        let sc: &[u8] = if hold { &[] } else { &list[i] };
        if !hold {
            if want_c {
                stage += 1;
                held = 0;
            }
            i += 1;
        }
        let f = g.step(&Input { scancodes: sc }, &mut |_g: &Game, _pt: u32| {});
        lay.push_frame(&f.draws);
        if f.exit.is_some() {
            break;
        }
        let frame = g.state().flow.frame;
        if !f.draws.is_empty() && step % (if stage >= 4 { 4 } else { EVERY as usize }) == 0 && last_shot != (stage, frame) {
            last_shot = (stage, frame);
            let name = |kind: &str| out.join(format!("s{}_{frame:05}_{kind}.png", stage + 1));
            let (a, w, h) = {
                let (img, w, h) = lay.render(1.0, RenderOpts { interpolate: false, hd: false, show_counter: false, player: [None, None] });
                (img.to_vec(), w, h)
            };
            std::fs::write(name("o"), png::encode_rgba(w, h, &a)).unwrap();
            let (b, w, h) = {
                let (img, w, h) = lay.render(1.0, RenderOpts { interpolate: false, hd: true, show_counter: false, player: [None, None] });
                (img.to_vec(), w, h)
            };
            std::fs::write(name("h"), png::encode_rgba(w, h, &b)).unwrap();
            shots += 1;
        }
    }
    println!("巡檢：{step} 步，{shots} 組畫面，缺圖 {:?}，調色盤 {:?}", lay.missing_hd(), lay.palette_name());
}
