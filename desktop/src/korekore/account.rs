//! The account (ADR-0018), the user's own keys and making a Character (ADR-0020) in the korekore look. The design has
//! no boards for them, so they use its cards, rows and pills; the state and messages are the current tabs' in main.rs.

use iced::widget::{Space, button, column, container, image, row, text};
use iced::{Alignment, Border, Element, Length, Padding};

use super::controls::{Fill, bar, bold, card, card_with, divider, field, heading, note, picker, pill, rows, semibold, small, tag};
use super::theme;
use crate::i18n::tr;
use crate::status;
use crate::{Lender, MakeForm, Message, ROBOT, assets, client, making};

impl crate::App {
    // MARK: Account

    /// At the top of the General page: who is signed in, the plan and today's use, then the own keys.
    pub(super) fn kk_account(&self) -> Element<'_, Message> {
        let title = |label: String| container(text(label).font(bold()).size(15).color(theme::INK)).padding(Padding::ZERO.left(4));
        let keys = self.status.as_ref().and_then(|status| status.voice_keys.as_ref()).map(|keys| column![title(tr("Your own keys", &[])), self.kk_own_keys(keys)].spacing(14));
        column![title(tr("Account", &[])), rows(self.kk_account_rows())].push(keys).spacing(14).into()
    }

    fn kk_account_rows(&self) -> Vec<Element<'_, Message>> {
        fn line(content: Element<'_, Message>) -> Element<'_, Message> {
            container(content).padding(Padding::from([12, 16])).width(Length::Fill).into()
        }
        let Some(status) = &self.status else {
            return vec![line(note(tr("The daemon isn't running. Restart it on the Advanced tab.", &[])))];
        };
        let account = &status.account;
        if !account.available {
            return vec![line(note(tr("Signing in isn't available in this version of VibeBuddy.", &[])))];
        }
        if !account.signed_in {
            return vec![line(
                row![container(note(tr("Signing in opens your browser. Voice commands need an account.", &[]))).width(Length::Fill)]
                    .push(self.signing_in.then(|| tag(tr("Waiting for the browser…", &[]), theme::WAIT_TEXT)))
                    .push(pill(tr("Sign in", &[]), Fill::Sun, Some(Message::SignIn)))
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .into(),
            )];
        }
        let who = match (&account.github_login, account.github_id) {
            (Some(login), _) => tr("Signed in with GitHub as %@", &[login]),
            (None, Some(id)) => tr("Signed in with GitHub (user %lld)", &[&id]),
            (None, None) => tr("Signed in as %@", &[&account.email.as_deref().unwrap_or("—")]),
        };
        let mut items = vec![line(
            row![
                text(who).font(bold()).size(14).color(theme::INK).width(Length::Fill),
                pill(tr("Sign out", &[]), Fill::Paper, Some(Message::SignOut)),
                pill(tr("Delete account…", &[]), Fill::Paper, Some(Message::AskDeleteAccount(true))),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
        )];
        if self.confirm_delete_account {
            items.push(divider());
            items.push(line(
                column![
                    text(tr("Delete your VibeBuddy account?", &[])).font(bold()).size(16).color(theme::KORE_PINK_TEXT),
                    note(tr("This deletes the account and everything kept about it, and signs out on every computer. It can't be undone.", &[])),
                    row![
                        Space::new().width(Length::Fill),
                        pill(tr("Cancel", &[]), Fill::Paper, Some(Message::AskDeleteAccount(false))),
                        pill(tr("Delete account", &[]), Fill::Pink, Some(Message::DeleteAccount)),
                    ]
                    .spacing(8),
                ]
                .spacing(10)
                .into(),
            ));
        }
        if status::Plan::SHOWN && let Some(plan) = &account.plan {
            let (name, color) = if plan.paid() { (tr("Paid plan", &[]), theme::DONE_TEXT) } else { (tr("Free", &[]), theme::INK) };
            items.push(divider());
            items.push(line(row![text(tr("Plan", &[])).font(bold()).size(14).color(theme::INK).width(Length::Fill), tag(name, color)].align_y(Alignment::Center).into()));
            if let Some(month) = plan.month_line() {
                items.push(divider());
                items.push(line(tag(month, theme::MUTED_2)));
            }
        }
        let today = match (account.refused.as_deref(), &account.today) {
            (Some(refused), _) => {
                let back = status::voice_comes_back(chrono::Utc::now(), &chrono::Local);
                let line = if refused == "global_cap" {
                    tr("Voice is paused for everyone today. It comes back at %@.", &[&back])
                } else {
                    tr("Today's voice is used up. It comes back at %@.", &[&back])
                };
                Some(tag(line, theme::KORE_PINK_TEXT))
            }
            (None, Some(usage)) => Some(tag(tr("Voice today: %lld of %lld", &[&usage.commands, &usage.commands_limit]), theme::MUTED_2)),
            (None, None) => None,
        };
        if let Some(today) = today {
            items.push(divider());
            items.push(line(today));
        }
        items
    }

    /// Each key is typed once and never shown again: the daemon only says whether it has one.
    fn kk_own_keys(&self, keys: &status::VoiceKeys) -> Element<'_, Message> {
        let mut items: Vec<Element<'_, Message>> = vec![
            container(note(tr("With your own key, spoken replies in that provider's voices go straight to it, billed to your account there, and never through VibeBuddy.", &[])))
                .padding(Padding::from([12, 16]))
                .into(),
        ];
        for provider in client::KeyProvider::ALL {
            let (name, key, refused) = match provider {
                client::KeyProvider::ElevenLabs => ("ElevenLabs".to_owned(), &keys.elevenlabs, tr("ElevenLabs refused your own key. Replies in its voices stay silent until you set another one.", &[])),
                client::KeyProvider::Volc => (tr("Volcano Engine", &[]), &keys.volc, tr("Volcano Engine refused your own key. Replies in its voices stay silent until you set another one.", &[])),
            };
            let entry: Element<'_, Message> = if key.set {
                row![tag(tr("Set", &[]), theme::DONE_TEXT), pill(tr("Clear", &[]), Fill::Paper, Some(Message::ClearOwnKey(provider)))].spacing(10).align_y(Alignment::Center).into()
            } else {
                let draft = &self.own_key_drafts[provider as usize];
                row![
                    field(&tr("Your own key", &[]), draft)
                        .secure(true)
                        .on_input(move |draft| Message::OwnKeyDraft(provider, draft))
                        .on_submit(Message::SaveOwnKey(provider))
                        .width(240),
                    pill(tr("Save", &[]), Fill::Sun, (!draft.trim().is_empty()).then_some(Message::SaveOwnKey(provider))),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
            };
            let link = button(text(tr("Get a key", &[])).font(semibold()).size(12).color(theme::KORE_PINK_TEXT))
                .padding(0)
                .style(|_, _| button::Style::default())
                .on_press(Message::OpenOwnKeyPage(provider));
            items.push(divider());
            items.push(
                container(
                    column![row![text(name).font(bold()).size(14).color(theme::INK).width(Length::Fill), entry, link].spacing(10).align_y(Alignment::Center)]
                        .push(key.refused.then(|| tag(refused, theme::KORE_PINK_TEXT)))
                        .spacing(6),
                )
                .padding(Padding::from([11, 16]))
                .into(),
            );
        }
        rows(items).into()
    }

    // MARK: Making a Character

    /// A Character the user made, as a Character card with Use or Resume, and Delete.
    pub(super) fn kk_made_card<'a>(&'a self, made: &'a making::MadeCharacter, job: Option<&'a making::Making>) -> Element<'a, Message> {
        let online = self.online();
        let busy = self.busy();
        let in_use = self.status.as_ref().and_then(|status| status.device.voice.as_deref()) == Some(made.id.as_str());
        let making_this = job.is_some_and(|job| job.running() && job.id == made.id);
        let job_running = job.is_some_and(making::Making::running);
        let action: Element<'_, Message> = if in_use {
            tag(tr("In use", &[]), theme::KORE_PINK_TEXT)
        } else if made.complete {
            pill(tr("Use", &[]), Fill::Paper, (online && !busy).then_some(Message::UseMade(made.id.clone()))).into()
        } else if making_this {
            tag(tr("Making…", &[]), theme::WAIT_TEXT)
        } else {
            pill(tr("Resume", &[]), Fill::Paper, (!job_running).then_some(Message::ResumeMade(made.id.clone()))).into()
        };
        let base = assets::voice_name(&made.base);
        let voice = self.character_voices.iter().find(|voice| voice.id == made.voice).map_or_else(|| made.voice.clone(), |voice| voice.name.clone());
        let detail = if made.complete { tr("Made from %@ · voice: %@", &[&base, &voice]) } else { tr("Not finished · made from %@ · voice: %@", &[&base, &voice]) };
        let face: Element<'_, Message> = match self.made_faces.iter().find(|(id, _)| *id == made.id) {
            Some((_, face)) => super::pages::figure(face.clone()).into(),
            None => Space::new().into(),
        };
        let delete: Element<'_, Message> = if self.confirm_delete_made.as_deref() == Some(made.id.as_str()) {
            column![
                note(tr("Delete “%@”? This can't be undone.", &[&made.name])),
                small(tr("Its lines made in its voice go with it; making it again would make them again.", &[])),
                row![
                    Space::new().width(Length::Fill),
                    pill(tr("Cancel", &[]), Fill::Paper, Some(Message::AskDeleteMade(None))),
                    pill(tr("Delete", &[]), Fill::Pink, Some(Message::DeleteMade(made.id.clone()))),
                ]
                .spacing(6),
            ]
            .spacing(6)
            .into()
        } else if in_use {
            small(tr("It's on the box: put another character on the box to delete this one.", &[]))
        } else {
            row![Space::new().width(Length::Fill), pill(tr("Delete…", &[]), Fill::Paper, (!making_this).then(|| Message::AskDeleteMade(Some(made.id.clone()))))].into()
        };
        let content = column![
            container(face).center_x(Length::Fill).align_y(Alignment::End).height(104).padding(Padding::ZERO.bottom(4)).style(|_| super::pages::stage(theme::BOX_CARD)),
            row![text(made.name.clone()).font(bold()).size(16).width(Length::Fill), action].align_y(Alignment::Center),
            small(detail),
            delete,
        ]
        .spacing(6);
        let (outline, stroke) = if in_use { (theme::KORE_PINK, theme::STROKE_BOLD) } else { (theme::INK, theme::STROKE_BASE) };
        card_with(content, theme::PAPER, outline, stroke).padding(9).into()
    }

    /// The Character being made, or the last one: its progress, why it stopped, or that it is ready for the box.
    pub(super) fn kk_making_row(&self, job: &making::Making) -> Option<Element<'_, Message>> {
        let made = self.made.iter().find(|made| made.id == job.id);
        if job.running() {
            return Some(
                card(
                    column![
                        row![heading(tr("Making “%@”…", &[&job.name])), Space::new().width(Length::Fill), tag(format!("{} / {}", job.done, job.total), theme::INK)],
                        bar(job.progress(), theme::SUN),
                    ]
                    .spacing(8),
                )
                .into(),
            );
        }
        let provider = made
            .and_then(|made| self.character_voices.iter().find(|voice| voice.id == made.voice))
            .map_or_else(|| tr("the voice's provider", &[]), making::Voice::provider_name);
        if let Some(why) = job.explanation(&provider) {
            let resume = made.map(|made| pill(tr("Resume", &[]), Fill::Sun, Some(Message::ResumeMade(made.id.clone()))));
            return Some(
                card_with(row![container(note(why)).width(Length::Fill)].push(resume).spacing(12).align_y(Alignment::Center), theme::CREAM, theme::KORE_PINK_TEXT, theme::STROKE_BASE)
                    .into(),
            );
        }
        let current = self.status.as_ref().and_then(|status| status.device.voice.as_deref());
        let made = made.filter(|made| made.complete && current != Some(made.id.as_str()))?;
        Some(
            card_with(
                row![
                    tag(tr("“%@” is made.", &[&made.name]), theme::DONE_TEXT),
                    Space::new().width(Length::Fill),
                    pill(tr("Write to box", &[]), Fill::Pink, (self.online() && !self.busy()).then_some(Message::UseMade(made.id.clone()))),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
                theme::PAPER,
                theme::DONE_TEXT,
                theme::STROKE_BASE,
            )
            .into(),
        )
    }

    /// One way in to a Character of the user's own: their drawings, their lines, a voice they pick.
    pub(super) fn kk_make_character(&self) -> Element<'_, Message> {
        super::pages::dashed(
            tr("Make your own", &[]),
            tr("Your drawings, your lines, a voice you pick", &[]),
            self.status.is_some().then_some(Message::MakeCharacter(true)),
        )
    }

    /// Making a Character of the user's own, in place of the Character page: start from a preset, bring a look, change
    /// its lines (occasions stay, each keeps a line) and pick a voice. With only a look nothing is made: the drawings
    /// go to the box with the preset lending its voice. With new lines or another voice, the daemon makes the audio.
    pub(super) fn kk_make_form<'a>(&'a self, form: &'a MakeForm) -> Element<'a, Message> {
        let bases: Vec<Lender> = self.voices.iter().map(Lender::of).collect();
        // In one order whatever the lines: the voice comes first, so the lines don't move it.
        let voices = self.character_voices.clone();
        let keys = self.status.as_ref().and_then(|status| status.voice_keys.as_ref());
        let account = self.status.as_ref().map(|status| status.account.clone()).unwrap_or_default();
        let readiness = form.voice.as_ref().map(|voice| (making::Readiness::of(voice, keys, &account), voice.provider_name()));
        let job_running = self.status.as_ref().and_then(|status| status.making.as_ref()).is_some_and(making::Making::running);
        let problem = form.problem.clone().or_else(|| if form.lines.is_empty() { None } else { making::problem(&form.name, &form.lines, &form.occasions) });
        let look_only = form.look_only();
        let problem = if look_only { form.problem.clone() } else { problem };
        let can_make = !form.starting && !form.lines.is_empty() && problem.is_none() && readiness.as_ref().is_some_and(|(ready, _)| ready.can_start()) && !job_running;
        let base_face = self.voices.iter().position(|voice| voice.id == form.base.id).and_then(|index| self.faces.get(index).cloned().flatten());
        let face = form.look.as_ref().map(|(_, face)| face.clone()).or(base_face);
        let readiness = readiness.filter(|_| !look_only).map(|(ready, provider)| {
            text(ready.explanation(&provider)).font(super::controls::regular()).size(13).color(if ready.can_start() { theme::MUTED } else { theme::KORE_PINK_TEXT })
        });
        // A voice of the user's own has no language of its own: it says the lines as they are written.
        let mixed = form.voice.as_ref().is_some_and(|voice| !voice.own && Some(voice.language.as_str()) != assets::language_of(form.base.id).map(assets::Language::code));
        let mixed = mixed.then(|| text(tr("This voice and the lines are in different languages, so the lines may sound wrong.", &[])).font(super::controls::regular()).size(13).color(theme::WAIT_TEXT));
        let busy = self.busy();
        let label = |words: String| text(words).font(bold()).size(14).color(theme::INK).width(120);
        let line = |content: Element<'a, Message>| -> Element<'a, Message> { container(content).padding(Padding::from([10, 14])).width(Length::Fill).into() };
        let buttons = row![
            pill(tr("Choose images…", &[]), Fill::Paper, Some(Message::MakeChooseDrawings)),
            pill(tr("Copy a prompt", &[]), Fill::Paper, Some(Message::CopyPrompt)),
        ]
        .push(form.look.is_some().then(|| pill(tr("Use %@'s look", &[&assets::voice_name(form.base.id)]), Fill::Paper, Some(Message::MakeBaseLook))))
        .spacing(8);
        let look = row![]
            .push(face.map(|face| container(image(face).filter_method(image::FilterMethod::Nearest)).padding(4).style(|_| super::pages::stage(theme::BOX_CARD))))
            .push(column![note(tr("One PNG or JPEG image: the figure four times in a 2 × 2 grid, on a plain white or transparent background. Top left normal, top right eyes closed, bottom left happy, bottom right sad. Any size: VibeBuddy cuts out each figure and makes it 48 × 64 pixels in 15 colors. No drawing yet? Copy a prompt into an image tool, such as ChatGPT or Gemini, with your character in place of the brackets.", &[])), buttons].spacing(10))
            .spacing(14);
        let basics = rows([line(
            row![label(tr("Name", &[])), field(&tr("Name", &[]), &form.name).on_input(Message::MakeName).width(280)].spacing(8).align_y(Alignment::Center).into(),
        )]);
        // A row per voice, so each has ▶: the preview of the preset it comes from, when this App has it.
        let mut voice_rows: Vec<Element<'a, Message>> = Vec::new();
        for (index, voice) in voices.into_iter().enumerate() {
            let chosen = form.voice.as_ref() == Some(&voice);
            let preview = making::preview_of(&voice.id);
            let preview = if preview == ROBOT { Some(ROBOT) } else { self.voices.iter().find(|shipped| shipped.id == preview).map(|shipped| shipped.id) };
            let playing = preview.is_some_and(|id| self.previewing.as_ref().is_some_and(|(playing, _)| playing == id));
            let dot = container(Space::new().width(10).height(10)).padding(1).style(move |_| container::Style {
                background: Some(if chosen { theme::KORE_PINK } else { theme::PAPER }.into()),
                border: Border { color: theme::INK, width: theme::STROKE_BASE, radius: 8.0.into() },
                ..container::Style::default()
            });
            let provider = voice.provider_name();
            let made = (voice.id == form.base.id && form.lines == form.original).then(|| tag(tr("Free · already made", &[]), theme::DONE_TEXT));
            let yours = voice.own.then(|| tag(tr("Yours", &[]), theme::KORE_PINK_TEXT));
            let remove = voice.own.then(|| pill(tr("Remove", &[]), Fill::Paper, Some(Message::RemoveOwnVoice(voice.id.clone()))));
            let pick = button(
                row![dot, text(voice.name.clone()).font(bold()).size(14).color(theme::INK), tag(provider, theme::MUTED_2)]
                    .push(yours)
                    .push(made)
                    .spacing(8)
                    .align_y(Alignment::Center),
            )
            .padding(0)
            .style(|_, _| button::Style::default())
            .on_press(Message::MakeVoiceAndLines(voice));
            if index > 0 {
                voice_rows.push(divider());
            }
            voice_rows.push(line(
                row![pick, Space::new().width(Length::Fill)]
                    .push(remove)
                    .push(preview.map(|id| pill(if playing { "■".to_owned() } else { "▶".to_owned() }, Fill::Paper, (!busy).then_some(Message::TogglePreview(id)))))
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .into(),
            ));
        }
        voice_rows.push(divider());
        voice_rows.push(line(self.kk_own_voice_form(form.own.as_ref())));
        let mut page = column![
            text(tr("Make your own", &[])).font(bold()).size(18).color(theme::INK),
            note(tr("Bring your own drawings, pick a voice and say what it says. A character's own voice with its lines as they are is free: they are already made. Any other voice, or lines you change, VibeBuddy makes in that voice, which needs the Paid plan or your own key.", &[])),
            basics,
            heading(tr("Look", &[])),
            rows([line(look.into())]),
            heading(tr("Voice", &[])),
            rows(voice_rows),
        ]
        .push(mixed)
        .push(readiness)
        .push(heading(tr("Lines", &[])))
        .push(rows([line(row![label(tr("Lines from", &[])), picker(bases, Some(form.base), Message::MakeBase)].spacing(8).align_y(Alignment::Center).into())]))
        .spacing(14);
        for occasion in &form.occasions {
            let mut items: Vec<Element<'a, Message>> = Vec::new();
            for (index, entry) in form.lines.iter().enumerate().filter(|(_, entry)| &entry.occasion == occasion) {
                if !items.is_empty() {
                    items.push(divider());
                }
                items.push(line(
                    row![
                        field("", &entry.text).on_input(move |text| Message::MakeLine(index, text)),
                        pill("−".to_owned(), Fill::Paper, making::can_remove(index, &form.lines).then_some(Message::MakeRemoveLine(index))),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .into(),
                ));
            }
            page = page.push(heading(making::occasion_name(occasion))).push(rows(items));
        }
        page = page
            .push(note(tr("{address} is where the buddy says what it calls you; each line with it is made once with each form of address.", &[])))
            .push(problem.map(|problem| tag(problem, theme::KORE_PINK_TEXT)))
            .push(
                row![
                    Space::new().width(Length::Fill),
                    pill(tr("Cancel", &[]), Fill::Paper, Some(Message::MakeCharacter(false))),
                    if look_only {
                        pill(tr("Use this look", &[]), Fill::Pink, (self.online() && !busy).then_some(Message::MakeUseLook))
                    } else {
                        pill(tr("Make", &[]), Fill::Sun, can_make.then_some(Message::MakeStart))
                    },
                ]
                .spacing(8),
            );
        super::pages::page(page)
    }

    /// The last voice row: a voice cloned or designed in the user's own account, tried with their own key on the first
    /// line before it is kept.
    fn kk_own_voice_form<'a>(&'a self, own: Option<&'a crate::OwnVoiceForm>) -> Element<'a, Message> {
        let Some(own) = own else { return pill(tr("Your own voice…", &[]), Fill::Paper, Some(Message::MakeOwnOpen(true))).into() };
        let keys = self.status.as_ref().and_then(|status| status.voice_keys.as_ref());
        let has_key = keys.is_some_and(|keys| if own.provider == crate::OwnProvider::Volc { keys.volc.set } else { keys.elevenlabs.set });
        let id_label = if own.provider == crate::OwnProvider::Volc { tr("Speaker ID", &[]) } else { tr("Voice ID", &[]) };
        let ready = has_key && !own.trying && !own.voice.trim().is_empty() && !own.name.trim().is_empty();
        column![
            note(tr("A voice you cloned or designed in your own ElevenLabs or Volcano Engine account. Paste its ID: VibeBuddy tries it with your own key by saying the first line, and keeps it for next time.", &[])),
            row![
                picker(vec![crate::OwnProvider::ElevenLabs, crate::OwnProvider::Volc], Some(own.provider), Message::MakeOwnProvider),
                match (own.provider, &own.listed) {
                    (crate::OwnProvider::ElevenLabs, Some(Ok(voices))) if !voices.is_empty() => {
                        let picked = voices.iter().find(|voice| voice.voice == own.voice).cloned();
                        picker(voices.clone(), picked, Message::MakeOwnPick)
                    }
                    _ => field(&id_label, &own.voice).on_input(Message::MakeOwnVoice).into(),
                },
                field(&tr("Name", &[]), &own.name).on_input(Message::MakeOwnName).width(160),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .push(match (own.provider, &own.listed) {
            (crate::OwnProvider::ElevenLabs, Some(Err(why))) if has_key => Some(
                text(tr("Couldn't list the voices of your ElevenLabs key, so paste a voice id: %@", &[why])).font(super::controls::regular()).size(13).color(theme::WAIT_TEXT),
            ),
            _ => None,
        })
        .push((!has_key).then(|| text(making::Readiness::OwnKeyOnly.explanation(&own.provider.to_string())).font(super::controls::regular()).size(13).color(theme::KORE_PINK_TEXT)))
        .push(own.problem.clone().map(|problem| tag(problem, theme::KORE_PINK_TEXT)))
        .push(
            row![
                Space::new().width(Length::Fill),
                pill(tr("Cancel", &[]), Fill::Paper, Some(Message::MakeOwnOpen(false))),
                pill(if own.trying { tr("Trying…", &[]) } else { tr("Try", &[]) }, Fill::Sun, ready.then_some(Message::MakeOwnTry)),
            ]
            .spacing(8),
        )
        .spacing(10)
        .into()
    }
}
