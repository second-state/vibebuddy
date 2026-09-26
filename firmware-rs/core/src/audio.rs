//! 语音播报与 ES8311 codec。
//!
//! C 固件经 esp_codec_dev 驱动 ES8311。那套库在 Rust 里没有，这里把它对本板
//! 配置（从模式、不用 MCLK、BCLK 当时钟源、24 kHz、16 bit、I2S 标准格式）
//! 实际发出的寄存器读写逐条搬过来，顺序与取值都按 esp_codec_dev 1.6.2 的
//! es8311.c：`es8311_codec_new`（open）→ `esp_codec_dev_open`（set_fs、
//! enable）→ 设音量。测试里有一份逐条的期望序列。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prompt {
    InputRequired = 0,
    Done = 1,
    Failed = 2,
    /// 番茄钟：专注结束、休息结束各播一次。
    FocusDone = 3,
    BreakDone = 4,
}

impl Prompt {
    pub const ALL: [Prompt; 5] = [Prompt::InputRequired, Prompt::Done, Prompt::Failed, Prompt::FocusDone, Prompt::BreakDone];
}

/// 音频采样率：内置与语音包里的 PCM 都是 24 kHz、16 bit、双声道、小端序。
pub const SAMPLE_RATE: u32 = 24000;

/// 音量：codec 的 0 到 100 刻度。下限不到零——能存下来的零音量就是从后门
/// 做出来的持久静音，而静音有意不持久化。
pub const VOLUME_MIN: u32 = 20;
pub const VOLUME_MAX: u32 = 100;
pub const VOLUME_DEFAULT: u32 = 65;

pub fn clamp_volume(level: u32) -> u32 {
    level.clamp(VOLUME_MIN, VOLUME_MAX)
}

pub const ES8311_ADDRESS: u8 = 0x18;

/// ES8311 的寄存器口：读一个、写一个。设备那边是 I2C，测试里是记录器。
pub trait Registers {
    type Error;
    fn read(&mut self, register: u8) -> Result<u8, Self::Error>;
    fn write(&mut self, register: u8, value: u8) -> Result<(), Self::Error>;
}

fn update<R: Registers>(codec: &mut R, register: u8, change: impl FnOnce(u8) -> u8) -> Result<(), R::Error> {
    let value = codec.read(register)?;
    codec.write(register, change(value))
}

/// esp_codec_dev 的音量换算：0–100 映射到 -50–0 dB（0 是 -96 dB），扣掉
/// 功放 5 V、DAC 3.3 V 的硬件增益，再按寄存器 0x00=-95.5 dB、0xFF=+32 dB
/// 线性取整。
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

/// 打开 codec 并开始播放，最后设上音量。每一步都对应 es8311.c 里的一次读写。
pub fn start_es8311<R: Registers>(codec: &mut R, volume: u32) -> Result<(), R::Error> {
    // es8311_open
    let system = codec.read(0x0D)?;
    if system != 0xFA {
        codec.write(0x0D, 0xFA)?;
    }
    // 增强 I2C 抗噪；第一次写偶尔失败，所以写两遍。
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
    // 从模式。
    update(codec, 0x00, |value| value & 0xBF)?;
    // 内部 MCLK 取自 BCLK，不反相。
    codec.write(0x01, 0xBF)?;
    // SCLK 不反相。
    update(codec, 0x06, |value| value & !0x20)?;
    codec.write(0x13, 0x10)?;
    codec.write(0x1B, 0x0A)?;
    codec.write(0x1C, 0x6A)?;
    codec.write(0x44, 0x58)?;

    // es8311_set_fs：16 bit、I2S 标准格式、24 kHz。
    update(codec, 0x09, |value| value | 0x0C)?;
    update(codec, 0x0A, |value| value | 0x0C)?;
    update(codec, 0x09, |value| value & 0xFC)?;
    update(codec, 0x0A, |value| value & 0xFC)?;
    // MCLK = 24 kHz × 256 = 6.144 MHz 那一行系数：pre_div 1、adc/dac_div 1、
    // 单速、osr 0x10、lrck 0x00FF、bclk_div 4。不用 MCLK 时倍频固定取 ×8。
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
    // 取消静音。
    update(codec, 0x31, |value| value & 0x9F)?;

    // esp_codec_dev_open 最后按设备的初始音量 0 与未静音各补一次，
    // 然后 C 固件立刻设成存下来的音量。
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

    /// 记下每一次读写；寄存器初值都当 0。
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
        // hw_gain = 20·log10(3.3/5) ≈ -3.609 dB；reg = (db + 3.609 + 95.5) × 2。
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
