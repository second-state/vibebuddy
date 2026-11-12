//! The VibeGame's own screens, which the box has no need of: the pairing code, and looking for a computer
//! (docs/vibegame.md). One drawing for each set of screens, made of the box's own parts: its canvas, its type, its
//! colours, Kore's sprites, and the robot as the firmware's Display draws it.

use vibebuddy_firmware_core::canvas::{Canvas, FRAME_BYTES, WIDTH};
use vibebuddy_firmware_core::display::{Display, Scene, Screen, Screens};
use vibebuddy_firmware_core::korekore::Sprite;
use vibebuddy_firmware_core::korekore::glyphs::{TYPE_9, TYPE_10, TYPE_11, TYPE_14, TYPE_16, TYPE_30, WORDMARK_14};
use vibebuddy_firmware_core::korekore::sprites::{KORE_INPUT, KORE_WORKING};
use vibebuddy_firmware_core::korekore::text::{self as kt, Face};
use vibebuddy_firmware_core::korekore::theme::{BOX_CARD, BOX_GROUND, BOX_TEXT, BOX_TEXT_MUTED, KORE_PINK, SKY, SUN};
use vibebuddy_firmware_core::leisure::Leisure;
use vibebuddy_firmware_core::pomodoro::Pomodoro;

/// What the person calls the device on the screen.
pub const DEVICE: &str = "VibeGame";

/// The robot screens' colours (display.rs).
const ROBOT_BACKGROUND: u16 = 0x0841;
const ROBOT_MUTED: u16 = 0x8410;
const ROBOT_TEXT: u16 = 0xffff;
const ROBOT_READY: u16 = 0x2dff;
const ROBOT_INPUT: u16 = 0xffe0;
const ROBOT_SCREEN: u16 = 0x10a4;
/// Where the robot stands on the box's idle screen, which is copied from there.
const ROBOT_AT: (i32, i32, i32, i32) = (104, 28, 112, 128);

/// One of the VibeGame's own screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Own<'a> {
    /// Waiting to be paired: the code, and the name the computer lists it by. `back`: it has computers already, so B
    /// goes back.
    Pairing { code: &'a str, name: &'a str, back: bool },
    /// Paired, and no computer has connected since the app started.
    Looking { name: &'a str },
}

struct Lcd(Vec<u8>);

impl Screen for Lcd {
    fn frame(&mut self) -> &mut [u8] {
        &mut self.0
    }
    fn present(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn set_backlight(&mut self, _on: bool) -> Result<(), ()> {
        Ok(())
    }
}

/// Draws the VibeGame's own screens. Keeps a Display of its own on the idle screen, for the robot: it breathes and
/// blinks there as on the box.
pub struct OwnScreens {
    display: Display,
    lcd: Lcd,
    pomodoro: Pomodoro,
    leisure: Leisure,
}

impl Default for OwnScreens {
    fn default() -> Self {
        Self::new()
    }
}

/// The 5×7 font is capitals only: the robot screens write in capitals.
fn upper(text: &str) -> Vec<u8> {
    text.to_ascii_uppercase().into_bytes()
}

/// The code in two groups of three, as people read it out.
fn grouped(code: &str) -> String {
    match code.len() {
        6 => format!("{} {}", &code[..3], &code[3..]),
        _ => code.to_owned(),
    }
}

/// A rounded rectangle, as korekore's chips are drawn.
fn fill_round(canvas: &mut Canvas, x: i32, y: i32, width: i32, height: i32, radius: i32, color: u16) {
    for row in 0..height {
        let edge = row.min(height - 1 - row);
        let inset = if edge >= radius {
            0
        } else {
            let dy = f64::from(radius - edge) - 0.5;
            radius - (f64::from(radius * radius) - dy * dy).max(0.0).sqrt().round() as i32
        };
        canvas.fill_rect(x + inset, y + row, width - 2 * inset, 1, color);
    }
}

fn sprite(canvas: &mut Canvas, sprite: &Sprite, x: i32, y: i32, scale: i32, mirror: bool) {
    for row in 0..sprite.height {
        for column in 0..sprite.width {
            let source = if mirror { sprite.width - 1 - column } else { column };
            if let Some(color) = sprite.pixel(source, row) {
                canvas.fill_rect(x + i32::from(column) * scale, y + i32::from(row) * scale, scale, scale, color);
            }
        }
    }
}

/// A korekore key hint: the key on a chip, then what it does. Returns the width used.
fn kk_hint(canvas: &mut Canvas, face: &Face, x: i32, y: i32, key: &[u8], action: &[u8]) -> i32 {
    let chip = kt::width(face, key) + 8;
    fill_round(canvas, x, y, chip, face.line(), 3, BOX_TEXT);
    kt::draw(canvas, face, x + 4, y, key, BOX_GROUND, i32::MAX);
    chip + 4 + kt::draw(canvas, face, x + chip + 4, y, action, BOX_TEXT_MUTED, i32::MAX)
}

/// A robot key hint, as the box's menu draws them.
fn robot_hint(canvas: &mut Canvas, x: i32, y: i32, key: &[u8], action: &[u8]) -> i32 {
    let key_width = key.len() as i32 * 6 - 1;
    canvas.fill_rect(x, y - 2, key_width + 6, 11, ROBOT_MUTED);
    canvas.draw_text(x + 3, y, key, 1, ROBOT_SCREEN, 12);
    let action_width = action.len() as i32 * 6 - 1;
    canvas.draw_text(x + key_width + 10, y, action, 1, ROBOT_TEXT, 16);
    key_width + 10 + action_width + 14
}

fn hints(own: Own) -> Vec<(&'static [u8], &'static [u8])> {
    match own {
        Own::Pairing { back: true, .. } => vec![(b"B", b"BACK"), (b"X", b"MENU")],
        Own::Pairing { back: false, .. } => Vec::new(),
        Own::Looking { .. } => vec![(b"X", b"MENU")],
    }
}

impl OwnScreens {
    pub fn new() -> Self {
        let mut screens = Self { display: Display::new(), lcd: Lcd(vec![0; FRAME_BYTES]), pomodoro: Pomodoro::new(), leisure: Leisure::new(1, 0) };
        let scene = Scene { now_ms: 0, pomodoro: &screens.pomodoro, leisure: &screens.leisure };
        let _ = screens.display.start(&mut screens.lcd, &scene);
        screens
    }

    /// Draws `own` into `frame`, a 320 × 240 RGB565 frame like the box's.
    pub fn draw(&mut self, own: Own, screens: Screens, now: u32, frame: &mut [u8]) {
        match screens {
            Screens::Robot => self.robot(own, now, frame),
            Screens::Korekore => korekore(own, now, frame),
        }
    }

    fn robot(&mut self, own: Own, now: u32, frame: &mut [u8]) {
        let scene = Scene { now_ms: now, pomodoro: &self.pomodoro, leisure: &self.leisure };
        self.display.tick(&mut self.lcd, &scene);
        let mut canvas = Canvas::new(frame);
        canvas.fill_rect(0, 0, WIDTH, 240, ROBOT_BACKGROUND);
        let bar = if matches!(own, Own::Pairing { .. }) { ROBOT_INPUT } else { ROBOT_READY };
        canvas.fill_rect(0, 0, WIDTH, 4, bar);
        canvas.draw_text_centered(12, b"VibeBuddy", 2, ROBOT_TEXT);
        // The robot from the box's idle screen, on the left.
        let (from_x, from_y, width, height) = ROBOT_AT;
        let source = Canvas::new(&mut self.lcd.0);
        for row in 0..height {
            for column in 0..width {
                let color = source.pixel(((from_y + row) * WIDTH + from_x + column) as usize);
                canvas.fill_rect(column, 40 + row, 1, 1, color);
            }
        }
        let left = 120;
        let device = upper(DEVICE);
        match own {
            Own::Pairing { code, name, .. } => {
                canvas.draw_text(left, 46, b"PAIR", 2, ROBOT_MUTED, 16);
                canvas.draw_text(left, 68, grouped(code).as_bytes(), 4, ROBOT_INPUT, 7);
                canvas.draw_text(left, 110, b"ON YOUR COMPUTER, OPEN", 1, ROBOT_TEXT, 32);
                canvas.draw_text(left, 122, b"VIBEBUDDY AND PICK THIS", 1, ROBOT_TEXT, 32);
                let mut line = device.clone();
                line.extend_from_slice(b". THEN TYPE THE CODE.");
                canvas.draw_text(left, 134, &line, 1, ROBOT_TEXT, 32);
                kt::draw(&mut canvas, &TYPE_11, left, 154, name.as_bytes(), ROBOT_MUTED, WIDTH - left - 4);
            }
            Own::Looking { name } => {
                canvas.draw_text(left, 46, b"LOOKING FOR", 2, ROBOT_TEXT, 16);
                canvas.draw_text(left, 64, b"YOUR COMPUTER", 2, ROBOT_TEXT, 16);
                let dots = (now / 500 % 4) as usize;
                canvas.draw_text(left, 82, &b"..."[..dots.min(3)], 2, ROBOT_READY, 3);
                canvas.draw_text(left, 110, b"OPEN VIBEBUDDY ON A PAIRED", 1, ROBOT_TEXT, 32);
                canvas.draw_text(left, 122, b"COMPUTER ON THIS WI-FI.", 1, ROBOT_TEXT, 32);
                kt::draw(&mut canvas, &TYPE_11, left, 154, name.as_bytes(), ROBOT_MUTED, WIDTH - left - 4);
            }
        }
        let hints = hints(own);
        if !hints.is_empty() {
            canvas.fill_rect(12, 196, WIDTH - 24, 1, ROBOT_MUTED);
            let mut x = 12;
            for (key, action) in hints {
                x += robot_hint(&mut canvas, x, 206, key, action);
            }
        }
    }
}

fn korekore(own: Own, now: u32, frame: &mut [u8]) {
    let mut canvas = Canvas::new(frame);
    let pairing = matches!(own, Own::Pairing { .. });
    let color = if pairing { SUN } else { SKY };
    canvas.fill_rect(0, 0, WIDTH, 240, BOX_GROUND);
    canvas.fill_rect(0, 0, WIDTH, 3, color);
    // The side panel, as on duty: korekore, the mark, Kore and a word.
    canvas.fill_rect(0, 3, 96, 237, BOX_CARD);
    kt::draw_bitmap(&mut canvas, &WORDMARK_14, 48 - i32::from(WORDMARK_14.width) / 2, 11, KORE_PINK);
    let beat = now / 500;
    if pairing {
        let bob = if beat.is_multiple_of(2) { -2 } else { 0 };
        kt::draw_centered(&mut canvas, &TYPE_11, 48, 31, b"!", color);
        sprite(&mut canvas, &KORE_INPUT, 8, 51 + bob, 4, false);
        kt::draw_centered(&mut canvas, &TYPE_14, 48, 201, b"PAIR", color);
    } else {
        kt::draw_centered(&mut canvas, &TYPE_11, 48, 31, b"< >", color);
        sprite(&mut canvas, &KORE_WORKING, 8, 51, 4, beat % 4 < 2);
        kt::draw_centered(&mut canvas, &TYPE_14, 48, 201, b"LOOKING", color);
    }
    let left = 104;
    let right = 312;
    match own {
        Own::Pairing { code, name, .. } => {
            kt::draw(&mut canvas, &TYPE_9, left, 11, b"PAIRING", BOX_TEXT_MUTED, 200);
            kt::draw(&mut canvas, &TYPE_16, left, 30, format!("Pair this {DEVICE}").as_bytes(), BOX_TEXT, right - left);
            kt::draw(&mut canvas, &TYPE_30, left, 58, grouped(code).as_bytes(), SUN, right - left);
            kt::draw(&mut canvas, &TYPE_10, left, 104, b"On your computer, open VibeBuddy,", BOX_TEXT_MUTED, right - left);
            kt::draw(&mut canvas, &TYPE_10, left, 118, format!("pick this {DEVICE},").as_bytes(), BOX_TEXT_MUTED, right - left);
            kt::draw(&mut canvas, &TYPE_10, left, 132, b"then type the code.", BOX_TEXT_MUTED, right - left);
            kt::draw(&mut canvas, &TYPE_11, left, 158, name.as_bytes(), BOX_TEXT, right - left);
        }
        Own::Looking { name } => {
            kt::draw(&mut canvas, &TYPE_9, left, 11, b"NOT LINKED", BOX_TEXT_MUTED, 200);
            kt::draw(&mut canvas, &TYPE_16, left, 30, b"Looking for", BOX_TEXT, right - left);
            let dots = ["", ".", "..", "..."][(beat % 4) as usize];
            kt::draw(&mut canvas, &TYPE_16, left, 50, format!("your computer{dots}").as_bytes(), BOX_TEXT, right - left);
            kt::draw(&mut canvas, &TYPE_10, left, 104, b"Open VibeBuddy on a computer", BOX_TEXT_MUTED, right - left);
            kt::draw(&mut canvas, &TYPE_10, left, 118, format!("paired with this {DEVICE},").as_bytes(), BOX_TEXT_MUTED, right - left);
            kt::draw(&mut canvas, &TYPE_10, left, 132, b"on the same Wi-Fi.", BOX_TEXT_MUTED, right - left);
            kt::draw(&mut canvas, &TYPE_11, left, 158, name.as_bytes(), BOX_TEXT, right - left);
        }
    }
    let mut x = left;
    for (key, action) in hints(own) {
        x += kk_hint(&mut canvas, &TYPE_9, x, 220, key, &action.to_ascii_lowercase()) + 10;
    }
}
