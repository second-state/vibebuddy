//! The app with QingOS's test doubles: connections in, lines, mDNS and data out. The computer's side is snow, as
//! in the daemon. `VIBEGAME_SHOTS=<folder> cargo test --test app shots` also saves every screen as a PNG.

use std::sync::Mutex;

use ed25519_dalek::SigningKey;
use qapp::full::{Frame, H, Lang, W, key, native};
use qapp::host;
use qapp::host::test::{conn_closed, conn_sent, mdns};
use serde_json::{Value, json};
use vibegame_app::app::{App, Page};
use vibegame_app::art::{self, Art};
use vibegame_app::screens;
use vibegame_core::noise;

/// QingOS's doubles are shared by the whole test binary: one test at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

const DESK: [u8; 32] = [11; 32];

fn fresh() -> App {
    for name in ["identity", "computers", "settings"] {
        let _ = host::data_delete(name);
    }
    conn_sent();
    conn_closed();
    App::new()
}

fn sent() -> Vec<String> {
    conn_sent().into_iter().map(|(_, line)| line).collect()
}

fn b64(line: &str) -> Vec<u8> {
    noise::decode(line).unwrap()
}

/// A computer as the daemon is one.
struct Computer {
    session: snow::TransportState,
}

impl Computer {
    fn secret(seed: &[u8; 32]) -> [u8; 32] {
        SigningKey::from_bytes(seed).to_scalar_bytes()
    }

    fn vibegame_public(app: &App) -> [u8; 32] {
        let key = noise::decode_key(&app.server.identity().key()).unwrap();
        ed25519_dalek::VerifyingKey::from_bytes(&key).unwrap().to_montgomery().to_bytes()
    }

    fn write(handshake: &mut snow::HandshakeState) -> String {
        let mut buffer = [0u8; 512];
        let n = handshake.write_message(&[], &mut buffer).unwrap();
        noise::encode(&buffer[..n])
    }

    fn pair(app: &mut App, id: u64, code: &str) -> Value {
        app.conn_open(id);
        assert_eq!(serde_json::from_str::<Value>(&sent()[0]).unwrap()["event"], "link.noise");
        app.conn_line(id, r#"{"version":1,"event":"vibegame.pair_start"}"#);
        let mut handshake = snow::Builder::new("Noise_XX_25519_ChaChaPoly_SHA256".parse().unwrap())
            .local_private_key(&Self::secret(&DESK))
            .unwrap()
            .prologue(noise::PAIR_PROLOGUE)
            .unwrap()
            .build_initiator()
            .unwrap();
        app.conn_line(id, &Self::write(&mut handshake));
        handshake.read_message(&b64(&sent()[0]), &mut [0u8; 512]).unwrap();
        app.conn_line(id, &Self::write(&mut handshake));
        let mut computer = Computer { session: handshake.into_transport_mode().unwrap() };
        let request = json!({"version": 1, "event": "vibegame.pair", "code": code, "name": "Desk Mac"}).to_string();
        let sealed = computer.seal(&request);
        app.conn_line(id, &sealed);
        assert_eq!(conn_closed(), vec![id]);
        computer.open(&sent()[0])
    }

    fn link(app: &mut App, id: u64) -> Computer {
        app.conn_open(id);
        sent();
        let mut handshake = snow::Builder::new("Noise_IK_25519_ChaChaPoly_SHA256".parse().unwrap())
            .local_private_key(&Self::secret(&DESK))
            .unwrap()
            .remote_public_key(&Self::vibegame_public(app))
            .unwrap()
            .prologue(noise::LINK_PROLOGUE)
            .unwrap()
            .build_initiator()
            .unwrap();
        app.conn_line(id, &Self::write(&mut handshake));
        let lines = sent();
        handshake.read_message(&b64(&lines[0]), &mut [0u8; 512]).unwrap();
        let mut computer = Computer { session: handshake.into_transport_mode().unwrap() };
        assert_eq!(computer.open(&lines[1])["event"], "vibegame.hello");
        computer
    }

    fn seal(&mut self, text: &str) -> String {
        let mut buffer = vec![0u8; text.len() + 16];
        let n = self.session.write_message(text.as_bytes(), &mut buffer).unwrap();
        noise::encode(&buffer[..n])
    }

    fn open(&mut self, line: &str) -> Value {
        let sealed = b64(line);
        let mut buffer = vec![0u8; sealed.len()];
        let n = self.session.read_message(&sealed, &mut buffer).unwrap();
        serde_json::from_slice(&buffer[..n]).unwrap()
    }

    fn send(&mut self, app: &mut App, id: u64, event: Value) {
        let sealed = self.seal(&event.to_string());
        app.conn_line(id, &sealed);
    }
}

fn press(app: &mut App, keys: u32) {
    app.step(keys);
    app.step(0);
}

fn paired_app() -> App {
    let mut app = fresh();
    let code = app.server.code().unwrap().to_owned();
    assert_eq!(Computer::pair(&mut app, 1, &code)["event"], "vibegame.paired");
    app
}

fn working() -> Value {
    json!({"version":1,"event":"task.start","tasks_total":4,"stats":["7 DONE","3 ASKS","4H 12M BUSY"],"quiet":false,
        "tasks":[{"id":"t1","agent":"claude-code","title":"CC:Add Chinese to the box","status":"working","project":"vibebuddy","elapsed_s":420},
                 {"id":"t2","agent":"codex","title":"CX:Fix voice reply playback","status":"working","project":"vibebuddy","elapsed_s":65},
                 {"id":"t3","agent":"github-actions","title":"CI:App tests","status":"working","project":"vibebuddy","elapsed_s":130}]})
}

#[test]
fn a_new_vibegame_makes_its_key_and_answers_mdns() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let app = fresh();
    let saved = host::data_read("identity").expect("the key is kept");
    assert_eq!(saved.len(), 32);
    let (instance, txt) = mdns().unwrap();
    let key = app.server.identity().key();
    assert_eq!(instance, format!("VibeGame {}", &key[..8]));
    assert_eq!(txt, vec![("key".into(), key.clone()), ("name".into(), instance.clone()), ("paired".into(), "0".into())]);
    assert!(app.server.code().is_some());
    assert_eq!(App::new().server.identity().key(), key, "the same key the next time");
}

#[test]
fn pairing_keeps_the_computer_and_says_so_in_mdns() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let app = paired_app();
    assert_eq!(app.server.code(), None);
    let saved: Value = serde_json::from_slice(&host::data_read("computers").unwrap()).unwrap();
    assert_eq!(saved[0]["name"], "Desk Mac");
    assert_eq!(mdns().unwrap().1[2], ("paired".into(), "1".into()));
    assert_eq!(App::new().server.computers().len(), 1, "kept for the next time");
}

#[test]
fn a_sends_k2_and_a_picked_card_sends_focus() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = paired_app();
    let mut computer = Computer::link(&mut app, 2);
    computer.send(&mut app, 2, working());
    press(&mut app, key::A);
    assert_eq!(computer.open(&sent()[0]), json!({"version":1,"event":"button","button":"K2","action":"press"}));
    press(&mut app, key::DOWN);
    press(&mut app, key::DOWN);
    assert_eq!(app.picked.map(|(at, _)| at), Some(1));
    press(&mut app, key::A);
    assert_eq!(computer.open(&sent()[0]), json!({"version":1,"event":"focus","task":"t2"}));
    assert_eq!(app.picked, None);
}

#[test]
fn fifteen_seconds_without_a_line_is_no_link() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = paired_app();
    let mut computer = Computer::link(&mut app, 2);
    computer.send(&mut app, 2, working());
    assert!(app.link_alive());
    for _ in 0..(14 * 60) {
        app.step(0);
    }
    assert!(app.link_alive());
    for _ in 0..(60 + 1) {
        app.step(0);
    }
    assert!(!app.link_alive());
    assert_eq!(conn_closed(), vec![2], "the computer is gone, so its connection is closed");
}

#[test]
fn the_menu_forgets_computers_after_asking_and_mutes() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = paired_app();
    press(&mut app, key::X);
    press(&mut app, key::DOWN);
    press(&mut app, key::DOWN);
    press(&mut app, key::A);
    assert!(app.muted);
    assert_eq!(host::data_read("settings").unwrap(), br#"{"muted":true}"#);
    press(&mut app, key::UP);
    press(&mut app, key::A);
    assert!(app.menu.unwrap().confirm, "it asks first");
    press(&mut app, key::B);
    assert!(!app.menu.unwrap().confirm);
    press(&mut app, key::A);
    press(&mut app, key::A);
    assert!(app.server.computers().is_empty() && app.server.code().is_some() && app.menu.is_none());
    assert_eq!(mdns().unwrap().1[2], ("paired".into(), "0".into()));
}

#[test]
fn start_runs_the_pomodoro_and_select_switches_pages() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = paired_app();
    press(&mut app, key::SELECT);
    assert_eq!(app.page, Page::Pomodoro);
    press(&mut app, key::SELECT);
    assert_eq!(app.page, Page::Duty);
    press(&mut app, key::START);
    assert_eq!((app.page, app.pomodoro.run), (Page::Pomodoro, vibegame_core::pomodoro::Run::Running));
    for _ in 0..50 {
        app.step(key::START);
    }
    app.step(0);
    assert_eq!(app.pomodoro.run, vibegame_core::pomodoro::Run::Pending, "held, it gives up");
}

fn save(frame: &Frame, path: &std::path::Path, scale: usize) {
    let mut rgb = Vec::with_capacity(W * H * 3 * scale * scale);
    for y in 0..H * scale {
        for x in 0..W * scale {
            rgb.extend_from_slice(&frame.palette[frame.pixels[(y / scale) * W + x / scale] as usize]);
        }
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut encoder = png::Encoder::new(file, (W * scale) as u32, (H * scale) as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&rgb).unwrap();
}

/// Every screen, in English and Chinese. Drawn always, so a screen that panics fails the test; saved only with
/// `VIBEGAME_SHOTS`.
#[test]
fn shots() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let folder = std::env::var_os("VIBEGAME_SHOTS").map(std::path::PathBuf::from);
    if let Some(folder) = &folder {
        std::fs::create_dir_all(folder).unwrap();
    }
    let art = Art::new();
    for (lang, suffix) in [(Lang::En, "en"), (Lang::Zh, "zh")] {
        native::set_lang(lang);
        let mut frame = Frame::default();
        let mut shoot = |app: &App, name: &str| {
            screens::draw(app, &art, &mut frame);
            if let Some(folder) = &folder {
                save(&frame, &folder.join(format!("{name}-{suffix}.png")), 3);
            }
        };
        let mut app = fresh();
        for _ in 0..20 {
            app.step(0);
        }
        shoot(&app, "1-not-paired");
        let mut app = paired_app();
        for _ in 0..40 {
            app.step(0);
        }
        shoot(&app, "2-looking");
        let mut computer = Computer::link(&mut app, 2);
        computer.send(&mut app, 2, working());
        for _ in 0..20 {
            app.step(0);
        }
        shoot(&app, "3-duty-working");
        press(&mut app, key::DOWN);
        press(&mut app, key::DOWN);
        shoot(&app, "4-duty-picked");
        press(&mut app, key::B);
        let mut waiting = working();
        waiting["event"] = json!("agent.input_required");
        waiting["tasks"][1]["status"] = json!("input_required");
        waiting["tasks"][2]["status"] = json!("input_required");
        computer.send(&mut app, 2, waiting);
        shoot(&app, "5-needs-input");
        computer.send(&mut app, 2, json!({"event":"task.done","title":"CC:Release notes","tasks":[{"id":"t9","agent":"claude-code","title":"CC:Release notes","status":"working","project":"website","elapsed_s":30}],"stats":["8 DONE"]}));
        shoot(&app, "6-done");
        computer.send(&mut app, 2, json!({"event":"task.error","tasks":[{"id":"t3","agent":"github-actions","title":"CI:App tests","status":"working","project":"vibebuddy","elapsed_s":200}],"stats":["8 DONE"]}));
        shoot(&app, "7-failed");
        computer.send(&mut app, 2, json!({"event":"agent.idle","tasks":[],"stats":["8 DONE","3 ASKS","4H 40M BUSY"]}));
        shoot(&app, "8-duty-ready");
        computer.send(&mut app, 2, working());
        for _ in 0..(16 * 60) {
            app.step(0);
        }
        shoot(&app, "9-no-link");
        let mut app = paired_app();
        let mut computer = Computer::link(&mut app, 2);
        computer.send(&mut app, 2, working());
        press(&mut app, key::START);
        for second in 0..83 {
            if second % 5 == 0 {
                computer.send(&mut app, 2, json!({"version":1,"event":"device.heartbeat"}));
            }
            for _ in 0..60 {
                app.step(0);
            }
        }
        shoot(&app, "10-pomodoro");
        press(&mut app, key::X);
        shoot(&app, "11-menu");
        press(&mut app, key::DOWN);
        press(&mut app, key::A);
        shoot(&app, "12-menu-forget");
        press(&mut app, key::B);
        press(&mut app, key::UP);
        press(&mut app, key::A);
        shoot(&app, "13-pair-another");
    }
    native::set_lang(Lang::En);
    // The cover on the VibeGame's home screen: 96×80.
    if let Some(folder) = &folder {
        let mut frame = Frame { palette: art.palette, ..Frame::default() };
        frame.clear(art::INK);
        art::robot(&mut frame, 9, 4, 3, art::Mood::Happy);
        let mut rgb = Vec::new();
        for y in 0..80 {
            for x in 0..96 {
                rgb.extend_from_slice(&frame.palette[frame.pixels[y * W + x] as usize]);
            }
        }
        let file = std::io::BufWriter::new(std::fs::File::create(folder.join("cover.png")).unwrap());
        let mut encoder = png::Encoder::new(file, 96, 80);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&rgb).unwrap();
    }
}
