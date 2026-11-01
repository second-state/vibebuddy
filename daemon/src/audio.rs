use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use std::time::{Duration, Instant};

const MAX_RATE: u32 = 16_000;
const MAX_DURATION_MS: u32 = 60_000;
const MAX_FRAME_BYTES: usize = 640;
const TIMING_TOLERANCE_FRAMES: u32 = 25;
/// A peer that lost power sends no TCP reset, so a half-open link would otherwise leave an armed
/// session active forever. The device's worst legitimate silence between audio lines is its 4 s
/// send patience plus reconnect slack; past this the recording can no longer finish.
const SESSION_STALL: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default, Serialize)]
pub struct AudioStats {
    pub sample_rate: u32,
    pub frames: u32,
    pub pcm_bytes: usize,
    pub duration_ms: u32,
    pub transport_ms: u128,
    pub last_lag_ms: i128,
    pub max_interval_ms: u128,
    pub lost_frames: u32,
    pub duplicate_frames: u32,
    pub reordered_frames: u32,
    pub discontinuities: u32,
    pub device_overruns: u32,
    pub device_slots: u32,
    pub timing_padding_frames: u32,
    pub timing_trim_frames: u32,
}

struct Session {
    rate: u32,
    frame_samples: usize,
    max_frames: u32,
    next_sequence: u32,
    pcm: Vec<u8>,
    stats: AudioStats,
    armed: Instant,
    last_arrival: Option<Instant>,
    last_sequence: u32,
    begun: bool,
    allow_short: bool,
}

struct Completed {
    wav: Vec<u8>,
    stats: AudioStats,
}

#[derive(Default)]
pub struct AudioIngestor {
    session: Option<Session>,
    completed: Option<Completed>,
    last_stats: Option<AudioStats>,
    last_error: Option<String>,
}

impl AudioIngestor {
    pub fn arm(&mut self, rate: u32, duration_ms: u32) -> Result<(), String> {
        if self.session.is_some() {
            return Err("a recording is already active".to_owned());
        }
        if !matches!(rate, 8_000 | 16_000) || rate > MAX_RATE {
            return Err("sample rate must be 8000 or 16000".to_owned());
        }
        if duration_ms == 0 || duration_ms > MAX_DURATION_MS {
            return Err("duration must be between 1 and 60000 ms".to_owned());
        }
        let frame_samples = (rate / 50) as usize;
        let max_frames = duration_ms / 20;
        let capacity = (rate as usize * 2 * duration_ms as usize / 1000).min(MAX_RATE as usize * 2 * 60);
        self.completed = None;
        self.last_stats = None;
        self.last_error = None;
        self.session = Some(Session {
            rate,
            frame_samples,
            max_frames,
            next_sequence: 0,
            pcm: Vec::with_capacity(capacity),
            stats: AudioStats { sample_rate: rate, ..AudioStats::default() },
            armed: Instant::now(),
            last_arrival: None,
            last_sequence: 0,
            begun: false,
            allow_short: false,
        });
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(session) = self.session.as_mut() {
            session.allow_short = true;
        }
    }

    pub fn cancel(&mut self) {
        self.session = None;
        self.completed = None;
        self.last_error = None;
    }

    pub fn disconnected(&mut self) {
        self.cancel();
        self.last_error = Some("device disconnected during recording".to_owned());
    }

    pub fn error(&mut self) -> Option<String> {
        self.expire_stalled();
        self.last_error.clone()
    }

    pub fn stats(&mut self) -> Option<AudioStats> {
        self.expire_stalled();
        self.completed.as_ref().map(|completed| completed.stats.clone())
            .or_else(|| self.session.as_ref().map(|session| session.stats.clone()))
            .or_else(|| self.last_stats.clone())
    }

    fn expire_stalled(&mut self) {
        let stalled = self.session.as_ref().is_some_and(|session| {
            session.last_arrival.unwrap_or(session.armed).elapsed() >= SESSION_STALL
        });
        if stalled {
            let _: Result<(), String> = self.fail("device stopped sending the recording");
        }
    }

    pub fn take_wav(&mut self) -> Option<Vec<u8>> {
        let completed = self.completed.take()?;
        self.last_error = None;
        self.last_stats = Some(completed.stats);
        Some(completed.wav)
    }

    pub fn process_line(&mut self, line: &str) -> Result<bool, String> {
        if !line.starts_with("AUDIO ") {
            return Ok(false);
        }
        if self.session.is_none() {
            return Ok(true);
        }
        if line.starts_with("AUDIO BEGIN ") {
            return self.begin(line).map(|()| true);
        }
        if line.starts_with("AUDIO FRAME ") {
            return self.frame(line).map(|()| true);
        }
        if line.starts_with("AUDIO END ") {
            return self.end(line).map(|()| true);
        }
        self.fail("unknown audio record")
    }

    fn fail<T>(&mut self, message: &str) -> Result<T, String> {
        self.session = None;
        self.completed = None;
        self.last_error = Some(message.to_owned());
        Err(message.to_owned())
    }

    fn begin(&mut self, line: &str) -> Result<(), String> {
        let mut rate = None;
        let mut samples = None;
        for item in line["AUDIO BEGIN ".len()..].split(' ') {
            if let Some(value) = item.strip_prefix("rate=") {
                rate = value.parse::<u32>().ok();
            } else if let Some(value) = item.strip_prefix("samples=") {
                samples = value.parse::<usize>().ok();
            } else {
                return self.fail("malformed audio begin");
            }
        }
        let session = self.session.as_mut().expect("session checked");
        if session.begun || rate != Some(session.rate) || samples != Some(session.frame_samples) {
            return self.fail("audio begin does not match the armed recording");
        }
        session.begun = true;
        Ok(())
    }

    fn frame(&mut self, line: &str) -> Result<(), String> {
        let mut parts = line.splitn(6, ' ');
        let (_, _, sequence, length, flags, encoded) = (
            parts.next(), parts.next(), parts.next(), parts.next(), parts.next(), parts.next(),
        );
        let (Some(sequence), Some(length), Some(flags), Some(encoded)) = (sequence, length, flags, encoded) else {
            return self.fail("malformed audio frame");
        };
        let (Ok(sequence), Ok(length), Ok(flags)) = (sequence.parse::<u32>(), length.parse::<usize>(), flags.parse::<u8>()) else {
            return self.fail("malformed audio frame numbers");
        };
        if length > MAX_FRAME_BYTES || length % 2 != 0 || encoded.len() > 1024 || flags > 3 {
            return self.fail("oversized audio frame or flags");
        }
        let session = self.session.as_mut().expect("session checked");
        if !session.begun {
            return self.fail("audio frame before begin");
        }
        if sequence < session.next_sequence {
            if sequence + 1 == session.next_sequence {
                session.stats.duplicate_frames += 1;
            } else {
                session.stats.reordered_frames += 1;
            }
            return Ok(());
        }
        if sequence >= session.max_frames + TIMING_TOLERANCE_FRAMES {
            return self.fail("audio exceeds the armed duration");
        }
        if sequence > session.next_sequence {
            let missing = sequence - session.next_sequence;
            session.stats.lost_frames += missing;
            for _ in 0..missing {
                session.pcm.resize(session.pcm.len() + session.frame_samples * 2, 0);
            }
            session.next_sequence = sequence;
        }
        let frame_bytes = session.frame_samples * 2;
        let final_frame = flags & 1 != 0;
        if (!final_frame && length != frame_bytes) || (final_frame && length > frame_bytes) {
            return self.fail("audio frame length does not match the sample rate");
        }
        let decoded = match STANDARD.decode(encoded) {
            Ok(decoded) => decoded,
            Err(_) => return self.fail("invalid audio base64"),
        };
        if decoded.len() != length {
            return self.fail("audio length does not match its content");
        }
        let now = Instant::now();
        if let Some(previous) = session.last_arrival {
            session.stats.max_interval_ms = session.stats.max_interval_ms.max(now.duration_since(previous).as_millis());
        }
        session.last_arrival = Some(now);
        session.last_sequence = sequence;
        session.pcm.extend_from_slice(&decoded);
        session.stats.frames += 1;
        if flags & 2 != 0 {
            session.stats.discontinuities += 1;
        }
        session.next_sequence += 1;
        Ok(())
    }

    fn end(&mut self, line: &str) -> Result<(), String> {
        let mut parts = line.split(' ');
        let (Ok(sequence), Ok(overruns)) = (
            parts.nth(2).unwrap_or("").parse::<u32>(),
            parts.next().unwrap_or("").parse::<u32>(),
        ) else {
            return self.fail("malformed audio end");
        };
        let slots = match parts.next() {
            Some(value) => value.parse::<u32>().map_err(|_| "malformed audio slot count".to_owned()),
            None => Ok(0),
        };
        let slots = match slots {
            Ok(slots) => slots,
            Err(error) => return self.fail(&error),
        };
        if parts.next().is_some() {
            return self.fail("malformed audio end");
        }
        let session = self.session.as_mut().expect("session checked");
        if !session.begun || sequence < session.next_sequence || sequence > session.max_frames + TIMING_TOLERANCE_FRAMES {
            return self.fail("audio end sequence is invalid");
        }
        if !session.allow_short
            && (sequence + TIMING_TOLERANCE_FRAMES < session.max_frames || sequence > session.max_frames + TIMING_TOLERANCE_FRAMES)
        {
            return self.fail("recording duration differs too much from the armed duration");
        }
        if sequence > session.next_sequence {
            let missing = sequence - session.next_sequence;
            session.stats.lost_frames += missing;
            for _ in 0..missing {
                session.pcm.resize(session.pcm.len() + session.frame_samples * 2, 0);
            }
            session.next_sequence = sequence;
        }
        if !session.allow_short && sequence < session.max_frames {
            let padding = session.max_frames - sequence;
            session.stats.timing_padding_frames = padding;
            for _ in 0..padding {
                session.pcm.resize(session.pcm.len() + session.frame_samples * 2, 0);
            }
        } else if !session.allow_short && sequence > session.max_frames {
            session.stats.timing_trim_frames = sequence - session.max_frames;
            session.pcm.truncate(session.max_frames as usize * session.frame_samples * 2);
        }
        session.stats.device_overruns = overruns;
        session.stats.device_slots = slots;
        session.stats.pcm_bytes = session.pcm.len();
        session.stats.duration_ms = (session.pcm.len() as u32 * 1000) / (session.rate * 2);
        session.stats.transport_ms = session.last_arrival.unwrap_or(session.armed).duration_since(session.armed).as_millis();
        session.stats.last_lag_ms = session.stats.transport_ms as i128 - (session.last_sequence as i128 * 20);
        let session = self.session.take().expect("session exists");
        let wav = wav(session.rate, &session.pcm);
        self.completed = Some(Completed { wav, stats: session.stats });
        Ok(())
    }
}

fn wav(rate: u32, pcm: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(44 + pcm.len());
    output.extend_from_slice(b"RIFF");
    output.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    output.extend_from_slice(b"WAVEfmt ");
    output.extend_from_slice(&16_u32.to_le_bytes());
    output.extend_from_slice(&1_u16.to_le_bytes());
    output.extend_from_slice(&1_u16.to_le_bytes());
    output.extend_from_slice(&rate.to_le_bytes());
    output.extend_from_slice(&(rate * 2).to_le_bytes());
    output.extend_from_slice(&2_u16.to_le_bytes());
    output.extend_from_slice(&16_u16.to_le_bytes());
    output.extend_from_slice(b"data");
    output.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    output.extend_from_slice(pcm);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(sequence: u32, flags: u8, bytes: &[u8]) -> String {
        format!("AUDIO FRAME {sequence} {} {flags} {}", bytes.len(), STANDARD.encode(bytes))
    }

    fn assert_rejected(line: &str, expected: &str) {
        let mut audio = AudioIngestor::default();
        audio.arm(8000, 20).unwrap();
        audio.process_line("AUDIO BEGIN rate=8000 samples=160").unwrap();
        assert_eq!(audio.process_line(line).unwrap_err(), expected);
        assert_eq!(audio.error().as_deref(), Some(expected));
        assert!(audio.stats().is_none());
        assert!(audio.take_wav().is_none());
    }

    #[test]
    fn reconstructs_a_little_endian_wav_without_retaining_a_session() {
        let mut audio = AudioIngestor::default();
        audio.arm(8000, 20).unwrap();
        audio.process_line("AUDIO BEGIN rate=8000 samples=160").unwrap();
        let pcm: Vec<u8> = (0..320).map(|value| value as u8).collect();
        audio.process_line(&frame(0, 0, &pcm)).unwrap();
        assert_eq!(audio.stats().unwrap().frames, 1);
        audio.process_line("AUDIO END 1 0").unwrap();
        let stats = audio.stats().unwrap();
        assert_eq!((stats.frames, stats.pcm_bytes, stats.duration_ms), (1, 320, 20));
        let wav = audio.take_wav().unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 8000);
        assert_eq!(u32::from_le_bytes(wav[28..32].try_into().unwrap()), 16000);
        assert_eq!(&wav[44..], pcm);
        assert!(audio.take_wav().is_none());
        assert_eq!(audio.stats().unwrap().pcm_bytes, 320);
        assert_eq!(audio.error(), None);
    }

    #[test]
    fn gaps_are_zero_filled_and_duplicates_and_reordering_do_not_add_audio() {
        let mut audio = AudioIngestor::default();
        audio.arm(16000, 40).unwrap();
        audio.process_line("AUDIO BEGIN rate=16000 samples=320").unwrap();
        let samples = vec![7; 640];
        audio.process_line(&frame(1, 2, &samples)).unwrap();
        let active = audio.stats().unwrap();
        assert_eq!((active.frames, active.lost_frames), (1, 1));
        audio.process_line(&frame(1, 0, &vec![8; 640])).unwrap();
        audio.process_line(&frame(0, 0, &vec![9; 640])).unwrap();
        audio.process_line("AUDIO END 2 3").unwrap();
        let stats = audio.stats().unwrap();
        assert_eq!((stats.frames, stats.lost_frames, stats.duplicate_frames, stats.reordered_frames), (1, 1, 1, 1));
        assert_eq!((stats.discontinuities, stats.device_overruns, stats.pcm_bytes), (1, 3, 1280));
        let wav = audio.take_wav().unwrap();
        assert_eq!(&wav[44..684], vec![0; 640]);
        assert_eq!(&wav[684..], samples);
    }

    #[test]
    fn rejects_malformed_oversized_and_inconsistent_frames_with_terminal_reasons() {
        let cases = [
            ("AUDIO FRAME nope 320 0 AA==".to_owned(), "malformed audio frame numbers"),
            ("AUDIO FRAME 0 321 0 AA==".to_owned(), "oversized audio frame or flags"),
            ("AUDIO FRAME 0 320 4 AA==".to_owned(), "oversized audio frame or flags"),
            (frame(0, 0, &[0; 642]), "oversized audio frame or flags"),
            (frame(0, 0, &[0; 2]), "audio frame length does not match the sample rate"),
            (frame(26, 0, &vec![0; 320]), "audio exceeds the armed duration"),
            ("AUDIO FRAME 0 320 0 not-base64".to_owned(), "invalid audio base64"),
            ("AUDIO FRAME 0 320 0 AA==".to_owned(), "audio length does not match its content"),
        ];
        for (line, expected) in cases {
            assert_rejected(&line, expected);
        }
    }

    #[test]
    fn a_frame_before_begin_is_terminal_until_rearmed() {
        let mut audio = AudioIngestor::default();
        audio.arm(8000, 20).unwrap();
        assert_eq!(audio.process_line(&frame(0, 0, &vec![0; 320])).unwrap_err(), "audio frame before begin");
        assert_eq!(audio.error().as_deref(), Some("audio frame before begin"));
        assert!(audio.process_line("AUDIO END 1 0").unwrap());
        audio.arm(8000, 20).unwrap();
        assert_eq!(audio.error(), None);
        assert_eq!(audio.stats().unwrap().frames, 0);
    }

    #[test]
    fn small_timing_differences_are_normalized_separately_from_loss() {
        let mut audio = AudioIngestor::default();
        audio.arm(8000, 100).unwrap();
        audio.process_line("AUDIO BEGIN rate=8000 samples=160").unwrap();
        audio.process_line("AUDIO END 0 0").unwrap();
        let stats = audio.stats().unwrap();
        assert_eq!((stats.timing_padding_frames, stats.lost_frames, stats.pcm_bytes, stats.duration_ms), (5, 0, 1600, 100));
        audio.arm(8000, 20).unwrap();
        audio.process_line("AUDIO BEGIN rate=8000 samples=160").unwrap();
        audio.process_line(&frame(0, 0, &vec![1; 320])).unwrap();
        audio.process_line(&frame(1, 0, &vec![2; 320])).unwrap();
        audio.process_line("AUDIO END 2 0").unwrap();
        let stats = audio.stats().unwrap();
        assert_eq!((stats.timing_trim_frames, stats.lost_frames, stats.pcm_bytes, stats.duration_ms), (1, 0, 320, 20));
        assert_eq!(&audio.take_wav().unwrap()[44..], vec![1; 320]);
    }

    #[test]
    fn an_early_end_is_rejected_unless_stop_was_requested() {
        let mut audio = AudioIngestor::default();
        audio.arm(8000, 1000).unwrap();
        audio.process_line("AUDIO BEGIN rate=8000 samples=160").unwrap();
        let expected = "recording duration differs too much from the armed duration";
        assert_eq!(audio.process_line("AUDIO END 0 0").unwrap_err(), expected);
        assert_eq!(audio.error().as_deref(), Some(expected));
        assert!(audio.take_wav().is_none());
        audio.arm(8000, 1000).unwrap();
        audio.process_line("AUDIO BEGIN rate=8000 samples=160").unwrap();
        audio.stop();
        audio.process_line("AUDIO END 0 0").unwrap();
        let stats = audio.stats().unwrap();
        assert_eq!((stats.pcm_bytes, stats.duration_ms, stats.timing_padding_frames), (0, 0, 0));
        assert_eq!(audio.error(), None);
        assert_eq!(audio.take_wav().unwrap().len(), 44);
    }

    #[test]
    fn cancellation_drops_partial_audio_without_an_error() {
        let mut audio = AudioIngestor::default();
        audio.arm(8000, 1000).unwrap();
        audio.process_line("AUDIO BEGIN rate=8000 samples=160").unwrap();
        audio.process_line(&frame(0, 0, &vec![0; 320])).unwrap();
        audio.cancel();
        assert!(audio.stats().is_none());
        assert!(audio.take_wav().is_none());
        assert_eq!(audio.error(), None);
    }

    #[test]
    fn disconnect_preserves_its_reason_and_a_new_recording_recovers() {
        let mut audio = AudioIngestor::default();
        audio.arm(8000, 1000).unwrap();
        audio.process_line("AUDIO BEGIN rate=8000 samples=160").unwrap();
        audio.process_line(&frame(0, 0, &vec![0; 320])).unwrap();
        audio.disconnected();
        assert_eq!(audio.error().as_deref(), Some("device disconnected during recording"));
        assert!(audio.stats().is_none());
        assert!(audio.take_wav().is_none());
        assert!(audio.process_line("AUDIO END 1 0").unwrap());
        assert_eq!(audio.error().as_deref(), Some("device disconnected during recording"));
        audio.arm(8000, 20).unwrap();
        assert_eq!(audio.error(), None);
        assert_eq!(audio.stats().unwrap().frames, 0);
        audio.process_line("AUDIO BEGIN rate=8000 samples=160").unwrap();
        audio.stop();
        audio.process_line("AUDIO END 0 0").unwrap();
        assert_eq!(audio.take_wav().unwrap().len(), 44);
    }

    #[test]
    fn a_session_that_goes_silent_expires_with_a_terminal_reason() {
        let mut audio = AudioIngestor::default();
        audio.arm(16000, 60000).unwrap();
        audio.process_line("AUDIO BEGIN rate=16000 samples=320").unwrap();
        audio.process_line(&frame(0, 0, &vec![1; 640])).unwrap();
        // A device that lost power sends no reset; age the last arrival past the stall bound.
        audio.session.as_mut().unwrap().last_arrival =
            Some(Instant::now() - Duration::from_secs(11));
        assert!(audio.stats().is_none());
        assert_eq!(audio.error().as_deref(), Some("device stopped sending the recording"));
        assert!(audio.take_wav().is_none());
        // A session armed but never begun expires the same way, from its arm time.
        audio.arm(8000, 2000).unwrap();
        audio.session.as_mut().unwrap().armed = Instant::now() - Duration::from_secs(11);
        assert!(audio.stats().is_none());
        assert_eq!(audio.error().as_deref(), Some("device stopped sending the recording"));
        assert!(audio.arm(8000, 20).is_ok());
    }
}
