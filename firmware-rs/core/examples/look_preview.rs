//! Renders a Character's look on the real screens, to check it before it goes into a pack: each duty
//! state and each leisure skit, frame by frame, as PPM. `tools/preview-look.sh` turns them into GIFs.
//!
//! Usage: cargo run -p vibebuddy-firmware-core --example look_preview -- <look.bin> <directory>

use std::fs;
use std::path::{Path, PathBuf};

use vibebuddy_firmware_core::canvas::{FRAME_BYTES, HEIGHT, WIDTH};
use vibebuddy_firmware_core::display::{Display, Mode, Scene, Screen, State, TaskInput};
use vibebuddy_firmware_core::leisure::{self, Leisure, Skit};
use vibebuddy_firmware_core::look::Look;
use vibebuddy_firmware_core::pomodoro::Pomodoro;

struct Frame {
    bytes: Vec<u8>,
}

impl Screen for Frame {
    fn frame(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
    fn present(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn set_backlight(&mut self, _on: bool) -> Result<(), ()> {
        Ok(())
    }
}

fn write_ppm(path: &Path, bytes: &[u8]) {
    let mut out = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
    for pair in bytes[..FRAME_BYTES].as_chunks::<2>().0 {
        let pixel = u16::from_be_bytes([pair[0], pair[1]]) as u32;
        out.push((((pixel >> 11) & 0x1f) * 255 / 31) as u8);
        out.push((((pixel >> 5) & 0x3f) * 255 / 63) as u8);
        out.push(((pixel & 0x1f) * 255 / 31) as u8);
    }
    fs::write(path, out).expect("write PPM");
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let (Some(look_path), Some(directory)) = (arguments.get(1), arguments.get(2)) else {
        panic!("usage: look_preview <look.bin> <output directory>");
    };
    let directory = PathBuf::from(directory);
    fs::create_dir_all(&directory).expect("create directory");
    let look = Look::parse(&fs::read(look_path).expect("read look")).expect("not a look");

    let mut screen = Frame { bytes: vec![0; FRAME_BYTES] };
    let mut display = Display::new();
    let pomodoro = Pomodoro::new();
    let mut leisure = Leisure::new(1, 0);
    let mut now = 1000;
    macro_rules! scene {
        () => {
            Scene { now_ms: now, pomodoro: &pomodoro, leisure: &leisure }
        };
    }
    display.start(&mut screen, &scene!()).unwrap();
    display.set_firmware_build(&mut screen, &scene!(), b"look preview");
    display.set_stats(&[b"7 DONE", b"4 ASKS", b"1H23 BUSY"]);
    display.set_look(&mut screen, &scene!(), Some(look));

    let cards = [
        TaskInput { title: Some(b"CC:CHARACTERS"), state: State::Working, elapsed_s: 75, project: Some(b"VIBE-BUDDY") },
        TaskInput { title: Some(b"CX:EROS-TRAINING"), state: State::Working, elapsed_s: 900, project: None },
    ];
    let states: [(&str, State, &[TaskInput], u32); 6] = [
        ("idle", State::Idle, &[], 120),
        ("working", State::Working, &cards, 12),
        ("input", State::InputRequired, &[], 8),
        ("done", State::Done, &[], 8),
        ("failed", State::Failed, &[], 4),
        ("offline", State::Offline, &[], 1),
    ];
    for (name, state, tasks, frames) in states {
        let shown = if state == State::Offline { State::Idle } else { state };
        let _ = display.show_tasks(&mut screen, &scene!(), shown, Some(b"CC:VIBE-BUDDY"), tasks);
        display.set_link_lost(&mut screen, &scene!(), state == State::Offline);
        for frame in 0..frames {
            display.preview_pose(Some(frame), None, None);
            display.refresh(&mut screen, &scene!());
            write_ppm(&directory.join(format!("duty_{name}_{frame:03}.ppm")), &screen.bytes);
        }
    }
    display.set_link_lost(&mut screen, &scene!(), false);
    let _ = display.show_tasks(&mut screen, &scene!(), State::Idle, None, &[]);

    let skits = [
        (Skit::Patrol, "patrol", 96),
        (Skit::Hide, "hide", 80),
        (Skit::Startle, "startle", 80),
        (Skit::Stars, "stars", 120),
        (Skit::Dream, "dream", 96),
        (Skit::Sleep, "sleep", 48),
        (Skit::None, "rest", 48),
    ];
    for (skit, name, frames) in skits {
        let start = 100_000;
        leisure = Leisure::new(1, start);
        leisure.start_skit(skit, start);
        display.set_mode(&mut screen, &scene!(), Mode::Duty);
        display.set_mode(&mut screen, &scene!(), Mode::Leisure);
        for frame in 0..frames {
            now = start + frame * leisure::FRAME_MS;
            display.refresh(&mut screen, &scene!());
            write_ppm(&directory.join(format!("skit_{name}_{frame:03}.ppm")), &screen.bytes);
        }
    }
}
