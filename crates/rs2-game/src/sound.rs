//! 音效開關：AdLib 音樂的播放與停止、靜音鍵、PC 喇叭參數。
//!
//! 音樂與喇叭的聲音由接收端產生（`AudioCmd`），這裡只維護原版資料段上的開關與參數，
//! 並在狀態改變時送命令。狀態（[`Sound`]）掛在 `Core::sound`，經 `Mirror` 對應回資料段。
//!
//! PC 喇叭的參數式引擎（`sub_18672` 選參數，`sub_18736` 逐 tick 開關閘門）只有參數被寫入，
//! 逐 tick 的函式沒有執行路徑（`docs/re/13-audio-and-misc.md` 第 4.2 節），所以參數寫了沒有人讀，
//! 但資料段對拍逐位元組比它們，仍是狀態。
//!
//! | 原版 | 這裡 |
//! |---|---|
//! | `10AD4h` | [`Core::start_music`] |
//! | `10B12h` | [`Core::stop_music`] |
//! | `sub_187C3` | [`Core::sync_sound_switch`]，由 [`Core::show_page`](crate::video) 顯示頁 0 時呼叫 |
//! | `sub_18672` | [`Core::apply_speaker_flags`] |

use crate::draw::AudioCmd;
use crate::game::Core;

/// `byte_1CBCE` 的值：音樂可用。
pub(crate) const MUSIC_ON: u8 = b'Y';
/// `byte_1CBCE` 的值：音樂不可用或靜音中。
pub(crate) const MUSIC_OFF: u8 = b'N';

/// 音效開關與喇叭參數。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sound {
    /// `byte_1CBCE`：[`MUSIC_ON`] 才播放、停止音樂。沒有 AdLib 時原版把 `10AD4h`、`10B12h` 改成 `retn`，重製一律當作有。
    /// 靜音時 [`Core::sync_sound_switch`] 改成 [`MUSIC_OFF`]。
    pub music: u8,
    /// `byte_2684E`：位元 0 為 1 是靜音。INT 9 的 S 鍵切換（`Core::toggle_sound_mute`），[`Core::sync_sound_switch`] 讀。
    pub mute: u8,
    /// `byte_2684F`：喇叭音效旗標，[`Core::apply_speaker_flags`] 依位元選參數。
    pub speaker_flags: u8,
    pub speaker: SpeakerParams,
}

/// 喇叭參數式引擎的五個字組（`docs/re/13-audio-and-misc.md` 第 4.2 節的表）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpeakerParams {
    /// `word_26850`。
    pub duration: u16,
    /// `word_26852`。
    pub period: u16,
    /// `word_26854`。
    pub threshold: u16,
    /// `word_26856`。
    pub pitch: u16,
    /// `word_26858`。
    pub counter: u16,
}

impl Core {
    /// 開始播放目前載入的曲目（原版 `10AD4h`）。
    pub(crate) fn start_music(&mut self) {
        if self.sound.music == MUSIC_ON {
            self.audio.push(AudioCmd::Play(self.banks.mix));
        }
    }

    /// 停止播放（原版 `10B12h`）。
    pub(crate) fn stop_music(&mut self) {
        if self.sound.music == MUSIC_ON {
            self.audio.push(AudioCmd::Stop);
        }
    }

    /// 靜音鍵切換靜音旗標（原版 `sub_1A891` 的 `xor byte_2684E, 1`）。實際停播與恢復在 [`Core::sync_sound_switch`]。
    pub(crate) fn toggle_sound_mute(&mut self) {
        self.sound.mute ^= 1;
    }

    /// 是不是靜音。
    pub(crate) fn sound_muted(&self) -> bool {
        self.sound.mute & 1 != 0
    }

    /// 寫喇叭音效旗標（`byte_2684F`），[`Core::apply_speaker_flags`] 讀。
    pub(crate) fn set_speaker_flags(&mut self, flags: u8) {
        self.sound.speaker_flags = flags;
    }

    /// 同步音效開關（原版 `sub_187C3`）：靜音旗標置位就停音樂、關閉音樂旗標，並讓喇叭閘門不再動
    /// （原版把 `sub_187B3`、`sub_187BB` 改成 `retn`）；清除就恢復，並從頭播。
    pub(crate) fn sync_sound_switch(&mut self) {
        if self.sound_muted() {
            if self.sound.music != MUSIC_OFF {
                self.audio.push(AudioCmd::Speaker(false));
                self.stop_music();
                self.sound.music = MUSIC_OFF;
            }
        } else if self.sound.music != MUSIC_ON {
            self.audio.push(AudioCmd::Speaker(true));
            self.sound.music = MUSIC_ON;
            self.start_music();
        }
    }

    /// 測試用：音樂可用，`start_music` 才會送命令。
    #[cfg(test)]
    pub(crate) fn enable_music(&mut self) {
        self.sound.music = MUSIC_ON;
    }

    /// 依喇叭音效旗標設喇叭參數（原版 `sub_18672`）。位元 0、1、2、3 依序測試，先命中者勝，都沒有就不動。
    pub(crate) fn apply_speaker_flags(&mut self) {
        let flags = self.sound.speaker_flags;
        // (period, threshold, counter, duration, pitch)，設定函式分別是 sub_186BA、18717、186F8、186D9。
        let (period, threshold, counter, duration, pitch) = if flags & 1 != 0 {
            (0x32, 5, 0x32, 0, 0x1F40)
        } else if flags & 2 != 0 {
            (2, 1, 2, 7, 0x1F4)
        } else if flags & 4 != 0 {
            (2, 1, 2, 0x1E, 0x3E8)
        } else if flags & 8 != 0 {
            (3, 2, 3, 0xC8, 0x2EE0)
        } else {
            return;
        };
        self.sound.speaker = SpeakerParams { duration, period, threshold, pitch, counter };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Fixes, Start};

    fn core() -> Core {
        let mut c = Core::for_test(Start::Demo, Fixes::NONE);
        c.audio.clear();
        c
    }

    #[test]
    fn music_commands_need_the_music_flag() {
        let mut c = core();
        c.start_music();
        c.stop_music();
        assert!(c.audio.is_empty(), "旗標不是 Y 就不送");
        c.sound.music = MUSIC_ON;
        c.start_music();
        c.stop_music();
        assert_eq!(c.audio, [AudioCmd::Play(c.banks.mix), AudioCmd::Stop]);
    }

    #[test]
    fn the_mute_switch_stops_and_restarts_the_music_once() {
        let mut c = core();
        c.sound.music = MUSIC_ON;
        c.toggle_sound_mute();
        c.sync_sound_switch();
        c.sync_sound_switch();
        assert_eq!(c.audio, [AudioCmd::Speaker(false), AudioCmd::Stop], "靜音只送一次");
        assert_eq!(c.sound.music, MUSIC_OFF);
        c.audio.clear();
        c.toggle_sound_mute();
        c.sync_sound_switch();
        c.sync_sound_switch();
        assert_eq!(c.audio, [AudioCmd::Speaker(true), AudioCmd::Play(c.banks.mix)]);
        assert_eq!(c.sound.music, MUSIC_ON);
    }

    #[test]
    fn the_lowest_set_flag_bit_picks_the_speaker_parameters() {
        let mut c = core();
        c.sound.speaker = SpeakerParams { duration: 1, period: 2, threshold: 3, pitch: 4, counter: 5 };
        c.set_speaker_flags(0);
        c.apply_speaker_flags();
        assert_eq!(c.sound.speaker, SpeakerParams { duration: 1, period: 2, threshold: 3, pitch: 4, counter: 5 }, "沒有位元就不動");
        c.set_speaker_flags(0b1100);
        c.apply_speaker_flags();
        assert_eq!(c.sound.speaker, SpeakerParams { duration: 0x1E, period: 2, threshold: 1, pitch: 0x3E8, counter: 2 }, "位元 2 先於位元 3");
        c.set_speaker_flags(0b1111);
        c.apply_speaker_flags();
        assert_eq!(c.sound.speaker.pitch, 0x1F40, "位元 0 優先");
    }
}
