//! The VibePlay as the server on the local network (docs/vibeplay.md): what it does with each connection QingOS
//! hands it. It touches no socket and reads no clock: the app passes in the connection events, the time in
//! milliseconds and the randomness, and carries out the [`Out`] it gets back.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::identity::{Computer, Identity};
use crate::noise::{Responder, Session};

/// A VibePlay keeps up to this many paired computers.
pub const MAX_COMPUTERS: usize = 4;
/// Wrong codes before the VibePlay makes a new one.
pub const CODE_TRIES: u8 = 3;
/// How long one pairing code lasts.
pub const CODE_LIFETIME_MS: u64 = 10 * 60 * 1000;
/// A linked computer that has sent nothing this long is gone. It sends a heartbeat every 5 s.
pub const SILENCE_MS: u64 = 15 * 1000;
/// A connection that hasn't finished its handshake or its pairing in this long is closed.
pub const HANDSHAKE_MS: u64 = 10 * 1000;

/// Fills a buffer from the system's random source.
pub type Random = Box<dyn FnMut(&mut [u8]) + Send>;

/// What the app must do.
#[derive(Debug, PartialEq)]
pub enum Out {
    /// Send this line on this connection.
    Send(u64, String),
    /// Close this connection.
    Close(u64),
    /// The paired computers changed: keep them, and answer mDNS again (`paired`).
    Computers,
    /// A paired computer linked, by its name.
    Linked(String),
    /// The linked computer went away.
    Unlinked,
    /// A line from the linked computer.
    Event(Value),
}

/// The pairing code on the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Code {
    pub digits: String,
    until: u64,
    wrong: u8,
}

enum Conn {
    /// `link.noise` is sent; the first line from the computer comes next.
    Opened,
    /// `vibeplay.pair_start` came; the first XX message comes next.
    PairStarted,
    /// The second XX message is sent; the third comes next.
    Pairing(Box<Responder>),
    /// XX is done; the sealed `vibeplay.pair` comes next.
    PairSession { computer: [u8; 32], session: Session },
    /// IK is done and this is the linked computer.
    Linked { computer: [u8; 32], session: Session },
}

struct Connection {
    conn: Conn,
    /// When it opened, or when the linked computer last sent a line.
    heard: u64,
}

pub struct Server {
    identity: Identity,
    build: String,
    computers: Vec<Computer>,
    conns: BTreeMap<u64, Connection>,
    linked: Option<u64>,
    code: Option<Code>,
    random: Random,
}

fn line(event: &str, fields: Value) -> String {
    let mut value = json!({"version": 1, "event": event});
    if let (Some(map), Value::Object(fields)) = (value.as_object_mut(), fields) {
        map.extend(fields);
    }
    value.to_string()
}

impl Server {
    /// `random` fills a buffer from the system's random source. A VibePlay with no computers shows a code at once.
    pub fn new(identity: Identity, computers: Vec<Computer>, build: &str, random: Random, now: u64) -> Self {
        let mut server = Self { identity, build: build.to_owned(), computers, conns: BTreeMap::new(), linked: None, code: None, random };
        if server.computers.is_empty() {
            server.new_code(now);
        }
        server
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn computers(&self) -> &[Computer] {
        &self.computers
    }

    /// The code to show, if the VibePlay is waiting to be paired.
    pub fn code(&self) -> Option<&str> {
        self.code.as_ref().map(|code| code.digits.as_str())
    }

    /// The linked computer's name.
    pub fn linked(&self) -> Option<&str> {
        let Conn::Linked { computer, .. } = &self.conns.get(&self.linked?)?.conn else { return None };
        self.computers.iter().find(|c| c.key == *computer).map(|c| c.name.as_str())
    }

    fn new_code(&mut self, now: u64) {
        let mut bytes = [0u8; 8];
        (self.random)(&mut bytes);
        let digits = format!("{:06}", u64::from_le_bytes(bytes) % 1_000_000);
        self.code = Some(Code { digits, until: now + CODE_LIFETIME_MS, wrong: 0 });
    }

    fn ephemeral(&mut self) -> [u8; 32] {
        let mut key = [0u8; 32];
        (self.random)(&mut key);
        key
    }

    /// Pair a computer, from the menu: show a code.
    pub fn show_code(&mut self, now: u64) {
        if self.code.is_none() {
            self.new_code(now);
        }
    }

    /// Leave the pairing screen. A VibePlay with no computers keeps its code.
    pub fn hide_code(&mut self) {
        if !self.computers.is_empty() {
            self.code = None;
        }
    }

    /// Forget computers, from the menu: forget them all and close every connection.
    pub fn forget_all(&mut self, now: u64) -> Vec<Out> {
        let mut out: Vec<Out> = self.conns.keys().map(|&id| Out::Close(id)).collect();
        if self.linked.take().is_some() {
            out.push(Out::Unlinked);
        }
        self.conns.clear();
        self.computers.clear();
        self.new_code(now);
        out.push(Out::Computers);
        out
    }

    /// A computer connected.
    pub fn open(&mut self, id: u64, now: u64) -> Vec<Out> {
        self.conns.insert(id, Connection { conn: Conn::Opened, heard: now });
        vec![Out::Send(id, line("link.noise", json!({"vibeplay": self.identity.key()})))]
    }

    /// A connection closed, from either end.
    pub fn closed(&mut self, id: u64) -> Vec<Out> {
        self.conns.remove(&id);
        if self.linked == Some(id) {
            self.linked = None;
            return vec![Out::Unlinked];
        }
        Vec::new()
    }

    fn drop_conn(&mut self, id: u64, mut out: Vec<Out>) -> Vec<Out> {
        out.extend(self.closed(id));
        out.push(Out::Close(id));
        out
    }

    /// A line from a computer.
    pub fn line(&mut self, id: u64, text: &str, now: u64) -> Vec<Out> {
        let Some(connection) = self.conns.remove(&id) else { return Vec::new() };
        match connection.conn {
            Conn::Opened if text.starts_with('{') => {
                let start = serde_json::from_str::<Value>(text).ok().filter(|value| value["event"] == "vibeplay.pair_start");
                if start.is_none() {
                    return self.drop_conn(id, vec![Out::Send(id, line("link.error", json!({"code": "bad_message"})))]);
                }
                self.conns.insert(id, Connection { conn: Conn::PairStarted, heard: connection.heard });
                Vec::new()
            }
            Conn::Opened => self.link(id, text, now),
            Conn::PairStarted => {
                let mut responder = Responder::pair(self.identity.noise_secret(), self.ephemeral());
                let Some(reply) = responder.read(text).and_then(|()| responder.write()) else { return self.drop_conn(id, Vec::new()) };
                self.conns.insert(id, Connection { conn: Conn::Pairing(Box::new(responder)), heard: connection.heard });
                vec![Out::Send(id, reply)]
            }
            Conn::Pairing(mut responder) => {
                let session = responder.read(text).and_then(|()| Some((responder.remote()?, responder.finish()?)));
                let Some((computer, session)) = session else { return self.drop_conn(id, Vec::new()) };
                self.conns.insert(id, Connection { conn: Conn::PairSession { computer, session }, heard: connection.heard });
                Vec::new()
            }
            Conn::PairSession { computer, mut session } => {
                let request = session.open(text).and_then(|text| serde_json::from_str::<Value>(&text).ok());
                let Some(request) = request.filter(|request| request["event"] == "vibeplay.pair") else { return self.drop_conn(id, Vec::new()) };
                let mut out = Vec::new();
                let answer = self.pair(computer, &request, now, &mut out);
                out.extend(session.seal(&answer).map(|sealed| Out::Send(id, sealed)));
                self.drop_conn(id, out)
            }
            Conn::Linked { computer, mut session } => {
                let Some(event) = session.open(text).and_then(|text| serde_json::from_str::<Value>(&text).ok()) else {
                    return self.drop_conn(id, Vec::new());
                };
                if event["event"] == "vibeplay.unpair" {
                    self.computers.retain(|c| c.key != computer);
                    if self.computers.is_empty() {
                        self.new_code(now);
                    }
                    return self.drop_conn(id, vec![Out::Computers]);
                }
                self.conns.insert(id, Connection { conn: Conn::Linked { computer, session }, heard: now });
                vec![Out::Event(event)]
            }
        }
    }

    /// Checks the code in `vibeplay.pair` and returns the answer to seal.
    fn pair(&mut self, computer: [u8; 32], request: &Value, now: u64, out: &mut Vec<Out>) -> String {
        self.expire(now);
        let known = self.computers.iter().any(|c| c.key == computer);
        let Some(code) = self.code.as_mut() else { return line("vibeplay.pair_error", json!({"reason": "closed"})) };
        if request["code"].as_str() != Some(code.digits.as_str()) {
            code.wrong += 1;
            if code.wrong >= CODE_TRIES {
                self.new_code(now);
            }
            return line("vibeplay.pair_error", json!({"reason": "code"}));
        }
        if !known && self.computers.len() >= MAX_COMPUTERS {
            return line("vibeplay.pair_error", json!({"reason": "full"}));
        }
        let name = request["name"].as_str().unwrap_or_default().chars().take(64).collect();
        self.computers.retain(|c| c.key != computer);
        self.computers.push(Computer { key: computer, name });
        self.code = None;
        out.push(Out::Computers);
        line("vibeplay.paired", json!({}))
    }

    /// The first IK message: a paired computer links, unless another one has the VibePlay.
    fn link(&mut self, id: u64, text: &str, now: u64) -> Vec<Out> {
        let mut responder = Responder::link(self.identity.noise_secret(), self.ephemeral());
        let paired = responder.read(text).and_then(|()| responder.remote()).filter(|remote| self.computers.iter().any(|c| c.key == *remote));
        let Some(computer) = paired else {
            return self.drop_conn(id, vec![Out::Send(id, line("link.error", json!({"code": "auth"})))]);
        };
        let Some((reply, mut session)) = responder.write().and_then(|reply| Some((reply, responder.finish()?))) else {
            return self.drop_conn(id, Vec::new());
        };
        let mut out = vec![Out::Send(id, reply)];
        if let Some(owner) = self.linked() {
            let busy = line("link.busy", json!({"reason": "linked", "owner": owner}));
            out.extend(session.seal(&busy).map(|sealed| Out::Send(id, sealed)));
            return self.drop_conn(id, out);
        }
        let hello = line("vibeplay.hello", json!({"build": self.build, "name": self.identity.name(), "screen": [720, 480]}));
        out.extend(session.seal(&hello).map(|sealed| Out::Send(id, sealed)));
        self.conns.insert(id, Connection { conn: Conn::Linked { computer, session }, heard: now });
        self.linked = Some(id);
        let name = self.linked().unwrap_or_default().to_owned();
        out.push(Out::Linked(name));
        out
    }

    /// A line for the linked computer: `button` or `focus`. Nothing happens when none is linked.
    pub fn send(&mut self, text: &str) -> Vec<Out> {
        let Some(id) = self.linked else { return Vec::new() };
        let Some(Connection { conn: Conn::Linked { session, .. }, .. }) = self.conns.get_mut(&id) else { return Vec::new() };
        match session.seal(text) {
            Some(sealed) => vec![Out::Send(id, sealed)],
            None => self.drop_conn(id, Vec::new()),
        }
    }

    fn expire(&mut self, now: u64) {
        if self.code.as_ref().is_some_and(|code| now >= code.until) {
            self.new_code(now);
        }
    }

    /// Once a frame or so: a new code when the old one runs out, and connections that went quiet are closed.
    pub fn tick(&mut self, now: u64) -> Vec<Out> {
        self.expire(now);
        let stale: Vec<u64> = self
            .conns
            .iter()
            .filter(|(_, c)| {
                let limit = if matches!(c.conn, Conn::Linked { .. }) { SILENCE_MS } else { HANDSHAKE_MS };
                now.saturating_sub(c.heard) >= limit
            })
            .map(|(&id, _)| id)
            .collect();
        stale.into_iter().fold(Vec::new(), |out, id| self.drop_conn(id, out))
    }
}

#[cfg(test)]
mod tests;
