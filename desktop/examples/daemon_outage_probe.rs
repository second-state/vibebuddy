//! Headless driver for the daemon-failure notification. The app's real status stream feeds the outage state machine,
//! and its effects run through the adapters the app uses, so the decisions are the app's. Arguments: the daemon's base
//! URL and the grace period in milliseconds (the app's is 30 s). Prints one line per event with the effect it caused.

#[allow(dead_code)]
#[path = "../src/client.rs"]
mod client;
#[allow(dead_code)]
#[path = "../src/i18n.rs"]
mod i18n;
#[allow(dead_code)]
#[path = "../src/outage.rs"]
mod outage;
#[allow(dead_code)]
#[path = "../src/status.rs"]
mod status;

use std::io::Write;
use std::time::{Duration, Instant};

use futures::StreamExt;
use outage::{Effect, Event, Outage};

// The probe always uses English and never reads/writes the user's saved language.
fn config_dir() -> Option<std::path::PathBuf> {
    None
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let base = args.next().expect("the daemon's base URL");
    // Exercise the recovery adapter the real app uses, including when its HTTP server is gone.
    if base == "--restart" {
        if let Err(error) = client::restart_daemon().await {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    let grace = Duration::from_millis(args.next().expect("the grace period in ms").parse().expect("a number of ms"));
    let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel::<Event>();

    let updates_tx = events_tx.clone();
    tokio::spawn(async move {
        let mut updates = std::pin::pin!(client::status_updates_at(base));
        while let Some(update) = updates.next().await {
            let event = match update {
                client::Update::Status(status) => Event::Status {
                    notify_link: status.config.notify_link,
                },
                client::Update::Down => Event::Down,
            };
            if updates_tx.send(event).is_err() {
                break;
            }
        }
    });

    let mut outage = Outage::new(grace);
    while let Some(event) = events.recv().await {
        let label = format!("{event:?}");
        let effect = outage.handle(event, Instant::now());
        match effect {
            Some(Effect::Wait(after)) => {
                let tx = events_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(after).await;
                    let _ = tx.send(Event::GraceElapsed);
                });
            }
            Some(Effect::CheckService) => {
                let tx = events_tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(Event::Service(outage::service().await));
                });
            }
            Some(Effect::Notify) => {
                if let Err(error) = outage::notify() {
                    println!("notify failed: {error}");
                }
            }
            None => {}
        }
        println!("{label} -> {effect:?}");
        std::io::stdout().flush().expect("flush the decision log");
    }
}
