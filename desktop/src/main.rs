//! `vibebuddy-desktop`: VibeBuddy's tray icon and settings window outside macOS, the counterpart of the Mac
//! app's menu bar icon and settings window. It is only a client of `vibebuddyd`: systemd supervises the
//! daemon here, so quitting the app leaves the box online.

// The tray, and the face it draws, exist only on Linux; other builds are for working on the window.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod agents;
mod assets;
mod character;
mod client;
mod custom;
mod face;
mod firmware_file;
mod i18n;
mod making;
#[cfg(target_os = "linux")]
mod instance;
mod korekore;
mod preview;
mod status;
mod theme;
#[cfg(target_os = "linux")]
mod tray;

use std::time::{Duration, SystemTime};

use iced::widget::{button, checkbox, column, container, image, pick_list, progress_bar, radio, row, scrollable, slider, space, text, text_input, toggler};
use iced::{Element, Font, Length, Subscription, Task, Theme, window};

use i18n::{UiLanguage, tr};
use status::{Config, MenuState, Status};

/// How many exchanges the Conversation tab reads, as on the Mac.
const CONVERSATION_LIMIT: usize = 20;

/// Omarchy's monospace font, used when Omarchy's theme is: the window should look like the rest of the desktop.
const OMARCHY_FONT: &str = "JetBrainsMono Nerd Font";

/// Passed to the copy a language change starts: it waits for this one to quit and opens Settings where the user was.
const RESTARTED: &str = "--restarted";
/// Followed by a voice id: written once the restarted copy sees the box answer, then forgotten.
const WRITE_VOICE: &str = "--write-voice";
/// Passed by the Omarchy bar plugin: open Settings, even from a launch that would otherwise stay in the background.
const SETTINGS: &str = "--settings";

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
    if korekore::requested() {
        // The korekore UI's fonts, there before any window opens (ADR-0021); its own face is the default. The
        // current UI keeps the system's, or Omarchy's.
        for font in korekore::FONTS {
            app = app.font(font);
        }
        app = app.default_font(korekore::DEFAULT_FONT);
    } else if theme::stamp().is_some() {
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
    /// Only while the daemon runs the resident session (experimental).
    Conversation,
    Account,
    Advanced,
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
    /// A page of the first-run guide, or (None) leave it for the tabs.
    Guide(Option<usize>),
    WindowClosed(window::Id),
    Tab(Tab),
    NotifyLink(bool),
    CheckUpdates(bool),
    Relay(bool),
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
    /// Which set of screens the box draws (ADR-0021).
    SetScreens(BoxScreens),
    /// The user says which of several devices is the box, by its USB serial number.
    ChooseBox(String),
    /// Forget another computer on the box, by its key.
    Unpair(String),
    TakeOver,
    WifiSsid(String),
    WifiPassword(String),
    JoinWifi,
    /// Show the fields again to join another network.
    ChangeWifi,
    ForgetWifi,
    ScanWifi,
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
    /// Making a Character (ADR-0020): open the form in place of the Character tab, or (false) leave it.
    MakeCharacter(bool),
    MakeBase(Lender),
    /// A preset's lines, for the form if it still starts from that preset.
    MakeLines(&'static str, Result<Vec<making::Line>, String>),
    MakeName(String),
    MakeVoice(making::Voice),
    /// The korekore sheet's pick: a preset's voice takes that preset's lines too, while none has changed.
    MakeVoiceAndLines(making::Voice),
    MakeLine(usize, String),
    MakeRemoveLine(usize),
    MakeChooseDrawings,
    MakeDrawings(Result<Option<Vec<character::Image>>, String>),
    /// Wear the base's look again rather than the user's.
    MakeBaseLook,
    /// The form has only a new look: it goes to the box as the user's own Character, the preset lending its voice.
    MakeUseLook,
    MakeStart,
    /// The daemon took the job, from the form or a Resume, or didn't.
    MakeStarted(Result<making::Making, String>),
    MadeCharacters(Result<Vec<making::MadeCharacter>, String>),
    MadeFaces(Vec<(String, image::Handle)>),
    CharacterVoices(Result<Vec<making::Voice>, String>),
    /// Write a made Character to the box, by id.
    UseMade(String),
    /// Make the rest of a made Character whose job stopped, by id.
    ResumeMade(String),
    /// Ask to delete a made Character (by id), or (None) don't; deleting waits for a second click.
    AskDeleteMade(Option<String>),
    /// Asking before forgetting the user's own Character, or not.
    AskDeleteCustom(bool),
    DeleteCustom,
    DeleteMade(String),
    MadeDeleted(Result<(), String>),
    AskFirmwareUpdate(bool),
    FlashFirmware,
    FlashFromFile,
    FirmwareFile(Result<Option<firmware_file::Package>, String>),
    /// Flash the package picked from a file, or (false) forget it.
    FlashFile(bool),
    TakeScreenshot,
    Screenshot(Result<Vec<u8>, String>),
    SaveScreenshot,
    RestartDaemon,
    LaunchAtLogin(bool),
    /// Through systemd, which works when the daemon isn't answering, unlike its own restart endpoint.
    StartDaemon,
    /// The tray's Check for Updates…: check, and show General, where the answer appears (the Mac shows Sparkle's).
    CheckFromMenu,
    /// Play a Character's preview lines here, or stop the one playing.
    TogglePreview(&'static str),
    PreviewDone(String, Result<(), String>),
    /// Whether the daemon runs the resident session, asked each time it comes up.
    Resident(Result<bool, String>),
    RefreshConversation,
    Conversation(Result<Vec<client::Exchange>, client::HistoryError>),
    /// Ask `GET /v1/resident` again while the Conversation tab shows.
    PollConversation,
    ConversationPolled(Result<client::ResidentInfo, client::HistoryError>),
    OpenCodexInstallPage,
    OpenClaudeInstallPage,
    OpenAnthropicConsole,
    /// Which Agent answers voice commands.
    PickBrain(client::BrainChoice),
    AnthropicKeyDraft(String),
    SaveAnthropicKey,
    RemoveAnthropicKey,
    /// The daemon's answer to a brain or key change.
    ResidentSettingSaved(Result<client::ResidentInfo, String>),
    /// Ask `vibebuddy-hook` for each agent's state again.
    RefreshAgents,
    Agents(Result<Vec<agents::Setup>, String>),
    /// Connect or Repair (true) or Remove (false) one agent: first see what would change.
    PlanHooks(String, bool),
    HookPlan(Result<agents::Plan, String>),
    /// Write the plan shown, or (false) drop it.
    ApplyHooks(bool),
    HooksWritten(Result<String, String>),
    /// A finished action whose outcome is worth a line at the bottom of the window.
    Notice(Result<String, String>),
    OpenLogsFolder,
    ShowDaemonLog,
    ExportDiagnostics,
    /// Open the browser to sign in to the VibeBuddy account; clicking again starts over.
    SignIn,
    SignInUrl(Result<String, String>),
    SignOut,
    AskDeleteAccount(bool),
    DeleteAccount,
    /// Signing out or deleting the account finished; the account itself comes with the next status.
    AccountDone(Result<(), String>),
    OwnKeyDraft(client::KeyProvider, String),
    SaveOwnKey(client::KeyProvider),
    ClearOwnKey(client::KeyProvider),
    OpenOwnKeyPage(client::KeyProvider),
    /// The daemon took the key, or didn't; whether it is set comes with the next status.
    OwnKeySaved(client::KeyProvider, Result<(), String>),
    Done(Result<(), String>),
    Quit,
    /// A section picked in the korekore UI's sidebar (ADR-0021).
    Korekore(korekore::Section),
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
    /// The first-run guide's page while it shows in place of the tabs.
    guide: Option<usize>,
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
    /// `custom_look` is the look last written (with `characters.custom_lender`), not one just chosen.
    custom_look_written: bool,
    lender: Option<Lender>,
    custom_problem: Option<String>,
    /// The robot's face for its card.
    robot_face: Option<image::Handle>,
    /// The Characters made on this computer, each one's face, and the voices one can be made with (ADR-0020); asked
    /// when the daemon comes up and whenever a job changes state.
    made: Vec<making::MadeCharacter>,
    made_faces: Vec<(String, image::Handle)>,
    character_voices: Vec<making::Voice>,
    /// The Make a Character form, while it shows in place of the Character tab.
    make: Option<MakeForm>,
    /// The made Character waiting for a second click to be deleted, since it can't be undone.
    confirm_delete_made: Option<String>,
    confirm_delete_custom: bool,
    /// The firmware update waits for a second click, since the box restarts.
    confirm_firmware: bool,
    /// A firmware zip picked with Flash from file…, waiting for the user to confirm.
    file_firmware: Option<firmware_file::Package>,
    /// What is typed into the Wi-Fi fields; the password is cleared once sent.
    wifi_ssid: String,
    wifi_password: String,
    /// The fields are open again for another network; once one is set they fold away.
    wifi_editing: bool,
    /// Read from the autostart entry at start, then kept as the toggle writes it.
    launch_at_login: bool,
    /// The Character being previewed, and how to stop it.
    previewing: Option<(String, std::sync::Arc<tokio::sync::Notify>)>,
    /// Each agent's hook config, as `vibebuddy-hook status` last reported it.
    agents: Vec<agents::Setup>,
    /// A change to an agent's config, shown before it's written.
    hook_plan: Option<agents::Plan>,
    /// The daemon runs the experimental resident session, so the Conversation tab shows. Kept while the daemon is
    /// down, so the tab can say so.
    resident: bool,
    /// The last voice commands and spoken replies read, or why they couldn't be; None before the first read.
    conversation: Option<Result<Vec<client::Exchange>, client::HistoryError>>,
    conversation_loading: bool,
    /// The history's change marker when it was last read; None before that, and after the daemon went away.
    conversation_seen: Option<u64>,
    /// The daemon has the change marker; an older one gets its history read every 5 seconds instead.
    conversation_marked: bool,
    conversation_polling: bool,
    /// What voice commands are missing (the user's Codex or Claude Code), as the daemon last said.
    resident_problem: Option<client::Problem>,
    /// The daemon's last word on the resident session: which Agent the user picked, whether a key is stored.
    resident_info: Option<client::ResidentInfo>,
    /// The Anthropic API key being typed; cleared once the daemon has it, and never filled from the daemon.
    anthropic_key_draft: String,
    /// Why the last brain or key change failed.
    resident_setting_error: Option<String>,
    /// The browser was opened to sign in, until the status shows the account signed in.
    signing_in: bool,
    /// Deleting the account waits for a second click, since it can't be undone.
    confirm_delete_account: bool,
    /// The own keys being typed, by provider; cleared once the daemon has one, and never filled from the daemon.
    own_key_drafts: [String; 2],
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
    /// VibeBuddy's Omarchy plugin is in the bar, showing the face; the tray icon would be a second one.
    #[cfg(target_os = "linux")]
    plugin: bool,
    /// The korekore UI was asked for at launch (`VIBEBUDDY_UI=korekore`, ADR-0021); its sections are the tabs.
    korekore: bool,
}

/// Making a Character: the preset it starts from, its lines as edited (occasions stay, each keeps a line), the voice,
/// and the user's own look, if any.
struct MakeForm {
    base: Lender,
    name: String,
    voice: Option<making::Voice>,
    lines: Vec<making::Line>,
    /// The preset's lines as they came, to tell whether any changed.
    original: Vec<making::Line>,
    /// The preset's occasions, in its order.
    occasions: Vec<String>,
    /// The user's own look and its face; None wears the base's.
    look: Option<(Vec<u8>, image::Handle)>,
    problem: Option<String>,
    starting: bool,
}

impl MakeForm {
    /// Only the look is new: nothing to make, the preset lends its voice and lines.
    fn look_only(&self) -> bool {
        self.look.is_some() && self.lines == self.original && self.voice.as_ref().is_none_or(|voice| voice.id == self.base.id)
    }
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
            // The korekore sidebar starts on Character, as the design does.
            tab: if korekore::requested() { Tab::Sound } else { Tab::General },
            guide: None,
            volume: None,
            screenshot: None,
            screenshot_busy: false,
            voices: Vec::new(),
            faces: Vec::new(),
            characters: custom::Settings::load(),
            custom_look: None,
            custom_look_written: false,
            lender: None,
            custom_problem: None,
            robot_face: None,
            made: Vec::new(),
            made_faces: Vec::new(),
            character_voices: Vec::new(),
            make: None,
            confirm_delete_made: None,
            confirm_delete_custom: false,
            confirm_firmware: false,
            file_firmware: None,
            wifi_ssid: String::new(),
            wifi_password: String::new(),
            wifi_editing: false,
            launch_at_login: launch_at_login(),
            previewing: None,
            agents: Vec::new(),
            hook_plan: None,
            resident: false,
            conversation: None,
            conversation_loading: false,
            conversation_seen: None,
            conversation_marked: true,
            conversation_polling: false,
            resident_problem: None,
            resident_info: None,
            anthropic_key_draft: String::new(),
            resident_setting_error: None,
            signing_in: false,
            confirm_delete_account: false,
            own_key_drafts: Default::default(),
            notice: None,
            language: UiLanguage::saved(),
            restart_offer: None,
            pending_voice: launch_voice(),
            tray_host: true,
            #[cfg(target_os = "linux")]
            tray: None,
            #[cfg(target_os = "linux")]
            plugin: plugin_in_omarchy_bar(),
            korekore: korekore::requested(),
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
        app.custom_look_written = app.custom_look.is_some();
        // Debug builds only: open the korekore UI on a given section or first-run step, for checking it on a desktop
        // without a pointer (VIBEBUDDY_KOREKORE_SECTION=device, VIBEBUDDY_KOREKORE_GUIDE=1).
        #[cfg(debug_assertions)]
        if app.korekore
            && let Ok(name) = std::env::var("VIBEBUDDY_KOREKORE_SECTION")
            && let Some(section) = korekore::Section::ALL.into_iter().find(|section| format!("{section:?}").eq_ignore_ascii_case(&name))
        {
            app.tab = section.tab();
        }
        let restarted = std::env::args().any(|arg| arg == RESTARTED || arg == SETTINGS);
        let first = first_launch();
        // The install script connected the agents and set up the start at login; the guide is for what it couldn't.
        app.guide = (first && cfg!(target_os = "linux")).then_some(0);
        #[cfg(debug_assertions)]
        if app.korekore {
            app.guide = std::env::var("VIBEBUDDY_KOREKORE_GUIDE").ok().and_then(|page| page.parse().ok()).or(app.guide);
        }
        let open = first || restarted || !cfg!(target_os = "linux");
        let task = if open { Task::done(Message::OpenSettings) } else { Task::none() };
        (app, task)
    }

    fn title(&self, _window: window::Id) -> String {
        "VibeBuddy".to_owned()
    }

    fn theme(&self, _window: window::Id) -> Theme {
        self.theme.clone()
    }

    fn subscription(&self) -> Subscription<Message> {
        #[cfg(target_os = "linux")]
        let (tray, instance) = (
            if self.plugin { Subscription::none() } else { Subscription::run(run_tray) },
            Subscription::run(run_instance),
        );
        #[cfg(not(target_os = "linux"))]
        let (tray, instance) = (Subscription::none(), Subscription::none());
        // Only while the Conversation tab is in an open window: the history is read again when it changed.
        let conversation = if self.settings.is_some() && self.resident && self.tab == Tab::Conversation {
            let every = if self.conversation_marked { 2 } else { 5 };
            iced::time::every(Duration::from_secs(every)).map(|_| Message::PollConversation)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            Subscription::run(client::status_updates).map(Message::Daemon),
            iced::time::every(Duration::from_secs(2)).map(|_| Message::ThemeTick),
            window::close_events().map(Message::WindowClosed),
            tray,
            instance,
            conversation,
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
                notify_new_firmware(&status);
                if status.account.signed_in {
                    self.signing_in = false;
                } else {
                    self.confirm_delete_account = false;
                }
                // Only the daemon's environment at start decides, so ask once each time it comes up.
                let resident = if self.status.is_none() {
                    Task::perform(client::resident_enabled(), Message::Resident)
                } else {
                    Task::none()
                };
                let job = |status: &Status| status.making.as_ref().map(|making| (making.id.clone(), making.state.clone()));
                let made = if self.status.as_ref().is_none_or(|before| job(before) != job(&status)) {
                    self.refresh_made()
                } else {
                    Task::none()
                };
                self.status = Some(*status);
                return Task::batch([
                    self.refresh_tray(),
                    voice.map_or_else(Task::none, |id| Task::done(Message::UseVoice(id))),
                    resident,
                    made,
                ]);
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
                // The plugin can be enabled or disabled while the app runs: the tray icon gives way, or comes back
                // when the subscription starts it again.
                #[cfg(target_os = "linux")]
                {
                    let plugin = plugin_in_omarchy_bar();
                    if plugin != self.plugin {
                        self.plugin = plugin;
                        if let Some(TrayHandle(handle)) = self.tray.take() {
                            return Task::future(async move { handle.shutdown().await }).discard();
                        }
                    }
                }
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
                // The same as the Mac's: room for the whole Character tab, resizable down to the old size. The
                // korekore window is the design's 960 × 772, down to 900 × 760.
                let (size, min_size) = if self.korekore {
                    (iced::Size::new(960.0, 772.0), iced::Size::new(900.0, 760.0))
                } else {
                    (iced::Size::new(760.0, 640.0), iced::Size::new(640.0, 480.0))
                };
                let (id, open) = window::open(window::Settings {
                    size,
                    min_size: Some(min_size),
                    #[cfg(target_os = "linux")]
                    platform_specific: window::settings::PlatformSpecific {
                        application_id: "vibebuddy".to_owned(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
                self.settings = Some(id);
                // Reopened on the Conversation tab: read it again, as when the tab opens.
                let conversation = if self.tab == Tab::Conversation {
                    self.conversation_seen = None;
                    Task::done(Message::PollConversation)
                } else {
                    Task::none()
                };
                // The Agents tab reads each agent's config from disk; have it ready, whichever tab shows first.
                return Task::batch([open.discard(), Task::done(Message::RefreshAgents), conversation]);
            }
            Message::WindowClosed(id) => {
                if self.settings == Some(id) {
                    self.settings = None;
                    if let Some((_, stop)) = self.previewing.take() {
                        stop.notify_one();
                    }
                    self.notice = None;
                }
            }
            Message::Guide(page) => self.guide = page,
            Message::Tab(tab) => {
                self.tab = tab;
                // Agents can be installed or set up behind the app's back; look again whenever the tab opens.
                if tab == Tab::Agents {
                    return Task::done(Message::RefreshAgents);
                }
                // The conversation goes on on the box while the window is elsewhere: read it each time the tab opens.
                if tab == Tab::Conversation {
                    self.conversation_seen = None;
                    return Task::done(Message::PollConversation);
                }
                // Reading the day's use costs a request to the service, so only when the tab opens; the status brings it.
                if tab == Tab::Account && self.status.as_ref().is_some_and(|status| status.account.signed_in) {
                    return Task::perform(client::refresh_account(), |_| Message::AccountDone(Ok(())));
                }
            }
            Message::Resident(Ok(enabled)) => {
                self.resident = enabled;
                if !enabled && self.tab == Tab::Conversation {
                    self.tab = Tab::General;
                }
            }
            Message::Resident(Err(_)) => {}
            Message::RefreshConversation => {
                if !self.conversation_loading {
                    self.conversation_loading = true;
                    return Task::perform(client::resident_history(CONVERSATION_LIMIT), Message::Conversation);
                }
            }
            Message::Conversation(result) => {
                self.conversation_loading = false;
                self.conversation = Some(result);
            }
            Message::PollConversation => {
                if !self.conversation_polling {
                    self.conversation_polling = true;
                    return Task::perform(client::resident_info(), Message::ConversationPolled);
                }
            }
            Message::ConversationPolled(result) => {
                self.conversation_polling = false;
                match result {
                    Ok(info) => {
                        self.conversation_marked = info.revision.is_some();
                        let became_ready = self.resident_problem.is_some() && info.problem.is_none();
                        self.resident_problem = info.problem.clone();
                        self.resident_info = Some(info.clone());
                        let changed = info.revision.is_none() || info.revision != self.conversation_seen || became_ready;
                        // A read already under way is left alone; the next poll sees the change again.
                        if (changed || self.conversation.is_none()) && !self.conversation_loading {
                            self.conversation_seen = info.revision;
                            return Task::done(Message::RefreshConversation);
                        }
                    }
                    Err(client::HistoryError::DaemonDown) => {
                        // Read again once it answers: a restarted daemon counts from 0 again.
                        self.conversation_seen = None;
                        self.resident_problem = None;
                        self.conversation = Some(Err(client::HistoryError::DaemonDown));
                    }
                    Err(client::HistoryError::Unavailable(_)) => {}
                }
            }
            Message::OpenCodexInstallPage => {
                if let Err(error) = launch("xdg-open", &[std::ffi::OsStr::new(CODEX_INSTALL_PAGE)]) {
                    self.notice = Some(Err(error));
                }
            }
            Message::OpenClaudeInstallPage => {
                if let Err(error) = launch("xdg-open", &[std::ffi::OsStr::new(CLAUDE_INSTALL_PAGE)]) {
                    self.notice = Some(Err(error));
                }
            }
            Message::OpenAnthropicConsole => {
                if let Err(error) = launch("xdg-open", &[std::ffi::OsStr::new(ANTHROPIC_CONSOLE)]) {
                    self.notice = Some(Err(error));
                }
            }
            Message::PickBrain(choice) => return Task::perform(client::set_resident_brain(choice), Message::ResidentSettingSaved),
            Message::AnthropicKeyDraft(draft) => self.anthropic_key_draft = draft,
            Message::SaveAnthropicKey => {
                let key = self.anthropic_key_draft.trim().to_owned();
                if !key.is_empty() {
                    return Task::perform(client::set_anthropic_key(Some(key)), Message::ResidentSettingSaved);
                }
            }
            Message::RemoveAnthropicKey => return Task::perform(client::set_anthropic_key(None), Message::ResidentSettingSaved),
            Message::ResidentSettingSaved(result) => match result {
                Ok(info) => {
                    if info.anthropic_key_set == Some(true) {
                        self.anthropic_key_draft.clear();
                    }
                    self.resident_problem = info.problem.clone();
                    self.resident_info = Some(info);
                    self.resident_setting_error = None;
                    // Another Agent may answer now, with its own history.
                    self.conversation_seen = None;
                    return Task::done(Message::PollConversation);
                }
                Err(error) => self.resident_setting_error = Some(error),
            }
            Message::NotifyLink(enabled) => {
                let mut config = self.status.as_ref().map(|status| status.config.clone()).unwrap_or_default();
                config.notify_link = enabled;
                return Task::perform(client::put_config(config), Message::ConfigSaved);
            }
            Message::Relay(enabled) => {
                let mut config = self.status.as_ref().map(|status| status.config.clone()).unwrap_or_default();
                config.relay = enabled;
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
            Message::SetScreens(screens) => return Task::perform(client::set_screens(screens.0), Message::Done),
            Message::ChooseBox(usb_serial) => return Task::perform(client::choose_box(usb_serial), Message::Done),
            Message::Unpair(key) => return Task::perform(client::unpair(key), Message::Done),
            Message::TakeOver => return Task::perform(client::take_over(), Message::Done),
            Message::WifiSsid(ssid) => self.wifi_ssid = ssid,
            Message::WifiPassword(password) => self.wifi_password = password,
            Message::ChangeWifi => self.wifi_editing = true,
            Message::JoinWifi => {
                self.wifi_editing = false;
                let password = std::mem::take(&mut self.wifi_password);
                return Task::perform(client::set_wifi(self.wifi_ssid.clone(), password), Message::Done);
            }
            Message::ForgetWifi => return Task::perform(client::forget_wifi(), Message::Done),
            Message::ScanWifi => return Task::perform(client::scan_wifi(), Message::Done),
            Message::Identify => return Task::perform(client::identify(), Message::Done),
            Message::UseVoice(id) => {
                let form = assets::language_of(id).and_then(|language| self.characters.address(language)).map(str::to_owned);
                let write = async move {
                    let pack = assets::read_character(id, form.as_deref()).await?;
                    client::write_voice_pack(pack.build().ok_or("the Character pack doesn't fit")?, None, form.as_deref()).await
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
                if let Some(made) = self.made.iter().find(|made| made.complete && current.as_deref() == Some(made.id.as_str()))
                    && self.made_language(made) == Some(language)
                {
                    return Task::done(Message::UseMade(made.id.clone()));
                }
                let lends = self.lender.is_some_and(|lender| assets::language_of(lender.id) == Some(language));
                if current.as_deref() == Some(CUSTOM) && lends && self.custom_look.is_some() {
                    return Task::done(Message::UseCustom);
                }
            }
            Message::ChooseDrawings => return Task::perform(custom::choose_drawings(), Message::Drawings),
            Message::Drawings(Ok(None)) => {}
            Message::Drawings(Ok(Some(drawings))) => {
                match character::build_look(&drawings) {
                    Some(look) => {
                        let frames = character::look_frames(&look).unwrap_or_default();
                        self.custom_look = Some((look, frames.iter().map(|frame| handle(frame, 2)).collect()));
                        self.custom_look_written = false;
                        self.custom_problem = None;
                    }
                    None => self.custom_problem = Some(tr("No figure found: use a plain white or transparent background.", &[])),
                }
                return self.show_custom_look();
            }
            Message::Drawings(Err(error)) => {
                self.custom_problem = Some(error);
                return self.show_custom_look();
            }
            Message::PickLender(lender) => self.lender = Some(lender),
            Message::UseRobot => {
                self.characters.robot_lender = None;
                self.characters.save();
                let write = async move { client::write_voice_pack(assets::read_pack(ROBOT).await?, None, None).await };
                return Task::perform(write, Message::Done);
            }
            Message::UseCustom => {
                let (Some((look, _)), Some(lender)) = (self.custom_look.clone(), self.lender) else { return Task::none() };
                if let Some(path) = custom::custom_look_file() {
                    custom::write(&path, &look);
                }
                self.characters.custom_lender = Some(lender.id.to_owned());
                self.characters.save();
                self.custom_look_written = true;
                let form = assets::language_of(lender.id).and_then(|language| self.characters.address(language)).map(str::to_owned);
                let write = async move {
                    let pack = assets::read_character(lender.id, form.as_deref()).await?.with_look(look, CUSTOM);
                    client::write_voice_pack(pack.build().ok_or("that character can't lend its voice")?, Some(lender.id), form.as_deref()).await
                };
                return Task::perform(write, Message::Done);
            }
            Message::CopyPrompt => return iced::clipboard::write(CUSTOM_PROMPT.to_owned()),
            Message::MakeCharacter(open) => {
                self.make = None;
                self.stop_preview();
                let Some(base) = self.voices.first().map(Lender::of).filter(|_| open) else { return Task::none() };
                self.make = Some(MakeForm {
                    base,
                    name: String::new(),
                    voice: None,
                    lines: Vec::new(),
                    original: Vec::new(),
                    occasions: Vec::new(),
                    look: None,
                    problem: None,
                    starting: false,
                });
                return Task::batch([Task::done(Message::MakeBase(base)), self.refresh_made()]);
            }
            Message::MakeBase(base) => {
                let Some(form) = &mut self.make else { return Task::none() };
                (form.base, form.lines, form.occasions) = (base, Vec::new(), Vec::new());
                return Task::perform(client::preset_lines(base.id), move |lines| Message::MakeLines(base.id, lines));
            }
            Message::MakeLines(base, lines) => {
                let Some(form) = self.make.as_mut().filter(|form| form.base.id == base) else { return Task::none() };
                match lines {
                    Ok(lines) => {
                        form.occasions = lines.iter().fold(Vec::new(), |mut seen, line| {
                            if !seen.contains(&line.occasion) {
                                seen.push(line.occasion.clone());
                            }
                            seen
                        });
                        form.original = lines.clone();
                        form.lines = lines;
                        form.problem = None;
                    }
                    Err(error) => form.problem = Some(tr("Couldn't read the lines of %@: %@", &[&assets::voice_name(base), &error])),
                }
                // The preset's own voice to begin with (voices.tsv has each preset's under its id); one the user picked stays.
                if form.voice.is_none() {
                    form.voice = self.character_voices.iter().find(|voice| voice.id == base).cloned();
                }
            }
            Message::MakeName(name) => {
                if let Some(form) = &mut self.make {
                    form.name = name;
                }
            }
            Message::MakeVoice(voice) => {
                if let Some(form) = &mut self.make {
                    form.voice = Some(voice);
                }
            }
            Message::MakeVoiceAndLines(voice) => {
                let Some(form) = &mut self.make else { return Task::none() };
                let preset = self.voices.iter().find(|shipped| shipped.id == voice.id).map(Lender::of);
                let follow = form.lines == form.original && preset.is_some_and(|preset| preset != form.base);
                form.voice = Some(voice);
                if let Some(preset) = preset.filter(|_| follow) {
                    return Task::done(Message::MakeBase(preset));
                }
            }
            Message::MakeLine(index, text) => {
                if let Some(line) = self.make.as_mut().and_then(|form| form.lines.get_mut(index)) {
                    line.text = text;
                }
            }
            Message::MakeRemoveLine(index) => {
                if let Some(form) = self.make.as_mut().filter(|form| making::can_remove(index, &form.lines)) {
                    form.lines.remove(index);
                }
            }
            Message::MakeChooseDrawings => return Task::perform(custom::choose_drawings(), Message::MakeDrawings),
            Message::MakeDrawings(drawings) => {
                let Some(form) = &mut self.make else { return Task::none() };
                match drawings {
                    Ok(None) => {}
                    Ok(Some(drawings)) => match character::build_look(&drawings) {
                        Some(look) => {
                            let face = character::look_frames(&look).and_then(|frames| frames.first().map(|frame| handle(frame, 1)));
                            form.look = face.map(|face| (look, face));
                            form.problem = None;
                        }
                        None => form.problem = Some(tr("No figure found: use a plain white or transparent background.", &[])),
                    },
                    Err(error) => form.problem = Some(error),
                }
            }
            Message::MakeUseLook => {
                let Some(form) = self.make.take() else { return Task::none() };
                let Some((look, _)) = form.look else { return Task::none() };
                let frames = character::look_frames(&look).unwrap_or_default();
                self.custom_look = Some((look, frames.iter().map(|frame| handle(frame, 2)).collect()));
                self.custom_look_written = false;
                self.custom_problem = None;
                self.lender = Some(form.base);
                return Task::done(Message::UseCustom);
            }
            Message::MakeBaseLook => {
                if let Some(form) = &mut self.make {
                    form.look = None;
                }
            }
            Message::MakeStart => {
                let Some(form) = &mut self.make else { return Task::none() };
                let Some(voice) = &form.voice else { return Task::none() };
                form.starting = true;
                let request = making::Request {
                    id: None,
                    name: form.name.trim().to_owned(),
                    base: form.base.id.to_owned(),
                    voice: voice.id.clone(),
                    lines: form.lines.clone(),
                };
                return Task::perform(client::make_character(request), Message::MakeStarted);
            }
            Message::MakeStarted(Ok(started)) => {
                self.stop_preview();
                // The user's look stays with the Character's packs, so it can be written again later.
                if let Some((look, _)) = self.make.take().and_then(|form| form.look)
                    && let Some(dir) = making::directory(&started.id)
                {
                    custom::write(&dir.join("look.bin"), &look);
                }
                return self.refresh_made();
            }
            Message::MakeStarted(Err(error)) => match &mut self.make {
                Some(form) => (form.starting, form.problem) = (false, Some(error)),
                None => self.notice = Some(Err(error)),
            },
            Message::MadeCharacters(Ok(made)) => {
                let faces = made.clone();
                self.made = made;
                return Task::perform(
                    async move {
                        let mut found = Vec::new();
                        for made in faces {
                            let frame = making::look_of(&made).await.as_deref().and_then(character::look_frames).and_then(|frames| frames.into_iter().next());
                            if let Some(frame) = frame {
                                found.push((made.id, handle(&frame, 1)));
                            }
                        }
                        found
                    },
                    Message::MadeFaces,
                );
            }
            Message::MadeCharacters(Err(_)) => {}
            Message::MadeFaces(faces) => self.made_faces = faces,
            Message::CharacterVoices(Ok(voices)) => {
                if let Some(form) = self.make.as_mut().filter(|form| form.voice.is_none()) {
                    form.voice = voices.iter().find(|voice| voice.id == form.base.id).cloned();
                }
                self.character_voices = voices;
            }
            Message::CharacterVoices(Err(_)) => {}
            Message::UseMade(id) => {
                let Some(made) = self.made.iter().find(|made| made.id == id).cloned() else { return Task::none() };
                let form = self.made_language(&made).and_then(|language| self.characters.address(language)).map(str::to_owned);
                let write = async move {
                    let pack = making::pack_of(&made, form.as_deref()).await?;
                    client::write_voice_pack(pack, None, form.as_deref()).await
                };
                return Task::perform(write, Message::Done);
            }
            Message::AskDeleteMade(id) => self.confirm_delete_made = id,
            Message::AskDeleteCustom(asking) => self.confirm_delete_custom = asking,
            // A box wearing it goes back to the Character that lent it voice and lines.
            Message::DeleteCustom => {
                self.confirm_delete_custom = false;
                if let Some(path) = custom::custom_look_file() {
                    let _ = std::fs::remove_file(path);
                }
                self.custom_look = None;
                self.custom_look_written = false;
                self.custom_problem = None;
                let lender = self.characters.custom_lender.take().and_then(|id| self.voices.iter().find(|voice| voice.id == id).map(|voice| voice.id));
                self.characters.save();
                let current = self.status.as_ref().and_then(|status| status.device.voice.clone());
                if let Some(lender) = lender.filter(|_| current.as_deref() == Some(CUSTOM)) {
                    return Task::done(Message::UseVoice(lender));
                }
            }
            Message::DeleteMade(id) => {
                self.confirm_delete_made = None;
                return Task::perform(client::delete_character(id), Message::MadeDeleted);
            }
            Message::MadeDeleted(result) => {
                if let Err(error) = result {
                    self.notice = Some(Err(error));
                }
                return self.refresh_made();
            }
            Message::ResumeMade(id) => match making::manifest(&id) {
                Ok(request) => return Task::perform(client::make_character(request), Message::MakeStarted),
                Err(error) => self.notice = Some(Err(error)),
            },
            Message::AskFirmwareUpdate(asking) => self.confirm_firmware = asking,
            Message::FlashFirmware => {
                self.confirm_firmware = false;
                if let Some(directory) = self.updates().and_then(status::Updates::firmware_directory) {
                    return Task::perform(client::flash_firmware(directory.to_path_buf()), Message::Done);
                }
            }
            Message::FlashFromFile => return Task::perform(firmware_file::pick(), Message::FirmwareFile),
            Message::FirmwareFile(Ok(package)) => self.file_firmware = package,
            Message::FirmwareFile(Err(error)) => self.notice = Some(Err(error)),
            Message::FlashFile(flash) => {
                if let Some(package) = self.file_firmware.take().filter(|_| flash) {
                    return Task::perform(client::flash_firmware(package.directory), Message::Done);
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
            Message::CheckFromMenu => {
                self.tab = Tab::General;
                return Task::batch([Task::done(Message::OpenSettings), Task::done(Message::CheckForUpdatesNow)]);
            }
            Message::StartDaemon => {
                let args = ["--user", "restart", "vibebuddyd"].map(std::ffi::OsStr::new);
                if let Err(error) = launch("systemctl", &args) {
                    self.notice = Some(Err(error));
                }
            }
            Message::LaunchAtLogin(enabled) => {
                match set_launch_at_login(enabled) {
                    Ok(()) => self.launch_at_login = enabled,
                    Err(error) => self.notice = Some(Err(tr("Couldn't change Launch at login: %@", &[&error]))),
                }
            }
            Message::TogglePreview(id) => {
                let playing = self.previewing.take();
                if let Some((_, stop)) = &playing {
                    stop.notify_one();
                }
                if playing.is_some_and(|(playing, _)| playing == id) {
                    return Task::none();
                }
                let stop = std::sync::Arc::new(tokio::sync::Notify::new());
                self.previewing = Some((id.to_owned(), stop.clone()));
                let play = async move {
                    let pack = assets::read_pack(id).await?;
                    let pcm = preview::pcm(&pack).ok_or_else(|| format!("{id} has no lines to preview"))?;
                    preview::play(pcm, stop).await
                };
                return Task::perform(play, move |result| Message::PreviewDone(id.to_owned(), result));
            }
            Message::PreviewDone(id, result) => {
                if self.previewing.as_ref().is_some_and(|(playing, _)| *playing == id) {
                    self.previewing = None;
                }
                if let Err(error) = result {
                    self.notice = Some(Err(error));
                }
            }
            Message::RefreshAgents => return Task::perform(agents::status(), Message::Agents),
            Message::Agents(Ok(setups)) => self.agents = setups,
            Message::Agents(Err(error)) => self.notice = Some(Err(error)),
            Message::PlanHooks(agent, installing) => return Task::perform(agents::plan(agent, installing), Message::HookPlan),
            Message::HookPlan(Ok(plan)) => self.hook_plan = Some(plan),
            Message::HookPlan(Err(error)) => self.notice = Some(Err(error)),
            Message::ApplyHooks(write) => {
                if let Some(plan) = self.hook_plan.take().filter(|_| write) {
                    return Task::perform(agents::apply(plan), Message::HooksWritten);
                }
            }
            Message::HooksWritten(result) => {
                self.notice = Some(result);
                return Task::done(Message::RefreshAgents);
            }
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
            Message::SignIn => return Task::perform(client::sign_in(), Message::SignInUrl),
            Message::SignInUrl(result) => {
                match result.and_then(|url| launch("xdg-open", &[std::ffi::OsStr::new(&url)])) {
                    Ok(()) => self.signing_in = true,
                    Err(error) => {
                        self.signing_in = false;
                        self.notice = Some(Err(tr("Couldn't sign in: %@", &[&error])));
                    }
                }
            }
            Message::SignOut => {
                return Task::perform(client::sign_out(), |result| {
                    Message::AccountDone(result.map_err(|error| tr("Couldn't sign out: %@", &[&error])))
                });
            }
            Message::AskDeleteAccount(asking) => self.confirm_delete_account = asking,
            Message::DeleteAccount => {
                self.confirm_delete_account = false;
                return Task::perform(client::delete_account(), |result| {
                    Message::AccountDone(result.map_err(|error| tr("Couldn't delete the account: %@", &[&error])))
                });
            }
            Message::AccountDone(result) => {
                if let Err(error) = result {
                    self.notice = Some(Err(error));
                }
            }
            Message::OwnKeyDraft(provider, draft) => self.own_key_drafts[provider as usize] = draft,
            Message::SaveOwnKey(provider) => {
                let key = self.own_key_drafts[provider as usize].trim().to_owned();
                if !key.is_empty() {
                    return Task::perform(client::set_voice_key(provider, Some(key)), move |result| Message::OwnKeySaved(provider, result));
                }
            }
            Message::ClearOwnKey(provider) => {
                return Task::perform(client::set_voice_key(provider, None), move |result| Message::OwnKeySaved(provider, result));
            }
            Message::OpenOwnKeyPage(provider) => {
                let page = match provider {
                    client::KeyProvider::ElevenLabs => ELEVENLABS_KEYS,
                    client::KeyProvider::Volc => VOLC_CONSOLE,
                };
                if let Err(error) = launch("xdg-open", &[std::ffi::OsStr::new(page)]) {
                    self.notice = Some(Err(error));
                }
            }
            Message::OwnKeySaved(provider, result) => match result {
                Ok(()) => self.own_key_drafts[provider as usize].clear(),
                Err(error) => self.notice = Some(Err(tr("Couldn't save your own key: %@", &[&error]))),
            },
            Message::Done(result) => {
                // The box answers a volume change through the status stream; drop the dragged value then.
                self.volume = None;
                if let Err(error) = result {
                    self.notice = Some(Err(error));
                }
            }
            Message::Quit => return iced::exit(),
            // The korekore sections are the tabs, so what a tab does on opening happens here too.
            Message::Korekore(section) => return Task::done(Message::Tab(section.tab())),
        }
        Task::none()
    }

    /// Pushes the menu and the face's color to the tray; a no-op until the tray is up.
    /// Asks for the Characters made here, and the voices once.
    /// The korekore Character page opens the look's card below the grid; go down to it.
    fn show_custom_look(&self) -> Task<Message> {
        if self.korekore { iced::widget::operation::snap_to_end(korekore::pages::CHARACTER_PAGE) } else { Task::none() }
    }

    fn refresh_made(&self) -> Task<Message> {
        let voices = if self.character_voices.is_empty() {
            Task::perform(client::character_voices(), Message::CharacterVoices)
        } else {
            Task::none()
        };
        Task::batch([Task::perform(client::made_characters(), Message::MadeCharacters), voices])
    }

    /// Stops a preview playing here, as the form whose voices it was played from closes.
    fn stop_preview(&mut self) {
        if let Some((_, stop)) = self.previewing.take() {
            stop.notify_one();
        }
    }

    /// The language a made Character speaks: its voice's, else its base preset's.
    fn made_language(&self, made: &making::MadeCharacter) -> Option<assets::Language> {
        let voice = self.character_voices.iter().find(|voice| voice.id == made.voice);
        match voice.map(|voice| voice.language.as_str()) {
            Some("zh") => Some(assets::Language::Chinese),
            Some("en") => Some(assets::Language::English),
            _ => assets::language_of(&made.base),
        }
    }

    /// The language of the Character the box wears, the one whose form of address it says; None for the robot, an
    /// unknown Character or no box.
    fn box_language(&self) -> Option<assets::Language> {
        let current = self.status.as_ref()?.device.voice.as_deref()?;
        if let Some(language) = assets::language_of(current) {
            return Some(language);
        }
        if let Some(made) = self.made.iter().find(|made| made.complete && made.id == current) {
            return self.made_language(made);
        }
        if current == CUSTOM {
            return self.characters.custom_lender.as_deref().and_then(assets::language_of);
        }
        None
    }

    fn refresh_tray(&self) -> Task<Message> {
        #[cfg(target_os = "linux")]
        if let Some(TrayHandle(handle)) = self.tray.clone() {
            let menu = MenuState::derive(self.status.as_ref());
            let kore = self.korekore.then(|| korekore::picture::BoxPicture::derive(self.status.as_ref()));
            let [r, g, b, _] = self.theme.palette().text.into_rgba8();
            return Task::future(async move {
                handle
                    .update(move |tray| {
                        tray.menu = menu;
                        tray.color = [r, g, b];
                        tray.kore = kore;
                    })
                    .await;
            })
            .discard();
        }
        Task::none()
    }

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        if self.korekore {
            if let Some(page) = self.guide {
                return self.kk_guide(page);
            }
            return self.korekore(korekore::Section::of(self.tab));
        }
        if let Some(page) = self.guide {
            return self.guide(page);
        }
        let tabs = [
            (Tab::General, tr("General", &[])),
            (Tab::Sound, tr("Character", &[])),
            (Tab::Agents, tr("Agents", &[])),
            (Tab::Device, tr("Device", &[])),
            (Tab::Conversation, tr("Conversation", &[])),
            (Tab::Account, tr("Account", &[])),
            (Tab::Advanced, tr("Advanced", &[])),
        ];
        // Voice commands are experimental (VIBEBUDDY_RESIDENT_SESSION=1): no tab while the daemon has them off.
        let tabs = tabs.into_iter().filter(|(tab, _)| *tab != Tab::Conversation || self.resident);
        let tab_bar = row(tabs.map(|(tab, label)| {
            let style = if tab == self.tab { button::primary } else { button::text };
            button(text(label)).style(style).on_press(Message::Tab(tab)).into()
        }))
        .spacing(4);
        let body = match self.tab {
            Tab::General => self.general(),
            Tab::Sound => self.sound(),
            Tab::Agents => self.agents(),
            Tab::Device => self.device(),
            Tab::Conversation => self.conversation(),
            Tab::Account => self.account(),
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

    /// The korekore settings window (ADR-0021); its pages come in the next steps.
    fn korekore(&self, section: korekore::Section) -> Element<'_, Message> {
        // Voice commands are experimental: no Conversation section while the daemon has them off.
        let sections: Vec<korekore::Section> = korekore::Section::ALL
            .into_iter()
            .filter(|&section| section != korekore::Section::Conversation || self.resident)
            .collect();
        let picture = korekore::picture::BoxPicture::derive(self.status.as_ref());
        korekore::view::settings(&picture, &sections, section, env!("CARGO_PKG_VERSION"), Message::Korekore, self.korekore_page(section))
    }

    /// First run on Linux: what the install script couldn't do. It already connected the agents and set up the start
    /// at login, so the guide is the Mac's without those pages: find the box, pick a character, put the box on Wi-Fi.
    fn guide(&self, page: usize) -> Element<'_, Message> {
        const LAST: usize = 4;
        let titles = [tr("Welcome", &[]), tr("Find the box", &[]), tr("Pick a character", &[]), tr("Wi-Fi", &[]), tr("Done", &[])];
        let steps = row(titles.into_iter().enumerate().map(|(index, title)| {
            let title = text(title).size(13);
            if index == page { title.into() } else { title.style(text::secondary).into() }
        }))
        .spacing(16);
        let device = self.status.as_ref().map(|status| &status.device);
        let online = device.is_some_and(|device| device.connected);
        // No paging while flashing: once you page away, nobody is watching the progress.
        let flashing = self.status.as_ref().and_then(|status| status.operation.as_ref()).is_some_and(|operation| {
            operation.kind == status::OperationKind::Firmware && operation.running()
        });
        let body: Element<'_, Message> = match page {
            0 => column![
                text(tr("VibeBuddy turns what your coding agents are doing into pictures and sounds on the box.", &[])).size(18),
                text(tr(
                    "The install script has connected your agents and set VibeBuddy to start when you log in. Next: find the box, pick a character, and put the box on Wi-Fi. Everything can be changed or skipped.",
                    &[]
                )),
            ]
            .spacing(10)
            .into(),
            1 => scrollable(
                column![text(tr("Connect the box to this computer with a USB cable.", &[])).size(18), self.box_section()]
                    .push(online.then(|| {
                        row![
                            button(text(tr("Make it blink", &[]))).on_press(Message::Identify),
                            text(tr("If the backlight flashes, that's the one.", &[])).size(13),
                        ]
                        .spacing(12)
                        .align_y(iced::Alignment::Center)
                    }))
                    .spacing(12)
                    .padding(iced::Padding::ZERO.right(16)),
            )
            .into(),
            2 => column![text(tr("Pick a character.", &[])).size(18), self.sound()].spacing(12).into(),
            3 => scrollable(
                column![
                    text(tr("Put the box on Wi-Fi?", &[])).size(18),
                    text(tr(
                        "Then this computer reaches the box without the cable: over the local network, or through the VibeBuddy Relay when you're elsewhere.",
                        &[]
                    )),
                ]
                .push(match self.wifi_section() {
                    Some(wifi) if !device.is_some_and(|device| device.network) => Element::from(wifi),
                    _ => text(tr("Plug in the box to set its network. You can also do it later on the Device tab in Settings.", &[])).size(13).into(),
                })
                .spacing(12)
                    .padding(iced::Padding::ZERO.right(16)),
            )
            .into(),
            _ => column![
                text(tr("All set.", &[])).size(18),
                text(tr(
                    "VibeBuddy lives in the tray, or in Omarchy's bar: the buddy's face, with its eyes open while the box is online. To change anything, open Settings from there, or run VibeBuddy again.",
                    &[]
                )),
            ]
            .spacing(10)
            .into(),
        };
        let buttons = row![]
            .push((page > 0 && page < LAST).then(|| button(text(tr("Back", &[]))).style(button::secondary).on_press(Message::Guide(Some(page - 1)))))
            .push(space::horizontal())
            .push((page > 0 && page < LAST).then(|| {
                button(text(tr("Skip", &[]))).style(button::secondary).on_press_maybe((!flashing).then_some(Message::Guide(Some(page + 1))))
            }))
            .push(if page < LAST {
                button(text(tr("Next", &[]))).on_press_maybe((!flashing).then_some(Message::Guide(Some(page + 1))))
            } else {
                button(text(tr("Get started", &[]))).on_press(Message::Guide(None))
            })
            .spacing(8);
        let notice = self.notice.as_ref().map(|notice| match notice {
            Ok(message) => text(message.clone()).size(13),
            Err(error) => text(error.clone()).size(13).style(text::danger),
        });
        let content = column![steps, container(body).height(Length::Fill)].push(notice).push(buttons).spacing(16).padding(20);
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
                (None, Some(app), _) => tr("VibeBuddy %@ is available", &[&app.version]),
                (None, None, Some(checked)) => tr("Up to date · checked %@", &[&short_time(checked)]),
                (None, None, None) => tr("Not checked yet", &[]),
            },
            _ => tr("Off", &[]),
        };
        // No self-update here: a new release is installed by rerunning its install.sh, as the README says.
        let upgrade = updates.and_then(|updates| updates.app.as_ref()).map(|_| {
            text(tr("To upgrade, download the new release and run its install.sh again.", &[])).size(13)
        });
        let unsupported = updates.filter(|updates| updates.unsupported_app).map(|_| {
            text(tr("This version of VibeBuddy is no longer supported. Update it to keep getting firmware for the box.", &[]))
                .style(text::warning)
        });
        column![]
            .push(unsupported)
            .push(text(menu.device_line))
            .push(text(menu.mode_line))
            .push(text(menu.today_line))
            .push(space().height(8))
            .push(toggler(self.launch_at_login).label(tr("Launch at login", &[])).on_toggle(Message::LaunchAtLogin))
            .push(
                toggler(notify)
                    .label(tr("Notify me when the box disconnects or the daemon fails", &[]))
                    .on_toggle_maybe(self.status.is_some().then_some(Message::NotifyLink)),
            )
            // ADR-0012: the Relay carries the box's messages through VibeBuddy's server, so it can be turned off.
            .push(
                toggler(self.status.as_ref().is_none_or(|status| status.config.relay))
                    .label(tr("Reach the box through the VibeBuddy Relay when it's away from this computer", &[]))
                    .on_toggle_maybe(self.status.is_some().then_some(Message::Relay)),
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
            .push(row![text("App").width(140), text(env!("CARGO_PKG_VERSION"))].spacing(12))
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
        column![text(tr("Restart VibeBuddy to change the language?", &[]))]
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
        if let Some(form) = &self.make {
            return self.make_form(form);
        }
        let box_volume = self.status.as_ref().and_then(|status| status.device.volume);
        let level = self.volume.or(box_volume).unwrap_or(60);
        let online = self.status.as_ref().is_some_and(|status| status.device.connected);
        let operation = self.status.as_ref().and_then(|status| status.operation.as_ref());
        let busy = operation.is_some_and(status::Operation::running);
        let mut volume = slider(20..=100, level, Message::VolumeDragged).step(5u8);
        if online && !busy {
            volume = volume.on_release(Message::VolumeReleased);
        }
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
            let card = row![self.preview_button(voice.id, busy)]
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
        let robot_card = row![self.preview_button(ROBOT, busy)]
            .push(self.robot_face.clone().map(image))
            .push(column![
                row![text("VibeBuddy"), text(tr("Default", &[])).size(13).style(text::primary)]
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
        let job = self.status.as_ref().and_then(|status| status.making.as_ref());
        let job_running = job.is_some_and(making::Making::running);
        let made_cards = self.made.iter().map(|made| {
            let in_use = current.as_deref() == Some(made.id.as_str());
            let action: Element<'_, Message> = if in_use {
                text(tr("In use", &[])).style(text::success).into()
            } else if made.complete {
                button(text(tr("Use", &[]))).on_press_maybe((online && !busy).then_some(Message::UseMade(made.id.clone()))).into()
            } else if job.is_some_and(|job| job.running() && job.id == made.id) {
                text(tr("Making…", &[])).size(13).into()
            } else {
                button(text(tr("Resume", &[]))).on_press_maybe((!job_running).then_some(Message::ResumeMade(made.id.clone()))).into()
            };
            let base = assets::voice_name(&made.base);
            let voice = self.character_voices.iter().find(|voice| voice.id == made.voice).map_or_else(|| made.voice.clone(), |voice| voice.name.clone());
            let detail = if made.complete {
                tr("Made from %@ · voice: %@", &[&base, &voice])
            } else {
                tr("Not finished · made from %@ · voice: %@", &[&base, &voice])
            };
            let face = self.made_faces.iter().find(|(id, _)| *id == made.id).map(|(_, face)| image(face.clone()));
            let deletable = !in_use && !job.is_some_and(|job| job.running() && job.id == made.id);
            let delete: Element<'_, Message> = if self.confirm_delete_made.as_deref() == Some(made.id.as_str()) {
                row![
                    text(tr("Delete “%@”? This can't be undone.", &[&made.name])).size(13),
                    button(text(tr("Delete", &[]))).style(button::danger).on_press(Message::DeleteMade(made.id.clone())),
                    button(text(tr("Cancel", &[]))).style(button::secondary).on_press(Message::AskDeleteMade(None)),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .into()
            } else {
                button(text(tr("Delete…", &[]))).style(button::secondary).on_press_maybe(deletable.then(|| Message::AskDeleteMade(Some(made.id.clone())))).into()
            };
            let hint = in_use.then(|| text(tr("It's on the box: put another character on the box to delete this one.", &[])).size(13));
            let card = row![]
                .push(face)
                .push(column![text(made.name.clone()), text(detail).size(13)].push(hint))
                .push(space::horizontal())
                .push(delete)
                .push(action)
                .spacing(12)
                .align_y(iced::Alignment::Center);
            character_card(card.into(), in_use)
        });
        let make_button = button(text(tr("Make a Character…", &[])))
            .style(button::secondary)
            .on_press_maybe((self.status.is_some() && !job_running).then_some(Message::MakeCharacter(true)));
        let job_row = job.and_then(|job| self.making_row(job, online && !busy));
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
            button(text(tr("Play a line on the box", &[]))).on_press_maybe((online && !busy).then_some(Message::PlayLine)),
            text(tr("Saved on the box and kept across restarts. To mute, long-press K2 on the box.", &[])).size(13),
            space().height(8),
            text(tr("Character", &[])).size(18),
            text(tr(
                "The box is speaking as “%@”. Each character has its own voice and lines. Preview one, then click Use to write it to the box — no firmware flash needed. Over the UART port this takes a few minutes; when it's done the box says a line as the new character.",
                &[&using]
            ))
            .size(13),
            address,
            character_card(robot_card.into(), robot_in_use),
            column(cards).spacing(10),
            character_card(custom_card.into(), custom_in_use),
            column(made_cards).spacing(10),
            make_button,
        ]
        .push(job_row)
        .push(progress)
        .spacing(12);
        scrollable(page).into()
    }

    /// The Character being made, or the last one: its progress, why it stopped, or that it is ready for the box.
    fn making_row(&self, job: &making::Making, can_write: bool) -> Option<Element<'_, Message>> {
        let made = self.made.iter().find(|made| made.id == job.id);
        if job.running() {
            let label = tr("Making “%@”… %lld of %lld", &[&job.name, &job.done, &job.total]);
            return Some(row![progress_bar(0.0..=1.0, job.progress()).length(200), text(label).size(13)].spacing(12).align_y(iced::Alignment::Center).into());
        }
        let provider = made
            .and_then(|made| self.character_voices.iter().find(|voice| voice.id == made.voice))
            .map_or_else(|| tr("the voice's provider", &[]), making::Voice::provider_name);
        if let Some(why) = job.explanation(&provider) {
            let resume = made.map(|made| button(text(tr("Resume", &[]))).on_press(Message::ResumeMade(made.id.clone())));
            return Some(row![text(why).size(13).style(text::danger).width(Length::Fill)].push(resume).spacing(12).align_y(iced::Alignment::Center).into());
        }
        let current = self.status.as_ref().and_then(|status| status.device.voice.as_deref());
        let made = made.filter(|made| made.complete && current != Some(made.id.as_str()))?;
        Some(
            row![
                text(tr("“%@” is made.", &[&made.name])).size(13).style(text::success),
                space::horizontal(),
                button(text(tr("Write to box", &[]))).on_press_maybe(can_write.then_some(Message::UseMade(made.id.clone()))),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center)
            .into(),
        )
    }

    /// Making a Character, in place of the Character tab: start from a preset, change its lines (occasions stay, each
    /// keeps a line), pick a voice and, optionally, bring a look; the daemon then makes the line audio.
    fn make_form<'a>(&'a self, form: &'a MakeForm) -> Element<'a, Message> {
        let bases: Vec<Lender> = self.voices.iter().map(Lender::of).collect();
        let voices = making::sorted(&self.character_voices, assets::language_of(form.base.id).map(assets::Language::code));
        let keys = self.status.as_ref().and_then(|status| status.voice_keys.as_ref());
        let account = self.status.as_ref().map(|status| status.account.clone()).unwrap_or_default();
        let readiness = form.voice.as_ref().map(|voice| (making::Readiness::of(voice, keys, &account), voice.provider_name()));
        let job_running = self.status.as_ref().and_then(|status| status.making.as_ref()).is_some_and(making::Making::running);
        let problem = form.problem.clone().or_else(|| if form.lines.is_empty() { None } else { making::problem(&form.name, &form.lines, &form.occasions) });
        let can_make = !form.starting && !form.lines.is_empty() && problem.is_none() && readiness.as_ref().is_some_and(|(ready, _)| ready.can_start()) && !job_running;
        let base_face = self.voices.iter().position(|voice| voice.id == form.base.id).and_then(|index| self.faces.get(index).cloned().flatten());
        let face = form.look.as_ref().map(|(_, face)| face.clone()).or(base_face);
        let readiness = readiness.map(|(ready, provider)| {
            let note = text(ready.explanation(&provider)).size(13);
            if ready.can_start() { note.style(text::secondary) } else { note.style(text::danger) }
        });
        // Rows rather than a list that drops down, so each voice has ▶: the preview of the preset it comes from, when
        // this App has it.
        let busy = self.status.as_ref().and_then(|status| status.operation.as_ref()).is_some_and(status::Operation::running);
        let selected = voices.iter().position(|voice| Some(voice) == form.voice.as_ref());
        let mut voice_rows = column![].spacing(4);
        for (index, voice) in voices.into_iter().enumerate() {
            let preview = making::preview_of(&voice.id);
            let preview = if preview == ROBOT { Some(ROBOT) } else { self.voices.iter().find(|shipped| shipped.id == preview).map(|shipped| shipped.id) };
            let choice = radio(voice.to_string(), index, selected, move |_| Message::MakeVoice(voice));
            voice_rows = voice_rows.push(row![].push(preview.map(|id| self.preview_button(id, busy))).push(choice).spacing(8).align_y(iced::Alignment::Center));
        }
        let mut lines = column![].spacing(6);
        for occasion in &form.occasions {
            lines = lines.push(text(making::occasion_name(occasion)).size(14));
            for (index, line) in form.lines.iter().enumerate().filter(|(_, line)| &line.occasion == occasion) {
                lines = lines.push(
                    row![
                        text_input("", &line.text).on_input(move |text| Message::MakeLine(index, text)),
                        button(text("−")).style(button::secondary).on_press_maybe(making::can_remove(index, &form.lines).then_some(Message::MakeRemoveLine(index))),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                );
            }
        }
        let look = row![text(tr("Look", &[]))]
            .push(face.map(image))
            .push(button(text(tr("Choose images…", &[]))).style(button::secondary).on_press(Message::MakeChooseDrawings))
            .push(form.look.is_some().then(|| {
                button(text(tr("Use %@'s look", &[&assets::voice_name(form.base.id)]))).style(button::secondary).on_press(Message::MakeBaseLook)
            }))
            .spacing(8)
            .align_y(iced::Alignment::Center);
        let page = column![
            text(tr("Make a Character", &[])).size(18),
            text(tr("Start from a character, change what it says, and pick a voice. VibeBuddy makes each line in that voice, then you can write it to the box.", &[])).size(13),
            row![text(tr("Start from", &[])), pick_list(bases, Some(form.base), Message::MakeBase)].spacing(8).align_y(iced::Alignment::Center),
            row![text(tr("Name", &[])), text_input(&tr("Name", &[]), &form.name).on_input(Message::MakeName).width(320)]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            row![text(tr("Voice", &[])), voice_rows].spacing(8),
        ]
        .push(readiness)
        .push(look)
        .push(lines)
        .push(text(tr("{address} is where the buddy says what it calls you; each line with it is made once with each form of address.", &[])).size(13))
        .push(problem.map(|problem| text(problem).size(13).style(text::danger)))
        .push(
            row![
                space::horizontal(),
                button(text(tr("Cancel", &[]))).style(button::secondary).on_press(Message::MakeCharacter(false)),
                button(text(tr("Make", &[]))).on_press_maybe(can_make.then_some(Message::MakeStart)),
            ]
            .spacing(8),
        )
        .spacing(12);
        scrollable(page).into()
    }

    /// ▶ plays a Character's lines here, ■ stops them; not while the box is being written, as on the Mac.
    fn preview_button(&self, id: &'static str, busy: bool) -> Element<'_, Message> {
        let playing = self.previewing.as_ref().is_some_and(|(playing, _)| playing == id);
        button(text(if playing { "■" } else { "▶" }))
            .style(button::secondary)
            .on_press_maybe((!busy).then_some(Message::TogglePreview(id)))
            .into()
    }

    fn agents(&self) -> Element<'_, Message> {
        let hooks = self.status.as_ref().map(|status| &status.hooks);
        let row_for = |setup: &agents::Setup| -> Element<'_, Message> {
            let last = hooks.and_then(|hooks| match setup.agent.as_str() {
                "claude" => hooks.claude.as_deref(),
                "codex" => hooks.codex.as_deref(),
                "opencode" => hooks.opencode.as_deref(),
                "copilot" => hooks.copilot.as_deref(),
                "pi" => hooks.pi.as_deref(),
                _ => None,
            });
            // Only Codex has a trust step; the others pick up config changes in their next session.
            let codex = (setup.agent == "codex" && setup.installed)
                .then(|| agents::codex_trust(setup.modified.as_deref(), last));
            let (dot, state) = if !setup.present {
                (DOT_OFF, tr("%@ wasn't found on this computer", &[&setup.name]))
            } else if !setup.installed {
                (DOT_OFF, tr("Not set up", &[]))
            } else if let Some(agents::CodexTrust::ChangedSinceLastEvent(changed)) = &codex {
                (DOT_BAD, tr(
                    "The config changed at %@ and no Codex event has arrived since. Codex silently disables hooks that change: type /hooks in Codex and re-trust VibeBuddy's six hooks.",
                    &[&short_time(changed)],
                ))
            } else if let Some(time) = last {
                (DOT_GOOD, tr("Last event %@", &[&short_time(time)]))
            } else if setup.agent == "codex" {
                (DOT_WAIT, tr("Waiting for the first event… (Codex only runs this config after you trust it in /hooks)", &[]))
            } else {
                (DOT_WAIT, tr("Waiting for the first event…", &[]))
            };
            let id = setup.agent.clone();
            let actions: Element<'_, Message> = if setup.installed {
                row![
                    button(text(tr("Repair", &[]))).on_press(Message::PlanHooks(id.clone(), true)),
                    button(text(tr("Remove", &[]))).style(button::secondary).on_press(Message::PlanHooks(id, false)),
                ]
                .spacing(8)
                .into()
            } else {
                button(text(tr("Connect", &[]))).on_press_maybe(setup.present.then_some(Message::PlanHooks(id, true))).into()
            };
            container(
                row![
                    text("●").color(dot),
                    column![text(setup.name.clone()), text(state).size(13)].spacing(2).width(Length::Fill),
                    actions,
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center),
            )
            .padding(10)
            .style(container::rounded_box)
            .into()
        };
        // Before anything is written, what it would change, as the Mac's sheet shows it.
        let plan = self.hook_plan.as_ref().map(|plan| {
            let lines: Element<'_, Message> = if plan.lines.is_empty() {
                text(tr("Nothing to change.", &[])).size(13).into()
            } else {
                scrollable(column(plan.lines.iter().map(|line| {
                    let color = if line.starts_with('+') { DOT_GOOD } else { DOT_BAD };
                    text(line.clone()).size(12).font(Font::MONOSPACE).color(color).into()
                })))
                .height(Length::Fixed(200.0))
                .into()
            };
            container(
                column![text(tr("Will change %@", &[&plan.config.display()])), lines]
                    .push((plan.agent == "codex").then(|| {
                        text(tr(
                            "After writing, open /hooks in Codex to review and trust this config — the app can't do that for you.",
                            &[],
                        ))
                        .size(13)
                    }))
                    .push(
                        row![
                            button(text(tr("Write", &[]))).on_press_maybe((!plan.lines.is_empty()).then_some(Message::ApplyHooks(true))),
                            button(text(tr("Cancel", &[]))).style(button::secondary).on_press(Message::ApplyHooks(false)),
                        ]
                        .spacing(8),
                    )
                    .spacing(8),
            )
            .padding(10)
            .style(container::bordered_box)
        });
        let page = column![
            text(tr("Connect agents", &[])).size(18),
            text(tr(
                "VibeBuddy only forwards session IDs, event names and working directories — never prompts or replies.",
                &[]
            ))
            .size(13),
            text(tr(
                "This writes user-level config, so one install covers both the desktop app's sessions and the same agent running in a terminal.",
                &[]
            ))
            .size(13),
        ]
        .push(plan)
        .push(column(self.agents.iter().map(row_for)).spacing(8))
        .spacing(12);
        scrollable(page).into()
    }

    /// Where the box is and what it runs: the link, help finding it, and firmware to flash. The Device tab and the
    /// first-run guide's Find the box page both show it.
    fn box_section(&self) -> iced::widget::Column<'_, Message> {
        let device = self.status.as_ref().map(|status| &status.device);
        let link = match device {
            None => tr("daemon isn't running", &[]),
            Some(device) if !device.connected && !device.candidates.is_empty() => tr("Several devices found", &[]),
            Some(device) if !device.connected => tr("Box not found", &[]),
            Some(device) if device.relay => tr("Relay", &[]),
            Some(device) => {
                let kind = match (device.network, device.bridge) {
                    (true, _) => tr("Wi-Fi", &[]),
                    (false, true) => tr("UART bridge", &[]),
                    (false, false) => tr("native USB", &[]),
                };
                format!("{} · {kind}", device.port.as_deref().unwrap_or("?"))
            }
        };
        let firmware = device.and_then(|device| device.firmware_label()).unwrap_or_else(|| "—".to_owned());
        let online = device.is_some_and(|device| device.connected);
        // Firmware that reports TRANSFER 1 is updated over whatever carries the link (ADR-0016); older firmware is
        // flashed over the cable only, and over Wi-Fi or the Relay the daemon refuses it.
        let can_flash = online && (!device.is_some_and(|device| device.network) || device.is_some_and(|device| device.transfer));
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
                _ => tr("Update checks are off, so VibeBuddy can't download firmware.", &[]),
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
            confirm_flash(title, detail, action, (!busy).then_some(Message::FlashFirmware), Message::AskFirmwareUpdate(false))
        };
        // A package from a file is confirmed the same way, with what it holds next to what the box runs.
        let from_file = self.file_firmware.as_ref().map(|package| {
            let current = device.and_then(|device| device.firmware_label()).unwrap_or_else(|| tr("unknown", &[]));
            confirm_flash(
                tr("Flash this firmware?", &[]),
                tr(
                    "Firmware package: %@\nBox now: %@\nThe box restarts once; its voice pack and today's stats are kept. Over the UART bridge this takes a few minutes.",
                    &[&package.label(), &current],
                ),
                tr("Flash", &[]),
                (online && !busy).then_some(Message::FlashFile(true)),
                Message::FlashFile(false),
            )
        });
        let update: Option<Element<'_, Message>> = match (foreign, outdated && can_flash, self.confirm_firmware) {
            (true, _, true) => Some(confirm(
                tr("Flash the box with VibeBuddy?", &[]),
                tr(
                    "The box's current firmware and data will be erased and can't be recovered. It restarts on its own when done; over the UART port this takes a few minutes.",
                    &[],
                ),
                tr("Flash", &[]),
            )),
            (true, _, false) => Some(
                column![
                    text(tr("The box isn't running VibeBuddy firmware.", &[])).style(text::warning),
                    button(text(tr("Flash VibeBuddy firmware", &[])))
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
            // Firmware that updates over the link carries on from where it got to; download mode is for the rest.
            let over_the_link = device.is_some_and(|device| device.transfer);
            let failed = (operation.state == status::OperationState::Failed).then(|| {
                column![]
                    .push((!over_the_link).then(|| {
                        text(tr("Before retrying, hold K0 on the box and replug the cable to put it in download mode.", &[])).size(13)
                    }))
                    .push(button(text(tr("Retry", &[]))).on_press_maybe((can_flash && downloaded).then_some(Message::FlashFirmware)))
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
                    "Still nothing? Hold K0 on the box while you plug in the cable. The box starts in download mode with a dark screen, ready to be flashed with VibeBuddy firmware.",
                    &[]
                ))
                .size(13),
            ]
            .spacing(4)
        });
        // Another computer has the box (ADR-0011): say who, and offer to take it unless it's on their cable.
        let busy_row = device.filter(|_| !online).and_then(|device| device.busy.as_ref()).map(|busy| {
            let owner = if busy.owner.is_empty() { tr("another computer", &[]) } else { busy.owner.clone() };
            if busy.reason == "usb" {
                row![text(tr("The box is plugged into %@.", &[&owner])).size(13)]
            } else {
                row![
                    text(tr("The box is with %@.", &[&owner])).size(13),
                    space::horizontal(),
                    button(text(tr("Take over", &[]))).on_press(Message::TakeOver),
                ]
                .spacing(12)
            }
        });
        let refusal = device.filter(|_| !online).and_then(status::Device::relay_refusal).map(|refusal| text(refusal).size(13));
        column![row![text(tr("Link", &[])).width(140), text(link)].spacing(12)]
            .push(busy_row)
            .push(refusal)
            .push(pin)
            .push(choice)
            .push(not_found)
            .push(row![text(tr("Box firmware", &[])).width(140), text(firmware)].spacing(12))
            .push(row![text(tr("Latest firmware", &[])).width(140), text(latest)].spacing(12))
            .push(device.filter(|_| online).and_then(|device| device.unsupported_board.as_ref()).map(|board| {
                text(tr("This is a %@ board, which released firmware doesn't run on, so no update is offered.", &[board])).size(13)
            }))
            .push(update)
            .push((outdated && online && !can_flash).then(|| text(tr("Plug in the cable to update the box's firmware.", &[])).size(13)))
            .push(unavailable)
            .push(flashing)
            .push(from_file)
            .spacing(12)
    }

    fn device(&self) -> Element<'_, Message> {
        let device = self.status.as_ref().map(|status| &status.device);
        let online = device.is_some_and(|device| device.connected);
        let can_flash = online && (!device.is_some_and(|device| device.network) || device.is_some_and(|device| device.transfer));
        let busy = self.status.as_ref().and_then(|status| status.operation.as_ref()).is_some_and(status::Operation::running);
        // The computers the box is paired with (ADR-0012). Every computer it's plugged into pairs on its own, so this
        // one has no Remove: it would only pair again.
        let paired = device.filter(|device| online && device.box_key.is_some()).map(|device| {
            let rows = device.paired_computers.iter().map(|computer| {
                let name = if computer.name.is_empty() { tr("Unnamed computer", &[]) } else { computer.name.clone() };
                let action: Element<'_, Message> = if device.computer_key.as_ref() == Some(&computer.key) {
                    text(tr("This computer", &[])).size(13).style(text::secondary).into()
                } else {
                    button(text(tr("Remove", &[])))
                        .style(button::secondary)
                        .on_press_maybe((!busy).then_some(Message::Unpair(computer.key.clone())))
                        .into()
                };
                row![text(name).size(13), space::horizontal(), action].spacing(12).into()
            });
            // The box lists them over the cable only (ERROR cable_only otherwise), so the list would be empty.
            let unlisted = device.network.then(|| text(tr("Plug in the cable to see the paired computers.", &[])).size(13));
            column![text(tr("Paired computers", &[]))]
                .extend(rows)
                .push(unlisted)
                .push(
                    text(tr("A computer pairs with the box when it's plugged in. To forget them all, use COMPUTERS in the box's menu.", &[]))
                        .size(13),
                )
                .spacing(6)
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
        // The tab outgrew the window: it scrolls, like the Character tab.
        scrollable(
            self.box_section()
                .push(self.wifi_section())
                .push(paired)
                .push(column![
                    // Separately distributed firmware (VibeBuddy-firmware-*.zip on Releases) comes in here.
                    row![
                        button(text(tr("Flash from file…", &[])))
                            .on_press_maybe((can_flash && !busy).then_some(Message::FlashFromFile)),
                        button(text(tr("Make the box blink", &[])))
                            .on_press_maybe((online && !busy).then_some(Message::Identify)),
                    ]
                    .spacing(8),
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
                    // A fixed height: the tab scrolls, and a scrolled column has no height left to fill.
                    container(screen)
                        .padding(4)
                        .center_x(Length::Fill)
                        .center_y(Length::Fixed(260.0))
                        .style(|_| container::background(iced::Color::from_rgb8(0x16, 0x16, 0x16))),
                ]
                .spacing(12))
                .spacing(12)
                // Room for the scroll bar, which otherwise covers the ends of the lines.
                .padding(iced::Padding::ZERO.right(16)),
        )
        .into()
    }

    /// The Wi-Fi network the box joins; None until a box that can pair is on the cable or the network.
    fn wifi_section(&self) -> Option<iced::widget::Column<'_, Message>> {
        let device = self.status.as_ref().map(|status| &status.device);
        let online = device.is_some_and(|device| device.connected);
        let busy = self.status.as_ref().and_then(|status| status.operation.as_ref()).is_some_and(status::Operation::running);
        // The Wi-Fi network the box joins, so it can be reached without the cable (ADR-0012). Set over the cable only;
        // the password goes to the box and is kept nowhere else, so it is never shown again.
        device.filter(|device| online && device.box_key.is_some()).map(|device| {
            let network = device.wifi_network.clone().unwrap_or_else(|| tr("Not set", &[]));
            // After Join the fields fold away and this says how it's going: joining, joined, or why not.
            let state = device.wifi_network.as_ref().map(|_| {
                let state = match (&device.wifi_error, &device.wifi_address) {
                    (Some(why), _) => text(wifi_failure(why)).style(text::danger),
                    (None, Some(address)) => text(tr("Joined · %@", &[address])).style(text::success),
                    // A wrong password takes the network 10 to 15 s to turn down.
                    (None, None) => text(tr("Joining… This can take up to 15 seconds.", &[])),
                };
                row![text(tr("Status", &[])).width(140), state].spacing(12)
            });
            let mut wifi = column![
                text(tr("Wi-Fi", &[])),
                row![text(tr("Network", &[])).width(140), text(network)].spacing(12),
            ]
            .push(state)
            .spacing(6);
            let fields = device.wifi_network.is_none() || device.wifi_error.is_some() || self.wifi_editing;
            if device.network {
                wifi = wifi.push(text(tr("Plug in the cable to change the box's network.", &[])).size(13));
            } else if !fields {
                wifi = wifi.push(
                    row![
                        button(text(tr("Change network", &[]))).style(button::secondary).on_press(Message::ChangeWifi),
                        button(text(tr("Forget network", &[]))).style(button::secondary).on_press_maybe((!busy).then_some(Message::ForgetWifi)),
                    ]
                    .spacing(8),
                );
            } else {
                let valid = !self.wifi_ssid.is_empty() && self.wifi_ssid.len() <= 32 && self.wifi_password.len() <= 63;
                // Names are case-sensitive, so the box's own scan is the way to pick one; typing stays for hidden networks.
                let seen = device.wifi_seen.iter().take(8).map(|network| {
                    let label = format!("{}  {} dBm", network.ssid, network.rssi);
                    let style = if network.ssid == self.wifi_ssid { button::primary } else { button::secondary };
                    button(text(label).size(13)).style(style).on_press(Message::WifiSsid(network.ssid.clone())).into()
                });
                wifi = wifi
                    .push(
                        row![text(tr("Nearby networks", &[])).size(13), space::horizontal(), button(text(tr("Scan", &[]))).style(button::secondary).on_press_maybe((!busy).then_some(Message::ScanWifi))]
                            .spacing(8),
                    )
                    .push(column(seen).spacing(4))
                    .push(text_input(&tr("Network name", &[]), &self.wifi_ssid).on_input(Message::WifiSsid))
                    .push(text_input(&tr("Password", &[]), &self.wifi_password).secure(true).on_input(Message::WifiPassword))
                    .push(
                        row![button(text(tr("Join", &[]))).on_press_maybe((valid && !busy).then_some(Message::JoinWifi))]
                            .push(device.wifi_network.as_ref().map(|_| {
                                button(text(tr("Forget network", &[])))
                                    .style(button::secondary)
                                    .on_press_maybe((!busy).then_some(Message::ForgetWifi))
                            }))
                            .spacing(8),
                    );
            }
            wifi.push(text(tr("With Wi-Fi, a paired computer on the same network can drive the box without the cable.", &[])).size(13))
        })
    }

    /// The user's last voice commands and the buddy's spoken replies, newest at the bottom, as on the Mac.
    fn conversation(&self) -> Element<'_, Message> {
        let header = row![
            text(tr("Your last voice commands and the buddy's spoken replies, newest at the bottom.", &[]))
                .size(13)
                .style(text::secondary)
                .width(Length::Fill),
            button(text(tr("Refresh", &[]))).on_press_maybe((!self.conversation_loading).then_some(Message::RefreshConversation)),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        let placeholder = |title: String, detail: String| -> Element<'_, Message> {
            container(column![text(title).size(16), text(detail).size(13).style(text::secondary)].spacing(6).align_x(iced::Alignment::Center))
                .center(Length::Fill)
                .into()
        };
        // A daemon older than the setting doesn't say it; then there is only Codex.
        let choice = self.resident_info.as_ref().and_then(|info| info.brain_setting);
        // Anything else that keeps K0 from working: the UART bridge too slow, the account.
        let voice_off = self
            .resident_info
            .as_ref()
            .filter(|_| self.resident_problem.is_none())
            .and_then(client::ResidentInfo::voice_off_note)
            .map(|note| text(note).size(13).style(text::warning));
        let header = column![header].push(choice.map(|choice| self.agent_setting(choice))).push(voice_off).spacing(12);
        // Voice needs the user's Codex, installed and logged in, or Claude Code with a key: say what's missing first.
        if let Some(problem) = &self.resident_problem {
            let (title, detail) = match problem {
                client::Problem::CodexMissing => (tr("Voice needs Codex", &[]), tr("Voice commands are answered by Codex on this computer, and it isn't installed. Install the Codex CLI, then sign in with `codex login`. This tab notices within half a minute.", &[])),
                client::Problem::CodexLoggedOut => (tr("Voice needs Codex", &[]), tr("Voice commands are answered by Codex on this computer, and it isn't signed in. Run `codex login` in a terminal and sign in. This tab notices within half a minute.", &[])),
                client::Problem::ClaudeMissing => (tr("Voice needs Claude Code", &[]), tr("Voice commands are answered by Claude Code on this computer, and it isn't installed. Install Claude Code; you don't need to sign in to it. This tab notices within half a minute.", &[])),
                client::Problem::AnthropicKeyMissing => (tr("Voice needs Claude Code", &[]), tr("Claude Code answers voice commands only with your own Anthropic API key. Add your key above.", &[])),
                client::Problem::Other => (tr("Voice needs Codex", &[]), tr("The buddy's session isn't available: %@", &[&"?"])),
            };
            let codex_problem = matches!(problem, client::Problem::CodexMissing | client::Problem::CodexLoggedOut);
            let or_claude = (codex_problem && choice == Some(client::BrainChoice::Auto))
                .then(|| text(tr("Or let Claude Code answer, with your own Anthropic API key: add it above.", &[])).size(13).style(text::secondary));
            let install = match problem {
                client::Problem::CodexMissing => Some(button(text(tr("How to install the Codex CLI", &[]))).on_press(Message::OpenCodexInstallPage)),
                client::Problem::ClaudeMissing => Some(button(text(tr("How to install Claude Code", &[]))).on_press(Message::OpenClaudeInstallPage)),
                _ => None,
            };
            let body = container(
                column![text(title).size(16), text(detail).size(13).style(text::secondary)]
                    .push(or_claude)
                    .push(install)
                    .spacing(6)
                    .align_x(iced::Alignment::Center),
            )
            .center(Length::Fill);
            return column![header, body].spacing(12).into();
        }
        let body = match &self.conversation {
            None => container(text(tr("Refreshing…", &[])).style(text::secondary)).center(Length::Fill).into(),
            Some(Err(client::HistoryError::DaemonDown)) => placeholder(
                tr("Couldn't read the conversation", &[]),
                tr("The daemon isn't running. Restart it on the Advanced tab.", &[]),
            ),
            Some(Err(client::HistoryError::Unavailable(why))) => placeholder(
                tr("Couldn't read the conversation", &[]),
                tr("The buddy's session isn't available: %@", &[why]),
            ),
            Some(Ok(exchanges)) if exchanges.is_empty() => placeholder(
                tr("Nothing said yet", &[]),
                tr("Hold K0 on the box and talk to the buddy. What you say and what it says back show up here.", &[]),
            ),
            Some(Ok(exchanges)) => scrollable(
                column(exchanges.iter().map(|exchange| {
                    column![bubble(&exchange.said, true)]
                        .extend(exchange.replies.iter().map(|reply| bubble(reply, false)))
                        .spacing(6)
                        .into()
                }))
                .spacing(10)
                // Room for the scroll bar, which otherwise covers the ends of the lines.
                .padding(iced::Padding::ZERO.right(16)),
            )
            // Opens on the newest, at the bottom.
            .anchor_bottom()
            .height(Length::Fill)
            .into(),
        };
        column![header, body].spacing(12).into()
    }

    /// Which Agent answers, and the user's Anthropic API key for Claude Code, as on the Mac. The key is typed once
    /// and never shown again: the daemon only says whether it has one.
    fn agent_setting(&self, choice: client::BrainChoice) -> Element<'_, Message> {
        let option = |label: String, option: client::BrainChoice| {
            let style = if option == choice { button::primary } else { button::secondary };
            button(text(label)).style(style).on_press(Message::PickBrain(option))
        };
        let mut setting = column![
            row![
                text(tr("Answered by", &[])),
                option(tr("Automatic", &[]), client::BrainChoice::Auto),
                option("Codex".to_owned(), client::BrainChoice::Codex),
                option("Claude Code".to_owned(), client::BrainChoice::Claude),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
        ]
        .spacing(6);
        if choice == client::BrainChoice::Auto {
            setting = setting.push(text(tr("Automatic: Codex when it's ready, else Claude Code with your API key.", &[])).size(12).style(text::secondary));
        }
        if choice != client::BrainChoice::Codex {
            let saved = self.resident_info.as_ref().and_then(|info| info.anthropic_key_set) == Some(true);
            let key = if saved {
                row![
                    text(tr("Anthropic API key saved", &[])),
                    button(text(tr("Remove", &[]))).style(button::secondary).on_press(Message::RemoveAnthropicKey),
                ]
            } else {
                let draft = self.anthropic_key_draft.trim();
                row![
                    text_input(&tr("Anthropic API key", &[]), &self.anthropic_key_draft)
                        .secure(true)
                        .on_input(Message::AnthropicKeyDraft)
                        .on_submit(Message::SaveAnthropicKey)
                        .width(320),
                    button(text(tr("Save", &[]))).on_press_maybe((!draft.is_empty()).then_some(Message::SaveAnthropicKey)),
                ]
            };
            setting = setting
                .push(
                    key.push(button(text(tr("Get an API key", &[]))).style(button::text).on_press(Message::OpenAnthropicConsole))
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                )
                .push(text(tr("Claude Code answers only with your own Anthropic API key, billed to your Anthropic account; a Claude subscription can't be used here. What you say and your Agents' task titles go to Anthropic.", &[])).size(12).style(text::secondary));
        }
        if let Some(error) = &self.resident_setting_error {
            setting = setting.push(text(error.clone()).size(12).style(text::danger));
        }
        setting.into()
    }

    /// The Account tab: the VibeBuddy account, then the user's own keys, as on the Mac.
    fn account(&self) -> Element<'_, Message> {
        let keys = self.status.as_ref().and_then(|status| status.voice_keys.as_ref()).map(|keys| self.own_keys(keys));
        column![self.vibebuddy_account()].push(keys).spacing(24).into()
    }

    /// The user's own keys for spoken replies (ADR-0020). Each is typed once and never shown again: the daemon only
    /// says whether it has one.
    fn own_keys(&self, keys: &status::VoiceKeys) -> Element<'_, Message> {
        let mut section = column![
            text(tr("Your own keys", &[])).size(16),
            text(tr("With your own key, spoken replies in that provider's voices go straight to it, billed to your account there, and never through VibeBuddy.", &[])).size(12).style(text::secondary),
        ]
        .spacing(8);
        for provider in client::KeyProvider::ALL {
            let (name, key, refused) = match provider {
                client::KeyProvider::ElevenLabs => ("ElevenLabs".to_owned(), &keys.elevenlabs, tr("ElevenLabs refused your own key. Replies in its voices stay silent until you set another one.", &[])),
                client::KeyProvider::Volc => (tr("Volcano Engine", &[]), &keys.volc, tr("Volcano Engine refused your own key. Replies in its voices stay silent until you set another one.", &[])),
            };
            let entry = if key.set {
                row![
                    text(tr("Set", &[])).style(text::secondary),
                    button(text(tr("Clear", &[]))).style(button::secondary).on_press(Message::ClearOwnKey(provider)),
                ]
            } else {
                let draft = &self.own_key_drafts[provider as usize];
                row![
                    text_input(&tr("Your own key", &[]), draft)
                        .secure(true)
                        .on_input(move |draft| Message::OwnKeyDraft(provider, draft))
                        .on_submit(Message::SaveOwnKey(provider))
                        .width(260),
                    button(text(tr("Save", &[]))).on_press_maybe((!draft.trim().is_empty()).then_some(Message::SaveOwnKey(provider))),
                ]
            };
            section = section.push(
                row![text(name).width(140)]
                    .push(entry.spacing(8).align_y(iced::Alignment::Center))
                    .push(button(text(tr("Get a key", &[]))).style(button::text).on_press(Message::OpenOwnKeyPage(provider)))
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
            );
            if key.refused {
                section = section.push(text(refused).size(12).style(text::danger));
            }
        }
        section.into()
    }

    /// The VibeBuddy account (ADR-0018): the daemon keeps the session; this only opens the browser to sign in.
    fn vibebuddy_account(&self) -> Element<'_, Message> {
        let Some(status) = &self.status else {
            return text(tr("The daemon isn't running. Restart it on the Advanced tab.", &[])).size(13).into();
        };
        let account = &status.account;
        if !account.available {
            return text(tr("Signing in isn't available in this version of VibeBuddy.", &[])).size(13).into();
        }
        if !account.signed_in {
            let waiting = self.signing_in.then(|| text(tr("Waiting for the browser…", &[])).size(13).style(text::secondary));
            return column![
                text(tr("Signing in opens your browser. Voice commands need an account.", &[])).size(13),
                button(text(tr("Sign in", &[]))).on_press(Message::SignIn),
            ]
            .push(waiting)
            .spacing(12)
            .into();
        }
        let who = match (&account.github_login, account.github_id) {
            (Some(login), _) => tr("Signed in with GitHub as %@", &[login]),
            (None, Some(id)) => tr("Signed in with GitHub (user %lld)", &[&id]),
            (None, None) => tr("Signed in as %@", &[&account.email.as_deref().unwrap_or("—")]),
        };
        let actions: Element<'_, Message> = if self.confirm_delete_account {
            confirm_flash(
                tr("Delete your VibeBuddy account?", &[]),
                tr("This deletes the account and everything kept about it, and signs out on every computer. It can't be undone.", &[]),
                tr("Delete account", &[]),
                Some(Message::DeleteAccount),
                Message::AskDeleteAccount(false),
            )
        } else {
            row![
                button(text(tr("Sign out", &[]))).on_press(Message::SignOut),
                button(text(tr("Delete account…", &[]))).style(button::secondary).on_press(Message::AskDeleteAccount(true)),
            ]
            .spacing(8)
            .into()
        };
        let today = match (account.refused.as_deref(), &account.today) {
            (Some(refused), _) => {
                let back = status::voice_comes_back(chrono::Utc::now(), &chrono::Local);
                let line = if refused == "global_cap" {
                    tr("Voice is paused for everyone today. It comes back at %@.", &[&back])
                } else {
                    tr("Today's voice is used up. It comes back at %@.", &[&back])
                };
                Some(text(line).size(13).style(text::warning))
            }
            (None, Some(usage)) => Some(text(tr("Voice today: %lld of %lld", &[&usage.commands, &usage.commands_limit])).size(13).style(text::secondary)),
            (None, None) => None,
        };
        let plan = account.plan.as_ref().map(|plan| {
            let name = if plan.paid() { tr("Paid plan", &[]) } else { tr("Free", &[]) };
            row![text(tr("Plan", &[])).width(140), text(name)].spacing(12)
        });
        let month = account.plan.as_ref().and_then(|plan| plan.month_line()).map(|line| text(line).size(13).style(text::secondary));
        column![text(who)].push(plan).push(month).push(today).push(actions).spacing(12).into()
    }

    fn advanced(&self) -> Element<'_, Message> {
        let daemon = match &self.status {
            Some(status) => tr("Running · %@", &[&status.daemon.short_build()]),
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
/// OpenAI's page on installing the Codex CLI, which voice commands need.
const CODEX_INSTALL_PAGE: &str = "https://learn.chatgpt.com/docs/codex/cli";
/// Anthropic's page on installing Claude Code, the other Agent that can answer voice commands.
const CLAUDE_INSTALL_PAGE: &str = "https://code.claude.com/docs/en/setup";
/// The Claude Console, where the user makes the Anthropic API key Claude Code answers voice commands with.
const ANTHROPIC_CONSOLE: &str = "https://platform.claude.com/";
/// Where the user makes their own key for spoken replies in ElevenLabs' voices.
const ELEVENLABS_KEYS: &str = "https://elevenlabs.io/app/settings/api-keys";
/// Volcano Engine's speech console, where the user makes their own key for the Doubao voices.
const VOLC_CONSOLE: &str = "https://console.volcengine.com/speech/";
const NEW_ISSUE: &str = "https://github.com/second-state/vibebuddy/issues/new";

/// The id under which the box wears the user's own Character.
const CUSTOM: &str = "custom";
/// The robot, the default Character: written as `robot`, or `builtin` when the box has no pack at all.
const ROBOT: &str = "robot";
const BUILTIN: &str = "builtin";

/// The Mac app's `CustomCharacterCard.prompt`: a start for any image model.
const CUSTOM_PROMPT: &str = "Pixel art game sprite sheet of [describe your character]: one image, a 2 × 2 grid of the same character in exactly the same pose, with clear white space between the four. Chibi proportions, standing, front view, full body, arms down, flat colors, thick dark outline, limited 16-color palette, plain solid white background. Top left: normal. Top right: eyes closed. Bottom left: a big happy smile. Bottom right: a sad face.";

/// A look frame as an image to draw, each pixel `scale` pixels wide.
/// The user's words on the right in the accent color, the buddy's replies on the left, as in a chat.
fn bubble(words: &str, mine: bool) -> Element<'_, Message> {
    let bubble = container(text(words).size(14)).padding([7, 12]).max_width(440).style(move |theme: &Theme| {
        let palette = theme.extended_palette();
        let pair = if mine { palette.primary.base } else { palette.background.weak };
        container::Style {
            background: Some(pair.color.into()),
            text_color: Some(pair.text),
            border: iced::border::rounded(12),
            ..container::Style::default()
        }
    });
    if mine { row![space::horizontal(), bubble] } else { row![bubble, space::horizontal()] }.into()
}

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

/// One entry of the box screens picker: `robot` or `korekore` (ADR-0021).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BoxScreens(&'static str);

impl BoxScreens {
    const ALL: [BoxScreens; 2] = [BoxScreens("robot"), BoxScreens("korekore")];

    fn of(style: &str) -> Option<BoxScreens> {
        Self::ALL.into_iter().find(|screens| screens.0 == style)
    }
}

impl std::fmt::Display for BoxScreens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            "robot" => f.write_str(&tr("Robot", &[])),
            other => f.write_str(other),
        }
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

/// The Agents tab's status dots: not set up, waiting for the first event, working, and Codex needing trust again.
const DOT_OFF: iced::Color = iced::Color::from_rgb(0.55, 0.55, 0.55);
const DOT_WAIT: iced::Color = iced::Color::from_rgb(0.95, 0.6, 0.15);
const DOT_GOOD: iced::Color = iced::Color::from_rgb(0.3, 0.75, 0.35);
const DOT_BAD: iced::Color = iced::Color::from_rgb(0.9, 0.3, 0.3);

/// The app starts at login through an XDG autostart entry: install.sh writes one in the user's autostart folder, the
/// pacman package one in /etc/xdg/autostart. A user entry decides when there is one; `Hidden=true` there turns a
/// system-wide one off, as the spec says.
fn autostart_entry() -> Option<std::path::PathBuf> {
    Some(config_dir()?.parent()?.join("autostart/vibebuddy.desktop"))
}

fn launch_at_login() -> bool {
    match autostart_entry().and_then(|path| std::fs::read_to_string(path).ok()) {
        Some(entry) => !entry.lines().any(|line| line.trim() == "Hidden=true"),
        None => std::path::Path::new("/etc/xdg/autostart/vibebuddy.desktop").is_file(),
    }
}

fn set_launch_at_login(enabled: bool) -> Result<(), String> {
    let path = autostart_entry().ok_or("HOME is not set")?;
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let entry = if enabled {
        format!(
            "[Desktop Entry]\nType=Application\nName=VibeBuddy\nComment=Tray icon and settings for the VibeBuddy box\nExec={}\nIcon=vibebuddy\nTerminal=false\nCategories=Utility;\nStartupWMClass=vibebuddy\n",
            exe.display()
        )
    } else {
        "[Desktop Entry]\nType=Application\nName=VibeBuddy\nHidden=true\n".to_owned()
    };
    std::fs::create_dir_all(path.parent().unwrap_or(&path)).map_err(|error| error.to_string())?;
    std::fs::write(&path, entry).map_err(|error| format!("{}: {error}", path.display()))
}

/// One notification per new firmware version, only while the box is there to take it and once it's downloaded, as on
/// the Mac; the version last told about is kept in the state directory.
fn notify_new_firmware(status: &Status) {
    let Some(offer) = status.updates.as_ref().filter(|updates| updates.firmware_update_available()).and_then(|updates| updates.firmware.as_ref())
    else {
        return;
    };
    let Some(marker) = state_dir().map(|dir| dir.join("notified-firmware")) else { return };
    if !status.device.connected || std::fs::read_to_string(&marker).is_ok_and(|told| told.trim() == offer.version) {
        return;
    }
    let _ = std::fs::create_dir_all(marker.parent().unwrap_or(&marker));
    let _ = std::fs::write(&marker, &offer.version);
    let title = tr("Box firmware %@ is available", &[&offer.version]);
    // The korekore UI words it its own way (canvas: "SYSTEM NOTIFICATIONS").
    let body = if korekore::requested() { tr("Update it from Settings › Device.", &[]) } else { tr("Open Settings → Device to update the box.", &[]) };
    if let Err(error) = launch("notify-send", &["--app-name=VibeBuddy".as_ref(), title.as_ref(), body.as_ref()]) {
        eprintln!("vibebuddy-desktop: {error}");
    }
}

/// Asks once more before flashing, since the box restarts: a title, what will happen, and the two buttons.
fn confirm_flash<'a>(title: String, detail: String, action: String, flash: Option<Message>, cancel: Message) -> Element<'a, Message> {
    column![
        text(title),
        text(detail).size(13),
        row![
            button(text(action)).on_press_maybe(flash),
            button(text(tr("Cancel", &[]))).style(button::secondary).on_press(cancel),
        ]
        .spacing(8),
    ]
    .spacing(8)
    .into()
}

/// The daemon's XDG directories (see `daemon/src/config.rs`), where it keeps state and config and the hook its log.
fn state_dir() -> Option<std::path::PathBuf> {
    xdg_dir("XDG_STATE_HOME", ".local/state")
}

fn config_dir() -> Option<std::path::PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

/// Read from the shell's own config, which `omarchy plugin enable` and `disable` rewrite.
#[cfg(target_os = "linux")]
fn plugin_in_omarchy_bar() -> bool {
    // `config_dir` is ours, inside the XDG config directory; Omarchy's sits next to it.
    config_dir()
        .and_then(|dir| std::fs::read_to_string(dir.parent()?.join("omarchy/shell.json")).ok())
        .is_some_and(|text| tray::plugin_in_bar(&text))
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

#[cfg(target_os = "linux")]
fn run_tray() -> impl futures::Stream<Item = Message> {
    use futures::{SinkExt, StreamExt};
    use ksni::TrayMethods;

    iced::stream::channel(16, async |mut output| {
        let (events, mut clicks) = futures::channel::mpsc::unbounded();
        let kore = korekore::requested().then(|| korekore::picture::BoxPicture::derive(None));
        let tray = tray::Tray { menu: MenuState::derive(None), kore, color: [255, 255, 255], events };
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
                tray::TrayEvent::RestartDaemon => Message::StartDaemon,
                tray::TrayEvent::CheckForUpdates => Message::CheckFromMenu,
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

/// What `device.wifi_error` means (docs/protocol.md "Wi-Fi network").
fn wifi_failure(why: &str) -> String {
    match why {
        "wrong_password" => tr("The network turned the box away; the password is probably wrong. Check it, then join again.", &[]),
        "not_found" => tr("The box can't find this network. It only joins 2.4 GHz Wi-Fi: check the name, and that the box is in range.", &[]),
        "security" => tr("The box can't join this network's kind of security. It joins WPA2 and open networks.", &[]),
        "password" => tr("The password must be 8 to 63 characters.", &[]),
        "ssid" => tr("The network name is too long: 32 bytes at most.", &[]),
        _ => tr("Couldn't join (%@). Check the network name and password, then join again.", &[&why]),
    }
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
