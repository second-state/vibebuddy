//! What the tests share: QingOS's test doubles, and a computer as the daemon is one (snow, the daemon's Noise
//! library).

#![allow(dead_code, reason = "each test binary uses some of it")]

use std::sync::Mutex;

use ed25519_dalek::SigningKey;
use qapp::host;
use qapp::host::test::{conn_closed, conn_sent};
use serde_json::{Value, json};
use vibegame_app::app::App;
use vibegame_core::frame::{BIG_HEIGHT, BIG_WIDTH};
use vibegame_core::noise;

/// QingOS's doubles are shared by the whole test binary: one test at a time.
pub static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

pub const DESK: [u8; 32] = [11; 32];

pub fn fresh() -> App {
    for name in ["identity", "computers", "settings", "tally"] {
        let _ = host::data_delete(name);
    }
    conn_sent();
    conn_closed();
    App::new()
}

pub fn sent() -> Vec<String> {
    conn_sent().into_iter().map(|(_, line)| line).collect()
}

pub fn b64(line: &str) -> Vec<u8> {
    noise::decode(line).unwrap()
}

/// A computer as the daemon is one.
pub struct Computer {
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

    pub fn pair(app: &mut App, id: u64, code: &str) -> Value {
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

    pub fn link(app: &mut App, id: u64) -> Computer {
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

    pub fn seal(&mut self, text: &str) -> String {
        let mut buffer = vec![0u8; text.len() + 16];
        let n = self.session.write_message(text.as_bytes(), &mut buffer).unwrap();
        noise::encode(&buffer[..n])
    }

    pub fn open(&mut self, line: &str) -> Value {
        let sealed = b64(line);
        let mut buffer = vec![0u8; sealed.len()];
        let n = self.session.read_message(&sealed, &mut buffer).unwrap();
        serde_json::from_slice(&buffer[..n]).unwrap()
    }

    pub fn send(&mut self, app: &mut App, id: u64, event: Value) {
        let sealed = self.seal(&event.to_string());
        app.conn_line(id, &sealed);
    }
}

pub fn press(app: &mut App, keys: u32) {
    app.step(keys);
    app.step(0);
}

pub fn paired_app() -> App {
    let mut app = fresh();
    let code = app.server.code().unwrap().to_owned();
    assert_eq!(Computer::pair(&mut app, 1, &code)["event"], "vibegame.paired");
    app
}

pub fn working() -> Value {
    json!({"version":1,"event":"task.start","tasks_total":4,"stats":["7 DONE","3 ASKS","4H 12M BUSY"],"quiet":false,
        "tasks":[{"id":"t1","agent":"claude-code","title":"CC:Add Chinese to the box","status":"working","project":"vibebuddy","elapsed_s":420},
                 {"id":"t2","agent":"codex","title":"CX:Fix voice reply playback","status":"working","project":"vibebuddy","elapsed_s":65},
                 {"id":"t3","agent":"github-actions","title":"CI:App tests","status":"working","project":"vibebuddy","elapsed_s":130}]})
}


/// One RGB565 pixel as the firmware's previews widen it to 8 bits a channel.
pub fn rgb(pixel: u16) -> [u8; 3] {
    let pixel = u32::from(pixel);
    [((pixel >> 11) & 0x1f) * 255 / 31, ((pixel >> 5) & 0x3f) * 255 / 63, (pixel & 0x1f) * 255 / 31].map(|c| c as u8)
}

/// A 640 × 480 frame, RGB565 high byte first, as a PNG.
pub fn save_png(big: &[u8], path: &std::path::Path) {
    let rgb: Vec<u8> = big.as_chunks::<2>().0.iter().flat_map(|pair| rgb(u16::from_be_bytes(*pair))).collect();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).unwrap()), BIG_WIDTH as u32, BIG_HEIGHT as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&rgb).unwrap();
}
