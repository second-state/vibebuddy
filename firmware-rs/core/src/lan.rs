//! The box's side of a link over the local network (ADR-0011, ADR-0012, ADR-0015): who may connect, and
//! which one computer the box is linked to. The device layer runs the TCP server and hands each connection
//! here by a number; this decides, and says what to send, what to close and which lines to act on.
//!
//! A connection starts with a Noise handshake ([`crate::noise`]). The box sends `link.noise`; the
//! computer answers with the first handshake message, whose payload says whether it takes the box over;
//! the box answers with the second, then, sealed, `link.welcome`, or `link.busy` and closes. Every line
//! after that is sealed both ways, and only those lines are the VibeBuddy Protocol. A computer that isn't
//! paired gets `link.error` in the clear and is closed.
//!
//! A box links to one computer at a time. USB wins: while a computer talks over the cable, a LAN
//! link is refused or ended. Otherwise a paired computer links when the box is free, when it was the
//! one linked last, or once the last one has been gone for [`GRACE_MS`]; a takeover, which the user
//! asked for, always gets the box.

use alloc::string::String;
use alloc::vec::Vec;

use serde_json::Value;

use crate::link::Link;
use crate::noise::{self, Opener, Refused, Sealer};

/// How long a linked computer must have been gone before another paired computer may link on its own.
pub const GRACE_MS: u32 = 10 * 60_000;
/// How long a connection may take to finish the handshake.
pub const HANDSHAKE_MS: u32 = 10_000;
/// How long after the last line over USB the cable still counts as in use; the same silence makes the
/// firmware call the link lost.
pub const USB_QUIET_MS: u32 = 15_000;
/// Longest line read from a connection: a sealed protocol line of 1024 bytes is 1388 characters.
const MAX_LINE_BYTES: usize = 2048;

/// A connection, numbered by the device layer.
pub type Conn = u32;

/// What the device layer must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Write this line (without its newline) to the connection, as it is.
    Send(Conn, String),
    /// Seal this line for the linked computer and write it to its connection ([`Output::Peer`]).
    SendPeer(String),
    Close(Conn),
    /// A line from the linked computer, to be handled as the VibeBuddy Protocol.
    Line(Vec<u8>),
    /// The box's diagnostic output should now go to this connection too, sealed with the sealer
    /// [`Lan::take_sealer`] hands over, or no longer to any.
    Peer(Option<Conn>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Sent `link.noise` at this time; the ephemeral key waits for the computer's first message.
    Waiting { ephemeral: [u8; 32], since: u32 },
    Linked,
}

struct Session {
    conn: Conn,
    state: State,
    /// Bytes of a line not yet ended.
    partial: Vec<u8>,
    /// Opens the computer's lines once linked.
    opener: Option<Opener>,
    /// Seals the box's lines until the device layer takes it.
    sealer: Option<Sealer>,
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

impl Lan {
    pub fn new() -> Self {
        Self::default()
    }

    /// The computer linked over the network right now.
    pub fn linked_key(&self) -> Option<[u8; 32]> {
        self.owner.filter(|owner| owner.gone_since.is_none() && owner.conn.is_some()).map(|owner| owner.key)
    }

    /// A computer is linked through the Relay right now.
    pub fn relay_link_active(&self) -> bool {
        self.owner.is_some_and(|owner| owner.conn.is_none() && owner.gone_since.is_none())
    }

    /// A computer talked over the cable in the last [`USB_QUIET_MS`].
    pub fn cable_in_use(&self, now: u32) -> bool {
        self.usb_at.is_some_and(|at| now.wrapping_sub(at) < USB_QUIET_MS)
    }

    /// A computer linked through the Relay, or that link ended. The Relay decides that link (ADR-0012); this
    /// only keeps the local network from taking the box from it without a takeover, and lets the same
    /// computer move to the local network.
    pub fn relay_linked(&mut self, key: Option<[u8; 32]>, now: u32) {
        match key {
            Some(key) => self.owner = Some(Owner { key, conn: None, gone_since: None }),
            None => {
                if let Some(owner) = &mut self.owner
                    && owner.conn.is_none()
                    && owner.gone_since.is_none()
                {
                    owner.gone_since = Some(now);
                }
            }
        }
    }

    /// A new connection. `ephemeral` must be fresh randomness: the box's ephemeral key for this session.
    pub fn opened(&mut self, link: &Link, conn: Conn, ephemeral: [u8; 32], now: u32) -> Vec<Output> {
        self.sessions.push(Session { conn, state: State::Waiting { ephemeral, since: now }, partial: Vec::new(), opener: None, sealer: None });
        alloc::vec![Output::Send(conn, noise::opener(&link.public_key()))]
    }

    /// The sealer for a connection that just became the peer ([`Output::Peer`]).
    pub fn take_sealer(&mut self, conn: Conn) -> Option<Sealer> {
        self.sessions.iter_mut().find(|session| session.conn == conn)?.sealer.take()
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
                State::Linked => match session.opener.as_mut().and_then(|opener| opener.open(&line)) {
                    Some(line) => out.push(Output::Line(line)),
                    // Tampered, replayed or lost: the session is over, and the computer starts a new one.
                    None => {
                        out.extend(self.drop_conn(conn, now));
                        out.push(Output::Close(conn));
                        break;
                    }
                },
                State::Waiting { ephemeral, .. } => out.extend(self.handshake(link, conn, ephemeral, &line, now)),
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
            out.push(Output::SendPeer(unlinked("usb")));
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
                    out.push(Output::SendPeer(unlinked("unpaired")));
                    out.push(Output::Close(conn));
                    out.extend(self.drop_conn(conn, now));
                }
                self.owner = None;
                out
            }
            _ => Vec::new(),
        }
    }

    fn handshake(&mut self, link: &Link, conn: Conn, ephemeral: [u8; 32], line: &[u8], now: u32) -> Vec<Output> {
        let refuse = |code: &str| alloc::vec![Output::Send(conn, error(code)), Output::Close(conn)];
        let accepted = match noise::accept(link, ephemeral, line) {
            Ok(accepted) => accepted,
            Err(Refused::BadMessage) => return self.refused(conn, refuse("bad_message")),
            Err(Refused::Auth) => return self.refused(conn, refuse("auth")),
        };
        let key = accepted.computer;
        let takeover = serde_json::from_slice::<Value>(&accepted.payload).is_ok_and(|payload| payload.get("takeover") == Some(&Value::Bool(true)));
        let mut sealer = accepted.sealer;
        let mut out = alloc::vec![Output::Send(conn, accepted.reply)];
        let usb_in_use = self.usb_at.is_some_and(|at| now.wrapping_sub(at) < USB_QUIET_MS);
        let free = match self.owner {
            None => true,
            Some(owner) => owner.key == key || owner.gone_since.is_some_and(|since| now.wrapping_sub(since) >= GRACE_MS),
        };
        if usb_in_use || !(free || takeover) {
            let (reason, owner) = match self.owner {
                _ if usb_in_use => ("usb", ""),
                Some(owner) => ("linked", link.paired.iter().find(|paired| paired.key == owner.key).map_or("", |paired| paired.name.as_str())),
                None => ("linked", ""),
            };
            let busy = serde_json::to_string(&serde_json::json!({"version": 1, "event": "link.busy", "reason": reason, "owner": owner})).unwrap_or_default();
            out.extend(sealer.seal(busy.as_bytes()).map(|line| Output::Send(conn, line)));
            out.push(Output::Close(conn));
            return self.refused(conn, out);
        }

        if let Some(previous) = self.owner.and_then(|owner| owner.conn).filter(|&previous| previous != conn) {
            out.push(Output::SendPeer(unlinked("takeover")));
            out.push(Output::Close(previous));
            self.sessions.retain(|session| session.conn != previous);
        }
        self.owner = Some(Owner { key, conn: Some(conn), gone_since: None });
        out.extend(sealer.seal(br#"{"version":1,"event":"link.welcome"}"#).map(|line| Output::Send(conn, line)));
        if let Some(session) = self.sessions.iter_mut().find(|session| session.conn == conn) {
            session.state = State::Linked;
            session.opener = Some(accepted.opener);
            session.sealer = Some(sealer);
        }
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
    use crate::link;
    use crate::noise::Initiator;
    use ed25519_dalek::SigningKey;
    use std::string::ToString;

    struct Computer {
        seed: [u8; 32],
    }

    impl Computer {
        fn new(seed: u8) -> Self {
            Self { seed: [seed; 32] }
        }

        fn public(&self) -> [u8; 32] {
            SigningKey::from_bytes(&self.seed).verifying_key().to_bytes()
        }

        /// Answers the box's `link.noise` with the first handshake message.
        fn hello(&self, opener: &str, takeover: bool) -> (Initiator, Vec<u8>) {
            let value: Value = serde_json::from_str(opener).unwrap();
            assert_eq!(value["event"], "link.noise");
            let box_key = link::decode::<32>(value["box"].as_str().unwrap()).unwrap();
            let payload = alloc::format!(r#"{{"takeover":{takeover}}}"#);
            let (initiator, line) = Initiator::start(self.seed, &box_key, [5; 32], payload.as_bytes()).unwrap();
            (initiator, alloc::format!("{line}\n").into_bytes())
        }
    }

    /// What one connection was sent: the handshake reply opens the session, the rest is opened with it.
    struct Received {
        lines: Vec<String>,
        session: Option<(Sealer, Opener)>,
    }

    fn sent(out: &[Output], conn: Conn) -> Vec<String> {
        out.iter()
            .filter_map(|output| match output {
                Output::Send(to, line) if *to == conn => Some(line.clone()),
                _ => None,
            })
            .collect()
    }

    fn sent_to_peer(out: &[Output]) -> Vec<String> {
        out.iter()
            .filter_map(|output| match output {
                Output::SendPeer(line) => Some(line.clone()),
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

    /// Opens a connection and says hello; returns what came back, the sealed lines opened.
    fn connect(lan: &mut Lan, link: &Link, conn: Conn, computer: &Computer, takeover: bool, now: u32) -> (Vec<Output>, Received) {
        let opened = lan.opened(link, conn, [conn as u8; 32], now);
        let Output::Send(_, line) = &opened[0] else { panic!() };
        let (initiator, hello) = computer.hello(line, takeover);
        let out = lan.received(link, conn, &hello, now);
        let mut lines = sent(&out, conn).into_iter();
        let first = lines.next().unwrap();
        let Some((sealer, mut opener)) = initiator.finish(first.as_bytes()) else {
            return (out, Received { lines: std::vec![first], session: None });
        };
        let lines = lines.map(|line| String::from_utf8(opener.open(line.as_bytes()).unwrap()).unwrap()).collect();
        (out, Received { lines, session: Some((sealer, opener)) })
    }

    fn first_event(received: &Received) -> String {
        event(&received.lines[0])
    }

    #[test]
    fn a_paired_computer_links_in_a_sealed_session() {
        let a = Computer::new(2);
        let (mut lan, link) = setup(&[&a]);
        let (out, received) = connect(&mut lan, &link, 1, &a, false, 0);
        assert_eq!(received.lines, [r#"{"version":1,"event":"link.welcome"}"#]);
        assert!(out.contains(&Output::Peer(Some(1))));
        assert_eq!(lan.linked_key(), Some(a.public()));
        assert!(lan.take_sealer(1).is_some(), "the device layer gets the box's half");

        let (mut sealer, _) = received.session.unwrap();
        let line = sealer.seal(br#"{"version":1,"event":"device.hello"}"#).unwrap();
        let out = lan.received(&link, 1, alloc::format!("{line}\n").as_bytes(), 10);
        assert_eq!(out, [Output::Line(br#"{"version":1,"event":"device.hello"}"#.to_vec())]);
    }

    #[test]
    fn a_line_in_the_clear_after_the_handshake_ends_the_session() {
        let a = Computer::new(2);
        let (mut lan, link) = setup(&[&a]);
        connect(&mut lan, &link, 1, &a, false, 0);
        let out = lan.received(&link, 1, b"{\"version\":1,\"event\":\"device.volume\",\"level\":20}\n", 10);
        assert!(!out.iter().any(|output| matches!(output, Output::Line(_))));
        assert!(out.contains(&Output::Close(1)));
        assert_eq!(lan.linked_key(), None);
    }

    #[test]
    fn an_unpaired_computer_or_a_line_in_the_clear_is_turned_away() {
        let a = Computer::new(2);
        let stranger = Computer::new(3);
        let (mut lan, link) = setup(&[&a]);
        let (out, received) = connect(&mut lan, &link, 1, &stranger, false, 0);
        assert_eq!(received.lines, [error("auth")]);
        assert!(out.contains(&Output::Close(1)));

        lan.opened(&link, 2, [2; 32], 0);
        let out = lan.received(&link, 2, b"{\"version\":1,\"event\":\"device.volume\",\"level\":20}\n", 0);
        assert_eq!(sent(&out, 2), [error("bad_message")]);
        assert!(!out.iter().any(|output| matches!(output, Output::Line(_))));
        assert_eq!(lan.linked_key(), None);
    }

    #[test]
    fn a_second_computer_is_busy_until_grace_or_takeover() {
        let a = Computer::new(2);
        let b = Computer::new(3);
        let (mut lan, link) = setup(&[&a, &b]);
        connect(&mut lan, &link, 1, &a, false, 0);
        let (_, busy) = connect(&mut lan, &link, 2, &b, false, 1000);
        assert_eq!(first_event(&busy), "link.busy");
        assert!(busy.lines[0].contains(r#""owner":"a""#), "names who has the box");

        let (out, _) = connect(&mut lan, &link, 3, &b, true, 2000);
        assert_eq!(sent_to_peer(&out), [unlinked("takeover")], "sealed for a, which is still the peer");
        assert!(out.contains(&Output::Close(1)));
        assert_eq!(lan.linked_key(), Some(b.public()));

        // b goes away; a may come back only after the grace period, unless it takes over.
        lan.closed(3, 5000);
        assert_eq!(first_event(&connect(&mut lan, &link, 4, &a, false, 5000 + GRACE_MS - 1).1), "link.busy");
        assert_eq!(first_event(&connect(&mut lan, &link, 5, &a, false, 5000 + GRACE_MS).1), "link.welcome");
    }

    #[test]
    fn the_last_computer_comes_back_at_once() {
        let a = Computer::new(2);
        let (mut lan, link) = setup(&[&a]);
        connect(&mut lan, &link, 1, &a, false, 0);
        assert_eq!(lan.closed(1, 100), [Output::Peer(None)]);
        assert_eq!(first_event(&connect(&mut lan, &link, 2, &a, false, 200).1), "link.welcome");
    }

    #[test]
    fn usb_wins() {
        let a = Computer::new(2);
        let (mut lan, link) = setup(&[&a]);
        connect(&mut lan, &link, 1, &a, false, 0);
        let out = lan.usb_line(100);
        assert_eq!(sent_to_peer(&out), [unlinked("usb")]);
        assert!(out.contains(&Output::Peer(None)));
        let busy = &connect(&mut lan, &link, 2, &a, true, 200).1.lines[0];
        assert!(busy.contains(r#""reason":"usb""#), "not even a takeover while the cable talks: {busy}");
        assert_eq!(first_event(&connect(&mut lan, &link, 3, &a, false, 100 + USB_QUIET_MS).1), "link.welcome");
    }

    #[test]
    fn a_relay_link_holds_off_other_computers_but_not_its_own() {
        let a = Computer::new(2);
        let b = Computer::new(3);
        let (mut lan, link) = setup(&[&a, &b]);
        lan.relay_linked(Some(a.public()), 0);
        assert_eq!(first_event(&connect(&mut lan, &link, 1, &b, false, 10).1), "link.busy");
        assert_eq!(first_event(&connect(&mut lan, &link, 2, &a, false, 20).1), "link.welcome", "the same computer moves to the network");
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
        let (_, received) = connect(&mut lan, &link, 1, &a, false, 0);
        let (mut sealer, _) = received.session.unwrap();
        let x = sealer.seal(b"x").unwrap();
        let y = sealer.seal(b"y").unwrap();
        let bytes = alloc::format!("{x}\r\n{y}\n").into_bytes();
        let (head, tail) = bytes.split_at(5);
        assert!(lan.received(&link, 1, head, 0).is_empty());
        assert_eq!(lan.received(&link, 1, tail, 0), [Output::Line(b"x".to_vec()), Output::Line(b"y".to_vec())]);
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
        assert_eq!(sent_to_peer(&out), [unlinked("unpaired")]);
        assert_eq!(lan.linked_key(), None);
    }
}
