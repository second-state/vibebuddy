//! What the korekore UI shows of the box (ADR-0021): the mini box in the sidebar and the tray's Kore. It is drawn
//! from the daemon's status, not from a screen capture, so it costs nothing over the link. The Mac derives the same
//! in `BoxPicture.swift`.

use crate::status::{Status, Today};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Ready,
    Working,
    Input,
    Pomodoro,
    Leisure,
    NoLink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Usb,
    Lan,
    Relay,
}

impl Link {
    pub fn label(self) -> &'static str {
        match self {
            Link::Usb => "USB",
            Link::Lan => "LAN",
            Link::Relay => "RELAY",
        }
    }
}

/// A task card as the box shows it: the agent's tile and the title without its prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub icon: &'static str,
    pub title: String,
    pub needs_input: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoxPicture {
    pub mood: Mood,
    pub daemon_alive: bool,
    /// How the box is linked; None without a link.
    pub link: Option<Link>,
    /// The newest three, as the box's first page shows them.
    pub tickets: Vec<Ticket>,
    pub today: Today,
    pub firmware_version: Option<String>,
}

/// The box can show this many cards on a page.
const TICKETS_SHOWN: usize = 3;

impl BoxPicture {
    /// `None` means the daemon isn't answering.
    pub fn derive(status: Option<&Status>) -> Self {
        let activities = status.map(|status| status.activities.as_slice()).unwrap_or_default();
        let device = status.map(|status| &status.device);
        let link = device.filter(|device| device.connected).map(|device| {
            if device.relay {
                Link::Relay
            } else if device.network {
                Link::Lan
            } else {
                Link::Usb
            }
        });
        let mood = match (link, device.and_then(|device| device.mode.as_deref())) {
            (None, _) => Mood::NoLink,
            (_, Some("pomodoro")) => Mood::Pomodoro,
            (_, Some("leisure")) => Mood::Leisure,
            _ if activities.iter().any(|activity| activity.state == "needs_input") => Mood::Input,
            _ if !activities.is_empty() => Mood::Working,
            _ => Mood::Ready,
        };
        let tickets = activities
            .iter()
            .take(TICKETS_SHOWN)
            .map(|activity| Ticket {
                icon: icon_for_agent(&activity.agent),
                title: card_title(&activity.title).to_owned(),
                needs_input: activity.state == "needs_input",
            })
            .collect();
        Self {
            mood,
            daemon_alive: status.is_some(),
            link,
            tickets,
            today: status.map(|status| status.today.clone()).unwrap_or_default(),
            firmware_version: device.and_then(|device| device.firmware_version.clone()),
        }
    }

    pub fn needs_you(&self) -> bool {
        self.tickets.iter().any(|ticket| ticket.needs_input)
    }
}

/// The agent's tile in docs/design/korekore/pixel/icons.json; anything else is the user's own events.
pub fn icon_for_agent(agent: &str) -> &'static str {
    match agent {
        "claude-code" => "cc",
        "codex" => "cx",
        "github-actions" => "ci",
        "opencode" => "oc",
        "copilot" => "gh",
        "pi" => "pi",
        _ => "api",
    }
}

/// The title without the prefix the daemon adds to tell agents apart (`CC:`, `CX:`…): the tile says that now.
pub fn card_title(title: &str) -> &str {
    match title.split_once(':') {
        Some((prefix, rest))
            if prefix.chars().count() <= 3 && prefix.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) =>
        {
            rest.trim_start()
        }
        _ => title,
    }
}

/// Busy time as the stat chips show it: `35m`, `4h12m`.
pub fn busy(seconds: u64) -> String {
    let minutes = seconds / 60;
    if minutes < 60 { format!("{minutes}m") } else { format!("{}h{:02}m", minutes / 60, minutes % 60) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::Activity;

    fn status(connected: bool, mode: &str, activities: &[(&str, &str, &str)]) -> Status {
        let mut status = Status::default();
        status.device.connected = connected;
        status.device.mode = Some(mode.to_owned());
        status.activities = activities
            .iter()
            .map(|(agent, title, state)| Activity {
                agent: (*agent).to_owned(),
                title: (*title).to_owned(),
                state: (*state).to_owned(),
                ..Activity::default()
            })
            .collect();
        status
    }

    #[test]
    fn the_mood_follows_the_link_the_mode_and_the_cards() {
        assert_eq!(BoxPicture::derive(None).mood, Mood::NoLink);
        assert_eq!(BoxPicture::derive(Some(&status(false, "duty", &[]))).mood, Mood::NoLink);
        assert_eq!(BoxPicture::derive(Some(&status(true, "duty", &[]))).mood, Mood::Ready);
        let working = [("codex", "CX:Fix", "working")];
        assert_eq!(BoxPicture::derive(Some(&status(true, "duty", &working))).mood, Mood::Working);
        let asking = [("codex", "CX:Fix", "working"), ("claude-code", "CC:Ask", "needs_input")];
        assert_eq!(BoxPicture::derive(Some(&status(true, "duty", &asking))).mood, Mood::Input);
        assert_eq!(BoxPicture::derive(Some(&status(true, "pomodoro", &asking))).mood, Mood::Pomodoro);
    }

    #[test]
    fn tickets_are_the_newest_three_with_tiles_and_no_prefix() {
        let four = [
            ("claude-code", "CC:Add Chinese to the box", "working"),
            ("codex", "CX:Review changes", "needs_input"),
            ("github-actions", "CI:App tests", "working"),
            ("pi", "PI:Older", "working"),
        ];
        let picture = BoxPicture::derive(Some(&status(true, "duty", &four)));
        let tickets: Vec<(&str, &str, bool)> =
            picture.tickets.iter().map(|ticket| (ticket.icon, ticket.title.as_str(), ticket.needs_input)).collect();
        assert_eq!(
            tickets,
            [("cc", "Add Chinese to the box", false), ("cx", "Review changes", true), ("ci", "App tests", false)]
        );
        assert!(picture.needs_you());
    }

    #[test]
    fn only_a_short_capital_prefix_is_dropped() {
        assert_eq!(card_title("CX:Review"), "Review");
        assert_eq!(card_title("Fix: the build"), "Fix: the build");
        assert_eq!(card_title("No prefix"), "No prefix");
        assert_eq!(busy(35 * 60), "35m");
        assert_eq!(busy(4 * 3600 + 12 * 60), "4h12m");
    }
}
