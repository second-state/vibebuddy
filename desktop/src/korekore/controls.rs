//! The korekore controls in iced, the counterparts of the Mac's `KorekoreUI/Controls.swift`: cards, pill buttons, the
//! switch, pixel state tags, progress bars, pickers and fields. Every page uses these.

use std::fmt::Display;

use iced::widget::{Space, button, column, container, pick_list, row, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Padding, Shadow, Vector};

use super::theme::{self, typography};

pub const PIXEL: Font = typography::BOX_TEXT.font;
/// Fusion Pixel 12 px (fonts/korekore), the small pixel type.
const PIXEL_SMALL: Font = Font { family: iced::font::Family::Name("Fusion Pixel 12px Prop latin"), ..Font::DEFAULT };

/// Pixel type at a size, as the Mac's `Font.pixel` does it: a pixel font is clean only at its own size or a whole
/// multiple, so below 15 px it is Fusion Pixel at 12, then DotGothic16 at 16, and at 32 from 24 px.
pub fn pixel(size: f32) -> (Font, f32) {
    if size < 15.0 {
        (PIXEL_SMALL, 12.0)
    } else if size < 24.0 {
        (PIXEL, 16.0)
    } else {
        (PIXEL, 32.0)
    }
}

pub fn rounded(weight: iced::font::Weight) -> Font {
    Font { family: iced::font::Family::Name("Rounded Mplus 1c"), weight, ..Font::DEFAULT }
}

pub fn bold() -> Font {
    rounded(iced::font::Weight::ExtraBold)
}

pub fn semibold() -> Font {
    rounded(iced::font::Weight::Bold)
}

pub fn regular() -> Font {
    rounded(iced::font::Weight::Normal)
}

/// A white card: 2 px ink outline, radius 16, the small sticker shadow.
pub fn card<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> container::Container<'a, M> {
    card_with(content, theme::PAPER, theme::INK, theme::STROKE_BASE)
}

pub fn card_with<'a, M: 'a>(content: impl Into<Element<'a, M>>, fill: Color, outline: Color, stroke: f32) -> container::Container<'a, M> {
    container(content).padding(14).width(Length::Fill).style(move |_| container::Style {
        background: Some(fill.into()),
        border: Border { color: outline, width: stroke, radius: theme::RADIUS_MD.into() },
        shadow: Shadow { color: theme::INK, offset: Vector::new(0.0, theme::STICKER_SM), blur_radius: 0.0 },
        text_color: Some(theme::INK),
        ..container::Style::default()
    })
}

/// A card of rows with 2 px `line` dividers between them.
pub fn rows<'a, M: 'a>(items: impl IntoIterator<Item = Element<'a, M>>) -> container::Container<'a, M> {
    let mut list = column![];
    for (index, item) in items.into_iter().enumerate() {
        if index > 0 {
            list = list.push(divider());
        }
        list = list.push(item);
    }
    card(list).padding(0)
}

pub fn divider<'a, M: 'a>() -> Element<'a, M> {
    container(Space::new())
        .width(Length::Fill)
        .height(theme::STROKE_BASE)
        .style(|_| container::Style { background: Some(theme::LINE.into()), ..container::Style::default() })
        .into()
}

/// A settings row: a 14/700 label, then what controls it.
pub fn setting<'a, M: 'a>(label: String, control: impl Into<Element<'a, M>>) -> Element<'a, M> {
    row![text(label).font(semibold()).size(14).color(theme::INK).width(Length::Fill), control.into()]
        .spacing(12)
        .align_y(Alignment::Center)
        .padding(Padding::from([12, 16]))
        .into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    Paper,
    Sun,
    Pink,
    Ink,
}

impl Fill {
    fn colors(self) -> (Color, Color) {
        match self {
            Fill::Paper => (theme::PAPER, theme::INK),
            Fill::Sun => (theme::SUN, theme::INK),
            Fill::Pink => (theme::KORE_PINK, theme::INK),
            Fill::Ink => (theme::INK, theme::CREAM),
        }
    }
}

/// Everything you press: a pill with a 2 px ink outline, 12/800 (13/800 for the larger ones). None disables it.
pub fn pill<'a, M: Clone + 'a>(label: String, fill: Fill, on_press: Option<M>) -> button::Button<'a, M> {
    pill_sized(label, fill, 12.0, on_press)
}

pub fn pill_sized<'a, M: Clone + 'a>(label: String, fill: Fill, size: f32, on_press: Option<M>) -> button::Button<'a, M> {
    let (background, foreground) = fill.colors();
    let padding = if size > 12.0 { Padding::from([6, 14]) } else { Padding::from([4, 12]) };
    button(text(label).font(bold()).size(size).wrapping(text::Wrapping::None))
        .padding(padding)
        .on_press_maybe(on_press)
        .style(move |_, status| {
            let disabled = matches!(status, button::Status::Disabled);
            let fade = |color: Color| if disabled { Color { a: 0.45, ..color } } else { color };
            button::Style {
                background: Some(Background::Color(fade(background))),
                text_color: fade(foreground),
                border: Border { color: fade(theme::INK), width: theme::STROKE_BASE, radius: theme::RADIUS_PILL.into() },
                ..button::Style::default()
            }
        })
}

/// The switch: a 48 × 28 pill, `kore-pink` on, `backdrop` off, a white knob with an ink outline.
pub fn switch<'a, M: Clone + 'a>(on: bool, toggle: Option<M>) -> Element<'a, M> {
    let knob = container(Space::new()).width(20).height(20).style(|_| container::Style {
        background: Some(theme::PAPER.into()),
        border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: 10.0.into() },
        ..container::Style::default()
    });
    let track = container(knob)
        .width(48)
        .height(28)
        .padding(2)
        .align_x(if on { Alignment::End } else { Alignment::Start })
        .align_y(Alignment::Center)
        .style(move |_| container::Style {
            background: Some((if on { theme::KORE_PINK } else { theme::BACKDROP }).into()),
            border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: theme::RADIUS_PILL.into() },
            ..container::Style::default()
        });
    button(track).padding(0).on_press_maybe(toggle).style(|_, _| button::Style::default()).into()
}

/// A state in pixel type, in a readable shade of its colour, no background: WATCHING, NOT FOUND, UP TO DATE.
pub fn tag<'a, M: 'a>(label: impl Display, color: Color) -> Element<'a, M> {
    text(label.to_string().to_uppercase()).font(pixel(12.0).0).size(pixel(12.0).1).color(color).into()
}

/// Progress: a 12 px pill track in cream with a 2 px ink outline, filled in the given colour.
pub fn bar<'a, M: 'a>(progress: f32, fill: Color) -> Element<'a, M> {
    let progress = progress.clamp(0.0, 1.0);
    let filled = container(Space::new()).width(Length::FillPortion((progress * 1000.0) as u16)).height(Length::Fill).style(move |_| {
        container::Style { background: Some(fill.into()), ..container::Style::default() }
    });
    let rest = Space::new().width(Length::FillPortion(((1.0 - progress) * 1000.0) as u16));
    container(row![filled, rest])
        .height(12)
        .width(Length::Fill)
        .padding(2)
        .clip(true)
        .style(|_| container::Style {
            background: Some(theme::CREAM.into()),
            border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: theme::RADIUS_PILL.into() },
            ..container::Style::default()
        })
        .into()
}

pub fn heading<'a, M: 'a>(label: impl Display) -> Element<'a, M> {
    text(label.to_string()).font(bold()).size(15).color(theme::INK).into()
}

/// Body copy, 13 px `muted`.
pub fn note<'a, M: 'a>(label: impl Display) -> Element<'a, M> {
    text(label.to_string()).font(regular()).size(13).color(theme::MUTED).into()
}

pub fn small<'a, M: 'a>(label: impl Display) -> Element<'a, M> {
    text(label.to_string()).font(regular()).size(12).color(theme::MUTED).into()
}

/// A pick from a short list, as a pill.
pub fn picker<'a, T, M>(options: Vec<T>, selected: Option<T>, on_select: impl Fn(T) -> M + 'a) -> Element<'a, M>
where
    T: ToString + PartialEq + Clone + 'a,
    M: Clone + 'a,
{
    pick_list(options, selected, on_select)
        .font(bold())
        .text_size(13)
        .padding(Padding::from([4, 12]))
        .style(|_, _| pick_list::Style {
            text_color: theme::INK,
            placeholder_color: theme::MUTED_2,
            handle_color: theme::INK,
            background: theme::PAPER.into(),
            border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: theme::RADIUS_PILL.into() },
        })
        .into()
}

/// A text field: DotGothic16 14 px in a white box with a 2 px ink outline, radius 10.
pub fn field<'a, M: Clone + 'a>(placeholder: &str, value: &str) -> text_input::TextInput<'a, M> {
    text_input(placeholder, value).font(pixel(14.0).0).size(pixel(14.0).1).padding(Padding::from([6, 10])).style(|_, _| text_input::Style {
        background: theme::PAPER.into(),
        border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: theme::RADIUS_SM.into() },
        icon: theme::INK,
        placeholder: theme::MUTED_2,
        value: theme::INK,
        selection: theme::SKY,
    })
}

/// Kore (or another sprite) on a small Ink stage, as the pages put it beside a note.
pub fn badge<'a, M: 'a>(sprite: &super::Sprite, gray: bool, fill: Color) -> Element<'a, M> {
    let handle = if gray { sprite.grayed(0.75).handle(2, None) } else { sprite.handle(2, None) };
    container(iced::widget::image(handle).filter_method(iced::widget::image::FilterMethod::Nearest))
        .padding(Padding::from([5, 6]))
        .style(move |_| container::Style {
            background: Some(fill.into()),
            border: Border { radius: 12.0.into(), ..Border::default() },
            ..container::Style::default()
        })
        .into()
}

/// A sprite at a whole-number scale.
pub fn sprite<'a, M: 'a>(sprite: &super::Sprite, scale: u32) -> Element<'a, M> {
    iced::widget::image(sprite.handle(scale, None))
        .width(sprite.width as f32 * scale as f32)
        .height(sprite.height as f32 * scale as f32)
        .filter_method(iced::widget::image::FilterMethod::Nearest)
        .into()
}
