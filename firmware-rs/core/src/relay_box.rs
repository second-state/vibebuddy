//! The box's side of the Relay protocol (docs/relay-protocol.md), without the socket: what to answer,
//! what to report and which lines to act on. The device's Relay task and the simulator's `--relay`
//! mode each move text frames; this decides what is in them.
//!
//! The box signs the challenge with its key, then keeps the Relay told who it is paired with and
//! what it is linked to on its own (`linked` via usb or lan, or `unlinked`). When the Relay links a
//! computer to it, the box sends `link.noise` and answers the computer's handshake (ADR-0015); after
//! that every `data` line either way is sealed, and only those carry the computer's lines.

use alloc::string::String;
use alloc::vec::Vec;

use serde_json::{Value, json};

use crate::firmware::{Board, Firmware, LocalLink};
use crate::link;
use crate::noise::{self, Opener};

/// What the box last told the Relay, so it says only what changed.
#[derive(Default)]
pub struct RelayBox {
    welcomed: bool,
    paired: Option<Vec<(String, String)>>,
    local: Option<LocalLink>,
    /// The computer the Relay linked to the box, waiting for or holding a session.
    computer: Option<[u8; 32]>,
    /// The session with it: the box's output goes to it, sealed by the board.
    opener: Option<Opener>,
}

/// The Relay protocol version the box speaks (docs/relay-protocol.md).
pub const PROTOCOL_VERSION: u32 = 1;

/// The Relay a box uses until the computer on the cable names another (docs/relay-protocol.md, "Which Relay").
pub const DEFAULT_URL: &str = "wss://relay.korekore.ai";

/// The host of a Relay URL the box can use: `wss://` and a host name, nothing else. The box always
/// connects on port 443 to `/v1/boxes/<box-id>`.
pub fn url_host(url: &str) -> Option<&str> {
    let host = url.strip_prefix("wss://")?;
    let host = host.strip_suffix('/').unwrap_or(host);
    let valid = !host.is_empty()
        && host.len() <= 253
        && host.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-']);
    valid.then_some(host)
}

/// The exact text the box sends every 30 s as a sign of life; the Relay answers it without waking.
pub const PING: &str = r#"{"t":"ping"}"#;

/// What the box signs to prove its key (docs/relay-protocol.md).
pub fn signed_message(role: &str, box_id: &str, nonce: &str) -> Vec<u8> {
    alloc::format!("vibebuddy-relay-v1\n{role}\n{box_id}\n{nonce}").into_bytes()
}

fn text(value: Value) -> String {
    serde_json::to_string(&value).unwrap_or_default()
}

/// One sealed line for the computer linked through the Relay, as a `data` message.
pub fn data(line: &str) -> String {
    text(json!({"t": "data", "line": line}))
}

impl RelayBox {
    pub fn new() -> Self {
        Self::default()
    }

    /// A computer is linked through the Relay, in a session.
    pub fn has_peer(&self) -> bool {
        self.opener.is_some()
    }

    /// A new socket's challenge: the hello to send, or None without a key.
    pub fn hello(&mut self, firmware: &Firmware, nonce: &str) -> Option<String> {
        *self = Self::new();
        let link = firmware.link()?;
        let box_id = link::encode(&link.public_key());
        let sig = link.sign(&signed_message("box", &box_id, nonce));
        Some(text(json!({"t": "hello", "role": "box", "key": box_id, "sig": link::encode(&sig), "v": PROTOCOL_VERSION})))
    }

    /// A message from the Relay other than the challenge; returns what to send the Relay at once, before
    /// any of the box's output.
    pub fn message<B: Board>(&mut self, firmware: &mut Firmware, board: &mut B, value: &Value) -> Vec<String> {
        let mut out = Vec::new();
        match value["t"].as_str() {
            Some("welcome") => {
                self.welcomed = true;
                firmware.set_relay_up(true);
            }
            // A computer linked, or linked again: a new session either way.
            Some("linked") => {
                let key = value["computer"].as_str().and_then(link::decode_key);
                firmware.relay_linked(board.now_ms(), key);
                self.end_session(board);
                self.computer = key;
                if key.is_some() {
                    out.extend(self.opener_line(firmware));
                }
            }
            // Only the computer's link ended, and its session with it; the socket stays up and the box keeps reporting.
            Some("unlinked") => {
                firmware.relay_linked(board.now_ms(), None);
                self.end_session(board);
                self.computer = None;
            }
            Some("data") => {
                let Some(line) = value["line"].as_str() else { return out };
                if let Some(opener) = &mut self.opener {
                    match opener.open(line.as_bytes()) {
                        Some(line) => firmware.relay_line(board, &line),
                        // Tampered, replayed or lost: start over, and tell the computer so.
                        None => {
                            self.end_session(board);
                            out.extend(self.opener_line(firmware));
                        }
                    }
                } else if let (Some(link), Some(ephemeral)) = (firmware.link(), board.random())
                    && let Ok(accepted) = noise::accept(link, ephemeral, line.as_bytes())
                    // With the computer the Relay linked. A box that missed `linked` (it was restarting when the
                    // computer linked) takes a handshake from any paired computer: the Relay passes on data only from
                    // the computer it linked, and a handshake only succeeds for a paired one.
                    && self.computer.is_none_or(|computer| computer == accepted.computer)
                {
                    if self.computer.is_none() {
                        self.computer = Some(accepted.computer);
                        firmware.relay_linked(board.now_ms(), Some(accepted.computer));
                    }
                    out.push(data(&accepted.reply));
                    self.opener = Some(accepted.opener);
                    board.relay_peer(Some(accepted.sealer));
                }
                // Anything else in the clear is never acted on.
            }
            _ => {}
        }
        out
    }

    fn opener_line(&self, firmware: &Firmware) -> Option<String> {
        firmware.link().map(|link| data(&noise::opener(&link.public_key())))
    }

    fn end_session<B: Board>(&mut self, board: &mut B) {
        if self.opener.take().is_some() {
            board.relay_peer(None);
        }
    }

    /// The socket closed: nobody is linked through it, and everything is said again on the next welcome.
    pub fn gone<B: Board>(&mut self, firmware: &mut Firmware, board: &mut B) {
        firmware.relay_linked(board.now_ms(), None);
        firmware.set_relay_up(false);
        self.end_session(board);
        *self = Self::new();
    }

    /// What has changed since the last report: the paired list, and what the box is linked to. At most `room`
    /// messages, the room in the queue to the Relay: what doesn't fit is said on a later call, so a full queue never
    /// loses the news that the cable took the box.
    pub fn report(&mut self, firmware: &Firmware, now: u32, room: usize) -> Vec<String> {
        let mut out = Vec::new();
        if !self.welcomed {
            return out;
        }
        let Some(link) = firmware.link() else { return out };
        let paired: Vec<(String, String)> = link.paired.iter().map(|paired| (link::encode(&paired.key), paired.name.clone())).collect();
        if self.paired.as_ref() != Some(&paired) {
            if out.len() == room {
                return out;
            }
            let computers: Vec<Value> = paired.iter().map(|(key, name)| json!({"key": key, "name": name})).collect();
            out.push(text(json!({"t": "paired", "computers": computers})));
            self.paired = Some(paired);
        }
        let local = firmware.local_link(now);
        if self.local != Some(local) {
            // Nothing to undo on a fresh socket that was never linked.
            if self.local.is_some() || local != LocalLink::None {
                if out.len() == room {
                    return out;
                }
                out.push(text(match local {
                    LocalLink::Usb(key) => json!({"t": "linked", "via": "usb", "computer": key.map(|key| link::encode(&key))}),
                    LocalLink::Lan(key) => json!({"t": "linked", "via": "lan", "computer": link::encode(&key)}),
                    LocalLink::None => json!({"t": "unlinked"}),
                }));
            }
            self.local = Some(local);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_wss_host_is_a_relay_url() {
        assert_eq!(url_host(DEFAULT_URL), Some("relay.korekore.ai"));
        assert_eq!(url_host("wss://relay-staging.korekore.ai/"), Some("relay-staging.korekore.ai"));
        for bad in ["ws://relay.korekore.ai", "wss://", "wss://host:8443", "wss://host/v1", "https://host", "wss://-host", "wss://a b"] {
            assert_eq!(url_host(bad), None, "{bad}");
        }
    }
}
