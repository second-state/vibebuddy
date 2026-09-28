//! The message envelope between the Mac and the device. The daemon encodes it and the firmware
//! decodes it: one type, compiled once on each side, so a field mismatch fails to compile. The
//! firmware has no std, so this crate uses only `core` and `alloc`; the `std` feature adds nothing
//! but a `std::error::Error` impl.
#![no_std]

extern crate alloc;
#[cfg(any(feature = "std", test))]
extern crate std;

use alloc::borrow::ToOwned;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const VERSION: i64 = 1;
pub const MAX_MESSAGE_BYTES: usize = 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Event {
    pub version: i64,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Event {
    /// An envelope with only an event name; device maintenance commands (hello, identify, screenshot) all look like this.
    pub fn named(event: &str) -> Self {
        Self {
            version: VERSION,
            event: event.to_owned(),
            id: None,
            title: None,
            message: None,
            extra: BTreeMap::new(),
        }
    }

    pub fn to_ndjson(&self) -> Result<Vec<u8>, ProtocolError> {
        self.validate()?;

        let mut bytes = serde_json::to_vec(self).map_err(ProtocolError::Serialize)?;
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(ProtocolError::MessageTooLarge(bytes.len()));
        }
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.version != VERSION {
            return Err(ProtocolError::UnsupportedVersion(self.version));
        }
        if self.event.trim().is_empty() {
            return Err(ProtocolError::EmptyEvent);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum ProtocolError {
    EmptyEvent,
    MessageTooLarge(usize),
    Serialize(serde_json::Error),
    UnsupportedVersion(i64),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyEvent => write!(formatter, "event must not be empty"),
            Self::MessageTooLarge(size) => {
                write!(
                    formatter,
                    "message is {size} bytes, over the {MAX_MESSAGE_BYTES}-byte limit"
                )
            }
            Self::Serialize(error) => write!(formatter, "JSON encoding failed: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported Vibe Buddy Protocol version {version}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ProtocolError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_without_id_is_valid_ndjson() {
        let event: Event =
            serde_json::from_str(r#"{"version":1,"event":"task.done","title":"Hello"}"#)
                .expect("hello should parse");

        assert_eq!(
            event.to_ndjson().expect("hello should encode"),
            b"{\"version\":1,\"event\":\"task.done\",\"title\":\"Hello\"}\n"
        );
    }

    #[test]
    fn unknown_fields_survive_encoding() {
        let event: Event =
            serde_json::from_str(r#"{"version":1,"event":"task.progress","progress":42}"#)
                .expect("extension fields should parse");

        let encoded = String::from_utf8(event.to_ndjson().expect("extension fields should encode"))
            .expect("encoding must be UTF-8");
        assert!(encoded.contains(r#""progress":42"#));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let event: Event =
            serde_json::from_str(r#"{"version":2,"event":"task.done"}"#).expect("message structure should parse");

        assert!(matches!(
            event.to_ndjson(),
            Err(ProtocolError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn oversized_message_is_rejected() {
        let event = Event {
            version: VERSION,
            event: "message".to_owned(),
            id: None,
            title: None,
            message: Some("x".repeat(MAX_MESSAGE_BYTES)),
            extra: BTreeMap::new(),
        };

        assert!(matches!(
            event.to_ndjson(),
            Err(ProtocolError::MessageTooLarge(_))
        ));
    }
}
