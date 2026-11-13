//! A company edition's card and its way back (ADR-0026): the box sends the company Character's card on
//! `device.edition`, and copies its pack back into `voices` on `device.edition.restore`. See docs/protocol.md.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::broadcast;
use vibebuddy_protocol::Event;

use crate::serial_transport::{DeviceMessage, Transport};

/// The card arrives in under a second over the cable, and a line every 30 ms over the network or the Relay.
const CARD_SILENCE: Duration = Duration::from_secs(10);
/// The box reports each 10 % of a restore; erasing the partition before the first can take a few seconds.
const RESTORE_SILENCE: Duration = Duration::from_secs(30);

/// The company Character's card, for the App's Character list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub summary: String,
    /// The look (docs/characters.md), base64; None for a Character without one.
    pub look: Option<String>,
}

/// Asks the box for its edition's card and collects the `CARD` lines.
pub async fn fetch_card(transport: Arc<dyn Transport>, mut bus: broadcast::Receiver<DeviceMessage>) -> Result<Card, String> {
    send(&transport, "device.edition")?;
    let mut card: Option<Card> = None;
    loop {
        let line = next_line(&mut bus, CARD_SILENCE, "the card").await?;
        if line == "ERROR no_edition" {
            return Err("the box has no edition".to_owned());
        }
        if let Some(id) = line.strip_prefix("CARD BEGIN ") {
            card = Some(Card { id: id.to_owned(), name: String::new(), summary: String::new(), look: None });
            continue;
        }
        let Some(card) = card.as_mut() else { continue };
        if let Some(name) = line.strip_prefix("CARD NAME ") {
            card.name = name.to_owned();
        } else if let Some(summary) = line.strip_prefix("CARD SUMMARY ") {
            card.summary = summary.to_owned();
        } else if let Some(look) = line.strip_prefix("CARD LOOK ") {
            // Each line but the last holds a multiple of 3 bytes, so the pieces join into one base64 text.
            card.look.get_or_insert_with(String::new).push_str(look);
        } else if line == "CARD END" {
            return Ok(card.clone());
        }
    }
}

/// Has the box copy the company Character back into `voices`; returns the Character it then wears.
pub async fn restore(
    transport: Arc<dyn Transport>,
    mut bus: broadcast::Receiver<DeviceMessage>,
    progress: impl Fn(f32),
) -> Result<String, String> {
    send(&transport, "device.edition.restore")?;
    let mut started = false;
    loop {
        let line = next_line(&mut bus, RESTORE_SILENCE, "the restore").await?;
        if let Some(percent) = line.strip_prefix("EDITION RESTORE ") {
            if let Some(code) = percent.strip_prefix("FAILED ") {
                return Err(format!("the box couldn't restore its company Character ({code})"));
            }
            if let Ok(percent) = percent.parse::<f32>() {
                started = true;
                progress(percent / 100.0);
            }
            continue;
        }
        match line.as_str() {
            "ERROR no_edition" => return Err("the box has no edition".to_owned()),
            "ERROR busy" => return Err("the box is writing something else".to_owned()),
            _ => {}
        }
        if started && let Some(voice) = line.strip_prefix("VOICES ") {
            return Ok(voice.to_owned());
        }
    }
}

/// `device.edition.preview`: the box plays the company Character's line.
pub fn preview(transport: &Arc<dyn Transport>) -> Result<(), String> {
    send(transport, "device.edition.preview")
}

fn send(transport: &Arc<dyn Transport>, event: &str) -> Result<(), String> {
    let line = Event::named(event).to_ndjson().map_err(|error| error.to_string())?;
    transport.send(line).map_err(|error| format!("{error:?}"))
}

async fn next_line(bus: &mut broadcast::Receiver<DeviceMessage>, silence: Duration, what: &str) -> Result<String, String> {
    loop {
        match tokio::time::timeout(silence, bus.recv()).await {
            Ok(Ok(DeviceMessage::Line(line))) => return Ok(line),
            Ok(Ok(DeviceMessage::Disconnected)) => return Err("link lost".to_owned()),
            Ok(Ok(_)) => {}
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => return Err(format!("device messages backed up, {what} lost")),
            Ok(Err(broadcast::error::RecvError::Closed)) => return Err("device message channel closed".to_owned()),
            Err(_) => return Err(format!("timed out waiting for {what}")),
        }
    }
}
