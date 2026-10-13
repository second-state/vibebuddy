//! Voice commands on the box (ADR-0017; the contract is docs/protocol.md, "Voice commands (experimental)"):
//! while the computer offers voice, holding K0 records, and the recording streams out as `VOICE` lines; the
//! computer answers with what it heard and the resident session's spoken replies, which the box shows on an
//! overlay. Like the device menu, the overlay is not a mode: the mode underneath keeps running.
//!
//! This module is the turn and the overlay, with no hardware: it takes the microphone's samples, encodes them,
//! frames the lines and says what the overlay shows. `firmware.rs` feeds it keys, samples and events, writes
//! its lines and starts and stops the microphone.

use alloc::string::String;
use alloc::vec::Vec;

use crate::adpcm::Encoder;
use crate::display::{Panel, PanelKind, Tone, VOICE_TEXT_WIDTH, voice_text_rows};
use crate::font;
use crate::mic::encode_base64;

/// The microphone runs at the speaker's rate (docs/firmware-bringup.md, "Microphone").
pub const SAMPLE_RATE: u32 = 24_000;
/// Samples per `VOICE DATA` line: 576 bytes of ADPCM, 768 base64 characters, the protocol's limit. A line
/// is 48 ms of audio, so about 21 lines a second, 16 KB/s of text.
pub const SAMPLES_PER_LINE: usize = 1152;
/// A recording stops on its own after this much audio.
pub const MAX_SAMPLES: u32 = SAMPLE_RATE * 30;
/// If the samples stop coming, the recording still ends this long after it started.
const MAX_RECORDING_MS: u32 = 31_000;
/// Shorter than this is a slip of the finger: the turn is cancelled.
pub const MIN_RECORDING_MS: u32 = 500;
/// The ES8311's microphone gain while recording a voice command. At the 30 dB the bring-up diagnostic
/// defaults to, a voice close to the box was only about -31 dBFS; 42 dB (the most the PGA has) brings it
/// to about -19.
pub const GAIN_DB: u32 = 42;
/// With nothing from the computer for this long after the recording, the overlay closes.
pub const QUIET_CLOSE_MS: u32 = 60_000;
/// After `voice.done`, the replies stay up this long to be read (or until K2).
pub const READ_MS: u32 = 15_000;
/// An error stays up this long.
pub const ERROR_MS: u32 = 6_000;
/// Segments in the level meter.
pub const METER_STEPS: u8 = 16;
/// The meter shows the level of each 100 ms.
const LEVEL_WINDOW: u32 = SAMPLE_RATE / 10;
/// The meter's range: silence at the bottom, a voice up close near the top.
const METER_FLOOR_DB: f32 = -60.0;
const METER_TOP_DB: f32 = -10.0;
/// Replies kept for the overlay; older ones scroll away anyway.
const MAX_REPLIES: usize = 8;

/// A line for the computer. Data lines may only go where an audio stream fits (docs/protocol.md): the
/// device layer drops them, whole, rather than hold up the main loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    Line(String),
    Data(String),
}

/// Why a turn ended badly, from `voice.error`'s `reason`, or the box's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    NotHeard,
    Unavailable,
    Busy,
    /// A reason this firmware doesn't know.
    Other,
    /// The box couldn't start its microphone.
    NoMicrophone,
}

impl Reason {
    fn from_name(name: &str) -> Self {
        match name {
            "not_heard" => Self::NotHeard,
            "unavailable" => Self::Unavailable,
            "busy" => Self::Busy,
            _ => Self::Other,
        }
    }

    fn title(self) -> &'static [u8] {
        match self {
            Self::NotHeard => b"NOT HEARD",
            Self::Unavailable => b"UNAVAILABLE",
            Self::Busy => b"BUSY",
            Self::Other => b"VOICE ERROR",
            Self::NoMicrophone => b"NO MICROPHONE",
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::NotHeard => "Didn't catch that. Try again, a bit closer.",
            Self::Unavailable => "Voice isn't working on the computer right now.",
            Self::Busy => "Still on the last one.",
            Self::Other => "Something went wrong.",
            Self::NoMicrophone => "This box can't record.",
        }
    }
}

/// The recording of one turn: the encoder, the line being filled and the level meter.
struct Recording {
    turn: u32,
    started: u32,
    /// Samples encoded so far, including those in lines that were dropped on the way out.
    samples: u32,
    seq: u32,
    encoder: Encoder,
    /// The coder's state before the first nibble of the line being filled.
    line_state: (i16, u8),
    bytes: Vec<u8>,
    /// A sample waiting for the next one: a byte holds two.
    held: Option<i16>,
    /// The meter window: sum and sum of squares (the mean is taken out, the ADC has a DC offset).
    sum: i64,
    squares: u64,
    count: u32,
    level: u8,
}

impl Recording {
    fn new(turn: u32, now: u32) -> Self {
        Self {
            turn,
            started: now,
            samples: 0,
            seq: 0,
            encoder: Encoder::default(),
            line_state: (0, 0),
            bytes: Vec::with_capacity(SAMPLES_PER_LINE / 2),
            held: None,
            sum: 0,
            squares: 0,
            count: 0,
            level: 0,
        }
    }

    fn full(&self) -> bool {
        self.samples >= MAX_SAMPLES
    }

    fn push(&mut self, samples: &[i16], out: &mut Vec<Out>) {
        for &sample in samples {
            self.measure(sample);
            if self.full() {
                continue;
            }
            let Some(first) = self.held.take() else {
                self.held = Some(sample);
                continue;
            };
            if self.bytes.is_empty() {
                self.line_state = self.encoder.state();
            }
            self.bytes.push(self.encoder.encode_pair(first, sample));
            self.samples += 2;
            if self.bytes.len() * 2 == SAMPLES_PER_LINE {
                out.push(self.data_line());
            }
        }
    }

    fn measure(&mut self, sample: i16) {
        self.sum += sample as i64;
        self.squares += (sample as i64 * sample as i64) as u64;
        self.count += 1;
        if self.count == LEVEL_WINDOW {
            let mean = self.sum as f64 / self.count as f64;
            let power = (self.squares as f64 / self.count as f64 - mean * mean).max(0.0);
            self.level = meter_level(libm::sqrt(power) as f32);
            (self.sum, self.squares, self.count) = (0, 0, 0);
        }
    }

    fn data_line(&mut self) -> Out {
        let (predictor, index) = self.line_state;
        let mut line = alloc::format!("VOICE DATA {} {} {} {} ", self.turn, self.seq, predictor, index);
        line.push_str(&encode_base64(&self.bytes));
        line.push('\n');
        self.seq += 1;
        self.bytes.clear();
        Out::Data(line)
    }

    /// The last, shorter line and `VOICE END`. A sample left without a partner isn't sent.
    fn finish(&mut self, out: &mut Vec<Out>) {
        if !self.bytes.is_empty() {
            out.push(self.data_line());
        }
        out.push(Out::Line(alloc::format!("VOICE END {} {}\n", self.turn, self.samples)));
    }
}

/// An RMS in sample units to meter segments: -60 dBFS and below is none, -10 dBFS and above all.
pub fn meter_level(rms: f32) -> u8 {
    if rms < 1.0 {
        return 0;
    }
    let db = 20.0 * libm::log10f(rms / 32768.0);
    let steps = (db - METER_FLOOR_DB) / (METER_TOP_DB - METER_FLOOR_DB) * METER_STEPS as f32;
    (steps.max(0.0) as u8).min(METER_STEPS)
}

enum Stage {
    Idle,
    Listening(Recording),
    /// The recording went out; waiting for the computer. Quiet since the last thing it said.
    Thinking { turn: u32, heard: Option<String>, replies: Vec<String>, quiet_since: u32 },
    /// `voice.done`: the replies stay up to be read.
    Done { heard: Option<String>, replies: Vec<String>, until: u32 },
    Failed { reason: Reason, message: Option<String>, until: u32 },
}

pub struct VoiceCommand {
    available: bool,
    /// The last turn's number; turns count from 1 after each boot.
    turn: u32,
    stage: Stage,
}

impl Default for VoiceCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceCommand {
    pub const fn new() -> Self {
        Self { available: false, turn: 0, stage: Stage::Idle }
    }

    /// The computer offers voice: K0 long is push-to-talk.
    pub fn available(&self) -> bool {
        self.available
    }

    /// `voice.available`, or a new link (which starts with voice off). Withdrawn mid-recording, the turn is
    /// given up.
    pub fn set_available(&mut self, on: bool) -> Option<Out> {
        self.available = on;
        if on { None } else { self.cancel() }
    }

    /// The last turn's number.
    pub fn turn(&self) -> u32 {
        self.turn
    }

    pub fn listening(&self) -> bool {
        matches!(self.stage, Stage::Listening(_))
    }

    /// The overlay is up.
    /// Waiting for the computer's first reply: the THINKING panel animates, so it is redrawn each pass.
    pub fn waiting(&self) -> bool {
        matches!(&self.stage, Stage::Thinking { replies, .. } if replies.is_empty())
    }

    pub fn is_open(&self) -> bool {
        !matches!(self.stage, Stage::Idle)
    }

    /// K0 held long enough, with the microphone running: a new turn. A turn still waiting for its replies is
    /// left to the computer, which answers the new one `busy` if it must.
    pub fn start(&mut self, now: u32) -> Out {
        self.turn += 1;
        self.stage = Stage::Listening(Recording::new(self.turn, now));
        Out::Line(alloc::format!("VOICE START {}\n", self.turn))
    }

    /// The microphone couldn't start: no turn, just the overlay saying so.
    pub fn no_microphone(&mut self, now: u32) {
        self.fail(Reason::NoMicrophone, None, now);
    }

    /// The microphone's samples since the last call: `VOICE DATA` lines for each full line. Past 30 s of
    /// audio they only move the meter; [`Self::due`] then ends the recording.
    pub fn push(&mut self, samples: &[i16]) -> Vec<Out> {
        let mut out = Vec::new();
        if let Stage::Listening(recording) = &mut self.stage {
            recording.push(samples, &mut out);
        }
        out
    }

    /// The recording has run its 30 s (or the samples stopped coming): end it as if K0 were let go.
    pub fn due(&self, now: u32) -> bool {
        match &self.stage {
            Stage::Listening(recording) => recording.full() || now.wrapping_sub(recording.started) >= MAX_RECORDING_MS,
            _ => false,
        }
    }

    /// K0 let go (or the 30 s are up): `VOICE END`, or `VOICE CANCEL` for a press under half a second, which
    /// closes the overlay too. The caller has already handed over the last samples.
    pub fn release(&mut self, now: u32) -> Vec<Out> {
        let Stage::Listening(recording) = &mut self.stage else { return Vec::new() };
        let turn = recording.turn;
        if now.wrapping_sub(recording.started) < MIN_RECORDING_MS {
            self.stage = Stage::Idle;
            return alloc::vec![Out::Line(alloc::format!("VOICE CANCEL {turn}\n"))];
        }
        let mut out = Vec::new();
        recording.finish(&mut out);
        self.stage = Stage::Thinking { turn, heard: None, replies: Vec::new(), quiet_since: now };
        out
    }

    /// K2, or something that needs the user taking the screen: a recording is given up with `VOICE CANCEL`;
    /// anything else just closes, and the rest of that turn's events are ignored.
    pub fn cancel(&mut self) -> Option<Out> {
        let stage = core::mem::replace(&mut self.stage, Stage::Idle);
        match stage {
            Stage::Listening(recording) => Some(Out::Line(alloc::format!("VOICE CANCEL {}\n", recording.turn))),
            _ => None,
        }
    }

    /// The turn this event is about, if it is the one on the overlay.
    fn current(&self, turn: u32) -> bool {
        match &self.stage {
            Stage::Listening(recording) => recording.turn == turn,
            Stage::Thinking { turn: current, .. } => *current == turn,
            _ => false,
        }
    }

    /// `voice.heard`, `voice.reply`, `voice.done` or `voice.error` from the computer. Events for any turn but
    /// the one on the overlay are ignored. True when the overlay changed.
    pub fn event(&mut self, event: &str, turn: u32, text: Option<&str>, reason: Option<&str>, now: u32) -> bool {
        if !self.current(turn) {
            return false;
        }
        match event {
            "voice.error" => {
                let message = text.filter(|text| !text.is_empty()).map(String::from);
                self.fail(Reason::from_name(reason.unwrap_or("")), message, now);
                true
            }
            // Only a recording that has ended can be heard or answered; the computer is ahead of itself.
            _ if self.listening() => false,
            "voice.done" => {
                let Stage::Thinking { heard, replies, .. } = core::mem::replace(&mut self.stage, Stage::Idle) else { return false };
                // With nothing to read, there's nothing to wait for.
                if heard.is_some() || !replies.is_empty() {
                    self.stage = Stage::Done { heard, replies, until: now.wrapping_add(READ_MS) };
                }
                true
            }
            "voice.heard" | "voice.reply" => {
                let Stage::Thinking { heard, replies, quiet_since, .. } = &mut self.stage else { return false };
                *quiet_since = now;
                let Some(text) = text.filter(|text| !text.trim().is_empty()) else { return false };
                if event == "voice.heard" {
                    *heard = Some(String::from(text.trim()));
                } else {
                    if replies.len() == MAX_REPLIES {
                        replies.remove(0);
                    }
                    replies.push(String::from(text.trim()));
                }
                true
            }
            _ => false,
        }
    }

    fn fail(&mut self, reason: Reason, message: Option<String>, now: u32) {
        self.stage = Stage::Failed { reason, message, until: now.wrapping_add(ERROR_MS) };
    }

    /// Closes the overlay once its time is up: an error or the replies read, or the computer quiet for a
    /// minute. True if it just closed.
    pub fn expire(&mut self, now: u32) -> bool {
        let over = match &self.stage {
            Stage::Thinking { quiet_since, .. } => now.wrapping_sub(*quiet_since) >= QUIET_CLOSE_MS,
            Stage::Done { until, .. } | Stage::Failed { until, .. } => now.wrapping_sub(*until) as i32 >= 0,
            Stage::Idle | Stage::Listening(_) => false,
        };
        if over {
            self.stage = Stage::Idle;
        }
        over
    }

    /// How often THINKING gains a dot while it waits.
const THINKING_DOT_MS: u32 = 400;

/// What the overlay shows now; None when it is closed.
    pub fn panel(&self, now: u32) -> Option<Panel> {
        let close = alloc::vec![(&b"K2"[..], &b"CLOSE"[..])];
        match &self.stage {
            Stage::Idle => None,
            Stage::Listening(recording) => {
                let elapsed_s = now.wrapping_sub(recording.started) / 1000;
                let left = crate::text!(16, "{} S LEFT", 30u32.saturating_sub(elapsed_s));
                Some(Panel {
                    kind: PanelKind::Meter(recording.level),
                    title: b"LISTENING".to_vec(),
                    lines: alloc::vec![(left.as_bytes().to_vec(), Vec::new(), Tone::Dim)],
                    selected: None,
                    hints: alloc::vec![(b"K0", b"LET GO"), (b"K2", b"CANCEL")],
                })
            }
            Stage::Thinking { turn: _, heard, replies, quiet_since } => {
                if !replies.is_empty() {
                    return Some(text_panel(b"REPLY", heard.as_deref(), replies, close));
                }
                // Recognition and the resident session take seconds: the dots and the count keep the wait visibly
                // alive.
                let waited = now.wrapping_sub(*quiet_since);
                let mut title = b"THINKING".to_vec();
                title.extend(core::iter::repeat_n(b'.', (waited / Self::THINKING_DOT_MS % 4) as usize));
                let mut panel = text_panel(&title, heard.as_deref(), replies, close);
                panel.lines.push((crate::text!(8, "{} S", waited / 1000).as_bytes().to_vec(), Vec::new(), Tone::Dim));
                Some(panel)
            }
            Stage::Done { heard, replies, .. } => Some(text_panel(b"REPLY", heard.as_deref(), replies, close)),
            Stage::Failed { reason, message, .. } => {
                let text = [String::from(message.as_deref().unwrap_or(reason.text()))];
                Some(text_panel(reason.title(), None, &text, close))
            }
        }
    }
}

/// The heard text dimmed, then the replies, wrapped to the panel: large type while it fits, small type
/// after that, and past what small type holds, the oldest rows give way.
fn text_panel(title: &[u8], heard: Option<&str>, replies: &[String], hints: Vec<(&'static [u8], &'static [u8])>) -> Panel {
    let layout = |scale: u8| {
        let mut lines: Vec<(Vec<u8>, Vec<u8>, Tone)> = Vec::new();
        if let Some(heard) = heard {
            lines.extend(font::wrap(heard, VOICE_TEXT_WIDTH, scale as i32).into_iter().map(|row| (row, Vec::new(), Tone::Dim)));
        }
        for reply in replies {
            lines.extend(font::wrap(reply, VOICE_TEXT_WIDTH, scale as i32).into_iter().map(|row| (row, Vec::new(), Tone::Plain)));
        }
        let rows = voice_text_rows(scale, lines.iter().any(|(row, _, _)| font::has_wide(row)));
        let fits = lines.len() <= rows;
        if !fits {
            lines.drain(..lines.len() - rows);
        }
        (lines, fits)
    };
    let (lines, scale) = match layout(2) {
        (lines, true) => (lines, 2),
        _ => (layout(1).0, 1),
    };
    Panel { kind: PanelKind::Text(scale), title: title.to_vec(), lines, selected: None, hints }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adpcm::Decoder;
    use crate::voice_pack::decode_base64;
    use std::string::ToString;

    fn lines(out: &[Out]) -> Vec<String> {
        out.iter()
            .map(|out| match out {
                Out::Line(line) | Out::Data(line) => line.clone(),
            })
            .collect()
    }

    fn tone(level: i16, count: usize) -> Vec<i16> {
        (0..count).map(|index| if index % 2 == 0 { level } else { -level }).collect()
    }

    #[test]
    fn thinking_counts_dots_and_seconds_while_it_waits() {
        let mut voice = VoiceCommand::new();
        voice.set_available(true);
        voice.start(0);
        voice.push(&tone(3000, SAMPLES_PER_LINE));
        voice.release(1000);
        assert!(voice.waiting());
        let at = |now| voice.panel(now).unwrap();
        assert_eq!(at(1000).title, b"THINKING");
        assert_eq!(at(1000 + 2 * VoiceCommand::THINKING_DOT_MS).title, b"THINKING..");
        let panel = at(4500);
        assert_eq!(panel.lines.last().unwrap().0, b"3 S");
    }

    #[test]
    fn a_turn_streams_numbered_lines_and_ends_with_the_sample_count() {
        let mut voice = VoiceCommand::new();
        voice.set_available(true);
        assert_eq!(voice.start(1000), Out::Line("VOICE START 1\n".into()));
        // 2.5 lines' worth arrives in uneven pieces.
        let samples = tone(3000, SAMPLES_PER_LINE * 5 / 2);
        let mut out = Vec::new();
        for piece in samples.chunks(700) {
            out.extend(voice.push(piece));
        }
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], Out::Data(_)));
        out.extend(voice.release(2000));
        let lines = lines(&out);
        assert_eq!(lines.len(), 4);
        for (seq, line) in lines[..3].iter().enumerate() {
            assert!(line.starts_with(&std::format!("VOICE DATA 1 {seq} ")), "{line}");
            assert!(line.len() < 1024 && line.ends_with('\n'));
        }
        assert_eq!(lines[3], "VOICE END 1 2880\n");
        assert!(!voice.listening());
        assert!(voice.is_open(), "the overlay waits for the computer");
    }

    #[test]
    fn a_line_holds_768_base64_characters_and_decodes_on_its_own() {
        let mut voice = VoiceCommand::new();
        voice.start(0);
        let samples: Vec<i16> = (0..SAMPLES_PER_LINE * 3).map(|index| (libm::sinf(index as f32 * 0.04) * 8000.0) as i16).collect();
        let out = lines(&voice.push(&samples));
        assert_eq!(out.len(), 3);
        let mut decoded_lines = Vec::new();
        for line in &out {
            let fields: Vec<&str> = line.trim_end().split(' ').collect();
            assert_eq!(fields.len(), 7, "VOICE DATA turn seq predictor index base64");
            assert_eq!(fields[6].len(), 768);
            let (predictor, index): (i16, u8) = (fields[4].parse().unwrap(), fields[5].parse().unwrap());
            let mut bytes = [0u8; 576];
            assert_eq!(decode_base64(fields[6].as_bytes(), &mut bytes), Some(576));
            let mut decoder = Decoder::with_state(predictor, index);
            decoded_lines.push(bytes.iter().flat_map(|&byte| [byte & 15, byte >> 4]).map(|nibble| decoder.decode(nibble)).collect::<Vec<i16>>());
        }
        // Each line decoded from its own header continues the previous exactly: the third alone matches
        // the third stretch of the stream decoded from the start.
        let mut decoder = Decoder::default();
        let mut encoder = Encoder::default();
        let whole: Vec<i16> = samples
            .chunks(2)
            .flat_map(|pair| {
                let byte = encoder.encode_pair(pair[0], pair[1]);
                [decoder.decode(byte & 15), decoder.decode(byte >> 4)]
            })
            .collect();
        assert_eq!(decoded_lines[2], whole[SAMPLES_PER_LINE * 2..]);
        let longest = "VOICE DATA 4294967295 4294967295 -32768 88 ".len() + 768 + 1;
        assert!(longest < 1024, "the longest possible line is {longest} bytes");
    }

    #[test]
    fn under_half_a_second_is_cancelled_and_closes() {
        let mut voice = VoiceCommand::new();
        voice.start(1000);
        voice.push(&tone(100, 4000));
        assert_eq!(lines(&voice.release(1499)), ["VOICE CANCEL 1\n"]);
        assert!(!voice.is_open());
        // Turns keep counting.
        assert_eq!(voice.start(2000), Out::Line("VOICE START 2\n".into()));
        assert_eq!(lines(&voice.release(2500))[0], "VOICE END 2 0\n");
    }

    #[test]
    fn thirty_seconds_is_the_most_a_turn_records() {
        let mut voice = VoiceCommand::new();
        voice.start(0);
        let second = tone(500, SAMPLE_RATE as usize);
        let mut data = 0;
        for at in 1..=31 {
            data += voice.push(&second).len();
            if voice.due(at * 1000) {
                break;
            }
        }
        assert!(voice.due(30_000));
        let out = lines(&voice.release(30_000));
        assert_eq!(out.last().unwrap(), &std::format!("VOICE END 1 {MAX_SAMPLES}\n"));
        assert_eq!(data + out.len() - 1, (MAX_SAMPLES as usize).div_ceil(SAMPLES_PER_LINE));
    }

    #[test]
    fn a_recording_whose_samples_stop_still_ends() {
        let mut voice = VoiceCommand::new();
        voice.start(0);
        assert!(!voice.due(30_999));
        assert!(voice.due(31_000));
    }

    #[test]
    fn an_odd_sample_is_held_for_its_partner() {
        let mut voice = VoiceCommand::new();
        voice.start(0);
        voice.push(&[1]);
        voice.push(&[2, 3]);
        assert_eq!(lines(&voice.release(1000)).last().unwrap(), "VOICE END 1 2\n", "the third sample has no partner");
    }

    #[test]
    fn withdrawing_voice_or_k2_gives_up_a_recording() {
        let mut voice = VoiceCommand::new();
        voice.set_available(true);
        voice.start(0);
        assert_eq!(voice.set_available(false), Some(Out::Line("VOICE CANCEL 1\n".into())));
        assert!(!voice.available() && !voice.is_open());
        voice.set_available(true);
        voice.start(0);
        assert_eq!(voice.cancel(), Some(Out::Line("VOICE CANCEL 2\n".into())));
        voice.start(0);
        voice.release(1000);
        assert_eq!(voice.cancel(), None, "after the recording, closing says nothing");
        assert!(!voice.event("voice.reply", 3, Some("late"), None, 2000), "a closed turn's events are ignored");
    }

    #[test]
    fn heard_then_replies_then_done_then_read_time() {
        let mut voice = VoiceCommand::new();
        voice.start(0);
        voice.release(1000);
        assert_eq!(voice.panel(1000).unwrap().title, b"THINKING");
        assert!(!voice.event("voice.heard", 7, Some("other turn"), None, 1100));
        assert!(voice.event("voice.heard", 1, Some("what is codex doing"), None, 1200));
        assert!(voice.event("voice.reply", 1, Some("It's writing the tests."), None, 1300));
        assert!(voice.event("voice.reply", 1, Some("Almost done."), None, 1400));
        let panel = voice.panel(1400).unwrap();
        assert_eq!(panel.title, b"REPLY");
        assert_eq!(panel.kind, PanelKind::Text(2));
        let rows: Vec<(String, Tone)> = panel.lines.iter().map(|(row, _, tone)| (String::from_utf8(row.clone()).unwrap(), *tone)).collect();
        assert_eq!(
            rows,
            [
                ("what is codex doing".to_string(), Tone::Dim),
                ("It's writing the".to_string(), Tone::Plain),
                ("tests.".to_string(), Tone::Plain),
                ("Almost done.".to_string(), Tone::Plain),
            ]
        );
        assert!(voice.event("voice.done", 1, None, None, 2000));
        assert!(!voice.expire(2000 + READ_MS - 1));
        assert_eq!(voice.panel(2000).unwrap().title, b"REPLY");
        assert!(voice.expire(2000 + READ_MS));
        assert!(voice.panel(2000 + READ_MS).is_none());
    }

    #[test]
    fn done_with_nothing_to_read_closes_at_once() {
        let mut voice = VoiceCommand::new();
        voice.start(0);
        voice.release(1000);
        assert!(voice.event("voice.done", 1, None, None, 1500));
        assert!(!voice.is_open());
    }

    #[test]
    fn a_quiet_computer_closes_the_overlay_after_a_minute() {
        let mut voice = VoiceCommand::new();
        voice.start(0);
        voice.release(1000);
        voice.event("voice.heard", 1, Some("hello"), None, 20_000);
        assert!(!voice.expire(20_000 + QUIET_CLOSE_MS - 1), "each event restarts the wait");
        assert!(voice.expire(20_000 + QUIET_CLOSE_MS));
    }

    #[test]
    fn errors_show_a_short_message_and_go() {
        let mut voice = VoiceCommand::new();
        voice.start(0);
        voice.release(1000);
        assert!(voice.event("voice.error", 1, None, Some("not_heard"), 3000));
        let panel = voice.panel(3000).unwrap();
        assert_eq!(panel.title, b"NOT HEARD");
        assert_eq!(panel.lines[0].0, b"Didn't catch that.");
        assert!(voice.expire(3000 + ERROR_MS));

        // The computer's own message wins; and `busy` can come while still recording, which ends it.
        voice.start(10_000);
        assert!(voice.event("voice.error", 2, Some("Another turn is running"), Some("busy"), 10_100));
        assert!(!voice.listening());
        let panel = voice.panel(10_100).unwrap();
        assert_eq!((panel.title.as_slice(), panel.lines[0].0.as_slice()), (&b"BUSY"[..], &b"Another turn is"[..]));
        assert!(!voice.event("voice.reply", 2, Some("x"), None, 10_200), "the turn is over");
    }

    #[test]
    fn the_overlay_says_listening_with_a_meter_and_time_left() {
        let mut voice = VoiceCommand::new();
        voice.start(1000);
        assert_eq!(voice.panel(1000).unwrap().kind, PanelKind::Meter(0));
        // 100 ms of a loud tone on a DC offset: the offset doesn't count.
        let loud: Vec<i16> = tone(8000, LEVEL_WINDOW as usize).iter().map(|sample| sample + 2000).collect();
        voice.push(&loud);
        let panel = voice.panel(4500).unwrap();
        assert_eq!(panel.title, b"LISTENING");
        assert_eq!(panel.kind, PanelKind::Meter(meter_level(8000.0)));
        assert_eq!(panel.lines[0].0, b"27 S LEFT");
        assert_eq!(panel.hints, [(&b"K0"[..], &b"LET GO"[..]), (&b"K2"[..], &b"CANCEL"[..])]);
    }

    #[test]
    fn the_meter_spans_silence_to_a_voice_up_close() {
        assert_eq!(meter_level(0.0), 0);
        assert_eq!(meter_level(30.0), 0, "-61 dBFS");
        // A close voice at 42 dB, about -19 dBFS, is high on the meter but not pinned.
        let close = meter_level(32768.0 * libm::powf(10.0, -19.0 / 20.0));
        assert!((12..METER_STEPS).contains(&close), "{close}");
        assert_eq!(meter_level(32767.0), METER_STEPS);
    }

    #[test]
    fn long_text_moves_to_small_type_and_keeps_the_newest() {
        let mut voice = VoiceCommand::new();
        voice.start(0);
        voice.release(1000);
        voice.event("voice.heard", 1, Some("tell me everything"), None, 1000);
        let long = "word ".repeat(40);
        voice.event("voice.reply", 1, Some(&long), None, 1000);
        let panel = voice.panel(1000).unwrap();
        assert_eq!(panel.kind, PanelKind::Text(1));
        for _ in 0..5 {
            voice.event("voice.reply", 1, Some(&long), None, 1000);
        }
        voice.event("voice.reply", 1, Some("the end"), None, 1000);
        let panel = voice.panel(1000).unwrap();
        assert_eq!(panel.lines.len(), voice_text_rows(1, false));
        assert_eq!(panel.lines.last().unwrap().0, b"the end");
    }
}
