//! The korekore screens (ADR-0021; docs/design/korekore/README.md, "Box"): the same state, pages, taps and
//! animation pacing as the robot screens, drawn in the korekore look. Calm states live on Ink; a card that needs
//! you, one that is done and one that failed take over the screen. A child of `display`, so it reads the display's
//! state directly; the robot screens don't change.

use super::*;
use crate::korekore::Sprite;
use crate::korekore::glyphs::*;
use crate::korekore::sprites as kk;
use crate::korekore::text::{self as kt, Face};
use crate::korekore::theme::*;

/// Which set of screens the box draws (ADR-0021): the robot's, the default, or korekore's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screens {
    #[default]
    Robot,
    Korekore,
}

impl Screens {
    pub fn name(self) -> &'static [u8] {
        match self {
            Screens::Robot => b"ROBOT",
            Screens::Korekore => b"KOREKORE",
        }
    }

    pub fn parse(style: &str) -> Option<Self> {
        match style {
            "robot" => Some(Screens::Robot),
            "korekore" => Some(Screens::Korekore),
            _ => None,
        }
    }
}

/// Kore's six expressions; a Character's look shows the nearest of its four frames.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expr {
    Normal,
    Working,
    Input,
    Happy,
    Sad,
    Sleep,
}

impl Expr {
    fn sprite(self) -> &'static Sprite {
        match self {
            Expr::Normal => &kk::KORE_NORMAL,
            Expr::Working => &kk::KORE_WORKING,
            Expr::Input => &kk::KORE_INPUT,
            Expr::Happy => &kk::KORE_HAPPY,
            Expr::Sad => &kk::KORE_SAD,
            Expr::Sleep => &kk::KORE_SLEEP,
        }
    }

    fn frame(self) -> look::Frame {
        match self {
            Expr::Normal | Expr::Working | Expr::Input => look::Frame::Normal,
            Expr::Sleep => look::Frame::EyesClosed,
            Expr::Happy => look::Frame::Happy,
            Expr::Sad => look::Frame::Sad,
        }
    }
}

/// Kore is 20 × 18; this many pixels at scale 1.
const KORE_WIDTH: i32 = 20;
const KORE_HEIGHT: i32 = 18;
/// The design's "no link" orange for STG is not a token; the nearest is Claude Code's tile colour.
const STG: u16 = AGENT_CC;
/// The menu's level meter: low, middle, near the top.
const METER_LOW: u16 = MINT;

/// The card column on duty: x 104–312, from y 29, 39 high, 5 apart (the design's 7 px padding and 5 px gap).
fn card_rect(slot: usize) -> Rect {
    Rect { x: 104, y: 29 + slot as i32 * 44, width: 208, height: 39 }
}

/// Kore in the side panel on duty, at 4×.
const BUDDY: Rect = Rect { x: 8, y: 51, width: 80, height: 72 };
/// The page line and its bar under the cards.
const PAGE_LINE: Rect = Rect { x: 98, y: 216, width: 220, height: 22 };
/// The ticket on a takeover screen (needs input, done, failed).
const TICKET: Rect = Rect { x: 12, y: 112, width: 296, height: 42 };
const DONE_TICKET: Rect = Rect { x: 12, y: 150, width: 296, height: 42 };
const FAILED_TICKET: Rect = Rect { x: 15, y: 115, width: 290, height: 42 };
/// The cream card a panel is drawn on, 10 px in from the edges.
const PANEL_CARD: Rect = Rect { x: 10, y: 10, width: 300, height: 220 };
const PANEL_INNER_LEFT: i32 = 23;
const PANEL_INNER_RIGHT: i32 = 297;
/// The rows of a panel's list, 23 apart, under the title's rule.
fn panel_row(index: usize) -> Rect {
    Rect { x: PANEL_INNER_LEFT, y: 49 + index as i32 * 23, width: PANEL_INNER_RIGHT - PANEL_INNER_LEFT, height: 20 }
}
const PANEL_HINTS_Y: i32 = 208;
const PANEL_RULE_Y: i32 = 202;

// ---------- drawing helpers ----------

/// A sprite at a whole-number scale; mirrored left to right, or in grays for a lost link.
fn sprite(canvas: &mut Canvas, sprite: &Sprite, x: i32, y: i32, scale: i32, mirror: bool, gray: bool) {
    for row in 0..sprite.height {
        for column in 0..sprite.width {
            let source = if mirror { sprite.width - 1 - column } else { column };
            if let Some(color) = sprite.pixel(source, row) {
                let color = if gray { look::gray(color) } else { color };
                canvas.fill_rect(x + i32::from(column) * scale, y + i32::from(row) * scale, scale, scale, color);
            }
        }
    }
}

/// How far a rounded corner's row is inset: `row` from the top of a corner of `radius`.
fn corner_inset(radius: i32, row: i32) -> i32 {
    if row >= radius {
        return 0;
    }
    let dy = radius as f32 - row as f32 - 0.5;
    let dx = libm::sqrtf((radius * radius) as f32 - dy * dy);
    radius - libm::roundf(dx) as i32
}

fn fill_round(canvas: &mut Canvas, rect: Rect, radius: i32, color: u16) {
    let radius = radius.min(rect.width / 2).min(rect.height / 2);
    for row in 0..rect.height {
        let inset = corner_inset(radius, row.min(rect.height - 1 - row));
        canvas.fill_rect(rect.x + inset, rect.y + row, rect.width - 2 * inset, 1, color);
    }
}

/// A rounded rectangle with an outline `stroke` thick.
fn outlined_round(canvas: &mut Canvas, rect: Rect, radius: i32, stroke: i32, outline: u16, fill: u16) {
    fill_round(canvas, rect, radius, outline);
    let inner = Rect { x: rect.x + stroke, y: rect.y + stroke, width: rect.width - 2 * stroke, height: rect.height - 2 * stroke };
    fill_round(canvas, inner, (radius - stroke).max(0), fill);
}

/// Text set with CSS `line-height: 1`: the line is as tall as the type, so the face's own leading comes off the top.
fn tight(face: &Face, y: i32) -> i32 {
    y - (face.line() - i32::from(face.size)) / 2
}

/// A small chip: `text` on `fill`, radius 3, 4 px either side, as tall as the face's line. Returns its width.
fn chip(canvas: &mut Canvas, face: &Face, x: i32, y: i32, label: &[u8], ink: u16, fill: u16) -> i32 {
    let width = kt::width(face, label) + 8;
    fill_round(canvas, Rect { x, y, width, height: face.line() }, 3, fill);
    kt::draw(canvas, face, x + 4, y, label, ink, i32::MAX);
    width
}

/// A chip and what the key does, as the design's key hints are drawn. Returns the width used.
#[allow(clippy::too_many_arguments, reason = "where, what and in which colours")]
fn hint(canvas: &mut Canvas, face: &Face, x: i32, y: i32, key: &[u8], action: &[u8], chip_ink: u16, chip_fill: u16, color: u16) -> i32 {
    let used = chip(canvas, face, x, y, key, chip_ink, chip_fill) + 4;
    used + kt::draw(canvas, face, x + used, y, action, color, i32::MAX)
}

/// "▸", which DotGothic16 doesn't have: a small triangle pointing right.
fn pointer(canvas: &mut Canvas, x: i32, y: i32, color: u16) {
    for column in 0..4 {
        canvas.fill_rect(x + column, y + column, 1, 7 - 2 * column, color);
    }
}

/// The 12 × 7 link glyphs at 2×: a plug for USB, waves for the network, a cloud for the Relay.
fn link_glyph(canvas: &mut Canvas, path: LinkPath, right: i32, y: i32, color: u16) -> i32 {
    draw_icon(canvas, right - ICON_WIDTH * ICON_SCALE, y, path.icon(), color);
    ICON_WIDTH * ICON_SCALE
}

/// The design's 5 × 5 pixel heart at 2×.
fn heart(canvas: &mut Canvas, x: i32, y: i32) {
    const ROWS: [&[u8; 5]; 5] = [b".x.x.", b"xxxxx", b"xxxxx", b".xxx.", b"..x.."];
    for (row, cells) in ROWS.iter().enumerate() {
        for (column, &cell) in cells.iter().enumerate() {
            if cell == b'x' {
                canvas.fill_rect(x + column as i32 * 2, y + row as i32 * 2, 2, 2, PEACH);
            }
        }
    }
}

/// The agent's tile from the korekore icons, at 2×.
fn agent_tile(icon: AgentIcon) -> &'static Sprite {
    match icon {
        AgentIcon::ClaudeCode => &kk::ICON_CC,
        AgentIcon::Codex => &kk::ICON_CX,
        AgentIcon::OpenCode => &kk::ICON_OC,
        AgentIcon::Copilot => &kk::ICON_GH,
        AgentIcon::Pi => &kk::ICON_PI,
        AgentIcon::Ci => &kk::ICON_CI,
    }
}

/// Every pixel to the gray of its brightness, a little darker: the design's no-link filter.
fn gray_out(canvas: &mut Canvas) {
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let color = canvas.pixel((y * WIDTH + x) as usize);
            canvas.fill_rect(x, y, 1, 1, look::gray(color));
        }
    }
}

/// The colour of a card's state and the word on its pill.
fn pill(state: State) -> (u16, &'static [u8]) {
    let fill = match state {
        State::InputRequired => SUN,
        State::Done => MINT,
        State::Failed => PEACH,
        _ => SKY,
    };
    (fill, short_state_label(state))
}

/// A wrapped line's worth of `text`: as many whole words as fit in `max_width`, and the rest.
fn split_line<'t>(face: &Face, text: &'t [u8], max_width: i32) -> (&'t [u8], &'t [u8]) {
    if kt::width(face, text) <= max_width {
        return (text, &[]);
    }
    let mut cut = 0;
    for (index, &byte) in text.iter().enumerate() {
        if byte == b' ' {
            if kt::width(face, &text[..index]) > max_width {
                break;
            }
            cut = index;
        }
    }
    if cut == 0 {
        return (text, &[]);
    }
    (&text[..cut], &text[cut + 1..])
}

impl Display {
    /// Which screens the box draws.
    pub fn screens(&self) -> Screens {
        self.screens
    }

    /// Draws the robot's or korekore's screens from now on; redraws on a change.
    pub fn set_screens(&mut self, screen: &mut dyn Screen, scene: &Scene, screens: Screens) {
        if self.screens == screens {
            return;
        }
        self.screens = screens;
        self.highlight = None;
        if self.ready {
            let _ = self.render(screen, scene);
        }
    }

    /// The buddy at `scale` (Kore's): Kore, or the Character's look in the nearest frame, at the largest whole scale
    /// that keeps it as tall as Kore would be (at least 1×), standing on the same bottom line and centred on the same
    /// column. A look is 64 high and Kore 18, so a look draws at 1× where Kore draws at 4× to 6×.
    #[allow(clippy::too_many_arguments, reason = "where, how big and how")]
    fn kk_buddy(&self, canvas: &mut Canvas, expr: Expr, x: i32, y: i32, scale: i32, mirror: bool, gray: bool) {
        match &self.look {
            Some(look) => {
                let look_scale = (KORE_HEIGHT * scale / look::HEIGHT).max(1);
                let left = x + (KORE_WIDTH * scale - look::WIDTH * look_scale) / 2;
                let top = y + KORE_HEIGHT * scale - look::HEIGHT * look_scale;
                look.draw_scaled(canvas, expr.frame(), left, top, look_scale, mirror, gray);
            }
            None => sprite(canvas, expr.sprite(), x, y, scale, mirror, gray),
        }
    }

    /// The task on a takeover screen: the first in `state`, by its place in the stack.
    fn kk_ticket_task(&self, state: State) -> Option<usize> {
        self.tasks.iter().position(|task| task.state == state)
    }

    /// Which takeover is up on duty, if any, and its task.
    fn kk_takeover(&self) -> Option<(State, Option<usize>)> {
        if self.link_lost || self.mode != Mode::Duty {
            return None;
        }
        match self.state {
            State::InputRequired => self.kk_ticket_task(State::InputRequired).map(|index| (State::InputRequired, Some(index))),
            State::Done => Some((State::Done, self.kk_ticket_task(State::Done).or(if self.tasks.is_empty() { None } else { Some(0) }))),
            State::Failed => Some((State::Failed, self.kk_ticket_task(State::Failed))),
            _ => None,
        }
    }

    pub(super) fn render_korekore(&mut self, screen: &mut dyn Screen, scene: &Scene) -> Result<(), ()> {
        let leisure = scene.leisure.view(scene.now_ms);
        let mut dim = false;
        if self.mode == Mode::Leisure {
            if leisure.lights_out {
                self.ensure_backlight(screen, false);
                return Ok(());
            }
            dim = leisure.dim;
        }
        self.ensure_backlight(screen, true);
        let alarm = if self.mode == Mode::Pomodoro { self.ring_alarm_active(&scene.pomodoro.view(scene.now_ms)) } else { false };
        let mut canvas = Canvas::new(screen.frame());
        match self.mode {
            Mode::Pomodoro => self.kk_pomodoro(&mut canvas, scene, alarm),
            Mode::Leisure => self.kk_leisure(&mut canvas, scene, &leisure),
            Mode::Duty => self.kk_duty(&mut canvas, scene),
        }
        if dim {
            canvas.dim();
        }
        if let Some(panel) = &self.panel {
            self.kk_panel(&mut canvas, panel);
        }
        if let Some((target, _)) = self.highlight
            && let Some(rect) = self.kk_target_rect(target)
        {
            rect.outline(&mut canvas, SKY);
        }
        screen.present()
    }

    // ---------- duty ----------

    fn kk_duty(&self, canvas: &mut Canvas, scene: &Scene) {
        if self.link_lost && self.tasks.is_empty() {
            self.kk_no_link(canvas);
            return;
        }
        match self.kk_takeover() {
            Some((State::InputRequired, Some(index))) => self.kk_needs_input(canvas, scene, index),
            Some((State::Done, index)) => self.kk_done(canvas, scene, index),
            Some((State::Failed, index)) => self.kk_failed(canvas, scene, index),
            _ => {
                self.kk_on_duty(canvas, scene);
                if self.link_lost {
                    gray_out(canvas);
                }
            }
        }
    }

    /// The state's word, colour, Kore's expression and the mark over its head.
    fn kk_state_style(&self) -> (&'static [u8], u16, Expr, &'static [u8]) {
        if self.link_lost {
            return (b"NO LINK", MIST, Expr::Sleep, b"z Z");
        }
        match self.state {
            State::Working => (b"WORKING", SKY, Expr::Working, [b"." as &[u8], b". .", b". . ."][(self.animation_frame % 3) as usize]),
            State::InputRequired => (b"ASK", SUN, Expr::Input, b"!?"),
            State::Done => (b"DONE", MINT, Expr::Happy, b""),
            State::Failed => (b"FAIL", PEACH, Expr::Sad, b""),
            _ => (b"READY", LAVENDER, Expr::Normal, b""),
        }
    }

    /// The line under the state word: on a lost link where it waits, while idle today's numbers in turn.
    fn kk_sub_line<'s>(&'s self, scene: &Scene, tally: &'s [u8]) -> &'s [u8] {
        if self.link_lost {
            return "waiting…".as_bytes();
        }
        let pomodoro = scene.pomodoro.view(scene.now_ms);
        let mut lines: [&[u8]; 1 + MAX_STATS] = [&[]; 1 + MAX_STATS];
        let mut count = 0;
        for stat in &self.stats {
            lines[count] = stat;
            count += 1;
        }
        if pomodoro.completed > 0 {
            lines[count] = tally;
            count += 1;
        }
        if count == 0 {
            return b"";
        }
        // The design rotates every 3 s; an idle frame is 500 ms.
        lines[(self.animation_frame / IDLE_ROTATE_FRAMES) as usize % count]
    }

    fn kk_on_duty(&self, canvas: &mut Canvas, scene: &Scene) {
        let (word, color, mut expr, mut mark) = self.kk_state_style();
        canvas.fill_rect(0, 0, WIDTH, HEIGHT, BOX_GROUND);
        canvas.fill_rect(0, 0, WIDTH, 3, color);

        // The side panel: korekore, the mark, Kore, the state and today's line.
        canvas.fill_rect(0, 3, 96, HEIGHT - 3, BOX_CARD);
        kt::draw_bitmap(canvas, &WORDMARK_14, 48 - i32::from(WORDMARK_14.width) / 2, 11, KORE_PINK);
        let petted = self.petted(scene.now_ms);
        let mut hop = 0;
        if petted {
            expr = Expr::Happy;
            hop = if self.animation_frame.is_multiple_of(2) { -3 } else { 0 };
        } else if !self.link_lost && self.state == State::Idle {
            match idle_mood(self.animation_frame) {
                IdleMood::Nap => (expr, mark) = (Expr::Sleep, b"Z z"),
                IdleMood::Look => (expr, mark) = (Expr::Working, b"< >"),
                IdleMood::Stretch => (expr, mark, hop) = (Expr::Happy, b"^", -3),
                IdleMood::None => {
                    if self.animation_frame % 8 == 7 {
                        expr = Expr::Sleep;
                    }
                }
            }
        } else if self.state == State::Working {
            hop = if self.animation_frame.is_multiple_of(2) { 0 } else { -1 };
        }
        kt::draw_centered(canvas, &DOT_11, 48, 31, mark, color);
        let mirror = !self.link_lost && self.state == State::Idle && idle_mood(self.animation_frame) == IdleMood::Look && self.animation_frame % 4 < 2;
        self.kk_buddy(canvas, expr, BUDDY.x, BUDDY.y + hop, 4, mirror, self.link_lost);
        if petted && !self.link_lost {
            heart(canvas, BUDDY.x + BUDDY.width, BUDDY.y - 10 + hop);
        }
        kt::draw_centered(canvas, &DOT_14, 48, 201, word, color);
        let tally = format_tally(scene.pomodoro.view(scene.now_ms).completed, scene.pomodoro.view(scene.now_ms).focus_s);
        let sub = self.kk_sub_line(scene, tally.as_bytes());
        let sub = &sub[..font::fit(sub, 1, 90).min(sub.len())];
        kt::draw_centered(canvas, &DOT_9, 48, 222, sub, BOX_TEXT_MUTED);

        self.kk_top_line(canvas, scene);
        if self.tasks.is_empty() {
            let quiet: &[u8] = if self.link_lost { b"" } else { b"ALL QUIET" };
            kt::draw_centered(canvas, &DOT_12, 208, 100, quiet, BOX_TEXT);
            let mut stats = Text::<64>::new();
            for (index, stat) in self.stats.iter().enumerate() {
                if index > 0 {
                    stats.push_bytes(" · ".as_bytes());
                }
                stats.push_bytes(stat);
            }
            kt::draw_centered(canvas, &DOT_10, 208, 120, stats.as_bytes(), BOX_TEXT_MUTED);
            return;
        }
        for (slot, task) in self.tasks_on_page().iter().enumerate() {
            self.kk_card(canvas, scene, task, card_rect(slot), BOX_CARD, 8, &DOT_11, false);
        }
        kt::draw(canvas, &DOT_9, 104, 223, self.page_line().as_bytes(), BOX_TEXT_MUTED, 150);
        let pages = self.pages() as i32;
        if pages > 1 {
            for page in 0..pages {
                let x = 312 - (pages - page) * 17 + 3;
                let fill = if page == self.page as i32 { BOX_TEXT } else { BOX_FACE };
                fill_round(canvas, Rect { x, y: 227, width: 14, height: 4 }, 2, fill);
            }
        }
    }

    /// The top line of the card column: MUTE and ON DUTY on the left; the Pomodoro badge, the link and STG on the right.
    fn kk_top_line(&self, canvas: &mut Canvas, scene: &Scene) {
        let mut x = 104;
        if self.muted {
            x += chip(canvas, &DOT_9, x, 11, b"MUTE", BOX_GROUND, BOX_TEXT) + 6;
        }
        kt::draw(canvas, &DOT_9, x, 11, b"ON DUTY", BOX_TEXT_MUTED, 100);
        let mut right = 312;
        if self.link.1 {
            right -= kt::width(&DOT_9, b"STG");
            kt::draw(canvas, &DOT_9, right, 11, b"STG", STG, i32::MAX);
            right -= 6;
        }
        if let Some(path) = self.link.0 {
            right -= link_glyph(canvas, path, right, 10, BOX_TEXT) + 6;
        }
        let view = scene.pomodoro.view(scene.now_ms);
        if !view.is_idle() && !(view.run == Run::Paused && self.animation_frame % 2 == 1) {
            let mut badge = Text::<11>::new();
            badge.push_bytes(if view.phase == Phase::Focus { b"F " } else { b"B " });
            badge.push_bytes(format_countdown(view.remaining_ms).as_bytes());
            let width = kt::width(&DOT_10, badge.as_bytes()) + 8;
            chip(canvas, &DOT_10, right - width, 10, badge.as_bytes(), BOX_GROUND, KORE_PINK);
        }
    }

    /// One card: the agent's tile, the title, project and time, and the state's pill.
    #[allow(clippy::too_many_arguments, reason = "the card and how it is drawn")]
    fn kk_card(&self, canvas: &mut Canvas, scene: &Scene, task: &Task, rect: Rect, fill: u16, radius: i32, title_face: &Face, takeover: bool) {
        fill_round(canvas, rect, radius, fill);
        let padding = if takeover { 7 } else { 6 };
        let (pill_fill, pill_word) = pill(task.state);
        let pill_width = kt::width(&DOT_9, pill_word) + 8;
        let pill_x = rect.x + rect.width - padding - pill_width;
        chip(canvas, &DOT_9, pill_x, rect.y + (rect.height - DOT_9.line()) / 2, pill_word, BOX_GROUND, pill_fill);
        let mut text_x = rect.x + padding;
        let mut title = &task.title[..];
        if let Some(icon) = task.icon {
            sprite(canvas, agent_tile(icon), text_x, rect.y + (rect.height - 24) / 2, 2, false, false);
            text_x += 24 + 7;
            title = title.strip_prefix(icon.prefix()).unwrap_or(title);
        }
        let text_width = pill_x - 7 - text_x;
        let title_y = rect.y + padding;
        kt::draw(canvas, title_face, text_x, title_y, title, BOX_TEXT, text_width);
        let line_y = title_y + title_face.line() + 3;
        let elapsed = format_elapsed(self.task_elapsed_seconds(task, scene.now_ms));
        let time: Text<24> = match (takeover, task.state) {
            (true, State::InputRequired) => text!(24, "waiting {}", core::str::from_utf8(elapsed.as_bytes()).unwrap_or("")),
            (true, _) => Text::new(),
            _ => text!(24, "{}", core::str::from_utf8(elapsed.as_bytes()).unwrap_or("")),
        };
        let time_color = if task.state == State::InputRequired && !takeover { SUN } else { BOX_TEXT_MUTED };
        let mut line = Text::<80>::new();
        if !task.project.is_empty() && !contains(&task.title, &task.project) {
            line.push_bytes(&task.project);
        }
        if takeover {
            if !line.is_empty() && !time.is_empty() {
                line.push_bytes(" · ".as_bytes());
            }
            line.push_bytes(time.as_bytes());
            kt::draw(canvas, &DOT_9, text_x, line_y, line.as_bytes(), BOX_TEXT_MUTED, text_width);
        } else {
            let time_width = kt::width(&DOT_9, time.as_bytes());
            kt::draw(canvas, &DOT_9, text_x, line_y, line.as_bytes(), BOX_TEXT_MUTED, text_width - time_width - 6);
            kt::draw_right(canvas, &DOT_9, pill_x - 7, line_y, time.as_bytes(), time_color);
        }
    }

    /// A card waits for an answer: the whole screen in `sun`.
    fn kk_needs_input(&self, canvas: &mut Canvas, scene: &Scene, index: usize) {
        canvas.fill_rect(0, 0, WIDTH, HEIGHT, SUN);
        let bob = if self.animation_frame.is_multiple_of(2) { -2 } else { 0 };
        self.kk_buddy(canvas, Expr::Input, 12, 10 + bob, 5, false, false);
        kt::draw(canvas, &DOT_34, 122, tight(&DOT_34, 20), "これこれ!".as_bytes(), BOX_GROUND, i32::MAX);
        kt::draw(canvas, &DOT_16, 122, tight(&DOT_16, 60), b"KORE KORE!", BOX_GROUND, i32::MAX);
        kt::draw(canvas, &DOT_10, 122, 79, b"this one needs you", BOX_GROUND, i32::MAX);
        self.kk_card(canvas, scene, &self.tasks[index], TICKET, BOX_GROUND, 10, &DOT_12, true);
        let others = self.tasks.iter().filter(|task| task.state == State::InputRequired).count() - 1;
        if others > 0 {
            let more = text!(24, "+{} more waiting", others % 100);
            kt::draw(canvas, &DOT_10, 12, 218, more.as_bytes(), BOX_GROUND, i32::MAX);
        }
        self.kk_key_hint_right(canvas, 308, 218, b"take me there", BOX_GROUND, SUN, BOX_GROUND);
    }

    /// K2 and what it does, against the right edge.
    #[allow(clippy::too_many_arguments, reason = "where and in which colours")]
    fn kk_key_hint_right(&self, canvas: &mut Canvas, right: i32, y: i32, action: &[u8], chip_fill: u16, chip_ink: u16, color: u16) {
        let width = kt::width(&DOT_10, b"K2") + 8 + 5 + kt::width(&DOT_10, action);
        hint(canvas, &DOT_10, right - width + 1, y, b"K2", action, chip_ink, chip_fill, color);
    }

    /// Done: 3 s in `mint` with sparkles, then back on duty (the firmware settles after the line).
    fn kk_done(&self, canvas: &mut Canvas, scene: &Scene, index: Option<usize>) {
        canvas.fill_rect(0, 0, WIDTH, HEIGHT, MINT);
        let hop = if self.animation_frame.is_multiple_of(2) { -6 } else { 0 };
        self.kk_buddy(canvas, Expr::Happy, 110, 14 + hop, 5, false, false);
        let twinkle = self.animation_frame % 2;
        let left: [(i32, i32, &[u8]); 3] = [(86, 16, b"+"), (96, 44, b"*"), (86, 72, b"+")];
        let right: [(i32, i32, &[u8]); 3] = [(234, 16, b"*"), (224, 44, b"+"), (230, 72, b"*")];
        for (place, (x, y, mark)) in left.iter().chain(right.iter()).enumerate() {
            if place as u32 % 2 == twinkle || place < 2 {
                kt::draw(canvas, &DOT_14, *x, *y, mark, BOX_GROUND, i32::MAX);
            }
        }
        kt::draw_centered(canvas, &DOT_36, 160, tight(&DOT_36, 108), b"YATTA!", BOX_GROUND);
        if let Some(index) = index {
            self.kk_card(canvas, scene, &self.tasks[index], DONE_TICKET, BOX_GROUND, 10, &DOT_12, true);
        }
    }

    /// Failed: gentle, on Ink with a `peach` border, never a red alarm.
    fn kk_failed(&self, canvas: &mut Canvas, scene: &Scene, index: Option<usize>) {
        canvas.fill_rect(0, 0, WIDTH, HEIGHT, PEACH);
        canvas.fill_rect(3, 3, WIDTH - 6, HEIGHT - 6, BOX_GROUND);
        self.kk_buddy(canvas, Expr::Sad, 15, 15, 5, false, false);
        kt::draw(canvas, &DOT_22, 127, tight(&DOT_22, 26), b"daijoubu,", PEACH, i32::MAX);
        kt::draw(canvas, &DOT_22, 127, tight(&DOT_22, 52), b"daijoubu.", PEACH, i32::MAX);
        kt::draw(canvas, &DOT_10, 127, 80, b"it happens. have a look?", BOX_TEXT_MUTED, i32::MAX);
        if let Some(index) = index {
            self.kk_card(canvas, scene, &self.tasks[index], FAILED_TICKET, BOX_CARD, 10, &DOT_12, true);
        }
        self.kk_key_hint_right(canvas, 305, 213, b"open the run", BOX_TEXT, BOX_GROUND, BOX_TEXT);
    }

    /// No link and nothing kept: honest gray, Kore asleep.
    fn kk_no_link(&self, canvas: &mut Canvas) {
        canvas.fill_rect(0, 0, WIDTH, HEIGHT, BOX_GRAY_GROUND);
        self.kk_buddy(canvas, Expr::Sleep, 93, 41, 6, false, true);
        let float = (self.animation_frame % 4) as i32;
        kt::draw(canvas, &DOT_14, 227, 45 - float, b"Z", MIST, i32::MAX);
        kt::draw(canvas, &DOT_14, 217, 62 - float, b"z", MIST, i32::MAX);
        kt::draw_centered(canvas, &DOT_22, 160, tight(&DOT_22, 155), b"NO LINK", MIST);
        kt::draw_centered(canvas, &DOT_10, 160, 187, b"waiting for the computer", MIST_TEXT);
    }

    // ---------- pomodoro ----------

    fn kk_pomodoro(&self, canvas: &mut Canvas, scene: &Scene, alarm: bool) {
        let view = scene.pomodoro.view(scene.now_ms);
        let color = if view.is_idle() {
            LAVENDER
        } else if view.phase == Phase::Break {
            MINT
        } else {
            KORE_PINK
        };
        if alarm {
            self.kk_pomodoro_ended(canvas, &view);
            return;
        }
        let paused = view.run == Run::Paused;
        canvas.fill_rect(0, 0, WIDTH, HEIGHT, BOX_GROUND);
        canvas.fill_rect(0, 0, WIDTH, 3, color);

        // The dial: 60 ticks a minute each, the minutes gone lit, and a hand.
        let (center_x, center_y) = (78.0f32, 121.0f32);
        let minutes = if view.run == Run::Pending { 0 } else { ((view.total_ms - view.remaining_ms) / 60_000) as i32 };
        for tick in 0..60 {
            let fill = if tick < minutes { color } else { BOX_FACE };
            fill_rotated(canvas, center_x, center_y, tick as f32 * 6.0, 62.0, 4.0, 9.0, fill);
        }
        fill_rotated(canvas, center_x, center_y, minutes as f32 * 6.0, 44.0, 4.0, 18.0, BOX_TEXT);
        if !paused || self.animation_frame.is_multiple_of(2) {
            let digits = if paused { BOX_TEXT_MUTED } else { BOX_TEXT };
            kt::draw_centered(canvas, &DOT_38, 78, tight(&DOT_38, 99), format_countdown(view.remaining_ms).as_bytes(), digits);
        }
        let under: &[u8] = match (view.is_idle(), view.phase, paused) {
            (true, _, _) => b"FOCUS 25 MIN",
            (_, _, true) => b"paused",
            (_, Phase::Break, _) => b"rest your eyes",
            _ => b"ganbatte!",
        };
        kt::draw_centered(canvas, &DOT_10, 78, 143, under, color);

        // The column on the right: the phase, the keys, today, the buddy and the agents.
        let label: &[u8] = if view.is_idle() {
            b"READY"
        } else if view.phase == Phase::Break {
            b"BREAK"
        } else {
            b"FOCUS"
        };
        let label_width = kt::draw(canvas, &DOT_22, 164, tight(&DOT_22, 11), label, color, i32::MAX);
        if paused {
            chip(canvas, &DOT_9, 164 + label_width + 6, 16, b"PAUSED", BOX_GROUND, BOX_TEXT);
        }
        let hints: [(&[u8], &[u8]); 2] = match (view.run, view.is_idle()) {
            (Run::Pending, true) => [(b"K0", b"start focus"), (b"", b"")],
            (Run::Pending, false) => [(b"K0", b"start break"), (b"HOLD K0", b"skip")],
            (Run::Paused, _) => [(b"K0", b"resume"), (b"HOLD K0", b"stop")],
            (Run::Running, _) => [(b"K0", b"pause"), (b"HOLD K0", b"stop")],
        };
        for (row, (key, action)) in hints.iter().enumerate() {
            if !key.is_empty() {
                hint(canvas, &DOT_9, 164, 38 + row as i32 * 14, key, action, BOX_GROUND, BOX_TEXT, BOX_TEXT_MUTED);
            }
        }
        kt::draw(canvas, &DOT_9, 164, 70, b"TODAY", BOX_TEXT_MUTED, i32::MAX);
        let squares = view.completed.clamp(4, 12) as i32;
        for square in 0..squares {
            let fill = if (square as u32) < view.completed { KORE_PINK } else { BOX_FACE };
            fill_round(canvas, Rect { x: 164 + square * 11, y: 84, width: 8, height: 8 }, 2, fill);
        }
        kt::draw(canvas, &DOT_10, 164, 95, format_tally(view.completed, view.focus_s).as_bytes(), KORE_PINK, 148);

        let expr = match (view.is_idle(), view.phase, paused) {
            (true, _, _) | (_, _, true) => Expr::Normal,
            (_, Phase::Break, _) => Expr::Sleep,
            _ => Expr::Working,
        };
        self.kk_buddy(canvas, expr, 164, 198, 2, false, false);
        self.kk_agent_strip(canvas, Rect { x: 210, y: 203, width: 102, height: 31 });
    }

    /// The agents during a Pomodoro: they never take the screen, only this strip.
    fn kk_agent_strip(&self, canvas: &mut Canvas, rect: Rect) {
        let asking = self.tasks.iter().any(|task| task.state == State::InputRequired);
        let (fill, ink, word_color, word): (u16, u16, u16, &[u8]) = if self.link_lost {
            (BOX_GRAY_GROUND, MIST_TEXT, MIST, b"NO LINK")
        } else if asking {
            (SUN, BOX_GROUND, BOX_GROUND, b"KORE KORE! ASK")
        } else {
            match self.state {
                State::Working => (BOX_CARD, BOX_TEXT, SKY, b"AGENTS WORKING"),
                State::Done => (BOX_CARD, BOX_TEXT, MINT, b"DONE"),
                State::Failed => (BOX_CARD, BOX_TEXT, PEACH, b"FAILED"),
                _ => (BOX_CARD, BOX_TEXT_MUTED, BOX_TEXT_MUTED, b"NO AGENTS"),
            }
        };
        fill_round(canvas, rect, 6, fill);
        kt::draw(canvas, &DOT_9, rect.x + 5, rect.y + 4, word, word_color, rect.width - 10);
        let title: &[u8] = if self.link_lost {
            b"timer keeps going"
        } else {
            let task = self.tasks.iter().find(|task| task.state == State::InputRequired).or(self.tasks.first());
            task.map(|task| task.icon.and_then(|icon| task.title.strip_prefix(icon.prefix())).unwrap_or(&task.title)).unwrap_or(b"all quiet")
        };
        kt::draw(canvas, &DOT_9, rect.x + 5, rect.y + 16, title, ink, rect.width - 10);
    }

    /// A phase just ended: the screen fills, pink after focus and lavender after a break, until a key.
    fn kk_pomodoro_ended(&self, canvas: &mut Canvas, view: &pomodoro::View) {
        // After focus ends the next phase, a break, waits to start.
        let focus_ended = view.phase == Phase::Break;
        let ground = if focus_ended { KORE_PINK } else { LAVENDER };
        canvas.fill_rect(0, 0, WIDTH, HEIGHT, ground);
        let tally = format_tally(view.completed, view.focus_s);
        kt::draw(canvas, &DOT_10, 14, 15, b"POMODORO", BOX_GROUND, i32::MAX);
        kt::draw_right(canvas, &DOT_10, 306, 15, tally.as_bytes(), BOX_GROUND);
        let hop = if self.animation_frame.is_multiple_of(2) { -4 } else { 0 };
        self.kk_buddy(canvas, Expr::Happy, 14, 35 + hop, 5, false, false);
        let line: &[u8] = if focus_ended { b"Nice work. Time for a break!" } else { b"Break's over. Ready?" };
        let bubble = Rect { x: 126, y: 41, width: 180, height: 54 };
        outlined_round(canvas, bubble, 10, 2, BOX_GROUND, CREAM);
        let (first, rest) = split_line(&DOT_12, line, bubble.width - 16);
        kt::draw(canvas, &DOT_12, bubble.x + 8, bubble.y + 8, first, BOX_GROUND, bubble.width - 16);
        kt::draw(canvas, &DOT_12, bubble.x + 8, bubble.y + 8 + DOT_12.line(), rest, BOX_GROUND, bubble.width - 16);
        let big: &[u8] = if focus_ended { b"BREAK 05:00" } else { b"FOCUS 25:00" };
        kt::draw(canvas, &DOT_30, 14, tight(&DOT_30, 133), big, BOX_GROUND, i32::MAX);
        let hints: [(&[u8], &[u8]); 2] = if focus_ended { [(b"K0", b"start break"), (b"HOLD K0", b"skip")] } else { [(b"K0", b"start focus"), (b"K1", b"on duty")] };
        let mut x = 14;
        for (key, action) in hints {
            x += hint(canvas, &DOT_10, x, 216, key, action, CREAM, BOX_GROUND, BOX_GROUND) + 10;
        }
    }

    // ---------- leisure ----------

    fn kk_leisure(&self, canvas: &mut Canvas, scene: &Scene, view: &leisure::View) {
        let frame = view.skit_frame;
        let ground = if view.skit == Skit::Stars { BOX_DIM } else { BOX_GROUND };
        canvas.fill_rect(0, 0, WIDTH, HEIGHT, ground);
        kt::draw(canvas, &DOT_9, 8, 6, b"LEISURE", BOX_TEXT_MUTED, i32::MAX);
        let pomodoro = scene.pomodoro.view(scene.now_ms);
        if !pomodoro.is_idle() {
            let mut badge = Text::<11>::new();
            badge.push_bytes(if pomodoro.phase == Phase::Focus { b"F " } else { b"B " });
            badge.push_bytes(format_countdown(pomodoro.remaining_ms).as_bytes());
            let width = kt::width(&DOT_10, badge.as_bytes()) + 8;
            chip(canvas, &DOT_10, 312 - width, 5, badge.as_bytes(), BOX_GROUND, KORE_PINK);
        } else if let Some(stat) = self.stats.first() {
            kt::draw_right(canvas, &DOT_9, 312, 6, stat, BOX_TEXT_MUTED);
        }
        canvas.fill_rect(0, 198, WIDTH, 2, BOX_FACE);

        let resting = if frame % 24 == 23 { Expr::Sleep } else { Expr::Normal };
        let breathe = ((frame / 8) % 2) as i32;
        // Where Kore stands (its top left at 4×, standing on the floor) and how.
        let (mut x, mut y, mut expr, mut mirror) = (120, 126 + breathe, resting, false);
        let looks = self.look.is_some();
        match view.skit {
            Skit::Patrol => {
                let (dx, walking, left) = patrol_path(frame);
                x = 120 + dx;
                y = 126 - if walking && frame % 4 >= 2 { 2 } else { 0 };
                (expr, mirror) = (Expr::Working, left);
                let arrows: &[u8] = if left { b"< < <" } else { b"> > >" };
                if walking {
                    kt::draw(canvas, &DOT_12, if left { x - 50 } else { x + 90 }, 168, arrows, BOX_TEXT_MUTED, i32::MAX);
                }
            }
            Skit::Ball if !looks => {
                (x, expr) = (100, Expr::Happy);
                let t = (frame % 24) as f32 / 24.0;
                let ball_x = 190 + (frame % 24) as i32 * 2;
                let ball_y = 168 - (30.0 * libm::sinf(core::f32::consts::PI * t)) as i32;
                outlined_round(canvas, Rect { x: ball_x, y: ball_y, width: 22, height: 22 }, 11, 3, KORE_PINK, BOX_TEXT);
            }
            Skit::Read if !looks => {
                (x, expr) = (100, Expr::Working);
                canvas.fill_rect(170, 150, 34, 24, BOX_TEXT);
                canvas.fill_rect(172, 152, 30, 20, SKY);
                canvas.fill_rect(186, 152, 2, 20, BOX_TEXT);
                if frame >= 60 {
                    kt::draw(canvas, &DOT_16, 150, 104, b"!", SUN, i32::MAX);
                }
            }
            Skit::Stars => {
                const SKY_STARS: [(i32, i32); 7] = [(30, 40), (80, 70), (140, 30), (210, 55), (270, 35), (250, 95), (40, 100)];
                let count = (frame / 12 + 1).min(7) as usize;
                for &(star_x, star_y) in &SKY_STARS[..count] {
                    kt::draw(canvas, &DOT_12, star_x, star_y, b"+", SUN, i32::MAX);
                }
                if frame >= 88 {
                    expr = Expr::Sleep;
                } else {
                    expr = Expr::Normal;
                    let number = text!(4, "{}", count);
                    kt::draw(canvas, &DOT_18, 210, 104, number.as_bytes(), SUN, i32::MAX);
                }
            }
            Skit::Hide => {
                let dx = hide_path(frame);
                x = 120 + dx;
                (expr, mirror) = (Expr::Happy, hide_path(frame + 1) < dx);
                if (28..52).contains(&frame) {
                    kt::draw(canvas, &DOT_12, 236, 112, b"hi!", BOX_TEXT, i32::MAX);
                }
            }
            Skit::Startle => match frame {
                24..32 => {
                    (expr, y) = (Expr::Input, 120);
                    kt::draw(canvas, &DOT_22, 210, 98, b"!", SUN, i32::MAX);
                }
                32..56 => (expr, mirror) = (Expr::Input, (36..44).contains(&frame)),
                _ => expr = Expr::Sleep,
            },
            Skit::Dream => {
                (x, expr) = (60, Expr::Sleep);
                fill_round(canvas, Rect { x: 150, y: 70, width: 150, height: 24 }, 10, BOX_FACE);
                let line: &[u8] = if self.stats.is_empty() { b"zzz" } else { &self.stats[(frame / 32) as usize % self.stats.len()] };
                kt::draw(canvas, &DOT_10, 158, 76, line, BOX_TEXT, 134);
            }
            Skit::Sleep => {
                expr = Expr::Sleep;
                kt::draw(canvas, &DOT_16, 210, 110 - (frame % 16) as i32, b"Z", LAVENDER, i32::MAX);
            }
            Skit::Ball | Skit::Read | Skit::None => {}
        }
        if self.look.is_some() {
            // A look has the room to stand at 2× here: as tall as Kore at 8×, on the same floor and column.
            self.kk_buddy(canvas, expr, x + 40 - 80, y + 72 - 144, 8, mirror, false);
        } else {
            self.kk_buddy(canvas, expr, x, y, 4, mirror, false);
        }
    }

    // ---------- panels ----------

    /// A panel (the menu, a voice command, a transfer) on a cream card over the dimmed screen.
    fn kk_panel(&self, canvas: &mut Canvas, panel: &Panel) {
        canvas.fill_rect(0, 0, WIDTH, HEIGHT, BOX_DIM);
        outlined_round(canvas, PANEL_CARD, 12, 3, BOX_GROUND, CREAM);
        kt::draw(canvas, &DOT_16, PANEL_INNER_LEFT, 21, &panel.title, title_color(&panel.title), PANEL_INNER_RIGHT - PANEL_INNER_LEFT);
        canvas.fill_rect(PANEL_INNER_LEFT, 44, PANEL_INNER_RIGHT - PANEL_INNER_LEFT, 2, BOX_GROUND);
        let tone = |tone: Tone| match tone {
            Tone::Plain => BOX_GROUND,
            Tone::Dim => MUTED_2,
            Tone::Accent => DONE_TEXT,
            Tone::Warn => KORE_PINK_TEXT,
        };
        match panel.kind {
            PanelKind::List | PanelKind::Facts => {
                for (index, (label, value, value_tone)) in panel.lines.iter().enumerate() {
                    let row = panel_row(index);
                    let selected = panel.kind == PanelKind::List && panel.selected == Some(index);
                    if selected {
                        fill_round(canvas, row, 4, KORE_PINK);
                        pointer(canvas, row.x + 6, row.y + 7, BOX_GROUND);
                    }
                    // A Facts label's second line is a note the robot draws small; one line here.
                    let label = label.split(|&byte| byte == b'\n').next().unwrap_or_default();
                    kt::draw(canvas, &DOT_13, row.x + 14, row.y + 2, label, BOX_GROUND, 150);
                    kt::draw_right(canvas, &DOT_13, row.x + row.width - 6, row.y + 2, value, tone(*value_tone));
                }
            }
            PanelKind::Meter(level) => {
                self.kk_buddy(canvas, Expr::Input, PANEL_INNER_LEFT, 56, 3, false, false);
                let steps = crate::voice_command::METER_STEPS as i32;
                let left = PANEL_INNER_LEFT + 70;
                let bar = (PANEL_INNER_RIGHT - left - 2 * (steps - 1)) / steps;
                for step in 0..steps {
                    let lit = step < i32::from(level);
                    let height = 6 + step * 28 / steps;
                    let color = match step * 16 / steps {
                        _ if !lit => BACKDROP,
                        0..=9 => METER_LOW,
                        10..=12 => SUN,
                        _ => AGENT_CC,
                    };
                    canvas.fill_rect(left + step * (bar + 2), 90 - height, bar, height, color);
                }
                if let Some((label, _, line_tone)) = panel.lines.first() {
                    kt::draw(canvas, &DOT_12, left, 98, label, tone(*line_tone), PANEL_INNER_RIGHT - left);
                }
            }
            PanelKind::Text(scale) => {
                let face = if scale >= 2 { &DOT_16 } else { &DOT_11 };
                for (index, (label, _, line_tone)) in panel.lines.iter().enumerate() {
                    let y = 50 + index as i32 * (face.line() + 1);
                    if y + face.line() > PANEL_RULE_Y {
                        break;
                    }
                    kt::draw(canvas, face, PANEL_INNER_LEFT, y, label, tone(*line_tone), PANEL_INNER_RIGHT - PANEL_INNER_LEFT);
                }
            }
            PanelKind::Progress(percent) => {
                self.kk_buddy(canvas, Expr::Working, PANEL_INNER_LEFT, 56, 3, false, false);
                let bar = Rect { x: PANEL_INNER_LEFT + 70, y: 70, width: PANEL_INNER_RIGHT - PANEL_INNER_LEFT - 70, height: 16 };
                outlined_round(canvas, bar, 8, 2, BOX_GROUND, PAPER);
                let lit = (i32::from(percent.min(100)) + 2) / 5;
                let segment = (bar.width - 8 - 19 * 2) / 20;
                for index in 0..lit {
                    canvas.fill_rect(bar.x + 4 + index * (segment + 2), bar.y + 4, segment, bar.height - 8, KORE_PINK);
                }
                for (index, (label, _, line_tone)) in panel.lines.iter().enumerate() {
                    kt::draw(canvas, &DOT_12, bar.x, 94 + index as i32 * 16, label, tone(*line_tone), bar.width);
                }
            }
        }
        canvas.fill_rect(PANEL_INNER_LEFT, PANEL_RULE_Y, PANEL_INNER_RIGHT - PANEL_INNER_LEFT, 2, BOX_GROUND);
        for (rect, (key, action)) in kk_hint_rects(panel).iter().zip(&panel.hints) {
            hint(canvas, &DOT_10, rect.x + 2, PANEL_HINTS_Y, key, action, CREAM, BOX_GROUND, BOX_GROUND);
        }
    }

    // ---------- taps ----------

    pub(super) fn kk_target_rect(&self, target: Target) -> Option<Rect> {
        match (target, &self.panel) {
            (Target::Row(index), Some(panel)) if panel.kind == PanelKind::List && index < panel.lines.len() => Some(panel_row(index)),
            (Target::Hint(index), Some(panel)) => kk_hint_rects(panel).get(index).copied(),
            (Target::Card(index), None) if self.mode == Mode::Duty && index < self.tasks.len() => match self.kk_takeover() {
                Some((State::InputRequired, Some(shown))) if shown == index => Some(TICKET),
                Some((State::Done, Some(shown))) if shown == index => Some(DONE_TICKET),
                Some((State::Failed, Some(shown))) if shown == index => Some(FAILED_TICKET),
                Some(_) => None,
                None => index.checked_sub(self.page * CARDS_PER_PAGE).filter(|&slot| slot < CARDS_PER_PAGE).map(card_rect),
            },
            (Target::PageLine, None) if self.mode == Mode::Duty && self.kk_takeover().is_none() && self.pages() > 1 => Some(PAGE_LINE),
            (Target::Buddy, None) if self.mode == Mode::Duty && self.kk_takeover().is_none() => Some(BUDDY),
            _ => None,
        }
    }

    pub(super) fn kk_target_at(&self, x: i32, y: i32) -> Option<Target> {
        if let Some(panel) = &self.panel {
            if !PANEL_CARD.contains(x, y) {
                return Some(Target::OutsidePanel);
            }
            if let Some(index) = kk_hint_rects(panel).iter().position(|rect| rect.contains(x, y)) {
                return Some(Target::Hint(index));
            }
            if panel.kind == PanelKind::List {
                return (0..panel.lines.len())
                    .find(|&index| {
                        let row = panel_row(index);
                        Rect { y: row.y - 1, height: 23, ..row }.contains(x, y)
                    })
                    .map(Target::Row);
            }
            return None;
        }
        if self.mode != Mode::Duty {
            return None;
        }
        match self.kk_takeover() {
            Some((State::InputRequired, Some(index))) if TICKET.contains(x, y) => return Some(Target::Card(index)),
            Some((State::Done, Some(index))) if DONE_TICKET.contains(x, y) => return Some(Target::Card(index)),
            Some((State::Failed, Some(index))) if FAILED_TICKET.contains(x, y) => return Some(Target::Card(index)),
            Some(_) => return None,
            None => {}
        }
        if let Some(slot) = (0..self.tasks_on_page().len()).find(|&slot| card_rect(slot).contains(x, y)) {
            return Some(Target::Card(self.page * CARDS_PER_PAGE + slot));
        }
        if self.pages() > 1 && PAGE_LINE.contains(x, y) {
            return Some(Target::PageLine);
        }
        BUDDY.contains(x, y).then_some(Target::Buddy)
    }
}

/// A panel's title colour: a voice command's errors and the last confirmation before a new identity in
/// `kore-pink-text`, the other confirmations in `wait-text` (canvas: "voice, transfers, menu sub-views").
fn title_color(title: &[u8]) -> u16 {
    const ERRORS: [&[u8]; 6] = [b"NOT HEARD", b"UNAVAILABLE", b"BUSY", b"VOICE ERROR", b"NO MICROPHONE", b"SURE?"];
    if ERRORS.contains(&title) {
        KORE_PINK_TEXT
    } else if title.ends_with(b"?") {
        WAIT_TEXT
    } else {
        BOX_GROUND
    }
}

/// Each key hint along the bottom of a panel, down to the card's edge; the last takes the rest of the line, as the
/// robot's does.
fn kk_hint_rects(panel: &Panel) -> Vec<Rect> {
    let mut x = PANEL_INNER_LEFT;
    let mut rects: Vec<Rect> = panel
        .hints
        .iter()
        .map(|(key, action)| {
            let width = kt::width(&DOT_10, key) + 8 + 4 + kt::width(&DOT_10, action) + 10;
            let rect = Rect { x: x - 2, y: PANEL_RULE_Y + 2, width, height: PANEL_CARD.y + PANEL_CARD.height - PANEL_RULE_Y - 2 };
            x += width;
            rect
        })
        .collect();
    if let Some(last) = rects.last_mut() {
        last.width = PANEL_CARD.x + PANEL_CARD.width - last.x;
    }
    rects
}

/// A `width` × `height` bar rotated `degrees` clockwise from 12 o'clock, its middle `radius` from the centre.
#[allow(clippy::too_many_arguments, reason = "the bar's place, size and colour")]
fn fill_rotated(canvas: &mut Canvas, center_x: f32, center_y: f32, degrees: f32, radius: f32, width: f32, height: f32, color: u16) {
    let angle = degrees.to_radians();
    let (sin, cos) = (libm::sinf(angle), libm::cosf(angle));
    let (mid_x, mid_y) = (center_x + sin * radius, center_y - cos * radius);
    let reach = libm::ceilf(height.max(width)) as i32;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let (px, py) = (dx as f32, dy as f32);
            // Into the bar's own frame: across it and along it.
            let across = px * cos + py * sin;
            let along = -px * sin + py * cos;
            if across.abs() <= width / 2.0 && along.abs() <= height / 2.0 {
                canvas.fill_rect(libm::roundf(mid_x) as i32 + dx, libm::roundf(mid_y) as i32 + dy, 1, 1, color);
            }
        }
    }
}
