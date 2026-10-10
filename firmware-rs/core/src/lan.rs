//! The box's side of a link over the local network (ADR-0011, ADR-0012): who may connect, and which
//! one computer the box is linked to. The device layer runs the TCP server and hands each connection
//! here by a number; this decides, and says what to send, what to close and which lines to act on.
//!
//! A connection starts with a handshake in which both ends prove their keys. The box sends
//! `link.challenge` with a nonce. The computer answers `link.hello` with its key, its signature over
//! that nonce and a nonce of its own. The box replies `link.welcome` with its signature over the
//! computer's nonce, or `link.busy` and closes. Only after that are the computer's lines the
//! VibeBuddy Protocol. Lines before the welcome, and from a computer that isn't paired, are never
//! acted on.
//!
//! A box links to one computer at a time. USB wins: while a computer talks over the cable, a LAN
//! link is refused or ended. Otherwise a paired computer links when the box is free, when it was the
//! one linked last, or once the last one has been gone for [`GRACE_MS`]; a `takeover` hello, which
//! the user asked for, always gets the box.

use alloc::string::String;
use alloc::vec::Vec;

use serde_json::Value;

use crate::link::{self, Link};

/// How long a linked computer must have been gone before another paired computer may link on its own.
pub const GRACE_MS: u32 = 10 * 60_000;
/// How long a connection may take to finish the handshake.
pub const HANDSHAKE_MS: u32 = 10_000;
/// How long after the last line over USB the cable still counts as in use; the same silence makes the
/// firmware call the link lost.
pub const USB_QUIET_MS: u32 = 15_000;
/// Longest line read from a connection, the protocol's limit plus room for the envelope.
const MAX_LINE_BYTES: usize = 1024;

/// A connection, numbered by the device layer.
pub type Conn = u32;

/// What the device layer must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Write this line (without its newline) to the connection.
    Send(Conn, String),
    Close(Conn),
    /// A line from the linked computer, to be handled as the VibeBuddy Protocol.
    Line(Vec<u8>),
    /// The box's diagnostic output should now go to this connection too, or no longer to any.
    Peer(Option<Conn>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Challenged with this nonce at this time.
    Waiting { nonce: [u8; 32], since: u32 },
    Linked,
}

struct Session {
    conn: Conn,
    state: State,
    /// Bytes of a line not yet ended.
    partial: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Owner {
    key: [u8; 32],
    conn: Option<Conn>,
    /// When its connection ended; None while linked.
    gone_since: Option<u32>,
}

#[derive(Default)]
pub struct Lan {
    sessions: Vec<Session>,
    owner: Option<Owner>,
    /// The last time a line came over USB.
    usb_at: Option<u32>,
}

/// What a computer signs to link: bound to the role, the box and the nonce it was given.
pub fn signed_message(role: &str, box_key: &[u8; 32], nonce: &[u8; 32]) -> Vec<u8> {
    alloc::format!("vibebuddy-lan-v1\n{role}\n{}\n{}", link::encode(box_key), link::encode(nonce)).into_bytes()
}

impl Lan {
    pub fn new() -> Self {
        Self::default()
    }

    /// The computer linked over the network right now.
    pub fn linked_key(&self) -> Option<[u8; 32]> {
        self.owner.filter(|owner| owner.gone_since.is_none() && owner.conn.is_some()).map(|owner| owner.key)
    }

    /// A new connection. `nonce` must be fresh randomness.
    pub fn opened(&mut self, link: &Link, conn: Conn, nonce: [u8; 32], now: u32) -> Vec<Output> {
        self.sessions.push(Session { conn, state: State::Waiting { nonce, since: now }, partial: Vec::new() });
        let line = alloc::format!(
            r#"{{"version":1,"event":"link.challenge","box":"{}","nonce":"{}"}}"#,
            link::encode(&link.public_key()),
            link::encode(&nonce)
        );
        alloc::vec![Output::Send(conn, line)]
    }

    /// Bytes read from a connection.
    pub fn received(&mut self, link: &Link, conn: Conn, bytes: &[u8], now: u32) -> Vec<Output> {
        let mut out = Vec::new();
        let mut rest = bytes;
        while !rest.is_empty() {
            let Some(session) = self.sessions.iter_mut().find(|session| session.conn == conn) else { break };
            let (piece, line_ended) = match rest.iter().position(|&byte| byte == b'\n') {
                Some(end) => (&rest[..end], true),
                None => (rest, false),
            };
            rest = if line_ended { &rest[piece.len() + 1..] } else { &[] };
            if session.partial.len() + piece.len() > MAX_LINE_BYTES {
                out.extend(self.drop_conn(conn, now));
                out.push(Output::Close(conn));
                break;
            }
            session.partial.extend_from_slice(piece);
            if !line_ended {
                break;
            }
            let mut line = core::mem::take(&mut session.partial);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            match session.state {
                State::Linked => out.push(Output::Line(line)),
                State::Waiting { nonce, .. } => out.extend(self.hello(link, conn, nonce, &line, now)),
            }
        }
        out
    }

    /// A connection ended, from either side.
    pub fn closed(&mut self, conn: Conn, now: u32) -> Vec<Output> {
        self.drop_conn(conn, now)
    }

    /// A line came over USB: the cable is in use, and a computer linked over the network gives way.
    pub fn usb_line(&mut self, now: u32) -> Vec<Output> {
        self.usb_at = Some(now);
        let mut out = Vec::new();
        if let Some(conn) = self.owner.and_then(|owner| owner.conn) {
            out.push(Output::Send(conn, unlinked("usb")));
            out.push(Output::Close(conn));
            self.sessions.retain(|session| session.conn != conn);
            out.push(Output::Peer(None));
        }
        // The cable's computer has the box; nobody is waiting out the grace period for it.
        self.owner = None;
        out
    }

    /// Closes connections that took too long to finish the handshake.
    pub fn poll(&mut self, now: u32) -> Vec<Output> {
        let stale: Vec<Conn> = self
            .sessions
            .iter()
            .filter(|session| matches!(session.state, State::Waiting { since, .. } if now.wrapping_sub(since) >= HANDSHAKE_MS))
            .map(|session| session.conn)
            .collect();
        let mut out = Vec::new();
        for conn in stale {
            out.push(Output::Send(conn, error("timeout")));
            out.push(Output::Close(conn));
            self.sessions.retain(|session| session.conn != conn);
        }
        out
    }

    /// Forgets a computer that was unpaired, ending its link.
    pub fn unpaired(&mut self, key: &[u8; 32], now: u32) -> Vec<Output> {
        match self.owner {
            Some(owner) if &owner.key == key => {
                let mut out = Vec::new();
                if let Some(conn) = owner.conn {
                    out.push(Output::Send(conn, unlinked("unpaired")));
                    out.push(Output::Close(conn));
                    out.extend(self.drop_conn(conn, now));
                }
                self.owner = None;
                out
            }
            _ => Vec::new(),
        }
    }

    fn hello(&mut self, link: &Link, conn: Conn, nonce: [u8; 32], line: &[u8], now: u32) -> Vec<Output> {
        let refuse = |code: &str| alloc::vec![Output::Send(conn, error(code)), Output::Close(conn)];
        let Ok(value) = serde_json::from_slice::<Value>(line) else { return self.refused(conn, refuse("bad_message")) };
        let text = |name: &str| value.get(name).and_then(Value::as_str);
        let key = text("key").and_then(link::decode::<32>);
        let signature = text("sig").and_then(link::decode::<64>);
        let theirs = text("nonce").and_then(link::decode::<32>);
        let (Some("link.hello"), Some(key), Some(signature), Some(theirs)) = (text("event"), key, signature, theirs) else {
            return self.refused(conn, refuse("bad_message"));
        };
        let box_key = link.public_key();
        if !link.is_paired(&key) || !link::verify(&key, &signed_message("computer", &box_key, &nonce), &signature) {
            return self.refused(conn, refuse("auth"));
        }
        let takeover = value.get("takeover") == Some(&Value::Bool(true));
        let usb_in_use = self.usb_at.is_some_and(|at| now.wrapping_sub(at) < USB_QUIET_MS);
        let free = match self.owner {
            None => true,
            Some(owner) => owner.key == key || owner.gone_since.is_some_and(|since| now.wrapping_sub(since) >= GRACE_MS),
        };
        if usb_in_use || !(free || takeover) {
            let reason = if usb_in_use { "usb" } else { "linked" };
            let busy = alloc::format!(r#"{{"version":1,"event":"link.busy","reason":"{reason}"}}"#);
            return self.refused(conn, alloc::vec![Output::Send(conn, busy), Output::Close(conn)]);
        }

        let mut out = Vec::new();
        if let Some(previous) = self.owner.and_then(|owner| owner.conn).filter(|&previous| previous != conn) {
            out.push(Output::Send(previous, unlinked("takeover")));
            out.push(Output::Close(previous));
            self.sessions.retain(|session| session.conn != previous);
        }
        self.owner = Some(Owner { key, conn: Some(conn), gone_since: None });
        if let Some(session) = self.sessions.iter_mut().find(|session| session.conn == conn) {
            session.state = State::Linked;
        }
        let sig = link.sign(&signed_message("box", &box_key, &theirs));
        out.push(Output::Send(conn, alloc::format!(r#"{{"version":1,"event":"link.welcome","sig":"{}"}}"#, link::encode(&sig))));
        out.push(Output::Peer(Some(conn)));
        out
    }

    fn refused(&mut self, conn: Conn, out: Vec<Output>) -> Vec<Output> {
        self.sessions.retain(|session| session.conn != conn);
        out
    }

    fn drop_conn(&mut self, conn: Conn, now: u32) -> Vec<Output> {
        self.sessions.retain(|session| session.conn != conn);
        match &mut self.owner {
            Some(owner) if owner.conn == Some(conn) => {
                owner.conn = None;
                owner.gone_since = Some(now);
                alloc::vec![Output::Peer(None)]
            }
            _ => Vec::new(),
        }
    }
}

fn error(code: &str) -> String {
    alloc::format!(r#"{{"version":1,"event":"link.error","code":"{code}"}}"#)
}

fn unlinked(reason: &str) -> String {
    alloc::format!(r#"{{"version":1,"event":"link.unlinked","reason":"{reason}"}}"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::string::ToString;

    struct Computer {
        key: SigningKey,
    }

    impl Computer {
        fn new(seed: u8) -> Self {
            Self { key: SigningKey::from_bytes(&[seed; 32]) }
        }

        fn public(&self) -> [u8; 32] {
            self.key.verifying_key().to_bytes()
        }

        /// Answers a challenge line.
        fn hello(&self, challenge: &str, takeover: bool) -> Vec<u8> {
            let value: Value = serde_json::from_str(challenge).unwrap();
            let box_key = link::decode::<32>(value["box"].as_str().unwrap()).unwrap();
            let nonce = link::decode::<32>(value["nonce"].as_str().unwrap()).unwrap();
            let sig = self.key.sign(&signed_message("computer", &box_key, &nonce)).to_bytes();
            alloc::format!(
                r#"{{"version":1,"event":"link.hello","key":"{}","sig":"{}","nonce":"{}","takeover":{takeover}}}"#,
                link::encode(&self.public()),
                link::encode(&sig),
                link::encode(&[9; 32])
            )
            .into_bytes()
            .into_iter()
            .chain(*b"\n")
            .collect()
        }
    }

    fn sent(out: &[Output], conn: Conn) -> Vec<String> {
        out.iter()
            .filter_map(|output| match output {
                Output::Send(to, line) if *to == conn => Some(line.clone()),
                _ => None,
            })
            .collect()
    }

    fn event(line: &str) -> String {
        serde_json::from_str::<Value>(line).unwrap()["event"].as_str().unwrap().to_string()
    }

    fn setup(computers: &[&Computer]) -> (Lan, Link) {
        let mut link = Link::new([1; 32]);
        for (index, computer) in computers.iter().enumerate() {
            link.pair(computer.public(), if index == 0 { "a" } else { "b" }).unwrap();
        }
        (Lan::new(), link)
    }

    /// Opens a connection and says hello; returns what came back.
    fn connect(lan: &mut Lan, link: &Link, conn: Conn, computer: &Computer, takeover: bool, now: u32) -> Vec<Output> {
        let challenge = lan.opened(link, conn, [conn as u8; 32], now);
        let Output::Send(_, line) = &challenge[0] else { panic!() };
        lan.received(link, conn, &computer.hello(line, takeover), now)
    }

    #[test]
    fn a_paired_computer_links_and_the_box_proves_itself() {
        let a = Computer::new(2);
        let (mut lan, link) = setup(&[&a]);
        let out = connect(&mut lan, &link, 1, &a, false, 0);
        let welcome = &sent(&out, 1)[0];
        assert_eq!(event(welcome), "link.welcome");
        let value: Value = serde_json::from_str(welcome).unwrap();
        let sig = link::decode::<64>(value["sig"].as_str().unwrap()).unwrap();
        assert!(link::verify(&link.public_key(), &signed_message("box", &link.public_key(), &[9; 32]), &sig));
        assert!(out.contains(&Output::Peer(Some(1))));
        assert_eq!(lan.linked_key(), Some(a.public()));

        let out = lan.received(&link, 1, b"{\"version\":1,\"event\":\"device.hello\"}\n", 10);
        assert_eq!(out, [Output::Line(b"{\"version\":1,\"event\":\"device.hello\"}".to_vec())]);
    }

    #[test]
    fn an_unpaired_computer_or_a_bad_signature_is_turned_away() {
        let a = Computer::new(2);
        let stranger = Computer::new(3);
        let (mut lan, link) = setup(&[&a]);
        let out = connect(&mut lan, &link, 1, &stranger, false, 0);
        assert_eq!(sent(&out, 1), [error("auth")]);
        assert!(out.contains(&Output::Close(1)));

        let challenge = lan.opened(&link, 2, [2; 32], 0);
        let Output::Send(_, line) = &challenge[0] else { panic!() };
        // Signed over another nonce.
        let forged = line.replace(&link::encode(&[2; 32]), &link::encode(&[7; 32]));
        let out = lan.received(&link, 2, &a.hello(&forged, false), 0);
        assert_eq!(sent(&out, 2), [error("auth")]);
        assert_eq!(lan.linked_key(), None);
    }

    #[test]
    fn nothing_before_the_welcome_is_acted_on() {
        let a = Computer::new(2);
        let (mut lan, link) = setup(&[&a]);
        lan.opened(&link, 1, [1; 32], 0);
        let out = lan.received(&link, 1, b"{\"version\":1,\"event\":\"device.volume\",\"level\":20}\n", 0);
        assert!(!out.iter().any(|output| matches!(output, Output::Line(_))));
        assert!(out.contains(&Output::Close(1)));
    }

    #[test]
    fn a_second_computer_is_busy_until_grace_or_takeover() {
        let a = Computer::new(2);
        let b = Computer::new(3);
        let (mut lan, link) = setup(&[&a, &b]);
        connect(&mut lan, &link, 1, &a, false, 0);
        let out = connect(&mut lan, &link, 2, &b, false, 1000);
        assert_eq!(event(&sent(&out, 2)[0]), "link.busy");

        let out = connect(&mut lan, &link, 3, &b, true, 2000);
        assert_eq!(sent(&out, 1), [unlinked("takeover")]);
        assert!(out.contains(&Output::Close(1)));
        assert_eq!(lan.linked_key(), Some(b.public()));

        // b goes away; a may come back only after the grace period, unless it takes over.
        lan.closed(3, 5000);
        assert_eq!(event(&sent(&connect(&mut lan, &link, 4, &a, false, 5000 + GRACE_MS - 1), 4)[0]), "link.busy");
        assert_eq!(event(&sent(&connect(&mut lan, &link, 5, &a, false, 5000 + GRACE_MS), 5)[0]), "link.welcome");
    }

    #[test]
    fn the_last_computer_comes_back_at_once() {
        let a = Computer::new(2);
        let (mut lan, link) = setup(&[&a]);
        connect(&mut lan, &link, 1, &a, false, 0);
        assert_eq!(lan.closed(1, 100), [Output::Peer(None)]);
        assert_eq!(event(&sent(&connect(&mut lan, &link, 2, &a, false, 200), 2)[0]), "link.welcome");
    }

    #[test]
    fn usb_wins() {
        let a = Computer::new(2);
        let (mut lan, link) = setup(&[&a]);
        connect(&mut lan, &link, 1, &a, false, 0);
        let out = lan.usb_line(100);
        assert_eq!(sent(&out, 1), [unlinked("usb")]);
        assert!(out.contains(&Output::Peer(None)));
        let busy = &sent(&connect(&mut lan, &link, 2, &a, true, 200), 2)[0];
        assert!(busy.contains(r#""reason":"usb""#), "not even a takeover while the cable talks: {busy}");
        assert_eq!(event(&sent(&connect(&mut lan, &link, 3, &a, false, 100 + USB_QUIET_MS), 3)[0]), "link.welcome");
    }

    #[test]
    fn a_slow_handshake_is_closed() {
        let a = Computer::new(2);
        let (mut lan, link) = setup(&[&a]);
        lan.opened(&link, 1, [1; 32], 0);
        assert!(lan.poll(HANDSHAKE_MS - 1).is_empty());
        assert_eq!(lan.poll(HANDSHAKE_MS), [Output::Send(1, error("timeout")), Output::Close(1)]);
    }

    #[test]
    fn lines_split_across_reads_are_joined() {
        let a = Computer::new(2);
        let (mut lan, link) = setup(&[&a]);
        connect(&mut lan, &link, 1, &a, false, 0);
        assert!(lan.received(&link, 1, b"{\"version\":1,", 0).is_empty());
        let out = lan.received(&link, 1, b"\"event\":\"x\"}\r\n{\"version\":1,\"event\":\"y\"}\n", 0);
        assert_eq!(out, [Output::Line(b"{\"version\":1,\"event\":\"x\"}".to_vec()), Output::Line(b"{\"version\":1,\"event\":\"y\"}".to_vec())]);
    }

    #[test]
    fn an_overlong_line_drops_the_connection() {
        let a = Computer::new(2);
        let (mut lan, link) = setup(&[&a]);
        connect(&mut lan, &link, 1, &a, false, 0);
        let out = lan.received(&link, 1, &[b'x'; MAX_LINE_BYTES + 1], 0);
        assert!(out.contains(&Output::Close(1)));
        assert_eq!(lan.linked_key(), None);
    }

    #[test]
    fn unpairing_the_linked_computer_ends_its_link() {
        let a = Computer::new(2);
        let (mut lan, link) = setup(&[&a]);
        connect(&mut lan, &link, 1, &a, false, 0);
        let out = lan.unpaired(&a.public(), 10);
        assert_eq!(sent(&out, 1), [unlinked("unpaired")]);
        assert_eq!(lan.linked_key(), None);
    }
}
