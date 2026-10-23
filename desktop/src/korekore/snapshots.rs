//! Draws the korekore views headless, with the canvas's sample data, into PNGs to set beside the design's boards:
//! `cargo test -p vibebuddy-desktop korekore::snapshots -- --ignored`, then look in target/korekore-preview.

use iced::{Element, Settings, Theme};
use iced_test::simulator::Simulator;

use super::picture::BoxPicture;
use super::{FONTS, Section, view};
use crate::status::{Activity, Status};

fn picture(connected: bool, activities: &[(&str, &str, &str)]) -> BoxPicture {
    let mut status = Status::default();
    status.device.connected = connected;
    status.device.mode = Some("duty".to_owned());
    status.device.firmware_version = Some("0.5.0".to_owned());
    status.today.done = 7;
    status.today.asks = 3;
    status.today.busy_seconds = 15_120;
    status.activities = activities
        .iter()
        .map(|(agent, title, state)| Activity {
            agent: (*agent).to_owned(),
            title: (*title).to_owned(),
            state: (*state).to_owned(),
            ..Activity::default()
        })
        .collect();
    BoxPicture::derive(Some(&status))
}

fn save(name: &str, size: (f32, f32), element: Element<'_, view::Pick>) {
    let settings = Settings { fonts: FONTS.iter().map(|font| (*font).into()).collect(), ..Settings::default() };
    let mut simulator = Simulator::with_size(settings, size, element);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/korekore-preview").join(format!("{name}.png"));
    // The simulator only writes a file that isn't there yet.
    let written = path.with_file_name(format!("{name}-tiny-skia.png"));
    let _ = std::fs::remove_file(&written);
    simulator.snapshot(&Theme::Light).unwrap().matches_image(&path).unwrap();
    println!("wrote {}", written.display());
}

#[test]
#[ignore = "writes PNGs for review; run with --ignored"]
fn draw_the_korekore_views() {
    let working = picture(
        true,
        &[
            ("claude-code", "CC:Add Chinese to the box", "working"),
            ("codex", "CX:Fix voice reply", "working"),
            ("github-actions", "CI:App tests", "working"),
        ],
    );
    let asking = picture(true, &[("codex", "CX:Review changes", "needs_input"), ("claude-code", "CC:Add Chinese to the box", "working")]);
    // The window's inside, as on the canvas (960 × 772 less its 3 px outline).
    save("window-character", (954.0, 766.0), view::settings(&working, &Section::ALL, Section::Character, "0.5.0", view::pending()));
    save("window-device-asking", (954.0, 766.0), view::settings(&asking, &Section::ALL, Section::Device, "0.5.0", view::pending()));
    for (name, sample) in [("working", &working), ("asking", &asking), ("quiet", &picture(true, &[])), ("no-link", &picture(false, &[]))] {
        save(&format!("minibox-{name}"), (214.0, 166.0), view::mini_box(sample));
    }
}
