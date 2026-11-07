//! Renders each decoration (ADR-0022) over both sets of box screens to PPM on the Mac, a few states each, to check
//! that one drawing reads well on every set: `<screens>-<decoration>-<state>.ppm`. Also the frames the apps play as
//! the decoration's preview, the ready screen every `FRAME_MS`: `frame-<screens>-<decoration>-<nn>.ppm` (one frame
//! for off). `tools/decoration-previews.sh` turns those into the apps' strips in `decorations/`.
//!
//! Usage: cargo run -p vibebuddy-firmware-core --example decoration_preview -- <directory>

use std::fs;
use std::path::{Path, PathBuf};

use vibebuddy_firmware_core::agent_icon::AgentIcon;
use vibebuddy_firmware_core::canvas::{FRAME_BYTES, HEIGHT, WIDTH};
use vibebuddy_firmware_core::decoration::{self, Decoration};
use vibebuddy_firmware_core::display::{Display, LinkPath, Scene, Screen, Screens, State, TaskInput};
use vibebuddy_firmware_core::leisure::Leisure;
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

fn task<'a>(title: &'a [u8], state: State, icon: AgentIcon) -> TaskInput<'a> {
    TaskInput { title: Some(title), state, elapsed_s: 420, project: Some(b"vibebuddy"), id: Some(b"a1"), icon: Some(icon) }
}

/// Frames in an app preview: 2.4 s at the box's own pace.
const PREVIEW_FRAMES: u32 = 16;

/// The ready screen in `screens` with `decoration`, as the box draws it at `now_ms`.
fn ready(screens: Screens, decoration: Decoration, now_ms: u32) -> Vec<u8> {
    let (pomodoro, leisure) = (Pomodoro::new(), Leisure::new(1, 0));
    let scene = Scene { now_ms, pomodoro: &pomodoro, leisure: &leisure };
    let mut screen = Frame { bytes: vec![0; FRAME_BYTES] };
    let mut display = Display::new();
    display.start(&mut screen, &scene).unwrap();
    display.set_screens(&mut screen, &scene, screens);
    display.set_link(&mut screen, &scene, Some(LinkPath::Usb), false);
    display.set_decoration(&mut screen, &scene, decoration);
    display.refresh(&mut screen, &scene);
    screen.bytes
}

fn main() {
    let directory = PathBuf::from(std::env::args().nth(1).expect("usage: decoration_preview <output directory>"));
    fs::create_dir_all(&directory).expect("create directory");
    let pomodoro = Pomodoro::new();
    let leisure = Leisure::new(1, 0);
    let working = [
        task(b"CC:Add Chinese to the box", State::Working, AgentIcon::ClaudeCode),
        task(b"CX:Fix voice reply playback", State::Working, AgentIcon::Codex),
        task(b"CI:App tests", State::Working, AgentIcon::Ci),
    ];
    let asking = [task(b"CX:Review changes", State::InputRequired, AgentIcon::Codex)];
    let done = [task(b"CC:Release notes", State::Done, AgentIcon::ClaudeCode)];
    let states: [(&str, State, &[TaskInput]); 4] =
        [("ready", State::Idle, &[]), ("working", State::Working, &working), ("needs-input", State::InputRequired, &asking), ("done", State::Done, &done)];
    for screens in [Screens::Robot, Screens::Korekore] {
        for decoration in [Decoration::Off, Decoration::Thanksgiving, Decoration::Christmas] {
            let frames = if decoration == Decoration::Off { 1 } else { PREVIEW_FRAMES };
            for frame in 0..frames {
                let screens_name = String::from_utf8_lossy(screens.name()).to_lowercase();
                let decoration_name = String::from_utf8_lossy(decoration.name()).to_lowercase();
                let bytes = ready(screens, decoration, 61_000 + frame * decoration::FRAME_MS);
                write_ppm(&directory.join(format!("frame-{screens_name}-{decoration_name}-{frame:02}.ppm")), &bytes);
            }
        }
        for decoration in [Decoration::Thanksgiving, Decoration::Christmas] {
            for (name, state, tasks) in states {
                let mut screen = Frame { bytes: vec![0; FRAME_BYTES] };
                let mut display = Display::new();
                // Some way into the fall, so the flakes are spread over the screen.
                let scene = Scene { now_ms: 61_000, pomodoro: &pomodoro, leisure: &leisure };
                display.start(&mut screen, &scene).unwrap();
                display.set_screens(&mut screen, &scene, screens);
                display.set_link(&mut screen, &scene, Some(LinkPath::Usb), false);
                display.set_decoration(&mut screen, &scene, decoration);
                let _ = display.show_tasks(&mut screen, &scene, state, tasks.first().and_then(|task| task.title), tasks, tasks.len());
                display.refresh(&mut screen, &scene);
                let screens = String::from_utf8_lossy(screens.name()).to_lowercase();
                let decoration = String::from_utf8_lossy(decoration.name()).to_lowercase();
                write_ppm(&directory.join(format!("{screens}-{decoration}-{name}.ppm")), &screen.bytes);
            }
        }
    }
}
