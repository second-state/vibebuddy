//! The korekore settings window (canvas: "Mac · Character (sidebar)" and its siblings): the sidebar with the mini
//! box, and the section's page beside it. The same layout as the Mac's `KorekoreUI`, in iced. Omarchy draws the
//! window's frame here, so the view is the inside of the design's window: no outline or shadow of its own.

use iced::widget::{Space, button, column, container, image, row, text};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Padding, Shadow, Vector};

use super::picture::{BoxPicture, Mood};
use super::sprites;
use super::theme::{self, typography};
use super::{Section, Sprite};
use crate::i18n::tr;

/// What the view sends back: the user picked a section.
#[derive(Clone, Copy, Debug)]
pub struct Pick(pub Section);

const ROUNDED: &str = "Rounded Mplus 1c";
const PIXEL: Font = typography::BOX_TEXT.font;

fn rounded(weight: iced::font::Weight) -> Font {
    Font { family: iced::font::Family::Name(ROUNDED), weight, ..Font::DEFAULT }
}

fn sprite(sprite: &Sprite, scale: u32, gray: bool) -> image::Image<image::Handle> {
    sized(sprite, scale, gray, scale as f32)
}

/// A sprite drawn at `scale` whole times and shown at `shown` points per pixel, never resampled smoothly.
fn sized(sprite: &Sprite, scale: u32, gray: bool, shown: f32) -> image::Image<image::Handle> {
    let handle = if gray { sprite.grayed(0.75).handle(scale, None) } else { sprite.handle(scale, None) };
    image(handle)
        .width(Length::Fixed(sprite.width as f32 * shown))
        .height(Length::Fixed(sprite.height as f32 * shown))
        .filter_method(image::FilterMethod::Nearest)
}

fn sticker(fill: Color, radius: f32, stroke: f32, shadow: f32) -> container::Style {
    container::Style {
        background: Some(Background::Color(fill)),
        border: Border { color: theme::INK, width: stroke, radius: radius.into() },
        shadow: Shadow { color: theme::INK, offset: Vector::new(0.0, shadow), blur_radius: 0.0 },
        ..container::Style::default()
    }
}

fn mood_word(mood: Mood) -> &'static str {
    match mood {
        Mood::Ready => "READY",
        Mood::Working => "WORKING",
        Mood::Input => "ASK",
        Mood::Pomodoro => "POMODORO",
        Mood::Leisure => "LEISURE",
        Mood::NoLink => "NO LINK",
    }
}

fn mood_color(mood: Mood) -> Color {
    match mood {
        Mood::Ready | Mood::Leisure => theme::LAVENDER,
        Mood::Working => theme::SKY,
        Mood::Input => theme::SUN,
        Mood::Pomodoro => theme::KORE_PINK,
        Mood::NoLink => theme::MIST,
    }
}

fn kore(mood: Mood) -> &'static Sprite {
    match mood {
        Mood::Ready | Mood::Pomodoro | Mood::Leisure => &sprites::KORE_NORMAL,
        Mood::Working => &sprites::KORE_WORKING,
        Mood::Input => &sprites::KORE_INPUT,
        Mood::NoLink => &sprites::KORE_SLEEP,
    }
}

fn tile(name: &str) -> &'static Sprite {
    sprites::ALL.iter().find(|(key, _)| *key == name).map(|(_, sprite)| *sprite).unwrap_or(&sprites::ICON_API)
}

/// The box's screen redrawn (canvas: "App sidebar component"): laid out at the box's 320 × 240 and shown at 0.65,
/// 208 × 156, as the design does, inside a 3 px outline.
pub fn mini_box<'a, M: 'a>(picture: &BoxPicture) -> Element<'a, M> {
    const S: f32 = 0.65;
    // iced clips square, so the screen's own corners are rounded to sit inside the outline.
    const INNER: f32 = theme::RADIUS_MD - theme::STROKE_BOLD;
    let screen: Element<'a, M> = if picture.mood == Mood::NoLink {
        container(
            column![
                text("Z z").font(PIXEL).size(14.0 * S).color(theme::MIST),
                sized(&sprites::KORE_SLEEP, 6, true, 6.0 * S),
                text("NO LINK").font(PIXEL).size(typography::BOX_STATE.size * S).color(theme::MIST),
                text("waiting for the Mac").font(PIXEL).size(11.0 * S).color(theme::MIST_TEXT),
            ]
            .spacing(6.0 * S)
            .align_x(Alignment::Center),
        )
        .center(Length::Fill)
        .style(|_| container::Style {
            background: Some(theme::BOX_GRAY_GROUND.into()),
            border: Border { radius: INNER.into(), ..Border::default() },
            ..container::Style::default()
        })
        .into()
    } else {
        let mood = picture.mood;
        let side = container(
            column![
                sized(kore(mood), 4, false, 4.0 * S),
                text(mood_word(mood)).font(PIXEL).size(16.0 * S).color(mood_color(mood)),
            ]
            .spacing(8.0 * S)
            .align_x(Alignment::Center),
        )
        .center_y(Length::Fill)
        .center_x(Length::Fixed(96.0 * S))
        .style(|_| container::Style {
            background: Some(theme::BOX_CARD.into()),
            border: Border { radius: iced::border::Radius::default().left(INNER), ..Border::default() },
            ..container::Style::default()
        });
        let cards: Element<'a, M> = if picture.tickets.is_empty() {
            container(
                column![
                    text("ALL QUIET").font(PIXEL).size(12.0 * S).color(theme::BOX_TEXT),
                    text(format!(
                        "{} done · {} asks · {} busy",
                        picture.today.done,
                        picture.today.asks,
                        super::picture::busy(picture.today.busy_seconds).to_uppercase()
                    ))
                    .font(PIXEL)
                    .size(10.0 * S)
                    .color(theme::BOX_TEXT_MUTED),
                ]
                .spacing(6.0 * S)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into()
        } else {
            column(picture.tickets.iter().map(|ticket| {
                container(
                    row![
                        sized(tile(&format!("icon-{}", ticket.icon)), 2, false, 2.0 * S),
                        text(ticket.title.clone()).font(PIXEL).size(13.0 * S).color(theme::BOX_TEXT).wrapping(text::Wrapping::None),
                    ]
                    .spacing(8.0 * S)
                    .align_y(Alignment::Center),
                )
                .padding(7.0 * S)
                .width(Length::Fill)
                .clip(true)
                .style(|_| container::Style {
                    background: Some(theme::BOX_CARD.into()),
                    border: Border { radius: (8.0 * S).into(), ..Border::default() },
                    ..container::Style::default()
                })
                .into()
            }))
            .spacing(8.0 * S)
            .padding(Padding::from([10.0 * S, 8.0 * S]))
            .into()
        };
        container(row![side, cards])
            .style(|_| container::Style {
                background: Some(theme::BOX_GROUND.into()),
                border: Border { radius: INNER.into(), ..Border::default() },
                ..container::Style::default()
            })
            .into()
    };
    container(container(screen).width(208.0).height(156.0).clip(true))
        .padding(theme::STROKE_BOLD)
        .style(|_| sticker(theme::INK, theme::RADIUS_MD, theme::STROKE_BOLD, theme::STICKER_SM))
        .into()
}

fn sidebar<'a>(picture: &BoxPicture, sections: &[Section], current: Section, app_version: &str) -> Element<'a, Pick> {
    let link_line = if !picture.daemon_alive {
        tr("DAEMON NOT RUNNING", &[])
    } else if let Some(link) = picture.link {
        tr("BOX ONLINE · %@", &[&link.label()])
    } else {
        tr("NO BOX LINKED", &[])
    };
    let linked = picture.link.is_some();
    let dot = container(Space::new())
        .width(8)
        .height(8)
        .style(move |_| container::Style {
            background: Some((if linked { theme::MINT } else { theme::MIST }).into()),
            border: Border { color: theme::INK, width: theme::STROKE_THIN, radius: 0.0.into() },
            ..container::Style::default()
        });
    let status = column![
        row![dot, text(link_line).font(PIXEL).size(12).color(theme::INK)].spacing(6).align_y(Alignment::Center),
        text(tr("%lld done · %lld asks today", &[&picture.today.done, &picture.today.asks])).font(PIXEL).size(12).color(theme::MUTED),
    ]
    .spacing(2)
    .padding(Padding::from([0, 4]));
    let nav = column(sections.iter().map(|&section| {
        let on = section == current;
        button(
            row![sprite(tile(section.tile()), 2, false), text(section.title()).font(rounded(iced::font::Weight::ExtraBold)).size(15)]
                .spacing(10)
                .align_y(Alignment::Center),
        )
        .padding(Padding::from([6, 8]))
        .width(Length::Fill)
        .on_press(Pick(section))
        .style(move |_, _| button::Style {
            background: on.then_some(Background::Color(theme::INK)),
            text_color: if on { theme::CREAM } else { theme::INK },
            border: Border { radius: 12.0.into(), ..Border::default() },
            ..button::Style::default()
        })
        .into()
    }))
    .spacing(4);
    let version = match &picture.firmware_version {
        Some(firmware) => tr("app %@ · box fw %@", &[&app_version, firmware]),
        None => tr("app %@", &[&app_version]),
    };
    let content = column![
        column![
            text("korekore").font(rounded(iced::font::Weight::ExtraBold)).size(26).color(theme::INK),
            container(sprite(&sprites::TAG_YOUR_VIBEBUDDY, 2, false)).padding(Padding::ZERO.top(4)),
        ]
        .spacing(2)
        .padding(Padding::from([0, 2])),
        container(mini_box(picture)).center_x(Length::Fill),
        status,
        nav,
        Space::new().height(Length::Fill),
        text(version).font(PIXEL).size(12).color(theme::MUTED_2),
    ]
    .spacing(14);
    container(container(content).padding(Padding { top: 14.0, right: 14.0, bottom: 16.0, left: 14.0 }))
        .width(240.0 - theme::STROKE_BOLD)
        .height(Length::Fill)
        .style(|_| container::Style { background: Some(theme::PAPER.into()), ..container::Style::default() })
        .into()
}

/// The whole window: sidebar, its 3 px ink edge, and the page.
pub fn settings<'a>(
    picture: &BoxPicture,
    sections: &[Section],
    current: Section,
    app_version: &str,
    page: Element<'a, Pick>,
) -> Element<'a, Pick> {
    let edge = container(Space::new()).width(theme::STROKE_BOLD).height(Length::Fill).style(|_| container::Style {
        background: Some(theme::INK.into()),
        ..container::Style::default()
    });
    let title = row![
        text(current.title()).font(typography::TITLE.font).size(typography::TITLE.size).color(theme::INK),
        Space::new().width(Length::Fill),
        text(current.subtitle()).font(rounded(iced::font::Weight::Normal)).size(13).color(theme::MUTED),
    ]
    .align_y(Alignment::End);
    let main = container(column![title, page].spacing(14))
        .padding(Padding { top: 22.0, right: 22.0, bottom: 20.0, left: 22.0 })
        .width(Length::Fill)
        .height(Length::Fill);
    container(row![sidebar(picture, sections, current, app_version), edge, main])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style { background: Some(theme::CREAM.into()), ..container::Style::default() })
        .into()
}

/// Stands in for a page that comes with a later step of the korekore UI.
pub fn pending<'a>() -> Element<'a, Pick> {
    container(text(tr("This page comes with the next step of korekore.", &[])).font(rounded(iced::font::Weight::Normal)).size(14).color(theme::MUTED))
        .center_x(Length::Fill)
        .center_y(Length::Fixed(120.0))
        .style(|_| container::Style {
            border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: theme::RADIUS_MD.into() },
            ..container::Style::default()
        })
        .into()
}
