//! A spoken reply's audio (docs/protocol.md, "Voice commands (experimental)"): the computer sends each reply
//! of a voice command's turn as an MP3 stream in numbered `voice.audio` chunks, and the box plays the replies
//! one after another through the same speaker path as announcements.
//!
//! [`ReplyPlayer`] takes the chunks as they come and hands out the codec's 24 kHz stereo PCM a piece at a time,
//! like [`crate::adpcm::LineStream`] does for a Character's lines. It holds no hardware: the firmware feeds it
//! ([`ReplyAudio`], through [`crate::firmware::Board::reply_audio`]) and the device's audio task pulls from it
//! when no announcement is playing.
//!
//! - Each reply's bytes are kept on the heap while it waits and plays, and let go of as they are decoded; the
//!   decoder (about 22 KB) is allocated when a reply starts and freed once nothing is left to play.
//! - A reply starts once [`START_MS`] of it is buffered, or once all of it is in, and plays without waiting for
//!   its end. If it runs dry it goes quiet until that much is buffered again, rather than stutter.
//! - Replies play in their order. One whose audio never comes (synthesis failed) is simply not there: the next
//!   one plays.
//! - A missing chunk is skipped: the decoder finds the next frame, so a lost line costs a moment of the reply,
//!   not the rest of it. A chunk that comes late (behind one already taken) is ignored. A reply too long to hold
//!   or in a sample rate the speaker doesn't run at drops its audio, and the next reply follows.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

use nanomp3_core::{Decoder, MAX_SAMPLES_PER_FRAME};

/// The speaker's rate; the voice service's MP3 is 24 kHz mono, 48 kbps.
pub const SAMPLE_RATE: u32 = 24_000;
/// 48 kbps is 6 bytes a millisecond.
const BYTES_PER_MS: usize = 6;
/// A reply starts, and resumes after running dry, once this much of it is buffered.
pub const START_MS: usize = 300;
const START_BYTES: usize = START_MS * BYTES_PER_MS;
/// While more is coming, a frame is only decoded with this much past its start: the decoder trusts a frame once
/// it sees the next one's header, and skips a frame cut short as junk. The largest MPEG-2 Layer III frame at
/// 24 kHz (160 kbps) is 481 bytes.
const LOOKAHEAD: usize = 1024;
/// The most of one reply held at a time, about 43 s at 48 kbps; past it the reply is dropped.
pub const MAX_REPLY_BYTES: usize = 256 * 1024;
/// Decoded bytes are let go of in steps of this.
const COMPACT_BYTES: usize = 4096;

/// What the firmware hands the player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyAudio {
    /// The next bytes of a reply's MP3 stream.
    Chunk { reply: u32, seq: u32, data: Vec<u8> },
    /// The reply's audio is over; `seq` follows its last chunk.
    End { reply: u32, seq: u32 },
    /// Stop playing and forget every reply.
    Stop,
}

struct Reply {
    index: u32,
    next_seq: u32,
    bytes: Vec<u8>,
    /// Decoded up to here.
    read: usize,
    ended: bool,
    dropped: bool,
}

impl Reply {
    fn ahead(&self) -> usize {
        self.bytes.len() - self.read
    }

    fn drop_audio(&mut self) {
        self.dropped = true;
        self.bytes = Vec::new();
        self.read = 0;
    }
}

pub struct ReplyPlayer {
    /// Replies with audio, in their order; the first is the one playing or next to.
    replies: VecDeque<Reply>,
    /// Replies numbered below this are over (played, dropped, or passed by because a later one started); their
    /// chunks are ignored.
    floor: u32,
    /// The playing reply's decoder; each reply is its own stream.
    decoder: Option<Box<Decoder>>,
    /// The last decoded frame, and how much of it is handed out.
    pcm: Vec<i16>,
    pcm_read: usize,
    pcm_len: usize,
    /// The first reply had enough buffered to start and hasn't run dry since.
    sounding: bool,
}

impl Default for ReplyPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplyPlayer {
    pub const fn new() -> Self {
        Self { replies: VecDeque::new(), floor: 0, decoder: None, pcm: Vec::new(), pcm_read: 0, pcm_len: 0, sounding: false }
    }

    pub fn apply(&mut self, audio: ReplyAudio) {
        match audio {
            ReplyAudio::Chunk { reply, seq, data } => self.chunk(reply, seq, Some(&data)),
            ReplyAudio::End { reply, seq } => self.chunk(reply, seq, None),
            // Everything goes, the memory too.
            ReplyAudio::Stop => *self = Self::new(),
        }
    }

    /// Something is still to be played: a reply buffered or playing.
    pub fn pending(&self) -> bool {
        !self.replies.is_empty() || self.pcm_read < self.pcm_len
    }

    /// A reply is making sound right now (not waiting for its first bytes or after running dry).
    pub fn speaking(&self) -> bool {
        self.sounding || self.pcm_read < self.pcm_len
    }

    /// A chunk of a reply, or with no data its end.
    fn chunk(&mut self, index: u32, seq: u32, data: Option<&[u8]>) {
        if index < self.floor {
            return;
        }
        let at = match self.replies.iter().position(|reply| reply.index >= index) {
            Some(at) if self.replies[at].index == index => at,
            found => {
                let at = found.unwrap_or(self.replies.len());
                let reply = Reply { index, next_seq: 0, bytes: Vec::new(), read: 0, ended: false, dropped: false };
                self.replies.insert(at, reply);
                at
            }
        };
        let reply = &mut self.replies[at];
        if reply.dropped || reply.ended {
            return;
        }
        if seq < reply.next_seq {
            return;
        }
        reply.next_seq = seq + 1;
        let Some(data) = data else {
            reply.ended = true;
            return;
        };
        if reply.bytes.len() + data.len() > MAX_REPLY_BYTES || reply.bytes.try_reserve(data.len()).is_err() {
            return self.drop_reply(at);
        }
        reply.bytes.extend_from_slice(data);
    }

    fn drop_reply(&mut self, at: usize) {
        self.replies[at].drop_audio();
        // The one playing stops at once, with what it had decoded.
        if at == 0 {
            self.pcm_len = 0;
            self.pcm_read = 0;
        }
    }

    /// Fills `out` with the next 24 kHz stereo 16-bit PCM and returns how many bytes (a multiple of 4) were
    /// written; fewer than asked, or none, while there is nothing to play right now. The caller pads with silence.
    pub fn fill(&mut self, out: &mut [u8]) -> usize {
        let mut written = 0;
        while written + 4 <= out.len() {
            if self.pcm_read == self.pcm_len && !self.decode_next() {
                break;
            }
            let [low, high] = self.pcm[self.pcm_read].to_le_bytes();
            out[written..written + 4].copy_from_slice(&[low, high, low, high]);
            self.pcm_read += 1;
            written += 4;
        }
        written
    }

    /// Decodes the next frame of the first reply; false when there is nothing to play right now.
    fn decode_next(&mut self) -> bool {
        loop {
            let Some(reply) = self.replies.front_mut() else {
                // Nothing left: the decoder and its buffers go back to the heap.
                self.decoder = None;
                self.pcm = Vec::new();
                self.sounding = false;
                return false;
            };
            if reply.dropped || (reply.ended && reply.ahead() == 0) {
                self.floor = self.floor.max(reply.index + 1);
                self.replies.pop_front();
                self.decoder = None;
                self.sounding = false;
                continue;
            }
            if !self.sounding {
                if reply.ahead() < START_BYTES && !reply.ended {
                    return false;
                }
                self.sounding = true;
                // Once a reply starts, an earlier one that shows up late is passed by.
                self.floor = self.floor.max(reply.index);
            }
            if reply.ahead() < LOOKAHEAD && !reply.ended {
                // Ran dry: quiet until it has enough to start again.
                self.sounding = false;
                return false;
            }
            if self.decoder.is_none() {
                self.decoder = new_decoder();
            }
            let Some(decoder) = self.decoder.as_mut() else {
                reply.drop_audio();
                continue;
            };
            if self.pcm.is_empty() {
                self.pcm = alloc::vec![0; MAX_SAMPLES_PER_FRAME];
            }
            let (used, result) = decoder.decode(&reply.bytes[reply.read..], &mut self.pcm);
            // Nothing consumed only happens on an empty input; with the reply over, the rest is junk.
            reply.read += if used == 0 { reply.ahead() } else { used };
            if reply.read >= COMPACT_BYTES {
                reply.bytes.drain(..reply.read);
                reply.read = 0;
            }
            match result {
                Ok(frame) if frame.sample_rate != SAMPLE_RATE => reply.drop_audio(),
                Ok(frame) if frame.samples_produced > 0 => {
                    // Mono as the service sends it; of a stereo stream, the left channel.
                    let channels = frame.channels.num() as usize;
                    for sample in 1..frame.samples_produced {
                        self.pcm[sample] = self.pcm[sample * channels];
                    }
                    self.pcm_read = 0;
                    self.pcm_len = frame.samples_produced;
                    return true;
                }
                // The first frames after a start lean on a bit reservoir that isn't there; a corrupt frame is skipped.
                _ => {}
            }
        }
    }
}

/// The decoder, built straight on the heap: 22 KB must not pass through the stack on its way there.
fn new_decoder() -> Option<Box<Decoder>> {
    let layout = core::alloc::Layout::new::<Decoder>();
    // SAFETY: `Decoder::new()` is all zero bytes in nanomp3-core 0.2.0 (pinned with `=` for this): zeroed
    // float arrays, zero counters and a `false`. The test `a_zeroed_decoder_decodes_like_a_new_one` holds it
    // to that. A non-null pointer from `alloc_zeroed` with Decoder's layout is what `Box::from_raw` takes.
    unsafe {
        let raw = alloc::alloc::alloc_zeroed(layout).cast::<Decoder>();
        (!raw.is_null()).then(|| Box::from_raw(raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    const FIXTURE: &[u8] = include_bytes!("fixtures/spoken_reply.mp3");

    /// The fixture decoded in one go with a decoder on the stack: the reference, as stereo bytes.
    fn one_shot(mp3: &[u8], mut decoder: Decoder) -> Vec<u8> {
        let mut pcm = [0i16; MAX_SAMPLES_PER_FRAME];
        let mut out = Vec::new();
        let mut rest = mp3;
        while !rest.is_empty() {
            let (used, result) = decoder.decode(rest, &mut pcm);
            rest = &rest[used.max(1).min(rest.len())..];
            if let Ok(frame) = result {
                assert_eq!((frame.sample_rate, frame.channels.num()), (24_000, 1));
                for &sample in &pcm[..frame.samples_produced] {
                    out.extend_from_slice(&sample.to_le_bytes());
                    out.extend_from_slice(&sample.to_le_bytes());
                }
            }
        }
        out
    }

    fn chunks(mp3: &[u8], sizes: &[usize]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut at = 0;
        for size in sizes.iter().cycle() {
            if at == mp3.len() {
                break;
            }
            let end = (at + size).min(mp3.len());
            out.push(mp3[at..end].to_vec());
            at = end;
        }
        out
    }

    fn send(player: &mut ReplyPlayer, reply: u32, pieces: &[Vec<u8>]) {
        for (seq, data) in pieces.iter().enumerate() {
            player.apply(ReplyAudio::Chunk { reply, seq: seq as u32, data: data.clone() });
        }
        player.apply(ReplyAudio::End { reply, seq: pieces.len() as u32 });
    }

    /// Everything the player has to give, pulled in pieces the size the audio task uses.
    fn drain(player: &mut ReplyPlayer, piece: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buffer = vec![0u8; piece];
        loop {
            let count = player.fill(&mut buffer);
            out.extend_from_slice(&buffer[..count]);
            if count == 0 {
                return out;
            }
        }
    }

    #[test]
    fn the_fixture_is_the_services_format() {
        let pcm = one_shot(FIXTURE, Decoder::new());
        // Stereo 16-bit bytes at 24 kHz: a few seconds of speech.
        let seconds = pcm.len() as f32 / 4.0 / 24_000.0;
        assert!((3.0..6.0).contains(&seconds), "{seconds} s");
    }

    #[test]
    fn a_zeroed_decoder_decodes_like_a_new_one() {
        let zeroed = new_decoder().unwrap();
        assert_eq!(one_shot(FIXTURE, (*zeroed).clone()), one_shot(FIXTURE, Decoder::new()));
    }

    #[test]
    fn odd_chunks_pulled_as_they_come_give_the_same_pcm_as_one_go() {
        let expected = one_shot(FIXTURE, Decoder::new());
        for (sizes, piece) in [(&[1usize, 7, 333, 577][..], 1000usize), (&[576], 4), (&[97, 1500], 1004), (&[FIXTURE.len()], 3000)] {
            let mut player = ReplyPlayer::new();
            let pieces = chunks(FIXTURE, sizes);
            let mut out = Vec::new();
            let mut buffer = vec![0u8; piece];
            // The audio task pulls between chunks, so the player runs dry and waits along the way.
            for (seq, data) in pieces.iter().enumerate() {
                player.apply(ReplyAudio::Chunk { reply: 0, seq: seq as u32, data: data.clone() });
                let count = player.fill(&mut buffer);
                out.extend_from_slice(&buffer[..count]);
            }
            player.apply(ReplyAudio::End { reply: 0, seq: pieces.len() as u32 });
            out.extend(drain(&mut player, piece));
            assert!(out == expected, "chunks {sizes:?}, pulled {piece} at a time: {} bytes, expected {}", out.len(), expected.len());
            assert!(!player.pending() && player.decoder.is_none() && player.pcm.is_empty(), "everything is let go of");
        }
    }

    #[test]
    fn a_reply_starts_with_300_ms_buffered_without_waiting_for_its_end() {
        let mut player = ReplyPlayer::new();
        let mut buffer = [0u8; 1000];
        player.apply(ReplyAudio::Chunk { reply: 0, seq: 0, data: FIXTURE[..START_BYTES - 1].to_vec() });
        assert_eq!(player.fill(&mut buffer), 0, "not yet");
        assert!(!player.speaking() && player.pending());
        player.apply(ReplyAudio::Chunk { reply: 0, seq: 1, data: FIXTURE[START_BYTES - 1..START_BYTES].to_vec() });
        assert_eq!(player.fill(&mut buffer), 1000);
        assert!(player.speaking());
        // It plays until less than a frame's lookahead is left, then waits quietly instead of stuttering.
        let played = drain(&mut player, 1000).len() + 1000;
        assert!(played < one_shot(&FIXTURE[..START_BYTES], Decoder::new()).len() && !player.speaking(), "{played}");
        player.apply(ReplyAudio::Chunk { reply: 0, seq: 2, data: FIXTURE[START_BYTES..START_BYTES + 100].to_vec() });
        assert_eq!(player.fill(&mut buffer), 0, "a little more isn't enough to start again");
        // A short reply that ends below the threshold plays at once.
        let mut short = ReplyPlayer::new();
        send(&mut short, 0, &[FIXTURE[..600].to_vec()]);
        assert!(short.fill(&mut buffer) > 0);
    }

    #[test]
    fn replies_play_in_order_whatever_order_their_audio_comes_in() {
        let first = &FIXTURE[..FIXTURE.len() / 2];
        let expected_first = one_shot(first, Decoder::new());
        let expected_whole = one_shot(FIXTURE, Decoder::new());
        let mut player = ReplyPlayer::new();
        // Reply 1 comes in whole before reply 0's end, but waits its turn.
        player.apply(ReplyAudio::Chunk { reply: 0, seq: 0, data: first[..100].to_vec() });
        send(&mut player, 1, &chunks(FIXTURE, &[768]));
        player.apply(ReplyAudio::Chunk { reply: 0, seq: 1, data: first[100..].to_vec() });
        player.apply(ReplyAudio::End { reply: 0, seq: 2 });
        let out = drain(&mut player, 1000);
        assert!(out[..expected_first.len()] == expected_first[..], "reply 0 first");
        assert!(out[expected_first.len()..] == expected_whole[..], "then reply 1, from a fresh decoder");
    }

    #[test]
    fn a_reply_without_audio_is_passed_by_and_a_late_earlier_one_is_ignored() {
        let mut player = ReplyPlayer::new();
        // Reply 0 never got audio; reply 2 starts, so reply 1 turning up afterwards is too late.
        send(&mut player, 2, &chunks(FIXTURE, &[768]));
        let mut buffer = [0u8; 1000];
        assert!(player.fill(&mut buffer) > 0);
        send(&mut player, 1, &chunks(FIXTURE, &[768]));
        let out = drain(&mut player, 1000).len() + 1000;
        assert_eq!(out, one_shot(FIXTURE, Decoder::new()).len());
    }

    #[test]
    fn a_missing_chunk_is_skipped_and_a_late_one_ignored() {
        let pieces = chunks(FIXTURE, &[768]);
        let expected = one_shot(FIXTURE, Decoder::new());
        // A chunk lost in the middle: the reply plays on past it, only a little shorter, then the next one whole.
        let mut player = ReplyPlayer::new();
        for (seq, data) in pieces.iter().enumerate() {
            if seq != 3 {
                player.apply(ReplyAudio::Chunk { reply: 0, seq: seq as u32, data: data.clone() });
            }
        }
        player.apply(ReplyAudio::End { reply: 0, seq: pieces.len() as u32 });
        send(&mut player, 1, &pieces);
        let played = drain(&mut player, 1000);
        assert!(played.ends_with(&expected), "the next reply plays whole");
        let first = played.len() - expected.len();
        assert!(first > expected.len() * 3 / 4 && first < expected.len(), "{first} of {}", expected.len());

        // A chunk that comes after a later one was taken is ignored; the reply carries on.
        let mut player = ReplyPlayer::new();
        for (seq, data) in pieces.iter().enumerate() {
            if seq == 5 {
                continue;
            }
            player.apply(ReplyAudio::Chunk { reply: 0, seq: seq as u32, data: data.clone() });
            if seq == 6 {
                player.apply(ReplyAudio::Chunk { reply: 0, seq: 5, data: pieces[5].clone() });
            }
        }
        player.apply(ReplyAudio::End { reply: 0, seq: pieces.len() as u32 });
        let played = drain(&mut player, 1000).len();
        assert!(played > expected.len() * 3 / 4 && played < expected.len(), "{played}");
    }

    #[test]
    fn a_reply_cut_short_plays_what_it_has() {
        // Synthesis stopped partway: the end arrives in order after fewer bytes.
        let part = &FIXTURE[..FIXTURE.len() / 3];
        let mut player = ReplyPlayer::new();
        send(&mut player, 0, &chunks(part, &[768]));
        assert!(drain(&mut player, 1000) == one_shot(part, Decoder::new()));
    }

    #[test]
    fn stop_forgets_everything_and_garbage_is_not_played() {
        let mut player = ReplyPlayer::new();
        send(&mut player, 0, &chunks(FIXTURE, &[768]));
        let mut buffer = [0u8; 1000];
        assert_eq!(player.fill(&mut buffer), 1000);
        player.apply(ReplyAudio::Stop);
        assert!(!player.pending() && player.decoder.is_none());
        assert_eq!(player.fill(&mut buffer), 0);
        // After a stop, a new turn's replies count from 0 again.
        send(&mut player, 0, &[vec![0x55; 3000]]);
        assert_eq!(drain(&mut player, 1000).len(), 0, "no frames, no sound");
        assert!(!player.pending());
    }

    #[test]
    fn a_reply_too_long_to_hold_is_dropped() {
        let mut player = ReplyPlayer::new();
        let piece = vec![0u8; 64 * 1024];
        for seq in 0..5 {
            player.apply(ReplyAudio::Chunk { reply: 0, seq, data: piece.clone() });
        }
        assert!(player.replies[0].dropped && player.replies[0].bytes.capacity() == 0);
    }
}
