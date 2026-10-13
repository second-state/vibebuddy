//! Microphone bring-up diagnostic (`device.mic.record`, docs/firmware-bringup.md): record a few seconds of
//! I2S input, then report levels and stream the raw samples back as lines. Not a user feature.
//!
//! What the board has is settled on the device, not here: the command first reports which chips answer on
//! I2C (the ES8311 at 0x18, and anything at 0x40-0x43, where an ES7210 would sit), then records whatever
//! arrives on the I2S data-in pin. The input shares the speaker's clocks (one LRCK on the ES8311), so it
//! runs at the speaker's rate: 24 kHz, 16-bit, two slots. With esp_codec_dev's ES8311 setup (register
//! 0x44 = 0x58, "ADCL + DACR") the left slot carries the ADC and the right slot the DAC reference, as a
//! recording on one box showed (docs/hardware.md, 2026-10-13); the per-slot levels show it on others.

use alloc::string::String;
use alloc::vec::Vec;

use crate::audio::{ES8311_ADDRESS, Registers, SAMPLE_RATE};

/// Two 16-bit slots per frame.
pub const CHANNELS: u32 = 2;
pub const FRAME_BYTES: usize = 4;
pub const DEFAULT_MS: u32 = 3000;
/// At most 5 s: 480 KB of samples (the device's DMA buffer is 2.5 times that, in PSRAM), and about a
/// minute to send over the UART.
pub const MAX_MS: u32 = 5000;
pub const MIN_MS: u32 = 100;
/// xiaozhi's AUDIO_CODEC_DEFAULT_MIC_GAIN for this board.
pub const DEFAULT_GAIN_DB: u32 = 30;
pub const MAX_GAIN_DB: u32 = 42;
/// Raw bytes per `MIC DATA` line: 768 base64 characters, a line under 800 bytes like the protocol's 1 KB lines.
pub const DATA_CHUNK_BYTES: usize = 576;
/// One `MIC LEVEL` line per this much audio.
pub const LEVEL_WINDOW_MS: u32 = 100;

/// Addresses whose chip ID the command reports, with the two ID registers read at each: the ES8311's
/// 0xFD/0xFE, and the ES7210's four strap addresses (AD1/AD0) with 0x3D/0x3E, the ID registers its
/// datasheet is reported to use (not confirmed from a primary source; the raw bytes are printed as read).
pub const PROBES: [(u8, [u8; 2]); 5] = [
    (ES8311_ADDRESS, [0xFD, 0xFE]),
    (0x40, [0x3D, 0x3E]),
    (0x41, [0x3D, 0x3E]),
    (0x42, [0x3D, 0x3E]),
    (0x43, [0x3D, 0x3E]),
];

/// Bytes captured in `ms` of input.
pub fn capture_bytes(ms: u32) -> usize {
    (SAMPLE_RATE / 1000 * ms) as usize * FRAME_BYTES
}

/// esp_codec_dev's `es8311_set_mic_gain`: the PGA gain in 6 dB steps, 0 to 42 dB, rounded down, written
/// as the whole of register 0x16.
pub fn mic_gain_register(db: u32) -> u8 {
    (db.min(MAX_GAIN_DB) / 6) as u8
}

pub fn set_es8311_mic_gain<R: Registers>(codec: &mut R, db: u32) -> Result<(), R::Error> {
    codec.write(0x16, mic_gain_register(db))
}

/// RMS and peak of one slot over one window, in raw sample units (full scale is 32768).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Level {
    pub rms: u32,
    pub peak: u32,
}

/// Levels of both slots for each `LEVEL_WINDOW_MS` of a capture of interleaved little-endian frames.
pub fn levels(capture: &[u8]) -> Vec<[Level; 2]> {
    let window = (SAMPLE_RATE / 1000 * LEVEL_WINDOW_MS) as usize * FRAME_BYTES;
    capture
        .chunks(window)
        .map(|piece| {
            let mut squares = [0u64; 2];
            let mut peaks = [0u32; 2];
            let frames = (piece.len() / FRAME_BYTES).max(1) as u64;
            for frame in piece.as_chunks::<FRAME_BYTES>().0 {
                for slot in 0..2 {
                    let sample = i16::from_le_bytes([frame[slot * 2], frame[slot * 2 + 1]]) as i32;
                    squares[slot] += (sample * sample) as u64;
                    peaks[slot] = peaks[slot].max(sample.unsigned_abs());
                }
            }
            core::array::from_fn(|slot| Level { rms: libm::sqrt(squares[slot] as f64 / frames as f64) as u32, peak: peaks[slot] })
        })
        .collect()
}

/// Standard base64 with `=` padding, the counterpart of [`crate::voice_pack::decode_base64`].
pub fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for index in 0..4 {
            out.push(if index <= chunk.len() { ALPHABET[(bits >> (18 - 6 * index) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice_pack::decode_base64;
    use std::vec;

    #[test]
    fn gain_matches_esp_codec_dev() {
        // es8311_set_mic_gain: <6 dB → 0 dB (0), ..., 30 dB → 5, ≥42 dB → 7.
        assert_eq!(mic_gain_register(0), 0);
        assert_eq!(mic_gain_register(5), 0);
        assert_eq!(mic_gain_register(6), 1);
        assert_eq!(mic_gain_register(30), 5);
        assert_eq!(mic_gain_register(41), 6);
        assert_eq!(mic_gain_register(42), 7);
        assert_eq!(mic_gain_register(60), 7);
    }

    #[test]
    fn a_capture_is_24_khz_stereo_16_bit() {
        assert_eq!(capture_bytes(1000), 96_000);
        assert_eq!(capture_bytes(MAX_MS), 480_000);
    }

    #[test]
    fn levels_are_per_slot_and_per_window() {
        // 100 ms of a full-scale square wave on the left, silence on the right, then 50 ms of a quieter left.
        let mut capture = Vec::new();
        for index in 0..2400 {
            let left: i16 = if index % 2 == 0 { 32767 } else { -32767 };
            capture.extend_from_slice(&left.to_le_bytes());
            capture.extend_from_slice(&0i16.to_le_bytes());
        }
        for _ in 0..1200 {
            capture.extend_from_slice(&(-100i16).to_le_bytes());
            capture.extend_from_slice(&7i16.to_le_bytes());
        }
        assert_eq!(
            levels(&capture),
            vec![
                [Level { rms: 32767, peak: 32767 }, Level { rms: 0, peak: 0 }],
                [Level { rms: 100, peak: 100 }, Level { rms: 7, peak: 7 }],
            ]
        );
        // i16::MIN has no positive counterpart; its peak is still reported, not wrapped.
        assert_eq!(levels(&i16::MIN.to_le_bytes().repeat(2))[0][0].peak, 32768);
    }

    #[test]
    fn base64_round_trips() {
        for length in 0..10 {
            let bytes: Vec<u8> = (0..length).map(|index| (index * 37 + 250) as u8).collect();
            let encoded = encode_base64(&bytes);
            let mut out = [0u8; 16];
            assert_eq!(decode_base64(encoded.as_bytes(), &mut out), Some(length));
            assert_eq!(&out[..length], &bytes[..]);
        }
        assert_eq!(encode_base64(b"foob"), "Zm9vYg==");
        assert_eq!(encode_base64(&[0u8; DATA_CHUNK_BYTES]).len(), 768);
    }
}
