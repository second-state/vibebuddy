//! The buddy's whole screen: the scenes for each of the duty, pomodoro and leisure modes, plus animation
//! pacing, backlight and screenshots. Only draws into the frame buffer; committing and the backlight are
//! left to [`Screen`].
//!
//! Coordinates, colors and frame pacing are copied from the C firmware's agent_display.c; the screens
//! should match pixel for pixel.

use alloc::vec::Vec;

use crate::canvas::{Canvas, FRAME_BYTES, HEIGHT, WIDTH, half_bright};
use crate::leisure::{self, Skit};
use crate::pomodoro::{self, Phase, Run};
use crate::text;
use crate::text::{Text, truncated};

pub const MAX_TASKS: usize = 3;
pub const MAX_STATS: usize = 3;

const COLOR_BACKGROUND: u16 = 0x0841;
const COLOR_MUTED: u16 = 0x8410;
const COLOR_TEXT: u16 = 0xffff;
const COLOR_READY: u16 = 0x2dff;
const COLOR_WORKING: u16 = 0xfd20;
const COLOR_INPUT: u16 = 0xffe0;
const COLOR_DONE: u16 = 0x07e0;
const COLOR_FAILED: u16 = 0xf800;
const COLOR_PET: u16 = 0x3c9f;
const COLOR_PET_HIGHLIGHT: u16 = 0x7e5f;
const COLOR_SCREEN: u16 = 0x10a4;
const COLOR_FOCUS: u16 = 0xfa8a;
const COLOR_BREAK: u16 = 0x4ecc;

const TITLE_BYTES: usize = 63;
const BUILD_BYTES: usize = 47;

/// Idle small moves: one in the last 8 frames of every 40.
const IDLE_MOOD_PERIOD: u32 = 40;
const IDLE_MOOD_FRAMES: u32 = 8;
/// The rotating line on the idle screen changes every 6 frames.
const IDLE_ROTATE_FRAMES: u32 = 6;

const RING_CENTER_X: i32 = 118;
const RING_CENTER_Y: i32 = 122;
const RING_TICKS: i32 = 60;
const RING_TICK_INNER: i32 = 70;
const RING_TICK_OUTER: i32 = 79;
const RING_HAND_INNER: i32 = 64;
const RING_HAND_OUTER: i32 = 86;
/// The alarm first shakes for 20 frames of 100 ms each, 3 pixels to either side.
const RING_ALARM_SHAKE_FRAMES: u32 = 20;
const RING_ALARM_SHAKE_FRAME_MS: u32 = 100;
const RING_ALARM_SHAKE_PX: i32 = 3;
const PANEL_X: i32 = 208;
use core::f32::consts::TAU;
/// The ball's arc keeps the C firmware's 3.14159 rather than π: the screen must match it pixel for pixel.
#[allow(clippy::approx_constant, reason = "the same approximation as the C firmware")]
const BALL_PI: f32 = 3.14159;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Working,
    InputRequired,
    Done,
    Failed,
    Offline,
}

/// The buddy is in exactly one mode at a time, and each mode owns the whole screen. Duty
/// watches the agents, pomodoro times the user, and leisure is the buddy playing on its
/// own after duty has been idle long enough. Agent state keeps updating in all three
/// modes; pomodoro mode just gives it a one-line summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Duty,
    Pomodoro,
    Leisure,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Duty => "DUTY",
            Mode::Pomodoro => "POMODORO",
            Mode::Leisure => "LEISURE",
        }
    }
}

/// The input for one task card.
pub struct TaskInput<'a> {
    pub title: Option<&'a [u8]>,
    pub state: State,
    /// Seconds since entering the current state. The device keeps counting on its own,
    /// because the Mac sends nothing while the visible state is unchanged, yet the number
    /// on the card must keep moving.
    pub elapsed_s: i32,
    /// Owning project; drawn on the second line when the title is a session name, skipped when empty or
    /// the same as the title.
    pub project: Option<&'a [u8]>,
}

struct Task {
    title: Vec<u8>,
    project: Vec<u8>,
    state: State,
    elapsed_base: i32,
    received_ms: u32,
}

/// Screen hardware: frame buffer, commit, backlight.
pub trait Screen {
    fn frame(&mut self) -> &mut [u8];
    fn present(&mut self) -> Result<(), ()>;
    fn set_backlight(&mut self, on: bool) -> Result<(), ()>;
}

/// How a panel line's value is colored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Dim,
    Accent,
    Warn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    /// Rows: a label on the left, its value on the right, one row selected.
    List,
    /// Facts: a small label, a large value.
    Facts,
    /// A few lines of small text, values left empty.
    Message,
}

/// A panel over the whole screen (the device menu): what it says is the caller's, drawing it is the display's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    pub kind: PanelKind,
    pub title: Vec<u8>,
    /// Label, value and the value's tone.
    pub lines: Vec<(Vec<u8>, Vec<u8>, Tone)>,
    pub selected: Option<usize>,
    /// Key name and what it does now, along the bottom.
    pub hints: Vec<(&'static [u8], &'static [u8])>,
}

/// External state needed to draw a frame: the pomodoro and the leisure director belong to the main program.
pub struct Scene<'a> {
    pub now_ms: u32,
    pub pomodoro: &'a pomodoro::Pomodoro,
    pub leisure: &'a leisure::Leisure,
}

pub struct Display {
    ready: bool,
    state: State,
    link_lost: bool,
    mode: Mode,
    title: Vec<u8>,
    tasks: Vec<Task>,
    stats: Vec<Vec<u8>>,
    firmware_build: Vec<u8>,
    daemon_build: Vec<u8>,
    animation_frame: u32,
    next_animation_at: u32,
    /// Whether the backlight is on. Leisure mode turns it off after sleeping long at night; anything at all
    /// turns it back on.
    backlight_on: bool,
    /// Blink to identify: the backlight flashes until this time; None means not flashing.
    identify_until: Option<u32>,
    identify_next_toggle: u32,
    muted: bool,
    /// The alarm is ringing: the phase ended and the user hasn't acted yet. Records the phase at the end;
    /// stops as soon as the view changes (start, abandon, skip) or the screen is switched away.
    ring_alarm: bool,
    ring_alarm_phase: Phase,
    /// Shake frames left; after shaking, switch to pulsing.
    ring_alarm_shake_frames: u32,
    /// Drawn over everything while set.
    panel: Option<Panel>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IdleMood {
    None,
    Nap,
    Look,
    Stretch,
}

/// How many frames the small move has run; negative means no small move right now.
fn idle_mood_phase(frame: u32) -> i32 {
    (frame % IDLE_MOOD_PERIOD) as i32 - (IDLE_MOOD_PERIOD - IDLE_MOOD_FRAMES) as i32
}

/// Cycle through three small moves while idle. Breathing and blinking alone aren't enough: a device that is
/// always lit would look stuck rather than on standby.
fn idle_mood(frame: u32) -> IdleMood {
    if idle_mood_phase(frame) < 0 {
        return IdleMood::None;
    }
    [IdleMood::Nap, IdleMood::Look, IdleMood::Stretch][((frame / IDLE_MOOD_PERIOD) % 3) as usize]
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Eyes {
    #[default]
    Open,
    Closed,
    Wide,
    Half,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mouth {
    #[default]
    Smile,
    Flat,
    Open,
}

#[derive(Clone, Copy, Default)]
struct Pose {
    x: i32,
    y: i32,
    gaze_x: i32,
    gaze_y: i32,
    antenna: i32,
    left_arm: i32,
    right_arm: i32,
    left_foot: i32,
    right_foot: i32,
    eyes: Eyes,
    mouth: Mouth,
}

fn draw_pet_face(canvas: &mut Canvas, state: State, frame: u32, x_offset: i32, y_offset: i32, color: u16) {
    let face_y = 79 + y_offset;
    match state {
        State::Working => {
            canvas.draw_text(139 + x_offset, face_y + 5, b">", 2, color, 1);
            let dot_count = (frame % 3) as i32 + 1;
            for index in 0..dot_count {
                canvas.fill_rect(163 + x_offset + index * 8, face_y + 17, 5, 3, color);
            }
        }
        State::InputRequired => canvas.draw_text(141 + x_offset, face_y + 5, b"!?", 2, color, 2),
        State::Done => {
            canvas.draw_line(140 + x_offset, face_y + 12, 147 + x_offset, face_y + 6, color);
            canvas.draw_line(147 + x_offset, face_y + 6, 154 + x_offset, face_y + 12, color);
            canvas.draw_line(166 + x_offset, face_y + 12, 173 + x_offset, face_y + 6, color);
            canvas.draw_line(173 + x_offset, face_y + 6, 180 + x_offset, face_y + 12, color);
            canvas.draw_line(151 + x_offset, face_y + 19, 160 + x_offset, face_y + 23, color);
            canvas.draw_line(160 + x_offset, face_y + 23, 169 + x_offset, face_y + 19, color);
        }
        State::Failed => {
            canvas.draw_line(140 + x_offset, face_y + 7, 153 + x_offset, face_y + 18, color);
            canvas.draw_line(153 + x_offset, face_y + 7, 140 + x_offset, face_y + 18, color);
            canvas.draw_line(167 + x_offset, face_y + 7, 180 + x_offset, face_y + 18, color);
            canvas.draw_line(180 + x_offset, face_y + 7, 167 + x_offset, face_y + 18, color);
            canvas.draw_line(153 + x_offset, face_y + 25, 167 + x_offset, face_y + 25, color);
        }
        State::Offline => {
            // Closed eyes and a flat mouth: asleep, not an error.
            canvas.fill_rect(143 + x_offset, face_y + 14, 8, 3, color);
            canvas.fill_rect(169 + x_offset, face_y + 14, 8, 3, color);
            canvas.draw_line(154 + x_offset, face_y + 25, 166 + x_offset, face_y + 25, color);
        }
        State::Idle => {
            let mood = idle_mood(frame);
            if mood == IdleMood::Nap {
                // Dozing: closed eyes plus a floating Z. The color and this Z tell it apart from the link-lost closed eyes.
                canvas.fill_rect(143 + x_offset, face_y + 14, 8, 3, color);
                canvas.fill_rect(169 + x_offset, face_y + 14, 8, 3, color);
                canvas.draw_line(154 + x_offset, face_y + 25, 166 + x_offset, face_y + 25, color);
                canvas.draw_text(172 + x_offset, 44 + y_offset - idle_mood_phase(frame), b"Z", 2, color, 1);
                return;
            }
            // Looking around: only the eyes move sideways, as if sizing up the room.
            let gaze = if mood == IdleMood::Look { if frame % 4 < 2 { -3 } else { 3 } } else { 0 };
            let blinking = frame % 8 == 7;
            let (eye_y, eye_height) = if blinking { (14, 3) } else { (8, 10) };
            canvas.fill_rect(143 + x_offset + gaze, face_y + eye_y, 8, eye_height, color);
            canvas.fill_rect(169 + x_offset + gaze, face_y + eye_y, 8, eye_height, color);
            canvas.draw_line(154 + x_offset, face_y + 24, 160 + x_offset, face_y + 27, color);
            canvas.draw_line(160 + x_offset, face_y + 27, 166 + x_offset, face_y + 24, color);
        }
    }
}

/// Antenna, body, arms and the screen on the face. x and y are offsets from the duty position; raised arms
/// use positive values. The face and legs are drawn separately, since each has other poses too.
fn draw_pet_body(canvas: &mut Canvas, x: i32, y: i32, antenna: i32, knob_color: u16, left_arm: i32, right_arm: i32) {
    canvas.fill_rect(157 + x, 48 + y - antenna, 6, 13 + antenna, COLOR_PET_HIGHLIGHT);
    canvas.fill_rect(153 + x, 44 + y - antenna, 14, 10, knob_color);

    canvas.fill_rect(113 + x, 65 + y, 94, 52, COLOR_PET);
    canvas.fill_rect(121 + x, 59 + y, 78, 64, COLOR_PET);
    canvas.fill_rect(105 + x, 78 + y - left_arm, 12, 28, COLOR_PET_HIGHLIGHT);
    canvas.fill_rect(203 + x, 78 + y - right_arm, 12, 28, COLOR_PET_HIGHLIGHT);
    canvas.fill_rect(128 + x, 75 + y, 64, 40, COLOR_BACKGROUND);
    canvas.fill_rect(132 + x, 79 + y, 56, 32, COLOR_SCREEN);
}

/// Lower body and both feet; raised feet use positive values.
fn draw_pet_legs(canvas: &mut Canvas, x: i32, y: i32, left_foot: i32, right_foot: i32) {
    canvas.fill_rect(139 + x, 121 + y, 42, 25, COLOR_PET);
    canvas.fill_rect(126 + x, 125 + y - left_foot, 13, 17, COLOR_PET_HIGHLIGHT);
    canvas.fill_rect(181 + x, 125 + y - right_foot, 13, 17, COLOR_PET_HIGHLIGHT);
    canvas.fill_rect(143 + x, 145 + y - left_foot, 13, 8, COLOR_PET_HIGHLIGHT);
    canvas.fill_rect(164 + x, 145 + y - right_foot, 13, 8, COLOR_PET_HIGHLIGHT);
}

fn draw_buddy(canvas: &mut Canvas, state: State, frame: u32, x_offset: i32, color: u16) {
    let y_offset = match state {
        State::Working => if frame.is_multiple_of(2) { 0 } else { 2 },
        State::Done => if frame.is_multiple_of(2) { -5 } else { 0 },
        State::Failed => 3,
        State::Idle if frame.is_multiple_of(8) => 1,
        _ => 0,
    };
    // When stretching, only lengthen the antenna and keep the body still, so it reads as a stretch rather than a hop.
    let antenna = if state == State::Idle && idle_mood(frame) == IdleMood::Stretch { 5 } else { 0 };
    draw_pet_body(canvas, x_offset, y_offset, antenna, color, 0, 0);
    draw_pet_face(canvas, state, frame, x_offset, y_offset, color);
    draw_pet_legs(canvas, x_offset, y_offset, 0, 0);
}

fn short_state_label(state: State) -> &'static [u8] {
    match state {
        State::InputRequired => b"ASK",
        State::Done => b"DONE",
        State::Failed => b"FAIL",
        _ => b"RUN",
    }
}

/// The card's bottom-right corner is only three cells wide; past an hour, show hours only.
fn format_elapsed(seconds: i32) -> Text<8> {
    let seconds = seconds.max(0);
    if seconds < 60 {
        text!(8, "{}S", seconds)
    } else if seconds < 3600 {
        text!(8, "{}M", seconds / 60)
    } else {
        text!(8, "{}H", seconds / 3600)
    }
}

/// One line of the daily record: "3 FOCUS 1H15". Under an hour, only minutes.
fn format_tally(completed: u32, focus_s: u32) -> Text<23> {
    let minutes = focus_s / 60;
    if minutes < 60 {
        text!(23, "{} FOCUS {}M", completed % 10000, minutes % 60)
    } else {
        text!(23, "{} FOCUS {}H{:02}", completed % 10000, (minutes / 60) % 1000, minutes % 60)
    }
}

/// Rounds up to the second: shows 25:00 at the start and still 00:01 in the last millisecond.
fn format_countdown(remaining_ms: u32) -> Text<7> {
    let seconds = remaining_ms.div_ceil(1000);
    text!(7, "{:02}:{:02}", (seconds / 60) % 100, seconds % 60)
}

fn phase_color(view: &pomodoro::View) -> u16 {
    if view.is_idle() {
        COLOR_READY
    } else if view.phase == Phase::Break {
        COLOR_BREAK
    } else {
        COLOR_FOCUS
    }
}

/// Draws a radial segment outward from the center. Angles run clockwise from 12 o'clock; shift is the whole
/// ring's horizontal offset, nonzero only while the alarm shakes.
fn draw_radial(canvas: &mut Canvas, angle: f32, inner: i32, outer: i32, thickness: i32, color: u16, shift: i32) {
    let dx = libm::sinf(angle);
    let dy = -libm::cosf(angle);
    for radius in inner..=outer {
        let x = RING_CENTER_X + shift + libm::roundf(dx * radius as f32) as i32;
        let y = RING_CENTER_Y + libm::roundf(dy * radius as f32) as i32;
        canvas.fill_rect(x - thickness / 2, y - thickness / 2, thickness, thickness, color);
    }
}

/// Eyes open and smiling, blinking every three seconds.
fn resting_pose(frame: u32) -> Pose {
    Pose { eyes: if frame % 24 == 23 { Eyes::Closed } else { Eyes::Open }, mouth: Mouth::Smile, ..Pose::default() }
}

fn sleeping_pose(frame: u32) -> Pose {
    Pose { eyes: Eyes::Closed, mouth: Mouth::Flat, y: ((frame / 8) % 2) as i32, ..Pose::default() }
}

fn draw_pet_eyes(canvas: &mut Canvas, x: i32, y: i32, eyes: Eyes, gaze_x: i32, gaze_y: i32, color: u16) {
    let face_y = 79 + y;
    let left = 143 + x + gaze_x;
    let right = 169 + x + gaze_x;
    match eyes {
        Eyes::Closed => {
            canvas.fill_rect(left, face_y + 14, 8, 3, color);
            canvas.fill_rect(right, face_y + 14, 8, 3, color);
        }
        Eyes::Half => {
            canvas.fill_rect(left, face_y + 12 + gaze_y, 8, 5, color);
            canvas.fill_rect(right, face_y + 12 + gaze_y, 8, 5, color);
        }
        Eyes::Wide => {
            canvas.fill_rect(left - 1, face_y + 6 + gaze_y, 10, 13, color);
            canvas.fill_rect(right - 1, face_y + 6 + gaze_y, 10, 13, color);
        }
        Eyes::Open => {
            canvas.fill_rect(left, face_y + 8 + gaze_y, 8, 10, color);
            canvas.fill_rect(right, face_y + 8 + gaze_y, 8, 10, color);
        }
    }
}

fn draw_pet_mouth(canvas: &mut Canvas, x: i32, y: i32, mouth: Mouth, color: u16) {
    let face_y = 79 + y;
    match mouth {
        Mouth::Smile => {
            canvas.draw_line(154 + x, face_y + 24, 160 + x, face_y + 27, color);
            canvas.draw_line(160 + x, face_y + 27, 166 + x, face_y + 24, color);
        }
        Mouth::Flat => canvas.draw_line(154 + x, face_y + 25, 166 + x, face_y + 25, color),
        Mouth::Open => canvas.fill_rect(155 + x, face_y + 21, 10, 8, color),
    }
}

fn draw_pet_pose(canvas: &mut Canvas, pose: &Pose) {
    draw_pet_body(canvas, pose.x, pose.y, pose.antenna, COLOR_READY, pose.left_arm, pose.right_arm);
    draw_pet_eyes(canvas, pose.x, pose.y, pose.eyes, pose.gaze_x, pose.gaze_y, COLOR_READY);
    draw_pet_mouth(canvas, pose.x, pose.y, pose.mouth, COLOR_READY);
    draw_pet_legs(canvas, pose.x, pose.y, pose.left_foot, pose.right_foot);
}

fn draw_sleeping_z(canvas: &mut Canvas, x: i32, y: i32, frame: u32) {
    canvas.draw_text(172 + x, 44 + y - (frame % 16) as i32, b"Z", 2, COLOR_READY, 1);
}

/// Plain idle between skits: standing, breathing, blinking.
fn skit_rest(canvas: &mut Canvas, frame: u32) {
    let mut pose = resting_pose(frame);
    pose.y = ((frame / 8) % 2) as i32;
    draw_pet_pose(canvas, &pose);
}

fn skit_sleep(canvas: &mut Canvas, frame: u32) {
    let pose = sleeping_pose(frame);
    draw_pet_pose(canvas, &pose);
    draw_sleeping_z(canvas, 0, pose.y, frame);
}

/// Patrol: walk to the right, stop and glance at you, walk to the left, then come back.
fn skit_patrol(canvas: &mut Canvas, frame: u32) {
    let mut pose = resting_pose(frame);
    let frame_i = frame as i32;
    let mut walking = true;
    if frame < 24 {
        pose.x = 3 * frame_i;
        pose.gaze_x = 3;
    } else if frame < 36 {
        pose.x = 72;
        walking = false;
    } else if frame < 72 {
        pose.x = 72 - 3 * (frame_i - 36);
        pose.gaze_x = -3;
    } else if frame < 84 {
        pose.x = -36;
        walking = false;
    } else {
        pose.x = -36 + 3 * (frame_i - 84);
        pose.gaze_x = 3;
    }
    if walking {
        let left_step = frame % 4 < 2;
        pose.left_foot = if left_step { 4 } else { 0 };
        pose.right_foot = if left_step { 0 } else { 4 };
        pose.y = if left_step { 0 } else { 1 };
    }
    draw_pet_pose(canvas, &pose);
}

fn draw_ball(canvas: &mut Canvas, cx: i32, cy: i32, frame: u32) {
    canvas.fill_rect(cx - 4, cy - 4, 8, 8, COLOR_INPUT);
    canvas.fill_rect(cx - 3, cy - 5, 6, 10, COLOR_INPUT);
    canvas.fill_rect(cx - 5, cy - 3, 10, 6, COLOR_INPUT);
    // A dark dot circling around makes the ball look like it's rolling.
    const SPIN: [[i32; 2]; 4] = [[-2, -2], [2, -2], [2, 2], [-2, 2]];
    let spin = SPIN[(frame % 4) as usize];
    canvas.fill_rect(cx + spin[0] - 1, cy + spin[1] - 1, 2, 2, COLOR_BACKGROUND);
}

/// Ball: the ball rolls from the left to the feet, gets kicked away, bounces and rolls back, then is kicked
/// off screen. The ball always flies on the left of the body and never passes through it.
fn skit_ball(canvas: &mut Canvas, frame: u32) {
    const GROUND: i32 = 149;
    const AT_FOOT: i32 = 116;
    let frame_i = frame as i32;
    let mut ball_y = GROUND;
    let ball_x;
    if frame < 24 {
        ball_x = 20 + (AT_FOOT - 20) * frame_i / 24;
    } else if frame < 52 {
        let t = (frame - 24) as f32 / 28.0;
        ball_x = AT_FOOT - (86.0 * t) as i32;
        ball_y = GROUND - (50.0 * libm::sinf(BALL_PI * t)) as i32;
    } else if frame < 60 {
        let t = (frame - 52) as f32 / 8.0;
        ball_x = 30 - (10.0 * t) as i32;
        ball_y = GROUND - (16.0 * libm::sinf(BALL_PI * t)) as i32;
    } else if frame < 80 {
        ball_x = 20 + (AT_FOOT - 20) * (frame_i - 60) / 20;
    } else {
        let t = (frame - 80) as f32 / 16.0;
        ball_x = AT_FOOT - (150.0 * t) as i32;
        ball_y = GROUND - (40.0 * libm::sinf(BALL_PI * t)) as i32;
    }
    let kicking = (21..27).contains(&frame) || (77..83).contains(&frame);
    let mut pose = resting_pose(frame);
    pose.gaze_x = -3;
    pose.gaze_y = if ball_y < GROUND - 20 { -2 } else { 2 };
    if kicking {
        pose.left_foot = 8;
        pose.mouth = Mouth::Open;
    }
    if frame >= 88 {
        pose.eyes = Eyes::Wide;
    }
    draw_pet_pose(canvas, &pose);
    draw_ball(canvas, ball_x, ball_y, frame);
}

/// Reading: holds up a book and scans it line by line, turns a page every few seconds, and gets startled by
/// the plot midway.
fn skit_read(canvas: &mut Canvas, frame: u32) {
    let mut pose = resting_pose(frame);
    pose.left_arm = 10;
    pose.right_arm = 10;
    pose.gaze_y = 3;
    pose.gaze_x = ((frame / 2) % 6) as i32 - 2;
    let surprised = (72..80).contains(&frame);
    if surprised {
        pose.eyes = Eyes::Wide;
        pose.gaze_x = 0;
        pose.gaze_y = 0;
        pose.mouth = Mouth::Open;
    }
    draw_pet_pose(canvas, &pose);

    let (book_x, book_y) = (138, 114);
    canvas.fill_rect(book_x, book_y, 44, 26, COLOR_TEXT);
    canvas.fill_rect(book_x + 21, book_y, 2, 26, COLOR_MUTED);
    for line in 0..3 {
        canvas.fill_rect(book_x + 4, book_y + 5 + line * 6, 14, 2, COLOR_MUTED);
        canvas.fill_rect(book_x + 26, book_y + 5 + line * 6, 14, 2, COLOR_MUTED);
    }
    if frame % 40 >= 36 {
        canvas.fill_rect(book_x + 14, book_y - 6, 10, 32, COLOR_TEXT);
    }
    if surprised {
        canvas.draw_text(190, 44, b"!", 3, COLOR_INPUT, 1);
    }
}

/// Counting stars: looks up and counts to seven, slower and slower, and falls asleep counting.
fn skit_stars(canvas: &mut Canvas, frame: u32) {
    const STARS: [[i32; 2]; 12] = [
        [20, 36], [48, 52], [75, 40], [100, 60], [130, 34], [200, 44],
        [230, 62], [262, 38], [290, 54], [306, 70], [170, 66], [60, 72],
    ];
    for (index, star) in STARS.iter().enumerate() {
        if !(frame / 3 + index as u32).is_multiple_of(4) {
            canvas.fill_rect(star[0], star[1], 2, 2, if index % 3 == 0 { COLOR_TEXT } else { COLOR_MUTED });
        }
    }
    let mut pose = resting_pose(frame);
    pose.gaze_y = -3;
    pose.mouth = Mouth::Flat;
    if (56..88).contains(&frame) {
        pose.eyes = Eyes::Half;
    } else if frame >= 88 {
        pose = sleeping_pose(frame);
    }
    draw_pet_pose(canvas, &pose);
    if frame < 56 {
        let count = text!(3, "{}", (frame / 8 + 1) % 10);
        canvas.draw_text(200, 52, count.as_bytes(), 2, COLOR_MUTED, 4);
    } else if frame < 88 {
        canvas.draw_text(200, 52, b"...", 2, COLOR_MUTED, 3);
    } else {
        draw_sleeping_z(canvas, 0, pose.y, frame);
    }
}

/// Hide and seek: slips past the right edge of the screen until only a waving hand shows, leans half out for
/// a look, ducks back, and finally walks back.
fn skit_hide(canvas: &mut Canvas, frame: u32) {
    let mut pose = resting_pose(frame);
    let frame_i = frame as i32;
    if frame < 12 {
        pose.x = 16 * frame_i;
    } else if frame < 28 {
        pose.x = 192;
        pose.left_arm = if frame % 4 < 2 { 0 } else { 6 };
    } else if frame < 36 {
        pose.x = 192 - 9 * (frame_i - 28);
        pose.eyes = Eyes::Wide;
    } else if frame < 52 {
        pose.x = 120;
        pose.eyes = if frame == 44 { Eyes::Closed } else { Eyes::Wide };
        pose.mouth = Mouth::Open;
    } else if frame < 64 {
        pose.x = 120 + 6 * (frame_i - 52);
    } else {
        pose.x = 192 - 12 * (frame_i - 64);
    }
    draw_pet_pose(canvas, &pose);
}

/// Startled awake: asleep, then an exclamation mark jumps up, it looks left and right, yawns, and goes back
/// to sleep.
fn skit_startle(canvas: &mut Canvas, frame: u32) {
    let sleeping = !(24..56).contains(&frame);
    let mut pose = if sleeping { sleeping_pose(frame) } else { resting_pose(frame) };
    if !sleeping {
        if frame < 28 {
            pose.eyes = Eyes::Wide;
            pose.mouth = Mouth::Open;
            pose.y = -6;
            pose.antenna = 4;
        } else if frame < 44 {
            pose.eyes = Eyes::Wide;
            pose.gaze_x = if frame < 36 { -3 } else { 3 };
        } else {
            pose.eyes = Eyes::Half;
            pose.mouth = Mouth::Open;
            pose.left_arm = 6;
        }
    }
    draw_pet_pose(canvas, &pose);
    if sleeping {
        draw_sleeping_z(canvas, 0, pose.y, frame);
    }
    if (24..32).contains(&frame) {
        canvas.draw_text(190, 44, b"!", 3, COLOR_INPUT, 1);
    }
}

impl Default for Display {
    fn default() -> Self {
        Self::new()
    }
}

impl Display {
    pub fn new() -> Self {
        Self {
            ready: false,
            state: State::Idle,
            link_lost: false,
            mode: Mode::Duty,
            title: Vec::new(),
            tasks: Vec::new(),
            stats: Vec::new(),
            firmware_build: Vec::new(),
            daemon_build: Vec::new(),
            animation_frame: 0,
            next_animation_at: 0,
            backlight_on: false,
            identify_until: None,
            identify_next_toggle: 0,
            muted: false,
            ring_alarm: false,
            ring_alarm_phase: Phase::Focus,
            ring_alarm_shake_frames: 0,
            panel: None,
        }
    }

    /// The screen hardware is initialized and the backlight is still off: draw the first frame, then turn on the backlight.
    pub fn start(&mut self, screen: &mut dyn Screen, scene: &Scene) -> Result<(), ()> {
        self.ready = true;
        self.show_tasks(screen, scene, State::Idle, None, &[])?;
        screen.set_backlight(true)?;
        self.backlight_on = true;
        Ok(())
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether the agents have nothing going on: main state idle and no task cards. Leisure
    /// boredom accumulates on this, not on time since the last message, because long tasks
    /// send no messages midway anyway.
    pub fn agent_idle(&self) -> bool {
        self.state == State::Idle && self.tasks.is_empty()
    }

    fn state_label(&self) -> &'static [u8] {
        if self.link_lost {
            return b"NO LINK";
        }
        match self.state {
            State::Working => b"WORKING",
            State::InputRequired => b"INPUT REQUIRED",
            State::Done => b"DONE",
            State::Failed => b"FAILED",
            _ => b"READY",
        }
    }

    fn state_color(&self, state: State) -> u16 {
        if self.link_lost {
            // Everything turns gray while the link is lost: the state may be stale and shouldn't keep claiming it in
            // bright colors.
            return COLOR_MUTED;
        }
        match state {
            State::Working => COLOR_WORKING,
            State::InputRequired => COLOR_INPUT,
            State::Done => COLOR_DONE,
            State::Failed => COLOR_FAILED,
            _ => COLOR_READY,
        }
    }

    /// Seconds at the time the card was received, plus the time the device has counted since.
    fn task_elapsed_seconds(&self, task: &Task, now_ms: u32) -> i32 {
        task.elapsed_base + (now_ms.wrapping_sub(task.received_ms) / 1000) as i32
    }

    fn draw_task_cards(&self, canvas: &mut Canvas, now_ms: u32) {
        for (index, task) in self.tasks.iter().enumerate() {
            let index = index as i32;
            let x = 8 + index * 4;
            let y = 50 + index * 38;
            let width = 188 - index * 4;
            let card_color = if index == 0 { 0x18c6 } else { 0x1083 };
            let color = self.state_color(task.state);
            canvas.fill_rect(x, y, width, 32, COLOR_MUTED);
            canvas.fill_rect(x + 2, y + 2, width - 4, 28, card_color);
            canvas.fill_rect(x + 2, y + 2, 4, 28, color);
            canvas.draw_text(x + 12, y + 5, &task.title, 1, COLOR_TEXT, 26);
            canvas.draw_text(x + 12, y + 17, short_state_label(task.state), 1, color, 4);
            // When the first line is a session name, the project name moves to the second line; if the title is the
            // project name, don't repeat it.
            if !task.project.is_empty() && !contains(&task.title, &task.project) {
                canvas.draw_text(x + 42, y + 17, &task.project, 1, COLOR_MUTED, 16);
            }
            let elapsed = format_elapsed(self.task_elapsed_seconds(task, now_ms));
            let elapsed_width = elapsed.len() as i32 * 6 - 1;
            // While waiting for input, light up the duration too: this column answers exactly "how long has it waited".
            let elapsed_color = if task.state == State::InputRequired { color } else { COLOR_MUTED };
            canvas.draw_text(x + width - 8 - elapsed_width, y + 17, elapsed.as_bytes(), 1, elapsed_color, 8);
        }
    }

    /// The footer shows the build stamps of both sides: this firmware, and the Mac side carried by the heartbeat.
    ///
    /// Display only, no judgement. Flashing firmware means plugging in USB and stopping the daemon, while the
    /// daemon restarts after a one-line change, so most of the time the two sides aren't on the same commit;
    /// treating a mismatch as a warning would get it ignored within days. What actually breaks is a protocol
    /// capability mismatch, which a commit can't answer.
    ///
    /// Both lines are left-aligned to the same column: verbatim comparison relies on alignment, not color.
    fn draw_build_footer(&self, canvas: &mut Canvas) {
        let mut firmware_line = Text::<55>::new();
        firmware_line.push_bytes(b"FW     ");
        firmware_line.push_bytes(if self.firmware_build.is_empty() { b"?" } else { &self.firmware_build });
        let mut daemon_line = Text::<55>::new();
        daemon_line.push_bytes(b"APP    ");
        daemon_line.push_bytes(if self.daemon_build.is_empty() { b"?" } else { &self.daemon_build });
        let longest = firmware_line.len().max(daemon_line.len()) as i32;
        let x = ((WIDTH - (longest * 6 - 1)) / 2).max(2);
        canvas.draw_text(x, 216, firmware_line.as_bytes(), 1, COLOR_MUTED, 56);
        canvas.draw_text(x, 228, daemon_line.as_bytes(), 1, COLOR_MUTED, 56);
    }

    /// While idle, rotate between the title and stats. The idle screen shows up most often, so a single fixed
    /// sentence is a waste. The pomodoro's daily record is kept by the device itself and goes into the rotation too.
    fn draw_idle_line(&self, canvas: &mut Canvas, scene: &Scene) {
        let pomodoro = scene.pomodoro.view(scene.now_ms);
        let tally_line = format_tally(pomodoro.completed, pomodoro.focus_s);
        let mut lines: [&[u8]; 2 + MAX_STATS] = [&[]; 2 + MAX_STATS];
        let mut count = 0;
        let has_title = !self.title.is_empty();
        if has_title {
            lines[count] = &self.title;
            count += 1;
        }
        for stat in &self.stats {
            lines[count] = stat;
            count += 1;
        }
        if pomodoro.completed > 0 {
            lines[count] = tally_line.as_bytes();
            count += 1;
        }
        if count == 0 {
            canvas.draw_text_centered(195, b"YOUR VIBE BUDDY", 2, COLOR_MUTED);
            return;
        }
        let slot = (self.animation_frame / IDLE_ROTATE_FRAMES) as usize % count;
        let color = if has_title && slot == 0 { COLOR_TEXT } else { COLOR_MUTED };
        canvas.draw_text_centered(195, lines[slot], 2, color);
    }

    /// Whether the alarm is still ringing. After the end, any change of view means the user acted: starting the
    /// next phase makes it running, abandoning or skipping changes the phase.
    fn ring_alarm_active(&mut self, view: &pomodoro::View) -> bool {
        if self.ring_alarm && (view.run != Run::Pending || view.phase != self.ring_alarm_phase) {
            self.ring_alarm = false;
            self.ring_alarm_shake_frames = 0;
        }
        self.ring_alarm
    }

    fn ring_alarm_shaking(&self) -> bool {
        self.ring_alarm && self.ring_alarm_shake_frames > 0
    }

    /// The whole ring's horizontal offset while shaking: switches side every frame.
    fn ring_alarm_shift(&self) -> i32 {
        if !self.ring_alarm_shaking() {
            return 0;
        }
        if self.animation_frame.is_multiple_of(2) { RING_ALARM_SHAKE_PX } else { -RING_ALARM_SHAKE_PX }
    }

    fn draw_pomodoro_ring(&self, canvas: &mut Canvas, view: &pomodoro::View, alarm: bool) {
        let mut color = phase_color(view);
        let elapsed = view.total_ms - view.remaining_ms;
        let sweep = TAU * elapsed as f32 / view.total_ms as f32;
        let shift = self.ring_alarm_shift();
        // Alarm: the whole ring lights up in the next phase's color. After shaking, alternate a bright and a dim
        // frame like a heartbeat, not a hard flash between gray and bright.
        if alarm && !self.ring_alarm_shaking() && self.animation_frame % 2 == 1 {
            color = half_bright(color);
        }
        for index in 0..RING_TICKS {
            let angle = TAU * index as f32 / RING_TICKS as f32;
            let passed = alarm || (view.run != Run::Pending && angle <= sweep);
            draw_radial(canvas, angle, RING_TICK_INNER, RING_TICK_OUTER, 2, if passed { color } else { COLOR_MUTED }, shift);
        }
        draw_radial(canvas, sweep, RING_HAND_INNER, RING_HAND_OUTER, 3, color, shift);
    }

    /// In the pomodoro scene the agents keep only a few lines at the bottom right: they still answer "what needs
    /// my attention most", and voice lines still play; the screen just goes to the countdown.
    fn draw_agent_summary(&self, canvas: &mut Canvas, label: &[u8], status_color: u16) {
        canvas.draw_text(PANEL_X, 176, b"AGENT", 1, COLOR_MUTED, 18);
        canvas.draw_text(PANEL_X, 188, label, 1, status_color, 18);
        if let Some(task) = self.tasks.first() {
            canvas.draw_text(PANEL_X, 200, &task.title, 1, if self.link_lost { COLOR_MUTED } else { COLOR_TEXT }, 18);
        }
    }

    fn draw_pomodoro_scene(&mut self, canvas: &mut Canvas, scene: &Scene, label: &[u8], status_color: u16) {
        let view = scene.pomodoro.view(scene.now_ms);
        let color = phase_color(&view);
        let paused = view.run == Run::Paused;
        let alarm = self.ring_alarm_active(&view);
        self.draw_pomodoro_ring(canvas, &view, alarm);

        // While paused the digits blink, the old stopwatch rule. While the alarm shakes, the digits shake with the ring.
        if !paused || self.animation_frame.is_multiple_of(2) {
            let countdown = format_countdown(view.remaining_ms);
            canvas.draw_text(RING_CENTER_X - 58 + self.ring_alarm_shift(), RING_CENTER_Y - 14, countdown.as_bytes(), 4, COLOR_TEXT, 5);
        }

        // Idle shows READY; right after focus ends and before the break starts, show BREAK with 05:00
        // to tell it apart from idle: this screen is waiting for the break to start, not for focus to start.
        let phase_label: &[u8] = if view.is_idle() {
            b"READY"
        } else if view.phase == Phase::Break {
            b"BREAK"
        } else {
            b"FOCUS"
        };
        canvas.draw_text(PANEL_X, 44, phase_label, 2, color, 9);
        if view.run == Run::Pending {
            canvas.draw_text(PANEL_X, 66, b"K0 START", 1, COLOR_MUTED, 18);
            if !view.is_idle() {
                canvas.draw_text(PANEL_X, 78, b"HOLD K0 SKIP", 1, COLOR_MUTED, 18);
            }
        } else {
            canvas.draw_text(PANEL_X, 66, if paused { b"K0 RESUME" } else { b"K0 PAUSE" }, 1, COLOR_MUTED, 18);
            canvas.draw_text(PANEL_X, 78, b"HOLD K0 STOP", 1, COLOR_MUTED, 18);
        }
        if paused {
            canvas.draw_text(PANEL_X, 98, b"PAUSED", 2, color, 9);
        }

        // Daily record: one cell per completed session, plus a line with the count and total focus time. Resets
        // on the Mac's local date and survives restarts.
        canvas.draw_text(PANEL_X, 118, b"TODAY", 1, COLOR_MUTED, 18);
        let shown = view.completed.min(8);
        for index in 0..shown as i32 {
            canvas.fill_rect(PANEL_X + index * 12, 130, 8, 8, COLOR_FOCUS);
        }
        if view.completed > 8 {
            let more = text!(7, "+{}", (view.completed - 8) % 1000);
            canvas.draw_text(PANEL_X + 96, 130, more.as_bytes(), 1, COLOR_FOCUS, 8);
        }
        let tally_line = format_tally(view.completed, view.focus_s);
        canvas.draw_text(PANEL_X, 144, tally_line.as_bytes(), 1, if view.completed > 0 { COLOR_FOCUS } else { COLOR_MUTED }, 18);

        self.draw_agent_summary(canvas, label, status_color);
    }

    /// Small pomodoro badge at the top right of the buddy scene: switching back to watch the agents shouldn't
    /// hide the countdown.
    fn draw_pomodoro_badge(&self, canvas: &mut Canvas, scene: &Scene) {
        let view = scene.pomodoro.view(scene.now_ms);
        if view.is_idle() || (view.run == Run::Paused && self.animation_frame % 2 == 1) {
            return;
        }
        let countdown = format_countdown(view.remaining_ms);
        let mut badge = Text::<11>::new();
        badge.push_bytes(if view.phase == Phase::Focus { b"F " } else { b"B " });
        badge.push_bytes(countdown.as_bytes());
        canvas.draw_text(WIDTH - 8 - (7 * 6 - 1), 15, badge.as_bytes(), 1, phase_color(&view), 12);
    }

    /// Sleep talking: asleep, with a string of small bubbles overhead showing today's stats.
    fn skit_dream(&self, canvas: &mut Canvas, frame: u32) {
        let pose = sleeping_pose(frame);
        draw_pet_pose(canvas, &pose);
        const BUBBLES: [[i32; 3]; 3] = [[176, 58, 3], [186, 50, 4], [196, 42, 5]];
        for (index, bubble) in BUBBLES.iter().enumerate() {
            if (frame / 4) % 4 > index as u32 {
                canvas.fill_rect(bubble[0], bubble[1], bubble[2], bubble[2], COLOR_MUTED);
            }
        }
        canvas.fill_rect(204, 34, 100, 26, COLOR_MUTED);
        canvas.fill_rect(206, 36, 96, 22, COLOR_SCREEN);
        let line: &[u8] = if self.stats.is_empty() {
            b"ZZZ"
        } else {
            &self.stats[(frame / 32) as usize % self.stats.len()]
        };
        canvas.draw_text(212, 43, line, 1, COLOR_TEXT, 15);
    }

    fn draw_leisure_scene(&self, canvas: &mut Canvas, scene: &Scene, view: &leisure::View) {
        let frame = view.skit_frame;
        match view.skit {
            Skit::Patrol => skit_patrol(canvas, frame),
            Skit::Ball => skit_ball(canvas, frame),
            Skit::Read => skit_read(canvas, frame),
            Skit::Stars => skit_stars(canvas, frame),
            Skit::Hide => skit_hide(canvas, frame),
            Skit::Startle => skit_startle(canvas, frame),
            Skit::Dream => self.skit_dream(canvas, frame),
            Skit::Sleep => skit_sleep(canvas, frame),
            Skit::None => skit_rest(canvas, frame),
        }
        self.draw_pomodoro_badge(canvas, scene);
    }

    fn draw_pet_scene(&self, canvas: &mut Canvas, scene: &Scene, label: &[u8], status_color: u16) {
        if !self.tasks.is_empty() {
            self.draw_task_cards(canvas, scene.now_ms);
        }
        let state = if self.link_lost { State::Offline } else { self.state };
        draw_buddy(canvas, state, self.animation_frame, if self.tasks.is_empty() { 0 } else { 96 }, status_color);
        canvas.draw_text_centered(162, label, if self.state == State::InputRequired { 2 } else { 3 }, status_color);
        if !self.tasks.is_empty() {
            canvas.draw_text_centered(195, b"LATEST ON TOP", 1, COLOR_MUTED);
        } else if self.state == State::Idle && !self.link_lost {
            // No rotation while the link is lost: a moving screen looks alive, the exact opposite of NO LINK.
            self.draw_idle_line(canvas, scene);
        } else if !self.title.is_empty() {
            canvas.draw_text_centered(195, &self.title, 2, COLOR_TEXT);
        }
        self.draw_pomodoro_badge(canvas, scene);
    }

    fn ensure_backlight(&mut self, screen: &mut dyn Screen, wanted: bool) {
        if self.backlight_on != wanted && screen.set_backlight(wanted).is_ok() {
            self.backlight_on = wanted;
        }
    }

    fn render(&mut self, screen: &mut dyn Screen, scene: &Scene) -> Result<(), ()> {
        let label = self.state_label();
        let status_color = self.state_color(self.state);
        let mut dim = false;

        let leisure = scene.leisure.view(scene.now_ms);
        if self.mode == Mode::Leisure {
            if leisure.lights_out {
                // After sleeping long at night, turn off the backlight; a screen nobody watches needn't be drawn.
                self.ensure_backlight(screen, false);
                return Ok(());
            }
            dim = leisure.dim;
        }
        self.ensure_backlight(screen, true);

        let mut canvas = Canvas::new(screen.frame());
        canvas.fill_rect(0, 0, WIDTH, HEIGHT, COLOR_BACKGROUND);
        canvas.fill_rect(0, 0, WIDTH, 4, status_color);
        canvas.draw_text_centered(12, b"Vibe Buddy", 2, COLOR_TEXT);
        if self.muted {
            canvas.draw_text(8, 15, b"MUTE", 1, COLOR_INPUT, 4);
        }
        match self.mode {
            Mode::Pomodoro => self.draw_pomodoro_scene(&mut canvas, scene, label, status_color),
            Mode::Leisure => self.draw_leisure_scene(&mut canvas, scene, &leisure),
            Mode::Duty => self.draw_pet_scene(&mut canvas, scene, label, status_color),
        }
        self.draw_build_footer(&mut canvas);
        if dim {
            canvas.dim();
        }
        if let Some(panel) = &self.panel {
            draw_panel(&mut canvas, panel);
        }
        screen.present()
    }

    fn animation_period(&self) -> u32 {
        match self.mode {
            Mode::Leisure => leisure::FRAME_MS,
            // The countdown changes every second; blinking while paused needs a frame every half second; the alarm
            // shake is faster still.
            Mode::Pomodoro => if self.ring_alarm_shaking() { RING_ALARM_SHAKE_FRAME_MS } else { 500 },
            Mode::Duty => match self.state {
                State::Working => 250,
                State::Done => 200,
                State::InputRequired => 350,
                State::Failed => 700,
                _ => 500,
            },
        }
    }

    pub fn show_tasks(
        &mut self,
        screen: &mut dyn Screen,
        scene: &Scene,
        state: State,
        title: Option<&[u8]>,
        tasks: &[TaskInput],
    ) -> Result<(), ()> {
        if !self.ready {
            return Err(());
        }
        self.state = state;
        self.animation_frame = 0;
        self.title = title.map(|title| truncated(title, TITLE_BYTES)).unwrap_or_default();
        self.tasks = tasks
            .iter()
            .take(MAX_TASKS)
            .map(|task| Task {
                title: truncated(task.title.unwrap_or(b"CODEX"), TITLE_BYTES),
                project: truncated(task.project.unwrap_or(b""), TITLE_BYTES),
                state: task.state,
                elapsed_base: task.elapsed_s,
                received_ms: scene.now_ms,
            })
            .collect();
        self.next_animation_at = scene.now_ms.wrapping_add(self.animation_period());
        self.render(screen, scene)
    }

    /// The done announcement is over: go back to the cards that came with it and are still open, or to idle when
    /// none are. The Mac sends nothing while the visible state is unchanged, and a task can run for minutes
    /// without an event, so dropping the cards here would show READY over work still in progress.
    /// Returns the state shown.
    pub fn settle_after_done(&mut self, screen: &mut dyn Screen, scene: &Scene) -> Result<State, ()> {
        if !self.ready {
            return Err(());
        }
        self.tasks
            .retain(|task| matches!(task.state, State::Working | State::InputRequired));
        self.state = if self.tasks.iter().any(|task| task.state == State::InputRequired) {
            State::InputRequired
        } else if self.tasks.is_empty() {
            State::Idle
        } else {
            State::Working
        };
        self.title = self.tasks.first().map(|task| task.title.clone()).unwrap_or_default();
        self.animation_frame = 0;
        self.next_animation_at = scene.now_ms.wrapping_add(self.animation_period());
        self.render(screen, scene)?;
        Ok(self.state)
    }

    /// Screenshot: run-length encodes the current framebuffer and hands it to `write_line`
    /// line by line (without newlines). The first line is `SHOT BEGIN 320x240 BACKLIGHT ON|OFF`,
    /// each middle line holds several `rgb565:length` runs, and the last is `SHOT END`.
    /// Writes out while encoding instead of buffering the whole frame: one frame's run-length encoding is
    /// tens of KB, larger than the heap.
    pub fn dump(&self, frame: &[u8], write_line: &mut dyn FnMut(&[u8])) {
        let begin = text!(64, "SHOT BEGIN {}x{} BACKLIGHT {}", WIDTH, HEIGHT, if self.backlight_on { "ON" } else { "OFF" });
        write_line(begin.as_bytes());
        let pixel = |index: usize| u16::from_be_bytes([frame[index * 2], frame[index * 2 + 1]]);
        let total = FRAME_BYTES / 2;
        let mut index = 0;
        let mut line = Text::<200>::new();
        line.push_bytes(b"SHOT");
        let mut runs = 0;
        while index < total {
            let color = pixel(index);
            let mut run = 1;
            while index + run < total && pixel(index + run) == color && run < 60000 {
                run += 1;
            }
            let _ = core::fmt::Write::write_fmt(&mut line, format_args!(" {:04x}:{}", color, run));
            index += run;
            runs += 1;
            if runs == 16 || index == total {
                write_line(line.as_bytes());
                line = Text::new();
                line.push_bytes(b"SHOT");
                runs = 0;
            }
        }
        write_line(b"SHOT END");
    }

    /// Records today's stats, which the idle screen rotates through. Takes effect on the next draw.
    pub fn set_stats(&mut self, lines: &[&[u8]]) {
        self.stats = lines.iter().take(MAX_STATS).map(|line| truncated(line, TITLE_BYTES)).collect();
    }

    /// The heartbeat comes every 5 seconds; don't redraw if the stamp hasn't changed.
    fn set_build(&mut self, screen: &mut dyn Screen, scene: &Scene, firmware: bool, build: &[u8]) {
        let value = truncated(build, BUILD_BYTES);
        let slot = if firmware { &mut self.firmware_build } else { &mut self.daemon_build };
        if *slot == value {
            return;
        }
        *slot = value;
        if self.ready {
            let _ = self.render(screen, scene);
        }
    }

    /// Sets the build stamps of this firmware and of the Mac side. The device compares them
    /// for the user: asking people to read two hashes and compare them isn't reliable, and a
    /// mismatch is exactly the signal they need to see.
    pub fn set_firmware_build(&mut self, screen: &mut dyn Screen, scene: &Scene, build: &[u8]) {
        self.set_build(screen, scene, true, build);
    }

    pub fn set_daemon_build(&mut self, screen: &mut dyn Screen, scene: &Scene, build: &[u8]) {
        self.set_build(screen, scene, false, build);
    }

    /// Blink to identify: the backlight flashes for about a second, visible in any mode. Onboarding uses it to
    /// find the box.
    pub fn identify(&mut self, now_ms: u32) {
        if !self.ready {
            return;
        }
        self.identify_until = Some(now_ms.wrapping_add(1200));
        self.identify_next_toggle = now_ms;
    }

    /// Alarm at the end of a pomodoro phase: the ring shakes for two seconds, then the whole
    /// ring pulses until the user presses a key or leaves the screen. When muted this is the
    /// only reminder. The caller must then bring the pomodoro screen to the front.
    pub fn pomodoro_ended(&mut self, scene: &Scene) {
        let view = scene.pomodoro.view(scene.now_ms);
        self.ring_alarm = true;
        self.ring_alarm_phase = view.phase;
        self.ring_alarm_shake_frames = RING_ALARM_SHAKE_FRAMES;
        // The caller redraws the first frame right after; this only schedules the later frames on the shake rhythm.
        self.animation_frame = 0;
        self.next_animation_at = scene.now_ms.wrapping_add(RING_ALARM_SHAKE_FRAME_MS);
    }

    /// Overlay while the link is lost: the buddy closes its eyes and the screen turns gray.
    /// The underlying state and task cards are kept: they are the last known facts, just no
    /// longer trustworthy.
    pub fn set_link_lost(&mut self, screen: &mut dyn Screen, scene: &Scene, lost: bool) {
        if self.link_lost == lost {
            return;
        }
        self.link_lost = lost;
        if self.ready {
            let _ = self.render(screen, scene);
        }
    }

    pub fn set_mode(&mut self, screen: &mut dyn Screen, scene: &Scene, mode: Mode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.animation_frame = 0;
        // Switching away from the pomodoro screen means it was seen: the alarm can stop.
        if mode != Mode::Pomodoro {
            self.ring_alarm = false;
            self.ring_alarm_shake_frames = 0;
        }
        self.next_animation_at = scene.now_ms.wrapping_add(self.animation_period());
        if self.ready {
            let _ = self.render(screen, scene);
        }
    }

    /// Shows a panel over the screen, or takes it away; redraws only on a change.
    pub fn set_panel(&mut self, screen: &mut dyn Screen, scene: &Scene, panel: Option<Panel>) {
        if self.panel == panel {
            return;
        }
        self.panel = panel;
        if self.ready {
            let _ = self.render(screen, scene);
        }
    }

    /// While muted, a MUTE badge stays in the top left: mute is easy to forget, so it must stay visible.
    pub fn set_muted(&mut self, screen: &mut dyn Screen, scene: &Scene, muted: bool) {
        if self.muted == muted {
            return;
        }
        self.muted = muted;
        if self.ready {
            let _ = self.render(screen, scene);
        }
    }

    /// Redraws immediately. After a key press changes the pomodoro, don't wait for the next animation frame.
    pub fn refresh(&mut self, screen: &mut dyn Screen, scene: &Scene) {
        if self.ready {
            let _ = self.render(screen, scene);
        }
    }

    pub fn tick(&mut self, screen: &mut dyn Screen, scene: &Scene) {
        if !self.ready {
            return;
        }
        let now = scene.now_ms;
        if let Some(until) = self.identify_until {
            if now.wrapping_sub(until) as i32 >= 0 {
                self.identify_until = None;
                self.ensure_backlight(screen, true);
                self.next_animation_at = now;
            } else if now.wrapping_sub(self.identify_next_toggle) as i32 >= 0 {
                if screen.set_backlight(!self.backlight_on).is_ok() {
                    self.backlight_on = !self.backlight_on;
                }
                self.identify_next_toggle = now.wrapping_add(150);
            }
        }
        if (now.wrapping_sub(self.next_animation_at) as i32) < 0 {
            return;
        }
        self.animation_frame = self.animation_frame.wrapping_add(1);
        if self.ring_alarm_shake_frames > 0 {
            self.ring_alarm_shake_frames -= 1;
        }
        self.next_animation_at = now.wrapping_add(self.animation_period());
        if self.render(screen, scene).is_err() {
            self.next_animation_at = now.wrapping_add(1000);
        }
    }

    pub fn backlight_on(&self) -> bool {
        self.backlight_on
    }

    /// For previews: sets the animation frame, alarm shake and alarm switch directly, to compare against the C
    /// firmware's screens frame by frame.
    #[doc(hidden)]
    pub fn preview_pose(&mut self, frame: Option<u32>, shake_frames: Option<u32>, alarm: Option<bool>) {
        if let Some(frame) = frame {
            self.animation_frame = frame;
        }
        if let Some(shake_frames) = shake_frames {
            self.ring_alarm_shake_frames = shake_frames;
        }
        if let Some(alarm) = alarm {
            self.ring_alarm = alarm;
        }
    }
}

const PANEL_BORDER: u16 = COLOR_PET;
const PANEL_SELECTED: u16 = 0x2148;
const PANEL_LEFT: i32 = 44;
const PANEL_RIGHT: i32 = 276;

fn text_width(text: &[u8], scale: i32) -> i32 {
    if text.is_empty() { 0 } else { (text.len() as i32 * 6 - 1) * scale }
}

fn tone_color(tone: Tone) -> u16 {
    match tone {
        Tone::Plain => COLOR_TEXT,
        Tone::Dim => COLOR_MUTED,
        Tone::Accent => COLOR_PET_HIGHLIGHT,
        Tone::Warn => COLOR_WORKING,
    }
}

/// The menu panel: the screen underneath dimmed twice, a bordered box, the title, the lines, and
/// key hints along the bottom.
fn draw_panel(canvas: &mut Canvas, panel: &Panel) {
    canvas.dim();
    canvas.dim();
    canvas.fill_rect(30, 14, 260, 212, PANEL_BORDER);
    canvas.fill_rect(32, 16, 256, 208, COLOR_SCREEN);
    canvas.draw_text(PANEL_LEFT, 26, &panel.title, 2, COLOR_PET_HIGHLIGHT, 19);
    canvas.fill_rect(PANEL_LEFT, 46, PANEL_RIGHT - PANEL_LEFT, 1, COLOR_MUTED);
    for (index, (label, value, tone)) in panel.lines.iter().enumerate() {
        let index = index as i32;
        match panel.kind {
            PanelKind::List => {
                let y = 54 + index * 23;
                if panel.selected == Some(index as usize) {
                    canvas.fill_rect(38, y - 3, 244, 21, PANEL_SELECTED);
                    canvas.draw_text(42, y, b">", 2, COLOR_PET_HIGHLIGHT, 1);
                }
                canvas.draw_text(58, y, label, 2, COLOR_TEXT, 12);
                let value = &value[..value.len().min(8)];
                canvas.draw_text(PANEL_RIGHT - text_width(value, 2), y, value, 2, tone_color(*tone), 8);
            }
            PanelKind::Facts => {
                let y = 58 + index * 22;
                canvas.draw_text(PANEL_LEFT, y, label, 1, COLOR_MUTED, 12);
                canvas.draw_text(120, y - 3, value, 2, tone_color(*tone), 13);
            }
            PanelKind::Message => {
                canvas.draw_text(PANEL_LEFT, 62 + index * 14, label, 1, tone_color(*tone), 38);
            }
        }
    }
    canvas.fill_rect(PANEL_LEFT, 196, PANEL_RIGHT - PANEL_LEFT, 1, COLOR_MUTED);
    let mut x = PANEL_LEFT;
    for (key, action) in &panel.hints {
        canvas.fill_rect(x, 204, text_width(key, 1) + 6, 11, COLOR_MUTED);
        canvas.draw_text(x + 3, 206, key, 1, COLOR_SCREEN, 4);
        x += text_width(key, 1) + 10;
        canvas.draw_text(x, 206, action, 1, COLOR_TEXT, 12);
        x += text_width(action, 1) + 14;
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty() || haystack.windows(needle.len()).any(|window| window == needle)
}
