//! The VibePlay's screen against the box's, pixel for pixel. The firmware's previews draw fixed scenes straight
//! into a Display (`firmware-rs/core/examples/preview.rs` and `korekore_preview.rs`); here the app reaches the
//! same scenes the way it does on the handheld, through the computer's lines over the Noise link, then halves its
//! 640 × 480 frame and compares it with the preview's PPM.
//!
//! A preview also sets a few things directly that the VibePlay reaches only with time or never: the animation's
//! frame, a leisure skit, the box's link icon (USB in korekore's) and the box's key names. Those go through
//! `Buddy::preview`; everything else is the app's own path.
//!
//! `tools/compare-box-screens.sh` makes the previews and runs this. Without `VIBEPLAY_BOX_PREVIEW` it does nothing.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use qapp::host;
use serde_json::{Value, json};
use vibeplay_app::app::App;
use vibeplay_core::firmware::canvas::{HEIGHT, WIDTH};
use vibeplay_core::firmware::display::{KeyNames, LinkPath, Mode, Screens};
use vibeplay_core::firmware::leisure::{self, Leisure, Skit};
use vibeplay_core::frame::BIG_WIDTH;

struct Bench {
    app: App,
    computer: Computer,
    previews: PathBuf,
    shots: Option<PathBuf>,
    theme: &'static str,
    link: Option<LinkPath>,
    /// The preview's clock. The app counts frames, so its own clock can be up to a frame behind; the screen is
    /// drawn again at this time before each shot.
    ms: u32,
    compared: Vec<String>,
    different: Vec<String>,
}

fn read_ppm(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let header = format!("P6\n{WIDTH} {HEIGHT}\n255\n");
    assert!(bytes.starts_with(header.as_bytes()), "{}", path.display());
    bytes[header.len()..].to_vec()
}

fn task(title: &str, project: &str, status: &str, elapsed_s: u32, agent: &str) -> Value {
    json!({"id": "a1", "title": title, "project": project, "status": status, "elapsed_s": elapsed_s, "agent": agent})
}

/// A state event as the daemon sends it; `title` as the preview's.
fn event(name: &str, title: Option<&str>, tasks: &[Value], total: usize, stats: Option<&[&str]>) -> Value {
    let mut event = json!({"version": 1, "event": name, "tasks": tasks, "tasks_total": total, "quiet": false});
    if let Some(title) = title {
        event["title"] = json!(title);
    }
    if let Some(stats) = stats {
        event["stats"] = json!(stats);
    }
    event
}

impl Bench {
    /// A paired VibePlay with today's Pomodoro record, linked to a computer, drawing `screens` with the box's key names.
    fn new(previews: &Path, theme: &'static str, screens: Screens, tally: Value, link: Option<LinkPath>) -> Self {
        drop(paired_app());
        let _ = host::data_write("tally", tally.to_string().as_bytes());
        let mut app = App::new();
        let computer = Computer::link(&mut app, 2);
        app.buddy.set_screens(screens, 0);
        app.buddy.preview(0, |_| {}, |display, _, _| display.set_keys(KeyNames::BOX));
        let shots = std::env::var_os("VIBEPLAY_SHOTS").map(|folder| PathBuf::from(folder).join("box-compare"));
        if let Some(shots) = &shots {
            std::fs::create_dir_all(shots).unwrap();
        }
        Self { app, computer, previews: previews.join(theme), shots, theme, link, ms: 0, compared: Vec::new(), different: Vec::new() }
    }

    fn now(&self) -> u32 {
        self.ms
    }

    /// Sets the clock to `ms`, the app's as near as its frames allow, with the computer just heard from. The robot's
    /// preview, like the C one, counts in 10 ms ticks.
    fn at(&mut self, ms: u32) {
        self.ms = if self.theme == "robot" { ms / 10 * 10 } else { ms };
        self.app.frames = u64::from(ms) * 60 / 1000;
        self.app.heard = Some(self.app.now());
    }

    fn send(&mut self, event: Value) {
        self.computer.send(&mut self.app, 2, event);
    }

    fn pose(&mut self, frame: Option<u32>, shake: Option<u32>, alarm: Option<bool>) {
        let now = self.now();
        self.app.buddy.preview(now, |_| {}, |display, _, _| display.preview_pose(frame, shake, alarm));
    }

    fn tick(&mut self, link_alive: bool) {
        let now = self.now();
        self.app.buddy.tick(now, link_alive, false);
    }

    fn shoot(&mut self, name: &str) {
        let (now, link) = (self.now(), self.link);
        self.app.buddy.preview(now, |_| {}, |display, screen, scene| {
            display.set_link(screen, scene, link, false);
            display.refresh(screen, scene);
        });
        let big = self.app.shot().to_vec();
        if let Some(shots) = &self.shots {
            save_png(&big, &shots.join(format!("{}-{name}.png", self.theme)));
        }
        let preview = read_ppm(&self.previews.join(format!("{name}.ppm")));
        let mut wrong = 0;
        for y in 0..HEIGHT as usize {
            for x in 0..WIDTH as usize {
                let at = (y * 2 * BIG_WIDTH + x * 2) * 2;
                let ours = rgb(u16::from_be_bytes([big[at], big[at + 1]]));
                let theirs = &preview[(y * WIDTH as usize + x) * 3..][..3];
                wrong += usize::from(ours != theirs);
            }
        }
        self.compared.push(format!("{}/{name}", self.theme));
        if wrong > 0 {
            self.different.push(format!("{}/{name}: {wrong} pixels", self.theme));
        }
    }

    /// A leisure skit at `frame`, as the previews start one.
    fn skit(&mut self, skit: Skit, name: &str, frame: u32) {
        let start = 100_000;
        let before = self.now();
        self.app.buddy.preview(
            before,
            |leisure| {
                *leisure = Leisure::new(1, start);
                leisure.start_skit(skit, start);
            },
            |display, screen, scene| {
                display.set_mode(screen, scene, Mode::Duty);
                display.set_mode(screen, scene, Mode::Leisure);
            },
        );
        self.at(start + frame * leisure::FRAME_MS);
        self.shoot(name);
        self.at(before);
    }
}

const ROBOT_STATS: &[&str] = &["7 DONE", "4 ASKS", "1H23 BUSY"];
const KOREKORE_STATS: &[&str] = &["7 done today", "3 asks", "4H12M busy"];

/// The scenes of `preview.rs`.
fn robot(previews: &Path) -> (Vec<String>, Vec<String>) {
    let mut b = Bench::new(previews, "robot", Screens::Robot, json!({"day": 20260915, "completed": 3, "focus_s": 4500}), None);
    b.at(1000);
    b.send(event("agent.idle", None, &[], 0, Some(ROBOT_STATS)));
    b.app.buddy.switch(1000);
    b.shoot("pomodoro_idle");

    let tasks = [
        json!({"title": "CC:POMODORO TIMER FEATURE", "status": "working", "elapsed_s": 75, "project": "AGENT-BEACON"}),
        json!({"title": "CX:EROS-TRAINING-INFRA", "status": "input_required", "elapsed_s": 900, "project": "EROS-TRAINING-INFRA"}),
    ];
    b.send(event("agent.input_required", Some("CC:AGENT-BEACON"), &tasks, 2, None));
    b.app.buddy.start(1000);
    b.at(1000 + 399_000);
    b.shoot("pomodoro_focus");
    b.app.buddy.start(b.now());
    b.pose(Some(1), None, None);
    b.shoot("pomodoro_paused_blink");
    b.pose(Some(0), None, None);
    b.shoot("pomodoro_paused");
    b.app.buddy.start(b.now());
    b.app.buddy.switch(b.now());
    b.shoot("pet_with_badge");

    // The end of focus: the Pomodoro ran all along, so the buddy never got bored.
    let focus_end = 1000 + 25 * 60 * 1000 + 100;
    b.at(focus_end);
    b.app.buddy.preview(focus_end, |leisure| leisure.note_activity(focus_end), |_, _, _| {});
    b.tick(true);
    b.send(event("agent.idle", None, &[], 0, None));
    for frame in 0..30 {
        if frame < 20 {
            b.pose(Some(frame), Some(20 - frame), None);
        } else {
            b.pose(Some((frame - 20) / 5), Some(0), None);
        }
        b.shoot(&format!("pomodoro_alarm_{frame:03}"));
    }
    b.pose(Some(1), Some(20), None);
    b.shoot("pomodoro_alarm_shake");
    b.pose(None, Some(0), None);
    b.shoot("pomodoro_alarm_pulse_dim");
    b.pose(Some(0), None, None);
    b.shoot("pomodoro_alarm_pulse");
    b.pose(None, None, Some(false));
    b.shoot("pomodoro_break_pending");

    b.app.buddy.start(focus_end + 30_000);
    b.at(focus_end + 30_000 + 2 * 60 * 1000);
    b.shoot("pomodoro_break");
    b.tick(false);
    b.pose(Some(0), None, None);
    b.shoot("pomodoro_no_link");
    b.tick(true);
    b.app.buddy.set_muted(true, b.now());
    b.app.buddy.switch(b.now());
    b.shoot("pet_muted");

    b.app.buddy.set_muted(false, b.now());
    let three = [
        json!({"title": "CC:UPDATE README", "status": "working", "elapsed_s": 20, "project": "VIBEBUDDY"}),
        json!({"title": "CX:BUILD FIRMWARE", "status": "working", "elapsed_s": 60, "project": "VIBEBUDDY"}),
        json!({"title": "CI:APP TESTS", "status": "working", "elapsed_s": 120, "project": "VIBEBUDDY"}),
    ];
    b.send(event("task.start", None, &three, 3, None));
    b.shoot("pet_three_tasks");

    // The skits start from a box that has only just started (`preview <dir> leisure`).
    let mut skits = Bench::new(previews, "robot", Screens::Robot, json!({"day": 20260915, "completed": 3, "focus_s": 4500}), None);
    skits.send(event("agent.idle", None, &[], 0, Some(ROBOT_STATS)));
    for (skit, name, frames) in [
        (Skit::Patrol, "patrol", 96),
        (Skit::Ball, "ball", 96),
        (Skit::Read, "read", 120),
        (Skit::Stars, "stars", 120),
        (Skit::Hide, "hide", 80),
        (Skit::Startle, "startle", 80),
        (Skit::Dream, "dream", 96),
        (Skit::Sleep, "sleep", 48),
        (Skit::None, "rest", 48),
    ] {
        for frame in 0..frames {
            skits.skit(skit, &format!("skit_{name}_{frame:03}"), frame);
        }
    }
    b.compared.extend(skits.compared);
    b.different.extend(skits.different);
    (b.compared, b.different)
}

/// The scenes of `korekore_preview.rs`, all but the ones of a box alone: the Relay's badge and the box's own menus.
fn korekore(previews: &Path) -> (Vec<String>, Vec<String>) {
    let tally = json!({"day": 20261027, "completed": 2, "focus_s": 3000});
    let mut b = Bench::new(previews, "korekore", Screens::Korekore, tally, Some(LinkPath::Usb));
    let stats = Some(KOREKORE_STATS);
    let working = [
        task("CC:Add Chinese to the box", "vibebuddy", "working", 420, "claude-code"),
        task("CX:Fix voice reply playback", "vibebuddy", "working", 60, "codex"),
        task("CI:App tests", "vibebuddy", "working", 120, "github-actions"),
    ];
    let first = |tasks: &[Value]| tasks[0]["title"].as_str().map(str::to_owned);
    b.send(event("task.start", first(&working).as_deref(), &working, 3, stats));
    b.shoot("duty-working");
    b.send(event("agent.idle", None, &[], 0, stats));
    b.pose(Some(1), None, None);
    b.shoot("duty-ready");
    b.pose(Some(36), None, None);
    b.shoot("duty-ready-nap");
    b.app.buddy.set_muted(true, 0);
    b.link = Some(LinkPath::Lan);
    b.pose(Some(1), None, None);
    b.shoot("duty-ready-muted-lan");
    b.app.buddy.set_muted(false, 0);
    b.link = Some(LinkPath::Usb);
    // The preview's Relay scene, not shot here, leaves its clock six minutes on.
    b.at(6 * 60 * 1000);

    let many = [
        task("CC:Add Chinese to the box", "vibebuddy", "working", 420, "claude-code"),
        task("CX:Fix voice reply playback", "vibebuddy", "working", 60, "codex"),
        task("CI:App tests", "vibebuddy", "working", 120, "github-actions"),
        task("OC:Refactor the menu", "vibebuddy", "working", 30, "opencode"),
        task("PI:Write the docs", "site", "working", 3700, "pi"),
    ];
    b.send(event("task.start", first(&many).as_deref(), &many, 7, stats));
    b.shoot("duty-pages");
    // The box's touchscreen pets Kore. The VibePlay has none, but the preview's clock stands still from here, so
    // Kore stays petted in the scenes after it: pet it the same way.
    let now = b.now();
    b.app.buddy.preview(now, |_| {}, |display, screen, scene| display.pet(screen, scene));
    b.shoot("duty-pet");

    let asking = [
        task("CX:Review changes", "vibebuddy", "input_required", 12, "codex"),
        task("CC:Release notes", "website", "input_required", 40, "claude-code"),
        task("CI:App tests", "vibebuddy", "working", 120, "github-actions"),
    ];
    b.send(event("agent.input_required", first(&asking).as_deref(), &asking, 3, stats));
    b.shoot("needs-input");
    let done = [task("CC:Release notes", "website", "done", 1500, "claude-code")];
    b.send(event("task.done", first(&done).as_deref(), &done, 1, stats));
    b.shoot("done");
    let failed = [task("CI:App tests", "vibebuddy", "failed", 120, "github-actions")];
    b.send(event("task.error", first(&failed).as_deref(), &failed, 1, stats));
    b.shoot("failed");

    b.send(event("agent.idle", None, &[], 0, stats));
    b.link = None;
    b.tick(false);
    b.shoot("no-link");
    b.send(event("task.start", first(&working).as_deref(), &working[..2], 2, stats));
    b.shoot("no-link-cards-kept");
    b.tick(true);
    b.link = Some(LinkPath::Usb);

    b.send(event("task.start", first(&working).as_deref(), &working[..1], 1, stats));
    b.app.buddy.switch(b.now());
    b.shoot("pomodoro-ready");
    b.app.buddy.start(b.now());
    b.at(b.now() + 3 * 60 * 1000 + 3000);
    b.pose(Some(0), None, None);
    b.shoot("pomodoro-focus");
    b.app.buddy.start(b.now());
    b.shoot("pomodoro-paused");
    b.app.buddy.start(b.now());
    b.send(event("agent.input_required", first(&asking).as_deref(), &asking[..1], 1, stats));
    b.shoot("pomodoro-focus-ask");
    b.at(b.now() + 23 * 60 * 1000);
    let now = b.now();
    b.app.buddy.preview(now, |leisure| leisure.note_activity(now), |_, _, _| {});
    b.tick(true);
    b.shoot("pomodoro-focus-ended");
    b.app.buddy.start(b.now());
    b.at(b.now() + 2 * 60 * 1000);
    b.app.buddy.switch(b.now());
    b.app.buddy.switch(b.now());
    b.shoot("pomodoro-break");
    b.app.buddy.stop(b.now());

    b.send(event("agent.idle", None, &[], 0, stats));
    for (skit, name, frame) in [
        (Skit::Patrol, "patrol", 12),
        (Skit::Ball, "ball", 10),
        (Skit::Read, "read", 64),
        (Skit::Stars, "stars", 40),
        (Skit::Hide, "hide", 40),
        (Skit::Startle, "startle", 26),
        (Skit::Dream, "dream", 10),
        (Skit::Sleep, "sleep", 4),
    ] {
        b.skit(skit, &format!("leisure-{name}"), frame);
    }
    (b.compared, b.different)
}

#[test]
fn the_vibeplay_draws_what_the_box_draws() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let Some(previews) = std::env::var_os("VIBEPLAY_BOX_PREVIEW").map(PathBuf::from) else {
        eprintln!("VIBEPLAY_BOX_PREVIEW not set: run tools/compare-box-screens.sh");
        return;
    };
    let (mut compared, mut different) = robot(&previews);
    let (more, wrong) = korekore(&previews);
    compared.extend(more);
    different.extend(wrong);
    eprintln!("compared {} scenes with the box's previews", compared.len());
    assert!(different.is_empty(), "different from the box:\n{}", different.join("\n"));
}
