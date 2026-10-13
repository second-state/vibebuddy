//! Voice commands from a box linked over USB or the local network (ADR-0017; docs/protocol.md, "Voice commands
//! (experimental)").
//!
//! The box records while K0 is held and sends the audio as `VOICE` diagnostic lines: 24 kHz mono IMA ADPCM, each
//! line carrying the coder's state so it decodes on its own. On `VOICE END` the daemon decodes the turn, has it
//! recognised ([`crate::speech`]), sends `voice.heard`, hands the text to the resident session and sends each spoken
//! reply as `voice.reply` the moment the session says it, then `voice.done`. One turn at a time: a turn that ends
//! while another is still running gets `voice.error` `busy`.
//!
//! With a synthesizer ([`crate::synthesis`]), each reply is also spoken: its audio follows its `voice.reply` as
//! `voice.audio` pieces, streamed as the service makes them. Replies are synthesized one after another, in order,
//! on their own task, so a reply's text never waits for an earlier reply's audio; `voice.done` waits for the last
//! reply's audio `end`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use base64::Engine;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use vibebuddy_protocol::{Event, MAX_MESSAGE_BYTES};

use crate::resident::Resident;
use crate::serial_transport::{Transport, TransportError};
use crate::speech::Recognizer;
use crate::synthesis::{self, Synthesizer};

pub const SAMPLE_RATE: u32 = 24_000;
/// The most MP3 bytes one `voice.audio` carries: 768 base64 characters.
const AUDIO_BYTES: usize = 768 / 4 * 3;
/// The box stops at 30 s; a little more is kept in case its clock and ours disagree, nothing beyond.
const MAX_SAMPLES: usize = SAMPLE_RATE as usize * 31;
const LINE_PREFIX: &str = "VOICE ";

// IMA ADPCM, exactly as firmware-rs/core/src/adpcm.rs decodes it; the box encodes with its inverse.
const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130,
    143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166,
    1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845,
    8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX_CHANGE: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decoder {
    predictor: i32,
    index: i32,
}

impl Decoder {
    pub fn new(predictor: i16, index: u8) -> Self {
        Self { predictor: predictor.into(), index: i32::from(index).min(88) }
    }

    pub fn decode(&mut self, nibble: u8) -> i16 {
        let step = STEPS[self.index as usize];
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
        self.predictor = if nibble & 8 != 0 { self.predictor - diff } else { self.predictor + diff }.clamp(-32768, 32767);
        self.index = (self.index + INDEX_CHANGE[(nibble & 15) as usize]).clamp(0, 88);
        self.predictor as i16
    }
}

/// Two samples per byte, low nibble first.
pub fn decode_line(predictor: i16, index: u8, bytes: &[u8]) -> Vec<i16> {
    let mut decoder = Decoder::new(predictor, index);
    bytes.iter().flat_map(|byte| [byte & 15, byte >> 4]).map(|nibble| decoder.decode(nibble)).collect()
}

#[derive(Debug, PartialEq)]
pub enum VoiceLine {
    Start { turn: u32 },
    Data { turn: u32, seq: u32, predictor: i16, index: u8, bytes: Vec<u8> },
    End { turn: u32, samples: u32 },
    Cancel { turn: u32 },
}

/// None when the line isn't a `VOICE` line; an error when it is one but malformed.
pub fn parse_line(line: &str) -> Option<Result<VoiceLine, String>> {
    let rest = line.trim().strip_prefix(LINE_PREFIX)?;
    let fields: Vec<&str> = rest.split(' ').collect();
    let number = |index: usize| fields.get(index).and_then(|field| field.parse::<u32>().ok());
    let parsed = match (fields[0], fields.len()) {
        ("START", 2) => number(1).map(|turn| VoiceLine::Start { turn }),
        ("END", 3) => number(1).zip(number(2)).map(|(turn, samples)| VoiceLine::End { turn, samples }),
        ("CANCEL", 2) => number(1).map(|turn| VoiceLine::Cancel { turn }),
        ("DATA", 6) => (|| {
            let bytes = base64::engine::general_purpose::STANDARD.decode(fields[5]).ok()?;
            let index = fields[4].parse::<u8>().ok().filter(|index| *index <= 88)?;
            Some(VoiceLine::Data { turn: number(1)?, seq: number(2)?, predictor: fields[3].parse().ok()?, index, bytes })
        })(),
        _ => None,
    };
    Some(parsed.ok_or_else(|| format!("malformed voice line: {}", shorten(rest))))
}

fn shorten(text: &str) -> &str {
    let mut end = text.len().min(60);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// One turn's audio as it arrives, each line already decoded, by seq.
struct Recording {
    turn: u32,
    lines: BTreeMap<u32, Vec<i16>>,
    samples: usize,
    /// Answered `busy` at its start: the rest of it is dropped without another answer.
    refused: bool,
}

/// The turn's audio, in order, `samples` long; a lost line becomes silence as long as a line, and audio lost at the
/// end is filled up to `samples`. Returns the seqs that were missing.
fn assemble(lines: &BTreeMap<u32, Vec<i16>>, samples: usize) -> (Vec<i16>, Vec<u32>) {
    let samples = samples.min(MAX_SAMPLES);
    let line_length = lines.values().map(Vec::len).max().unwrap_or(0);
    let mut audio = Vec::with_capacity(samples);
    let mut missing = Vec::new();
    let mut next = 0;
    for (&seq, line) in lines {
        for lost in next..seq {
            missing.push(lost);
            audio.resize(audio.len() + line_length, 0);
        }
        audio.extend_from_slice(line);
        next = seq + 1;
    }
    while audio.len() < samples && line_length > 0 {
        missing.push(next);
        next += 1;
        audio.resize(audio.len() + line_length, 0);
    }
    audio.resize(samples, 0);
    (audio, missing)
}

/// A 24 kHz 16-bit mono WAV file.
pub fn wav(samples: &[i16]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// `voice.available`: whether the box may treat a long K0 as push-to-talk.
pub fn available_event(on: bool) -> Event {
    let mut event = Event::named("voice.available");
    event.extra.insert("on".to_owned(), on.into());
    event
}

fn turn_event(name: &str, turn: u32) -> Event {
    let mut event = Event::named(name);
    event.extra.insert("turn".to_owned(), turn.into());
    event
}

/// An event carrying `text`, cut at a character boundary (with an ellipsis) until the line fits.
fn fitted(text: &str, build: impl Fn(String) -> Event) -> Event {
    let mut keep = text.len();
    loop {
        let shown = if keep < text.len() { format!("{}…", &text[..keep]) } else { text.to_owned() };
        let event = build(shown);
        let size = serde_json::to_vec(&event).map(|bytes| bytes.len()).unwrap_or(0);
        if size <= MAX_MESSAGE_BYTES || keep == 0 {
            return event;
        }
        // Every byte of text is at least one byte of JSON, so this many fewer is always enough room.
        keep = keep.saturating_sub(size - MAX_MESSAGE_BYTES + '…'.len_utf8());
        while !text.is_char_boundary(keep) {
            keep -= 1;
        }
    }
}

fn text_event(name: &str, turn: u32, text: &str) -> Event {
    fitted(text, |text| {
        let mut event = turn_event(name, turn);
        event.extra.insert("text".to_owned(), text.into());
        event
    })
}

fn error_event(turn: u32, reason: &str, message: &str) -> Event {
    fitted(message, |message| {
        let mut event = turn_event("voice.error", turn);
        event.extra.insert("reason".to_owned(), reason.into());
        event.message = Some(message);
        event
    })
}

fn audio_event(turn: u32, reply: u32, seq: u32) -> Event {
    let mut event = turn_event("voice.audio", turn);
    event.extra.insert("reply".to_owned(), reply.into());
    event.extra.insert("seq".to_owned(), seq.into());
    event
}

fn send(transport: &dyn Transport, event: Event) {
    match event.to_ndjson() {
        Ok(frame) => {
            if let Err(error) = transport.send(frame) {
                warn!(?error, event = %event.event, "a voice event could not be queued for the box");
            }
        }
        Err(error) => warn!(%error, "failed to encode a voice event"),
    }
}

/// Clears the running flag however the turn ends.
struct Running(Arc<AtomicBool>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub struct Voice {
    recognizer: Option<Arc<dyn Recognizer>>,
    resident: Option<Arc<Resident>>,
    /// Speaks the replies; without one they stay text.
    synthesizer: Option<Arc<dyn Synthesizer>>,
    recording: std::sync::Mutex<Option<Recording>>,
    running: Arc<AtomicBool>,
}

impl Voice {
    pub fn new(recognizer: Option<Arc<dyn Recognizer>>, resident: Option<Arc<Resident>>) -> Self {
        Self { recognizer, resident, synthesizer: None, recording: std::sync::Mutex::new(None), running: Arc::default() }
    }

    pub fn with_synthesizer(self, synthesizer: Option<Arc<dyn Synthesizer>>) -> Self {
        Self { synthesizer, ..self }
    }

    /// Voice is offered when the resident session is on and speech recognition is configured.
    pub fn available(&self) -> bool {
        self.recognizer.is_some() && self.resident.as_ref().is_some_and(|resident| resident.enabled())
    }

    /// Takes one `VOICE` line from the box. A turn that ends starts a task, returned so tests can wait for it;
    /// its replies are spoken in the voice of `character`, the Character the box wears.
    pub fn handle_line(&self, line: &str, transport: &Arc<dyn Transport>, character: Option<&str>) -> Option<JoinHandle<()>> {
        let parsed = match parse_line(line) {
            None => return None,
            Some(Err(error)) => {
                warn!(%error, "ignoring a voice line");
                return None;
            }
            Some(Ok(parsed)) => parsed,
        };
        let mut recording = self.recording.lock().expect("recording lock");
        match parsed {
            VoiceLine::Start { turn } => {
                if let Some(old) = recording.as_ref().filter(|old| old.turn != turn) {
                    warn!(old = old.turn, new = turn, "a voice turn started before the last one ended; dropped the last one");
                }
                // Refused at once rather than at its end, so the user isn't left talking to a box that won't listen.
                let refused = self.running.load(Ordering::Acquire);
                if refused {
                    info!(turn, "a voice turn started while another is still running");
                    send(&**transport, error_event(turn, "busy", "another voice command is still running"));
                } else {
                    info!(turn, "voice turn recording");
                }
                *recording = Some(Recording { turn, lines: BTreeMap::new(), samples: 0, refused });
            }
            VoiceLine::Data { turn, seq, predictor, index, bytes } => {
                if recording.as_ref().is_none_or(|current| current.turn != turn) {
                    // Its START was lost: the audio still counts.
                    debug!(turn, "voice data without its start");
                    *recording = Some(Recording { turn, lines: BTreeMap::new(), samples: 0, refused: false });
                }
                let current = recording.as_mut().expect("recording was just made");
                if current.refused {
                    return None;
                }
                let line = decode_line(predictor, index, &bytes);
                if current.samples + line.len() > MAX_SAMPLES {
                    debug!(turn, seq, "voice turn over the longest recording; line dropped");
                } else {
                    current.samples += line.len();
                    current.lines.insert(seq, line);
                }
            }
            VoiceLine::Cancel { turn } => {
                if recording.as_ref().is_some_and(|current| current.turn == turn) {
                    *recording = None;
                }
                info!(turn, "voice turn cancelled by the box");
            }
            VoiceLine::End { turn, samples } => {
                let taken = recording.take_if(|current| current.turn == turn);
                drop(recording);
                if taken.as_ref().is_some_and(|taken| taken.refused) {
                    return None;
                }
                let speaker = self.synthesizer.clone().map(|synthesizer| (synthesizer, character.map(str::to_owned)));
                return self.end(turn, samples as usize, taken, transport.clone(), speaker);
            }
        }
        None
    }

    fn end(
        &self,
        turn: u32,
        samples: usize,
        recording: Option<Recording>,
        transport: Arc<dyn Transport>,
        speaker: Option<(Arc<dyn Synthesizer>, Option<String>)>,
    ) -> Option<JoinHandle<()>> {
        let (Some(recognizer), Some(resident)) = (self.recognizer.clone(), self.resident.clone().filter(|resident| resident.enabled()))
        else {
            send(&*transport, error_event(turn, "unavailable", "voice commands are off on this computer"));
            return None;
        };
        let Some(recording) = recording.filter(|recording| !recording.lines.is_empty()) else {
            warn!(turn, "a voice turn ended with no audio");
            send(&*transport, error_event(turn, "not_heard", "no audio arrived"));
            return None;
        };
        if self.running.swap(true, Ordering::AcqRel) {
            info!(turn, "a voice turn ended while another is still running");
            send(&*transport, error_event(turn, "busy", "another voice command is still running"));
            return None;
        }
        let running = Running(self.running.clone());
        let (audio, missing) = assemble(&recording.lines, samples);
        if !missing.is_empty() {
            warn!(turn, lost = missing.len(), seqs = ?missing, "voice lines were lost; filled with silence");
        }
        Some(tokio::spawn(async move {
            let _running = running;
            run_turn(turn, audio, recognizer, resident, transport, speaker).await;
        }))
    }
}

/// Speaks a turn's replies one after another, in the order they were said, streaming each one's audio to the box
/// as it arrives, on a task of its own so the text never waits for it.
struct Speaker {
    replies: tokio::sync::mpsc::UnboundedSender<String>,
    stopped: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

impl Speaker {
    fn start(turn: u32, synthesizer: Arc<dyn Synthesizer>, character: Option<String>, transport: Arc<dyn Transport>) -> Self {
        let (replies, mut texts) = tokio::sync::mpsc::unbounded_channel::<String>();
        let stopped = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn({
            let stopped = stopped.clone();
            async move {
                let mut reply = 0;
                while let Some(text) = texts.recv().await {
                    let (synthesizer, character, transport, stopped) =
                        (synthesizer.clone(), character.clone(), transport.clone(), stopped.clone());
                    let spoken = tokio::task::spawn_blocking(move || {
                        speak(turn, reply, &text, character.as_deref(), &*synthesizer, &*transport, &stopped)
                    });
                    if let Err(error) = spoken.await {
                        warn!(turn, reply, %error, "speech synthesis panicked");
                    }
                    reply += 1;
                }
            }
        });
        Self { replies, stopped, task }
    }

    fn say(&self, text: &str) {
        let _ = self.replies.send(text.to_owned());
    }

    /// Waits until every reply's audio has been sent.
    async fn finish(self) {
        drop(self.replies);
        let _ = self.task.await;
    }

    /// Sends no more audio: the reply being spoken stops at its next piece, and the rest are never started.
    async fn stop(self) {
        self.stopped.store(true, Ordering::Release);
        self.finish().await;
    }
}

/// One reply's audio, as `voice.audio` pieces and then its `end`; a failure ends it early, marked as one, and the
/// box keeps the text. Blocking.
fn speak(
    turn: u32,
    reply: u32,
    text: &str,
    character: Option<&str>,
    synthesizer: &dyn Synthesizer,
    transport: &dyn Transport,
    stopped: &AtomicBool,
) {
    if stopped.load(Ordering::Acquire) {
        return;
    }
    let voice = synthesis::voice_for(character, text);
    let started = Instant::now();
    let (mut seq, mut bytes, mut first_byte_ms) = (0, 0, None);
    let result = synthesizer.synthesize(text, voice, &mut |piece| {
        if stopped.load(Ordering::Acquire) {
            return false;
        }
        first_byte_ms.get_or_insert(started.elapsed().as_millis() as u64);
        bytes += piece.len();
        for chunk in piece.chunks(AUDIO_BYTES) {
            let mut event = audio_event(turn, reply, seq);
            event.extra.insert("data".to_owned(), base64::engine::general_purpose::STANDARD.encode(chunk).into());
            if !send_paced(transport, event, stopped) {
                return false;
            }
            seq += 1;
        }
        true
    });
    if stopped.load(Ordering::Acquire) {
        return;
    }
    let total_ms = started.elapsed().as_millis() as u64;
    let mut end = audio_event(turn, reply, seq);
    end.extra.insert("end".to_owned(), true.into());
    match result {
        Ok(()) => info!(turn, reply, voice, first_byte_ms, total_ms, bytes, "spoken reply synthesized"),
        Err(error) => {
            warn!(turn, reply, voice, %error, bytes, "speech synthesis failed; the reply stays text");
            end.extra.insert("error".to_owned(), "synthesis".into());
        }
    }
    send_paced(transport, end, stopped);
}

/// How long one piece of a reply's audio may wait for room in the link's send queue.
const AUDIO_QUEUE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Sends a piece of a reply's audio, waiting while the link's send queue is full: a reply arrives from the voice
/// service far faster than the link drains, and a dropped piece cuts the reply short on the box. Blocking. False when
/// it gave up (stopped, link closed, or no room for too long).
fn send_paced(transport: &dyn Transport, event: Event, stopped: &AtomicBool) -> bool {
    let Ok(frame) = event.to_ndjson() else {
        warn!(event = %event.event, "a voice event could not be encoded");
        return false;
    };
    let started = Instant::now();
    loop {
        match transport.send(frame.clone()) {
            Ok(()) => return true,
            Err(TransportError::QueueFull) if !stopped.load(Ordering::Acquire) && started.elapsed() < AUDIO_QUEUE_WAIT => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => {
                warn!(?error, event = %event.event, "a voice event could not be queued for the box");
                return false;
            }
        }
    }
}

async fn run_turn(
    turn: u32,
    audio: Vec<i16>,
    recognizer: Arc<dyn Recognizer>,
    resident: Arc<Resident>,
    transport: Arc<dyn Transport>,
    speaker: Option<(Arc<dyn Synthesizer>, Option<String>)>,
) {
    let started = Instant::now();
    let seconds = audio.len() as f32 / SAMPLE_RATE as f32;
    let wav = wav(&audio);
    let heard = tokio::task::spawn_blocking(move || recognizer.recognize(&wav))
        .await
        .unwrap_or_else(|error| Err(error.to_string()));
    let recognised_ms = started.elapsed().as_millis() as u64;
    let transcript = match heard {
        Ok(transcript) => transcript,
        Err(error) => {
            warn!(turn, %error, "speech recognition failed");
            send(&*transport, error_event(turn, "unavailable", &error));
            return;
        }
    };
    if transcript.text.is_empty() {
        info!(turn, seconds, recognised_ms, "nothing was heard in the voice turn");
        send(&*transport, error_event(turn, "not_heard", "nothing was recognised"));
        return;
    }
    info!(turn, seconds, recognised_ms, language = transcript.language.as_deref().unwrap_or("unknown"), chars = transcript.text.chars().count(), "voice turn heard");
    send(&*transport, text_event("voice.heard", turn, &transcript.text));
    let speaker = speaker.map(|(synthesizer, character)| Speaker::start(turn, synthesizer, character, transport.clone()));
    let mut replies = 0;
    let answered = resident
        .say_streaming(&transcript.text, &mut |reply| {
            replies += 1;
            info!(turn, elapsed_ms = started.elapsed().as_millis() as u64, "voice reply");
            send(&*transport, text_event("voice.reply", turn, reply));
            if let Some(speaker) = &speaker {
                speaker.say(reply);
            }
        })
        .await;
    match answered {
        Ok(_) => {
            // The turn is over once the last reply has been heard, not just said.
            if let Some(speaker) = speaker {
                speaker.finish().await;
            }
            info!(turn, replies, elapsed_ms = started.elapsed().as_millis() as u64, "voice turn done");
            send(&*transport, turn_event("voice.done", turn));
        }
        Err(error) => {
            if let Some(speaker) = speaker {
                speaker.stop().await;
            }
            warn!(turn, %error, "the resident session failed the voice turn");
            send(&*transport, error_event(turn, "unavailable", &error.to_string()));
        }
    }
}

#[cfg(test)]
pub mod tests {
    use serde_json::json;

    use super::*;

    /// The lines the box itself sends for a turn of `samples`: made by firmware-core's own voice command code,
    /// so these tests hold the daemon to what the box really writes.
    pub fn voice_lines(turn: u32, samples: &[i16]) -> Vec<String> {
        use vibebuddy_firmware_core::voice_command::{Out, VoiceCommand};
        let mut box_side = VoiceCommand::new();
        box_side.set_available(true);
        // Turns count from 1 after each boot.
        for _ in 1..turn {
            box_side.start(0);
            box_side.cancel();
        }
        let mut out = vec![box_side.start(0)];
        for piece in samples.chunks(700) {
            out.extend(box_side.push(piece));
        }
        out.extend(box_side.release(10_000));
        out.into_iter()
            .map(|out| match out {
                Out::Line(line) | Out::Data(line) => line.trim_end().to_owned(),
            })
            .collect()
    }

    pub fn sine(count: usize) -> Vec<i16> {
        (0..count).map(|index| ((index as f32 * 0.05).sin() * 12000.0) as i16).collect()
    }

    #[test]
    fn lines_parse_and_malformed_ones_are_told_apart_from_other_lines() {
        assert_eq!(parse_line("VOICE START 3"), Some(Ok(VoiceLine::Start { turn: 3 })));
        assert_eq!(parse_line("VOICE END 3 12000\r"), Some(Ok(VoiceLine::End { turn: 3, samples: 12000 })));
        assert_eq!(parse_line("VOICE CANCEL 3"), Some(Ok(VoiceLine::Cancel { turn: 3 })));
        assert_eq!(
            parse_line("VOICE DATA 3 7 -1200 42 AQL/"),
            Some(Ok(VoiceLine::Data { turn: 3, seq: 7, predictor: -1200, index: 42, bytes: vec![1, 2, 255] }))
        );
        assert!(parse_line("VOICES hsiaoyu").is_none());
        assert!(parse_line("MODE LEISURE").is_none());
        for bad in ["VOICE DATA 3 7 -1200 89 AQL/", "VOICE DATA 3 7 40000 1 AQL/", "VOICE DATA 3 x 0 0 AQL/", "VOICE DATA 3 7 0 0 !!", "VOICE END 3", "VOICE SING 1"] {
            assert!(matches!(parse_line(bad), Some(Err(_))), "{bad}");
        }
    }

    /// The decoder here and the box's are the same: fed the same nibbles from zero, they give the same samples.
    #[test]
    fn the_decoder_matches_firmware_core() {
        let mut ours = Decoder::new(0, 0);
        let mut box_side = vibebuddy_firmware_core::adpcm::Decoder::default();
        let mut seed = 0x1234_5678u32;
        for _ in 0..200_000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let nibble = (seed >> 28) as u8;
            assert_eq!(ours.decode(nibble), box_side.decode(nibble));
        }
    }

    /// A line decoded on its own, from the state it carries, is the same as the stream decoded straight through.
    #[test]
    fn a_line_decodes_on_its_own_from_the_state_it_carries() {
        let original = sine(10_000);
        let lines = voice_lines(1, &original);
        let mut box_side = vibebuddy_firmware_core::adpcm::Decoder::default();
        let mut straight = Vec::new();
        let mut separately = Vec::new();
        for line in &lines {
            if let Some(Ok(VoiceLine::Data { predictor, index, bytes, .. })) = parse_line(line) {
                straight.extend(bytes.iter().flat_map(|byte| [byte & 15, byte >> 4]).map(|nibble| box_side.decode(nibble)));
                separately.extend(decode_line(predictor, index, &bytes));
            }
        }
        assert_eq!(straight[..10_000], separately[..10_000]);
        for (k, (&decoded, &expected)) in separately.iter().zip(&original).enumerate().skip(60) {
            assert!((i32::from(decoded) - i32::from(expected)).abs() < 600, "sample {k}: {decoded} vs {expected}");
        }
    }

    #[test]
    fn lost_lines_become_silence_of_a_lines_length() {
        let lines: BTreeMap<u32, Vec<i16>> = [(0, vec![1; 4]), (2, vec![3; 4]), (3, vec![4; 2])].into();
        let (audio, missing) = assemble(&lines, 14);
        assert_eq!(audio, [1, 1, 1, 1, 0, 0, 0, 0, 3, 3, 3, 3, 4, 4]);
        assert_eq!(missing, [1]);
        // Lines lost at the end are filled up to what END says was recorded.
        let lines: BTreeMap<u32, Vec<i16>> = [(1, vec![2; 4])].into();
        let (audio, missing) = assemble(&lines, 11);
        assert_eq!(audio, [0, 0, 0, 0, 2, 2, 2, 2, 0, 0, 0]);
        assert_eq!(missing, [0, 2]);
        // An odd count cuts the padding nibble of the last byte.
        let lines: BTreeMap<u32, Vec<i16>> = [(0, vec![5; 4])].into();
        assert_eq!(assemble(&lines, 3).0, [5, 5, 5]);
    }

    #[test]
    fn audio_waits_for_room_in_a_full_queue_instead_of_dropping() {
        struct Busy {
            refusals: std::sync::Mutex<u32>,
            sent: std::sync::Mutex<u32>,
        }
        impl Transport for Busy {
            fn send(&self, _frame: Vec<u8>) -> Result<(), crate::serial_transport::TransportError> {
                let mut refusals = self.refusals.lock().unwrap();
                if *refusals > 0 {
                    *refusals -= 1;
                    return Err(crate::serial_transport::TransportError::QueueFull);
                }
                *self.sent.lock().unwrap() += 1;
                Ok(())
            }
        }
        let busy = Busy { refusals: std::sync::Mutex::new(3), sent: std::sync::Mutex::new(0) };
        let stopped = AtomicBool::new(false);
        assert!(send_paced(&busy, audio_event(1, 0, 0), &stopped));
        assert_eq!(*busy.sent.lock().unwrap(), 1);
        // Stopped: gives up on a full queue at once.
        *busy.refusals.lock().unwrap() = 1;
        stopped.store(true, Ordering::Release);
        assert!(!send_paced(&busy, audio_event(1, 0, 1), &stopped));
    }

    #[test]
    fn the_wav_header_is_24_khz_16_bit_mono() {
        let file = wav(&[1, -1]);
        assert_eq!(file.len(), 48);
        assert_eq!(&file[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(file[24..28].try_into().unwrap()), 24_000);
        assert_eq!(u16::from_le_bytes(file[22..24].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(file[34..36].try_into().unwrap()), 16);
        assert_eq!(&file[44..], [1, 0, 255, 255]);
    }

    #[test]
    fn long_text_is_cut_at_a_character_to_fit_the_line() {
        let long = "语音".repeat(400);
        let event = text_event("voice.heard", 1, &long);
        let frame = event.to_ndjson().expect("fits");
        assert!(frame.len() <= MAX_MESSAGE_BYTES + 1);
        let text = event.extra["text"].as_str().unwrap();
        assert!(text.ends_with('…') && long.starts_with(text.trim_end_matches('…')));
        assert!(frame.len() > MAX_MESSAGE_BYTES - 8, "cut no more than needed: {}", frame.len());
        // Quotes take two bytes of JSON each.
        let quoted = "\"".repeat(2000);
        assert!(text_event("voice.reply", 1, &quoted).to_ndjson().is_ok());
        assert_eq!(text_event("voice.reply", 1, "short").extra["text"], "short");
    }

    #[derive(Default)]
    pub struct RecordingTransport {
        frames: std::sync::Mutex<Vec<Vec<u8>>>,
    }

    impl Transport for RecordingTransport {
        fn send(&self, frame: Vec<u8>) -> Result<(), crate::serial_transport::TransportError> {
            self.frames.lock().unwrap().push(frame);
            Ok(())
        }
    }

    impl RecordingTransport {
        pub fn events(&self) -> Vec<Event> {
            self.frames.lock().unwrap().iter().map(|frame| serde_json::from_slice(frame).unwrap()).collect()
        }
    }

    /// Answers every recording with the same result, and keeps the WAV files it was given.
    struct FakeRecognizer {
        answer: Result<&'static str, &'static str>,
        heard: std::sync::Mutex<Vec<Vec<u8>>>,
    }

    impl Recognizer for FakeRecognizer {
        fn recognize(&self, wav: &[u8]) -> Result<crate::speech::Transcript, String> {
            self.heard.lock().unwrap().push(wav.to_vec());
            self.answer
                .map(|text| crate::speech::Transcript { text: text.to_owned(), language: Some("en".to_owned()) })
                .map_err(str::to_owned)
        }
    }

    use crate::resident::tests::{Shared, logged_in, resident, temp_dir, tool_call};

    /// Answers every reply with the same pieces of "audio", failing after them for texts that start with "fail",
    /// and keeps the text and voice of each call. With a gate, its first call waits until the gate opens.
    #[derive(Default)]
    struct FakeSynthesizer {
        pieces: Vec<Vec<u8>>,
        calls: std::sync::Mutex<Vec<(String, String)>>,
        gate: std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    }

    impl Synthesizer for FakeSynthesizer {
        fn synthesize(&self, text: &str, voice: &str, on_audio: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), String> {
            self.calls.lock().unwrap().push((text.to_owned(), voice.to_owned()));
            if let Some(gate) = self.gate.lock().unwrap().take() {
                let _ = gate.recv();
            }
            for piece in &self.pieces {
                if !on_audio(piece) {
                    return Ok(());
                }
            }
            if text.starts_with("fail") { Err("the connection was cut".to_owned()) } else { Ok(()) }
        }
    }

    struct Rig {
        voice: Voice,
        transport: Arc<RecordingTransport>,
        recognizer: Arc<FakeRecognizer>,
        script: Shared,
        dir: std::path::PathBuf,
        character: Option<&'static str>,
    }

    impl Rig {
        fn new(name: &str, answer: Result<&'static str, &'static str>, script: Shared) -> Self {
            Self::speaking(name, answer, script, None, None)
        }

        fn speaking(
            name: &str,
            answer: Result<&'static str, &'static str>,
            script: Shared,
            synthesizer: Option<Arc<dyn Synthesizer>>,
            character: Option<&'static str>,
        ) -> Self {
            let dir = temp_dir(&format!("voice-{name}"));
            let recognizer = Arc::new(FakeRecognizer { answer, heard: Default::default() });
            let resident = Arc::new(resident(&dir, &script, Arc::default()));
            let voice = Voice::new(Some(recognizer.clone()), Some(resident)).with_synthesizer(synthesizer);
            Self { voice, transport: Arc::default(), recognizer, script, dir, character }
        }

        /// Feeds the lines; returns the task the last one started, if any.
        fn feed(&self, lines: &[String]) -> Option<JoinHandle<()>> {
            let transport: Arc<dyn Transport> = self.transport.clone();
            lines.iter().filter_map(|line| self.voice.handle_line(line, &transport, self.character)).last()
        }

        fn names(&self) -> Vec<String> {
            self.transport.events().iter().map(|event| event.event.clone()).collect()
        }

        fn error(&self) -> (String, String) {
            let events = self.transport.events();
            let error = events.iter().find(|event| event.event == "voice.error").expect("a voice.error");
            (error.extra["reason"].as_str().unwrap().to_owned(), error.message.clone().unwrap_or_default())
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[tokio::test]
    async fn a_turn_is_heard_answered_reply_by_reply_and_done() {
        let script = logged_in();
        let hold = Arc::new(tokio::sync::Notify::new());
        script.lock().unwrap().hold = Some(hold.clone());
        script.lock().unwrap().turn_requests =
            vec![tool_call("speak", json!({"text": "Two agents are working."})), tool_call("speak", json!({"text": "One needs you."}))];
        let rig = Rig::new("turn", Ok("what are my agents doing"), script);

        let task = rig.feed(&voice_lines(4, &sine(30_000))).expect("the turn runs");
        // Both replies reach the box while the turn is still open in the app-server.
        while rig.names().len() < 3 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(rig.names(), ["voice.heard", "voice.reply", "voice.reply"]);
        assert!(!task.is_finished());
        hold.notify_one();
        task.await.unwrap();

        let events = rig.transport.events();
        assert_eq!(rig.names(), ["voice.heard", "voice.reply", "voice.reply", "voice.done"]);
        assert!(events.iter().all(|event| event.extra["turn"] == 4));
        assert_eq!(events[0].extra["text"], "what are my agents doing");
        assert_eq!(events[1].extra["text"], "Two agents are working.");
        assert_eq!(events[2].extra["text"], "One needs you.");
        let wav = rig.recognizer.heard.lock().unwrap()[0].clone();
        assert_eq!(wav.len(), 44 + 30_000 * 2, "the whole recording, as 16-bit samples");
        let turn_text = rig.script.lock().unwrap().received.iter().find(|message| message["method"] == "turn/start").unwrap()
            ["params"]["input"][0]["text"]
            .clone();
        assert_eq!(turn_text, "what are my agents doing");
    }

    /// One `voice.audio`: reply, seq, its bytes (None for its end) and its error.
    type Piece = (u64, u64, Option<Vec<u8>>, Option<String>);

    /// Each reply's audio, in the order it was sent.
    fn audio(events: &[Event]) -> Vec<Piece> {
        events
            .iter()
            .filter(|event| event.event == "voice.audio")
            .map(|event| {
                let data = event.extra.get("data").map(|data| {
                    let data = data.as_str().unwrap();
                    assert!(data.len() <= 768, "{} base64 characters", data.len());
                    base64::engine::general_purpose::STANDARD.decode(data).unwrap()
                });
                assert_eq!(data.is_none(), event.extra.get("end") == Some(&json!(true)));
                let error = event.extra.get("error").map(|error| error.as_str().unwrap().to_owned());
                (event.extra["reply"].as_u64().unwrap(), event.extra["seq"].as_u64().unwrap(), data, error)
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn each_reply_is_spoken_in_order_without_holding_up_the_text_and_done_comes_after_the_audio() {
        let script = logged_in();
        let hold = Arc::new(tokio::sync::Notify::new());
        script.lock().unwrap().hold = Some(hold.clone());
        script.lock().unwrap().turn_requests =
            vec![tool_call("speak", json!({"text": "Two agents are working."})), tool_call("speak", json!({"text": "One needs you."}))];
        let (open, gate) = std::sync::mpsc::channel();
        let pieces = vec![(0..=255).cycle().take(1300).collect(), vec![7; 10]];
        let synthesizer = Arc::new(FakeSynthesizer { pieces, gate: std::sync::Mutex::new(Some(gate)), ..Default::default() });
        let rig = Rig::speaking("speak", Ok("what are my agents doing"), script, Some(synthesizer.clone()), Some("ada"));

        let task = rig.feed(&voice_lines(4, &sine(30_000))).expect("the turn runs");
        // The first reply's audio is held up, and both replies' text still reach the box.
        while rig.names().len() < 3 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(rig.names(), ["voice.heard", "voice.reply", "voice.reply"]);
        // The turn has completed in the session, but isn't done until the audio is.
        hold.notify_one();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!task.is_finished());
        open.send(()).unwrap();
        task.await.unwrap();

        let events = rig.transport.events();
        assert!(events.iter().all(|event| event.extra["turn"] == 4));
        assert!(events.iter().all(|event| event.to_ndjson().unwrap().len() <= MAX_MESSAGE_BYTES + 1));
        assert_eq!(rig.names().last().map(String::as_str), Some("voice.done"));
        let audio = audio(&events);
        let order: Vec<(u64, u64, bool)> = audio.iter().map(|(reply, seq, data, _)| (*reply, *seq, data.is_none())).collect();
        // 1300 bytes are three pieces of at most 576; then the 10, then the end.
        let expected: Vec<(u64, u64, bool)> = [0, 1].into_iter().flat_map(|reply| (0..5).map(move |seq| (reply, seq, seq == 4))).collect();
        assert_eq!(order, expected);
        for reply in [0, 1] {
            let bytes: Vec<u8> = audio.iter().filter(|(r, ..)| *r == reply).filter_map(|(_, _, data, _)| data.clone()).flatten().collect();
            assert_eq!(bytes.len(), 1310);
            assert_eq!(bytes[..1300], (0..=255).cycle().take(1300).collect::<Vec<u8>>()[..]);
        }
        assert!(audio.iter().all(|(.., error)| error.is_none()));
        let calls = synthesizer.calls.lock().unwrap().clone();
        let voice = "en-GB-SoniaNeural".to_owned();
        assert_eq!(calls, [("Two agents are working.".to_owned(), voice.clone()), ("One needs you.".to_owned(), voice)]);
    }

    #[tokio::test]
    async fn a_failed_synthesis_ends_its_reply_early_marked_and_the_next_one_is_still_spoken() {
        let script = logged_in();
        script.lock().unwrap().turn_requests =
            vec![tool_call("speak", json!({"text": "fail: cut short"})), tool_call("speak", json!({"text": "还在。"}))];
        let synthesizer = Arc::new(FakeSynthesizer { pieces: vec![vec![1; 100]], ..Default::default() });
        let rig = Rig::speaking("speak-fail", Ok("hi"), script, Some(synthesizer.clone()), None);
        rig.feed(&voice_lines(1, &sine(12_000))).unwrap().await.unwrap();

        let audio = audio(&rig.transport.events());
        assert_eq!(
            audio,
            [
                (0, 0, Some(vec![1; 100]), None),
                (0, 1, None, Some("synthesis".to_owned())),
                (1, 0, Some(vec![1; 100]), None),
                (1, 1, None, None),
            ]
        );
        assert_eq!(rig.names().last().map(String::as_str), Some("voice.done"));
        // No Character pack: the built-in Character's voice, and a Chinese one for Chinese.
        let voices: Vec<String> = synthesizer.calls.lock().unwrap().iter().map(|(_, voice)| voice.clone()).collect();
        assert_eq!(voices, ["en-US-JennyNeural", "zh-CN-XiaoxiaoNeural"]);
    }

    #[test]
    fn a_stopped_speaker_sends_nothing_more() {
        let synthesizer = FakeSynthesizer { pieces: vec![vec![1; 10]], ..Default::default() };
        let transport = RecordingTransport::default();
        speak(1, 0, "hello", None, &synthesizer, &transport, &AtomicBool::new(true));
        assert!(transport.events().is_empty());
        assert!(synthesizer.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn the_largest_audio_piece_fits_a_line() {
        let mut event = audio_event(u32::MAX, u32::MAX, u32::MAX);
        event.extra.insert("data".to_owned(), base64::engine::general_purpose::STANDARD.encode([255u8; AUDIO_BYTES]).into());
        assert_eq!(event.extra["data"].as_str().unwrap().len(), 768);
        assert!(event.to_ndjson().unwrap().len() <= MAX_MESSAGE_BYTES + 1);
    }

    #[tokio::test]
    async fn lost_lines_still_make_a_turn_of_the_recorded_length() {
        let rig = Rig::new("gaps", Ok("hello"), logged_in());
        let original = sine(5_000);
        let mut lines = voice_lines(1, &original);
        assert_eq!(lines.last().map(String::as_str), Some("VOICE END 1 5000"));
        assert_eq!(lines.len(), 7, "START, four full lines of 1152 samples and one of 392, END");
        // No START and no line 2 (samples 2304 to 3455): the rest still counts.
        lines.retain(|line| !line.starts_with("VOICE START") && !line.starts_with("VOICE DATA 1 2 "));
        rig.feed(&lines).expect("the turn runs").await.unwrap();
        let wav = rig.recognizer.heard.lock().unwrap()[0].clone();
        assert_eq!(wav.len(), 44 + 5_000 * 2);
        let decoded: Vec<i16> = wav[44..].chunks(2).map(|pair| i16::from_le_bytes([pair[0], pair[1]])).collect();
        assert!(decoded[2_304..3_456].iter().all(|sample| *sample == 0), "line 2 is silence");
        // END counts the lost line's samples too, as the box's does, so what follows the gap keeps its place.
        for k in (3_456 + 60..5_000).step_by(97) {
            assert!((i32::from(decoded[k]) - i32::from(original[k])).abs() < 600, "sample {k}");
        }
        assert_eq!(rig.names().last().map(String::as_str), Some("voice.done"));

        // The last line lost: filled up to the count END gives.
        let rig = Rig::new("gap-at-end", Ok("hello"), logged_in());
        let mut lines = voice_lines(1, &original);
        lines.retain(|line| !line.starts_with("VOICE DATA 1 4 "));
        rig.feed(&lines).expect("the turn runs").await.unwrap();
        let wav = rig.recognizer.heard.lock().unwrap()[0].clone();
        assert_eq!(wav.len(), 44 + 5_000 * 2);
        assert!(wav[44 + 4_608 * 2..].iter().all(|byte| *byte == 0));
    }

    #[tokio::test]
    async fn nothing_recognised_is_not_heard_and_the_session_is_not_asked() {
        let rig = Rig::new("empty", Ok(""), logged_in());
        rig.feed(&voice_lines(2, &sine(12_000))).unwrap().await.unwrap();
        assert_eq!(rig.names(), ["voice.error"]);
        assert_eq!(rig.error().0, "not_heard");
        assert_eq!(rig.script.lock().unwrap().starts, 0);
    }

    #[tokio::test]
    async fn a_failed_recognizer_or_session_is_unavailable() {
        let rig = Rig::new("stt-down", Err("Workers AI answered 401: Authentication error"), logged_in());
        rig.feed(&voice_lines(1, &sine(12_000))).unwrap().await.unwrap();
        assert_eq!(rig.error(), ("unavailable".to_owned(), "Workers AI answered 401: Authentication error".to_owned()));

        // Codex isn't logged in: what was heard is still shown, then the error.
        let rig = Rig::new("no-login", Ok("hi"), Arc::new(std::sync::Mutex::new(Default::default())));
        rig.feed(&voice_lines(1, &sine(12_000))).unwrap().await.unwrap();
        assert_eq!(rig.names(), ["voice.heard", "voice.error"]);
        assert_eq!(rig.error().0, "unavailable");
    }

    #[tokio::test]
    async fn a_turn_that_ends_while_another_runs_is_busy() {
        let script = logged_in();
        let hold = Arc::new(tokio::sync::Notify::new());
        script.lock().unwrap().hold = Some(hold.clone());
        let rig = Rig::new("busy", Ok("first"), script);
        let first = rig.feed(&voice_lines(1, &sine(12_000))).unwrap();
        let second = voice_lines(2, &sine(12_000));
        // Refused as soon as it starts, so the box stops recording; the rest of it gets no second answer.
        assert!(rig.feed(&second[..1]).is_none());
        let errors = || rig.transport.events().into_iter().filter(|event| event.event == "voice.error").collect::<Vec<_>>();
        assert_eq!(errors().len(), 1);
        assert_eq!((errors()[0].extra["turn"].as_u64(), errors()[0].extra["reason"].as_str()), (Some(2), Some("busy")));
        assert!(rig.feed(&second[1..]).is_none());
        assert_eq!(errors().len(), 1);
        hold.notify_one();
        first.await.unwrap();
        // Once it is over, the next turn runs.
        hold.notify_one();
        rig.feed(&voice_lines(3, &sine(12_000))).expect("not busy any more").await.unwrap();
    }

    #[tokio::test]
    async fn a_cancelled_turn_is_dropped_and_an_end_without_audio_is_not_heard() {
        let rig = Rig::new("cancel", Ok("hi"), logged_in());
        let mut lines = voice_lines(5, &sine(6_000));
        lines.pop();
        lines.push("VOICE CANCEL 5".to_owned());
        assert!(rig.feed(&lines).is_none());
        assert!(rig.names().is_empty(), "a cancelled turn gets no answer");
        assert!(rig.feed(&["VOICE END 5 6000".to_owned()]).is_none());
        assert_eq!(rig.error().0, "not_heard");
        assert!(rig.recognizer.heard.lock().unwrap().is_empty());
    }

    /// The left channel of a 16-bit PCM WAV file, and its sample rate.
    fn left_channel(file: &[u8]) -> (Vec<i16>, u32) {
        let mut at = 12;
        let (mut channels, mut rate) = (1usize, 0u32);
        while at + 8 <= file.len() {
            let size = u32::from_le_bytes(file[at + 4..at + 8].try_into().unwrap()) as usize;
            let body = &file[at + 8..(at + 8 + size).min(file.len())];
            match &file[at..at + 4] {
                b"fmt " => {
                    channels = u16::from_le_bytes([body[2], body[3]]) as usize;
                    rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                    assert_eq!(u16::from_le_bytes([body[14], body[15]]), 16, "16-bit PCM only");
                }
                b"data" => {
                    let samples = body.chunks_exact(2 * channels).map(|frame| i16::from_le_bytes([frame[0], frame[1]])).collect();
                    return (samples, rate);
                }
                _ => {}
            }
            at += 8 + size + size % 2;
        }
        panic!("no data chunk");
    }

    /// Prints each event the moment the daemon sends it, with the time since `VOICE END`.
    struct TimedTransport {
        end: std::sync::Mutex<Option<Instant>>,
        events: std::sync::Mutex<Vec<(u128, Event)>>,
    }

    impl Transport for TimedTransport {
        fn send(&self, frame: Vec<u8>) -> Result<(), crate::serial_transport::TransportError> {
            assert!(frame.len() <= MAX_MESSAGE_BYTES + 1);
            let event: Event = serde_json::from_slice(&frame).unwrap();
            let ms = self.end.lock().unwrap().map(|end| end.elapsed().as_millis()).unwrap_or_default();
            println!("+{ms:>6} ms  {}", String::from_utf8_lossy(&frame).trim_end());
            self.events.lock().unwrap().push((ms, event));
            Ok(())
        }
    }

    /// A real recording through the whole computer side, without the box: encoded as the box encodes it, fed as
    /// `VOICE` lines, recognised by the real recognizer and answered by the real resident session. Run by hand:
    ///
    /// ```sh
    /// VIBEBUDDY_E2E_WAV=mic.wav VIBEBUDDY_STT_ENV_FILE=~/.vibebuddy-dev-ai.env \
    ///   cargo test -p vibebuddyd e2e_ -- --ignored --nocapture
    /// ```
    ///
    /// The thread and its Codex home live in a temporary state directory; the user's hooks don't load there, so
    /// nothing reaches the daemon on 7331.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "uses the network, a speech recognizer's credentials and the user's Codex"]
    async fn e2e_a_real_recording_through_the_real_recognizer_and_resident_session() {
        let _ = tracing_subscriber::fmt().with_env_filter("vibebuddyd=info").with_test_writer().try_init();
        let path = std::env::var("VIBEBUDDY_E2E_WAV").expect("VIBEBUDDY_E2E_WAV names a recording");
        let (left, rate) = left_channel(&std::fs::read(path).unwrap());
        assert_eq!(rate, SAMPLE_RATE, "the box records at 24 kHz");
        let recognizer = crate::speech::from_env().expect("a speech recognizer is configured");
        let dir = temp_dir("voice-e2e");
        let resident = Arc::new(Resident::new(true, &dir, crate::resident::Launcher::Codex, Arc::default()));
        let voice = Voice::new(Some(recognizer), Some(resident));
        let timed = Arc::new(TimedTransport { end: Default::default(), events: Default::default() });
        let transport: Arc<dyn Transport> = timed.clone();

        // 1152 samples are 576 bytes, 768 base64 characters: the most a line may carry.
        let lines = voice_lines(1, &left);
        assert!(lines.iter().all(|line| line.len() < 1024));
        println!("{:.2} s of audio in {} lines", left.len() as f32 / SAMPLE_RATE as f32, lines.len());
        let (end, data) = lines.split_last().unwrap();
        for line in data {
            assert!(voice.handle_line(line, &transport, None).is_none());
        }
        *timed.end.lock().unwrap() = Some(Instant::now());
        voice.handle_line(end, &transport, None).expect("the turn runs").await.unwrap();

        let events = timed.events.lock().unwrap();
        let names: Vec<&str> = events.iter().map(|(_, event)| event.event.as_str()).collect();
        assert_eq!(names.first(), Some(&"voice.heard"), "{names:?}");
        assert_eq!(names.last(), Some(&"voice.done"), "{names:?}");
        assert!(names.contains(&"voice.reply"), "{names:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Two replies through the real voice service, as the box would get them: the `voice.audio` pieces put back
    /// together into `<dir>/reply-zh.mp3` and `reply-en.mp3`. Run by hand:
    ///
    /// ```sh
    /// VIBEBUDDY_E2E_OUT=/some/dir cargo test -p vibebuddyd e2e_spoken -- --ignored --nocapture
    /// ```
    ///
    /// `VIBEBUDDY_TTS_BASE_URL` picks the service as it does for the daemon; unset, staging.
    #[test]
    #[ignore = "uses the network and the voice service"]
    fn e2e_spoken_replies_through_the_real_voice_service() {
        let out = std::path::PathBuf::from(std::env::var("VIBEBUDDY_E2E_OUT").expect("VIBEBUDDY_E2E_OUT names a directory"));
        let synthesizer = synthesis::from_env().expect("a voice service");
        for (name, character, text) in [
            ("zh", Some("wanwanxiaohe"), "两个智能体正在工作，有一个在等你回复。"),
            ("en", Some("ada"), "Two agents are working, and one of them is waiting for you."),
        ] {
            let timed = TimedTransport { end: std::sync::Mutex::new(Some(Instant::now())), events: Default::default() };
            speak(1, 0, text, character, &*synthesizer, &timed, &AtomicBool::new(false));
            let events = timed.events.lock().unwrap();
            let (first_ms, _) = events.first().expect("some audio");
            let (total_ms, last) = events.last().unwrap();
            assert_eq!(last.extra.get("end"), Some(&json!(true)));
            assert!(!last.extra.contains_key("error"), "synthesis failed");
            let pieces: Vec<Event> = events.iter().map(|(_, event)| event.clone()).collect();
            let bytes: Vec<u8> = audio(&pieces).into_iter().filter_map(|(_, _, data, _)| data).flatten().collect();
            let path = out.join(format!("reply-{name}.mp3"));
            std::fs::write(&path, &bytes).unwrap();
            println!(
                "{name} {}: first audio after {first_ms} ms, end after {total_ms} ms, {} bytes in {} lines -> {}",
                synthesis::voice_for(character, text),
                bytes.len(),
                events.len(),
                path.display()
            );
        }
    }

    #[tokio::test]
    async fn voice_is_offered_only_with_a_recognizer_and_the_resident_session_on() {
        let dir = temp_dir("voice-offer");
        let on = || Some(Arc::new(resident(&dir, &logged_in(), Arc::default())));
        let off = Some(Arc::new(Resident::new(false, &dir, crate::resident::Launcher::Codex, Arc::default())));
        let recognizer = || Some(Arc::new(FakeRecognizer { answer: Ok(""), heard: Default::default() }) as Arc<dyn Recognizer>);
        assert!(Voice::new(recognizer(), on()).available());
        assert!(!Voice::new(None, on()).available());
        assert!(!Voice::new(recognizer(), off).available());
        assert!(!Voice::new(recognizer(), None).available());

        // Lines that arrive anyway are told voice is off.
        let voice = Voice::new(None, on());
        let transport = Arc::new(RecordingTransport::default());
        let shared: Arc<dyn Transport> = transport.clone();
        for line in voice_lines(1, &sine(12_000)) {
            voice.handle_line(&line, &shared, None);
        }
        assert_eq!(transport.events()[0].extra["reason"], "unavailable");
        let _ = std::fs::remove_dir_all(dir);
    }
}
