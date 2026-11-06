//! Decorations (ADR-0022): a seasonal layer drawn over whichever box screens the box uses, after the screen and
//! before the dimming, the menu and the tap outline. Only things that don't depend on the layout, so one drawing
//! works on every set of screens. Each one is named for its holiday: Thanksgiving is falling leaves with an acorn
//! here and there, and Christmas is lights on a pine garland down the sides and along the bottom with snow
//! falling inside it. The garland leaves the top edge alone, where it would sit against the state colour.
//!
//! Every flake, leaf and bulb is a function of time alone, so a frame drawn at any moment is right and the
//! screens' own animation pace doesn't matter. Nothing is drawn in the top rows, where both sets show the state
//! colour, and nothing uses a state colour. A flake, leaf, acorn or bulb that would touch text, an edge or an icon isn't
//! drawn in that frame, so it seems to pass behind it and the text stays whole, on any layout.

use crate::canvas::{Canvas, HEIGHT, WIDTH, half_bright};

/// Which decoration the box draws: none, the default, or one of the holiday ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Decoration {
    #[default]
    Off,
    Thanksgiving,
    Christmas,
}

impl Decoration {
    pub fn name(self) -> &'static [u8] {
        match self {
            Decoration::Off => b"OFF",
            Decoration::Thanksgiving => b"THANKSGIVING",
            Decoration::Christmas => b"CHRISTMAS",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "off" => Some(Decoration::Off),
            "thanksgiving" => Some(Decoration::Thanksgiving),
            "christmas" => Some(Decoration::Christmas),
            _ => None,
        }
    }

    /// The number the settings record keeps; anything else, an erased word included, reads as off.
    pub fn code(self) -> u32 {
        match self {
            Decoration::Off => 0,
            Decoration::Thanksgiving => 1,
            Decoration::Christmas => 2,
        }
    }

    pub fn from_code(code: u32) -> Self {
        match code {
            1 => Decoration::Thanksgiving,
            2 => Decoration::Christmas,
            _ => Decoration::Off,
        }
    }
}

/// The rows the state colour takes at the top of both sets of screens (4 on the robot's, 3 on korekore's).
pub const TOP: i32 = 4;

/// How often a moving decoration is redrawn when the screens wouldn't redraw anyway.
pub const FRAME_MS: u32 = 150;

const SNOWFLAKES: u32 = 36;
const LEAVES: u32 = 12;

const SNOW: u16 = 0xf7bf;
const SNOW_SHADE: u16 = 0xbe5c;
/// Rust, brown, ochre and maroon: autumn, and none of them the working orange, the input yellow or the failed red.
const LEAF_COLORS: [u16; 4] = [0xb285, 0x8ac6, 0xcca7, 0x7965];

/// Leaf shapes, 7 wide and 5 high, a bit a pixel from the left: flat, then turned on its edge as it tumbles.
/// Each bit is drawn as a square of [`LEAF_SCALE`] pixels.
const LEAF_WIDTH: i32 = 7;
const LEAF_SCALE: i32 = 2;
const LEAF_FLAT: [u8; 5] = [0b0011000, 0b0111100, 0b1111110, 0b0111111, 0b0000110];
const LEAF_EDGE: [u8; 5] = [0b0001000, 0b0011100, 0b0011100, 0b0011100, 0b0001000];

/// Acorns among the leaves: fewer, heavier, so they fall faster and sway less, and don't tumble. 5 wide and 6 high
/// at [`LEAF_SCALE`]: the stem and the cap in a dark brown, the nut under it in tan.
const ACORNS: u32 = 4;
const ACORN_WIDTH: i32 = 5;
const ACORN_CAP: [u8; 3] = [0b00100, 0b01110, 0b11111];
const ACORN_NUT: [u8; 3] = [0b01110, 0b01110, 0b00100];
const ACORN_CAP_COLOR: u16 = 0x59e3;
const ACORN_NUT_COLOR: u16 = 0xd54d;

/// Bulbs: mostly warm white and gold, every fourth pink or violet. No red, green, yellow, orange or blue, which
/// the screens use for state.
const BULB_COLORS: [u16; 4] = [0xff9a, 0xf60a, 0xff9a, 0xf60a];
const BULB_ACCENTS: [u16; 2] = [0xf419, 0xac5f];
/// The pine garland the bulbs hang on: a band this deep along each edge (under the state colour at the top), its
/// needles dark, mid and light pine, its outer and inner rows ragged. Pine, far from the done green.
const GARLAND: i32 = 7;
const NEEDLES: [u16; 3] = [0x1a25, 0x2b27, 0x4c4a];
/// The bulbs hang along the middle of the garland.
const WIRE_INSET: i32 = GARLAND / 2;
/// A bulb every this many pixels along the garland, 4 × 4 with its corners off, centred on it.
const BULB_SPACING: i32 = 10;
const BULB_SIZE: i32 = 4;
/// More than there are bulbs around the screen.
const MAX_BULBS: usize = 120;
/// Each bulb steps through this many twinkle phases, one every `TWINKLE_MS`: dark in one, dim in one, lit in the rest.
const TWINKLE_PHASES: u64 = 8;
const TWINKLE_MS: u64 = 350;

/// A spread of bits from a flake's number, so flakes don't line up.
fn mix(index: u32, salt: u32) -> u32 {
    let mut value = index.wrapping_mul(0x9e37_79b9) ^ salt;
    value ^= value >> 15;
    value = value.wrapping_mul(0x2c1b_3c6d);
    value ^= value >> 12;
    value = value.wrapping_mul(0x297a_2d39);
    value ^ (value >> 15)
}

/// A triangle wave from -amplitude to amplitude over `period_ms`, for the sideways drift.
fn sway(t_ms: u64, period_ms: u64, phase: u32, amplitude: i32) -> i32 {
    let position = (t_ms + phase as u64 % period_ms) % period_ms;
    let half = period_ms / 2;
    let rising = if position < half { position } else { period_ms - position };
    (rising as i32 * 4 * amplitude / period_ms as i32) - amplitude
}

/// Where a falling thing is at `t_ms`: it enters above the decoration's top, falls at `speed` pixels a second
/// and starts over from the top once it is past the bottom.
fn fall(t_ms: u64, start: u32, speed: u32, size: i32) -> i32 {
    let span = (HEIGHT - TOP + size) as u64;
    let y = (start as u64 + t_ms * speed as u64 / 1000) % span;
    TOP - size + y as i32
}

/// One pixel, kept out of the state colour's rows.
fn dot(canvas: &mut Canvas, x: i32, y: i32, width: i32, height: i32, color: u16) {
    let top = y.max(TOP);
    canvas.fill_rect(x, top, width, y + height - top, color);
}

/// Whether the box from (x, y), `width` by `height`, and a pixel around it, is all one colour on the screen:
/// plain ground, not text, an edge or an icon. Off the screen counts as plain.
fn plain(canvas: &Canvas, x: i32, y: i32, width: i32, height: i32) -> bool {
    let mut ground = None;
    for row in (y - 1).max(TOP)..(y + height + 1).min(HEIGHT) {
        for column in (x - 1).max(0)..(x + width + 1).min(WIDTH) {
            let color = canvas.pixel((row * WIDTH + column) as usize);
            if *ground.get_or_insert(color) != color {
                return false;
            }
        }
    }
    true
}

/// The line the bulbs hang along, down the middle of the garland: top and bottom rows, left and right columns.
const WIRE_TOP: i32 = TOP + WIRE_INSET;
const WIRE_BOTTOM: i32 = HEIGHT - 1 - WIRE_INSET;
const WIRE_LEFT: i32 = WIRE_INSET;
const WIRE_RIGHT: i32 = WIDTH - 1 - WIRE_INSET;

/// The centres of the bulbs: down the right, along the bottom and up the left. None along the top, where they would
/// sit against the state colour.
fn bulbs() -> impl Iterator<Item = (i32, i32)> {
    let across = (0..).map(|step| WIRE_LEFT + 6 + step * BULB_SPACING).take_while(|x| *x <= WIRE_RIGHT - 6);
    let down = (1..).map(|step| WIRE_TOP + step * BULB_SPACING).take_while(|y| *y <= WIRE_BOTTOM - 6);
    down.clone().map(|y| (WIRE_RIGHT, y)).chain(across.map(|x| (x, WIRE_BOTTOM))).chain(down.map(|y| (WIRE_LEFT, y)))
}

/// One cross-section of the garland, `GARLAND` pixels from `(x, y)` one step of `(dx, dy)` at a time, from the edge
/// inwards. It is left out where anything but plain ground is under it or right beside its inner edge, so text
/// and icons near the edge stay whole.
fn garland_slice(canvas: &mut Canvas, x: i32, y: i32, dx: i32, dy: i32, along: i32) {
    let at = |step: i32| canvas.pixel(((y + dy * step) * WIDTH + x + dx * step) as usize);
    let ground = at(0);
    if (1..=GARLAND).any(|step| at(step) != ground) {
        return;
    }
    for depth in 0..GARLAND {
        let bits = mix(along as u32 * 16 + depth as u32, 0x917e);
        // Ragged at both edges, so it reads as needles rather than a stripe.
        if (depth == 0 || depth == GARLAND - 1) && bits & 1 == 0 {
            continue;
        }
        let color = NEEDLES[(bits >> 4) as usize % NEEDLES.len()];
        dot(canvas, x + dx * depth, y + dy * depth, 1, 1, color);
    }
}

/// Down both sides from just under the state colour, and along the bottom.
fn garland(canvas: &mut Canvas) {
    for x in 0..WIDTH {
        garland_slice(canvas, x, HEIGHT - 1, 0, -1, x + WIDTH);
    }
    for y in TOP..HEIGHT - GARLAND {
        garland_slice(canvas, 0, y, 1, 0, y + 2 * WIDTH);
        garland_slice(canvas, WIDTH - 1, y, -1, 0, y + 3 * WIDTH);
    }
}

fn lights(canvas: &mut Canvas, t_ms: u64) {
    // Which bulbs have room, judged on the screen before the garland goes on.
    let mut room = [false; MAX_BULBS];
    for (index, (x, y)) in bulbs().enumerate().take(MAX_BULBS) {
        room[index] = plain(canvas, x - BULB_SIZE / 2, y - BULB_SIZE / 2, BULB_SIZE, BULB_SIZE);
    }
    garland(canvas);
    for (index, (x, y)) in bulbs().enumerate().take(MAX_BULBS) {
        if !room[index] {
            continue;
        }
        let color = if index % 4 == 3 { BULB_ACCENTS[(index / 4) % BULB_ACCENTS.len()] } else { BULB_COLORS[index % BULB_COLORS.len()] };
        // Off is a dark bulb still on the garland; dim is half way.
        let color = match (t_ms / TWINKLE_MS + mix(index as u32, 0x11e5) as u64) % TWINKLE_PHASES {
            0 => half_bright(half_bright(color)),
            1 => half_bright(color),
            _ => color,
        };
        let (left, top) = (x - BULB_SIZE / 2, y - BULB_SIZE / 2);
        dot(canvas, left + 1, top, BULB_SIZE - 2, BULB_SIZE, color);
        dot(canvas, left, top + 1, BULB_SIZE, BULB_SIZE - 2, color);
    }
}

/// Draws `rows` of a pixel shape, `width` bits each, from (x, y), each bit a square of [`LEAF_SCALE`] pixels.
fn shape(canvas: &mut Canvas, x: i32, y: i32, width: i32, rows: &[u8], color: u16) {
    for (row, line) in rows.iter().enumerate() {
        for column in 0..width {
            if line & (1 << (width - 1 - column)) != 0 {
                dot(canvas, x + column * LEAF_SCALE, y + row as i32 * LEAF_SCALE, LEAF_SCALE, LEAF_SCALE, color);
            }
        }
    }
}

fn snow(canvas: &mut Canvas, t_ms: u64) {
    for index in 0..SNOWFLAKES {
        let bits = mix(index, 0x5a0e);
        // A third are bigger and fall faster, nearer the glass.
        let big = bits.is_multiple_of(3);
        let speed = if big { 22 } else { 12 } + (bits >> 4) % 8;
        let y = fall(t_ms, bits >> 8, speed, 3);
        let x = (bits >> 20) as i32 % WIDTH + sway(t_ms, 4000 + (bits % 2000) as u64, bits, 3);
        if !plain(canvas, x, y, 3, 3) {
            continue;
        }
        if big {
            dot(canvas, x + 1, y, 1, 3, SNOW);
            dot(canvas, x, y + 1, 3, 1, SNOW);
        } else {
            dot(canvas, x, y, 2, 2, if bits & 0x10 == 0 { SNOW } else { SNOW_SHADE });
        }
    }
}

fn leaves(canvas: &mut Canvas, t_ms: u64) {
    for index in 0..LEAVES {
        let bits = mix(index, 0x1eaf);
        let speed = 14 + (bits >> 4) % 10;
        let y = fall(t_ms, bits >> 8, speed, 5 * LEAF_SCALE);
        let period = 3000 + (bits % 2500) as u64;
        let x = (bits >> 20) as i32 % WIDTH + sway(t_ms, period, bits, 12);
        // It turns on its edge where the sway turns back.
        let rows = if sway(t_ms, period / 2, bits, 4).abs() > 2 { &LEAF_FLAT } else { &LEAF_EDGE };
        let color = LEAF_COLORS[(bits >> 12) as usize % LEAF_COLORS.len()];
        if plain(canvas, x, y, LEAF_WIDTH * LEAF_SCALE, rows.len() as i32 * LEAF_SCALE) {
            shape(canvas, x, y, LEAF_WIDTH, rows, color);
        }
    }
    let height = (ACORN_CAP.len() + ACORN_NUT.len()) as i32 * LEAF_SCALE;
    for index in 0..ACORNS {
        let bits = mix(index, 0xac04);
        let speed = 26 + (bits >> 4) % 8;
        let y = fall(t_ms, bits >> 8, speed, height);
        let x = (bits >> 20) as i32 % WIDTH + sway(t_ms, 5000 + (bits % 2000) as u64, bits, 2);
        if plain(canvas, x, y, ACORN_WIDTH * LEAF_SCALE, height) {
            shape(canvas, x, y, ACORN_WIDTH, &ACORN_CAP, ACORN_CAP_COLOR);
            shape(canvas, x, y + ACORN_CAP.len() as i32 * LEAF_SCALE, ACORN_WIDTH, &ACORN_NUT, ACORN_NUT_COLOR);
        }
    }
}

/// Draws `decoration` as it is at `t_ms`. Off draws nothing.
pub fn draw(canvas: &mut Canvas, decoration: Decoration, t_ms: u32) {
    let t_ms = t_ms as u64;
    match decoration {
        Decoration::Off => {}
        Decoration::Thanksgiving => leaves(canvas, t_ms),
        // The garland first, so the snow falls behind it: a flake only lands on plain ground.
        Decoration::Christmas => {
            lights(canvas, t_ms);
            snow(canvas, t_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::FRAME_BYTES;
    use std::vec;
    use std::vec::Vec;

    fn drawn(decoration: Decoration, t_ms: u32) -> Vec<u8> {
        let mut frame = vec![0u8; FRAME_BYTES];
        draw(&mut Canvas::new(&mut frame), decoration, t_ms);
        frame
    }

    #[test]
    fn names_and_codes_go_both_ways() {
        for decoration in [Decoration::Off, Decoration::Thanksgiving, Decoration::Christmas] {
            let name = core::str::from_utf8(decoration.name()).unwrap().to_ascii_lowercase();
            assert_eq!(Decoration::parse(&name), Some(decoration));
            assert_eq!(Decoration::from_code(decoration.code()), decoration);
        }
        assert_eq!(Decoration::parse("tinsel"), None);
        assert_eq!(Decoration::from_code(0xFFFF_FFFF), Decoration::Off, "an erased word is off");
    }

    #[test]
    fn off_draws_nothing_and_the_others_move() {
        assert!(drawn(Decoration::Off, 1234).iter().all(|byte| *byte == 0));
        for decoration in [Decoration::Thanksgiving, Decoration::Christmas] {
            let first = drawn(decoration, 1000);
            assert!(first.iter().any(|byte| *byte != 0));
            assert_ne!(first, drawn(decoration, 1000 + FRAME_MS), "{decoration:?} moves from one frame to the next");
            assert_eq!(first, drawn(decoration, 1000), "{decoration:?} is the same at the same time");
        }
    }

    #[test]
    fn the_state_colour_rows_stay_untouched_at_every_moment() {
        let top_bytes = (TOP * WIDTH * 2) as usize;
        for decoration in [Decoration::Thanksgiving, Decoration::Christmas] {
            for t_ms in (0..120_000).step_by(37) {
                assert!(drawn(decoration, t_ms)[..top_bytes].iter().all(|byte| *byte == 0), "{decoration:?} at {t_ms} ms");
            }
        }
    }

    #[test]
    fn christmas_has_snow_inside_the_garland_and_thanksgiving_has_acorns_among_the_leaves() {
        // Whether `color` is anywhere at least `inside` pixels in from the edges, under the state colour's rows.
        let has = |frame: &[u8], color: u16, inside: i32| {
            (TOP + inside..HEIGHT - inside).any(|y| {
                (inside..WIDTH - inside).any(|x| {
                    let at = ((y * WIDTH + x) * 2) as usize;
                    u16::from_be_bytes([frame[at], frame[at + 1]]) == color
                })
            })
        };
        let christmas = drawn(Decoration::Christmas, 61_000);
        assert!(has(&christmas, NEEDLES[0], 0) || has(&christmas, NEEDLES[1], 0), "the garland is there");
        assert!(has(&christmas, SNOW, GARLAND) || has(&christmas, SNOW_SHADE, GARLAND), "and snow falls inside it");
        let thanksgiving = drawn(Decoration::Thanksgiving, 61_000);
        assert!(has(&thanksgiving, ACORN_CAP_COLOR, 0), "an acorn falls among the leaves");
    }

    #[test]
    fn a_long_uptime_does_not_overflow() {
        drawn(Decoration::Christmas, u32::MAX);
        drawn(Decoration::Thanksgiving, u32::MAX - 7);
    }
}
