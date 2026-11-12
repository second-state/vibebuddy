use serde_json::json;

use super::*;

fn working() -> Value {
    json!({"version":1,"event":"task.start","tasks_total":4,"stats":["7 DONE","3 ASKS","4H 12M BUSY"],"quiet":false,
        "tasks":[{"id":"t1","agent":"claude-code","title":"CC:Add Chinese to the box","status":"working","project":"vibebuddy","elapsed_s":420},
                 {"id":"t2","agent":"codex","title":"CX:Fix voice reply playback","status":"working","project":"vibebuddy","elapsed_s":65},
                 {"id":"bad id!","title":"Third","status":"working"}]})
}

#[test]
fn a_state_event_says_its_line_unless_quiet_or_suppressed() {
    let mut buddy = Buddy::new(Screens::Robot, false, None, 1, 0);
    let mut event = working();
    event["event"] = json!("agent.input_required");
    let news = buddy.line(&event, 100);
    assert_eq!((news.sound, news.takes_screen), (Some(Sound::InputRequired), true));
    event["quiet"] = json!(true);
    event["event"] = json!("task.done");
    assert_eq!(buddy.line(&event, 200).sound, None, "a box speaks for it");
    event["quiet"] = json!(false);
    event["suppress_audio"] = json!(true);
    assert_eq!(buddy.line(&event, 300).sound, None);
    event["announcement"] = json!("failed");
    assert_eq!(buddy.line(&event, 400).sound, Some(Sound::Failed));
    event.as_object_mut().unwrap().remove("announcement");
    event["suppress_audio"] = json!(false);
    event["occasion"] = json!("first_done");
    assert_eq!(buddy.line(&event, 500).sound, Some(Sound::Done), "a special occasion plays its ordinary line");
}

#[test]
fn up_and_down_pick_the_cards_that_can_be_focused() {
    let mut buddy = Buddy::new(Screens::Robot, false, None, 1, 0);
    buddy.line(&working(), 0);
    buddy.pick(true, 10);
    assert_eq!(buddy.picked().as_deref(), Some("t1"));
    buddy.pick(true, 20);
    assert_eq!(buddy.picked().as_deref(), Some("t2"));
    buddy.pick(true, 30);
    assert_eq!(buddy.picked().as_deref(), Some("t1"), "the card with a bad id is skipped, then it wraps");
    buddy.pick(false, 40);
    assert_eq!(buddy.picked().as_deref(), Some("t2"));
    buddy.tick(40 + PICK_MS, true, false);
    assert_eq!(buddy.picked(), None, "let go after a while");
}

#[test]
fn the_cards_on_the_second_page_are_reached_by_turning_it() {
    let mut buddy = Buddy::new(Screens::Korekore, false, None, 1, 0);
    let tasks: Vec<Value> = (1..=5).map(|n| json!({"id": format!("c{n}"), "title": format!("Task {n}"), "status": "working"})).collect();
    buddy.line(&json!({"version":1,"event":"task.start","tasks":tasks}), 0);
    for _ in 0..3 {
        buddy.pick(true, 10);
    }
    assert_eq!(buddy.picked().as_deref(), Some("c3"));
    buddy.pick(true, 20);
    assert_eq!(buddy.picked().as_deref(), Some("c4"), "the page turned");
}

#[test]
fn done_settles_back_and_no_link_changes_the_screen() {
    let mut buddy = Buddy::new(Screens::Robot, false, None, 1, 0);
    buddy.line(&working(), 0);
    let working_frame = buddy.frame().unwrap().to_vec();
    buddy.line(&json!({"version":1,"event":"task.done","title":"CC:Add Chinese to the box","tasks":[]}), 1000);
    assert_ne!(buddy.frame().unwrap(), &working_frame[..]);
    buddy.tick(1000 + DONE_TO_IDLE_MS, true, false);
    let linked = buddy.frame().unwrap().to_vec();
    buddy.tick(1000 + DONE_TO_IDLE_MS + 1, false, false);
    assert_ne!(buddy.frame().unwrap(), &linked[..]);
}

#[test]
fn the_pomodoro_runs_on_start_and_gives_up_on_stop() {
    let mut buddy = Buddy::new(Screens::Robot, false, None, 1, 0);
    buddy.start(1000);
    assert_eq!(buddy.mode(), Mode::Pomodoro);
    let news = buddy.tick(1000 + 25 * 60 * 1000, true, false);
    assert_eq!((news.sound, news.tally, news.takes_screen), (Some(Sound::FocusEnded), true, true));
    assert_eq!(buddy.tally().completed, 1);
    buddy.start(1000 + 25 * 60 * 1000 + 10);
    buddy.stop(1000 + 25 * 60 * 1000 + 20);
    buddy.switch(1000 + 25 * 60 * 1000 + 30);
    assert_eq!(buddy.mode(), Mode::Duty);
}

#[test]
fn an_idle_buddy_wanders_off_and_comes_back_for_a_key() {
    let mut buddy = Buddy::new(Screens::Robot, false, None, 1, 0);
    buddy.line(&json!({"version":1,"event":"agent.idle","tasks":[]}), 0);
    let mut now = 0;
    while buddy.mode() != Mode::Leisure && now < 10 * 60 * 1000 {
        now += 1000;
        buddy.tick(now, true, false);
    }
    assert_eq!(buddy.mode(), Mode::Leisure);
    buddy.key(now + 10);
    assert_eq!(buddy.mode(), Mode::Duty);
}
