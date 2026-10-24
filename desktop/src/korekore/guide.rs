//! The first run on Linux in the korekore look (canvas: "Mac · Onboarding welcome", "Onboarding · find the box,
//! connect, pick, Wi-Fi", the end of onboarding on "App · alerts, notifications, banners"). As the current guide
//! (`App::guide`): the install script already connected the agents and set up the start at login, so it is the Mac's
//! without those two steps.

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Border, Element, Length, Padding, Shadow, Vector};

use super::controls::{Fill, PIXEL, bar, bold, card_with, heading, note, pill, regular, sprite, tag};
use super::{sprites, theme};
use crate::i18n::tr;
use crate::status::{OperationKind, OperationState};
use crate::{Message, ROBOT, BUILTIN};

/// The large pill the pages lead with: 15/800 (17 on the welcome page), a 3 px outline and the small shadow when it
/// is the primary action.
fn big<'a>(label: String, fill: Fill, primary: bool, size: f32, on_press: Option<Message>) -> Element<'a, Message> {
    let (background, foreground) = match fill {
        Fill::Sun => (theme::SUN, theme::INK),
        Fill::Pink => (theme::KORE_PINK, theme::INK),
        Fill::Ink => (theme::INK, theme::CREAM),
        Fill::Paper => (theme::PAPER, theme::INK),
    };
    let padding = if size > 15.0 { Padding::from([12, 28]) } else if primary { Padding::from([10, 22]) } else { Padding::from([10, 18]) };
    button(text(label).font(bold()).size(size))
        .padding(padding)
        .on_press_maybe(on_press)
        .style(move |_, status| {
            let disabled = matches!(status, button::Status::Disabled);
            let fade = |color: iced::Color| if disabled { iced::Color { a: 0.45, ..color } } else { color };
            button::Style {
                background: Some(fade(background).into()),
                text_color: fade(foreground),
                border: Border {
                    color: theme::INK,
                    width: if primary { theme::STROKE_BOLD } else { theme::STROKE_BASE },
                    radius: theme::RADIUS_PILL.into(),
                },
                shadow: if primary {
                    Shadow { color: theme::INK, offset: Vector::new(0.0, theme::STICKER_SM), blur_radius: 0.0 }
                } else {
                    Shadow::default()
                },
                ..button::Style::default()
            }
        })
        .into()
}

fn title<'a>(label: String, size: f32) -> Element<'a, Message> {
    text(label).font(bold()).size(size).color(theme::INK).into()
}

impl crate::App {
    pub(crate) fn kk_guide(&self, page: usize) -> Element<'_, Message> {
        let flashing = self.status.as_ref().and_then(|status| status.operation.as_ref()).is_some_and(|operation| {
            operation.kind == OperationKind::Firmware && operation.running()
        });
        let names = [tr("Welcome", &[]), tr("Find the box", &[]), tr("Pick a character", &[]), tr("Wi-Fi", &[]), tr("Done", &[])];
        // Done in mint, current in ink, the rest white; the ones done take you back.
        let chips = row(names.into_iter().enumerate().map(|(index, name)| {
            let (background, foreground) = match index.cmp(&page) {
                std::cmp::Ordering::Less => (theme::MINT, theme::INK),
                std::cmp::Ordering::Equal => (theme::INK, theme::CREAM),
                std::cmp::Ordering::Greater => (theme::PAPER, theme::INK),
            };
            button(text(name).font(super::controls::semibold()).size(12))
                .padding(Padding::from([3, 10]))
                .on_press_maybe((index < page && !flashing).then_some(Message::Guide(Some(index))))
                .style(move |_, _| button::Style {
                    background: Some(background.into()),
                    text_color: foreground,
                    border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: theme::RADIUS_PILL.into() },
                    ..button::Style::default()
                })
                .into()
        }))
        .spacing(8);
        let body = match page {
            0 => self.guide_welcome(),
            1 => self.guide_find(),
            2 => self.guide_pick(),
            3 => self.guide_wifi(),
            _ => self.guide_done(),
        };
        container(
            column![container(body).height(Length::Fill).width(Length::Fill)]
                .push(self.notice.as_ref().map(|notice| match notice {
                    Ok(message) => tag(message, theme::DONE_TEXT),
                    Err(error) => tag(error, theme::KORE_PINK_TEXT),
                }))
                .push(container(chips).center_x(Length::Fill).padding(Padding { top: 22.0, right: 0.0, bottom: 24.0, left: 0.0 }))
                .padding(Padding::ZERO.top(40)),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style { background: Some(theme::CREAM.into()), ..container::Style::default() })
        .into()
    }

    fn guide_welcome(&self) -> Element<'_, Message> {
        let stage = container(
            column![
                container(text("hajimemashite!").font(bold()).size(20).color(theme::INK)).padding(Padding::from([6, 14])).style(|_| container::Style {
                    background: Some(theme::CREAM.into()),
                    border: Border {
                        color: theme::INK,
                        width: theme::STROKE_BASE,
                        radius: iced::border::Radius { top_left: 16.0, top_right: 16.0, bottom_right: 16.0, bottom_left: 4.0 },
                    },
                    ..container::Style::default()
                }),
                sprite(&sprites::KORE_HAPPY, 11),
            ]
            .spacing(16)
            .align_x(Alignment::Center),
        )
        .center_x(340)
        .center_y(360)
        .style(|_| container::Style {
            background: Some(theme::KORE_PINK.into()),
            border: Border { color: theme::INK, width: theme::STROKE_BOLD, radius: theme::RADIUS_XL.into() },
            shadow: Shadow { color: theme::INK, offset: Vector::new(0.0, theme::STICKER_MD), blur_radius: 0.0 },
            ..container::Style::default()
        });
        let right = column![
            row![text("korekore").font(bold()).size(30).color(theme::INK), sprite(&sprites::TAG_YOUR_VIBEBUDDY, 2)].spacing(12).align_y(Alignment::End),
            text(tr("Kore kore!\nI'll call you when\nyour agents need you.", &[])).font(bold()).size(super::theme::typography::HERO.size).color(theme::INK),
            text(tr("The install script has connected your agents and set korekore to start when you log in. Let's wake up the box first.", &[]))
                .font(regular())
                .size(16)
                .color(theme::MUTED),
            row![big(tr("Let's go", &[]), Fill::Sun, true, 17.0, Some(Message::Guide(Some(1))))],
            container(text(tr("Already have a VibeBuddy? Same box, new name. Everything you set up carries over.", &[])).font(regular()).size(13).color(theme::MUTED))
                .padding(Padding::from([8, 12]))
                .style(|_| container::Style {
                    background: Some(theme::PAPER.into()),
                    border: Border { color: theme::MIST_TEXT, width: theme::STROKE_BASE, radius: 12.0.into() },
                    ..container::Style::default()
                }),
        ]
        .spacing(14);
        container(row![stage, right].spacing(36).align_y(Alignment::Center)).center(Length::Fill).padding(Padding::from([4, 44])).into()
    }

    fn guide_find(&self) -> Element<'_, Message> {
        let device = self.status.as_ref().map(|status| &status.device);
        let online = self.online();
        let operation = self.status.as_ref().and_then(|status| status.operation.as_ref()).filter(|operation| operation.kind == OperationKind::Firmware);
        let flashing = operation.is_some_and(|operation| operation.state != OperationState::Done);
        let (kore, word, screen, ink) = if flashing {
            (&sprites::KORE_WORKING, "UPDATING", theme::INK, theme::SKY)
        } else if online {
            (&sprites::KORE_HAPPY, "HELLO!", theme::SUN, theme::INK)
        } else {
            (&sprites::KORE_SLEEP, "Z z", theme::BOX_GRAY_GROUND, theme::MIST)
        };
        let gray = !online && !flashing;
        let face = if gray { super::controls::badge(kore, true, iced::Color::TRANSPARENT) } else { sprite(kore, 4) };
        let drawing = container(
            container(column![face, text(word).font(PIXEL).size(18).color(ink)].spacing(6).align_x(Alignment::Center))
                .center_x(240)
                .center_y(180)
                .style(move |_| container::Style {
                    background: Some(screen.into()),
                    border: Border { color: theme::INK, width: theme::STROKE_BOLD, radius: 12.0.into() },
                    ..container::Style::default()
                }),
        )
        .center_x(300)
        .center_y(240)
        .style(|_| container::Style {
            background: Some(theme::LINE.into()),
            border: Border { color: theme::INK, width: theme::STROKE_BOLD, radius: theme::RADIUS_XL.into() },
            shadow: Shadow { color: theme::INK, offset: Vector::new(0.0, theme::STICKER_MD), blur_radius: 0.0 },
            ..container::Style::default()
        });
        let side: Element<'_, Message> = if let Some(operation) = operation.filter(|_| flashing) {
            let state: Element<'_, Message> = match operation.state {
                OperationState::Failed => column![
                    note(tr("Hold K0 on the box and replug the cable to put it in download mode, then retry.", &[])),
                    big(tr("Try again", &[]), Fill::Sun, true, 15.0, Some(Message::FlashFirmware)),
                ]
                .spacing(10)
                .into(),
                OperationState::Replug => note(tr("The firmware is on the box, but it didn't start. Unplug the box and plug it back in. Don't hold K0 this time.", &[])),
                _ => column![
                    row![tag(format!("{:.0}%", operation.progress * 100.0), theme::INK), Space::new().width(Length::Fill), tag(tr("Keep it plugged in", &[]), theme::MUTED_2)],
                    bar(operation.progress, theme::SUN),
                ]
                .spacing(8)
                .into(),
            };
            column![title(tr("Flashing korekore firmware…", &[]), 26.0), state].spacing(12).into()
        } else if let Some(device) = device.filter(|_| online) {
            let port = device.port.as_deref().unwrap_or("?").trim_start_matches("/dev/");
            let kind = if device.network { tr("Wi-Fi", &[]) } else if device.bridge { tr("UART bridge", &[]) } else { tr("native USB", &[]) };
            column![
                title(tr("Is this your box?", &[]), 32.0),
                container(
                    row![
                        text(format!("{port} · {kind}")).font(PIXEL).size(14).color(theme::INK).width(Length::Fill),
                        pill(tr("Make it blink", &[]), Fill::Paper, Some(Message::Identify)),
                    ]
                    .align_y(Alignment::Center),
                )
                .padding(Padding::from([10, 14]))
                .style(|_| container::Style {
                    background: Some(theme::PAPER.into()),
                    border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: 14.0.into() },
                    ..container::Style::default()
                }),
                self.kk_finding(),
            ]
            .push(self.confirm_panel())
            .push(row![big(tr("Yes, that's it", &[]), Fill::Sun, true, 15.0, Some(Message::Guide(Some(2))))])
            .spacing(12)
            .into()
        } else {
            column![
                title(tr("Plug in your box", &[]), 32.0),
                note(if self.status.is_some() {
                    tr("Connect the box to this computer with a USB cable. This page notices it on its own.", &[])
                } else {
                    tr("Starting the daemon…", &[])
                }),
                self.kk_finding(),
                row![big(tr("Skip for now", &[]), Fill::Paper, false, 15.0, Some(Message::Guide(Some(2))))],
            ]
            .spacing(12)
            .into()
        };
        container(
            row![
                column![drawing, tag(if online { tr("the box blinks yellow", &[]) } else { tr("waiting for the box", &[]) }, theme::MUTED)]
                    .spacing(12)
                    .align_x(Alignment::Center)
                    .width(340),
                scrollable(container(side).padding(Padding::ZERO.right(14))).width(Length::Fill),
            ]
            .spacing(32)
            .align_y(Alignment::Center),
        )
        .center_y(Length::Fill)
        .padding(Padding::from([8, 40]))
        .into()
    }

    fn guide_pick(&self) -> Element<'_, Message> {
        let online = self.online();
        let busy = self.busy();
        let current = self.status.as_ref().and_then(|status| status.device.voice.clone());
        let robot_in_use = matches!(current.as_deref(), Some(ROBOT | BUILTIN)) && self.characters.robot_lender.is_none();
        let mut cards = vec![self.kore_card()];
        cards.push(self.character_card("VibeBuddy".to_owned(), tr("The original robot", &[]), self.robot_face.clone(), robot_in_use, Some(ROBOT), Message::UseRobot));
        for (voice, face) in self.voices.iter().zip(&self.faces) {
            cards.push(self.character_card(voice.name.clone(), voice.tag.clone(), face.clone(), current.as_deref() == Some(voice.id), Some(voice.id), Message::UseVoice(voice.id)));
        }
        let strip = scrollable(row(cards.into_iter().map(|card| container(card).width(180).into())).spacing(12).padding(Padding::ZERO.bottom(8)))
            .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::default()));
        let operation = self.status.as_ref().and_then(|status| status.operation.as_ref()).filter(|operation| operation.kind == OperationKind::VoicePack);
        let progress: Option<Element<'_, Message>> = operation.and_then(|operation| match operation.state {
            OperationState::Running => Some(
                card_with(
                    column![
                        row![heading(tr("Writing the character to the box…", &[])), Space::new().width(Length::Fill), tag(format!("{:.0}%", operation.progress * 100.0), theme::INK)],
                        bar(operation.progress, theme::KORE_PINK),
                    ]
                    .spacing(8),
                    theme::PAPER,
                    theme::INK,
                    theme::STROKE_BASE,
                )
                .into(),
            ),
            OperationState::Failed => Some(note(tr("The character didn't finish writing, so the box keeps its built-in voice for now. Write it again when you're ready.", &[]))),
            _ => None,
        });
        let done = operation.is_some_and(|operation| operation.state == OperationState::Done);
        let next = if done {
            big(tr("Next", &[]), Fill::Sun, true, 15.0, Some(Message::Guide(Some(3))))
        } else {
            big(tr("Skip", &[]), Fill::Paper, false, 15.0, (!busy).then_some(Message::Guide(Some(3))))
        };
        column![title(tr("Who lives in your box?", &[]), 30.0), strip]
            .push(progress)
            .push((!online).then(|| note(tr("Plug in the box to write a character to it. If you skip this, it speaks with its built-in voice, Jessica (English).", &[]))))
            .push(row![next])
            .spacing(14)
            .padding(Padding::from([8, 48]))
            .into()
    }

    fn guide_wifi(&self) -> Element<'_, Message> {
        let device = self.status.as_ref().map(|status| &status.device);
        let form: Element<'_, Message> = match device.filter(|device| self.online() && device.box_key.is_some() && !device.network) {
            Some(device) => self.wifi_card(device),
            None => note(tr("Plug in the box to set its network. You can also do it later on the Device page.", &[])),
        };
        let joined = device.is_some_and(|device| device.wifi_address.is_some());
        let next = if joined {
            big(tr("Next", &[]), Fill::Sun, true, 15.0, Some(Message::Guide(Some(4))))
        } else {
            big(tr("Skip for now", &[]), Fill::Paper, false, 15.0, Some(Message::Guide(Some(4))))
        };
        scrollable(
            column![
                column![
                    title(tr("Let the box go wireless?", &[]), 30.0),
                    text(tr("On Wi-Fi, this computer reaches the box without the cable. Keep the cable in for this step.", &[])).font(regular()).size(14).color(theme::MUTED),
                ]
                .spacing(4),
                form,
                row![next],
            ]
            .spacing(14)
            .padding(Padding::from([8, 48])),
        )
        .into()
    }

    fn guide_done(&self) -> Element<'_, Message> {
        let card = card_with(
            column![
                row![
                    container(sprite(&sprites::KORE_HAPPY, 3)).padding(Padding::from([6, 8])).style(|_| container::Style {
                        background: Some(theme::KORE_PINK.into()),
                        border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: 12.0.into() },
                        ..container::Style::default()
                    }),
                    column![
                        text(tr("All set. Yatta!", &[])).font(bold()).size(22).color(theme::INK),
                        note(tr("korekore lives in the tray now, or in Omarchy's bar. Kore kore when someone needs you.", &[])),
                    ]
                    .spacing(4),
                ]
                .spacing(14)
                .align_y(Alignment::Center),
                row![Space::new().width(Length::Fill), big(tr("Done", &[]), Fill::Sun, true, 15.0, Some(Message::Guide(None)))],
            ]
            .spacing(14),
            theme::CREAM,
            theme::INK,
            theme::STROKE_BOLD,
        )
        .padding(18)
        .max_width(560);
        container(card).center(Length::Fill).into()
    }
}
