//! Voice announcements and the ES8311 codec.
//!
//! The C firmware drives the ES8311 through esp_codec_dev. That library does not exist in Rust, so
//! this ports, one by one, the register reads and writes it actually issues for this board's
//! configuration (slave mode, no MCLK, BCLK as clock source, 24 kHz, 16 bit, standard I2S format),
//! with order and values following es8311.c in esp_codec_dev 1.6.2: `es8311_codec_new` (open) ->
//! `esp_codec_dev_open` (set_fs, enable) -> set volume. The tests hold the expected sequence.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prompt {
    InputRequired = 0,
    Done = 1,
    Failed = 2,
    /// Pomodoro: played once at the end of focus and once at the end of a break.
    FocusDone = 3,
    BreakDone = 4,
}

impl Prompt {
    pub const ALL: [Prompt; 5] = [Prompt::InputRequired, Prompt::Done, Prompt::Failed, Prompt::FocusDone, Prompt::BreakDone];
}

/// Audio sample rate: both built-in and voice pack PCM are 24 kHz, 16 bit, stereo, little endian.
pub const SAMPLE_RATE: u32 = 24000;

/// Volume on the codec's 0 to 100 scale. The floor is above zero: a zero volume that
/// could be saved would be a persistent mute through the back door, and mute is
/// deliberately not persisted.
pub const VOLUME_MIN: u32 = 20;
pub const VOLUME_MAX: u32 = 100;
pub const VOLUME_DEFAULT: u32 = 65;

pub fn clamp_volume(level: u32) -> u32 {
    level.clamp(VOLUME_MIN, VOLUME_MAX)
}

pub const ES8311_ADDRESS: u8 = 0x18;

/// The ES8311 register port: read one, write one. I2C on the device, a recorder in tests.
pub trait Registers {
    type Error;
    fn read(&mut self, register: u8) -> Result<u8, Self::Error>;
    fn write(&mut self, register: u8, value: u8) -> Result<(), Self::Error>;
}

fn update<R: Registers>(codec: &mut R, register: u8, change: impl FnOnce(u8) -> u8) -> Result<(), R::Error> {
    let value = codec.read(register)?;
    codec.write(register, change(value))
}

/// esp_codec_dev's volume conversion: 0-100 maps to -50-0 dB (0 is -96 dB), minus the hardware
/// gain of a 5 V amplifier and 3.3 V DAC, then linearly rounded down with register 0x00 = -95.5 dB
/// and 0xFF = +32 dB.
pub fn volume_register(volume: u32) -> u8 {
    let db = if volume == 0 {
        -96.0f32
    } else if volume >= 100 {
        0.0
    } else {
        -50.0 + volume as f32 * 0.5
    };
    let hardware_gain = 20.0 * libm::log10f(3.3 / 5.0);
    let db = db - hardware_gain;
    if db >= 32.0 {
        return 0xFF;
    }
    if db <= -95.5 {
        return 0x00;
    }
    let ratio = 255.0f32 / (32.0 - -95.5);
    ((db - -95.5) * ratio) as i32 as u8
}

/// Opens the codec and starts playback, then sets the volume. Each step matches one read or write
/// in es8311.c.
pub fn start_es8311<R: Registers>(codec: &mut R, volume: u32) -> Result<(), R::Error> {
    // es8311_open
    let system = codec.read(0x0D)?;
    if system != 0xFA {
        codec.write(0x0D, 0xFA)?;
    }
    // Improve I2C noise immunity; the first write occasionally fails, so write it twice.
    codec.write(0x44, 0x08)?;
    codec.write(0x44, 0x08)?;
    for (register, value) in [
        (0x01, 0x30),
        (0x02, 0x00),
        (0x03, 0x10),
        (0x16, 0x24),
        (0x04, 0x10),
        (0x05, 0x00),
        (0x0B, 0x00),
        (0x0C, 0x00),
        (0x10, 0x1F),
        (0x11, 0x7F),
        (0x00, 0x80),
    ] {
        codec.write(register, value)?;
    }
    // Slave mode.
    update(codec, 0x00, |value| value & 0xBF)?;
    // Internal MCLK taken from BCLK, not inverted.
    codec.write(0x01, 0xBF)?;
    // SCLK not inverted.
    update(codec, 0x06, |value| value & !0x20)?;
    codec.write(0x13, 0x10)?;
    codec.write(0x1B, 0x0A)?;
    codec.write(0x1C, 0x6A)?;
    codec.write(0x44, 0x58)?;

    // es8311_set_fs: 16 bit, standard I2S format, 24 kHz.
    update(codec, 0x09, |value| value | 0x0C)?;
    update(codec, 0x0A, |value| value | 0x0C)?;
    update(codec, 0x09, |value| value & 0xFC)?;
    update(codec, 0x0A, |value| value & 0xFC)?;
    // Coefficients from the MCLK = 24 kHz × 256 = 6.144 MHz row: pre_div 1, adc/dac_div 1,
    // single speed, osr 0x10, lrck 0x00FF, bclk_div 4. Without MCLK the multiplier is fixed at ×8.
    update(codec, 0x02, |value| (value & 0x07) | (3 << 3))?;
    codec.write(0x05, 0x00)?;
    update(codec, 0x03, |value| (value & 0x80) | 0x10)?;
    update(codec, 0x04, |value| (value & 0x80) | 0x10)?;
    update(codec, 0x07, |value| value & 0xC0)?;
    codec.write(0x08, 0xFF)?;
    update(codec, 0x06, |value| (value & 0xE0) | 3)?;

    // es8311_enable → es8311_start
    codec.write(0x00, 0x80)?;
    codec.write(0x01, 0xBF)?;
    let dac = codec.read(0x09)?;
    let adc = codec.read(0x0A)?;
    codec.write(0x09, dac & 0xBF)?;
    codec.write(0x0A, adc & 0xBF)?;
    codec.write(0x17, 0xBF)?;
    codec.write(0x0E, 0x02)?;
    codec.write(0x12, 0x00)?;
    codec.write(0x14, 0x1A)?;
    update(codec, 0x14, |value| value & !0x40)?;
    codec.write(0x0D, 0x01)?;
    codec.write(0x15, 0x40)?;
    codec.write(0x37, 0x08)?;
    codec.write(0x45, 0x00)?;
    // Unmute.
    update(codec, 0x31, |value| value & 0x9F)?;

    // esp_codec_dev_open finishes by applying the device's initial volume 0 and unmute once each,
    // then the C firmware immediately sets the saved volume.
    codec.write(0x32, volume_register(0))?;
    update(codec, 0x31, |value| value & 0x9F)?;
    set_es8311_volume(codec, volume)
}

pub fn set_es8311_volume<R: Registers>(codec: &mut R, volume: u32) -> Result<(), R::Error> {
    codec.write(0x32, volume_register(volume))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// Records every read and write; all registers start at 0.
    struct Recorder {
        values: [u8; 256],
        log: Vec<(char, u8, u8)>,
    }

    impl Registers for Recorder {
        type Error = ();
        fn read(&mut self, register: u8) -> Result<u8, ()> {
            let value = self.values[register as usize];
            self.log.push(('r', register, value));
            Ok(value)
        }
        fn write(&mut self, register: u8, value: u8) -> Result<(), ()> {
            self.values[register as usize] = value;
            self.log.push(('w', register, value));
            Ok(())
        }
    }


    #[test]
    fn volume_matches_esp_codec_dev() {
        // hw_gain = 20·log10(3.3/5) ≈ -3.609 dB; reg = (db + 3.609 + 95.5) × 2.
        assert_eq!(volume_register(65), 163);
        assert_eq!(volume_register(100), 198);
        assert_eq!(volume_register(20), 118);
        assert_eq!(volume_register(0), 6);
    }

    #[test]
    fn the_register_sequence_matches_the_c_driver() {
        let mut codec = Recorder { values: [0; 256], log: Vec::new() };
        start_es8311(&mut codec, 65).unwrap();
        let writes: Vec<(u8, u8)> =
            codec.log.iter().filter(|(kind, _, _)| *kind == 'w').map(|&(_, register, value)| (register, value)).collect();
        let expected: &[(u8, u8)] = &[
            (0x0D, 0xFA),
            (0x44, 0x08),
            (0x44, 0x08),
            (0x01, 0x30),
            (0x02, 0x00),
            (0x03, 0x10),
            (0x16, 0x24),
            (0x04, 0x10),
            (0x05, 0x00),
            (0x0B, 0x00),
            (0x0C, 0x00),
            (0x10, 0x1F),
            (0x11, 0x7F),
            (0x00, 0x80),
            (0x00, 0x80),
            (0x01, 0xBF),
            (0x06, 0x00),
            (0x13, 0x10),
            (0x1B, 0x0A),
            (0x1C, 0x6A),
            (0x44, 0x58),
            (0x09, 0x0C),
            (0x0A, 0x0C),
            (0x09, 0x0C),
            (0x0A, 0x0C),
            (0x02, 0x18),
            (0x05, 0x00),
            (0x03, 0x10),
            (0x04, 0x10),
            (0x07, 0x00),
            (0x08, 0xFF),
            (0x06, 0x03),
            (0x00, 0x80),
            (0x01, 0xBF),
            (0x09, 0x0C),
            (0x0A, 0x0C),
            (0x17, 0xBF),
            (0x0E, 0x02),
            (0x12, 0x00),
            (0x14, 0x1A),
            (0x14, 0x1A),
            (0x0D, 0x01),
            (0x15, 0x40),
            (0x37, 0x08),
            (0x45, 0x00),
            (0x31, 0x00),
            (0x32, 6),
            (0x31, 0x00),
            (0x32, 163),
        ];
        assert_eq!(writes, expected);
    }
}
