//! The korekore pages on Linux (canvas: the "Mac · …" boards and their "every state" boards): the same state and
//! messages as the current tabs in main.rs, laid out in the korekore look. Where the Mac shows a sheet, these show an
//! inline panel, as the alerts board says for Linux.

use iced::widget::{Space, button, column, container, image, row, scrollable, text};
use iced::{Alignment, Border, Element, Length, Padding};

use super::Section;
use super::controls::{
    Fill, PIXEL, badge, bar, bold, card, card_with, divider, field, heading, note, picker, pill, pill_sized, regular, rows, semibold,
    setting, small, sprite, switch, tag,
};
use super::picture::BoxPicture;
use super::{sprites, theme, view};
use crate::i18n::{UiLanguage, tr};
use crate::status::{self, OperationKind, OperationState};
use crate::{AddressChoice, BUILTIN, CUSTOM, Lender, Message, ROBOT, agents, assets, client, config_dir, short_time, wifi_failure};

/// Leaves room for the scroll bar and the cards' shadows.
fn page<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    scrollable(container(content).padding(Padding::ZERO.right(14).bottom(8))).into()
}

fn percent(progress: f32) -> String {
    format!("{:.0}%", progress * 100.0)
}

impl crate::App {
    pub(crate) fn korekore_page(&self, section: Section) -> Element<'_, Message> {
        match section {
            Section::Character => self.kk_character(),
            Section::Agents => self.kk_agents(),
            Section::Conversation => self.kk_conversation(),
            Section::Device => self.kk_device(),
            Section::General => self.kk_general(),
            Section::Advanced => self.kk_advanced(),
        }
    }

    pub(super) fn online(&self) -> bool {
        self.status.as_ref().is_some_and(|status| status.device.connected)
    }

    pub(super) fn busy(&self) -> bool {
        self.status.as_ref().and_then(|status| status.operation.as_ref()).is_some_and(status::Operation::running)
    }

    // MARK: Character

    fn kk_character(&self) -> Element<'_, Message> {
        let online = self.online();
        let busy = self.busy();
        let level = self.volume.or(self.status.as_ref().and_then(|status| status.device.volume)).unwrap_or(60);
        let mut volume = iced::widget::slider(20..=100, level, Message::VolumeDragged).step(5u8).style(|_, _| {
            iced::widget::slider::Style {
                rail: iced::widget::slider::Rail {
                    backgrounds: (theme::KORE_PINK.into(), theme::CREAM.into()),
                    width: 12.0,
                    border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: theme::RADIUS_PILL.into() },
                },
                handle: iced::widget::slider::Handle {
                    shape: iced::widget::slider::HandleShape::Circle { radius: 0.0 },
                    background: theme::KORE_PINK.into(),
                    border_width: 0.0,
                    border_color: theme::INK,
                },
            }
        });
        if online && !busy {
            volume = volume.on_release(Message::VolumeReleased);
        }
        let volume_row = row![
            column![
                row![text(tr("Volume", &[])).font(bold()).size(13), Space::new().width(Length::Fill), text(level.to_string()).font(PIXEL).size(13)],
                volume,
            ]
            .spacing(6)
            .width(Length::Fill),
            pill_sized(tr("Play on box", &[]), Fill::Sun, 13.0, (online && !busy).then_some(Message::PlayLine)),
        ]
        .spacing(14)
        .align_y(Alignment::Center)
        .padding(Padding::from([10, 14]));
        let address = row![text(tr("What the buddy calls you", &[])).font(semibold()).size(13).color(theme::INK).width(Length::Fill)]
            .extend(assets::Language::ALL.map(|language| {
                let options: Vec<AddressChoice> = std::iter::once(AddressChoice { form: None, language })
                    .chain(
                        assets::FORMS_OF_ADDRESS
                            .iter()
                            .filter(|form| form.language == language)
                            .map(|form| AddressChoice { form: Some(form.id), language }),
                    )
                    .collect();
                let picked = options.iter().copied().find(|choice| choice.form == self.characters.address(language));
                picker(options, picked, move |choice: AddressChoice| Message::PickAddress(language, choice.form))
            }))
            .spacing(10)
            .align_y(Alignment::Center)
            .padding(Padding::from([8, 14]));
        let current = self.status.as_ref().and_then(|status| status.device.voice.clone());
        let wears_robot = matches!(current.as_deref(), Some(ROBOT | BUILTIN));
        let robot_in_use = wears_robot && self.characters.robot_lender.is_none();
        let mut cards: Vec<Element<'_, Message>> = vec![self.kore_card()];
        cards.push(self.character_card(
            "VibeBuddy".to_owned(),
            tr("The original robot, drawn by the box itself", &[]),
            self.robot_face.clone(),
            robot_in_use,
            Some(ROBOT),
            Message::UseRobot,
        ));
        for (voice, face) in self.voices.iter().zip(&self.faces) {
            cards.push(self.character_card(
                voice.name.clone(),
                voice.summary.clone(),
                face.clone(),
                current.as_deref() == Some(voice.id),
                Some(voice.id),
                Message::UseVoice(voice.id),
            ));
        }
        cards.push(self.make_your_own());
        let mut grid = column![].spacing(12);
        let mut cards = cards.into_iter().peekable();
        while cards.peek().is_some() {
            let mut line = row![].spacing(12);
            for _ in 0..3 {
                line = line.push(container(cards.next().unwrap_or_else(|| Space::new().into())).width(Length::FillPortion(1)));
            }
            grid = grid.push(line);
        }
        let operation = self.status.as_ref().and_then(|status| status.operation.as_ref());
        let progress: Option<Element<'_, Message>> = operation.filter(|operation| operation.kind == OperationKind::VoicePack).and_then(|operation| match operation.state {
            OperationState::Running => Some(
                card(
                    column![
                        row![heading(tr("Writing the character to the box…", &[])), Space::new().width(Length::Fill), tag(percent(operation.progress), theme::INK)],
                        bar(operation.progress, theme::KORE_PINK),
                    ]
                    .spacing(8),
                )
                .into(),
            ),
            OperationState::Failed => Some(
                card_with(
                    note(tr("The character didn't finish writing, so the box keeps its built-in voice for now. Write it again when you're ready.", &[])),
                    theme::CREAM,
                    theme::INK,
                    theme::STROKE_BASE,
                )
                .into(),
            ),
            _ => None,
        });
        let custom = (self.custom_look.is_some() || self.custom_problem.is_some() || current.as_deref() == Some(CUSTOM))
            .then(|| self.custom_look_card(current.as_deref() == Some(CUSTOM)));
        page(
            column![rows([volume_row.into(), address.into()])]
                .push(progress)
                .push(grid)
                .push(custom)
                .spacing(14),
        )
    }

    pub(super) fn kore_card(&self) -> Element<'_, Message> {
        card(
            column![
                container(sprite(&sprites::KORE_NORMAL, 4)).center_x(Length::Fill).center_y(Length::Fixed(104.0)).style(|_| stage(theme::INK)),
                row![text("Kore").font(bold()).size(16), Space::new().width(Length::Fill), tag(tr("Coming soon", &[]), theme::MUTED_2)]
                    .align_y(Alignment::Center),
                small(tr("The box itself. “Kore kore!”", &[])),
            ]
            .spacing(6),
        )
        .padding(9)
        .into()
    }

    pub(super) fn character_card(
        &self,
        name: String,
        summary: String,
        face: Option<image::Handle>,
        in_use: bool,
        preview: Option<&'static str>,
        use_it: Message,
    ) -> Element<'_, Message> {
        let online = self.online();
        let busy = self.busy();
        let actions: Element<'_, Message> = if in_use {
            tag(tr("In use", &[]), theme::KORE_PINK_TEXT)
        } else {
            let playing = preview.is_some_and(|id| self.previewing.as_ref().is_some_and(|(playing, _)| playing == id));
            row![]
                .push(preview.map(|id| pill(if playing { "■".to_owned() } else { "▶".to_owned() }, Fill::Paper, (!busy).then_some(Message::TogglePreview(id)))))
                .push(pill(tr("Use", &[]), Fill::Paper, (online && !busy).then_some(use_it)))
                .spacing(5)
                .into()
        };
        let stage_content: Element<'_, Message> = match face {
            Some(face) => image(face).filter_method(image::FilterMethod::Nearest).into(),
            None => Space::new().into(),
        };
        let content = column![
            container(stage_content).center_x(Length::Fill).align_y(Alignment::End).height(104).padding(Padding::ZERO.bottom(4)).style(|_| stage(theme::BOX_CARD)),
            row![text(name).font(bold()).size(16).width(Length::Fill), actions].align_y(Alignment::Center),
            small(summary),
        ]
        .spacing(6);
        let (outline, stroke) = if in_use { (theme::KORE_PINK, theme::STROKE_BOLD) } else { (theme::INK, theme::STROKE_BASE) };
        card_with(content, theme::PAPER, outline, stroke).padding(9).into()
    }

    fn make_your_own(&self) -> Element<'_, Message> {
        button(
            container(
                column![
                    text("+").font(PIXEL).size(32).color(theme::KORE_PINK_TEXT),
                    text(tr("Make your own look", &[])).font(bold()).size(14).color(theme::INK),
                    small(tr("1–4 images, made pixel here", &[])),
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fixed(160.0)),
        )
        .padding(9)
        .width(Length::Fill)
        .on_press(Message::ChooseDrawings)
        .style(|_, _| iced::widget::button::Style {
            border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: theme::RADIUS_MD.into() },
            ..iced::widget::button::Style::default()
        })
        .into()
    }

    fn custom_look_card(&self, in_use: bool) -> Element<'_, Message> {
        let online = self.online();
        let busy = self.busy();
        let lenders: Vec<Lender> = self.voices.iter().map(Lender::of).collect();
        let frames = self.custom_look.as_ref().map(|(_, frames)| {
            container(row(frames.iter().map(|frame| image(frame.clone()).filter_method(image::FilterMethod::Nearest).into())).spacing(14))
                .center_x(Length::Fill)
                .padding(12)
                .style(|_| container::Style {
                    background: Some(theme::BOX_CARD.into()),
                    border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: 12.0.into() },
                    ..container::Style::default()
                })
        });
        let pick = self.custom_look.as_ref().map(|_| {
            column![
                row![note(tr("Voice and lines from", &[])), Space::new().width(Length::Fill), picker(lenders, self.lender, Message::PickLender)]
                    .align_y(Alignment::Center),
                row![
                    Space::new().width(Length::Fill),
                    pill(tr("Use this look", &[]), Fill::Pink, (online && !busy && self.lender.is_some()).then_some(Message::UseCustom))
                ],
            ]
            .spacing(8)
        });
        card_with(
            column![
                row![heading(tr("Your own character", &[])), Space::new().width(Length::Fill)]
                    .push(in_use.then(|| tag(tr("In use", &[]), theme::KORE_PINK_TEXT))),
            ]
            .push(frames)
            .push(note(tr(
                "Draw a figure with any image tool, on a plain white background: one image, or four in the order normal, eyes closed, happy, sad. The box shows it in place of the robot, with the voice and lines of the character you pick.",
                &[],
            )))
            .push(row![pill(tr("Choose images…", &[]), Fill::Paper, Some(Message::ChooseDrawings)), pill(tr("Copy a prompt", &[]), Fill::Paper, Some(Message::CopyPrompt))].spacing(8))
            .push(pick)
            .push(self.custom_problem.as_ref().map(|problem| tag(problem, theme::KORE_PINK_TEXT)))
            .spacing(10),
            theme::PAPER,
            theme::INK,
            theme::STROKE_BOLD,
        )
        .into()
    }

    // MARK: Agents

    fn kk_agents(&self) -> Element<'_, Message> {
        let privacy = row![
            badge(&sprites::KORE_NORMAL, false, theme::INK),
            container(text(tr("I only pass along session IDs, event names and folders, never your prompts or replies. One connect covers the app and the terminal.", &[])).font(regular()).size(13).color(theme::INK))
                .padding(Padding::from([8, 12]))
                .width(Length::Fill)
                .style(|_| container::Style {
                    background: Some(theme::PAPER.into()),
                    border: Border {
                        color: theme::INK,
                        width: theme::STROKE_BASE,
                        radius: iced::border::Radius { top_left: 14.0, top_right: 14.0, bottom_right: 14.0, bottom_left: 4.0 },
                    },
                    ..container::Style::default()
                }),
        ]
        .spacing(10)
        .align_y(Alignment::End);
        let agents = rows(self.agents.iter().map(|setup| self.agent_row(setup)));
        page(column![privacy].push(self.plan_panel()).push(agents).spacing(14))
    }

    fn agent_row<'a>(&'a self, setup: &'a agents::Setup) -> Element<'a, Message> {
        let hooks = self.status.as_ref().map(|status| &status.hooks);
        let last = hooks.and_then(|hooks| match setup.agent.as_str() {
            "claude" => hooks.claude.as_deref(),
            "codex" => hooks.codex.as_deref(),
            "opencode" => hooks.opencode.as_deref(),
            "copilot" => hooks.copilot.as_deref(),
            "pi" => hooks.pi.as_deref(),
            _ => None,
        });
        let codex = (setup.agent == "codex" && setup.installed).then(|| agents::codex_trust(setup.modified.as_deref(), last));
        let (state, color, detail) = if !setup.present {
            (tr("Not found", &[]), theme::MUTED_2, tr("install it, then connect", &[]))
        } else if !setup.installed {
            (tr("Not set up", &[]), theme::KORE_PINK_TEXT, tr("found on this computer", &[]))
        } else if let Some(agents::CodexTrust::ChangedSinceLastEvent(_)) = &codex {
            (
                tr("Trust again", &[]),
                theme::KORE_PINK_TEXT,
                tr("The hook config changed after Codex's last event, so Codex turned the hooks off. Type /hooks in Codex and trust korekore's hooks again.", &[]),
            )
        } else if let Some(time) = last {
            (tr("Watching", &[]), theme::DONE_TEXT, tr("last event %@", &[&short_time(time)]))
        } else if setup.agent == "codex" {
            (tr("Waiting", &[]), theme::WAIT_TEXT, tr("for the first event… Codex runs the hooks once you trust them in /hooks.", &[]))
        } else {
            (tr("Waiting", &[]), theme::WAIT_TEXT, tr("for the first event…", &[]))
        };
        let icon = match setup.agent.as_str() {
            "claude" => &sprites::ICON_CC,
            "codex" => &sprites::ICON_CX,
            "opencode" => &sprites::ICON_OC,
            "copilot" => &sprites::ICON_GH,
            "pi" => &sprites::ICON_PI,
            _ => &sprites::ICON_API,
        };
        let id = setup.agent.clone();
        let actions: Element<'_, Message> = if setup.installed {
            row![pill(tr("Repair", &[]), Fill::Paper, Some(Message::PlanHooks(id.clone(), true))), pill(tr("Remove", &[]), Fill::Paper, Some(Message::PlanHooks(id, false)))]
                .spacing(8)
                .into()
        } else if setup.present {
            pill(tr("Connect", &[]), Fill::Pink, Some(Message::PlanHooks(id, true))).padding(Padding::from([4, 16])).into()
        } else {
            Space::new().into()
        };
        row![
            sprite(icon, 3),
            column![
                text(setup.name.clone()).font(bold()).size(15).color(theme::INK),
                row![tag(state, color), small(detail)].spacing(8),
            ]
            .spacing(3)
            .width(Length::Fill),
            actions,
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .padding(Padding::from([11, 14]))
        .into()
    }

    /// Before anything is written, what it would change (canvas: "AGENTS · BEFORE WRITING A HOOK").
    fn plan_panel(&self) -> Option<Element<'_, Message>> {
        self.hook_plan.as_ref().map(|plan| {
            let lines: Element<'_, Message> = if plan.lines.is_empty() {
                tag(tr("Nothing to change · already connected", &[]), theme::MUTED_2)
            } else {
                container(
                    scrollable(column(plan.lines.iter().map(|line| {
                        let color = if line.starts_with('+') {
                            theme::MINT
                        } else if line.starts_with('-') {
                            theme::PEACH
                        } else {
                            theme::BOX_TEXT_MUTED
                        };
                        text(line.clone()).font(PIXEL).size(12).color(color).into()
                    })))
                    .height(Length::Fixed(160.0)),
                )
                .padding(Padding::from([10, 12]))
                .width(Length::Fill)
                .style(|_| container::Style {
                    background: Some(theme::INK.into()),
                    border: Border { radius: theme::RADIUS_SM.into(), ..Border::default() },
                    ..container::Style::default()
                })
                .into()
            };
            card_with(
                column![
                    text(tr("Will change %@", &[&plan.config.display()])).font(bold()).size(16).color(theme::INK),
                    note(tr("Only korekore's own entries; your other hooks stay.", &[])),
                    lines,
                ]
                .push((plan.agent == "codex").then(|| note(tr("After writing, open /hooks in Codex to review and trust this config — the app can't do that for you.", &[]))))
                .push(row![
                    Space::new().width(Length::Fill),
                    pill(tr("Cancel", &[]), Fill::Paper, Some(Message::ApplyHooks(false))),
                    pill(tr("Write", &[]), Fill::Sun, (!plan.lines.is_empty()).then_some(Message::ApplyHooks(true))),
                ]
                .spacing(8))
                .spacing(10),
                theme::PAPER,
                theme::INK,
                theme::STROKE_BOLD,
            )
            .into()
        })
    }

    // MARK: Conversation

    fn kk_conversation(&self) -> Element<'_, Message> {
        let choice = self.resident_info.as_ref().and_then(|info| info.brain_setting);
        let chip = |label: String, tile: Option<&'static super::Sprite>, option: client::BrainChoice| -> Element<'_, Message> {
            let on = choice == Some(option);
            let (background, foreground) = if on { (theme::INK, theme::CREAM) } else { (theme::PAPER, theme::INK) };
            button(row![].push(tile.map(|tile| sprite(tile, 2))).push(text(label).font(bold()).size(13)).spacing(6).align_y(Alignment::Center))
                .padding(if tile.is_some() { Padding { top: 3.0, right: 12.0, bottom: 3.0, left: 4.0 } } else { Padding::from([4, 12]) })
                .on_press(Message::PickBrain(option))
                .style(move |_, _| button::Style {
                    background: Some(background.into()),
                    text_color: foreground,
                    border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: theme::RADIUS_PILL.into() },
                    ..button::Style::default()
                })
                .into()
        };
        let header = row![]
            .push(choice.map(|_| text(tr("Answered by", &[])).font(bold()).size(13).color(theme::INK)))
            .push(choice.map(|_| {
                row![
                    chip(tr("Automatic", &[]), None, client::BrainChoice::Auto),
                    chip("Codex".to_owned(), Some(&sprites::ICON_CX), client::BrainChoice::Codex),
                    chip("Claude Code".to_owned(), Some(&sprites::ICON_CC), client::BrainChoice::Claude),
                ]
                .spacing(6)
            }))
            .push(Space::new().width(Length::Fill))
            .push(pill(tr("Refresh", &[]), Fill::Paper, (!self.conversation_loading).then_some(Message::RefreshConversation)))
            .spacing(10)
            .align_y(Alignment::Center);
        let key = choice.filter(|choice| *choice != client::BrainChoice::Codex).map(|_| self.key_card());
        let body: Element<'_, Message> = if let Some(problem) = &self.resident_problem {
            self.voice_needs(problem, choice)
        } else {
            match &self.conversation {
                None => card(tag(tr("Loading the conversation…", &[]), theme::INK)).into(),
                Some(Err(client::HistoryError::DaemonDown)) => trouble(tr("The daemon isn't running. Restart it on the Advanced page.", &[])),
                Some(Err(client::HistoryError::Unavailable(why))) => trouble(tr("The buddy's session isn't available: %@", &[why])),
                Some(Ok(exchanges)) if exchanges.is_empty() => card(
                    column![
                        row![badge(&sprites::KORE_NORMAL, false, theme::INK), heading(tr("Nothing said yet", &[]))].spacing(10).align_y(Alignment::Center),
                        note(tr("Hold K0 on the box and talk to the buddy. Your words and its replies show up here.", &[])),
                    ]
                    .spacing(8),
                )
                .into(),
                Some(Ok(exchanges)) => card(
                    scrollable(
                        column(exchanges.iter().map(|exchange| {
                            column![mine(&exchange.said)].extend(exchange.replies.iter().map(|reply| buddy(reply))).spacing(8).into()
                        }))
                        .spacing(12)
                        .padding(Padding::ZERO.right(14)),
                    )
                    .anchor_bottom()
                    .height(Length::Fill),
                )
                .padding(16)
                .height(Length::Fill)
                .into(),
            }
        };
        column![header]
            .push(key)
            .push(container(body).height(Length::Fill))
            .push(text(tr("last 20 exchanges · newest at the bottom · stays on this computer", &[])).font(PIXEL).size(12).color(theme::MUTED_2))
            .spacing(14)
            .height(Length::Fill)
            .into()
    }

    fn key_card(&self) -> Element<'_, Message> {
        let saved = self.resident_info.as_ref().and_then(|info| info.anthropic_key_set) == Some(true);
        let key: Element<'_, Message> = if saved {
            row![tag(tr("Anthropic API key saved", &[]), theme::DONE_TEXT), Space::new().width(Length::Fill), pill(tr("Remove", &[]), Fill::Paper, Some(Message::RemoveAnthropicKey))]
                .align_y(Alignment::Center)
                .into()
        } else {
            let draft = self.anthropic_key_draft.trim();
            column![
                text(tr("Anthropic API key", &[])).font(bold()).size(13).color(theme::INK),
                row![
                    field(&tr("Anthropic API key", &[]), &self.anthropic_key_draft)
                        .secure(true)
                        .on_input(Message::AnthropicKeyDraft)
                        .on_submit(Message::SaveAnthropicKey),
                    pill(tr("Save", &[]), Fill::Sun, (!draft.is_empty()).then_some(Message::SaveAnthropicKey)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                link(tr("Get a key in the Claude Console", &[]), Message::OpenAnthropicConsole),
            ]
            .spacing(8)
            .into()
        };
        card(
            column![key, note(tr("Claude Code answers only with your own Anthropic API key, billed to your Anthropic account; a Claude subscription can't be used here. What you say and your Agents' task titles go to Anthropic.", &[]))]
                .push(self.resident_setting_error.as_ref().map(|error| tag(error, theme::KORE_PINK_TEXT)))
                .spacing(8),
        )
        .into()
    }

    fn voice_needs(&self, problem: &client::Problem, choice: Option<client::BrainChoice>) -> Element<'_, Message> {
        let claude = matches!(problem, client::Problem::ClaudeMissing | client::Problem::AnthropicKeyMissing);
        let detail = match problem {
            client::Problem::CodexMissing => tr("Voice commands are answered by Codex on this computer, and it isn't installed. Install the Codex CLI, then sign in with `codex login`. This tab notices within half a minute.", &[]),
            client::Problem::CodexLoggedOut => tr("Voice commands are answered by Codex on this computer, and it isn't signed in. Run `codex login` in a terminal and sign in. This tab notices within half a minute.", &[]),
            client::Problem::ClaudeMissing => tr("Voice commands are answered by Claude Code on this computer, and it isn't installed. Install Claude Code; you don't need to sign in to it. This tab notices within half a minute.", &[]),
            client::Problem::AnthropicKeyMissing => tr("Claude Code answers voice commands only with your own Anthropic API key. Add your key above.", &[]),
            client::Problem::Other => tr("The buddy's session isn't available: %@", &[&"?"]),
        };
        let codex_problem = matches!(problem, client::Problem::CodexMissing | client::Problem::CodexLoggedOut);
        let install = match problem {
            client::Problem::CodexMissing => Some(link(tr("How to install the Codex CLI", &[]), Message::OpenCodexInstallPage)),
            client::Problem::ClaudeMissing => Some(link(tr("How to install Claude Code", &[]), Message::OpenClaudeInstallPage)),
            _ => None,
        };
        card(
            column![
                row![
                    sprite(if claude { &sprites::ICON_CC } else { &sprites::ICON_CX }, 3),
                    heading(if claude { tr("Voice needs Claude Code", &[]) } else { tr("Voice needs Codex", &[]) }),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                note(detail),
            ]
            .push((codex_problem && choice == Some(client::BrainChoice::Auto)).then(|| note(tr("Or let Claude Code answer, with your own Anthropic API key: add it above.", &[]))))
            .push(install)
            .spacing(8),
        )
        .into()
    }

    // MARK: Device

    fn kk_device(&self) -> Element<'_, Message> {
        let device = self.status.as_ref().map(|status| &status.device);
        let online = self.online();
        let busy = self.busy();
        let can_flash = online && (!device.is_some_and(|device| device.network) || device.is_some_and(|device| device.transfer));
        let updates = self.updates();
        let offer = updates.and_then(|updates| updates.firmware.as_ref());
        let downloaded = updates.and_then(status::Updates::firmware_directory).is_some();
        let outdated = updates.is_some_and(status::Updates::firmware_update_available);
        let finding = self.kk_finding();

        // The box's screen as last captured, at one point per pixel in the design's 352 × 264 frame: any other scale
        // makes the pixel art uneven. Before the first capture, the box redrawn from the status.
        let screen: Element<'_, Message> = match &self.screenshot {
            Some((_, handle)) => image(handle.clone()).filter_method(image::FilterMethod::Nearest).width(320).height(240).into(),
            None => view::screen(&BoxPicture::derive(self.status.as_ref()), 1.0),
        };
        let screen = container(container(screen).center_x(352).center_y(264).style(|_| container::Style {
            background: Some(theme::INK.into()),
            border: Border { radius: (theme::RADIUS_MD - theme::STROKE_BOLD).into(), ..Border::default() },
            ..container::Style::default()
        }))
        .padding(theme::STROKE_BOLD)
        .style(|_| sticker(theme::INK, theme::RADIUS_MD, theme::STROKE_BOLD, theme::STICKER_SM));
        let screen_actions = row![
            pill(tr("Refresh", &[]), Fill::Paper, (online && !self.screenshot_busy && !busy).then_some(Message::TakeScreenshot)),
            pill(tr("Save image", &[]), Fill::Paper, self.screenshot.is_some().then_some(Message::SaveScreenshot)),
            Space::new().width(Length::Fill),
        ]
        .push(self.screenshot_busy.then(|| tag(tr("Refreshing…", &[]), theme::MUTED_2)))
        .spacing(8)
        .align_y(Alignment::Center)
        .width(358);
        let link = match device {
            None => tr("daemon isn't running", &[]),
            Some(device) if !device.connected && !device.candidates.is_empty() => tr("Several devices found", &[]),
            Some(device) if !device.connected => tr("Box not found", &[]),
            Some(device) if device.relay => tr("Relay", &[]),
            Some(device) => {
                let port = device.port.as_deref().unwrap_or("?").trim_start_matches("/dev/");
                if device.network { format!("{} · {port}", tr("Wi-Fi", &[])) } else { format!("USB · {port}") }
            }
        };
        let version = device.and_then(|device| device.firmware_version.clone()).or_else(|| device.and_then(|device| device.firmware_label())).unwrap_or_else(|| "—".to_owned());
        let firmware: Element<'_, Message> = match offer {
            Some(offer) if outdated => row![
                text(format!("{version} ")).font(PIXEL).size(13).color(theme::INK),
                tag(tr("· %@ available", &[&offer.version]), theme::WAIT_TEXT)
            ]
            .into(),
            _ if online && (offer.is_some() || updates.is_some_and(|updates| updates.last_check.is_some())) => {
                row![text(format!("{version} ")).font(PIXEL).size(13).color(theme::INK), tag(tr("· up to date", &[]), theme::DONE_TEXT)].into()
            }
            _ => text(version).font(PIXEL).size(13).color(theme::INK).into(),
        };
        let the_box = card(
            column![
                heading(tr("The box", &[])),
                column![small(tr("Link", &[])), text(link).font(PIXEL).size(13).color(theme::INK)].spacing(2),
                column![small(tr("Firmware", &[])), firmware].spacing(2),
                Space::new().height(Length::Fill),
            ]
            .push((outdated && online && !can_flash).then(|| note(tr("Plug in the cable to update the box's firmware.", &[]))))
            .push((outdated && can_flash).then(|| {
                pill_sized(tr("Update to %@", &[&offer.map(|offer| offer.version.clone()).unwrap_or_default()]), Fill::Pink, 13.0, (!busy).then_some(Message::AskFirmwareUpdate(true)))
                    .width(Length::Fill)
            }))
            .push(pill_sized(tr("Make the box blink", &[]), Fill::Sun, 13.0, (online && !busy).then_some(Message::Identify)).width(Length::Fill))
            .push(pill_sized(tr("Flash from file…", &[]), Fill::Paper, 13.0, (can_flash && !busy).then_some(Message::FlashFromFile)).width(Length::Fill))
            .spacing(10)
            .height(Length::Fixed(290.0)),
        );
        let top = row![column![screen, screen_actions].spacing(8), the_box].spacing(14);
        let firmware_state = self.firmware_state(can_flash, downloaded);
        let networked = device.filter(|device| online && device.box_key.is_some()).map(|device| {
            column![]
                .push(device.network.then(|| {
                    card_with(note(tr("Plug in the cable to change the box's network, update its firmware or see its paired computers.", &[])), theme::CREAM, theme::INK, theme::STROKE_BASE)
                }))
                .push(row![self.wifi_card(device), self.paired_card(device)].spacing(14))
                .spacing(14)
        });
        page(
            column![finding, top]
                .push(self.confirm_panel())
                .push(firmware_state)
                .push(networked)
                .spacing(14),
        )
    }

    /// Where the box is and what it runs, when that needs saying: taken by another computer, a pin, several devices
    /// or none, other firmware, an unsupported board. The Device page and the first-run guide both show it.
    pub(super) fn kk_finding(&self) -> iced::widget::Column<'_, Message> {
        let device = self.status.as_ref().map(|status| &status.device);
        let online = self.online();
        let busy = self.busy();
        let updates = self.updates();
        let downloaded = updates.and_then(status::Updates::firmware_directory).is_some();
        let other_firmware = online && device.is_some_and(|device| device.foreign_firmware && device.firmware_build.is_none());
        let operation = self.status.as_ref().and_then(|status| status.operation.as_ref());
        let replug = operation.is_some_and(|operation| operation.state == OperationState::Replug);
        let mut finding = column![].spacing(14);
        if let Some(owner_busy) = device.filter(|_| !online).and_then(|device| device.busy.as_ref()) {
            let owner = if owner_busy.owner.is_empty() { tr("another computer", &[]) } else { owner_busy.owner.clone() };
            let body: Element<'_, Message> = if owner_busy.reason == "usb" {
                note(tr("The box is plugged into %@.", &[&owner]))
            } else {
                column![note(tr("Take it over to drive it from here.", &[])), pill(tr("Take over", &[]), Fill::Sun, Some(Message::TakeOver))].spacing(8).into()
            };
            finding = finding.push(card(column![heading(tr("The box is with %@.", &[&owner])), body].spacing(8)));
        }
        if let Some(pin) = device.and_then(|device| device.pin.as_ref()) {
            finding = finding.push(card_with(
                column![
                    text(tr("Only looking at %@, set by %@.", &[&pin.value, &pin.variable])).font(bold()).size(13).color(theme::INK),
                    note(tr("To find the box on its own again, unset %@ and restart the daemon.", &[&pin.variable])),
                ]
                .spacing(4),
                theme::CREAM,
                theme::INK,
                theme::STROKE_BASE,
            ));
        }
        let candidates = device.map(|device| device.candidates.as_slice()).unwrap_or_default();
        if !online && !candidates.is_empty() {
            let rows = candidates.iter().map(|candidate| {
                let label = match &candidate.usb_serial {
                    Some(serial) => format!("{} · {serial}", candidate.port),
                    None => candidate.port.clone(),
                };
                let pick = candidate.usb_serial.clone().map(|serial| pill(tr("This is the box", &[]), Fill::Sun, (!busy).then_some(Message::ChooseBox(serial))));
                column![divider(), row![text(label).font(PIXEL).size(12).color(theme::INK).width(Length::Fill)].push(pick).align_y(Alignment::Center)]
                    .spacing(8)
                    .into()
            });
            finding = finding.push(card(
                column![heading(tr("Which one is your box?", &[])), note(tr("Several devices are plugged in. Pick yours once; korekore remembers it.", &[]))]
                    .extend(rows)
                    .spacing(8),
            ));
        } else if self.status.is_some() && !online && !busy && !replug && device.is_some_and(|device| device.pin.is_none()) {
            finding = finding.push(card(
                column![
                    row![badge(&sprites::KORE_SLEEP, true, theme::INK), heading(tr("Box not found", &[]))].spacing(10).align_y(Alignment::Center),
                    note(tr("Not showing up? Use a cable that carries data, not just power.", &[])),
                    note(tr("If the daemon log on the Advanced tab says “Permission denied”, your account can't open the box's port yet: restart the computer once, then replug the box.", &[])),
                    note(tr("Still nothing? Hold K0 on the box while you plug in the cable. The box starts in download mode with a dark screen, ready to be flashed with VibeBuddy firmware.", &[])),
                ]
                .spacing(8),
            ));
        }
        if other_firmware {
            let action: Element<'_, Message> = if downloaded {
                column![
                    note(tr("Flash korekore to use it; its current firmware will be replaced.", &[])),
                    pill(tr("Flash korekore", &[]), Fill::Pink, (!busy).then_some(Message::AskFirmwareUpdate(true))),
                ]
                .spacing(8)
                .into()
            } else {
                self.firmware_unavailable()
            };
            finding = finding.push(card(
                column![heading(tr("The box isn't running korekore firmware", &[])), action, small(tr("If flashing can't start: hold K0 while plugging in, to enter download mode.", &[]))].spacing(8),
            ));
        }
        if let Some(board) = device.filter(|_| online).and_then(|device| device.unsupported_board.as_ref()) {
            finding = finding.push(card(
                column![heading(tr("This is a %@ board.", &[board])), note(tr("Released firmware doesn't run on it, so no update is offered.", &[]))].spacing(8),
            ));
        }

        finding
    }

    fn firmware_unavailable(&self) -> Element<'_, Message> {
        let updates = self.updates();
        let offer = updates.and_then(|updates| updates.firmware.as_ref());
        let reason: Element<'_, Message> = match updates {
            Some(updates) if updates.enabled => match (&updates.error, offer) {
                (Some(error), _) => tag(tr("Couldn't get the firmware: %@", &[error]), theme::KORE_PINK_TEXT),
                (None, Some(_)) => tag(tr("Downloading the firmware…", &[]), theme::MUTED_2),
                (None, None) => tag(tr("Looking for firmware…", &[]), theme::MUTED_2),
            },
            _ => note(tr("Update checks are off, so korekore doesn't download firmware. Turn them on in General, or get it from Releases.", &[])),
        };
        column![reason]
            .push(updates.is_some_and(|updates| updates.enabled).then(|| pill(tr("Check again", &[]), Fill::Paper, Some(Message::CheckForUpdatesNow))))
            .push(link(tr("Releases", &[]), Message::OpenReleases))
            .spacing(6)
            .into()
    }

    /// Flashing asks once more, inline (canvas: the confirmations, "inline panels on Linux").
    pub(super) fn confirm_panel(&self) -> Option<Element<'_, Message>> {
        let device = self.status.as_ref().map(|status| &status.device);
        let busy = self.busy();
        let online = self.online();
        let panel = |title: String, detail: String, action: String, flash: Option<Message>, cancel: Message, warn: bool| -> Element<'_, Message> {
            card_with(
                column![
                    text(title).font(bold()).size(16).color(if warn { theme::KORE_PINK_TEXT } else { theme::INK }),
                    note(detail),
                    row![Space::new().width(Length::Fill), pill(tr("Cancel", &[]), Fill::Paper, Some(cancel)), pill(action, if warn { Fill::Pink } else { Fill::Sun }, flash)].spacing(8),
                ]
                .spacing(10),
                theme::CREAM,
                if warn { theme::KORE_PINK_TEXT } else { theme::INK },
                theme::STROKE_BOLD,
            )
            .into()
        };
        if let Some(package) = &self.file_firmware {
            let current = device.and_then(|device| device.firmware_label()).unwrap_or_else(|| tr("unknown", &[]));
            return Some(panel(
                tr("Flash this firmware?", &[]),
                tr(
                    "Firmware package: %@\nBox now: %@\nThe box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes.",
                    &[&package.label(), &current],
                ),
                tr("Flash", &[]),
                (online && !busy).then_some(Message::FlashFile(true)),
                Message::FlashFile(false),
                false,
            ));
        }
        if !self.confirm_firmware {
            return None;
        }
        let other = online && device.is_some_and(|device| device.foreign_firmware && device.firmware_build.is_none());
        if other {
            return Some(panel(
                tr("Flash the box with korekore?", &[]),
                tr("The box's current firmware and data will be erased and can't be recovered. It restarts on its own when done; over the UART port this takes a few minutes.", &[]),
                tr("Flash", &[]),
                (!busy).then_some(Message::FlashFirmware),
                Message::AskFirmwareUpdate(false),
                true,
            ));
        }
        let offer = self.updates().and_then(|updates| updates.firmware.as_ref())?;
        let restart = tr("The box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes.", &[]);
        let detail = match offer.notes(crate::i18n::is_chinese()) {
            Some(notes) => format!("{notes}\n\n{restart}"),
            None => restart,
        };
        Some(panel(
            tr("Update the box firmware to %@?", &[&offer.version]),
            detail,
            tr("Update", &[]),
            (!busy).then_some(Message::FlashFirmware),
            Message::AskFirmwareUpdate(false),
            false,
        ))
    }

    fn firmware_state(&self, can_flash: bool, downloaded: bool) -> Option<Element<'_, Message>> {
        let device = self.status.as_ref().map(|status| &status.device);
        let operation = self.status.as_ref().and_then(|status| status.operation.as_ref()).filter(|operation| operation.kind == OperationKind::Firmware);
        let retry = |label: String| pill(label, Fill::Paper, (can_flash && downloaded).then_some(Message::FlashFirmware));
        match operation.map(|operation| (operation, operation.state)) {
            Some((operation, OperationState::Running)) => Some(
                card(
                    column![
                        row![heading(tr("Updating the firmware…", &[])), Space::new().width(Length::Fill), tag(percent(operation.progress), theme::INK)],
                        bar(operation.progress, theme::SUN),
                    ]
                    .spacing(8),
                )
                .into(),
            ),
            Some((_, OperationState::Done)) => {
                Some(card_with(tag(tr("Firmware flashed · the box has restarted", &[]), theme::DONE_TEXT), theme::PAPER, theme::DONE_TEXT, theme::STROKE_BASE).into())
            }
            Some((_, OperationState::Replug)) => Some(
                card_with(
                    column![heading(tr("One more step", &[])), note(tr("The firmware is on the box, but it didn't start. Unplug the box and plug it back in.", &[]))].spacing(6),
                    theme::CREAM,
                    theme::INK,
                    theme::STROKE_BASE,
                )
                .into(),
            ),
            Some((operation, OperationState::Failed)) => Some(
                card_with(
                    column![
                        text(tr("The update didn't finish", &[])).font(bold()).size(15).color(theme::KORE_PINK_TEXT),
                        note(tr("Failed: %@", &[&operation.message])),
                    ]
                    .push((!device.is_some_and(|device| device.transfer)).then(|| note(tr("Before retrying, hold K0 on the box and replug the cable to put it in download mode.", &[]))))
                    .push(row![retry(tr("Try again", &[])), pill(tr("Report a problem…", &[]), Fill::Paper, Some(Message::ReportProblem))].spacing(8))
                    .spacing(8),
                    theme::PAPER,
                    theme::KORE_PINK_TEXT,
                    theme::STROKE_BASE,
                )
                .into(),
            ),
            None if device.is_some_and(|device| device.rolled_back) => Some(
                card_with(
                    column![
                        text(tr("The update didn't take", &[])).font(bold()).size(15).color(theme::KORE_PINK_TEXT),
                        note(tr("The box went back to firmware %@ and is fine. Try again later or report a problem.", &[&device.and_then(|device| device.firmware_label()).unwrap_or_else(|| "—".to_owned())])),
                        row![retry(tr("Try again", &[])), pill(tr("Report a problem…", &[]), Fill::Paper, Some(Message::ReportProblem))].spacing(8),
                    ]
                    .spacing(8),
                    theme::PAPER,
                    theme::KORE_PINK_TEXT,
                    theme::STROKE_BASE,
                )
                .into(),
            ),
            None => {
                let offer = self.updates().and_then(|updates| updates.firmware.as_ref());
                let other = self.online() && device.is_some_and(|device| device.foreign_firmware && device.firmware_build.is_none());
                (!other && offer.is_some_and(|offer| offer.newer_than_box) && !downloaded).then(|| card(self.firmware_unavailable()).into())
            }
        }
    }

    pub(super) fn wifi_card<'a>(&'a self, device: &'a status::Device) -> Element<'a, Message> {
        let busy = self.busy();
        let line = |label: String, value: Element<'a, Message>| -> Element<'a, Message> {
            row![text(label).font(regular()).size(13).color(theme::MUTED), Space::new().width(Length::Fill), value].align_y(Alignment::Center).into()
        };
        let mut wifi = column![
            heading(tr("Wi-Fi", &[])),
            line(tr("Network", &[]), text(device.wifi_network.clone().unwrap_or_else(|| tr("Not set", &[]))).font(semibold()).size(13).color(theme::INK).into()),
        ]
        .spacing(8);
        if let Some(network) = &device.wifi_network {
            wifi = match (&device.wifi_error, &device.wifi_address) {
                (Some(why), _) => wifi.push(tag(tr("Couldn't join", &[]), theme::KORE_PINK_TEXT)).push(note(wifi_failure(why))),
                (None, Some(address)) => wifi.push(line(tr("Status", &[]), tag(tr("Joined · %@", &[address]), theme::DONE_TEXT))),
                (None, None) => wifi
                    .push(row![text(tr("Joining %@…", &[network])).font(bold()).size(13).color(theme::INK), Space::new().width(Length::Fill), tag(tr("Up to 15 seconds", &[]), theme::MUTED_2)])
                    .push(bar(0.35, theme::SKY)),
            };
        }
        let fields = device.wifi_network.is_none() || device.wifi_error.is_some() || self.wifi_editing;
        if device.network {
            wifi = wifi.push(note(tr("Plug in the cable to change the box's network.", &[])));
        } else if !fields {
            wifi = wifi.push(
                row![pill(tr("Change network", &[]), Fill::Paper, Some(Message::ChangeWifi)), pill(tr("Forget", &[]), Fill::Paper, (!busy).then_some(Message::ForgetWifi))].spacing(8),
            );
        } else {
            let valid = !self.wifi_ssid.is_empty() && self.wifi_ssid.len() <= 32 && self.wifi_password.len() <= 63;
            let seen = device.wifi_seen.iter().take(8).map(|network| {
                let fill = if network.ssid == self.wifi_ssid { Fill::Ink } else { Fill::Paper };
                pill(format!("{}  {} dBm", network.ssid, network.rssi), fill, Some(Message::WifiSsid(network.ssid.clone()))).into()
            });
            wifi = wifi
                .push(row![small(tr("Nearby networks", &[])), Space::new().width(Length::Fill), pill(tr("Scan", &[]), Fill::Paper, (!busy).then_some(Message::ScanWifi))].align_y(Alignment::Center))
                .push(column(seen).spacing(4))
                .push(field(&tr("Network name", &[]), &self.wifi_ssid).on_input(Message::WifiSsid))
                .push(field(&tr("Password", &[]), &self.wifi_password).secure(true).on_input(Message::WifiPassword))
                .push(
                    row![pill(tr("Join", &[]), Fill::Sun, (valid && !busy).then_some(Message::JoinWifi))]
                        .push(device.wifi_network.as_ref().map(|_| pill(tr("Forget", &[]), Fill::Paper, (!busy).then_some(Message::ForgetWifi))))
                        .spacing(8),
                );
        }
        card(wifi.push(small(tr("On the same network, a paired computer drives the box without the cable.", &[])))).width(Length::FillPortion(1)).into()
    }

    fn paired_card<'a>(&'a self, device: &'a status::Device) -> Element<'a, Message> {
        let busy = self.busy();
        let rows = device.paired_computers.iter().map(|computer| {
            let name = if computer.name.is_empty() { tr("Unnamed computer", &[]) } else { computer.name.clone() };
            let action: Element<'_, Message> = if device.computer_key.as_ref() == Some(&computer.key) {
                tag(tr("This computer", &[]), theme::KORE_PINK_TEXT)
            } else {
                pill(tr("Remove", &[]), Fill::Paper, (!busy).then_some(Message::Unpair(computer.key.clone()))).into()
            };
            container(row![text(name).font(semibold()).size(13).color(theme::INK).width(Length::Fill), action].align_y(Alignment::Center))
                .padding(Padding::from([6, 10]))
                .style(|_| container::Style {
                    border: Border { color: theme::LINE, width: theme::STROKE_BASE, radius: theme::RADIUS_SM.into() },
                    ..container::Style::default()
                })
                .into()
        });
        card(
            column![heading(tr("Paired computers", &[]))]
                .push(device.network.then(|| note(tr("Plug in the cable to see the paired computers.", &[]))))
                .extend(rows)
                .push(small(tr("A computer pairs when it's plugged in. To forget them all, use COMPUTERS in the box's menu.", &[])))
                .spacing(8),
        )
        .width(Length::FillPortion(1))
        .into()
    }

    // MARK: General

    fn kk_general(&self) -> Element<'_, Message> {
        let notify = self.status.as_ref().is_none_or(|status| status.config.notify_link);
        let relay = self.status.as_ref().is_none_or(|status| status.config.relay);
        let updates = self.updates();
        let enabled = updates.is_some_and(|updates| updates.enabled);
        let check = self.status.as_ref().and_then(|status| status.config.check_updates).unwrap_or(enabled);
        let have_status = self.status.is_some();
        let line = match updates {
            Some(updates) if updates.enabled => match (&updates.error, &updates.app, &updates.last_check) {
                (Some(error), _, _) => tag(tr("Last check failed: %@", &[error]), theme::KORE_PINK_TEXT),
                (None, Some(app), _) => tag(tr("korekore %@ is available", &[&app.version]), theme::DONE_TEXT),
                (None, None, Some(checked)) => tag(tr("Up to date · checked %@", &[&short_time(checked)]), theme::DONE_TEXT),
                (None, None, None) => tag(tr("Not checked yet", &[]), theme::MUTED_2),
            },
            _ => tag(tr("Update checks off", &[]), theme::MUTED_2),
        };
        let unsupported = updates.filter(|updates| updates.unsupported_app).map(|_| {
            card_with(
                text(tr("This version of korekore is no longer supported. It keeps working; update to keep getting firmware for the box.", &[])).font(regular()).size(13).color(theme::INK),
                theme::CREAM,
                theme::KORE_PINK_TEXT,
                theme::STROKE_BASE,
            )
        });
        // No self-update here: a new release is installed by rerunning its install.sh, as the README says.
        let upgrade = updates.and_then(|updates| updates.app.as_ref()).map(|_| note(tr("To upgrade, download the new release and run its install.sh again.", &[])));
        let settings = rows([
            setting(tr("Language", &[]), picker(UiLanguage::ALL.to_vec(), Some(self.language), Message::PickLanguage)),
            setting(tr("Launch at login", &[]), switch(self.launch_at_login, Some(Message::LaunchAtLogin(!self.launch_at_login)))),
            setting(tr("Tell me when the box disconnects or the daemon fails", &[]), switch(notify, have_status.then_some(Message::NotifyLink(!notify)))),
            setting(tr("Reach the box through the Relay when it's away from this computer", &[]), switch(relay, have_status.then_some(Message::Relay(!relay)))),
        ]);
        let updates_card = rows([
            setting(tr("Check for updates", &[]), switch(check, have_status.then_some(Message::CheckUpdates(!check)))),
            row![container(line).width(Length::Fill), pill(tr("Releases", &[]), Fill::Paper, Some(Message::OpenReleases)), pill(tr("Check now", &[]), Fill::Paper, enabled.then_some(Message::CheckForUpdatesNow))]
                .spacing(8)
                .align_y(Alignment::Center)
                .padding(Padding::from([12, 16]))
                .into(),
        ]);
        let about = card(
            row![
                container(sprite(&sprites::KORE_NORMAL, 2)).padding(Padding::from([5, 6])).style(|_| container::Style {
                    background: Some(theme::KORE_PINK.into()),
                    border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: 12.0.into() },
                    ..container::Style::default()
                }),
                column![
                    row![text("korekore").font(bold()).size(18).color(theme::INK), sprite(&sprites::TAG_YOUR_VIBEBUDDY, 2)].spacing(8).align_y(Alignment::End),
                    tag(format!("App {}", env!("CARGO_PKG_VERSION")), theme::MUTED),
                ]
                .spacing(3)
                .width(Length::Fill),
                pill(tr("Report a problem…", &[]), Fill::Paper, Some(Message::ReportProblem)),
            ]
            .spacing(14)
            .align_y(Alignment::Center),
        )
        .padding(Padding::from([14, 16]));
        page(
            column![]
                .push(unsupported)
                .push(settings)
                .push(self.restart_offer.as_ref().map(|offer| self.restart_panel(offer)))
                .push(container(text(tr("Updates", &[])).font(bold()).size(15).color(theme::INK)).padding(Padding::ZERO.left(4)))
                .push(updates_card)
                .push(upgrade)
                .push(about)
                .spacing(14),
        )
    }

    /// After a language change, inline (canvas: "Restart korekore to change the language?").
    fn restart_panel(&self, offer: &crate::RestartOffer) -> Element<'_, Message> {
        let device = self.status.as_ref().map(|status| &status.device);
        let voice: Option<Element<'_, Message>> = match &offer.voice {
            Some(voice) => {
                let label = if device.is_some_and(|device| device.bridge) {
                    tr("Also switch the box's voice to %@ (a few minutes over the UART bridge; keep it plugged in)", &[&voice.name])
                } else {
                    tr("Also switch the box's voice to %@", &[&voice.name])
                };
                Some(iced::widget::checkbox(offer.switch_voice).label(label).on_toggle(Message::SwitchVoice).font(regular()).text_size(13).into())
            }
            None if !device.is_some_and(|device| device.connected) => {
                Some(note(tr("The box isn't connected, so its character stays as it is. You can change it later on the Character tab.", &[])))
            }
            None => None,
        };
        card_with(
            column![text(tr("Restart korekore to change the language?", &[])).font(bold()).size(16).color(theme::INK)]
                .push(voice)
                .push(row![
                    Space::new().width(Length::Fill),
                    pill(tr("Not now", &[]), Fill::Paper, Some(Message::RestartForLanguage(false))),
                    pill(tr("Restart", &[]), Fill::Sun, Some(Message::RestartForLanguage(true))),
                ]
                .spacing(8))
                .spacing(10),
            theme::CREAM,
            theme::INK,
            theme::STROKE_BOLD,
        )
        .into()
    }

    // MARK: Advanced

    fn kk_advanced(&self) -> Element<'_, Message> {
        let running = self.status.is_some();
        let state: Element<'_, Message> = match &self.status {
            Some(status) => row![
                text(tr("Running", &[]).to_uppercase()).font(PIXEL).size(16).color(theme::MINT),
                text(format!(" · {}", status.daemon.short_build())).font(PIXEL).size(16).color(theme::CREAM),
            ]
            .into(),
            None => text(tr("Not running", &[]).to_uppercase()).font(PIXEL).size(16).color(theme::PEACH).into(),
        };
        let daemon = card_with(
            row![
                sprite(if running { &sprites::KORE_WORKING } else { &sprites::KORE_SAD }, 3),
                column![text("DAEMON").font(PIXEL).size(13).color(theme::BOX_TEXT_MUTED), state, text("127.0.0.1:7331").font(PIXEL).size(12).color(theme::BOX_TEXT_MUTED)]
                    .spacing(4)
                    .width(Length::Fill),
                button(text(tr("Restart daemon", &[])).font(bold()).size(13))
                    .padding(Padding::from([6, 14]))
                    .on_press_maybe(running.then_some(Message::RestartDaemon))
                    .style(|_, _| button::Style {
                        background: Some(theme::SUN.into()),
                        text_color: theme::INK,
                        border: Border { color: theme::CREAM, width: theme::STROKE_BASE, radius: theme::RADIUS_PILL.into() },
                        ..button::Style::default()
                    }),
            ]
            .spacing(16)
            .align_y(Alignment::Center),
            theme::INK,
            theme::INK,
            theme::STROKE_BASE,
        )
        .padding(16);
        let small_card = |title: String, detail: String, buttons: Element<'static, Message>| -> Element<'static, Message> {
            card(column![heading(title), small(detail), Space::new().height(Length::Fill), buttons].spacing(8).height(Length::Fixed(110.0))).width(Length::FillPortion(1)).into()
        };
        let config = config_dir().map(|dir| dir.join("config.json").display().to_string()).unwrap_or_default();
        let home = std::env::var("HOME").unwrap_or_default();
        let config = if !home.is_empty() && config.starts_with(&home) { config.replacen(&home, "~", 1) } else { config };
        page(
            column![
                daemon,
                row![
                    // The daemon logs to the journal here, not a file; the folder holds the hook's log.
                    small_card(
                        tr("Logs", &[]),
                        tr("The daemon's log in the journal, and the hook's log as a file.", &[]),
                        row![pill(tr("Show daemon log", &[]), Fill::Paper, Some(Message::ShowDaemonLog)), pill(tr("Open logs folder", &[]), Fill::Paper, Some(Message::OpenLogsFolder))]
                            .spacing(8)
                            .into(),
                    ),
                    small_card(
                        tr("Diagnostics", &[]),
                        tr("Logs, config and both sides' build IDs in one file, for a bug report.", &[]),
                        pill(tr("Export diagnostics…", &[]), Fill::Paper, Some(Message::ExportDiagnostics)).into(),
                    ),
                ]
                .spacing(14),
                container(column![small(tr("Config file", &[])), text(config).font(PIXEL).size(13).color(theme::INK)].spacing(2))
                    .padding(Padding::from([10, 14]))
                    .width(Length::Fill)
                    .style(|_| container::Style {
                        background: Some(theme::PAPER.into()),
                        border: Border { color: theme::MIST_TEXT, width: theme::STROKE_BASE, radius: 12.0.into() },
                        ..container::Style::default()
                    }),
            ]
            .spacing(14),
        )
    }
}

fn stage(fill: iced::Color) -> container::Style {
    container::Style {
        background: Some(fill.into()),
        border: Border { radius: theme::RADIUS_SM.into(), ..Border::default() },
        ..container::Style::default()
    }
}

fn sticker(fill: iced::Color, radius: f32, stroke: f32, shadow: f32) -> container::Style {
    container::Style {
        background: Some(fill.into()),
        border: Border { color: theme::INK, width: stroke, radius: radius.into() },
        shadow: iced::Shadow { color: theme::INK, offset: iced::Vector::new(0.0, shadow), blur_radius: 0.0 },
        ..container::Style::default()
    }
}

fn trouble<'a>(detail: String) -> Element<'a, Message> {
    card_with(
        column![text(tr("Couldn't read the conversation", &[])).font(bold()).size(15).color(theme::KORE_PINK_TEXT), note(detail)].spacing(8),
        theme::PAPER,
        theme::KORE_PINK_TEXT,
        theme::STROKE_BASE,
    )
    .into()
}

fn link<'a>(label: String, on_press: Message) -> Element<'a, Message> {
    button(text(label).font(regular()).size(13).color(theme::KORE_PINK_TEXT)).padding(0).on_press(on_press).style(|_, _| button::Style::default()).into()
}

/// Your words: right, in an Ink bubble.
fn mine(words: &str) -> Element<'_, Message> {
    container(
        container(text(words).font(regular()).size(14).color(theme::CREAM))
            .padding(Padding::from([8, 14]))
            .max_width(520)
            .style(|_| container::Style {
                background: Some(theme::INK.into()),
                border: Border { radius: iced::border::Radius { top_left: 16.0, top_right: 16.0, bottom_right: 4.0, bottom_left: 16.0 }, ..Border::default() },
                ..container::Style::default()
            }),
    )
    .align_right(Length::Fill)
    .into()
}

/// The buddy's reply: left, in a cream bubble, beside Kore.
fn buddy(words: &str) -> Element<'_, Message> {
    row![
        container(sprite(&sprites::KORE_HAPPY, 1)).padding(Padding::from([3, 4])).style(|_| container::Style {
            background: Some(theme::INK.into()),
            border: Border { radius: 8.0.into(), ..Border::default() },
            ..container::Style::default()
        }),
        container(text(words).font(regular()).size(14).color(theme::INK)).padding(Padding::from([8, 14])).max_width(560).style(|_| container::Style {
            background: Some(theme::CREAM.into()),
            border: Border {
                color: theme::INK,
                width: theme::STROKE_BASE,
                radius: iced::border::Radius { top_left: 16.0, top_right: 16.0, bottom_right: 16.0, bottom_left: 4.0 },
            },
            ..container::Style::default()
        }),
    ]
    .spacing(8)
    .align_y(Alignment::End)
    .into()
}
