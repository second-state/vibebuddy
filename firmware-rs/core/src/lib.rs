//! The hardware-independent part of the Vibe Buddy firmware.
//!
//! The device layer (`firmware-rs/device`, esp-hal + embassy) only wires pins, I2C, I2S, the LCD,
//! the serial port and flash onto [`firmware::Board`]; the state machine, drawing, protocol handling,
//! storage formats and chip register sequences all live here, no_std, and `cargo test` runs them on the Mac.
#![no_std]
#![allow(clippy::result_unit_err, reason = "hardware ports only report success or failure: the device layer knows the details, and the protocol reports just one line anyway")]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod text;

pub mod adpcm;
pub mod audio;
pub mod buttons;
pub mod canvas;
pub mod character_pack;
pub mod display;
pub mod firmware;
pub mod lcd;
pub mod leisure;
pub mod lines;
pub mod menu;
pub mod pomodoro;
pub mod storage;
pub mod voice_pack;
pub mod voices;

/// Time compression for on-device acceptance testing: with the `fast-clock` feature, the leisure and
/// pomodoro durations are divided by 60.
#[cfg(feature = "fast-clock")]
pub const TIME_SCALE: u32 = 60;
#[cfg(not(feature = "fast-clock"))]
pub const TIME_SCALE: u32 = 1;
