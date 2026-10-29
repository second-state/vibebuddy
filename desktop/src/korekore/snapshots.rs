//! Draws the korekore views headless, with the canvas's sample data, into PNGs to set beside the design's boards:
//! `cargo test -p vibebuddy-desktop korekore::snapshots -- --ignored`, then look in target/korekore-preview.

use iced::{Element, Settings, Theme};
use iced_test::simulator::Simulator;

use super::picture::BoxPicture;
use super::{FONTS, Section, view};
use crate::status::{Activity, Status};
use crate::{Message, agents, client};

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

fn save<M: Clone>(name: &str, size: (f32, f32), element: Element<'_, M>) {
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
    save("window-character", (954.0, 766.0), view::settings(&working, &Section::ALL, Section::Character, "0.5.0", |section| section, view::pending()));
    save("window-device-asking", (954.0, 766.0), view::settings(&asking, &Section::ALL, Section::Device, "0.5.0", |section| section, view::pending()));
    for (name, sample) in [("working", &working), ("asking", &asking), ("quiet", &picture(true, &[])), ("no-link", &picture(false, &[]))] {
        save(&format!("minibox-{name}"), (214.0, 166.0), view::mini_box::<Section>(sample));
    }
}

/// The canvas's sample status, as JSON from the daemon. Nothing here asks the daemon, and the conversation is canned:
/// the real one is the user's own.
const SAMPLE: &str = r#"{"daemon":{"build":"0.5.0 cc16829","revision":"firmware-v0.5.0-61-gcc16829"},
 "device":{"connected":true,"port":"/dev/ttyACM0","mode":"duty","firmware_version":"0.5.0","firmware_build":"cc16829",
   "voice":"robot","volume":50,"box_key":"k","computer_key":"me",
   "paired_computers":[{"key":"me","name":"ThinkPad · Omarchy"},{"key":"m","name":"Michael's Mac Studio"}],
   "wifi_network":"ByteTrade","wifi_address":"192.168.1.160","transfer":true,"screens":"korekore"},
 "today":{"done":7,"asks":3,"busy_seconds":15120},
 "hooks":{"codex":"2026-10-21T16:40:00+08:00","claude":"2026-10-21T17:03:00+08:00"},
 "config":{"notify_link":true,"check_updates":true,"relay":false},
 "updates":{"enabled":true,"last_check":"2026-10-21T13:51:00+08:00"},
 "account":{"available":true,"signed_in":true,"github_id":1,"github_login":"kore-sample",
   "today":{"day":"2026-10-21","commands":12,"commands_limit":200},
   "plan":{"name":"paid","period_end":"2026-11-24T00:00:00Z","characters":12300,"characters_limit":100000}},
 "voice_keys":{"elevenlabs":{"set":true,"refused":false},"volc":{"set":false,"refused":false}},
 "making":{"id":"made-1","name":"Night owl","state":"done","done":12,"total":12},
 "activities":[{"agent":"claude-code","title":"CC:Add Chinese to the box","state":"working"},
               {"agent":"codex","title":"CX:Fix voice reply","state":"working"},
               {"agent":"github-actions","title":"CI:App tests","state":"working"}]}"#;

#[test]
#[ignore = "writes PNGs for review; run with --ignored"]
fn draw_the_korekore_pages() {
    let (mut app, _) = crate::App::boot();
    app.korekore = true;
    app.status = Some(serde_json::from_str(SAMPLE).unwrap());
    app.resident = true;
    app.resident_info = serde_json::from_str(r#"{"enabled":true,"revision":1,"brain_setting":"codex","anthropic_key_set":false}"#).unwrap();
    app.conversation = Some(Ok(vec![
        client::Exchange {
            said: "Quick check: what time is it, and how many agents are working?".to_owned(),
            replies: vec!["It's 10:09 a.m. in Singapore. Three agents are working.".to_owned()],
            at: Some(chrono::Utc::now() - chrono::Duration::days(1)),
        },
        client::Exchange { said: "新加坡明天天气怎么样".to_owned(), replies: vec!["明天上午多云转晴，最高约三十四度；午后可能有雷阵雨，出门记得带伞。".to_owned()], at: Some(chrono::Utc::now()) },
    ]));
    let setup = |agent: &str, name: &str, present: bool, installed: bool| agents::Setup {
        agent: agent.to_owned(),
        name: name.to_owned(),
        present,
        installed,
        config: std::path::PathBuf::from("/tmp/config"),
        modified: None,
    };
    app.agents = vec![
        setup("claude", "Claude Code", true, true),
        setup("codex", "Codex", true, true),
        setup("opencode", "OpenCode", true, true),
        setup("pi", "Pi", true, false),
        setup("copilot", "GitHub Copilot CLI", false, false),
    ];
    for page in 0..5 {
        app.guide = Some(page);
        let element: Element<'_, Message> = app.view(iced::window::Id::unique());
        save(&format!("guide-{page}"), (954.0, 766.0), element);
    }
    app.guide = None;
    app.made = vec![crate::making::MadeCharacter {
        id: "made-1".to_owned(),
        name: "Night owl".to_owned(),
        base: "luna".to_owned(),
        voice: "luna".to_owned(),
        lines: 12,
        complete: true,
    }];
    for section in Section::ALL {
        app.tab = section.tab();
        let element: Element<'_, Message> = app.view(iced::window::Id::unique());
        save(&format!("page-{}", format!("{section:?}").to_lowercase()), (954.0, 766.0), element);
    }
    // Making a Character, which takes the Character page's place.
    if let Some(base) = app.voices.first().map(crate::Lender::of) {
        app.tab = Section::Character.tab();
        app.make = Some(crate::MakeForm {
            base,
            name: "Night owl".to_owned(),
            voice: None,
            lines: vec![crate::making::Line { occasion: "done".to_owned(), text: "All done, {address}.".to_owned() }],
            original: Vec::new(),
            occasions: vec!["done".to_owned()],
            look: None,
            problem: None,
            starting: false,
        });
        let element: Element<'_, Message> = app.view(iced::window::Id::unique());
        save("page-make", (954.0, 766.0), element);
    }
}
