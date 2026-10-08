//! `vibebuddy-desktop`: Vibe Buddy's tray icon and settings window outside macOS, the counterpart of the Mac
//! app's menu bar icon and settings window. It is only a client of `vibebuddyd`: systemd supervises the
//! daemon here, so quitting the app leaves the box online.

// The tray, and the face it draws, exist only on Linux; other builds are for working on the window.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod assets;
mod character;
mod client;
mod custom;
mod face;
mod i18n;
#[cfg(target_os = "linux")]
mod instance;
mod status;
mod theme;
#[cfg(target_os = "linux")]
mod tray;

use std::time::{Duration, SystemTime};

use iced::widget::{button, checkbox, column, container, image, pick_list, row, scrollable, slider, space, text, toggler};
use iced::{Element, Font, Length, Subscription, Task, Theme, window};

use i18n::{UiLanguage, tr};
use status::{Config, MenuState, Status};

/// Omarchy's monospace font, used when Omarchy's theme is: the window should look like the rest of the desktop.
const OMARCHY_FONT: &str = "JetBrainsMono Nerd Font";

/// Passed to the copy a language change starts: it waits for this one to quit and opens Settings where the user was.
const RESTARTED: &str = "--restarted";
/// Followed by a voice id: written once the restarted copy sees the box answer, then forgotten.
const WRITE_VOICE: &str = "--write-voice";

/// The running copy's socket, handed to the subscription that listens on it.
#[cfg(target_os = "linux")]
static INSTANCE: std::sync::Mutex<Option<std::os::unix::net::UnixListener>> = std::sync::Mutex::new(None);

fn main() -> iced::Result {
    #[cfg(target_os = "linux")]
    match instance::claim(std::env::args().any(|arg| arg == RESTARTED)) {
        instance::Claim::First(listener) => *INSTANCE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(listener),
        instance::Claim::HandedOff => return Ok(()),
        instance::Claim::Unavailable => {}
    }
    let mut app = iced::daemon(App::boot, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription);
    if theme::stamp().is_some() {
        app = app.default_font(Font::with_name(OMARCHY_FONT));
    }
    app.run()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    General,
    Sound,
    Agents,
    Device,
    Advanced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Agent {
    Claude,
    Codex,
}

#[derive(Clone, Debug)]
enum Message {
    Daemon(client::Update),
    #[cfg(target_os = "linux")]
    TrayReady(TrayHandle),
    /// No tray host on this desktop: the window is the only way in, so it opens.
    TrayUnavailable,
    /// The shell's tray host went away or came back; if it stays away, the window opens instead.
    TrayHost(bool),
    TrayHostCheck,
    ThemeTick,
    OpenSettings,
    WindowClosed(window::Id),
    Tab(Tab),
    NotifyLink(bool),
    CheckUpdates(bool),
    CheckForUpdatesNow,
    OpenReleases,
    ReportProblem,
    PickLanguage(UiLanguage),
    SwitchVoice(bool),
    /// Restart now (true) or later (false) to apply the picked language.
    RestartForLanguage(bool),
    ConfigSaved(Result<Config, String>),
    VolumeDragged(u8),
    VolumeReleased,
    PlayLine,
    /// The user says which of several devices is the box, by its USB serial number.
    ChooseBox(String),
    Identify,
    UseVoice(&'static str),
    /// A form of address for a language, or none.
    PickAddress(assets::Language, Option<&'static str>),
    ChooseDrawings,
    Drawings(Result<Option<Vec<character::Image>>, String>),
    PickLender(Lender),
    UseRobot,
    UseCustom,
    CopyPrompt,
    AskFirmwareUpdate(bool),
    FlashFirmware,
    TakeScreenshot,
    Screenshot(Result<Vec<u8>, String>),
    SaveScreenshot,
    RestartDaemon,
    Hooks(&'static str),
    /// A finished action whose outcome is worth a line at the bottom of the window.
    Notice(Result<String, String>),
    OpenLogsFolder,
    ShowDaemonLog,
    ExportDiagnostics,
    Done(Result<(), String>),
    Quit,
}

#[cfg(target_os = "linux")]
#[derive(Clone)]
struct TrayHandle(ksni::Handle<tray::Tray>);

#[cfg(target_os = "linux")]
impl std::fmt::Debug for TrayHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TrayHandle")
    }
}

struct App {
    /// `None` while the daemon isn't answering.
    status: Option<Status>,
    theme: Theme,
    theme_stamp: Option<SystemTime>,
    settings: Option<window::Id>,
    tab: Tab,
    /// The slider's position while it is being dragged; the box's own value otherwise.
    volume: Option<u8>,
    /// The last screen grabbed from the box, as PNG, and whether a grab is under way.
    screenshot: Option<(Vec<u8>, image::Handle)>,
    screenshot_busy: bool,
    /// Installed alongside the app; read once, since only reinstalling changes them.
    voices: Vec<assets::Voice>,
    /// Each installed Character's face, ready to draw.
    faces: Vec<Option<image::Handle>>,
    /// The forms of address picked and the user's own Character as last written.
    characters: custom::Settings,
    /// The user's own Character on the card: its look, its four frames to show, and who lends it voice and lines.
    custom_look: Option<(Vec<u8>, Vec<image::Handle>)>,
    lender: Option<Lender>,
    custom_problem: Option<String>,
    /// The robot's face for its card.
    robot_face: Option<image::Handle>,
    /// The firmware update waits for a second click, since the box restarts.
    confirm_firmware: bool,
    /// The outcome of the last action, shown at the bottom of the window.
    notice: Option<Result<String, String>>,
    /// The language picked in Settings, and, while it differs from the UI's, the offer to restart.
    language: UiLanguage,
    restart_offer: Option<RestartOffer>,
    /// A voice to write once the box answers, asked for when the language changed.
    pending_voice: Option<&'static str>,
    /// When the box's port was last seen open, to tell a box that never reports a build.
    /// Whether the shell has a tray host for the icon right now.
    tray_host: bool,
    #[cfg(target_os = "linux")]
    tray: Option<TrayHandle>,
}

struct RestartOffer {
    /// A voice in the new language for the box, when it speaks the other one; ticked by default.
    voice: Option<assets::Voice>,
    switch_voice: bool,
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let app = Self {
            status: None,
            theme: theme::load(),
            theme_stamp: theme::stamp(),
            settings: None,
            tab: Tab::General,
            volume: None,
            screenshot: None,
            screenshot_busy: false,
            voices: Vec::new(),
            faces: Vec::new(),
            characters: custom::Settings::load(),
            custom_look: None,
            lender: None,
            custom_problem: None,
            robot_face: None,
            confirm_firmware: false,
            notice: None,
            language: UiLanguage::saved(),
            restart_offer: None,
            pending_voice: launch_voice(),
            tray_host: true,
            #[cfg(target_os = "linux")]
            tray: None,
        };
        // The first launch shows the window, so it's clear where the app went; later ones stay in the tray, unless a
        // language change restarted the app from its window. Without a tray (macOS builds, for development) the
        // window is the only way in.
        let mut app = app;
        app.voices = assets::voices();
        app.faces = app.voices.iter().map(|voice| voice.face.as_ref().map(|face| handle(face, 1))).collect();
        app.lender = app
            .characters
            .custom_lender
            .as_deref()
            .and_then(|id| app.voices.iter().find(|voice| voice.id == id))
            .or(app.voices.first())
            .map(Lender::of);
        app.robot_face = ::image::load_from_memory(include_bytes!("../../characters/robot/face.png")).ok().map(|face| {
            let face = face.to_rgba8();
            image::Handle::from_rgba(face.width(), face.height(), face.into_raw())
        });
        app.custom_look = custom::custom_look_file()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|look| character::look_frames(&look).map(|frames| (look, frames.iter().map(|frame| handle(frame, 2)).collect())));
        let restarted = std::env::args().any(|arg| arg == RESTARTED);
        let open = first_launch() || restarted || !cfg!(target_os = "linux");
        let task = if open { Task::done(Message::OpenSettings) } else { Task::none() };
        (app, task)
    }

    fn title(&self, _window: window::Id) -> String {
        "Vibe Buddy".to_owned()
    }

    fn theme(&self, _window: window::Id) -> Theme {
        self.theme.clone()
    }

    fn subscription(&self) -> Subscription<Message> {
        #[cfg(target_os = "linux")]
        let (tray, instance) = (Subscription::run(run_tray), Subscription::run(run_instance));
        #[cfg(not(target_os = "linux"))]
        let (tray, instance) = (Subscription::none(), Subscription::none());
        Subscription::batch([
            Subscription::run(client::status_updates).map(Message::Daemon),
            iced::time::every(Duration::from_secs(2)).map(|_| Message::ThemeTick),
            window::close_events().map(Message::WindowClosed),
            tray,
            instance,
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Daemon(client::Update::Status(status)) => {
                let device = &status.device;
                let busy = status.operation.as_ref().is_some_and(status::Operation::running);
                // Wait for the build: the box has answered, not just had its port opened (which resets it).
                let voice = (device.connected && device.firmware_build.is_some() && !busy)
                    .then(|| self.pending_voice.take())
                    .flatten();
                self.status = Some(*status);
                return Task::batch([self.refresh_tray(), voice.map_or_else(Task::none, |id| Task::done(Message::UseVoice(id)))]);
            }
            Message::Daemon(client::Update::Down) => {
                self.status = None;
                return self.refresh_tray();
            }
            #[cfg(target_os = "linux")]
            Message::TrayReady(handle) => {
                self.tray = Some(handle);
                let pin = state_dir().map_or_else(Task::none, |dir| {
                    Task::future(tray::pin_in_omarchy_bar(dir.join("tray-pinned"))).discard()
                });
                return Task::batch([self.refresh_tray(), pin]);
            }
            Message::TrayUnavailable => return Task::done(Message::OpenSettings),
            Message::TrayHost(up) => {
                self.tray_host = up;
                // A shell restarting is back within a second or two; one that isn't up after this, isn't coming.
                if !up {
                    return Task::perform(tokio::time::sleep(Duration::from_secs(10)), |()| Message::TrayHostCheck);
                }
            }
            Message::TrayHostCheck => {
                if !self.tray_host {
                    return Task::done(Message::OpenSettings);
                }
            }
            Message::ThemeTick => {
                let stamp = theme::stamp();
                if stamp != self.theme_stamp {
                    self.theme_stamp = stamp;
                    self.theme = theme::load();
                    return self.refresh_tray();
                }
            }
            Message::OpenSettings => {
                if let Some(id) = self.settings {
                    return window::gain_focus(id);
                }
                // The same as the Mac's: room for the whole Character tab, resizable down to the old size.
                let (id, open) = window::open(window::Settings {
                    size: iced::Size::new(760.0, 640.0),
                    min_size: Some(iced::Size::new(640.0, 480.0)),
                    #[cfg(target_os = "linux")]
                    platform_specific: window::settings::PlatformSpecific {
                        application_id: "vibebuddy".to_owned(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
                self.settings = Some(id);
                return open.discard();
            }
            Message::WindowClosed(id) => {
                if self.settings == Some(id) {
                    self.settings = None;
                    self.notice = None;
                }
            }
            Message::Tab(tab) => self.tab = tab,
            Message::NotifyLink(enabled) => {
                let mut config = self.status.as_ref().map(|status| status.config.clone()).unwrap_or_default();
                config.notify_link = enabled;
                return Task::perform(client::put_config(config), Message::ConfigSaved);
            }
            Message::CheckUpdates(enabled) => {
                let mut config = self.status.as_ref().map(|status| status.config.clone()).unwrap_or_default();
                config.check_updates = Some(enabled);
                return Task::perform(client::put_config(config), Message::ConfigSaved);
            }
            Message::CheckForUpdatesNow => return Task::perform(client::check_for_updates(), Message::Done),
            Message::OpenReleases => {
                if let Err(error) = launch("xdg-open", &[std::ffi::OsStr::new(RELEASES_PAGE)]) {
                    self.notice = Some(Err(error));
                }
            }
            Message::ReportProblem => {
                let url = issue_url(&self.diagnostics_summary());
                if let Err(error) = launch("xdg-open", &[std::ffi::OsStr::new(url.as_str())]) {
                    self.notice = Some(Err(error));
                }
            }
            Message::PickLanguage(choice) => {
                if choice == self.language {
                    return Task::none();
                }
                if let Err(error) = choice.save() {
                    self.notice = Some(Err(error));
                    return Task::none();
                }
                self.language = choice;
                self.restart_offer = (choice.chinese() != i18n::is_chinese()).then(|| {
                    let target = if choice.chinese() { assets::Language::Chinese } else { assets::Language::English };
                    let device = self.status.as_ref().map(|status| &status.device);
                    // Only a box that has answered (build and voice reported) can take a voice; an unknown one isn't guessed at.
                    let voice = device
                        .filter(|device| device.connected && device.firmware_build.is_some())
                        .and_then(|device| device.voice.as_deref())
                        .and_then(|voice| assets::voice_switch(voice, target));
                    RestartOffer { voice, switch_voice: true }
                });
            }
            Message::SwitchVoice(on) => {
                if let Some(offer) = &mut self.restart_offer {
                    offer.switch_voice = on;
                }
            }
            Message::RestartForLanguage(now) => {
                let Some(offer) = self.restart_offer.take() else { return Task::none() };
                let voice = offer.voice.filter(|_| offer.switch_voice).map(|voice| voice.id);
                if now {
                    match restart(voice) {
                        Ok(()) => return iced::exit(),
                        Err(error) => self.notice = Some(Err(error)),
                    }
                } else if let Some(id) = voice {
                    return Task::done(Message::UseVoice(id));
                }
            }
            Message::ConfigSaved(result) => {
                match result {
                    Ok(config) => {
                        if let Some(status) = &mut self.status {
                            status.config = config;
                        }
                    }
                    Err(error) => self.notice = Some(Err(error)),
                }
            }
            Message::VolumeDragged(level) => self.volume = Some(level),
            Message::VolumeReleased => {
                if let Some(level) = self.volume {
                    return Task::perform(client::set_volume(level, false), Message::Done);
                }
            }
            Message::PlayLine => {
                let level = self.volume.or_else(|| self.status.as_ref()?.device.volume).unwrap_or(60);
                return Task::perform(client::set_volume(level, true), Message::Done);
            }
            Message::ChooseBox(usb_serial) => return Task::perform(client::choose_box(usb_serial), Message::Done),
            Message::Identify => return Task::perform(client::identify(), Message::Done),
            Message::UseVoice(id) => {
                let form = assets::language_of(id).and_then(|language| self.characters.address(language)).map(str::to_owned);
                let write = async move {
                    let pack = assets::read_character(id, form.as_deref()).await?;
                    client::write_voice_pack(pack.build().ok_or("the Character pack doesn't fit")?).await
                };
                return Task::perform(write, Message::Done);
            }
            Message::PickAddress(language, form) => {
                self.characters.set_address(language, form);
                self.characters.save();
                // The box says it at once when it wears a Character of that language.
                let current = self.status.as_ref().and_then(|status| status.device.voice.clone());
                if let Some(id) = current.as_deref().and_then(assets::voice_id).filter(|&id| assets::language_of(id) == Some(language)) {
                    return Task::done(Message::UseVoice(id));
                }
                let lends = self.lender.is_some_and(|lender| assets::language_of(lender.id) == Some(language));
                if current.as_deref() == Some(CUSTOM) && lends && self.custom_look.is_some() {
                    return Task::done(Message::UseCustom);
                }
            }
            Message::ChooseDrawings => return Task::perform(custom::choose_drawings(), Message::Drawings),
            Message::Drawings(Ok(None)) => {}
            Message::Drawings(Ok(Some(drawings))) => match character::build_look(&drawings) {
                Some(look) => {
                    let frames = character::look_frames(&look).unwrap_or_default();
                    self.custom_look = Some((look, frames.iter().map(|frame| handle(frame, 2)).collect()));
                    self.custom_problem = None;
                }
                None => self.custom_problem = Some(tr("No figure found: use a plain white or transparent background.", &[])),
            },
            Message::Drawings(Err(error)) => self.custom_problem = Some(error),
            Message::PickLender(lender) => self.lender = Some(lender),
            Message::UseRobot => {
                self.characters.robot_lender = None;
                self.characters.save();
                let write = async move { client::write_voice_pack(assets::read_pack(ROBOT).await?).await };
                return Task::perform(write, Message::Done);
            }
            Message::UseCustom => {
                let (Some((look, _)), Some(lender)) = (self.custom_look.clone(), self.lender) else { return Task::none() };
                if let Some(path) = custom::custom_look_file() {
                    custom::write(&path, &look);
                }
                self.characters.custom_lender = Some(lender.id.to_owned());
                self.characters.save();
                let form = assets::language_of(lender.id).and_then(|language| self.characters.address(language)).map(str::to_owned);
                let write = async move {
                    let pack = assets::read_character(lender.id, form.as_deref()).await?.with_look(look, CUSTOM);
                    client::write_voice_pack(pack.build().ok_or("that character can't lend its voice")?).await
                };
                return Task::perform(write, Message::Done);
            }
            Message::CopyPrompt => return iced::clipboard::write(CUSTOM_PROMPT.to_owned()),
            Message::AskFirmwareUpdate(asking) => self.confirm_firmware = asking,
            Message::FlashFirmware => {
                self.confirm_firmware = false;
                if let Some(directory) = self.updates().and_then(status::Updates::firmware_directory) {
                    return Task::perform(client::flash_firmware(directory.to_path_buf()), Message::Done);
                }
            }
            Message::TakeScreenshot => {
                self.screenshot_busy = true;
                return Task::perform(client::screenshot(), Message::Screenshot);
            }
            Message::Screenshot(result) => {
                self.screenshot_busy = false;
                match result {
                    Ok(png) => self.screenshot = Some((png.clone(), image::Handle::from_bytes(png))),
                    Err(error) => self.notice = Some(Err(error)),
                }
            }
            Message::SaveScreenshot => {
                if let Some((png, _)) = &self.screenshot {
                    self.notice = Some(save_screenshot(png));
                }
            }
            Message::RestartDaemon => return Task::perform(client::restart_daemon(), Message::Done),
            Message::Hooks(action) => return Task::perform(run_hook_tool(action), Message::Notice),
            Message::Notice(result) => self.notice = Some(result),
            Message::OpenLogsFolder => {
                if let Err(error) = state_dir().ok_or("HOME is not set".to_owned()).and_then(|dir| launch("xdg-open", &[dir.as_os_str()])) {
                    self.notice = Some(Err(error));
                }
            }
            Message::ShowDaemonLog => {
                let args = ["journalctl", "--user", "-u", "vibebuddyd", "-f"].map(std::ffi::OsStr::new);
                if let Err(error) = launch("xdg-terminal-exec", &args) {
                    self.notice = Some(Err(error));
                }
            }
            Message::ExportDiagnostics => {
                let summary = self.diagnostics_summary();
                let config = self.status.as_ref().map(|status| status.config.clone());
                return Task::perform(export_diagnostics(summary, config), Message::Notice);
            }
            Message::Done(result) => {
                // The box answers a volume change through the status stream; drop the dragged value then.
                self.volume = None;
                if let Err(error) = result {
                    self.notice = Some(Err(error));
                }
            }
            Message::Quit => return iced::exit(),
        }
        Task::none()
    }

    /// Pushes the menu and the face's color to the tray; a no-op until the tray is up.
    fn refresh_tray(&self) -> Task<Message> {
        #[cfg(target_os = "linux")]
        if let Some(TrayHandle(handle)) = self.tray.clone() {
            let menu = MenuState::derive(self.status.as_ref());
            let [r, g, b, _] = self.theme.palette().text.into_rgba8();
            return Task::future(async move {
                handle
                    .update(move |tray| {
                        tray.menu = menu;
                        tray.color = [r, g, b];
                    })
                    .await;
            })
            .discard();
        }
        Task::none()
    }

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let tabs = [
            (Tab::General, tr("General", &[])),
            (Tab::Sound, tr("Character", &[])),
            (Tab::Agents, tr("Agents", &[])),
            (Tab::Device, tr("Device", &[])),
            (Tab::Advanced, tr("Advanced", &[])),
        ];
        let tab_bar = row(tabs.into_iter().map(|(tab, label)| {
            let style = if tab == self.tab { button::primary } else { button::text };
            button(text(label)).style(style).on_press(Message::Tab(tab)).into()
        }))
        .spacing(4);
        let body = match self.tab {
            Tab::General => self.general(),
            Tab::Sound => self.sound(),
            Tab::Agents => self.agents(),
            Tab::Device => self.device(),
            Tab::Advanced => self.advanced(),
        };
        let notice = self.notice.as_ref().map(|notice| match notice {
            Ok(message) => text(message.clone()).size(13),
            Err(error) => text(error.clone()).size(13).style(text::danger),
        });
        let content = column![tab_bar, container(body).height(Length::Fill)]
            .push(notice)
            .spacing(16)
            .padding(20);
        container(content).width(Length::Fill).height(Length::Fill).into()
    }

    fn updates(&self) -> Option<&status::Updates> {
        self.status.as_ref()?.updates.as_ref()
    }

    fn general(&self) -> Element<'_, Message> {
        let notify = self.status.as_ref().is_none_or(|status| status.config.notify_link);
        let menu = MenuState::derive(self.status.as_ref());
        let updates = self.updates();
        let enabled = updates.is_some_and(|updates| updates.enabled);
        let check = self
            .status
            .as_ref()
            .and_then(|status| status.config.check_updates)
            .unwrap_or(enabled);
        let summary = match updates {
            Some(updates) if updates.enabled => match (&updates.error, &updates.app, &updates.last_check) {
                (Some(error), _, _) => tr("Last check failed: %@", &[error]),
                (None, Some(app), _) => tr("Vibe Buddy %@ is available", &[&app.version]),
                (None, None, Some(_)) => tr("Up to date", &[]),
                (None, None, None) => tr("Not checked yet", &[]),
            },
            _ => tr("Off", &[]),
        };
        // No self-update here: a new release is installed by rerunning its install.sh, as the README says.
        let upgrade = updates.and_then(|updates| updates.app.as_ref()).map(|_| {
            text(tr("To upgrade, download the new release and run its install.sh again.", &[])).size(13)
        });
        let unsupported = updates.filter(|updates| updates.unsupported_app).map(|_| {
            text(tr("This version of Vibe Buddy is no longer supported. Update it to keep getting firmware for the box.", &[]))
                .style(text::warning)
        });
        column![]
            .push(unsupported)
            .push(text(menu.device_line))
            .push(text(menu.mode_line))
            .push(text(menu.today_line))
            .push(space().height(8))
            .push(
                toggler(notify)
                    .label(tr("Notify me when the box disconnects or the daemon fails", &[]))
                    .on_toggle_maybe(self.status.is_some().then_some(Message::NotifyLink)),
            )
            .push(
                toggler(check)
                    .label(tr("Check for updates", &[]))
                    .on_toggle_maybe(self.status.is_some().then_some(Message::CheckUpdates)),
            )
            .push(
                row![
                    text(summary).size(13),
                    space::horizontal(),
                    button(text(tr("Check now", &[]))).on_press_maybe(enabled.then_some(Message::CheckForUpdatesNow)),
                    button(text(tr("Releases", &[]))).style(button::secondary).on_press(Message::OpenReleases),
                    button(text(tr("Report a Problem…", &[]))).style(button::secondary).on_press(Message::ReportProblem),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            )
            .push(upgrade)
            .push(space().height(8))
            .push(
                row![text(tr("Language", &[])), pick_list(UiLanguage::ALL, Some(self.language), Message::PickLanguage)]
                    .spacing(12)
                    .align_y(iced::Alignment::Center),
            )
            .push(self.restart_offer.as_ref().map(|offer| self.restart_prompt(offer)))
        .spacing(8)
        .into()
    }

    fn restart_prompt(&self, offer: &RestartOffer) -> Element<'_, Message> {
        let device = self.status.as_ref().map(|status| &status.device);
        let voice: Option<Element<'_, Message>> = match &offer.voice {
            Some(voice) => {
                let label = if device.is_some_and(|device| device.bridge) {
                    tr("Also switch the box's voice to %@ (a few minutes over the UART bridge; keep it plugged in)", &[&voice.name])
                } else {
                    tr("Also switch the box's voice to %@", &[&voice.name])
                };
                Some(checkbox(offer.switch_voice).label(label).on_toggle(Message::SwitchVoice).into())
            }
            None if !device.is_some_and(|device| device.connected) => Some(
                text(tr("The box isn't connected, so its character stays as it is. You can change it later on the Character tab.", &[]))
                    .size(13)
                    .into(),
            ),
            None => None,
        };
        column![text(tr("Restart Vibe Buddy to change the language?", &[]))]
            .push(voice)
            .push(
                row![
                    button(text(tr("Restart now", &[]))).on_press(Message::RestartForLanguage(true)),
                    button(text(tr("Later", &[]))).style(button::secondary).on_press(Message::RestartForLanguage(false)),
                ]
                .spacing(8),
            )
            .spacing(8)
            .into()
    }

    fn sound(&self) -> Element<'_, Message> {
        let box_volume = self.status.as_ref().and_then(|status| status.device.volume);
        let level = self.volume.or(box_volume).unwrap_or(60);
        let online = self.status.as_ref().is_some_and(|status| status.device.connected);
        let mut volume = slider(20..=100, level, Message::VolumeDragged).step(5u8);
        if online {
            volume = volume.on_release(Message::VolumeReleased);
        }
        let operation = self.status.as_ref().and_then(|status| status.operation.as_ref());
        let busy = operation.is_some_and(status::Operation::running);
        let current = self.status.as_ref().and_then(|status| status.device.voice.clone());
        let cards = self.voices.iter().zip(&self.faces).map(|(voice, face)| {
            let in_use = current.as_deref() == Some(voice.id);
            let action: Element<'_, Message> = if in_use {
                text(tr("In use", &[])).style(text::success).into()
            } else {
                button(text(tr("Use", &[])))
                    .on_press_maybe((online && !busy).then_some(Message::UseVoice(voice.id)))
                    .into()
            };
            let card = row![]
                .push(face.clone().map(image))
                .push(column![
                    text(voice.name.clone()),
                    text(voice.tag.clone()).size(13),
                    text(voice.summary.clone()).size(13),
                ])
                .push(space::horizontal())
                .push(action)
                .spacing(12)
                .align_y(iced::Alignment::Center);
            character_card(card.into(), in_use)
        });
        let address = row![text(tr("What the buddy calls you", &[]))]
            .extend(assets::Language::ALL.map(|language| {
                let options: Vec<AddressChoice> = std::iter::once(AddressChoice { form: None, language })
                    .chain(assets::FORMS_OF_ADDRESS.iter().filter(|form| form.language == language).map(|form| AddressChoice { form: Some(form.id), language }))
                    .collect();
                let picked = options.iter().copied().find(|choice| choice.form == self.characters.address(language));
                let label = match language {
                    assets::Language::Chinese => tr("Chinese", &[]),
                    assets::Language::English => tr("English", &[]),
                };
                row![text(label), pick_list(options, picked, move |choice: AddressChoice| Message::PickAddress(language, choice.form))]
                    .spacing(6)
                    .align_y(iced::Alignment::Center)
                    .into()
            }))
            .spacing(16)
            .align_y(iced::Alignment::Center);
        let custom_in_use = current.as_deref() == Some(CUSTOM);
        let wears_robot = matches!(current.as_deref(), Some(ROBOT | BUILTIN));
        // A box an older version wrote with another Character's voice on the robot still needs writing again.
        let robot_in_use = wears_robot && self.characters.robot_lender.is_none();
        let robot_action: Element<'_, Message> = if robot_in_use {
            text(tr("In use", &[])).style(text::success).into()
        } else {
            button(text(tr("Use", &[]))).on_press_maybe((online && !busy).then_some(Message::UseRobot)).into()
        };
        let robot_card = row![]
            .push(self.robot_face.clone().map(image))
            .push(column![
                row![text("Vibe Buddy"), text(tr("Default", &[])).size(13).style(text::primary)]
                    .spacing(6)
                    .align_y(iced::Alignment::Center),
                text(tr("The original robot, drawn by the box itself", &[])).size(13),
            ]
            .spacing(4))
            .push(space::horizontal())
            .push(robot_action)
            .spacing(12)
            .align_y(iced::Alignment::Center);
        let lenders: Vec<Lender> = self.voices.iter().map(Lender::of).collect();
        let custom_card = column![
            row![text(tr("Your own character", &[]))]
                .push(space::horizontal())
                .push(custom_in_use.then(|| text(tr("In use", &[])).style(text::success))),
            text(tr(
                "Draw a figure with any image tool, on a plain white background: one image, or four in the order normal, eyes closed, happy, sad. The box shows it in place of the robot, with the voice and lines of the character you pick.",
                &[]
            ))
            .size(13),
            row![
                button(text(tr("Choose images…", &[]))).on_press(Message::ChooseDrawings),
                button(text(tr("Copy a prompt", &[]))).style(button::secondary).on_press(Message::CopyPrompt),
            ]
            .spacing(8),
        ]
        .push(self.custom_problem.as_ref().map(|problem| text(problem.clone()).size(13).style(text::danger)))
        .push(self.custom_look.as_ref().map(|(_, frames)| {
            column![
                row(frames.iter().map(|frame| container(image(frame.clone())).style(|_: &Theme| container::Style {
                    background: Some(iced::Color::BLACK.into()),
                    ..container::Style::default()
                }).into()))
                .spacing(6),
                row![
                    text(tr("Voice and lines from", &[])),
                    pick_list(lenders, self.lender, Message::PickLender),
                    button(text(tr("Use", &[]))).on_press_maybe((online && !busy && self.lender.is_some()).then_some(Message::UseCustom)),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            ]
            .spacing(8)
        }))
        .spacing(8);
        let progress = operation.filter(|operation| operation.kind == status::OperationKind::VoicePack).map(|operation| {
            let failed = operation.state == status::OperationState::Failed;
            column![text(operation.summary()).size(13)].push(failed.then(|| {
                text(tr("Didn't finish, so the box keeps its built-in voice. Reconnect the cable and click Use again.", &[]))
                    .size(13)
            }))
        });
        // No pack at all is the robot too, with the lines it shipped with.
        let using = assets::voice_name(current.as_deref().filter(|&id| id != BUILTIN).unwrap_or(ROBOT));
        let page = column![
            row![text(tr("Volume", &[])), volume, text(level.to_string())].spacing(12),
            button(text(tr("Play a line on the box", &[]))).on_press_maybe(online.then_some(Message::PlayLine)),
            text(tr("Saved on the box and kept across restarts. To mute, long-press K2 on the box.", &[])).size(13),
            space().height(8),
            text(tr("Character", &[])).size(18),
            text(tr(
                "The box is speaking as “%@”. Each character has its own voice and lines. Click Use to write another one to it — no firmware flash needed. Over the UART port this takes a few minutes; when it's done the box says a line as the new character.",
                &[&using]
            ))
            .size(13),
            address,
            character_card(robot_card.into(), robot_in_use),
            column(cards).spacing(10),
            character_card(custom_card.into(), custom_in_use),
        ]
        .push(progress)
        .spacing(12);
        scrollable(page).into()
    }

    fn agents(&self) -> Element<'_, Message> {
        let line = |agent: Agent| {
            let (name, last) = match agent {
                Agent::Claude => ("Claude Code", self.status.as_ref().and_then(|s| s.hooks.claude.as_deref())),
                Agent::Codex => ("Codex", self.status.as_ref().and_then(|s| s.hooks.codex.as_deref())),
            };
            let state = match last {
                Some(time) => tr("Last event %@", &[&short_time(time)]),
                None => tr("Waiting for the first event…", &[]),
            };
            row![text(name).width(140), text(state)].spacing(12).into()
        };
        column![
            text(tr("Connect agents", &[])).size(18),
            text(tr(
                "Vibe Buddy only forwards session IDs, event names and working directories — never prompts or replies.",
                &[]
            ))
            .size(13),
            column([line(Agent::Claude), line(Agent::Codex)]).spacing(8),
            row![
                button(text(tr("Connect", &[]))).on_press(Message::Hooks("install")),
                button(text(tr("Remove", &[]))).style(button::secondary).on_press(Message::Hooks("uninstall")),
            ]
            .spacing(8),
            text(tr(
                "After writing, open /hooks in Codex to review and trust this config — the app can't do that for you.",
                &[]
            ))
            .size(13),
        ]
        .spacing(12)
        .into()
    }

    fn device(&self) -> Element<'_, Message> {
        let device = self.status.as_ref().map(|status| &status.device);
        let link = match device {
            None => tr("daemon isn't running", &[]),
            Some(device) if !device.connected && !device.candidates.is_empty() => tr("Several devices found", &[]),
            Some(device) if !device.connected => tr("Box not found", &[]),
            Some(device) => {
                let kind = if device.bridge { tr("UART bridge", &[]) } else { tr("native USB", &[]) };
                format!("{} · {kind}", device.port.as_deref().unwrap_or("?"))
            }
        };
        let firmware = device.and_then(|device| device.firmware_label()).unwrap_or_else(|| "—".to_owned());
        let online = device.is_some_and(|device| device.connected);
        let updates = self.updates();
        let offer = updates.and_then(|updates| updates.firmware.as_ref());
        // The offered version, or why there is none: a bare dash read as "no update checks at all".
        let latest = match (offer, updates) {
            (Some(offer), _) => offer.version.clone(),
            (None, None) => "—".to_owned(),
            (None, Some(updates)) if !updates.enabled => tr("Update checks are off (General)", &[]),
            (None, Some(updates)) if updates.error.is_some() => tr("Last check failed", &[]),
            (None, Some(updates)) if updates.last_check.is_none() => tr("Not checked yet", &[]),
            (None, Some(_)) => tr("None released yet", &[]),
        };
        let downloaded = updates.and_then(status::Updates::firmware_directory).is_some();
        let operation = self.status.as_ref().and_then(|status| status.operation.as_ref());
        let busy = operation.is_some_and(status::Operation::running);
        let outdated = updates.is_some_and(status::Updates::firmware_update_available);
        // The daemon judged the box to run other firmware: a factory box, Muse, or one held in download mode with K0.
        let other_firmware = online && device.is_some_and(|device| device.foreign_firmware && device.firmware_build.is_none());
        let foreign = other_firmware && downloaded;
        // Firmware is wanted (a factory box, or a newer one exists) but isn't on disk yet: say why, and what else works.
        let unavailable = ((other_firmware || offer.is_some_and(|offer| offer.newer_than_box)) && !downloaded).then(|| {
            let reason = match updates {
                Some(updates) if updates.enabled => match (&updates.error, offer) {
                    (Some(error), _) => tr("Couldn't get the firmware: %@", &[error]),
                    (None, Some(_)) => tr("Downloading the firmware…", &[]),
                    (None, None) => tr("Looking for firmware…", &[]),
                },
                _ => tr("Update checks are off, so Vibe Buddy can't download firmware.", &[]),
            };
            column![
                text(reason).size(13),
                text(tr("Or download the firmware zip yourself and use Flash from file… on the Device tab:", &[])).size(13),
                row![
                    button(text(tr("Check again", &[])))
                        .on_press_maybe(updates.is_some_and(|updates| updates.enabled).then_some(Message::CheckForUpdatesNow)),
                    button(text(tr("Releases", &[]))).style(button::secondary).on_press(Message::OpenReleases),
                ]
                .spacing(8),
            ]
            .spacing(4)
        });
        // Either offer asks once more before flashing, since the box restarts.
        let confirm = |title: String, detail: String, action: String| -> Element<'_, Message> {
            column![
                text(title),
                text(detail).size(13),
                row![
                    button(text(action)).on_press_maybe((!busy).then_some(Message::FlashFirmware)),
                    button(text(tr("Cancel", &[])))
                        .style(button::secondary)
                        .on_press(Message::AskFirmwareUpdate(false)),
                ]
                .spacing(8),
            ]
            .spacing(8)
            .into()
        };
        let update: Option<Element<'_, Message>> = match (foreign, outdated && online, self.confirm_firmware) {
            (true, _, true) => Some(confirm(
                tr("Flash the box with Vibe Buddy?", &[]),
                tr(
                    "The box's current firmware and data will be erased and can't be recovered. It restarts on its own when done; over the UART port this takes a few minutes.",
                    &[],
                ),
                tr("Flash", &[]),
            )),
            (true, _, false) => Some(
                column![
                    text(tr("The box isn't running Vibe Buddy firmware.", &[])).style(text::warning),
                    button(text(tr("Flash Vibe Buddy firmware", &[])))
                        .on_press_maybe((!busy).then_some(Message::AskFirmwareUpdate(true))),
                ]
                .spacing(8)
                .into(),
            ),
            (false, true, true) => Some(confirm(
                tr("Update the box firmware to %@?", &[&latest]),
                {
                    let restart = tr(
                        "The box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes.",
                        &[],
                    );
                    match offer.and_then(|offer| offer.notes(i18n::is_chinese())) {
                        Some(notes) => format!("{notes}\n\n{restart}"),
                        None => restart,
                    }
                },
                tr("Update", &[]),
            )),
            (false, true, false) => Some(
                button(text(tr("Update to %@", &[&latest])))
                    .on_press_maybe((!busy).then_some(Message::AskFirmwareUpdate(true)))
                    .into(),
            ),
            (false, false, _) => None,
        };
        let flashing = operation.filter(|operation| operation.kind == status::OperationKind::Firmware).map(|operation| {
            let failed = (operation.state == status::OperationState::Failed).then(|| {
                column![
                    text(tr("Before retrying, hold K0 on the box and replug the cable to put it in download mode.", &[]))
                        .size(13),
                    button(text(tr("Retry", &[])))
                        .on_press_maybe((online && downloaded).then_some(Message::FlashFirmware)),
                ]
                .spacing(8)
            });
            column![text(operation.summary()).size(13)].push(failed).spacing(8)
        });
        // Not while a flash waits for a replug: that row says not to hold K0 this time.
        let replug = operation.is_some_and(|operation| operation.state == status::OperationState::Replug);
        // A pin left set hides every other box behind "Box not found", so it is always shown.
        let pin = device.and_then(|device| device.pin.as_ref()).map(|pin| {
            column![
                text(tr("Only looking at %@, set by %@.", &[&pin.value, &pin.variable])).size(13).style(text::warning),
                text(tr("To find the box on its own again, unset %@ and restart the daemon.", &[&pin.variable])).size(13),
            ]
            .spacing(4)
        });
        // Every ESP32-S3 on native USB looks the same: with several plugged in and none of them the box seen
        // before, the user says which one it is.
        let candidates = device.map(|device| device.candidates.as_slice()).unwrap_or_default();
        let choice = (!online && !candidates.is_empty()).then(|| {
            let rows = candidates.iter().map(|candidate| {
                let label = match &candidate.usb_serial {
                    Some(serial) => format!("{} · {serial}", candidate.port),
                    None => candidate.port.clone(),
                };
                let pick = candidate.usb_serial.clone().map(|serial| {
                    button(text(tr("This is the box", &[]))).on_press_maybe((!busy).then_some(Message::ChooseBox(serial)))
                });
                row![text(label).size(13).font(Font::MONOSPACE), space::horizontal()].push(pick).spacing(12).into()
            });
            column![text(tr(
                "Several devices are plugged in, and none of them is the box seen before. Which one is the box?",
                &[]
            ))
            .size(13)]
            .extend(rows)
            .spacing(6)
        });
        let not_found = (self.status.is_some() && !online && !busy && !replug && candidates.is_empty() && pin.is_none()).then(|| {
            column![
                text(tr("Not showing up? Use a cable that carries data, not just power.", &[])).size(13),
                text(tr(
                    "If the daemon log on the Advanced tab says “Permission denied”, your account can't open the box's port yet: restart the computer once, then replug the box.",
                    &[]
                ))
                .size(13),
                text(tr(
                    "Still nothing? Hold K0 on the box while you plug in the cable. The box starts in download mode with a dark screen, ready to be flashed with Vibe Buddy firmware.",
                    &[]
                ))
                .size(13),
            ]
            .spacing(4)
        });
        // The frame is dark whatever the theme, so its text is light: in a light theme the theme's own text color
        // vanished there, and a grab in progress looked like a button that did nothing.
        let on_frame = |label: String| text(label).size(13).color(iced::Color::from_rgb8(0xd0, 0xd0, 0xd0));
        let screen: Element<'_, Message> = match (&self.screenshot, self.screenshot_busy) {
            (_, true) => on_frame(tr("Refreshing…", &[])).into(),
            // Nearest-neighbour keeps the box's pixels crisp when scaled up.
            (Some((_, handle)), false) => image(handle.clone())
                .filter_method(image::FilterMethod::Nearest)
                .width(Length::Fill)
                .into(),
            (None, false) => on_frame(tr("Click Refresh to see what the box is showing", &[])).into(),
        };
        column![row![text(tr("Link", &[])).width(140), text(link)].spacing(12)]
            .push(pin)
            .push(choice)
            .push(not_found)
            .push(row![text(tr("Box firmware", &[])).width(140), text(firmware)].spacing(12))
            .push(row![text(tr("Latest firmware", &[])).width(140), text(latest)].spacing(12))
            .push(device.filter(|_| online).and_then(|device| device.unsupported_board.as_ref()).map(|board| {
                text(tr("This is a %@ board, which released firmware doesn't run on, so no update is offered.", &[board])).size(13)
            }))
            .push(update)
            .push(unavailable)
            .push(flashing)
            .push(column![
                button(text(tr("Make the box blink", &[]))).on_press_maybe(online.then_some(Message::Identify)),
                row![
                    text(tr("Box screen", &[])),
                    space::horizontal(),
                    button(text(tr("Refresh", &[])))
                        .on_press_maybe((online && !self.screenshot_busy).then_some(Message::TakeScreenshot)),
                    button(text(tr("Save image", &[])))
                        .style(button::secondary)
                        .on_press_maybe(self.screenshot.is_some().then_some(Message::SaveScreenshot)),
                ]
                .spacing(8),
                container(screen)
                    .padding(4)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .center(Length::Fill)
                    .style(|_| container::background(iced::Color::from_rgb8(0x16, 0x16, 0x16))),
            ]
            .spacing(12))
            .spacing(12)
            .into()
    }

    fn advanced(&self) -> Element<'_, Message> {
        let daemon = match &self.status {
            Some(status) => tr("Running · %@", &[&status.daemon.build]),
            None => tr("Not running", &[]),
        };
        let config = config_dir()
            .map(|dir| dir.join("config.json").display().to_string())
            .unwrap_or_default();
        column![
            row![text("daemon").width(140), text(daemon)].spacing(12),
            button(text(tr("Restart daemon", &[]))).on_press_maybe(self.status.is_some().then_some(Message::RestartDaemon)),
            // The daemon logs to the journal here, not a file; the folder holds the hook's log.
            row![
                button(text(tr("Show daemon log", &[]))).on_press(Message::ShowDaemonLog),
                button(text(tr("Open logs folder", &[]))).style(button::secondary).on_press(Message::OpenLogsFolder),
                button(text(tr("Export diagnostics…", &[]))).style(button::secondary).on_press(Message::ExportDiagnostics),
            ]
            .spacing(8),
            text(tr("Config file: %@", &[&config])).size(13),
        ]
        .spacing(12)
        .into()
    }

    /// Both sides' build IDs, the voice and the OS, as in the Mac app's summary.txt; also what a problem report
    /// starts with. No logs and nothing an agent did.
    fn diagnostics_summary(&self) -> String {
        let device = self.status.as_ref().map(|status| &status.device);
        format!(
            "App {}\ndaemon {}\nfirmware {}\noffered firmware {}\nvoice {}\nOS {}\n",
            env!("CARGO_PKG_VERSION"),
            self.status.as_ref().map_or("not connected", |status| status.daemon.build.as_str()),
            device.and_then(|device| device.firmware_label()).as_deref().unwrap_or("—"),
            self.updates().and_then(|updates| updates.firmware.as_ref()).map_or("—", |offer| offer.version.as_str()),
            device.and_then(|device| device.voice.as_deref()).unwrap_or("—"),
            os_name(),
        )
    }
}

/// The distribution's own name for itself, from os-release.
fn os_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|release| {
            release.lines().find_map(|line| Some(line.strip_prefix("PRETTY_NAME=")?.trim_matches('"').to_owned()))
        })
        .unwrap_or_else(|| std::env::consts::OS.to_owned())
}

/// "Report a problem": a new GitHub issue with the versions filled in, as the Mac app's `IssueReport`.
fn issue_url(summary: &str) -> reqwest::Url {
    let body = format!(
        "**What happened?**\n\n\n\n**What did you expect?**\n\n\n\n---\n```\n{}\n```",
        summary.trim_end()
    );
    reqwest::Url::parse_with_params(NEW_ISSUE, [("body", body)]).expect("a fixed, valid URL")
}

/// Starts a desktop helper and lets it run on its own.
fn launch(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(drop)
        .map_err(|error| format!("cannot run {program}: {error}"))
}

/// Logs, config and both sides' build IDs in one folder under Downloads, with no hook payloads; the folder opens when
/// it's ready. The daemon's log comes from the journal, two days of it.
async fn export_diagnostics(summary: String, config: Option<Config>) -> Result<String, String> {
    let seconds = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let target = user_dir("DOWNLOAD", "Downloads")?.join(format!("vibe-buddy-diagnostics-{seconds}"));
    let fail = |error: std::io::Error| format!("{}: {error}", target.display());
    tokio::fs::create_dir_all(&target).await.map_err(fail)?;
    let journal = tokio::process::Command::new("journalctl")
        .args(["--user", "-u", "vibebuddyd", "--since", "-2d", "--no-pager", "-o", "short-iso"])
        .output()
        .await
        .map_err(|error| format!("cannot run journalctl: {error}"))?;
    tokio::fs::write(target.join("vibebuddyd.log"), journal.stdout).await.map_err(fail)?;
    if let Some(hooks) = state_dir().map(|dir| dir.join("codex-hooks.log")).filter(|path| path.is_file()) {
        tokio::fs::copy(hooks, target.join("codex-hooks.log")).await.map_err(fail)?;
    }
    if let Some(config) = config {
        let text = serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?;
        tokio::fs::write(target.join("config.json"), text).await.map_err(fail)?;
    }
    tokio::fs::write(target.join("summary.txt"), summary).await.map_err(fail)?;
    launch("xdg-open", &[target.as_os_str()])?;
    Ok(tr("Saved to %@", &[&target.display()]))
}

/// Where every release lives: the new App, and the firmware zip for flashing by hand when the daemon can't get it.
const RELEASES_PAGE: &str = "https://github.com/second-state/vibebuddy/releases";
const NEW_ISSUE: &str = "https://github.com/second-state/vibebuddy/issues/new";

/// The id under which the box wears the user's own Character.
const CUSTOM: &str = "custom";
/// The robot, the default Character: written as `robot`, or `builtin` when the box has no pack at all.
const ROBOT: &str = "robot";
const BUILTIN: &str = "builtin";

/// The Mac app's `CustomCharacterCard.prompt`: a start for any image model.
const CUSTOM_PROMPT: &str = "Pixel art game sprite of [describe your character], chibi proportions, standing, front view, full body, centered, arms down, flat colors, thick dark outline, limited 16-color palette, plain solid white background. Then the same character in exactly the same pose with the eyes closed; with a big happy smile; with a sad face.";

/// A look frame as an image to draw, each pixel `scale` pixels wide.
fn handle(frame: &character::Image, scale: usize) -> image::Handle {
    let (w, h) = (frame.width * scale, frame.height * scale);
    let mut pixels = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let from = ((y / scale) * frame.width + x / scale) * 4;
            pixels[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&frame.pixels[from..from + 4]);
        }
    }
    image::Handle::from_rgba(w as u32, h as u32, pixels)
}

/// A Character's card, outlined in the accent color when the box wears it.
fn character_card(content: Element<'_, Message>, in_use: bool) -> Element<'_, Message> {
    container(content)
        .padding(10)
        .width(Length::Fill)
        .style(move |theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.weak.color.into()),
                border: iced::Border {
                    color: if in_use { palette.primary.base.color } else { iced::Color::TRANSPARENT },
                    width: 2.0,
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}

/// A Character that can lend voice and lines, as the picker shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Lender {
    id: &'static str,
}

impl Lender {
    fn of(voice: &assets::Voice) -> Lender {
        Lender { id: voice.id }
    }
}

impl std::fmt::Display for Lender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&assets::voice_name(self.id))
    }
}

/// One entry of a form-of-address picker: a form of a language, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AddressChoice {
    form: Option<&'static str>,
    language: assets::Language,
}

impl std::fmt::Display for AddressChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.form.and_then(|id| assets::FORMS_OF_ADDRESS.iter().find(|form| form.id == id)) {
            Some(form) => f.write_str(form.words),
            None => f.write_str(&tr("Nothing", &[])),
        }
    }
}

/// The daemon's XDG directories (see `daemon/src/config.rs`), where it keeps state and config and the hook its log.
fn state_dir() -> Option<std::path::PathBuf> {
    xdg_dir("XDG_STATE_HOME", ".local/state")
}

fn config_dir() -> Option<std::path::PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

fn xdg_dir(variable: &str, default: &str) -> Option<std::path::PathBuf> {
    std::env::var_os(variable)
        .map(std::path::PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(default)))
        .map(|dir| dir.join("vibebuddy"))
}

/// A folder from `xdg-user-dir` (Pictures, Downloads…), or the usual name under HOME when that tool is missing.
fn user_dir(kind: &str, fallback: &str) -> Result<std::path::PathBuf, String> {
    std::process::Command::new("xdg-user-dir")
        .arg(kind)
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(fallback)))
        .ok_or_else(|| "HOME is not set".to_owned())
}

/// The voice a language change asked the restarted app to write (`--write-voice <id>`), if it is one we know.
fn launch_voice() -> Option<&'static str> {
    let args: Vec<String> = std::env::args().collect();
    let index = args.iter().position(|arg| arg == WRITE_VOICE)?;
    assets::voice_id(args.get(index + 1)?)
}

/// Starts a new copy that waits for this one to quit, then opens Settings. It goes through systemd-run: started by the
/// login autostart, this copy is a systemd service, and anything it left behind would be stopped along with it.
fn restart(voice: Option<&str>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut args = vec![exe.into_os_string(), RESTARTED.into()];
    if let Some(id) = voice {
        args.extend([WRITE_VOICE.into(), id.into()]);
    }
    let started = std::process::Command::new("systemd-run")
        .args(["--user", "--collect", "--quiet"])
        .args(&args)
        .status()
        .is_ok_and(|status| status.success());
    if started {
        return Ok(());
    }
    std::process::Command::new(&args[0])
        .args(&args[1..])
        .spawn()
        .map(drop)
        .map_err(|error| format!("cannot restart: {error}"))
}

/// True only the first time the app starts for this user; a marker in the state directory remembers it.
fn first_launch() -> bool {
    let Some(state) = state_dir() else { return false };
    let marker = state.join("desktop-launched");
    if marker.exists() {
        return false;
    }
    let _ = std::fs::create_dir_all(&state).and_then(|()| std::fs::write(&marker, ""));
    true
}

/// Saves into the user's Pictures directory, where Omarchy's own screenshots go: there is no save dialog
/// to borrow on a tiling desktop, and the path is shown afterwards.
fn save_screenshot(png: &[u8]) -> Result<String, String> {
    let pictures = user_dir("PICTURES", "Pictures")?;
    let seconds = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let path = pictures.join(format!("vibe-buddy-{seconds}.png"));
    std::fs::create_dir_all(&pictures)
        .and_then(|()| std::fs::write(&path, png))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(tr("Saved to %@", &[&path.display()]))
}

/// `2026-10-01T17:06:59.003+08:00` → `2026-10-01 17:06`, in the daemon's own (local) offset.
fn short_time(rfc3339: &str) -> String {
    rfc3339.get(..16).map(|prefix| prefix.replacen('T', " ", 1)).unwrap_or_else(|| rfc3339.to_owned())
}

/// Hook config is written by `vibebuddy-hook install|uninstall`, installed next to this binary; the rules for
/// merging into other programs' config live there and only there.
async fn run_hook_tool(action: &'static str) -> Result<String, String> {
    let hook = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .with_file_name("vibebuddy-hook");
    let output = tokio::process::Command::new(&hook)
        .arg(action)
        .output()
        .await
        .map_err(|error| format!("cannot run {}: {error}", hook.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if output.status.success() {
        Ok(stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

#[cfg(target_os = "linux")]
fn run_tray() -> impl futures::Stream<Item = Message> {
    use futures::{SinkExt, StreamExt};
    use ksni::TrayMethods;

    iced::stream::channel(16, async |mut output| {
        let (events, mut clicks) = futures::channel::mpsc::unbounded();
        let tray = tray::Tray { menu: MenuState::derive(None), color: [255, 255, 255], events };
        // At login the app can start before the shell's tray host is up; ksni then registers once it appears, and
        // again after the shell restarts. The app opens the window if it doesn't (`Message::TrayHost`).
        match tray.assume_sni_available(true).spawn().await {
            Ok(handle) => {
                let _ = output.send(Message::TrayReady(TrayHandle(handle))).await;
            }
            Err(error) => {
                eprintln!("vibebuddy-desktop: no tray available ({error}); opening the window instead");
                let _ = output.send(Message::TrayUnavailable).await;
                return;
            }
        }
        while let Some(event) = clicks.next().await {
            let message = match event {
                tray::TrayEvent::OpenSettings => Message::OpenSettings,
                tray::TrayEvent::CheckForUpdates => Message::CheckForUpdatesNow,
                tray::TrayEvent::ReportProblem => Message::ReportProblem,
                tray::TrayEvent::Quit => Message::Quit,
                tray::TrayEvent::HostGone => Message::TrayHost(false),
                tray::TrayEvent::HostBack => Message::TrayHost(true),
            };
            let _ = output.send(message).await;
        }
    })
}

/// Another copy started (the launcher, while this one runs) connects here: show Settings, as the Mac app does on reopen.
#[cfg(target_os = "linux")]
fn run_instance() -> impl futures::Stream<Item = Message> {
    use futures::SinkExt;

    iced::stream::channel(4, async |mut output| {
        let Some(listener) = INSTANCE.lock().ok().and_then(|mut listener| listener.take()) else { return };
        let listener = match listener.set_nonblocking(true).and_then(|()| tokio::net::UnixListener::from_std(listener)) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("vibebuddy-desktop: cannot listen for other copies ({error})");
                return;
            }
        };
        while listener.accept().await.is_ok() {
            let _ = output.send(Message::OpenSettings).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_times_read_as_local_minutes() {
        assert_eq!(short_time("2026-10-01T17:06:59.003370003+08:00"), "2026-10-01 17:06");
        assert_eq!(short_time("garbage"), "garbage");
    }

    #[test]
    fn a_problem_report_opens_a_new_issue_with_the_versions() {
        let url = issue_url("App 0.3.5\nOS Omarchy 3.1+beta\n");
        assert!(url.as_str().starts_with("https://github.com/second-state/vibebuddy/issues/new?body="));
        let body = url.query_pairs().find(|(name, _)| name == "body").map(|(_, value)| value.into_owned());
        assert!(body.is_some_and(|body| body.contains("App 0.3.5\nOS Omarchy 3.1+beta\n```")));
    }
}
