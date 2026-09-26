//! Vibe Buddy 固件里与硬件无关的部分。
//!
//! 设备层（`firmware-rs/device`，esp-hal + embassy）只负责把引脚、I2C、I2S、
//! LCD、串口、flash 接到 [`firmware::Board`] 上；状态机、绘制、协议处理、存储
//! 格式、芯片寄存器序列都在这里，no_std，Mac 上 `cargo test` 就能跑。
#![no_std]
#![allow(clippy::result_unit_err, reason = "硬件口只报成败：失败的细节设备层自己知道，协议上也只报一行")]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod text;

pub mod audio;
pub mod buttons;
pub mod canvas;
pub mod display;
pub mod firmware;
pub mod lcd;
pub mod leisure;
pub mod pomodoro;
pub mod storage;
pub mod voice_pack;
pub mod voices;

/// 实机验收用的时间压缩：带 `fast-clock` 特性时，休闲与番茄钟的时限都除以 60。
#[cfg(feature = "fast-clock")]
pub const TIME_SCALE: u32 = 60;
#[cfg(not(feature = "fast-clock"))]
pub const TIME_SCALE: u32 = 1;
