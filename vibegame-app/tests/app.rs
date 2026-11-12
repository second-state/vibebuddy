//! The app with QingOS's test doubles: connections in, lines, mDNS, data and the screen out. `VIBEGAME_SHOTS=<folder>
//! cargo test --test app shots` also saves every screen, in both sets of screens, as 640 × 480 PNGs.

mod common;

use common::*;
use qapp::full::key;
use qapp::host;
use qapp::host::test::{conn_closed, mdns};
use serde_json::{Value, json};
use vibegame_app::app::App;
use vibegame_core::firmware::display::{Mode, Screens};
use vibegame_core::frame::{BIG_HEIGHT, BIG_WIDTH};

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
    assert_eq!(app.buddy.picked().as_deref(), Some("t2"));
    press(&mut app, key::A);
    assert_eq!(computer.open(&sent()[0]), json!({"version":1,"event":"focus","task":"t2"}));
    assert_eq!(app.buddy.picked(), None);
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
fn the_menu_picks_the_screens_mutes_and_forgets_computers_after_asking() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = paired_app();
    assert_eq!(app.buddy.screens(), Screens::Robot, "the robot's, as on the box");
    press(&mut app, key::X);
    press(&mut app, key::DOWN);
    press(&mut app, key::DOWN);
    press(&mut app, key::A);
    assert_eq!(app.buddy.screens(), Screens::Korekore);
    press(&mut app, key::DOWN);
    press(&mut app, key::A);
    assert!(app.muted);
    assert_eq!(serde_json::from_slice::<Value>(&host::data_read("settings").unwrap()).unwrap(), json!({"muted": true, "screens": "korekore"}));
    assert_eq!(App::new().buddy.screens(), Screens::Korekore, "kept for the next time");
    press(&mut app, key::UP);
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
fn start_runs_the_pomodoro_and_select_switches_screens() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = paired_app();
    let mut computer = Computer::link(&mut app, 2);
    computer.send(&mut app, 2, working());
    press(&mut app, key::SELECT);
    assert_eq!(app.buddy.mode(), Mode::Pomodoro);
    press(&mut app, key::SELECT);
    assert_eq!(app.buddy.mode(), Mode::Duty);
    press(&mut app, key::START);
    assert_eq!(app.buddy.mode(), Mode::Pomodoro, "a phase that starts shows the Pomodoro");
    for _ in 0..50 {
        app.step(key::START);
    }
    app.step(0);
    assert!(app.buddy.pomodoro(app.now() as u32).is_idle(), "held, it gives up");
}

#[test]
fn a_frame_goes_to_qingos_only_when_it_changed() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = paired_app();
    let mut computer = Computer::link(&mut app, 2);
    computer.send(&mut app, 2, json!({"version":1,"event":"task.start","tasks":[{"id":"a","title":"One","status":"working"}]}));
    app.step(0);
    assert_eq!(app.picture().map(<[u8]>::len), Some(BIG_WIDTH * BIG_HEIGHT * 2));
    assert_eq!(app.picture(), None, "nothing changed");
    computer.send(&mut app, 2, json!({"version":1,"event":"agent.idle","tasks":[]}));
    assert!(app.picture().is_some());
}

fn steps(app: &mut App, frames: u32) {
    for _ in 0..frames {
        app.step(0);
    }
}

/// Every screen, in both sets of screens. Drawn always, so a screen that panics fails the test; saved only with
/// `VIBEGAME_SHOTS`.
#[test]
fn shots() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let folder = std::env::var_os("VIBEGAME_SHOTS").map(std::path::PathBuf::from);
    if let Some(folder) = &folder {
        std::fs::create_dir_all(folder).unwrap();
    }
    for (screens, suffix) in [(Screens::Robot, "robot"), (Screens::Korekore, "korekore")] {
        let shoot = |app: &mut App, name: &str| {
            let big = app.shot().to_vec();
            if let Some(folder) = &folder {
                save_png(&big, &folder.join(format!("{name}-{suffix}.png")));
            }
        };
        let set = |app: &mut App| {
            let now = app.now() as u32;
            app.buddy.set_screens(screens, now);
        };
        let mut app = fresh();
        set(&mut app);
        steps(&mut app, 20);
        shoot(&mut app, "01-not-paired");
        let mut app = paired_app();
        set(&mut app);
        steps(&mut app, 40);
        shoot(&mut app, "02-looking");
        let mut computer = Computer::link(&mut app, 2);
        computer.send(&mut app, 2, working());
        steps(&mut app, 20);
        shoot(&mut app, "03-duty-working");
        press(&mut app, key::DOWN);
        press(&mut app, key::DOWN);
        shoot(&mut app, "04-duty-picked");
        press(&mut app, key::B);
        let mut waiting = working();
        waiting["event"] = json!("agent.input_required");
        waiting["title"] = json!("CX:Fix voice reply playback");
        waiting["tasks"][1]["status"] = json!("input_required");
        computer.send(&mut app, 2, waiting);
        shoot(&mut app, "05-needs-input");
        computer.send(&mut app, 2, json!({"version":1,"event":"task.done","title":"CC:Release notes","tasks":[{"id":"t9","agent":"claude-code","title":"CC:Release notes","status":"done","project":"website","elapsed_s":1500}],"stats":["8 DONE","3 ASKS","4H 20M BUSY"]}));
        shoot(&mut app, "06-done");
        computer.send(&mut app, 2, json!({"version":1,"event":"task.error","title":"CI:App tests","tasks":[{"id":"t3","agent":"github-actions","title":"CI:App tests","status":"failed","project":"vibebuddy","elapsed_s":200}],"stats":["8 DONE","3 ASKS","4H 20M BUSY"]}));
        shoot(&mut app, "07-failed");
        computer.send(&mut app, 2, json!({"version":1,"event":"agent.idle","tasks":[],"stats":["8 DONE","3 ASKS","4H 40M BUSY"]}));
        shoot(&mut app, "08-duty-ready");
        computer.send(&mut app, 2, working());
        steps(&mut app, 16 * 60);
        shoot(&mut app, "09-no-link");
        let mut app = paired_app();
        set(&mut app);
        let mut computer = Computer::link(&mut app, 2);
        computer.send(&mut app, 2, working());
        press(&mut app, key::START);
        for second in 0..83 {
            if second % 5 == 0 {
                computer.send(&mut app, 2, json!({"version":1,"event":"device.heartbeat"}));
            }
            steps(&mut app, 60);
        }
        shoot(&mut app, "10-pomodoro");
        press(&mut app, key::X);
        shoot(&mut app, "11-menu");
        press(&mut app, key::DOWN);
        press(&mut app, key::A);
        shoot(&mut app, "12-menu-forget");
        press(&mut app, key::B);
        press(&mut app, key::B);
        press(&mut app, key::X);
        press(&mut app, key::A);
        shoot(&mut app, "13-pair-another");
    }
}
