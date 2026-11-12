//! The computer's side here is snow, the daemon's Noise library, set up as `daemon/src/noise_link.rs` sets it up.

use ed25519_dalek::SigningKey;
use serde_json::Value;

use super::*;
use crate::noise;

const DESK: [u8; 32] = [11; 32];
const LAPTOP: [u8; 32] = [22; 32];

fn x25519(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_montgomery().to_bytes()
}

/// A random source that is the same on every run.
fn random() -> Random {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    Box::new(move |bytes| {
        for byte in bytes {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = state as u8;
        }
    })
}

fn vibeplay(computers: Vec<Computer>) -> Server {
    Server::new(Identity::from_seed([5; 32]), computers, "test", random(), 0)
}

fn paired(seed: &[u8; 32], name: &str) -> Computer {
    Computer { key: x25519(seed), name: name.into() }
}

fn sent(out: &[Out]) -> Vec<String> {
    out.iter().filter_map(|o| if let Out::Send(_, line) = o { Some(line.clone()) } else { None }).collect()
}

fn closes(out: &[Out], id: u64) -> bool {
    out.contains(&Out::Close(id))
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn vibeplay_public(server: &Server) -> [u8; 32] {
    let key = noise::decode_key(&server.identity().key()).unwrap();
    ed25519_dalek::VerifyingKey::from_bytes(&key).unwrap().to_montgomery().to_bytes()
}

fn builder(prologue: &'static [u8], pattern: &str) -> snow::Builder<'static> {
    snow::Builder::new(format!("Noise_{pattern}_25519_ChaChaPoly_SHA256").parse().unwrap()).prologue(prologue).unwrap()
}

fn write(handshake: &mut snow::HandshakeState) -> String {
    let mut buffer = vec![0u8; 512];
    let length = handshake.write_message(&[], &mut buffer).unwrap();
    noise::encode(&buffer[..length])
}

fn read(handshake: &mut snow::HandshakeState, line: &str) {
    let mut buffer = vec![0u8; 512];
    handshake.read_message(&noise::decode(line).unwrap(), &mut buffer).unwrap();
}

fn seal(session: &mut snow::TransportState, text: &str) -> String {
    let mut buffer = vec![0u8; text.len() + 16];
    let length = session.write_message(text.as_bytes(), &mut buffer).unwrap();
    noise::encode(&buffer[..length])
}

fn open(session: &mut snow::TransportState, line: &str) -> Value {
    let sealed = noise::decode(line).unwrap();
    let mut buffer = vec![0u8; sealed.len()];
    let length = session.read_message(&sealed, &mut buffer).unwrap();
    serde_json::from_slice(&buffer[..length]).unwrap()
}

/// Pairs as the daemon's `pair_at` does and returns the VibePlay's answer and everything it did.
fn pair(server: &mut Server, id: u64, seed: &[u8; 32], code: &str) -> (Value, Vec<Out>) {
    let opener = json(&sent(&server.open(id, 0))[0]);
    assert_eq!(opener["event"], "link.noise");
    assert!(server.line(id, r#"{"version":1,"event":"vibeplay.pair_start"}"#, 0).is_empty());
    let secret = SigningKey::from_bytes(seed).to_scalar_bytes();
    let mut handshake = builder(noise::PAIR_PROLOGUE, "XX").local_private_key(&secret).unwrap().build_initiator().unwrap();
    let reply = sent(&server.line(id, &write(&mut handshake), 0)).remove(0);
    read(&mut handshake, &reply);
    assert_eq!(handshake.get_remote_static(), Some(vibeplay_public(server).as_slice()), "XX shows the key in link.noise");
    assert!(server.line(id, &write(&mut handshake), 0).is_empty());
    let mut session = handshake.into_transport_mode().unwrap();
    let request = format!(r#"{{"version":1,"event":"vibeplay.pair","code":"{code}","name":"Desk Mac"}}"#);
    let out = server.line(id, &seal(&mut session, &request), 0);
    assert!(closes(&out, id), "it closes after answering");
    (open(&mut session, &sent(&out)[0]), out)
}

/// Links as the daemon's `connect` does: returns the session and the VibePlay's lines after the handshake.
fn link(server: &mut Server, id: u64, seed: &[u8; 32], now: u64) -> (Option<snow::TransportState>, Vec<Out>) {
    sent(&server.open(id, now));
    let secret = SigningKey::from_bytes(seed).to_scalar_bytes();
    let mut handshake =
        builder(noise::LINK_PROLOGUE, "IK").local_private_key(&secret).unwrap().remote_public_key(&vibeplay_public(server)).unwrap().build_initiator().unwrap();
    let out = server.line(id, &write(&mut handshake), now);
    let lines = sent(&out);
    if lines[0].starts_with('{') {
        return (None, out);
    }
    read(&mut handshake, &lines[0]);
    (Some(handshake.into_transport_mode().unwrap()), out)
}

#[test]
fn every_connection_opens_with_the_vibeplay_key() {
    let mut server = vibeplay(Vec::new());
    let opener = json(&sent(&server.open(1, 0))[0]);
    assert_eq!(opener, serde_json::json!({"version": 1, "event": "link.noise", "vibeplay": server.identity().key()}));
}

#[test]
fn the_right_code_pairs_and_keeps_the_key_the_link_shows() {
    let mut server = vibeplay(Vec::new());
    let code = server.code().expect("a VibePlay with no computers shows a code").to_owned();
    assert_eq!(code.len(), 6);
    let (answer, out) = pair(&mut server, 1, &DESK, &code);
    assert_eq!(answer, serde_json::json!({"version": 1, "event": "vibeplay.paired"}));
    assert!(out.contains(&Out::Computers));
    assert_eq!(server.computers(), &[paired(&DESK, "Desk Mac")]);
    assert_eq!(server.code(), None, "the code is used up");

    let (session, out) = link(&mut server, 2, &DESK, 0);
    let mut session = session.expect("the paired computer links");
    let hello = open(&mut session, &sent(&out)[1]);
    assert_eq!(hello["event"], "vibeplay.hello");
    assert_eq!(hello["name"], server.identity().name());
    assert_eq!(hello["screen"], serde_json::json!([720, 480]));
    assert!(out.contains(&Out::Linked("Desk Mac".into())));
}

#[test]
fn three_wrong_codes_make_a_new_one() {
    let mut server = vibeplay(Vec::new());
    let first = server.code().unwrap().to_owned();
    let wrong = if first == "000000" { "111111" } else { "000000" };
    for id in 1..=2 {
        assert_eq!(pair(&mut server, id, &DESK, wrong).0["reason"], "code");
        assert_eq!(server.code(), Some(first.as_str()));
    }
    assert_eq!(pair(&mut server, 3, &DESK, wrong).0["reason"], "code");
    assert_ne!(server.code(), Some(first.as_str()), "a new code after the third wrong one");
    assert_eq!(pair(&mut server, 4, &DESK, &first).0["reason"], "code", "the old code is gone");
    assert!(server.computers().is_empty());
}

#[test]
fn a_code_lasts_ten_minutes() {
    let mut server = vibeplay(Vec::new());
    let first = server.code().unwrap().to_owned();
    server.tick(CODE_LIFETIME_MS - 1);
    assert_eq!(server.code(), Some(first.as_str()));
    server.tick(CODE_LIFETIME_MS);
    assert_ne!(server.code(), Some(first.as_str()));
}

#[test]
fn with_no_code_on_the_screen_pairing_is_closed() {
    let mut server = vibeplay(vec![paired(&LAPTOP, "Laptop")]);
    assert_eq!(server.code(), None, "a VibePlay with computers shows no code until asked");
    assert_eq!(pair(&mut server, 1, &DESK, "123456").0["reason"], "closed");
    server.show_code(0);
    let code = server.code().unwrap().to_owned();
    assert_eq!(pair(&mut server, 2, &DESK, &code).0["event"], "vibeplay.paired", "a second computer pairs from the menu");
    assert_eq!(server.computers().len(), 2);
    server.show_code(0);
    server.hide_code();
    assert_eq!(server.code(), None);
}

#[test]
fn four_computers_is_full_but_one_of_them_can_pair_again() {
    let others: Vec<Computer> = (0..4u8).map(|n| paired(&[100 + n; 32], "Other")).collect();
    let mut server = vibeplay(others);
    server.show_code(0);
    let code = server.code().unwrap().to_owned();
    assert_eq!(pair(&mut server, 1, &DESK, &code).0["reason"], "full");
    assert_eq!(pair(&mut server, 2, &[100; 32], &code).0["event"], "vibeplay.paired");
    assert_eq!(server.computers().len(), 4);
    assert_eq!(server.computers()[3].name, "Desk Mac", "the name is the new one");
}

#[test]
fn a_computer_that_isnt_paired_gets_auth_in_the_clear() {
    let mut server = vibeplay(vec![paired(&LAPTOP, "Laptop")]);
    let (session, out) = link(&mut server, 1, &DESK, 0);
    assert!(session.is_none());
    assert_eq!(json(&sent(&out)[0]), serde_json::json!({"version": 1, "event": "link.error", "code": "auth"}));
    assert!(closes(&out, 1));
}

#[test]
fn another_first_line_in_the_clear_is_a_bad_message() {
    let mut server = vibeplay(Vec::new());
    server.open(1, 0);
    let out = server.line(1, r#"{"version":1,"event":"hello"}"#, 0);
    assert_eq!(json(&sent(&out)[0])["code"], "bad_message");
    assert!(closes(&out, 1));
    server.open(2, 0);
    assert!(closes(&server.line(2, "not a handshake", 0), 2));
}

#[test]
fn the_link_carries_events_both_ways() {
    let mut server = vibeplay(vec![paired(&DESK, "Desk Mac")]);
    let (session, out) = link(&mut server, 1, &DESK, 0);
    let mut session = session.unwrap();
    assert_eq!(open(&mut session, &sent(&out)[1])["event"], "vibeplay.hello");
    assert_eq!(server.linked(), Some("Desk Mac"));
    let event = r#"{"version":1,"event":"agent.idle","tasks":[],"quiet":true}"#;
    assert_eq!(server.line(1, &seal(&mut session, event), 1000), vec![Out::Event(json(event))]);
    let out = server.send(r#"{"version":1,"event":"button","button":"K2","action":"press"}"#);
    assert_eq!(open(&mut session, &sent(&out)[0])["button"], "K2");
}

#[test]
fn a_second_computer_is_busy() {
    let mut server = vibeplay(vec![paired(&DESK, "Desk Mac"), paired(&LAPTOP, "Laptop")]);
    link(&mut server, 1, &DESK, 0).0.unwrap();
    let (session, out) = link(&mut server, 2, &LAPTOP, 0);
    let busy = open(&mut session.unwrap(), &sent(&out)[1]);
    assert_eq!(busy, serde_json::json!({"version": 1, "event": "link.busy", "reason": "linked", "owner": "Desk Mac"}));
    assert!(closes(&out, 2));
    assert_eq!(server.linked(), Some("Desk Mac"), "the first one keeps it");
}

#[test]
fn a_computer_silent_for_fifteen_seconds_is_gone() {
    let mut server = vibeplay(vec![paired(&DESK, "Desk Mac")]);
    let mut session = link(&mut server, 1, &DESK, 0).0.unwrap();
    server.line(1, &seal(&mut session, r#"{"version":1,"event":"device.heartbeat"}"#), 5000);
    assert!(server.tick(5000 + SILENCE_MS - 1).is_empty());
    let out = server.tick(5000 + SILENCE_MS);
    assert!(closes(&out, 1) && out.contains(&Out::Unlinked));
    assert_eq!(server.linked(), None);
    assert!(server.closed(1).is_empty(), "the close QingOS reports after that changes nothing");
}

#[test]
fn a_handshake_that_stalls_is_closed() {
    let mut server = vibeplay(Vec::new());
    server.open(1, 0);
    server.line(1, r#"{"version":1,"event":"vibeplay.pair_start"}"#, 0);
    assert!(server.tick(HANDSHAKE_MS - 1).is_empty());
    assert_eq!(server.tick(HANDSHAKE_MS), vec![Out::Close(1)]);
}

#[test]
fn unpair_forgets_that_computer() {
    let mut server = vibeplay(vec![paired(&DESK, "Desk Mac")]);
    let mut session = link(&mut server, 1, &DESK, 0).0.unwrap();
    let out = server.line(1, &seal(&mut session, r#"{"version":1,"event":"vibeplay.unpair"}"#), 0);
    assert!(out.contains(&Out::Computers) && out.contains(&Out::Unlinked) && closes(&out, 1));
    assert!(server.computers().is_empty());
    assert!(server.code().is_some(), "with no computer left it shows a code");
}

#[test]
fn forget_computers_closes_everything() {
    let mut server = vibeplay(vec![paired(&DESK, "Desk Mac")]);
    link(&mut server, 1, &DESK, 0).0.unwrap();
    server.open(2, 0);
    let out = server.forget_all(0);
    assert!(closes(&out, 1) && closes(&out, 2) && out.contains(&Out::Unlinked) && out.contains(&Out::Computers));
    assert!(server.computers().is_empty() && server.code().is_some());
    assert!(server.send("{}").is_empty());
}

#[test]
fn a_tampered_line_ends_the_link() {
    let mut server = vibeplay(vec![paired(&DESK, "Desk Mac")]);
    link(&mut server, 1, &DESK, 0).0.unwrap();
    let out = server.line(1, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", 0);
    assert!(closes(&out, 1) && out.contains(&Out::Unlinked));
}
