//! Stands in for `desktop/src/making.rs` in the headless probes. `status.rs` and `client.rs` name its types, but the
//! probes never read the Character being made, and the real module needs much of the app. These accept any JSON.

pub type Making = serde::de::IgnoredAny;
pub type Voice = serde::de::IgnoredAny;
pub type MadeCharacter = serde::de::IgnoredAny;
pub type Line = serde::de::IgnoredAny;
pub type Request = serde_json::Value;
