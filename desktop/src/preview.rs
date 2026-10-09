//! Previewing a Character on the computer before writing it to the box, the Mac's `VoicePreview` and
//! `VoicePack.previewPCM()`: the first line of each ordinary occasion (or an old voice pack's five clips), decoded
//! the way the box decodes them and played through whatever plays raw PCM here (PipeWire, PulseAudio or ALSA).

use std::sync::Arc;

use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;

const RATE: usize = 24_000;
const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118,
    130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060,
    1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484,
    7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX_CHANGE: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

fn u16_at(data: &[u8], at: usize) -> usize {
    u16::from_le_bytes([data[at], data[at + 1]]) as usize
}

fn u32_at(data: &[u8], at: usize) -> usize {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize
}

/// What a preview plays, as 24 kHz, 16-bit stereo PCM: the pieces joined with 300 ms of silence. None for a file
/// that is neither kind of pack.
pub fn pcm(pack: &[u8]) -> Option<Vec<u8>> {
    let pieces: Vec<Vec<u8>> = match pack.get(..4)? {
        b"VBVP" if pack.len() > 256 => (0..5)
            .map(|index| {
                let (offset, length) = (u32_at(pack, 48 + index * 4), u32_at(pack, 68 + index * 4));
                pack.get(offset..offset + length).map(<[u8]>::to_vec)
            })
            .collect::<Option<_>>()?,
        b"VBCP" if pack.len() > 1024 && pack[52] == 1 => {
            let lines = u16_at(pack, 54);
            // The five ordinary occasions come first in the table.
            (0..usize::from(pack[53]).min(5))
                .filter(|occasion| u16_at(pack, 58 + occasion * 4) > 0 && u16_at(pack, 56 + occasion * 4) < lines)
                .map(|occasion| {
                    let first = u16_at(pack, 56 + occasion * 4);
                    let (offset, samples) = (u32_at(pack, 128 + first * 8), u32_at(pack, 132 + first * 8));
                    pack.get(offset..offset + samples.div_ceil(2)).map(|bytes| decode(bytes, samples))
                })
                .collect::<Option<_>>()?
        }
        _ => return None,
    };
    let gap = vec![0u8; RATE * 2 * 2 * 3 / 10];
    Some(pieces.join(gap.as_slice()))
}

/// The box's line decoding (firmware-rs/core/src/adpcm.rs): 16 kHz mono IMA ADPCM in, 24 kHz stereo 16-bit PCM
/// out, by linear interpolation.
fn decode(bytes: &[u8], samples: usize) -> Vec<u8> {
    let (mut predictor, mut index) = (0i32, 0i32);
    let mut input = Vec::with_capacity(samples);
    for byte in bytes {
        for nibble in [byte & 15, byte >> 4] {
            if input.len() == samples {
                break;
            }
            let step = STEPS[index as usize];
            let mut diff = step >> 3;
            if nibble & 4 != 0 {
                diff += step;
            }
            if nibble & 2 != 0 {
                diff += step >> 1;
            }
            if nibble & 1 != 0 {
                diff += step >> 2;
            }
            predictor = (if nibble & 8 != 0 { predictor - diff } else { predictor + diff }).clamp(-32768, 32767);
            index = (index + INDEX_CHANGE[nibble as usize]).clamp(0, 88);
            input.push(predictor);
        }
    }
    if input.is_empty() {
        return Vec::new();
    }
    let outputs = (samples * 3).div_ceil(2);
    let mut pcm = Vec::with_capacity(outputs * 4);
    for k in 0..outputs {
        let at = k * 2;
        let position = at / 3;
        let current = input[position.min(input.len() - 1)];
        let next = input[(position + 1).min(input.len() - 1)];
        let sample = (current + (next - current) * (at % 3) as i32 / 3) as i16;
        pcm.extend_from_slice(&sample.to_le_bytes());
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    pcm
}

/// Plays `pcm` until it ends or `stop` is notified.
pub async fn play(pcm: Vec<u8>, stop: Arc<Notify>) -> Result<(), String> {
    let players: [&[&str]; 3] = [
        &["pw-play", "--rate", "24000", "--channels", "2", "--format", "s16", "-"],
        &["paplay", "--raw", "--rate=24000", "--channels=2", "--format=s16le"],
        &["aplay", "-q", "-t", "raw", "-f", "S16_LE", "-r", "24000", "-c", "2"],
    ];
    for player in players {
        let spawned = tokio::process::Command::new(player[0])
            .args(&player[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{}: {error}", player[0])),
        };
        let mut stdin = child.stdin.take().ok_or("no pipe to the player")?;
        let feed = async move {
            // A player stopped halfway closes the pipe; that's not an error worth showing.
            let _ = stdin.write_all(&pcm).await;
        };
        tokio::select! {
            _ = async { feed.await; child.wait().await } => {}
            () = stop.notified() => {
                let _ = child.kill().await;
            }
        }
        return Ok(());
    }
    Err("Can't play audio here: install pw-play, paplay or aplay.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoding_turns_16k_mono_into_24k_stereo() {
        // 4 samples of ADPCM become 6 stereo frames of 4 bytes.
        assert_eq!(decode(&[0x77, 0x77], 4).len(), 6 * 4);
        let silence = decode(&[0x00, 0x00], 4);
        assert!(silence.chunks(2).all(|sample| i16::from_le_bytes([sample[0], sample[1]]).abs() < 8));
    }

    #[test]
    fn a_shipped_character_pack_has_a_preview() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../characters");
        let pack = std::fs::read_dir(&root)
            .expect("characters")
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("pack.bin"))
            .find(|path| path.is_file())
            .expect("a pack.bin");
        let pcm = pcm(&std::fs::read(pack).unwrap()).expect("preview");
        // Several lines of speech: well over a second at 96 KB a second.
        assert!(pcm.len() > RATE * 4);
        assert_eq!(pcm.len() % 4, 0);
    }

    #[test]
    fn anything_else_has_none() {
        assert_eq!(pcm(b"not a pack"), None);
        assert_eq!(pcm(&[0u8; 2048]), None);
    }
}
