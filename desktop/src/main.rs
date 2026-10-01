//! `vibebuddy-desktop`: Vibe Buddy's tray icon and settings window outside macOS, the counterpart of the Mac
//! app's menu bar icon and settings window. It is only a client of `vibebuddyd`: systemd supervises the
//! daemon here, so quitting the app leaves the box online.

// The tray, and the face it draws, exist only on Linux; other builds are for working on the window.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod assets;
mod client;
mod face;
mod i18n;
mod status;
mod theme;
#[cfg(target_os = "linux")]
mod tray;

use std::time::{Duration, SystemTime};

use iced::widget::{button, column, container, image, row, scrollable, slider, space, text, toggler};
use iced::{Element, Font, Length, Subscription, Task, Theme, window};

use i18n::tr;
use status::{Config, MenuState, Status};

/// Omarchy's monospace font, used when Omarchy's theme is: the window should look like the rest of the desktop.
const OMARCHY_FONT: &str = "JetBrainsMono Nerd Font";

fn main() -> iced::Result {
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
    ThemeTick,
    OpenSettings,
    WindowClosed(window::Id),
    Tab(Tab),
    NotifyLink(bool),
    ConfigSaved(Result<Config, String>),
    VolumeDragged(u8),
    VolumeReleased,
    PlayLine,
    Identify,
    UseVoice(&'static str),
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
    firmware: Option<assets::Firmware>,
    /// The firmware update waits for a second click, since the box restarts.
    confirm_firmware: bool,
    /// The outcome of the last action, shown at the bottom of the window.
    notice: Option<Result<String, String>>,
    #[cfg(target_os = "linux")]
    tray: Option<TrayHandle>,
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
            voices: assets::voices(),
            firmware: assets::firmware(),
            confirm_firmware: false,
            notice: None,
            #[cfg(target_os = "linux")]
            tray: None,
        };
        // The first launch shows the window, so it's clear where the app went; later ones stay in the tray. Without a
        // tray (macOS builds, for development) the window is the only way in.
        let open = first_launch() || !cfg!(target_os = "linux");
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
        let tray = Subscription::run(run_tray);
        #[cfg(not(target_os = "linux"))]
        let tray = Subscription::none();
        Subscription::batch([
            Subscription::run(client::status_updates).map(Message::Daemon),
            iced::time::every(Duration::from_secs(2)).map(|_| Message::ThemeTick),
            window::close_events().map(Message::WindowClosed),
            tray,
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Daemon(client::Update::Status(status)) => {
                self.status = Some(*status);
                return self.refresh_tray();
            }
            Message::Daemon(client::Update::Down) => {
                self.status = None;
                return self.refresh_tray();
            }
            #[cfg(target_os = "linux")]
            Message::TrayReady(handle) => {
                self.tray = Some(handle);
                return self.refresh_tray();
            }
            Message::TrayUnavailable => return Task::done(Message::OpenSettings),
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
                let (id, open) = window::open(window::Settings {
                    size: iced::Size::new(620.0, 460.0),
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
            Message::Identify => return Task::perform(client::identify(), Message::Done),
            Message::UseVoice(id) => {
                let write = async move { client::write_voice_pack(assets::read_voice_pack(id).await?).await };
                return Task::perform(write, Message::Done);
            }
            Message::AskFirmwareUpdate(asking) => self.confirm_firmware = asking,
            Message::FlashFirmware => {
                self.confirm_firmware = false;
                if let Some(firmware) = self.firmware.clone() {
                    return Task::perform(client::flash_firmware(firmware), Message::Done);
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
            (Tab::Sound, tr("Sound", &[])),
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

    fn general(&self) -> Element<'_, Message> {
        let notify = self.status.as_ref().is_none_or(|status| status.config.notify_link);
        let menu = MenuState::derive(self.status.as_ref());
        column![
            text(menu.device_line),
            text(menu.mode_line),
            text(menu.today_line),
            space().height(8),
            toggler(notify)
                .label(tr("Notify me when the box disconnects or the daemon fails", &[]))
                .on_toggle_maybe(self.status.is_some().then_some(Message::NotifyLink)),
        ]
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
        let cards = self.voices.iter().map(|voice| {
            let action: Element<'_, Message> = if current.as_deref() == Some(voice.id) {
                text(tr("In use", &[])).style(text::success).into()
            } else {
                button(text(tr("Use", &[])))
                    .on_press_maybe((online && !busy).then_some(Message::UseVoice(voice.id)))
                    .into()
            };
            row![column![text(voice.name.clone()), text(voice.tag.clone()).size(13)], space::horizontal(), action]
                .spacing(12)
                .into()
        });
        let progress = operation.filter(|operation| operation.kind == status::OperationKind::VoicePack).map(|operation| {
            let failed = operation.state == status::OperationState::Failed;
            column![text(operation.summary()).size(13)].push(failed.then(|| {
                text(tr("Didn't finish, so the box keeps its built-in voice. Reconnect the cable and click Use again.", &[]))
                    .size(13)
            }))
        });
        let using = assets::voice_name(current.as_deref().unwrap_or("builtin"));
        let page = column![
            row![text(tr("Volume", &[])), volume, text(level.to_string())].spacing(12),
            button(text(tr("Play a line on the box", &[]))).on_press_maybe(online.then_some(Message::PlayLine)),
            text(tr("Saved on the box and kept across restarts. To mute, long-press K2 on the box.", &[])).size(13),
            space().height(8),
            text(tr("Announcement voice", &[])).size(18),
            text(tr(
                "The box is using “%@”. Click Use to write another voice to it — no firmware flash needed. Over the UART port this takes a few minutes; when it's done the box says a line in the new voice.",
                &[&using]
            ))
            .size(13),
            column(cards).spacing(10),
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
            Some(device) if !device.connected => tr("Box not found", &[]),
            Some(device) => {
                let kind = if device.bridge { tr("UART bridge", &[]) } else { tr("native USB", &[]) };
                format!("{} · {kind}", device.port.as_deref().unwrap_or("?"))
            }
        };
        let firmware = device.and_then(|device| device.firmware_build.clone()).unwrap_or_else(|| "—".to_owned());
        let online = device.is_some_and(|device| device.connected);
        let bundled = self
            .firmware
            .as_ref()
            .map(|firmware| firmware.build.clone())
            .unwrap_or_else(|| tr("This build has no bundled firmware", &[]));
        let operation = self.status.as_ref().and_then(|status| status.operation.as_ref());
        let busy = operation.is_some_and(status::Operation::running);
        let outdated = status::firmware_update_available(
            device.and_then(|device| device.firmware_build.as_deref()),
            self.firmware.as_ref().map(|firmware| firmware.build.as_str()),
        );
        let update: Option<Element<'_, Message>> = (outdated && online).then(|| {
            if self.confirm_firmware {
                column![
                    text(tr("Update the box firmware?", &[])),
                    text(tr(
                        "The box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes.",
                        &[]
                    ))
                    .size(13),
                    row![
                        button(text(tr("Update", &[]))).on_press_maybe((!busy).then_some(Message::FlashFirmware)),
                        button(text(tr("Cancel", &[])))
                            .style(button::secondary)
                            .on_press(Message::AskFirmwareUpdate(false)),
                    ]
                    .spacing(8),
                ]
                .spacing(8)
                .into()
            } else {
                button(text(tr("Update to bundled version", &[])))
                    .on_press_maybe((!busy).then_some(Message::AskFirmwareUpdate(true)))
                    .into()
            }
        });
        let flashing = operation
            .filter(|operation| operation.kind == status::OperationKind::Firmware)
            .map(|operation| text(operation.summary()).size(13));
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
        column![
            row![text(tr("Link", &[])).width(140), text(link)].spacing(12),
            row![text(tr("Box firmware", &[])).width(140), text(firmware)].spacing(12),
            row![text(tr("Bundled with app", &[])).width(140), text(bundled)].spacing(12),
        ]
        .push(update)
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

    /// Both sides' build IDs and the voice, as in the Mac app's summary.txt.
    fn diagnostics_summary(&self) -> String {
        let device = self.status.as_ref().map(|status| &status.device);
        format!(
            "App {}\ndaemon {}\nfirmware {}\nbundled firmware {}\nvoice {}\n",
            env!("CARGO_PKG_VERSION"),
            self.status.as_ref().map_or("not connected", |status| status.daemon.build.as_str()),
            device.and_then(|device| device.firmware_build.as_deref()).unwrap_or("—"),
            self.firmware.as_ref().map_or("—", |firmware| firmware.build.as_str()),
            device.and_then(|device| device.voice.as_deref()).unwrap_or("—"),
        )
    }
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
        match tray.spawn().await {
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
                tray::TrayEvent::Quit => Message::Quit,
            };
            let _ = output.send(message).await;
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
}
