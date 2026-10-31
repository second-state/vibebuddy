//! Renders the korekore screens (ADR-0021) to PPM on the Mac, one file per state, to set beside the design's box
//! boards (docs/design/korekore). `tools/korekore-box-preview.sh` turns them into PNGs.
//!
//! Usage: cargo run -p vibebuddy-firmware-core --example korekore_preview -- <directory> [look.bin]

use std::fs;
use std::path::{Path, PathBuf};

use vibebuddy_firmware_core::agent_icon::AgentIcon;
use vibebuddy_firmware_core::canvas::{FRAME_BYTES, HEIGHT, WIDTH};
use vibebuddy_firmware_core::display::{Display, LinkPath, Mode, Panel, PanelKind, Scene, Screen, Screens, State, TaskInput, Tone};
use vibebuddy_firmware_core::leisure::{self, Leisure, Skit};
use vibebuddy_firmware_core::look::Look;
use vibebuddy_firmware_core::pomodoro::{Pomodoro, Tally};

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

struct Preview {
    directory: PathBuf,
    screen: Frame,
    display: Display,
    pomodoro: Pomodoro,
    leisure: Leisure,
    now: u32,
}

macro_rules! scene {
    ($preview:ident) => {
        Scene { now_ms: $preview.now, pomodoro: &$preview.pomodoro, leisure: &$preview.leisure }
    };
}

impl Preview {
    fn shot(&mut self, name: &str) {
        self.display.refresh(&mut self.screen, &scene!(self));
        write_ppm(&self.directory.join(format!("{name}.ppm")), &self.screen.bytes);
    }

    fn show(&mut self, state: State, tasks: &[TaskInput], total: usize) {
        let _ = self.display.show_tasks(&mut self.screen, &scene!(self), state, tasks.first().and_then(|task| task.title), tasks, total);
    }

    fn mode(&mut self, mode: Mode) {
        self.display.set_mode(&mut self.screen, &scene!(self), mode);
    }

    fn panel(&mut self, panel: Option<Panel>) {
        self.display.set_panel(&mut self.screen, &scene!(self), panel);
    }

    fn frame(&mut self, frame: u32) {
        self.display.preview_pose(Some(frame), None, None);
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

fn task<'a>(title: &'a [u8], project: &'a [u8], state: State, elapsed_s: i32, icon: AgentIcon) -> TaskInput<'a> {
    TaskInput { title: Some(title), state, elapsed_s, project: Some(project), id: Some(b"a1"), icon: Some(icon) }
}

fn list(title: &[u8], rows: &[(&[u8], &[u8])], selected: Option<usize>, hints: Vec<(&'static [u8], &'static [u8])>) -> Panel {
    Panel {
        kind: PanelKind::List,
        title: title.to_vec(),
        lines: rows.iter().map(|(label, value)| (label.to_vec(), value.to_vec(), Tone::Plain)).collect(),
        selected,
        hints,
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let directory = PathBuf::from(arguments.get(1).expect("usage: korekore_preview <output directory> [look.bin]"));
    fs::create_dir_all(&directory).expect("create directory");
    let mut preview = Preview {
        directory,
        screen: Frame { bytes: vec![0; FRAME_BYTES] },
        display: Display::new(),
        pomodoro: Pomodoro::new(),
        leisure: Leisure::new(1, 0),
        now: 0,
    };
    preview.display.start(&mut preview.screen, &scene!(preview)).unwrap();
    preview.display.set_screens(&mut preview.screen, &scene!(preview), Screens::Korekore);
    preview.display.set_stats(&[b"7 done today", b"3 asks", b"4H12M busy"]);
    preview.pomodoro.restore_tally(Tally { day: 20261027, completed: 2, focus_s: 3000 });
    preview.display.set_link(&mut preview.screen, &scene!(preview), Some(LinkPath::Usb), false);
    if let Some(path) = arguments.get(2) {
        let look = Look::parse(&fs::read(path).expect("read look")).expect("a look");
        preview.display.set_look(&mut preview.screen, &scene!(preview), Some(look));
    }

    let working = [
        task(b"CC:Add Chinese to the box", b"vibebuddy", State::Working, 420, AgentIcon::ClaudeCode),
        task(b"CX:Fix voice reply playback", b"vibebuddy", State::Working, 60, AgentIcon::Codex),
        task(b"CI:App tests", b"vibebuddy", State::Working, 120, AgentIcon::Ci),
    ];
    preview.show(State::Working, &working, 3);
    preview.shot("duty-working");

    preview.show(State::Idle, &[], 0);
    preview.frame(1);
    preview.shot("duty-ready");
    preview.frame(36);
    preview.shot("duty-ready-nap");
    preview.display.set_muted(&mut preview.screen, &scene!(preview), true);
    preview.display.set_link(&mut preview.screen, &scene!(preview), Some(LinkPath::Lan), false);
    preview.frame(1);
    preview.shot("duty-ready-muted-lan");
    preview.display.set_muted(&mut preview.screen, &scene!(preview), false);
    preview.display.set_link(&mut preview.screen, &scene!(preview), Some(LinkPath::Relay), true);
    preview.show(State::Working, &working[..2], 2);
    preview.pomodoro.toggle(preview.now);
    preview.now += 6 * 60 * 1000;
    preview.shot("duty-relay-staging-badge");
    preview.pomodoro.stop();
    preview.display.set_link(&mut preview.screen, &scene!(preview), Some(LinkPath::Usb), false);

    let many = [
        task(b"CC:Add Chinese to the box", b"vibebuddy", State::Working, 420, AgentIcon::ClaudeCode),
        task(b"CX:Fix voice reply playback", b"vibebuddy", State::Working, 60, AgentIcon::Codex),
        task(b"CI:App tests", b"vibebuddy", State::Working, 120, AgentIcon::Ci),
        task(b"OC:Refactor the menu", b"vibebuddy", State::Working, 30, AgentIcon::OpenCode),
        task(b"PI:Write the docs", b"site", State::Working, 3700, AgentIcon::Pi),
    ];
    preview.show(State::Working, &many, 7);
    preview.shot("duty-pages");
    preview.display.pet(&mut preview.screen, &scene!(preview));
    preview.shot("duty-pet");

    let asking = [
        task(b"CX:Review changes", b"vibebuddy", State::InputRequired, 12, AgentIcon::Codex),
        task(b"CC:Release notes", b"website", State::InputRequired, 40, AgentIcon::ClaudeCode),
        task(b"CI:App tests", b"vibebuddy", State::Working, 120, AgentIcon::Ci),
    ];
    preview.show(State::InputRequired, &asking, 3);
    preview.shot("needs-input");

    preview.show(State::Done, &[task(b"CC:Release notes", b"website", State::Done, 1500, AgentIcon::ClaudeCode)], 1);
    preview.shot("done");

    preview.show(State::Failed, &[task(b"CI:App tests", b"vibebuddy", State::Failed, 120, AgentIcon::Ci)], 1);
    preview.shot("failed");

    preview.show(State::Idle, &[], 0);
    preview.display.set_link(&mut preview.screen, &scene!(preview), None, false);
    preview.display.set_link_lost(&mut preview.screen, &scene!(preview), true);
    preview.shot("no-link");
    preview.show(State::Working, &working[..2], 2);
    preview.shot("no-link-cards-kept");
    preview.display.set_link_lost(&mut preview.screen, &scene!(preview), false);
    preview.display.set_link(&mut preview.screen, &scene!(preview), Some(LinkPath::Usb), false);

    // Pomodoro: ready, focus three minutes in, paused, a break, and the end of focus.
    preview.show(State::Working, &working[..1], 1);
    preview.mode(Mode::Pomodoro);
    preview.shot("pomodoro-ready");
    preview.pomodoro.toggle(preview.now);
    preview.now += 3 * 60 * 1000 + 3000;
    preview.frame(0);
    preview.shot("pomodoro-focus");
    preview.pomodoro.toggle(preview.now);
    preview.shot("pomodoro-paused");
    preview.pomodoro.toggle(preview.now);
    preview.show(State::InputRequired, &asking[..1], 1);
    preview.shot("pomodoro-focus-ask");
    preview.now += 23 * 60 * 1000;
    preview.pomodoro.tick(preview.now);
    preview.display.pomodoro_ended(&scene!(preview));
    preview.shot("pomodoro-focus-ended");
    preview.pomodoro.toggle(preview.now);
    preview.now += 2 * 60 * 1000;
    preview.mode(Mode::Duty);
    preview.mode(Mode::Pomodoro);
    preview.shot("pomodoro-break");
    preview.pomodoro.stop();

    // Leisure: a frame from each skit.
    preview.show(State::Idle, &[], 0);
    for (skit, name, frame) in [
        (Skit::Patrol, "patrol", 12u32),
        (Skit::Ball, "ball", 10),
        (Skit::Read, "read", 64),
        (Skit::Stars, "stars", 40),
        (Skit::Hide, "hide", 40),
        (Skit::Startle, "startle", 26),
        (Skit::Dream, "dream", 10),
        (Skit::Sleep, "sleep", 4),
    ] {
        let start = 100_000;
        preview.leisure = Leisure::new(1, start);
        preview.leisure.start_skit(skit, start);
        preview.mode(Mode::Duty);
        preview.mode(Mode::Leisure);
        preview.now = start + frame * leisure::FRAME_MS;
        preview.shot(&format!("leisure-{name}"));
    }
    preview.mode(Mode::Duty);

    // Panels: the menu, status, listening, a reply, a transfer.
    preview.show(State::Working, &working, 3);
    preview.panel(Some(list(
        b"MENU",
        &[(b"VOLUME", b"65"), (b"MUTE", b"OFF"), (b"COMPUTERS", b"2"), (b"NETWORK", b">"), (b"STATUS", b">")],
        Some(0),
        vec![(b"K1", b"NEXT"), (b"K0", b"CHANGE"), (b"K2", b"CLOSE")],
    )));
    preview.shot("panel-menu");
    preview.panel(Some(Panel {
        kind: PanelKind::Facts,
        title: b"STATUS".to_vec(),
        lines: vec![
            (b"FIRMWARE".to_vec(), b"0.5.0".to_vec(), Tone::Plain),
            (b"LINK".to_vec(), b"USB".to_vec(), Tone::Accent),
            (b"CHARACTER".to_vec(), b"KORE".to_vec(), Tone::Plain),
            (b"TODAY".to_vec(), b"2 FOCUS 50M".to_vec(), Tone::Plain),
        ],
        selected: None,
        hints: vec![(b"K0", b"BACK"), (b"K2", b"CLOSE")],
    }));
    preview.shot("panel-status");
    preview.panel(Some(Panel {
        kind: PanelKind::Meter(9),
        title: b"LISTENING".to_vec(),
        lines: vec![(b"12 S LEFT".to_vec(), Vec::new(), Tone::Plain)],
        selected: None,
        hints: vec![(b"K0", b"LET GO: SEND"), (b"K2", b"CANCEL")],
    }));
    preview.shot("panel-listening");
    preview.panel(Some(Panel {
        kind: PanelKind::Text(1),
        title: b"REPLY".to_vec(),
        lines: vec![
            ("现在在忙什么?".as_bytes().to_vec(), Vec::new(), Tone::Dim),
            ("它在写 v0.5.1 的发布说明，大概还要五分钟。".as_bytes().to_vec(), Vec::new(), Tone::Plain),
            ("写完我提醒你。".as_bytes().to_vec(), Vec::new(), Tone::Plain),
        ],
        selected: None,
        hints: vec![(b"K2", b"CLOSE")],
    }));
    preview.shot("panel-reply");
    preview.panel(Some(Panel {
        kind: PanelKind::Progress(42),
        title: b"UPDATING".to_vec(),
        lines: vec![(b"42%".to_vec(), Vec::new(), Tone::Plain)],
        selected: None,
        hints: vec![(b"HOLD K2", b"CANCEL")],
    }));
    preview.shot("panel-progress");
    preview.panel(None);
}
