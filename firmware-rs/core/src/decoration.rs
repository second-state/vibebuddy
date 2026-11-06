//! Decorations (ADR-0022): a seasonal layer drawn over whichever box screens the box uses, after the screen and
//! before the dimming, the menu and the tap outline. Only things that don't depend on the layout, so one drawing
//! works on every set of screens. Each one is named for its holiday: Thanksgiving is bunting across the top, a pile
//! of leaves in the bottom left corner, a sheaf of wheat in the bottom right, and falling leaves and acorns.
//! Christmas is strings of lights hanging in swags across the top, pine boughs with baubles in the bottom left
//! corner, a small tree in the bottom right, and snow falling.
//!
//! Every flake, leaf and bulb is a function of time alone, so a frame drawn at any moment is right and the
//! screens' own animation pace doesn't matter. Nothing is drawn in the top rows, where both sets show the state
//! colour, and nothing uses a state colour. A flake, leaf, acorn or bulb that would touch text, an edge or an icon
//! isn't drawn in that frame, so it seems to pass behind it and the text stays whole, on any layout. The parts in
//! the corners are drawn a pixel at a time on their corner's ground only, so text and cards sit on top of them with
//! a pixel of ground around them.

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
/// Thanksgiving's parts that stay put. Bunting hangs in swags across the top like the Christmas lights: a pennant
/// every `PENNANT_SPACING` pixels on a twine, `PENNANT_WIDTH` wide and `PENNANT_HEIGHT` long, in the leaves' colours.
const TWINE: u16 = 0x8b68;
const PENNANT_SPACING: i32 = 20;
const PENNANT_WIDTH: i32 = 11;
const PENNANT_HEIGHT: i32 = 10;
const MAX_PENNANTS: usize = 16;
/// A pile of fallen leaves in the bottom left corner, `PILE_WIDTH` wide and up to `PILE_HEIGHT` high, with a few
/// whole leaves on it and two acorns at its foot.
const PILE_WIDTH: i32 = 84;
const PILE_HEIGHT: i32 = 26;
const PILE_LEAVES: [(i32, i32); 6] = [(6, 20), (22, 22), (40, 18), (56, 12), (14, 10), (34, 8)];
const PILE_ACORNS: [i32; 2] = [64, 76];
/// A sheaf of wheat in the bottom right corner: `STALKS` stalks tied together, fanning out above the band into ears
/// of grain and splaying a little below it. Golden browns and ochre, dark enough to show on the needs input yellow.
const STALKS: i32 = 9;
const SHEAF_INSET: i32 = 30;
const SHEAF_HEIGHT: i32 = 56;
const SHEAF_BAND: i32 = 16;
const EAR: i32 = 12;
const STALK: u16 = 0x9386;
const GRAIN: u16 = 0xab45;

/// Bulbs: mostly warm white and gold, every fourth pink or violet. No red, green, yellow, orange or blue, which
/// the screens use for state.
const BULB_COLORS: [u16; 4] = [0xff9a, 0xf60a, 0xff9a, 0xf60a];
const BULB_ACCENTS: [u16; 2] = [0xf419, 0xac5f];
/// The light strings hang in swags from just under the state colour: a hook every `SWAG_PERIOD` pixels, sagging
/// `SWAG_SAG` between them, on a wire of light pine.
const SWAG_TOP: i32 = TOP + 2;
const SWAG_PERIOD: i32 = 64;
const SWAG_SAG: i32 = 10;
/// A bulb every this many pixels across, hanging from a socket just under the wire: `BULB` is its shape, 6 wide.
const BULB_SPACING: i32 = 20;
const BULB_SIZE: i32 = 6;
const BULB: [u8; 7] = [0b011110, 0b111111, 0b111111, 0b111111, 0b111111, 0b011110, 0b001100];
/// More than there are bulbs across the screen.
const MAX_BULBS: usize = 24;
/// Each bulb steps through this many twinkle phases, one every `TWINKLE_MS`: dark in one, dim in one, lit in the rest.
const TWINKLE_PHASES: u64 = 8;
const TWINKLE_MS: u64 = 350;
/// Pine boughs in the two bottom corners: needles from dark to light pine, far from the done green, on a dark
/// brown twig. The lighter two are the upper side of a bough, where the light falls.
const NEEDLES: [u16; 4] = [0x1a25, 0x2b27, 0x4c4a, 0x5c6b];
const TWIG: u16 = 0x4163;
/// The boughs of the bottom left corner, each from just off the corner to (x, y), with y up from the bottom edge.
/// The right corner is the same, mirrored.
const BOUGHS: [(i32, i32); 8] = [(70, 4), (58, 18), (40, 10), (48, 34), (26, 26), (30, 48), (10, 40), (6, 58)];
/// The longest needles, at the base of a bough; they get shorter towards its tip.
const NEEDLE_LENGTH: i32 = 6;
/// Baubles hung in each corner's boughs, 7 × 7 and round, where (x, y) is their bottom left from the bottom left
/// corner.
const BAUBLES: [(i32, i32); 3] = [(18, 12), (40, 22), (8, 34)];
const BAUBLE: [u8; 7] = [0b0011100, 0b0111110, 0b1111111, 0b1111111, 0b1111111, 0b0111110, 0b0011100];
const BAUBLE_SIZE: i32 = 7;
const BAUBLE_CAP: u16 = 0x8c71;
/// A small Christmas tree in the bottom right corner: three tiers of pine over a brown trunk, a warm white star on
/// top and bulbs on it. Its foliage is `TREE_FOLIAGE` rows high, its centre `TREE_INSET` from the right edge.
const TREE_TIER: i32 = 13;
const TREE_FOLIAGE: i32 = 3 * TREE_TIER;
const TREE_TRUNK: i32 = 5;
const TREE_INSET: i32 = 28;
const TREE_STAR: i32 = 7;
const TRUNK: u16 = 0x6243;
/// Where the tree's bulbs are, from the top of the foliage and its centre line.
const TREE_BULBS: [(i32, i32); 9] = [(9, -2), (17, 4), (21, -6), (23, 1), (28, 8), (30, -9), (35, 2), (34, 13), (36, -14)];

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

/// How far down the light string is at `x`: at the hooks it is just under the state colour, and it sags between.
fn swag_y(x: i32) -> i32 {
    let u = 2 * (x.rem_euclid(SWAG_PERIOD)) - SWAG_PERIOD;
    SWAG_TOP + SWAG_SAG - SWAG_SAG * u * u / (SWAG_PERIOD * SWAG_PERIOD)
}

/// The centres of the bulbs, each just under the wire.
fn bulbs() -> impl Iterator<Item = (i32, i32)> {
    (0..).map(|step| BULB_SPACING / 2 + step * BULB_SPACING).take_while(|x| *x < WIDTH - BULB_SIZE).map(|x| (x, swag_y(x) + 3))
}

/// The light string's wire, a pixel wide, left out where it would touch anything but plain ground.
fn wire(canvas: &mut Canvas, color: u16) {
    for x in 0..WIDTH {
        let y = swag_y(x);
        if plain(canvas, x, y, 1, 1) {
            dot(canvas, x, y, 1, 1, color);
        }
    }
}

/// The colours of the parts that stay put in the corners: the pine and the tree, the leaf pile and the wheat.
fn ours(color: u16) -> bool {
    color == TWIG
        || color == TRUNK
        || color == STALK
        || color == GRAIN
        || color == ACORN_NUT_COLOR
        || NEEDLES.contains(&color)
        || LEAF_COLORS.iter().any(|leaf| *leaf == color || half_bright(*leaf) == color)
}

/// One pixel of a part in a corner, drawn only where it and the pixels around it are the corner's `ground` or such a
/// part already. So the part grows behind text, cards and icons, and keeps a pixel of ground around them: text on
/// the tree stays whole and readable, and a part never fills the inside of a card or a key cap.
fn part_dot(canvas: &mut Canvas, ground: u16, x: i32, y: i32, color: u16) {
    if !(0..WIDTH).contains(&x) || !(TOP..HEIGHT).contains(&y) {
        return;
    }
    if on_ground(canvas, ground, x, y, 1, 1) {
        dot(canvas, x, y, 1, 1, color);
    }
}

/// Whether the box from (x, y), `width` by `height`, and a pixel around it, is all `ground` or a corner's part.
fn on_ground(canvas: &Canvas, ground: u16, x: i32, y: i32, width: i32, height: i32) -> bool {
    for row in (y - 1).max(TOP)..(y + height + 1).min(HEIGHT) {
        for column in (x - 1).max(0)..(x + width + 1).min(WIDTH) {
            let color = canvas.pixel((row * WIDTH + column) as usize);
            if color != ground && !ours(color) {
                return false;
            }
        }
    }
    true
}

/// A straight line of a corner's part from (x0, y0) to (x1, y1).
fn part_line(canvas: &mut Canvas, ground: u16, (x0, y0): (i32, i32), (x1, y1): (i32, i32), color: u16) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    for step in 0..=steps {
        part_dot(canvas, ground, x0 + (x1 - x0) * step / steps, y0 + (y1 - y0) * step / steps, color);
    }
}

/// The square root, rounded down, for a bough's length.
fn root(value: i32) -> i32 {
    let mut root = 0;
    while (root + 1) * (root + 1) <= value {
        root += 1;
    }
    root
}

/// The boughs and baubles of the bottom left corner, on its `ground`.
fn corner(canvas: &mut Canvas, ground: u16) {
    let place = |x: i32, y: i32| (x, HEIGHT - 1 - y);
    let baubles = [BULB_COLORS[1], BULB_ACCENTS[1], BULB_ACCENTS[0]];
    let origin = (-3, -2);
    for (index, (end_x, end_y)) in BOUGHS.into_iter().enumerate() {
        let (dx, dy) = (end_x - origin.0, end_y - origin.1);
        let length = root(dx * dx + dy * dy);
        // Along the bough and across it, in 1/256ths of a pixel.
        let (ux, uy) = (dx * 256 / length, dy * 256 / length);
        part_line(canvas, ground, place(origin.0, origin.1), place(end_x, end_y), TWIG);
        for along in 0..length {
            let (x, y) = (origin.0 + dx * along / length, origin.1 + dy * along / length);
            let reach = 3 + (NEEDLE_LENGTH - 3) * (length - along) / length;
            for side in [-1, 1] {
                let bits = mix(index as u32 * 512 + along as u32 * 2 + (side + 1) as u32 / 2, 0x917e);
                if bits.is_multiple_of(5) {
                    continue;
                }
                // Swept towards the tip, as pine needles grow.
                let tip_x = x + (ux * 6 / 10 - side * uy) * reach / 256;
                let tip_y = y + (uy * 6 / 10 + side * ux) * reach / 256;
                // Lighter on the upper side, where the light falls.
                let shade = if side * ux > 0 { 2 + (bits >> 3) as usize % 2 } else { (bits >> 3) as usize % 2 };
                part_line(canvas, ground, place(x, y), place(tip_x, tip_y), NEEDLES[shade]);
            }
        }
    }
    for ((x, y), color) in BAUBLES.into_iter().zip(baubles) {
        let (left, top) = place(x, y + BAUBLE_SIZE);
        let left = left.min(place(x + BAUBLE_SIZE - 1, y).0);
        if !on_ground(canvas, ground, left, top - 2, BAUBLE_SIZE, BAUBLE_SIZE + 2) {
            continue;
        }
        // The cap, a shine at the upper left, and the ball.
        dot(canvas, left + 2, top - 2, 3, 2, BAUBLE_CAP);
        for (row, line) in BAUBLE.iter().enumerate() {
            for column in 0..BAUBLE_SIZE {
                if line & (1 << (BAUBLE_SIZE - 1 - column)) != 0 {
                    let shine = (row == 1 || row == 2) && column == 2;
                    dot(canvas, left + column, top + row as i32, 1, 1, if shine { 0xffff } else { color });
                }
            }
        }
    }
}

/// How far the tree's foliage reaches either side of its centre, `row` rows from its top: each tier starts narrow
/// and spreads, and each is wider than the one above.
fn tree_reach(row: i32) -> i32 {
    let tier = row / TREE_TIER;
    let local = row % TREE_TIER;
    2 + tier * 3 + local * (6 + tier * 2) / (TREE_TIER - 1)
}

/// The tree in the bottom right corner, on its `ground`.
fn tree(canvas: &mut Canvas, ground: u16, t_ms: u64) {
    let centre = WIDTH - TREE_INSET;
    let top = HEIGHT - TREE_TRUNK - TREE_FOLIAGE;
    for row in 0..TREE_FOLIAGE {
        let reach = tree_reach(row);
        for offset in -reach..=reach {
            let bits = mix((row * 64 + offset + 32) as u32, 0x7ee5);
            // Ragged at the edges, so it reads as needles.
            if offset.abs() == reach && bits & 1 == 0 {
                continue;
            }
            // Lighter on the left, where the light falls, darker on the right and along the bottom of each tier.
            let light = if row % TREE_TIER == TREE_TIER - 1 {
                0
            } else if offset < -reach / 3 {
                2
            } else if offset < reach / 3 {
                1
            } else {
                0
            };
            let shade = (light + (bits >> 3) as usize % 2).min(NEEDLES.len() - 1);
            part_dot(canvas, ground, centre + offset, top + row, NEEDLES[shade]);
        }
    }
    for row in 0..TREE_TRUNK {
        part_line(canvas, ground, (centre - 2, top + TREE_FOLIAGE + row), (centre + 2, top + TREE_FOLIAGE + row), TRUNK);
    }
    // The star, on the top: a plus with a fuller middle and a gold heart. It and the bulbs are left out where they
    // would touch anything but the ground and the tree.
    let star = top - TREE_STAR;
    let middle = star + TREE_STAR / 2;
    if on_ground(canvas, ground, centre - TREE_STAR / 2, star, TREE_STAR, TREE_STAR) {
        dot(canvas, centre, star, 1, TREE_STAR, BULB_COLORS[0]);
        dot(canvas, centre - TREE_STAR / 2, middle, TREE_STAR, 1, BULB_COLORS[0]);
        dot(canvas, centre - 1, middle - 1, 3, 3, BULB_COLORS[0]);
        dot(canvas, centre, middle, 1, 1, BULB_COLORS[1]);
    }
    for (index, (row, offset)) in TREE_BULBS.into_iter().enumerate() {
        let lit = [BULB_COLORS[1], BULB_ACCENTS[0], BULB_COLORS[0], BULB_ACCENTS[1]][index % 4];
        let color = if (t_ms / TWINKLE_MS + mix(index as u32, 0x7b01) as u64).is_multiple_of(3) { half_bright(lit) } else { lit };
        if on_ground(canvas, ground, centre + offset, top + row, 2, 2) {
            dot(canvas, centre + offset, top + row, 2, 2, color);
        }
    }
}

/// The pile of leaves in the bottom left corner, on its `ground`: a mound of leaf-sized patches of the leaves'
/// colours, lighter towards the top, with whole leaves lying on it and acorns at its foot.
fn pile(canvas: &mut Canvas, ground: u16) {
    for x in 0..PILE_WIDTH {
        // A quarter ellipse from the corner, ragged along the top.
        let across = PILE_WIDTH * PILE_WIDTH - x * x;
        let height = PILE_HEIGHT * root(across) / PILE_WIDTH - (mix(x as u32, 0x9113) % 3) as i32;
        for up in 0..height.max(0) {
            let patch = mix(((x / 4) * 64 + (up + x / 3) / 3) as u32, 0x1eaf);
            let color = LEAF_COLORS[(patch >> 4) as usize % LEAF_COLORS.len()];
            // Shade the inside of the pile, so the patches read as leaves on leaves.
            let color = if up < height - 4 && patch & 3 == 0 { half_bright(color) } else { color };
            part_dot(canvas, ground, x, HEIGHT - 1 - up, color);
        }
    }
    for (index, (x, y)) in PILE_LEAVES.into_iter().enumerate() {
        let color = LEAF_COLORS[(index + 1) % LEAF_COLORS.len()];
        for (row, line) in LEAF_FLAT.iter().enumerate() {
            for column in 0..LEAF_WIDTH {
                if line & (1 << (LEAF_WIDTH - 1 - column)) != 0 {
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        part_dot(canvas, ground, x + column * 2 + dx, HEIGHT - 1 - y + row as i32 * 2 + dy, color);
                    }
                }
            }
        }
    }
    let acorn_height = (ACORN_CAP.len() + ACORN_NUT.len()) as i32 * LEAF_SCALE;
    for x in PILE_ACORNS {
        let top = HEIGHT - 1 - acorn_height;
        if on_ground(canvas, ground, x, top, ACORN_WIDTH * LEAF_SCALE, acorn_height) {
            shape(canvas, x, top, ACORN_WIDTH, &ACORN_CAP, ACORN_CAP_COLOR);
            shape(canvas, x, top + ACORN_CAP.len() as i32 * LEAF_SCALE, ACORN_WIDTH, &ACORN_NUT, ACORN_NUT_COLOR);
        }
    }
}

/// The sheaf of wheat in the bottom right corner, on its `ground`.
fn sheaf(canvas: &mut Canvas, ground: u16) {
    let centre = WIDTH - SHEAF_INSET;
    let band = HEIGHT - 1 - SHEAF_BAND;
    let top = HEIGHT - 1 - SHEAF_HEIGHT;
    for stalk in -(STALKS / 2)..=STALKS / 2 {
        let foot = (centre + stalk * 3, HEIGHT - 1);
        let waist = (centre + stalk / 2, band);
        let head = (centre + stalk * 4, top + stalk.abs() * 3);
        part_line(canvas, ground, foot, waist, STALK);
        part_line(canvas, ground, waist, head, STALK);
        // The ear: grains in pairs either side of the top of the stalk, and a few awns past its tip.
        let (dx, dy) = (head.0 - waist.0, head.1 - waist.1);
        let length = root(dx * dx + dy * dy).max(1);
        for along in (0..EAR).step_by(2) {
            let (x, y) = (head.0 - dx * along / length, head.1 - dy * along / length);
            let color = if along % 4 == 0 { GRAIN } else { LEAF_COLORS[2] };
            part_line(canvas, ground, (x - 1, y + 1), (x + 1, y + 1), color);
            part_dot(canvas, ground, x, y, color);
        }
        part_line(canvas, ground, head, (head.0 + dx * 4 / length, head.1 + dy * 4 / length), STALK);
    }
    // The band that ties them.
    part_line(canvas, ground, (centre - 4, band), (centre + 4, band), TWIG);
    part_line(canvas, ground, (centre - 4, band + 1), (centre + 4, band + 1), TWIG);
}

/// The middles of the pennants along the twine.
fn pennants() -> impl Iterator<Item = (i32, i32)> {
    (0..)
        .map(|step| PENNANT_SPACING / 2 + step * PENNANT_SPACING)
        .take_while(|x| *x < WIDTH - PENNANT_WIDTH / 2)
        .map(|x| (x, swag_y(x) + 1))
}

/// Thanksgiving's parts that stay put: the bunting across the top, the leaf pile and the sheaf of wheat.
fn harvest(canvas: &mut Canvas) {
    let mut room = [false; MAX_PENNANTS];
    for (index, (x, y)) in pennants().enumerate().take(MAX_PENNANTS) {
        room[index] = plain(canvas, x - PENNANT_WIDTH / 2, y, PENNANT_WIDTH, PENNANT_HEIGHT);
    }
    let left_ground = canvas.pixel(((HEIGHT - 2) * WIDTH + 1) as usize);
    let right_ground = canvas.pixel(((HEIGHT - 2) * WIDTH + WIDTH - 2) as usize);
    pile(canvas, left_ground);
    sheaf(canvas, right_ground);
    wire(canvas, TWINE);
    for (index, (x, y)) in pennants().enumerate().take(MAX_PENNANTS) {
        if !room[index] {
            continue;
        }
        let color = LEAF_COLORS[index % LEAF_COLORS.len()];
        // A triangle pointing down, with its top row a shade darker where it folds over the twine.
        for row in 0..PENNANT_HEIGHT {
            let half = (PENNANT_WIDTH / 2) * (PENNANT_HEIGHT - row) / PENNANT_HEIGHT;
            dot(canvas, x - half, y + row, 2 * half + 1, 1, if row == 0 { half_bright(color) } else { color });
        }
    }
}

fn lights(canvas: &mut Canvas, t_ms: u64) {
    // Which bulbs have room, judged on the screen before anything goes on.
    let mut room = [false; MAX_BULBS];
    for (index, (x, y)) in bulbs().enumerate().take(MAX_BULBS) {
        room[index] = plain(canvas, x - BULB_SIZE / 2, y - 2, BULB_SIZE, BULB.len() as i32 + 2);
    }
    // Each corner's ground is the screen's colour a pixel in from it.
    let left_ground = canvas.pixel(((HEIGHT - 2) * WIDTH + 1) as usize);
    let right_ground = canvas.pixel(((HEIGHT - 2) * WIDTH + WIDTH - 2) as usize);
    corner(canvas, left_ground);
    tree(canvas, right_ground, t_ms);
    wire(canvas, NEEDLES[2]);
    for (index, (x, y)) in bulbs().enumerate().take(MAX_BULBS) {
        if !room[index] {
            continue;
        }
        let lit = if index % 4 == 3 { BULB_ACCENTS[(index / 4) % BULB_ACCENTS.len()] } else { BULB_COLORS[index % BULB_COLORS.len()] };
        // Off is a dark bulb still on the string; dim is half way.
        let color = match (t_ms / TWINKLE_MS + mix(index as u32, 0x11e5) as u64) % TWINKLE_PHASES {
            0 => half_bright(half_bright(lit)),
            1 => half_bright(lit),
            _ => lit,
        };
        // `y` is the bulb's top: the socket above it, then the bulb with a shine while it is lit.
        let left = x - BULB_SIZE / 2;
        dot(canvas, left + 2, y - 2, 2, 2, BAUBLE_CAP);
        for (row, line) in BULB.iter().enumerate() {
            for column in 0..BULB_SIZE {
                if line & (1 << (BULB_SIZE - 1 - column)) != 0 {
                    let shine = row == 1 && column == 1 && color == lit;
                    dot(canvas, left + column, y + row as i32, 1, 1, if shine { 0xffff } else { color });
                }
            }
        }
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
        // The parts that stay put first, so the leaves fall behind them.
        Decoration::Thanksgiving => {
            harvest(canvas);
            leaves(canvas, t_ms);
        }
        // The lights and boughs first, so the snow falls behind them: a flake only lands on plain ground.
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
    fn christmas_has_lights_boughs_and_snow_and_thanksgiving_has_bunting_wheat_and_acorns() {
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
        assert!(has(&christmas, NEEDLES[0], 0) || has(&christmas, NEEDLES[1], 0), "the boughs are there");
        assert!(has(&christmas, BULB_COLORS[0], 0) || has(&christmas, BULB_COLORS[1], 0), "and the lights");
        assert!(has(&christmas, SNOW, 0) || has(&christmas, SNOW_SHADE, 0), "and the snow");
        let thanksgiving = drawn(Decoration::Thanksgiving, 61_000);
        assert!(has(&thanksgiving, ACORN_CAP_COLOR, 0), "there are acorns");
        assert!(has(&thanksgiving, TWINE, 0), "and the bunting's twine");
        assert!(has(&thanksgiving, STALK, 0), "and the wheat");
    }

    #[test]
    fn a_long_uptime_does_not_overflow() {
        drawn(Decoration::Christmas, u32::MAX);
        drawn(Decoration::Thanksgiving, u32::MAX - 7);
    }
}
