//! 音訊輸出：SDL 音效裝置與 `--wav-out`。模擬不等音訊，音訊也不回頭影響模擬。
//!
//! 命令由主迴圈經通道送給裝置的 callback，所以音樂的時鐘是音效裝置的取樣時鐘。

use crate::session::AudioMsg;
use rs2_audio::{Engine, NATIVE_RATE, Track};
use rs2_core::MixFile;
use sdl2::AudioSubsystem;
use sdl2::audio::{AudioCallback, AudioDevice, AudioSpecDesired};
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};

fn track(m: MixFile) -> Track {
    match m {
        MixFile::A => Track::A,
        MixFile::B => Track::B,
    }
}

fn apply(e: &mut Engine, m: &AudioMsg) {
    match m {
        AudioMsg::Play(f) => e.play(track(*f), &mut |_, _| {}),
        AudioMsg::Stop => e.stop(&mut |_, _| {}),
        AudioMsg::Speaker(ev) => e.speaker(ev),
        AudioMsg::Sfx(k) => e.sfx(*k),
    }
}

struct Callback {
    engine: Engine,
    rx: Receiver<AudioMsg>,
}

impl AudioCallback for Callback {
    type Channel = i16;

    fn callback(&mut self, out: &mut [i16]) {
        while let Ok(m) = self.rx.try_recv() {
            apply(&mut self.engine, &m);
        }
        for s in out {
            *s = self.engine.sample(&mut |_, _| {});
        }
    }
}

/// 開著的音效裝置。
pub struct Output {
    _dev: AudioDevice<Callback>,
    tx: Sender<AudioMsg>,
}

impl Output {
    /// 開裝置。失敗回傳原因，呼叫端繼續跑，只是沒有聲音。
    pub fn open(sys: &AudioSubsystem, engine: Engine) -> Result<Output, String> {
        let (tx, rx) = channel();
        let want = AudioSpecDesired { freq: Some(NATIVE_RATE as i32), channels: Some(1), samples: Some(1024) };
        let dev = sys.open_playback(None, &want, |_spec| Callback { engine, rx })?;
        dev.resume();
        Ok(Output { _dev: dev, tx })
    }

    pub fn send(&self, m: AudioMsg) {
        // 裝置關掉之後送不出去也沒關係。
        let _ = self.tx.send(m);
    }
}

/// `--wav-out`：依遊戲時間算出整段音訊。一個 tick 是 65536 個 PIT 週期，一個取樣是 24 個 PIT 週期。
pub struct WavOut {
    engine: Engine,
    pcm: Vec<u8>,
    acc: u64,
}

impl WavOut {
    pub fn new(engine: Engine) -> WavOut {
        WavOut { engine, pcm: Vec::new(), acc: 0 }
    }

    pub fn send(&mut self, m: &AudioMsg) {
        apply(&mut self.engine, m);
    }

    /// 這一步之後要等 `ticks` 個 tick。
    pub fn advance(&mut self, ticks: u16) {
        self.acc += ticks as u64 * 65536;
        while self.acc >= rs2_audio::engine::PIT_PER_SAMPLE as u64 {
            self.acc -= rs2_audio::engine::PIT_PER_SAMPLE as u64;
            let s = self.engine.sample(&mut |_, _| {});
            self.pcm.extend_from_slice(&s.to_le_bytes());
        }
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let mut f = Vec::with_capacity(44 + self.pcm.len());
        let n = self.pcm.len() as u32;
        f.extend_from_slice(b"RIFF");
        f.extend_from_slice(&(36 + n).to_le_bytes());
        f.extend_from_slice(b"WAVEfmt ");
        f.extend_from_slice(&16u32.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&NATIVE_RATE.to_le_bytes());
        f.extend_from_slice(&(NATIVE_RATE * 2).to_le_bytes());
        f.extend_from_slice(&2u16.to_le_bytes());
        f.extend_from_slice(&16u16.to_le_bytes());
        f.extend_from_slice(b"data");
        f.extend_from_slice(&n.to_le_bytes());
        f.extend_from_slice(&self.pcm);
        std::fs::write(path, f).map_err(|e| format!("寫不了 {}：{e}", path.display()))
    }
}
