//! Decoding a Character pack line: 16 kHz mono IMA ADPCM in, the codec's 24 kHz stereo PCM out. And the
//! same coding the other way for voice commands ([`Encoder`], docs/protocol.md): the microphone's 24 kHz
//! samples out as IMA ADPCM, which the computer decodes with the same tables.
//!
//! Each line is a standalone stream: predictor and step index start at zero, two samples per byte,
//! low nibble first. The upsampling is linear interpolation, 2 input samples to 3 output samples;
//! on this speaker that is indistinguishable from anything fancier. `tools/character_pack.py`
//! holds the encoder, and the tests here check the two agree on the same sample.

const STEPS: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130,
    143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166,
    1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845,
    8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

const INDEX_CHANGE: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

#[derive(Clone, Copy, Debug, Default)]
pub struct Decoder {
    predictor: i32,
    index: i32,
}

impl Decoder {
    /// A decoder that picks up a stream at a known state: a voice command line carries the coder's
    /// predictor and step index before its first nibble, so each line decodes on its own.
    pub fn with_state(predictor: i16, index: u8) -> Self {
        Self { predictor: predictor as i32, index: (index as i32).clamp(0, 88) }
    }

    /// The predictor (the last sample) and the step index.
    pub fn state(&self) -> (i16, u8) {
        (self.predictor as i16, self.index as u8)
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

/// The standard IMA ADPCM encoder (the one `tools/character_pack.py` uses): each nibble is chosen against
/// the decoder's own prediction, so the decoder above reproduces exactly what the encoder tracked.
#[derive(Clone, Copy, Debug, Default)]
pub struct Encoder {
    decoder: Decoder,
}

impl Encoder {
    /// The state a decoder needs to start at the next nibble: predictor and step index.
    pub fn state(&self) -> (i16, u8) {
        self.decoder.state()
    }

    pub fn encode(&mut self, sample: i16) -> u8 {
        let step = STEPS[self.decoder.index as usize];
        let mut diff = sample as i32 - self.decoder.predictor;
        let mut nibble = 0u8;
        if diff < 0 {
            nibble = 8;
            diff = -diff;
        }
        if diff >= step {
            nibble |= 4;
            diff -= step;
        }
        if diff >= step >> 1 {
            nibble |= 2;
            diff -= step >> 1;
        }
        if diff >= step >> 2 {
            nibble |= 1;
        }
        self.decoder.decode(nibble);
        nibble
    }

    /// Two samples to one byte, low nibble first.
    pub fn encode_pair(&mut self, first: i16, second: i16) -> u8 {
        let low = self.encode(first);
        low | self.encode(second) << 4
    }
}

/// Turns one line's ADPCM bytes into 24 kHz stereo PCM, a piece at a time. The caller feeds the
/// stored bytes in order; `fill` asks for the next byte through `next_byte`.
pub struct LineStream {
    decoder: Decoder,
    /// Input samples not yet decoded.
    input_left: u32,
    /// The byte whose high nibble is still to be decoded, if any.
    held: Option<u8>,
    /// The two input samples the current output sits between, and the index of the first.
    current: i16,
    next: i16,
    position: u32,
    /// Output samples produced so far, and how many there will be in all.
    produced: u32,
    total: u32,
}

impl LineStream {
    pub fn new(samples: u32) -> Self {
        Self {
            decoder: Decoder::default(),
            input_left: samples,
            held: None,
            current: 0,
            next: 0,
            position: 0,
            produced: 0,
            total: (samples * 3).div_ceil(2),
        }
    }

    /// Bytes of 24 kHz stereo PCM still to come.
    pub fn remaining_bytes(&self) -> usize {
        (self.total - self.produced) as usize * 4
    }

    fn pull(&mut self, next_byte: &mut impl FnMut() -> u8) -> i16 {
        if self.input_left == 0 {
            return self.next;
        }
        self.input_left -= 1;
        let nibble = match self.held.take() {
            Some(byte) => byte >> 4,
            None => {
                let byte = next_byte();
                self.held = Some(byte);
                byte & 15
            }
        };
        self.decoder.decode(nibble)
    }

    /// Fills `out` (a multiple of 4 bytes) with stereo frames and returns how many bytes it wrote.
    pub fn fill(&mut self, out: &mut [u8], mut next_byte: impl FnMut() -> u8) -> usize {
        if self.produced == 0 && self.total > 0 {
            self.current = self.pull(&mut next_byte);
            self.next = self.pull(&mut next_byte);
        }
        let mut written = 0;
        while written + 4 <= out.len() && self.produced < self.total {
            // Output k sits at input position 2k/3.
            let at = self.produced * 2;
            while self.position < at / 3 {
                self.current = self.next;
                self.next = self.pull(&mut next_byte);
                self.position += 1;
            }
            let third = (at % 3) as i32;
            let sample = (self.current as i32 + (self.next as i32 - self.current as i32) * third / 3) as i16;
            let bytes = sample.to_le_bytes();
            out[written..written + 4].copy_from_slice(&[bytes[0], bytes[1], bytes[0], bytes[1]]);
            written += 4;
            self.produced += 1;
        }
        written
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// Packs samples two to a byte, as a Character pack line stores them.
    fn encode(samples: &[i16]) -> Vec<u8> {
        let mut encoder = Encoder::default();
        samples
            .chunks(2)
            .map(|pair| match pair {
                [first, second] => encoder.encode_pair(*first, *second),
                _ => encoder.encode(pair[0]),
            })
            .collect()
    }

    fn sine(count: usize) -> Vec<i16> {
        (0..count).map(|index| (libm::sinf(index as f32 * 0.05) * 12000.0) as i16).collect()
    }

    fn decode_all(bytes: &[u8], samples: u32) -> Vec<i16> {
        let mut stream = LineStream::new(samples);
        let mut out = std::vec![0u8; stream.remaining_bytes()];
        let mut source = bytes.iter().copied();
        let written = stream.fill(&mut out, || source.next().unwrap_or(0));
        assert_eq!(written, out.len());
        assert_eq!(stream.remaining_bytes(), 0);
        out.chunks(4)
            .map(|frame| {
                assert_eq!(frame[..2], frame[2..], "both channels carry the same sample");
                i16::from_le_bytes([frame[0], frame[1]])
            })
            .collect()
    }

    #[test]
    fn a_round_trip_stays_close_to_the_original() {
        let original = sine(1000);
        let decoded = decode_all(&encode(&original), 1000);
        assert_eq!(decoded.len(), 1500);
        // Every third output lands exactly on an input sample; after the first few samples, while
        // the step size is still catching up, it tracks the original closely.
        for (k, &sample) in decoded.iter().enumerate().skip(60).step_by(3) {
            let expected = original[k * 2 / 3] as i32;
            assert!((sample as i32 - expected).abs() < 600, "output {k}: {sample} vs {expected}");
        }
    }

    #[test]
    fn the_python_encoder_agrees() {
        // tools/character_pack.py encodes these samples (the start of sine()) to these bytes.
        let samples = [0, 599, 1198, 1793, 2384, 2968, 3546, 4114, 4673, 5219, 5753, 6272, 6775, 7262, 7730, 8179];
        assert_eq!(encode(&samples), [0x70, 0x77, 0x77, 0x77, 0x05, 0x11, 0x10, 0x11]);
    }

    #[test]
    fn filling_in_small_pieces_gives_the_same_audio() {
        let original = sine(301);
        let bytes = encode(&original);
        let whole = decode_all(&bytes, 301);

        let mut stream = LineStream::new(301);
        let mut source = bytes.iter().copied();
        let mut pieces = Vec::new();
        let mut buffer = [0u8; 12];
        loop {
            let written = stream.fill(&mut buffer, || source.next().unwrap_or(0));
            if written == 0 {
                break;
            }
            pieces.extend(buffer[..written].chunks(4).map(|frame| i16::from_le_bytes([frame[0], frame[1]])));
        }
        assert_eq!(pieces, whole);
        assert_eq!(pieces.len(), 452, "301 samples at 16 kHz become ceil(301 × 1.5) at 24 kHz");
    }

    /// What a computer does with a voice command line: start a decoder at the line's state and decode
    /// every nibble, low first.
    fn decode_line(predictor: i16, index: u8, bytes: &[u8]) -> Vec<i16> {
        let mut decoder = Decoder::with_state(predictor, index);
        bytes.iter().flat_map(|&byte| [byte & 15, byte >> 4]).map(|nibble| decoder.decode(nibble)).collect()
    }

    #[test]
    fn the_encoder_tracks_exactly_what_the_decoder_reproduces() {
        let original: Vec<i16> = (0..4800).map(|index| (libm::sinf(index as f32 * 0.07) * 9000.0 + libm::sinf(index as f32 * 0.31) * 3000.0) as i16).collect();
        let mut encoder = Encoder::default();
        let mut tracked = Vec::new();
        let bytes: Vec<u8> = original
            .chunks(2)
            .map(|pair| {
                let byte = encoder.encode_pair(pair[0], pair[1]);
                tracked.push(encoder.state().0);
                byte
            })
            .collect();
        let decoded = decode_line(0, 0, &bytes);
        assert_eq!(decoded.len(), original.len());
        // The encoder's predictor after each byte is the decoder's output for its second sample.
        assert_eq!(decoded.iter().skip(1).step_by(2).copied().collect::<Vec<_>>(), tracked);
        for (index, (&got, &want)) in decoded.iter().zip(&original).enumerate().skip(40) {
            assert!((got as i32 - want as i32).abs() < 700, "sample {index}: {got} vs {want}");
        }
    }

    #[test]
    fn a_stream_cut_into_lines_decodes_line_by_line_from_each_lines_state() {
        let original: Vec<i16> = (0..3000).map(|index| (libm::sinf(index as f32 * 0.05) * 15000.0) as i16).collect();
        let mut encoder = Encoder::default();
        let mut whole = Vec::new();
        let mut lines = Vec::new();
        for piece in original.chunks(600) {
            let (predictor, index) = encoder.state();
            let bytes: Vec<u8> = piece.chunks(2).map(|pair| encoder.encode_pair(pair[0], pair[1])).collect();
            whole.extend_from_slice(&bytes);
            lines.push((predictor, index, bytes));
        }
        let from_start = decode_line(0, 0, &whole);
        // Dropping the second line loses only its stretch: the third still decodes exactly.
        let third = &lines[2];
        assert_eq!(decode_line(third.0, third.1, &third.2), from_start[1200..1800]);
        let joined: Vec<i16> = lines.iter().flat_map(|(predictor, index, bytes)| decode_line(*predictor, *index, bytes)).collect();
        assert_eq!(joined, from_start);
    }

    #[test]
    fn extremes_stay_in_range() {
        let mut encoder = Encoder::default();
        let samples = [i16::MAX, i16::MIN, i16::MAX, i16::MIN, 0, 0, i16::MAX, i16::MAX];
        let bytes: Vec<u8> = samples.chunks(2).map(|pair| encoder.encode_pair(pair[0], pair[1])).collect();
        let decoded = decode_line(0, 0, &bytes);
        assert_eq!(decoded.len(), samples.len());
        let (_, index) = encoder.state();
        assert!(index <= 88);
        assert_eq!(Decoder::with_state(-5, 200).state(), (-5, 88), "an index past the table is clamped");
    }
}
