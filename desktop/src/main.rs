//! `vibebuddy-desktop`: VibeBuddy's tray icon and settings window outside macOS, the counterpart of the Mac
//! app's menu bar icon and settings window. It is only a client of `vibebuddyd`: systemd supervises the
//! daemon here, so quitting the app leaves the box online.

// The tray exists only on Linux; other builds are for working on the window.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod agents;
mod assets;
mod character;
mod client;
mod custom;
mod firmware_file;
mod i18n;
mod making;
#[cfg(target_os = "linux")]
mod instance;
mod korekore;
mod preview;
mod status;
#[cfg(target_os = "linux")]
mod tray;

use std::time::{Duration, SystemTime};

use iced::widget::image;
use iced::{Element, Subscription, Task, Theme, window};

use i18n::{UiLanguage, tr};
#[cfg(target_os = "linux")]
use status::MenuState;
use status::{Config, Status};

/// How many exchanges the Conversation tab reads, as on the Mac.
const CONVERSATION_LIMIT: usize = 20;

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
    // The korekore UI's fonts, there before any window opens (ADR-0021); its own face is the default.
    for font in korekore::FONTS {
        app = app.font(font);
    }
    app.default_font(korekore::DEFAULT_FONT).run()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    General,
    Sound,
    Agents,
    Device,
    /// Only while the daemon runs the resident session (experimental).
    Conversation,
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
    PluginTick,
    OpenSettings,
    /// A page of the first-run guide, or (None) leave it for the tabs.
    Guide(Option<usize>),
    /// The guide's box that blinked isn't the user's.
    NotThisOne,
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
    /// The form for a voice of the user's own: open or closed, its provider, voice id and name.
    MakeOwnOpen(bool),
    MakeOwnProvider(OwnProvider),
    MakeOwnVoice(String),
    MakeOwnName(String),
    MakeOwnPick(making::ProviderVoice),
    ElevenVoices(Result<Vec<making::ProviderVoice>, String>),
    MakeOwnTry,
    MakeOwnTried(Result<(making::Voice, Vec<u8>), String>),
    RemoveOwnVoice(String),
    OwnVoiceRemoved(String, Result<(), String>),
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
    settings: Option<window::Id>,
    tab: Tab,
    /// The first-run guide's page while it shows in place of the tabs.
    guide: Option<usize>,
    /// The user said the box the guide found isn't theirs: say how to find the right one.
    not_this_one: bool,
    /// The slider's position while it is being dragged; the box's own value otherwise.
    volume: Option<u8>,
    /// The last screen grabbed from the box, as PNG, with the local time it was grabbed, and whether a grab is under
    /// way.
    screenshot: Option<(Vec<u8>, image::Handle, String)>,
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
    /// Adding a voice of the user's own: None while the form is closed.
    own: Option<OwnVoiceForm>,
}

/// A voice of the user's own being added: where it is, the name to keep it under, and why trying it failed.
#[derive(Debug, Default)]
struct OwnVoiceForm {
    provider: OwnProvider,
    voice: String,
    name: String,
    trying: bool,
    problem: Option<String>,
    /// The voices of the own ElevenLabs key, to pick from, or why they couldn't be listed; None until asked.
    listed: Option<Result<Vec<making::ProviderVoice>, String>>,
}

/// The providers a voice of the user's own can be in, for the form's picker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OwnProvider {
    #[default]
    ElevenLabs,
    Volc,
}

impl OwnProvider {
    fn id(self) -> &'static str {
        match self {
            OwnProvider::ElevenLabs => "elevenlabs",
            OwnProvider::Volc => "volc",
        }
    }
}

impl std::fmt::Display for OwnProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OwnProvider::ElevenLabs => write!(f, "ElevenLabs"),
            OwnProvider::Volc => write!(f, "{}", tr("Volcano Engine", &[])),
        }
    }
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
            settings: None,
            // The korekore sidebar starts on Character, as the design does.
            tab: Tab::Sound,
            guide: None,
            not_this_one: false,
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
        // Debug builds only: open the window on a given section or first-run step, for checking it on a desktop
        // without a pointer (VIBEBUDDY_KOREKORE_SECTION=device, VIBEBUDDY_KOREKORE_GUIDE=1).
        #[cfg(debug_assertions)]
        if let Ok(name) = std::env::var("VIBEBUDDY_KOREKORE_SECTION")
            && let Some(section) = korekore::Section::ALL.into_iter().find(|section| format!("{section:?}").eq_ignore_ascii_case(&name))
        {
            app.tab = section.tab();
        }
        let restarted = std::env::args().any(|arg| arg == RESTARTED || arg == SETTINGS);
        let first = first_launch();
        // The install script connected the agents and set up the start at login; the guide is for what it couldn't.
        app.guide = (first && cfg!(target_os = "linux")).then_some(0);
        #[cfg(debug_assertions)]
        {
            app.guide = std::env::var("VIBEBUDDY_KOREKORE_GUIDE").ok().and_then(|page| page.parse().ok()).or(app.guide);
        }
        let open = first || restarted || !cfg!(target_os = "linux");
        let task = if open { Task::done(Message::OpenSettings) } else { Task::none() };
        (app, task)
    }

    fn title(&self, _window: window::Id) -> String {
        "VibeBuddy".to_owned()
    }

    /// The korekore pages draw their own colors; widgets without a style of their own get the light theme the
    /// snapshots are drawn with.
    fn theme(&self, _window: window::Id) -> Theme {
        Theme::Light
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
            iced::time::every(Duration::from_secs(2)).map(|_| Message::PluginTick),
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
            Message::PluginTick => {
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
            }
            Message::OpenSettings => {
                if let Some(id) = self.settings {
                    return window::gain_focus(id);
                }
                // The design's 960 × 772, down to 900 × 760.
                let (id, open) = window::open(window::Settings {
                    size: iced::Size::new(960.0, 772.0),
                    min_size: Some(iced::Size::new(900.0, 760.0)),
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
            Message::NotThisOne => self.not_this_one = true,
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
                // Reading the day's use costs a request to the service, so only when General, which shows the account,
                // opens; the status brings it.
                if tab == Tab::General && self.status.as_ref().is_some_and(|status| status.account.signed_in) {
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
                    own: None,
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
            Message::MakeOwnOpen(open) => {
                if let Some(form) = &mut self.make {
                    form.own = open.then(OwnVoiceForm::default);
                }
                // ElevenLabs lists the key's voices, so the id needn't be looked up; Volcano Engine can't with a key alone.
                if open {
                    return Task::perform(client::elevenlabs_voices(), Message::ElevenVoices);
                }
            }
            Message::MakeOwnProvider(provider) => {
                if let Some(own) = self.make.as_mut().and_then(|form| form.own.as_mut()) {
                    own.provider = provider;
                    own.voice.clear();
                    if provider == OwnProvider::ElevenLabs && !matches!(own.listed, Some(Ok(_))) {
                        return Task::perform(client::elevenlabs_voices(), Message::ElevenVoices);
                    }
                }
            }
            Message::MakeOwnPick(picked) => {
                if let Some(own) = self.make.as_mut().and_then(|form| form.own.as_mut()) {
                    own.voice = picked.voice;
                    own.name = picked.name;
                }
            }
            Message::ElevenVoices(listed) => {
                if let Some(own) = self.make.as_mut().and_then(|form| form.own.as_mut()) {
                    own.listed = Some(listed);
                }
            }
            Message::MakeOwnVoice(voice) => {
                if let Some(own) = self.make.as_mut().and_then(|form| form.own.as_mut()) {
                    own.voice = voice;
                }
            }
            Message::MakeOwnName(name) => {
                if let Some(own) = self.make.as_mut().and_then(|form| form.own.as_mut()) {
                    own.name = name;
                }
            }
            Message::MakeOwnTry => {
                let Some(form) = &mut self.make else { return Task::none() };
                // The first line, as the box would say it, is what the voice is tried with.
                let text = form.lines.first().map(|line| line.text.clone()).unwrap_or_else(|| tr("All done!", &[]));
                let Some(own) = form.own.as_mut().filter(|own| !own.trying) else { return Task::none() };
                own.trying = true;
                own.problem = None;
                let tried = client::add_own_voice(own.provider.id().to_owned(), own.voice.trim().to_owned(), own.name.trim().to_owned(), text);
                return Task::perform(tried, Message::MakeOwnTried);
            }
            Message::MakeOwnTried(result) => {
                let Some(form) = &mut self.make else { return Task::none() };
                match result {
                    Ok((voice, audio)) => {
                        self.character_voices.retain(|kept| kept.id != voice.id);
                        self.character_voices.push(voice.clone());
                        form.voice = Some(voice.clone());
                        form.own = None;
                        // What it said plays at once, as a preview does.
                        if let Some((_, stop)) = self.previewing.take() {
                            stop.notify_one();
                        }
                        let Some(pcm) = preview::wav(&audio) else { return Task::none() };
                        let stop = std::sync::Arc::new(tokio::sync::Notify::new());
                        self.previewing = Some((voice.id.clone(), stop.clone()));
                        return Task::perform(preview::play(pcm, stop), move |result| Message::PreviewDone(voice.id.clone(), result));
                    }
                    Err(error) => {
                        if let Some(own) = form.own.as_mut() {
                            own.trying = false;
                            own.problem = Some(tr("Couldn't use that voice: %@", &[&error]));
                        }
                    }
                }
            }
            Message::RemoveOwnVoice(id) => return Task::perform(client::delete_own_voice(id.clone()), move |result| Message::OwnVoiceRemoved(id.clone(), result)),
            Message::OwnVoiceRemoved(id, result) => match result {
                Ok(()) => {
                    self.character_voices.retain(|voice| voice.id != id);
                    if let Some(form) = self.make.as_mut().filter(|form| form.voice.as_ref().is_some_and(|voice| voice.id == id)) {
                        form.voice = self.character_voices.iter().find(|voice| voice.id == form.base.id).cloned();
                    }
                }
                Err(error) => self.notice = Some(Err(tr("Couldn't remove the voice: %@", &[&error]))),
            },
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
                    Ok(png) => self.screenshot = Some((png.clone(), image::Handle::from_bytes(png), chrono::Local::now().format("%H:%M").to_string())),
                    Err(error) => self.notice = Some(Err(error)),
                }
            }
            Message::SaveScreenshot => {
                if let Some((png, _, _)) = &self.screenshot {
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
        iced::widget::operation::snap_to_end(korekore::pages::CHARACTER_PAGE)
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
            let kore = korekore::picture::BoxPicture::derive(self.status.as_ref());
            return Task::future(async move {
                handle
                    .update(move |tray| {
                        tray.menu = menu;
                        tray.kore = kore;
                    })
                    .await;
            })
            .discard();
        }
        Task::none()
    }

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        if let Some(page) = self.guide {
            return self.kk_guide(page);
        }
        self.korekore(korekore::Section::of(self.tab))
    }

    /// The korekore settings window (ADR-0021).
    fn korekore(&self, section: korekore::Section) -> Element<'_, Message> {
        // Voice commands are experimental: no Conversation section while the daemon has them off.
        let sections: Vec<korekore::Section> = korekore::Section::ALL
            .into_iter()
            .filter(|&section| section != korekore::Section::Conversation || self.resident)
            .collect();
        let picture = korekore::picture::BoxPicture::derive(self.status.as_ref());
        korekore::view::settings(&picture, &sections, section, env!("CARGO_PKG_VERSION"), Message::Korekore, self.korekore_page(section))
    }

    fn updates(&self) -> Option<&status::Updates> {
        self.status.as_ref()?.updates.as_ref()
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

/// The Mac app's `CustomCharacter.prompt`: a start for any image model.
const CUSTOM_PROMPT: &str = "Pixel art game sprite sheet of [describe your character]: one image, a 2 × 2 grid of the same character in exactly the same pose, with clear white space between the four. Chibi proportions, standing, front view, full body, arms down, flat colors, thick dark outline, limited 16-color palette, plain solid white background. Top left: normal. Top right: eyes closed. Bottom left: a big happy smile. Bottom right: a sad face.";

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
    // As the korekore canvas words it ("SYSTEM NOTIFICATIONS").
    let body = tr("Update it from Settings › Device.", &[]);
    if let Err(error) = launch("notify-send", &["--app-name=VibeBuddy".as_ref(), title.as_ref(), body.as_ref()]) {
        eprintln!("vibebuddy-desktop: {error}");
    }
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
        let kore = korekore::picture::BoxPicture::derive(None);
        let tray = tray::Tray { menu: MenuState::derive(None), kore, events };
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
