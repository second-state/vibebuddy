//! Decoding a Character pack line: 16 kHz mono IMA ADPCM in, the codec's 24 kHz stereo PCM out.
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

    /// The same IMA encoder as tools/character_pack.py, so the two can be checked against each other.
    fn encode(samples: &[i16]) -> Vec<u8> {
        let mut decoder = Decoder::default();
        let mut out = Vec::new();
        for pair in samples.chunks(2) {
            let mut byte = 0u8;
            for (half, &sample) in pair.iter().enumerate() {
                let step = STEPS[decoder.index as usize];
                let mut diff = sample as i32 - decoder.predictor;
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
                decoder.decode(nibble);
                byte |= nibble << (half * 4);
            }
            out.push(byte);
        }
        out
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
}
