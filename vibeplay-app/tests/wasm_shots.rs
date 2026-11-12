//! The wasm QingOS runs draws what these tests draw, pixel for pixel. `tools/compare-wasm-shots.sh` takes a
//! `vibeplay shot` of the release wasm for each scene, with fixed data and a key script, and this replays the same
//! scenes here and compares. The scenes leave out the pairing code, which is random. Without `VIBEPLAY_WASM_SHOTS`
//! it does nothing.

mod common;

use std::path::Path;

use common::*;
use qapp::full::key;
use qapp::host;
use vibeplay_app::app::App;
use vibeplay_core::frame::{BIG_HEIGHT, BIG_WIDTH};

/// The keys held on each frame, from a `vibeplay` key script: "frame keys…" or "first-last keys…" a line.
fn keys(script: &str, frames: usize) -> Vec<u32> {
    let mut held = vec![0; frames + 1];
    for line in script.lines().map(|line| line.split('#').next().unwrap().trim()).filter(|line| !line.is_empty()) {
        let mut words = line.split_whitespace();
        let span = words.next().unwrap();
        let (first, last) = span.split_once('-').unwrap_or((span, span));
        let mask = words
            .map(|word| match word {
                "up" => key::UP,
                "down" => key::DOWN,
                "left" => key::LEFT,
                "right" => key::RIGHT,
                "a" => key::A,
                "b" => key::B,
                "x" => key::X,
                "y" => key::Y,
                "l1" => key::L1,
                "r1" => key::R1,
                "start" => key::START,
                "select" => key::SELECT,
                other => panic!("no key {other}"),
            })
            .fold(0, |mask, key| mask | key);
        for held in &mut held[first.parse::<usize>().unwrap()..=last.parse::<usize>().unwrap().min(frames)] {
            *held |= mask;
        }
    }
    held
}

fn read_png(path: &Path) -> Vec<u8> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width as usize, info.height as usize, info.color_type), (BIG_WIDTH, BIG_HEIGHT, png::ColorType::Rgb), "{}", path.display());
    pixels.truncate(info.buffer_size());
    pixels
}

/// The scene in `dir` here, RGB565: its data, then its keys up to its frame.
fn replay(dir: &Path) -> Vec<u8> {
    let _ = fresh();
    for name in ["identity", "computers", "settings"] {
        let _ = host::data_write(name, &std::fs::read(dir.join("data").join(name)).unwrap());
    }
    let frames: usize = std::fs::read_to_string(dir.join("frame")).unwrap().trim().parse().unwrap();
    let held = keys(&std::fs::read_to_string(dir.join("input")).unwrap(), frames);
    let mut app = App::new();
    for keys in &held[..frames] {
        app.step(*keys);
    }
    app.shot().to_vec()
}

#[test]
fn the_wasm_draws_what_the_tests_draw() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let Some(folder) = std::env::var_os("VIBEPLAY_WASM_SHOTS") else {
        eprintln!("VIBEPLAY_WASM_SHOTS not set: run tools/compare-wasm-shots.sh");
        return;
    };
    let mut scenes: Vec<_> = std::fs::read_dir(folder).unwrap().map(|entry| entry.unwrap().path()).filter(|path| path.is_dir()).collect();
    scenes.sort();
    let mut different = Vec::new();
    for dir in &scenes {
        let ours = replay(dir);
        let wasm = read_png(&dir.join("wasm.png"));
        // Compared as RGB565: the CLI widens each channel by repeating its top bits, these tests by scaling.
        let wrong = ours
            .as_chunks::<2>()
            .0
            .iter()
            .zip(wasm.as_chunks::<3>().0)
            .filter(|(ours, wasm)| {
                let pixel = u16::from_be_bytes(**ours);
                [(pixel >> 11) as u8, (pixel >> 5 & 0x3f) as u8, (pixel & 0x1f) as u8] != [wasm[0] >> 3, wasm[1] >> 2, wasm[2] >> 3]
            })
            .count();
        if wrong > 0 {
            save_png(&ours, &dir.join("ours.png"));
            different.push(format!("{}: {wrong} pixels", dir.file_name().unwrap().to_string_lossy()));
        }
    }
    eprintln!("compared {} scenes with the wasm's", scenes.len());
    assert!(!scenes.is_empty());
    assert!(different.is_empty(), "different from the wasm:\n{}", different.join("\n"));
}
