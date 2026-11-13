//! What the VibePlay app does, apart from QingOS: the VibePlay key, pairing and the link (docs/vibeplay.md), and the
//! box's screens drawn by the firmware's own code (`firmware-rs/core`). Plain Rust, so it is tested on the computer.

pub mod buddy;
pub mod frame;
pub mod identity;
pub mod noise;
pub mod pairing;
pub mod server;

/// The box's firmware, whose screens these are.
pub use vibebuddy_firmware_core as firmware;
