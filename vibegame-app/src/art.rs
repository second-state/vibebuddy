//! Colours, the robot, the Agent icons and text: what every screen is drawn with. The VibeGame draws at 240×160
//! with a 256-colour palette; QingOS shows it three times as big on the 720×480 screen.

use qapp::full::{Frame, W};
use vibegame::FONT;

// The palette, by name. The colours are korekore's box colours (`firmware-rs/core/src/korekore/theme.rs`) and the
// robot's (`characters/robot/face.png`).
/// Not slot 0: in a sprite, 0 is transparent, and big text is drawn as a sprite.
pub const INK: u8 = 16;
pub const CARD: u8 = 1;
pub const FACE: u8 = 2;
pub const TEXT: u8 = 3;
pub const MUTED: u8 = 4;
pub const SUN: u8 = 5;
pub const SKY: u8 = 6;
pub const MINT: u8 = 7;
pub const PEACH: u8 = 8;
pub const LAVENDER: u8 = 9;
pub const PINK: u8 = 10;
pub const BLUSH: u8 = 11;
pub const ROBOT_EYE: u8 = 12;
pub const ROBOT_LIGHT: u8 = 13;
pub const ROBOT_BODY: u8 = 14;
pub const ROBOT_SCREEN: u8 = 15;
/// The Agent icons' colours start here, one slot each.
const ICON_BASE: u8 = 32;

const COLOURS: [[u8; 3]; 17] = [
    [0x1C, 0x17, 0x30],
    [0x2A, 0x23, 0x45],
    [0x3B, 0x2F, 0x63],
    [0xFF, 0xF3, 0xDD],
    [0xA9, 0xA2, 0xC2],
    [0xFF, 0xD2, 0x3F],
    [0x7C, 0xC8, 0xFF],
    [0x6E, 0xE7, 0xB7],
    [0xFF, 0x7A, 0x7A],
    [0xB9, 0xA8, 0xFF],
    [0xF0, 0x58, 0x7F],
    [0xFF, 0x9E, 0xB8],
    [41, 190, 255],
    [123, 202, 255],
    [57, 145, 255],
    [16, 20, 32],
    [0x1C, 0x17, 0x30],
];

mod pixels {
    // The box's Agent icons: 16×16, RGB565, 0 for transparent.
    include!("../../firmware-rs/core/src/agent_icon/pixels.rs");
}

use vibegame_core::duty::Agent;

fn icon_pixels(agent: Agent) -> &'static [u16; 256] {
    match agent {
        Agent::ClaudeCode => &pixels::CLAUDE,
        Agent::Codex => &pixels::CODEX,
        Agent::OpenCode => &pixels::OPENCODE,
        Agent::Copilot => &pixels::COPILOT,
        Agent::Pi => &pixels::PI,
        Agent::Ci => &pixels::CI,
    }
}

const AGENTS: [Agent; 6] = [Agent::ClaudeCode, Agent::Codex, Agent::OpenCode, Agent::Copilot, Agent::Pi, Agent::Ci];

fn rgb565(c: u16) -> [u8; 3] {
    let (r, g, b) = ((c >> 11) & 0x1F, (c >> 5) & 0x3F, c & 0x1F);
    [(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8]
}

impl Default for Art {
    fn default() -> Self {
        Self::new()
    }
}

/// The palette, and the icons as palette pictures. Built once.
pub struct Art {
    pub palette: [[u8; 3]; 256],
    icons: Vec<(Agent, [u8; 256])>,
}

impl Art {
    pub fn new() -> Self {
        let mut palette = [[0u8; 3]; 256];
        palette[..COLOURS.len()].copy_from_slice(&COLOURS);
        let mut used: Vec<u16> = Vec::new();
        let icons = AGENTS
            .iter()
            .map(|&agent| {
                let mut picture = [0u8; 256];
                for (at, &c) in icon_pixels(agent).iter().enumerate() {
                    if c == 0 {
                        continue;
                    }
                    let slot = used.iter().position(|&u| u == c).unwrap_or_else(|| {
                        used.push(c);
                        used.len() - 1
                    });
                    picture[at] = ICON_BASE + slot as u8;
                }
                (agent, picture)
            })
            .collect();
        for (slot, &c) in used.iter().enumerate() {
            palette[ICON_BASE as usize + slot] = rgb565(c);
        }
        Self { palette, icons }
    }

    /// The same palette with every colour turned gray and dimmed: the screen while there is no link.
    pub fn gray(&self) -> [[u8; 3]; 256] {
        self.palette.map(|[r, g, b]| {
            let luma = (r as u32 * 30 + g as u32 * 59 + b as u32 * 11) / 100;
            let v = (36 + luma * 55 / 100) as u8;
            [v, v, v.saturating_add(6)]
        })
    }

    pub fn icon(&self, frame: &mut Frame, agent: Agent, x: i32, y: i32) {
        let Some((_, picture)) = self.icons.iter().find(|(a, _)| *a == agent) else { return };
        for (at, &c) in picture.iter().enumerate() {
            if c != 0 {
                frame.set(x + (at % 16) as i32, y + (at / 16) as i32, c);
            }
        }
    }
}

/// How the robot looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Idle,
    /// Eyes left, centre or right: `look` is -1, 0 or 1.
    Working(i32),
    Waiting,
    Happy,
    Worried,
    Asleep,
    Blink,
}

/// Draws the robot in a 26×23 grid, each cell `k`×`k` pixels, with its top left at (x, y).
pub fn robot(frame: &mut Frame, x: i32, y: i32, k: i32, mood: Mood) {
    let mut cell = |cx: i32, cy: i32, w: i32, h: i32, c: u8| frame.rect(x + cx * k, y + cy * k, w * k, h * k, c);
    // Antenna, head, ears, the face's screen.
    cell(12, 0, 2, 2, ROBOT_EYE);
    cell(12, 2, 2, 1, ROBOT_BODY);
    cell(5, 3, 16, 1, ROBOT_BODY);
    cell(4, 4, 18, 10, ROBOT_BODY);
    cell(5, 14, 16, 1, ROBOT_BODY);
    cell(2, 6, 2, 5, ROBOT_LIGHT);
    cell(22, 6, 2, 5, ROBOT_LIGHT);
    cell(6, 5, 14, 8, ROBOT_LIGHT);
    cell(7, 6, 12, 6, ROBOT_SCREEN);
    // Body, arms, feet.
    cell(8, 16, 10, 5, ROBOT_BODY);
    cell(5, 16, 2, 4, ROBOT_LIGHT);
    cell(19, 16, 2, 4, ROBOT_LIGHT);
    cell(9, 21, 3, 2, ROBOT_LIGHT);
    cell(14, 21, 3, 2, ROBOT_LIGHT);
    // The face.
    let e = ROBOT_EYE;
    match mood {
        Mood::Idle | Mood::Working(_) => {
            let look = if let Mood::Working(look) = mood { look.clamp(-1, 1) } else { 0 };
            cell(9 + look, 7, 2, 2, e);
            cell(15 + look, 7, 2, 2, e);
            smile(&mut cell);
        }
        Mood::Blink => {
            cell(9, 8, 2, 1, e);
            cell(15, 8, 2, 1, e);
            smile(&mut cell);
        }
        Mood::Waiting => {
            cell(9, 7, 2, 3, e);
            cell(15, 7, 2, 3, e);
            cell(12, 10, 2, 1, e);
        }
        Mood::Happy => {
            for ex in [9, 15] {
                cell(ex - 1, 8, 1, 1, e);
                cell(ex, 7, 2, 1, e);
                cell(ex + 2, 8, 1, 1, e);
            }
            cell(11, 9, 4, 2, e);
            cell(8, 10, 2, 1, BLUSH);
            cell(16, 10, 2, 1, BLUSH);
        }
        Mood::Worried => {
            cell(9, 8, 2, 1, e);
            cell(15, 8, 2, 1, e);
            cell(17, 6, 1, 2, ROBOT_LIGHT);
            cell(11, 11, 1, 1, e);
            cell(12, 10, 2, 1, e);
            cell(14, 11, 1, 1, e);
        }
        Mood::Asleep => {
            cell(8, 9, 3, 1, e);
            cell(15, 9, 3, 1, e);
        }
    }
}

fn smile(cell: &mut impl FnMut(i32, i32, i32, i32, u8)) {
    cell(11, 10, 1, 1, ROBOT_EYE);
    cell(12, 11, 2, 1, ROBOT_EYE);
    cell(14, 10, 1, 1, ROBOT_EYE);
}

/// The width of a line of text, in pixels.
pub fn width(s: &str) -> i32 {
    FONT.width(s)
}

/// Text that fits in `max` pixels: cut, with "…" at the end when it is cut.
pub fn fit(s: &str, max: i32) -> String {
    if width(s) <= max {
        return s.to_owned();
    }
    let mut out = String::new();
    for ch in s.chars() {
        let next = format!("{out}{ch}");
        if width(&next) + width("…") > max {
            break;
        }
        out = next;
    }
    out.push('…');
    out
}

pub fn text(frame: &mut Frame, x: i32, y: i32, s: &str, c: u8) -> i32 {
    frame.text(&FONT, x, y, s, c)
}

/// Text right-aligned so it ends at `right`.
pub fn text_right(frame: &mut Frame, right: i32, y: i32, s: &str, c: u8) {
    frame.text(&FONT, right - width(s), y, s, c);
}

/// Text centred between `left` and `right`.
pub fn text_mid(frame: &mut Frame, left: i32, right: i32, y: i32, s: &str, c: u8) {
    frame.text(&FONT, left + (right - left - width(s)) / 2, y, s, c);
}

/// Text `k` times as big.
pub fn big(frame: &mut Frame, x: i32, y: i32, s: &str, c: u8, k: i32) {
    frame.blit(&FONT.sprite(s, c).scaled(k), x, y, false);
}

pub fn big_width(s: &str, k: i32) -> i32 {
    width(s) * k
}

/// Text broken into lines of at most `max` pixels: at spaces for English, anywhere for Chinese.
pub fn wrap(s: &str, max: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in s.split_inclusive(' ') {
        if width(format!("{line}{word}").trim_end()) <= max {
            line.push_str(word);
            continue;
        }
        if !line.is_empty() {
            lines.push(line.trim_end().to_owned());
            line.clear();
        }
        for ch in word.chars() {
            if width(&format!("{line}{ch}")) > max {
                lines.push(line.trim_end().to_owned());
                line.clear();
            }
            line.push(ch);
        }
    }
    if !line.trim().is_empty() {
        lines.push(line.trim_end().to_owned());
    }
    lines
}

/// A small label on a coloured pill, like the box's RUN and ASK.
pub fn pill(frame: &mut Frame, right: i32, y: i32, s: &str, fill: u8) {
    let w = width(s) + 6;
    frame.rect(right - w, y, w, 13, fill);
    text(frame, right - w + 3, y + 1, s, INK);
}

/// A key on the screen, like the box's K2: the key's name on a chip of colour `c`, then what it does in `c`.
pub fn key_hint(frame: &mut Frame, x: i32, y: i32, key: &str, what: &str, c: u8) -> i32 {
    key_hint_on(frame, x, y, key, what, c, INK)
}

/// The same with the key's name in `letter`, for a dark chip on a light screen.
pub fn key_hint_on(frame: &mut Frame, x: i32, y: i32, key: &str, what: &str, c: u8, letter: u8) -> i32 {
    let w = width(key) + 4;
    frame.rect(x, y, w, 13, c);
    text(frame, x + 2, y + 1, key, letter);
    w + 3 + text(frame, x + w + 3, y + 1, what, c)
}

/// The whole width, for centring.
pub const SCREEN_W: i32 = W as i32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_cut_and_wrapped_to_fit() {
        assert_eq!(fit("short", 100), "short");
        let cut = fit("a very long title that will not fit", 60);
        assert!(cut.ends_with('…') && width(&cut) <= 60);
        let lines = wrap("Open VibeBuddy on your computer and pick this VibeGame", 120);
        assert!(lines.len() > 1 && lines.iter().all(|l| width(l) <= 120), "{lines:?}");
        let zh = wrap("在电脑上打开 VibeBuddy，选这台掌机", 72);
        assert!(zh.iter().all(|l| width(l) <= 72), "{zh:?}");
    }

    #[test]
    fn the_icons_fit_in_the_palette() {
        let art = Art::new();
        assert!(art.palette[ICON_BASE as usize..].iter().filter(|c| **c != [0, 0, 0]).count() < 100);
    }
}
