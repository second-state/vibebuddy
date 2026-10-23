mod account;
mod activity;
mod app_server;
mod ci;
mod agent_hooks;
mod claude_hooks;
mod codex_hooks;
mod config;
mod identity;
mod lan;
mod noise_link;
mod relay;
mod resident;
mod resident_claude;
mod link_alert;
mod occasions;
mod persona;
mod rom_flasher;
mod screenshot;
mod serial_transport;
mod session_titles;
mod source_opener;
mod speech;
mod status;
mod transfer;
mod synthesis;
mod updates;
mod voice;
mod voice_writer;

use std::convert::Infallible;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use activity::{ActivitySource, ActivityTracker};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use vibebuddy_protocol::{Event, VERSION};
use chrono::Timelike;
use ci::CiWatcher;
use agent_hooks::AgentHook;
use claude_hooks::ClaudeHook;
use codex_hooks::CodexHook;
use config::Config;
use serde::Serialize;
use serial_transport::{DeviceMessage, SerialConfig, SerialTransport, Transport, TransportError};
use session_titles::SessionTitles;
use status::{DaemonInfo, DeviceState, HooksSeen, Operation, OperationKind, OperationState, Status};
use tokio::sync::{Mutex, Notify, broadcast};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use updates::Updates;

/// Once the last session hangs there are no more hook events; only a periodic sweep can free the screen.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);
/// Heartbeat interval. The device uses this cadence to judge whether the link is alive; the firmware timeout is three times it.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// Our firmware reports `DISPLAY READY` about two seconds after a reset.
const FIRMWARE_BOOT_TIMEOUT: Duration = Duration::from_secs(15);
/// Build-time git description, written by `build.rs`.
const BUILD_REVISION: &str = env!("VIBEBUDDY_BUILD");

#[derive(Clone)]
struct AppState {
    transport: Arc<dyn Transport>,
    /// All agents share one aggregator: the device has just one screen and one little buddy.
    activities: Arc<Mutex<ActivityTracker>>,
    /// Session title lookup and cache: Claude app session titles and Codex thread names.
    titles: Arc<Mutex<SessionTitles>>,
    /// What the device looks like right now, pieced together from diagnostic lines.
    device: Arc<Mutex<DeviceState>>,
    hooks_seen: Arc<Mutex<HooksSeen>>,
    config: Arc<Mutex<Config>>,
    config_path: Option<PathBuf>,
    /// Writing a voice pack or flashing firmware; only one at a time.
    operation: Arc<Mutex<Option<Operation>>>,
    /// Broadcast of device messages: operations that wait for an ack, like voice-pack writes and screenshots, each subscribe.
    device_bus: broadcast::Sender<DeviceMessage>,
    /// Pinged whenever the status changes; the status stream pushes a new snapshot on it.
    status_changed: broadcast::Sender<()>,
    /// The app's version, reported to the device with the heartbeat; None when there is no app.
    app_version: Option<String>,
    /// The real serial worker, which must give up the port while flashing; absent in tests.
    serial: Option<Arc<SerialTransport>>,
    /// This computer's pairing key; None when it couldn't be read or made, and then nothing is paired.
    identity: Option<Arc<identity::Identity>>,
    /// Where the Relay is; the update manifest may move it.
    relay: Option<relay::RelayConfig>,
    /// Where the box's key and address are remembered; None in tests unless one sets it, so they never write
    /// over the user's own.
    state_dir: Option<PathBuf>,
    /// The update manifest and the firmware it offers; with no source, nothing is ever checked.
    updates: Arc<Mutex<Updates>>,
    /// "Check for updates" pressed: checks now instead of waiting for the daily one.
    check_updates: Arc<Notify>,
    /// The resident session voice commands go to (ADR-0017); None without a state directory, and in tests
    /// unless one sets it. Present even when switched off, so its hook events are still recognised.
    resident: Option<Arc<resident::Resident>>,
    /// Voice commands from the box (ADR-0017): its recording, and the turn being answered.
    voice: Arc<voice::Voice>,
    /// The box on the cable said `LINK NOISE` since its last `BOX KEY` (ADR-0015).
    noise_seen: Arc<std::sync::atomic::AtomicBool>,
    /// What the App said about the Character pack it last wrote (who lends a custom one, the form of address),
    /// kept in [`persona::RECORD_FILE`] in the state directory.
    written: Arc<Mutex<persona::Written>>,
    /// The person's VibeBuddy account (ADR-0018); unavailable in tests unless one sets it.
    account: Arc<account::Account>,
}

impl AppState {
    fn new(
        transport: Arc<dyn Transport>,
        activities: ActivityTracker,
        titles: SessionTitles,
        config_path: Option<PathBuf>,
    ) -> Self {
        let config = config_path
            .as_deref()
            .map(Config::load)
            .unwrap_or_default();
        let (device_bus, _) = broadcast::channel(1024);
        let (status_changed, _) = broadcast::channel(64);
        Self {
            transport,
            activities: Arc::new(Mutex::new(activities)),
            titles: Arc::new(Mutex::new(titles)),
            device: Arc::new(Mutex::new(DeviceState::default())),
            hooks_seen: Arc::new(Mutex::new(HooksSeen::default())),
            config: Arc::new(Mutex::new(config)),
            config_path,
            operation: Arc::new(Mutex::new(None)),
            device_bus,
            status_changed,
            app_version: env::var("VIBEBUDDY_APP_VERSION").ok().filter(|value| !value.is_empty()),
            serial: None,
            identity: None,
            relay: None,
            state_dir: None,
            updates: Arc::new(Mutex::new(Updates::new(None, None))),
            check_updates: Arc::new(Notify::new()),
            resident: None,
            voice: Arc::new(voice::Voice::new(None, None)),
            noise_seen: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            written: Arc::default(),
            account: Arc::new(account::Account::new(None, None, 7331)),
        }
    }

    async fn update_checks_enabled(&self) -> bool {
        self.config.lock().await.check_updates.unwrap_or_else(updates::enabled_by_default)
    }

    fn notify_status(&self) {
        let _ = self.status_changed.send(());
    }

    async fn snapshot(&self) -> Status {
        let device = self.device.lock().await.clone();
        let updates = self.updates.lock().await.status(&device, self.update_checks_enabled().await);
        Status {
            daemon: DaemonInfo {
                build: build_identity(self.app_version.as_deref()),
                revision: BUILD_REVISION.to_owned(),
                app_version: self.app_version.clone(),
            },
            device,
            today: self.activities.lock().await.today(),
            hooks: self.hooks_seen.lock().await.clone(),
            operation: self.operation.lock().await.clone(),
            config: self.config.lock().await.clone(),
            updates,
            account: self.account.status(),
        }
    }

    async fn set_operation(&self, operation: Option<Operation>) {
        *self.operation.lock().await = operation;
        self.notify_status();
    }

    async fn save_config(&self, change: impl FnOnce(&mut Config)) {
        let snapshot = {
            let mut config = self.config.lock().await;
            change(&mut config);
            config.clone()
        };
        if let Some(path) = &self.config_path
            && let Err(error) = snapshot.save(path)
        {
            warn!(%error, path = %path.display(), "failed to save config");
        }
        self.notify_status();
    }
}

#[derive(Serialize)]
struct ApiResponse {
    accepted: bool,
    message: String,
}

#[tokio::main]
async fn main() {
    let log_filter = env::var("RUST_LOG")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| EnvFilter::try_new(value).ok())
        .unwrap_or_else(|| EnvFilter::new("vibebuddyd=info"));
    tracing_subscriber::fmt().with_env_filter(log_filter).init();

    let bind_address = env::var("VIBEBUDDY_BIND")
        .unwrap_or_else(|_| "127.0.0.1:7331".to_owned())
        .parse::<SocketAddr>()
        .unwrap_or_else(|error| panic!("invalid VIBEBUDDY_BIND: {error}"));
    let identity = match identity::identity_file().map(|path| identity::Identity::load_or_create(&path, identity::computer_name())) {
        Some(Ok(identity)) => Some(Arc::new(identity)),
        Some(Err(error)) => {
            warn!(%error, "no pairing key; boxes won't be paired with this computer");
            None
        }
        None => None,
    };
    let mut serial_config = SerialConfig::from_env(known_box_file());
    let mut relay_config = None;
    // Paired, the box can also be reached over the local network when it isn't on the cable (ADR-0012).
    if let (Some(identity), Some(state_dir)) = (&identity, config::state_dir()) {
        serial_config = serial_config.with_lan(lan::LanConfig {
            identity: identity.clone(),
            state_dir,
            fixed_address: std::env::var("VIBEBUDDY_LAN_ADDR").ok(),
            discovery: lan::Discovery::default(),
        });
        // VIBEBUDDY_RELAY_URL points at another Relay, such as `ws://127.0.0.1:8787` under `wrangler dev`.
        let relay = relay::RelayConfig::new(identity.clone(), std::env::var("VIBEBUDDY_RELAY_URL").ok(), relay::release());
        serial_config = serial_config.with_relay(relay.clone());
        relay_config = Some(relay);
    }
    let pin = serial_config.pin();
    if let Some(pin) = &pin {
        warn!(variable = pin.variable, value = %pin.value, "the serial search is pinned; other boxes are ignored");
    }
    let (serial_transport, device_events) = SerialTransport::spawn(serial_config);
    let serial_transport = Arc::new(serial_transport);
    let transport: Arc<dyn Transport> = serial_transport.clone();
    let activities = match stats_file() {
        Some(path) => ActivityTracker::with_stats_file(path),
        None => ActivityTracker::default(),
    };
    let mut state = AppState::new(
        transport,
        activities,
        SessionTitles::from_home(),
        config::config_file(),
    );
    serial_transport.set_relay_enabled(state.config.lock().await.relay);
    state.serial = Some(serial_transport);
    if let Some(identity) = &identity {
        state.device.lock().await.computer_key = Some(identity.public_key());
    }
    state.identity = identity;
    state.relay = relay_config;
    state.account = Arc::new(account::Account::new(
        synthesis::service_base(env::var(synthesis::BASE_URL_VARIABLE).ok(), relay::release()),
        config::config_dir().as_deref(),
        bind_address.port(),
    ));
    state.state_dir = config::state_dir();
    if let Some(dir) = &state.state_dir {
        state.written = Arc::new(Mutex::new(persona::Written::load(&dir.join(persona::RECORD_FILE))));
    }
    state.resident =state.state_dir.as_deref().map(|dir| {
        let enabled = resident::enabled_by_env();
        if enabled {
            warn!("the experimental resident session is on");
        }
        let resident = resident::Resident::new(enabled, dir, resident::Launcher::Codex, state.activities.clone());
        Arc::new(match config::config_dir() {
            Some(config) => resident.with_settings_file(config.join(resident::SETTINGS_FILE)),
            None => resident,
        })
    });
    let session = Some(state.account.clone());
    state.voice = Arc::new(voice::Voice::new(speech::from_env(session.clone()), state.resident.clone()).with_synthesizer(synthesis::from_env(session)));
    if let Some(resident) = state.resident.clone() {
        let wearing = wearing(&state).await;
        tokio::spawn(async move { resident.warm(&wearing).await });
    }
    state.device.lock().await.pin = pin;
    state.updates = Arc::new(Mutex::new(Updates::new(
        updates::Source::configured(),
        config::state_dir().map(|dir| dir.join("updates")),
    )));
    if let (Some(relay), Some(url)) = (&state.relay, state.updates.lock().await.relay_url()) {
        relay.offer(&url);
    }
    if let Some(relay) = &state.relay {
        state.device.lock().await.relay_server = Some(relay.url());
    }
    tokio::spawn(watch_parent());
    tokio::spawn(sweep_expired_activities(state.clone()));
    tokio::spawn(send_heartbeats(state.clone()));
    tokio::spawn(poll_ci(state.clone()));
    tokio::spawn(handle_device_events(state.clone(), device_events));
    tokio::spawn(check_for_updates(state.clone()));
    if let Some(resident) = state.resident.clone().filter(|resident| resident.enabled()) {
        follow_resident_readiness(&state, resident);
    }
    follow_account(&state);
    tokio::spawn(hand_out_voice_tokens(state.clone()));
    // On macOS the app reports a lost link; elsewhere there is no app, so the daemon does it.
    if !cfg!(target_os = "macos") {
        tokio::spawn(link_alert::watch(state.clone()));
    }
    let app = app(state);
    let listener = tokio::net::TcpListener::bind(bind_address)
        .await
        .unwrap_or_else(|error| panic!("cannot listen on {bind_address}: {error}"));

    info!(address = %bind_address, "vibebuddyd started");
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|error| panic!("HTTP server failed: {error}"));
}

/// Device-to-Mac events currently only allow a K2 click. Open the current activity first; when idle, return the
/// most recent locatable agent/CI source.
async fn handle_device_events(
    state: AppState,
    mut events: tokio::sync::mpsc::Receiver<DeviceMessage>,
) {
    while let Some(message) = events.recv().await {
        publish_device_message(&state, message).await;
    }
}

/// Every device message goes through here: update device state, broadcast to operations waiting for acks, and K2 opens the source.
/// Tests inject device messages here too, so it must not depend on the serial port.
async fn publish_device_message(state: &AppState, message: DeviceMessage) {
    let (changed, booted, usb_serial) = {
        let mut device = state.device.lock().await;
        let had_build = device.firmware_build.is_some();
        let changed = device.apply(&message);
        (changed, !had_build && device.firmware_build.is_some(), device.usb_serial.clone())
    };
    // Our firmware reported its build: this device is the box, so prefer it when others are plugged in too.
    if booted && let (Some(serial), Some(path)) = (usb_serial, known_box_file()) {
        serial_transport::remember_box(&path, &serial);
    }
    if changed {
        state.notify_status();
    }
    // The daily greeting waits for the firmware to report its build: the box restarts when the port
    // opens, and a line sent before it is listening would be lost.
    if booted && let Some(greeting) = state.activities.lock().await.daily_greeting() {
        send_event(state, greeting);
    }
    // A box that reports its key can pair, and one at the end of this cable is ours to pair with
    // (ADR-0012). Pairing again is harmless: the box only writes flash when something changed.
    // Over the network the box refuses pairing, and what it says there isn't news about the cable.
    let on_cable = !state.device.lock().await.network;
    // The box says `LINK NOISE` just before `BOX KEY` when it links off the cable only through a Noise session
    // (ADR-0015); from then on this computer never links to it in the clear. Only the cable is believed, both ways.
    if let DeviceMessage::Line(line) = &message
        && on_cable
    {
        use std::sync::atomic::Ordering;
        if line.trim() == "LINK NOISE" {
            state.noise_seen.store(true, Ordering::Relaxed);
        } else if let Some(key) = line.strip_prefix(status::BOX_KEY_PREFIX)
            && let Some(dir) = &state.state_dir
        {
            let memory = noise_link::Memory::new(dir);
            match lan::decode_key(key.trim()) {
                Some(key) if state.noise_seen.swap(false, Ordering::Relaxed) => memory.learned(&key),
                _ => memory.forget(),
            }
        }
    }
    if let DeviceMessage::Line(line) = &message
        && on_cable
        && let Some(key) = line.strip_prefix(status::BOX_KEY_PREFIX)
        && let Some(identity) = &state.identity
    {
        send_event(state, pair_command(identity));
        send_event(state, device_command("device.pairs"));
        // The box uses the Relay its computer on the cable uses, so a development build and its box
        // meet on staging and a release and its box on production (docs/relay-protocol.md, "Which Relay").
        if let Some(url) = state.relay.as_ref().and_then(relay::RelayConfig::for_box) {
            let mut event = device_command("device.relay.set");
            event.extra.insert("url".to_owned(), url.into());
            send_event(state, event);
        }
        // Remembered so the box can be found and checked over the network later (ADR-0012). Only from the cable:
        // a key reported over the network or the Relay could be anyone's, and a new key after the box made itself
        // a new identity is believed only from the box itself (#122).
        if let Some(dir) = &state.state_dir {
            lan::remember(&lan::box_key_file(dir), key.trim());
        }
    }
    if let DeviceMessage::Line(line) = &message
        && on_cable
        && let Some(address) = line.strip_prefix(WIFI_ADDRESS_PREFIX)
        && let Some(dir) = &state.state_dir
    {
        lan::remember(&lan::box_address_file(dir), address.trim());
    }
    if let DeviceMessage::Line(line) = &message {
        handle_voice_line(state, line).await;
    }
    let _ = state.device_bus.send(message.clone());
    // Ask right after connecting; the device reports its mode, firmware build and voice again.
    if matches!(message, DeviceMessage::Connected { .. } | DeviceMessage::ConnectedNetwork { .. } | DeviceMessage::ConnectedRelay { .. }) {
        send_event(state, device_command("device.hello"));
    }
    let DeviceMessage::Event(event) = message else {
        return;
    };
    if is_k2_press(&event) {
        open_k2_source(state).await;
    } else if let Some(card) = tapped_card(&event) {
        open_card_source(state, card).await;
    } else if !event.event.starts_with("voice.") && event.event != "echo" {
        info!(event = %event.event, "ignoring unbound device event");
    }
}

/// Voice commands (docs/protocol.md, "Voice commands"): offered on every report of the box's build, which follows
/// hello on each new link and every reboot, and taken over native USB or the local network. The box sends no audio
/// over the UART bridge, which is too slow for it, and over the Relay it will talk to the voice service itself, so
/// voice isn't offered on either, and voice lines over the Relay are ignored.
async fn handle_voice_line(state: &AppState, line: &str) -> Option<tokio::task::JoinHandle<()>> {
    let (over_relay, on_bridge) = {
        let device = state.device.lock().await;
        (device.relay, device.bridge)
    };
    if line.contains(serial_transport::BUILD_MARKER) && state.voice.available() {
        if over_relay {
            info!("voice commands not offered: the box is reached through the Relay");
        } else if on_bridge {
            info!("voice commands not offered: the box is on the UART bridge, which can't carry its audio; use its native USB port");
        } else {
            send_event(state, voice::available_event(true));
        }
    }
    if !line.starts_with("VOICE ") {
        return None;
    }
    if over_relay {
        tracing::debug!("ignoring a voice line over the Relay");
        return None;
    }
    state.voice.handle_line(line, &state.transport, &wearing(state).await)
}

/// What the buddy wears for spoken replies: the Character the box reported wearing, before it has the one last
/// written to it, with what the App said when it wrote it (a custom Character's lender, the form of address).
async fn wearing(state: &AppState) -> persona::Wearing {
    let reported = state.device.lock().await.voice.clone();
    let id = match reported {
        Some(id) => Some(id),
        None => state.config.lock().await.voice.clone(),
    };
    persona::Wearing::of(id, &*state.written.lock().await)
}

/// Tells a box that has reported its build whether voice is on now, after the resident session's readiness or the account flipped
/// (the user's Codex installed or gone, logged in or out). Told `false`, a long K0 gives up the Pomodoro phase as it
/// did before voice. Never over the Relay or the UART bridge, where voice is never offered.
async fn offer_voice_again(state: &AppState) {
    let (booted, over_relay, on_bridge) = {
        let device = state.device.lock().await;
        (device.connected && device.firmware_build.is_some(), device.relay, device.bridge)
    };
    if booted && !over_relay && !on_bridge {
        let on = state.voice.available();
        info!(on, "voice readiness changed; telling the box whether voice is on");
        send_event(state, voice::available_event(on));
    }
}

/// Signing in or out, here or by the service refusing the session, changes the status and whether voice works.
fn follow_account(state: &AppState) {
    let mut changes = state.account.watch();
    let follower = state.clone();
    tokio::spawn(async move {
        while changes.changed().await.is_ok() {
            follower.notify_status();
            offer_voice_again(&follower).await;
        }
    });
}

/// How long a voice token is used before the next is fetched; it lasts an hour.
const VOICE_TOKEN_RENEW: Duration = Duration::from_secs(45 * 60);
/// How soon to try again when the service gave none.
const VOICE_TOKEN_RETRY: Duration = Duration::from_secs(60);

/// The box a voice token is due for, if any: linked through the Relay and saying `VOICE DIRECT`, while this computer is
/// signed in and recognition is the voice service's. With the user's own recognizer the audio comes over the link
/// instead, since the box can't hold their key (ADR-0018).
fn voice_token_box(device: &DeviceState, signed_in: bool, uses_voice_service: bool) -> Option<String> {
    (device.connected && device.relay && device.voice_direct && signed_in && uses_voice_service).then(|| device.box_key.clone()).flatten()
}

/// Keeps a box on the Relay supplied with voice tokens (docs/protocol.md, "Voice commands"): one as soon as it says
/// `VOICE DIRECT`, which it does on every new link and reboot (its token lives only in RAM), then a fresh one every
/// [`VOICE_TOKEN_RENEW`]. Nothing while the conditions of [`voice_token_box`] don't hold.
async fn hand_out_voice_tokens(state: AppState) {
    let mut status = state.status_changed.subscribe();
    let mut lines = state.device_bus.subscribe();
    // The box last given a token, and when the next is due.
    let mut handed: Option<(String, tokio::time::Instant)> = None;
    loop {
        let wanted = {
            let device = state.device.lock().await;
            voice_token_box(&device, state.account.status().signed_in, state.voice.uses_voice_service())
        };
        let due = match wanted {
            None => {
                handed = None;
                None
            }
            Some(box_key) => {
                let now = tokio::time::Instant::now();
                if handed.as_ref().is_none_or(|(given, at)| *given != box_key || *at <= now) {
                    let (account, key) = (state.account.clone(), box_key.clone());
                    let fetched = tokio::task::spawn_blocking(move || account.voice_token(&key))
                        .await
                        .unwrap_or_else(|error| Err(account::AccountError::Service(error.to_string())));
                    let next = match fetched {
                        Ok((token, expires_at)) => {
                            send_event(&state, voice::token_event(&token, &expires_at));
                            info!(%expires_at, "gave the box on the Relay a voice token");
                            now + VOICE_TOKEN_RENEW
                        }
                        Err(error) => {
                            warn!(%error, "no voice token for the box on the Relay; trying again soon");
                            now + VOICE_TOKEN_RETRY
                        }
                    };
                    handed = Some((box_key, next));
                }
                handed.as_ref().map(|(_, at)| *at)
            }
        };
        tokio::select! {
            _ = status.recv() => {}
            message = lines.recv() => {
                // A box that says it again has just linked or restarted, and lost its token.
                if matches!(&message, Ok(DeviceMessage::Line(line)) if line.trim() == "VOICE DIRECT") {
                    handed = None;
                }
            }
            _ = async {
                match due {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {}
        }
    }
}

/// Checks the resident session's readiness every [`resident::READINESS_TTL`] and tells the box each time it flips.
fn follow_resident_readiness(state: &AppState, resident: Arc<resident::Resident>) {
    let mut flips = resident.watch_ready();
    let follower = state.clone();
    tokio::spawn(async move {
        while flips.changed().await.is_ok() {
            offer_voice_again(&follower).await;
        }
    });
    tokio::spawn(async move {
        loop {
            resident.check_readiness().await;
            tokio::time::sleep(resident::READINESS_TTL).await;
        }
    });
}

async fn open_k2_source(state: &AppState) {
    {
        let sources = state.activities.lock().await.focus_sources();
        if sources.is_empty() {
            info!("K2 pressed, but there is no activity to open");
            return;
        }
        for source in sources {
            // Check a Codex thread still exists first: opening a missing thread gives a blank session.
            if let ActivitySource::Codex { thread_id, .. } = &source
                && state.titles.lock().await.codex_thread_known(thread_id) == Some(false)
            {
                warn!(%thread_id, "K2 skipped a Codex thread that no longer exists");
                continue;
            }
            match source_opener::open(source).await {
                // Log the link itself: when it jumps to the wrong place, the log should say exactly where it went.
                Ok(link) => {
                    info!(%link, "K2 opened the current activity's source");
                    break;
                }
                Err(error) => warn!(%error, "K2 failed to open source, trying the next candidate"),
            }
        }
    }
}

/// A tap on a task card on a box with a touchscreen (ADR-0019): the card's id.
fn tapped_card(event: &Event) -> Option<&str> {
    (event.event == "focus").then(|| event.extra.get("task").and_then(|value| value.as_str())).flatten()
}

/// Opens the agent behind a tapped card. If it has ended since, or has nothing to open, nothing happens: opening
/// something other than what was tapped would be more confusing than nothing.
async fn open_card_source(state: &AppState, card: &str) {
    let Some(source) = state.activities.lock().await.card_source(card) else {
        info!(%card, "a tapped card's activity is gone or has no source");
        return;
    };
    match source_opener::open(source).await {
        Ok(link) => info!(%link, "opened a tapped card's source"),
        Err(error) => warn!(%error, "failed to open a tapped card's source"),
    }
}

fn is_k2_press(event: &Event) -> bool {
    event.event == "button"
        && event.extra.get("button").and_then(|value| value.as_str()) == Some("K2")
        && event.extra.get("action").and_then(|value| value.as_str()) == Some("press")
}

/// When the app supervises it, the app puts its pid in VIBEBUDDY_PARENT_PID. If the app is force-killed, the daemon
/// is adopted by launchd and its parent pid becomes 1; then exit too, rather than holding the serial port and HTTP port so
/// the next app can't start. macOS has no prctl(PR_SET_PDEATHSIG), so this has to poll.
async fn watch_parent() {
    let Some(expected) = env::var("VIBEBUDDY_PARENT_PID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return;
    };
    let mut ticker = tokio::time::interval(Duration::from_secs(2));
    loop {
        ticker.tick().await;
        if std::os::unix::process::parent_id() != expected {
            info!(expected, "the supervising app is gone, exiting too");
            std::process::exit(0);
        }
    }
}

/// The daemon's build identifier: the git description plus the binary's own timestamp.
///
/// The timestamp is the executable's mtime, not a compile-time constant. `build.rs` only reruns when its declared
/// dependencies change; when you edit one line and relink, the time recorded at compile time doesn't update, lying to
/// you exactly when you most need it to be accurate.
fn build_identity(app_version: Option<&str>) -> String {
    let built = std::env::current_exe()
        .and_then(|path| path.metadata())
        .and_then(|metadata| metadata.modified())
        .ok()
        .map(|time| {
            chrono::DateTime::<chrono::Local>::from(time)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    build_identity_from(app_version, BUILD_REVISION, &built)
}

/// When the app is present its version comes first: the device's APP row shows the app version plus the build.
fn build_identity_from(app_version: Option<&str>, revision: &str, built: &str) -> String {
    let mut parts = Vec::new();
    if let Some(version) = app_version {
        parts.push(version);
    }
    parts.push(revision);
    parts.push(built);
    parts.join(" ").trim_end().to_owned()
}

/// Where today's stats are stored. Without `HOME`, fall back to in-memory counts so the daemon still starts.
fn stats_file() -> Option<PathBuf> {
    if let Ok(path) = env::var("VIBEBUDDY_STATS_FILE") {
        return Some(PathBuf::from(path));
    }
    Some(config::state_dir()?.join("stats.json"))
}

/// The USB serial number of the device that last proved to be the box.
fn known_box_file() -> Option<PathBuf> {
    Some(config::state_dir()?.join("box-usb-serial"))
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/v1/events", post(post_event))
        .route("/v1/codex-hooks", post(post_codex_hook))
        .route("/v1/claude-hooks", post(post_claude_hook))
        .route("/v1/agent-hooks", post(post_agent_hook))
        .route("/v1/status", get(get_status))
        .route("/v1/status/stream", get(status_stream))
        .route("/v1/config", get(get_config).put(put_config))
        .route("/v1/device/identify", post(post_identify))
        .route("/v1/device/volume", post(post_volume))
        .route("/v1/device/unpair", post(post_unpair))
        .route("/v1/device/takeover", post(post_takeover))
        .route("/v1/device/wifi", post(post_wifi))
        .route("/v1/device/wifi/forget", post(post_wifi_forget))
        .route("/v1/device/wifi/scan", post(post_wifi_scan))
        .route("/v1/device/choice", post(post_choice))
        .route(
            "/v1/device/voice-pack",
            post(post_voice_pack).layer(DefaultBodyLimit::max(4 * 1024 * 1024)),
        )
        .route("/v1/device/screenshot", post(post_screenshot))
        .route("/v1/device/firmware", post(post_firmware))
        .route("/v1/daemon/restart", post(post_restart))
        .route("/v1/updates/check", post(post_check_updates))
        .route("/v1/resident", get(get_resident))
        .route("/v1/resident/say", post(post_resident_say))
        .route("/v1/resident/history", get(get_resident_history))
        .route("/v1/resident/brain", axum::routing::put(put_resident_brain))
        .route("/v1/resident/anthropic-key", axum::routing::put(put_anthropic_key).delete(delete_anthropic_key))
        .route("/v1/account", axum::routing::delete(delete_account))
        .route("/v1/account/sign-in", post(post_sign_in))
        .route("/v1/account/callback", get(get_account_callback))
        .route("/v1/account/sign-out", post(post_sign_out))
        .with_state(state)
}

/// Capture the box's current screen and return a PNG. Refused while writing a voice pack or flashing: screenshot lines would crowd out acks.
async fn post_screenshot(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    if matches!(&*state.operation.lock().await, Some(current) if current.state == OperationState::Running) {
        return (StatusCode::CONFLICT, "another device operation is in progress").into_response();
    }
    let bus = state.device_bus.subscribe();
    let silence = screenshot::silence_timeout(state.device.lock().await.bridge);
    match screenshot::capture(state.transport.clone(), bus, silence).await {
        Ok(frame) => match screenshot::encode_png(&frame) {
            Ok(png) => ([(axum::http::header::CONTENT_TYPE, "image/png")], png).into_response(),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
        },
        Err(error) => {
            // Without this line a box that ignores the request leaves no trace at all.
            warn!(%error, "screenshot failed");
            (StatusCode::BAD_GATEWAY, error).into_response()
        }
    }
}

async fn get_status(State(state): State<AppState>) -> Json<Status> {
    Json(state.snapshot().await)
}

/// SSE: push one snapshot on connect, then a full snapshot whenever the status changes.
async fn status_stream(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    let changes = BroadcastStream::new(state.status_changed.subscribe()).map(|_| ());
    let stream = tokio_stream::once(()).chain(changes).then(move |()| {
        let state = state.clone();
        async move {
            let status = state.snapshot().await;
            let data = serde_json::to_string(&status).unwrap_or_default();
            Ok(SseEvent::default().event("status").data(data))
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn get_config(State(state): State<AppState>) -> Json<Config> {
    Json(state.config.lock().await.clone())
}

/// At start, once a day, and when asked. The first check waits a little, so the box has reported the firmware
/// version that goes with the request; being asked doesn't wait.
async fn check_for_updates(state: AppState) {
    let mut wait = Duration::from_secs(30);
    loop {
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = state.check_updates.notified() => {}
        }
        if state.update_checks_enabled().await {
            check_once(&state).await;
        }
        wait = updates::CHECK_EVERY;
    }
}

async fn check_once(state: &AppState) {
    let Some(source) = state.updates.lock().await.source.clone() else { return };
    let firmware_version = state.device.lock().await.firmware_version.clone();
    let fetched = tokio::task::spawn_blocking(move || updates::fetch(&source, env!("CARGO_PKG_VERSION"), firmware_version.as_deref()))
        .await
        .unwrap_or_else(|error| Err(error.to_string()));
    let wanted = match fetched {
        Ok((text, manifest)) => {
            let mut updates = state.updates.lock().await;
            let accepted = updates.accept(&text, manifest).map(|firmware| (firmware, updates.firmware_root()));
            if let (Some(relay), Some(url)) = (&state.relay, updates.relay_url()) {
                relay.offer(&url);
                state.device.lock().await.relay_server = Some(relay.url());
            }
            accepted
        }
        Err(error) => Err(error),
    };
    let error = match wanted {
        Ok((Some(firmware), Some(root))) => {
            let version = firmware.download.version.clone();
            let downloaded = tokio::task::spawn_blocking(move || updates::download(&firmware, &root))
                .await
                .unwrap_or_else(|error| Err(error.to_string()));
            match downloaded {
                Ok(dir) => {
                    info!(%version, dir = %dir.display(), "downloaded firmware");
                    None
                }
                Err(error) => Some(error),
            }
        }
        Ok(_) => None,
        Err(error) => Some(error),
    };
    if let Some(error) = &error {
        warn!(%error, "update check failed");
    }
    state.updates.lock().await.finish(error);
    state.notify_status();
}

async fn post_check_updates(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse>) {
    if !state.update_checks_enabled().await || state.updates.lock().await.source.is_none() {
        return (StatusCode::CONFLICT, Json(ApiResponse { accepted: false, message: "update checks are off".to_owned() }));
    }
    state.check_updates.notify_one();
    (StatusCode::ACCEPTED, Json(ApiResponse { accepted: true, message: "checking for updates".to_owned() }))
}

async fn put_config(State(state): State<AppState>, Json(config): Json<Config>) -> Json<Config> {
    if let Some(serial) = &state.serial {
        serial.set_relay_enabled(config.relay);
    }
    state.save_config(|current| *current = config).await;
    Json(state.config.lock().await.clone())
}

fn device_command(name: &str) -> Event {
    Event::named(name)
}

/// The address the box got on Wi-Fi, which it reports over the cable.
const WIFI_ADDRESS_PREFIX: &str = "WIFI ADDRESS ";

fn pair_command(identity: &identity::Identity) -> Event {
    let mut event = device_command("device.pair");
    event.extra.insert("key".to_owned(), identity.public_key().into());
    event.extra.insert("name".to_owned(), identity.name.clone().into());
    event
}

/// Only one operation can run on the device at a time: returns None once the slot is taken, otherwise a 409 for the caller.
async fn begin_operation(
    state: &AppState,
    kind: OperationKind,
    message: String,
) -> Option<(StatusCode, Json<ApiResponse>)> {
    let mut operation = state.operation.lock().await;
    if matches!(&*operation, Some(current) if current.state == OperationState::Running) {
        return Some((
            StatusCode::CONFLICT,
            Json(ApiResponse { accepted: false, message: "another device operation is in progress".to_owned() }),
        ));
    }
    *operation = Some(Operation { kind, state: OperationState::Running, progress: 0.0, message });
    drop(operation);
    state.notify_status();
    None
}

/// Report progress from a blocking thread or the write loop. Only written while the operation is running: a late callback
/// after completion must not turn 100% back.
fn report_progress(state: &AppState, fraction: f32, message: Option<String>) {
    let state = state.clone();
    tokio::spawn(async move {
        if let Some(operation) = state.operation.lock().await.as_mut()
            && operation.state == OperationState::Running
        {
            operation.progress = fraction;
            if let Some(message) = message {
                operation.message = message;
            }
        }
        state.notify_status();
    });
}

/// Changes only what a running operation says, as when an update waits for the link.
fn report_message(state: &AppState, message: &str) {
    let state = state.clone();
    let message = message.to_owned();
    tokio::spawn(async move {
        if let Some(operation) = state.operation.lock().await.as_mut()
            && operation.state == OperationState::Running
        {
            operation.message = message;
        }
        state.notify_status();
    });
}

async fn post_identify(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse>) {
    post_event(State(state), Json(device_command("device.identify"))).await
}

/// Matches the firmware's `AGENT_AUDIO_VOLUME_MIN/MAX`: the floor is above zero; muting has its own button and isn't persisted.
const VOLUME_RANGE: std::ops::RangeInclusive<u8> = 20..=100;

#[derive(serde::Deserialize)]
struct VolumeRequest {
    level: u8,
    /// Have the box play "task complete" at the new volume, so the slider isn't adjusted blind.
    #[serde(default)]
    preview: bool,
}

/// Set the volume: out-of-range values are rejected rather than adjusted for the user. The device answers with a `VOLUME` line
/// once applied, the status volume follows, and the app always shows the value on the box.
async fn post_volume(
    State(state): State<AppState>,
    Json(request): Json<VolumeRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    if !VOLUME_RANGE.contains(&request.level) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                accepted: false,
                message: format!("volume must be between {} and {}", VOLUME_RANGE.start(), VOLUME_RANGE.end()),
            }),
        );
    }
    let mut event = device_command("device.volume");
    event.extra.insert("level".to_owned(), request.level.into());
    if request.preview {
        event.extra.insert("preview".to_owned(), true.into());
    }
    post_event(State(state), Json(event)).await
}

#[derive(serde::Deserialize)]
struct UnpairRequest {
    key: String,
}

/// Forget another computer on the box. Not this one: it pairs again whenever it is plugged in.
async fn post_unpair(
    State(state): State<AppState>,
    Json(request): Json<UnpairRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    let refusal = {
        let device = state.device.lock().await;
        if device.computer_key.as_deref() == Some(request.key.as_str()) {
            Some("this computer pairs again whenever it is plugged in")
        } else if !device.connected || device.box_key.is_none() {
            Some("no box that can pair is connected")
        } else {
            None
        }
    };
    if let Some(message) = refusal {
        return (StatusCode::CONFLICT, Json(ApiResponse { accepted: false, message: message.to_owned() }));
    }
    let mut event = device_command("device.unpair");
    event.extra.insert("key".to_owned(), request.key.into());
    post_event(State(state), Json(event)).await
}

/// The user chose to take the box from the computer that has it: the next try over the network or the Relay
/// asks for it outright (ADR-0011). Not from another computer's cable, which the box refuses.
async fn post_takeover(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse>) {
    let refusal = {
        let device = state.device.lock().await;
        match &device.busy {
            _ if device.connected => Some("this computer already has the box"),
            Some(busy) if busy.reason == "usb" => Some("the box is on another computer's cable"),
            None => Some("the box isn't linked elsewhere"),
            Some(_) => None,
        }
    };
    if let Some(message) = refusal {
        return (StatusCode::CONFLICT, Json(ApiResponse { accepted: false, message: message.to_owned() }));
    }
    let Some(serial) = &state.serial else {
        return (StatusCode::NOT_IMPLEMENTED, Json(ApiResponse { accepted: false, message: "no link worker".to_owned() }));
    };
    serial.request_takeover();
    (StatusCode::ACCEPTED, Json(ApiResponse { accepted: true, message: "taking the box over".to_owned() }))
}

#[derive(serde::Deserialize)]
struct WifiRequest {
    ssid: String,
    password: String,
}

/// Tells the box which Wi-Fi network to join. Only over the cable: the box refuses it over the network, and
/// firmware that can't pair would show it as agent activity. The password is passed on, never logged or kept.
async fn post_wifi(State(state): State<AppState>, Json(request): Json<WifiRequest>) -> (StatusCode, Json<ApiResponse>) {
    if let Some(refused) = wifi_refusal(&state).await {
        return refused;
    }
    if request.ssid.is_empty() || request.ssid.len() > 32 || request.password.len() > 63 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse { accepted: false, message: "a network name is 1 to 32 bytes and a password at most 63".to_owned() }),
        );
    }
    let mut event = device_command("device.wifi.set");
    event.extra.insert("ssid".to_owned(), request.ssid.into());
    event.extra.insert("password".to_owned(), request.password.into());
    let response = post_event(State(state.clone()), Json(event)).await;
    if response.1.accepted {
        // The last failure was the old password's: this try says how it went once the box has tried.
        state.device.lock().await.wifi_error = None;
        state.notify_status();
    }
    response
}

async fn post_wifi_forget(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse>) {
    if let Some(refused) = wifi_refusal(&state).await {
        return refused;
    }
    post_event(State(state), Json(device_command("device.wifi.forget"))).await
}

/// Asks the box which networks it can hear; the answer comes back as `device.wifi_seen` in the status.
async fn post_wifi_scan(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse>) {
    if let Some(refused) = wifi_refusal(&state).await {
        return refused;
    }
    post_event(State(state), Json(device_command("device.wifi.scan"))).await
}

async fn wifi_refusal(state: &AppState) -> Option<(StatusCode, Json<ApiResponse>)> {
    let device = state.device.lock().await;
    let message = if !device.connected || device.box_key.is_none() {
        "no box that can join Wi-Fi is connected"
    } else if device.network {
        "the box is linked over Wi-Fi; plug in the cable to change its network"
    } else {
        return None;
    };
    Some((StatusCode::CONFLICT, Json(ApiResponse { accepted: false, message: message.to_owned() })))
}

#[derive(serde::Deserialize)]
struct ChoiceRequest {
    usb_serial: String,
}

/// The user picked which of several devices is the box: remember it as the box, and the serial worker connects
/// to it on its next look around, as it would to a box that had reported our build.
async fn post_choice(
    State(state): State<AppState>,
    Json(request): Json<ChoiceRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    let offered = state.device.lock().await.candidates.iter().any(|candidate| {
        candidate.usb_serial.as_deref().is_some_and(|serial| serial_transport::serials_equal(serial, &request.usb_serial))
    });
    if !offered {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse { accepted: false, message: "that device isn't one of the candidates".to_owned() }),
        );
    }
    let Some(path) = known_box_file() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse { accepted: false, message: "no state directory to remember the box in".to_owned() }),
        );
    };
    serial_transport::remember_box(&path, &request.usb_serial);
    info!(usb_serial = %request.usb_serial, "the user chose the box");
    (StatusCode::ACCEPTED, Json(ApiResponse { accepted: true, message: "remembered".to_owned() }))
}

/// What the App may say about a Character pack it writes, which the pack itself doesn't carry: the preset lending a
/// custom Character its voice, persona and lines, and the form of address its lines were made with.
#[derive(Default, serde::Deserialize)]
struct PackQuery {
    lender: Option<String>,
    address: Option<String>,
}

/// Write a voice pack: the request body is the pack itself. The write runs in the background with progress in the status stream.
async fn post_voice_pack(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<PackQuery>,
    body: Bytes,
) -> (StatusCode, Json<ApiResponse>) {
    let Some(voice) = voice_writer::voice_id_of(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse { accepted: false, message: "request body is not a voice pack".to_owned() }),
        );
    };
    if let Some(refused) = begin_operation(&state, OperationKind::VoicePack, format!("writing {voice}")).await {
        return refused;
    }
    let bus = state.device_bus.subscribe();
    let pack = body.to_vec();
    let task_state = state.clone();
    tokio::spawn(async move {
        let progress_state = task_state.clone();
        // Firmware that takes transfers gets the pack that way: windowed, resumable, over any link (ADR-0016).
        let over_the_link = task_state.device.lock().await.transfer;
        let result = if over_the_link {
            let resuming_state = task_state.clone();
            transfer::send_image(
                task_state.transport.clone(),
                bus,
                "character",
                pack,
                move |fraction| report_progress(&progress_state, fraction, None),
                move || report_message(&resuming_state, "waiting for the link to come back"),
            )
            .await
            .and_then(|done| done.extra.get("voice").and_then(|voice| voice.as_str()).map(str::to_owned).ok_or_else(|| "the box didn't say which Character it has".to_owned()))
        } else {
            voice_writer::write_pack(task_state.transport.clone(), bus, pack, move |fraction| {
                report_progress(&progress_state, fraction, None);
            })
            .await
        };
        match result {
            Ok(written) => {
                info!(voice = %written, "voice pack written to device");
                task_state.save_config(|config| config.voice = Some(written.clone())).await;
                let record = persona::Written { id: Some(written.clone()), lender: query.lender, address: query.address };
                if let Some(dir) = &task_state.state_dir
                    && let Err(error) = record.save(&dir.join(persona::RECORD_FILE))
                {
                    warn!(%error, "failed to save what the App said about the Character");
                }
                *task_state.written.lock().await = record;
                task_state
                    .set_operation(Some(Operation {
                        kind: OperationKind::VoicePack,
                        state: OperationState::Done,
                        progress: 1.0,
                        message: format!("wrote {written}"),
                    }))
                    .await;
            }
            Err(error) => {
                warn!(%error, "voice pack write failed");
                task_state
                    .set_operation(Some(Operation {
                        kind: OperationKind::VoicePack,
                        state: OperationState::Failed,
                        progress: 0.0,
                        message: error,
                    }))
                    .await;
            }
        }
    });
    (
        StatusCode::ACCEPTED,
        Json(ApiResponse { accepted: true, message: format!("started writing {voice}") }),
    )
}

#[derive(serde::Deserialize)]
struct FirmwareRequest {
    bootloader: PathBuf,
    partition_table: PathBuf,
    app: PathBuf,
}

/// Flash firmware: the app supplies the three image paths (all inside its bundle); a blank otadata, when the table has
/// one, goes with them so the box boots the app just written. The serial worker releases the port,
/// the ROM protocol writes and verifies each segment, then hard-resets and the worker reconnects. Progress goes through the status stream.
async fn post_firmware(
    State(state): State<AppState>,
    Json(request): Json<FirmwareRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    let mut segments = Vec::new();
    for (address, path) in [(0x0_u32, &request.bootloader), (0x8000, &request.partition_table), (0x10000, &request.app)] {
        match std::fs::read(path) {
            Ok(data) if !data.is_empty() => {
                if address == 0x8000 {
                    segments.extend(rom_flasher::blank_otadata(&data));
                }
                segments.push(rom_flasher::Segment { address, data });
            }
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse { accepted: false, message: format!("cannot read firmware file {}", path.display()) }),
                );
            }
        }
    }
    let (port, bridge) = {
        let device = state.device.lock().await;
        match &device.port {
            // Firmware that takes images over the link gets one that way, over whichever it is: it can roll back,
            // and it needs no cable (ADR-0016). The bootloader and partition table never change after the OTA layout.
            Some(_) if device.connected && device.transfer => {
                drop(device);
                return update_over_the_link(state, segments).await;
            }
            Some(_) if device.connected && device.network => {
                return (
                    StatusCode::CONFLICT,
                    Json(ApiResponse { accepted: false, message: "the box is linked over Wi-Fi; plug in the cable to flash it".to_owned() }),
                );
            }
            Some(port) if device.connected => (port.clone(), device.bridge),
            _ => {
                return (
                    StatusCode::CONFLICT,
                    Json(ApiResponse { accepted: false, message: "no box connected".to_owned() }),
                );
            }
        }
    };
    let Some(serial) = state.serial.clone() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(ApiResponse { accepted: false, message: "no serial worker".to_owned() }),
        );
    };
    if let Some(refused) = begin_operation(&state, OperationKind::Firmware, "releasing the serial port".to_owned()).await {
        return refused;
    }
    let task_state = state.clone();
    tokio::spawn(async move {
        serial.set_suspended(true);
        // Wait until the worker has actually let go of the port.
        tokio::time::sleep(Duration::from_millis(800)).await;
        let progress_state = task_state.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut report = |fraction: f32, message: &str| {
                report_progress(&progress_state, fraction, Some(message.to_owned()));
            };
            rom_flasher::flash(&port, bridge, &segments, &mut report)
        })
        .await
        .unwrap_or_else(|error| Err(format!("flash task crashed: {error}")));
        if result.is_ok() {
            // Forget the old build before the worker reconnects, so only the new firmware's own report counts as booted.
            task_state.device.lock().await.firmware_build = None;
        }
        serial.set_suspended(false);
        match result {
            Ok(()) => {
                info!("firmware flashed, waiting for the device to restart");
                await_first_boot(&task_state, FIRMWARE_BOOT_TIMEOUT).await;
            }
            Err(error) => {
                warn!(%error, "firmware flash failed");
                let operation = Operation { kind: OperationKind::Firmware, state: OperationState::Failed, progress: 0.0, message: error };
                task_state.set_operation(Some(operation)).await;
            }
        }
    });
    (StatusCode::ACCEPTED, Json(ApiResponse { accepted: true, message: "started flashing".to_owned() }))
}

/// The reset after flashing doesn't always start the new firmware: a box put into download mode by hand (K0 held while
/// plugging in) stays there until it loses power, with a dark screen. Only the firmware's own `DISPLAY READY` proves it
/// booted; until then the flash isn't done, and if it never comes the user has to replug the box. The replug wait has no
/// deadline: a stale `replug` would keep the app's onboarding from ever showing the box as found, however late it boots.
/// How long a box updated over the link may take to come back: restart, Wi-Fi, and the daemon finding it again.
const OTA_BOOT_TIMEOUT: Duration = Duration::from_secs(180);
/// Long enough for an image that fails before confirming itself to restart and be rolled back (seen on a box: about 2 s).
const OTA_SETTLE: Duration = Duration::from_secs(8);

/// Sends the app image over the link (ADR-0016), then waits for the box to come back on it. Success is the box
/// reporting its build again without `FIRMWARE ROLLED BACK`.
async fn update_over_the_link(state: AppState, segments: Vec<rom_flasher::Segment>) -> (StatusCode, Json<ApiResponse>) {
    let Some(image) = segments.into_iter().find(|segment| segment.address == 0x10000).map(|segment| segment.data) else {
        return (StatusCode::BAD_REQUEST, Json(ApiResponse { accepted: false, message: "no app image".to_owned() }));
    };
    if let Some(refused) = begin_operation(&state, OperationKind::Firmware, "sending the firmware to the box".to_owned()).await {
        return refused;
    }
    let task_state = state.clone();
    tokio::spawn(async move {
        let bus = task_state.device_bus.subscribe();
        let progress_state = task_state.clone();
        let resuming_state = task_state.clone();
        let result = transfer::send_image(
            task_state.transport.clone(),
            bus,
            "firmware",
            image,
            move |fraction| report_progress(&progress_state, fraction * 0.95, Some("sending the firmware to the box".to_owned())),
            move || report_message(&resuming_state, "waiting for the link to come back"),
        )
        .await;
        let firmware = |state, progress, message: &str| Some(Operation { kind: OperationKind::Firmware, state, progress, message: message.to_owned() });
        if let Err(error) = result {
            warn!(%error, "firmware update over the link failed");
            task_state.set_operation(firmware(OperationState::Failed, 0.0, &error)).await;
            return;
        }
        info!("firmware sent over the link, waiting for the box to restart");
        // Forget the old build, so only what the box reports after its restart counts.
        task_state.device.lock().await.firmware_build = None;
        task_state.set_operation(firmware(OperationState::Running, 0.95, "waiting for the box to restart")).await;
        if !wait_for_firmware_build(&task_state, OTA_BOOT_TIMEOUT).await {
            task_state.set_operation(firmware(OperationState::Failed, 0.0, "the box didn't come back after the update")).await;
            return;
        }
        // On the cable the new image reports its build before it confirms itself; one that fails in between restarts
        // at once, and the bootloader goes back. Over the network the box is reached only after it confirmed. So the
        // answer that counts is the one after a short wait, and the rollback comes right after its build.
        tokio::time::sleep(OTA_SETTLE).await;
        if !wait_for_firmware_build(&task_state, OTA_BOOT_TIMEOUT).await {
            task_state.set_operation(firmware(OperationState::Failed, 0.0, "the box didn't come back after the update")).await;
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        if task_state.device.lock().await.rolled_back {
            warn!("the new firmware didn't confirm itself; the box went back");
            task_state.set_operation(firmware(OperationState::Failed, 0.0, "the update didn't take; the box went back to its previous firmware")).await;
        } else {
            task_state.set_operation(firmware(OperationState::Done, 1.0, "updated, box restarted")).await;
        }
    });
    (StatusCode::ACCEPTED, Json(ApiResponse { accepted: true, message: "sending the firmware to the box".to_owned() }))
}

async fn await_first_boot(state: &AppState, boot_timeout: Duration) {
    let firmware = |state, message: &str| Some(Operation { kind: OperationKind::Firmware, state, progress: 1.0, message: message.to_owned() });
    state.set_operation(firmware(OperationState::Running, "waiting for the box to restart")).await;
    if wait_for_firmware_build(state, boot_timeout).await {
        state.set_operation(firmware(OperationState::Done, "flash complete, box restarted")).await;
        return;
    }
    warn!("the box did not start the new firmware, asking for a replug");
    state.set_operation(firmware(OperationState::Replug, "firmware flashed but not started, replug the box")).await;
    firmware_build_reported(state).await;
    let mut operation = state.operation.lock().await;
    // Another operation may have started in the meantime; only finish our own.
    if operation.as_ref().is_some_and(|operation| operation.kind == OperationKind::Firmware && operation.state == OperationState::Replug) {
        *operation = firmware(OperationState::Done, "flash complete, box restarted");
        drop(operation);
        state.notify_status();
    }
}

async fn wait_for_firmware_build(state: &AppState, timeout: Duration) -> bool {
    tokio::time::timeout(timeout, firmware_build_reported(state)).await.is_ok()
}

async fn firmware_build_reported(state: &AppState) {
    while state.device.lock().await.firmware_build.is_none() {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The app supervises the daemon: exiting means restarting. Send the response first, then exit.
async fn post_restart() -> (StatusCode, Json<ApiResponse>) {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        info!("exiting at the app's request; it will relaunch us");
        std::process::exit(0);
    });
    (StatusCode::ACCEPTED, Json(ApiResponse { accepted: true, message: "daemon is restarting".to_owned() }))
}

/// Periodically tell the device the link is still alive.
///
/// Without heartbeats, after a daemon crash or serial disconnect the device would keep showing the last state,
/// looking as if a task were still running. The worst failure for a status device is showing stale state without knowing it.
async fn send_heartbeats(state: AppState) {
    let build = build_identity(state.app_version.as_deref());
    info!(build = %build, "Mac-side build id");
    let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        ticker.tick().await;
        let now = chrono::Local::now();
        let heartbeat = heartbeat_event(&build, now.hour(), local_day(&now));
        match heartbeat.to_ndjson() {
            // A full queue means the device isn't receiving anything; a heartbeat is pointless then, so just drop it.
            Ok(frame) => drop(state.transport.send(frame)),
            Err(error) => warn!(%error, "failed to encode heartbeat"),
        }
    }
}

/// The heartbeat piggybacks three things: build identifier, local hour and local date. All are repeated with every heartbeat
/// because the device may restart at any time and a one-off handshake would be lost. The hour tells the buddy whether it's day
/// or night, and the date tells the pomodoro when a new day starts: the device has no clock and shouldn't
/// join Wi-Fi just for that.
fn heartbeat_event(build: &str, hour: u32, day: u32) -> Event {
    Event {
        version: VERSION,
        event: "device.heartbeat".to_owned(),
        id: None,
        title: None,
        message: None,
        extra: [
            ("build".to_owned(), serde_json::json!(build)),
            ("hour".to_owned(), serde_json::json!(hour)),
            ("day".to_owned(), serde_json::json!(day)),
        ]
        .into_iter()
        .collect(),
    }
}

/// Local date packed into one integer YYYYMMDD: the device only needs to compare whether it changed.
fn local_day(now: &chrono::DateTime<chrono::Local>) -> u32 {
    use chrono::Datelike;
    now.year() as u32 * 10_000 + now.month() * 100 + now.day()
}

/// Poll GitHub Actions. Does nothing when no repos are configured.
async fn poll_ci(state: AppState) {
    let mut watcher = CiWatcher::default();
    let mut ticker = tokio::time::interval(ci::POLL_INTERVAL);
    loop {
        ticker.tick().await;
        // Fetch before taking the lock: `gh` may run for seconds, and holding the lock meanwhile would block all hooks.
        let workspaces = state.activities.lock().await.recent_workspaces();
        let fetched = watcher.fetch(&workspaces).await;
        if fetched.is_empty() {
            continue;
        }
        let events = {
            let mut tracker = state.activities.lock().await;
            let mut events = watcher.apply(&mut tracker, fetched);
            for event in &mut events {
                tracker.stamp_live_fields(event);
            }
            events
        };
        if let Some(say) = state.activities.lock().await.take_say() {
            send_event(&state, say);
        }
        for event in events {
            info!(event = %event.event, "CI status changed");
            send_event(&state, event);
        }
    }
}

async fn sweep_expired_activities(state: AppState) {
    let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
    loop {
        ticker.tick().await;
        let event = {
            let mut tracker = state.activities.lock().await;
            tracker.sweep_expired().map(|mut event| {
                tracker.stamp_live_fields(&mut event);
                event
            })
        };
        let Some(event) = event else {
            continue;
        };
        info!(event = %event.event, "clearing expired activity");
        send_event(&state, event);
    }
}

/// Shared path for background tasks sending events. A full queue or encoding failure is only logged and doesn't affect the next round.
fn send_event(state: &AppState, event: Event) {
    match event.to_ndjson() {
        Ok(frame) => {
            if let Err(error) = state.transport.send(frame) {
                warn!(?error, "status could not be queued for sending");
            }
        }
        Err(error) => warn!(%error, "failed to encode status"),
    }
}

async fn post_event(
    State(state): State<AppState>,
    Json(event): Json<Event>,
) -> (StatusCode, Json<ApiResponse>) {
    let frame = match event.to_ndjson() {
        Ok(frame) => frame,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    accepted: false,
                    message: error.to_string(),
                }),
            );
        }
    };

    match state.transport.send(frame) {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(ApiResponse {
                accepted: true,
                message: "event queued for the device".to_owned(),
            }),
        ),
        Err(TransportError::QueueFull) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse {
                accepted: false,
                message: "device send queue is full".to_owned(),
            }),
        ),
        Err(TransportError::Closed) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse {
                accepted: false,
                message: "serial worker has stopped".to_owned(),
            }),
        ),
    }
}

async fn post_codex_hook(
    State(state): State<AppState>,
    Json(hook): Json<CodexHook>,
) -> (StatusCode, Json<ApiResponse>) {
    // The resident session is an Agent, but never an Activity: no card, announcement or stats (ADR-0017).
    if let Some(resident) = &state.resident
        && resident.owns_hook(&hook.session_id, hook.thread_id.as_deref(), hook.cwd.as_deref())
    {
        tracing::debug!(event = %hook.hook_event_name, "ignoring a hook from the resident session");
        return (
            StatusCode::ACCEPTED,
            Json(ApiResponse { accepted: true, message: "resident session, ignored".to_owned() }),
        );
    }
    state.hooks_seen.lock().await.codex = Some(chrono::Local::now());
    state.notify_status();
    let event = {
        let mut tracker = state.activities.lock().await;
        let mut titles = state.titles.lock().await;
        codex_hooks::apply(&mut tracker, &mut titles, hook).map(|mut event| {
            tracker.stamp_live_fields(&mut event);
            event
        })
    };
    forward(state, event).await
}

async fn post_claude_hook(
    State(state): State<AppState>,
    Json(hook): Json<ClaudeHook>,
) -> (StatusCode, Json<ApiResponse>) {
    // The resident session on Claude Code is never an Activity either (ADR-0017).
    if let Some(resident) = &state.resident
        && resident.owns_claude_hook(&hook.session_id, hook.cwd.as_deref())
    {
        tracing::debug!(event = %hook.hook_event_name, "ignoring a Claude Code hook from the resident session");
        return (
            StatusCode::ACCEPTED,
            Json(ApiResponse { accepted: true, message: "resident session, ignored".to_owned() }),
        );
    }
    state.hooks_seen.lock().await.claude = Some(chrono::Local::now());
    state.notify_status();
    let event = {
        let mut tracker = state.activities.lock().await;
        let mut titles = state.titles.lock().await;
        claude_hooks::apply(&mut tracker, &mut titles, hook).map(|mut event| {
            tracker.stamp_live_fields(&mut event);
            event
        })
    };
    forward(state, event).await
}

async fn post_agent_hook(
    State(state): State<AppState>,
    Json(hook): Json<AgentHook>,
) -> (StatusCode, Json<ApiResponse>) {
    {
        let mut seen = state.hooks_seen.lock().await;
        let now = Some(chrono::Local::now());
        match hook.agent.as_str() {
            "opencode" => seen.opencode = now,
            "copilot" => seen.copilot = now,
            "pi" => seen.pi = now,
            _ => {}
        }
    }
    state.notify_status();
    let event = {
        let mut tracker = state.activities.lock().await;
        agent_hooks::apply(&mut tracker, hook).map(|mut event| {
            tracker.stamp_live_fields(&mut event);
            event
        })
    };
    forward(state, event).await
}

async fn forward(state: AppState, event: Option<Event>) -> (StatusCode, Json<ApiResponse>) {
    // A greeting or welcome back this activity earned goes first: it speaks before the activity does.
    if let Some(say) = state.activities.lock().await.take_say() {
        send_event(&state, say);
    }
    let Some(event) = event else {
        return (
            StatusCode::ACCEPTED,
            Json(ApiResponse {
                accepted: true,
                message: "hook accepted, visible state unchanged".to_owned(),
            }),
        );
    };
    post_event(State(state), Json(event)).await
}

#[derive(serde::Deserialize)]
struct SayRequest {
    text: String,
}

#[derive(Serialize)]
struct ResidentInfo {
    enabled: bool,
    thread_id: Option<String>,
    /// Goes up when a voice command's turn ends or the thread changes; the Conversation tab polls it.
    revision: u64,
    /// On, and the user's Codex was last found installed and logged in: voice commands can be taken.
    ready: bool,
    /// What is missing when it is on but not ready: `codex_missing`, `codex_logged_out`, `claude_missing` or
    /// `anthropic_key_missing`.
    problem: Option<resident::Problem>,
    /// The brain voice commands go to while ready: `codex` or `claude`.
    brain: Option<resident::Brain>,
    /// The user's setting: `auto`, `codex` or `claude`.
    brain_setting: resident::BrainChoice,
    /// Whether an Anthropic API key is stored; the key itself is never sent back.
    anthropic_key_set: bool,
}

/// Asking checks the resident session's readiness again when the last check is older than
/// [`resident::READINESS_TTL`]; it never starts the app-server or Claude Code.
async fn get_resident(State(state): State<AppState>) -> Json<ResidentInfo> {
    let Some(resident) = state.resident.as_ref() else {
        return Json(ResidentInfo {
            enabled: false,
            thread_id: None,
            revision: 0,
            ready: false,
            problem: None,
            brain: None,
            brain_setting: resident::BrainChoice::default(),
            anthropic_key_set: false,
        });
    };
    let readiness = resident.check_readiness().await;
    let on = resident.enabled();
    Json(ResidentInfo {
        enabled: on,
        thread_id: resident.thread_id(),
        revision: resident.revision(),
        ready: resident.ready(),
        problem: if on { readiness.problem } else { None },
        brain: if resident.ready() { readiness.brain } else { None },
        brain_setting: resident.brain_setting(),
        anthropic_key_set: resident.anthropic_key_set(),
    })
}

#[derive(serde::Deserialize)]
struct BrainRequest {
    brain: resident::BrainChoice,
}

/// Sets which brain the resident session runs on. Its own endpoint, kept out of `PUT /v1/config`.
async fn put_resident_brain(State(state): State<AppState>, Json(request): Json<BrainRequest>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(resident) = state.resident.clone() else {
        return no_state_directory();
    };
    if let Err(error) = resident.set_brain(request.brain).await {
        warn!(%error, "failed to save the resident session's brain setting");
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiResponse { accepted: false, message: error.to_string() })).into_response();
    }
    get_resident(State(state)).await.into_response()
}

#[derive(serde::Deserialize)]
struct AnthropicKeyRequest {
    api_key: String,
}

/// Stores the user's Anthropic API key for Claude Code (mode 0600, in its own file); it is never sent back or
/// logged. Its own endpoint, so an App that doesn't know it can't wipe it.
async fn put_anthropic_key(State(state): State<AppState>, Json(request): Json<AnthropicKeyRequest>) -> axum::response::Response {
    use axum::response::IntoResponse;
    if request.api_key.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(ApiResponse { accepted: false, message: "api_key is empty".to_owned() })).into_response();
    }
    set_anthropic_key(state, Some(request.api_key)).await
}

async fn delete_anthropic_key(State(state): State<AppState>) -> axum::response::Response {
    set_anthropic_key(state, None).await
}

async fn set_anthropic_key(state: AppState, key: Option<String>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(resident) = state.resident.clone() else {
        return no_state_directory();
    };
    if let Err(error) = resident.set_anthropic_key(key).await {
        warn!(%error, "failed to save the Anthropic API key");
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiResponse { accepted: false, message: error.to_string() })).into_response();
    }
    get_resident(State(state)).await.into_response()
}

#[derive(Serialize)]
struct SignInResponse {
    url: String,
}

/// Starts signing in to the VibeBuddy account (ADR-0018): the App opens the URL in the browser, which comes back to
/// [`get_account_callback`].
async fn post_sign_in(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    match state.account.start() {
        Ok(url) => Json(SignInResponse { url }).into_response(),
        Err(error) => account_refusal(&error).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

async fn get_account_callback(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<CallbackQuery>,
) -> (StatusCode, axum::response::Html<String>) {
    let account = state.account.clone();
    let result = match (query.code, query.state) {
        (Some(code), Some(sign_in)) => tokio::task::spawn_blocking(move || account.finish(&code, &sign_in))
            .await
            .unwrap_or_else(|error| Err(account::AccountError::Service(error.to_string()))),
        _ => Err(account::AccountError::UnknownSignIn),
    };
    if let Err(error) = &result {
        warn!(%error, "signing in to the account failed");
    }
    state.notify_status();
    let (status, page) = account::callback_page(&result);
    (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), axum::response::Html(page))
}

async fn post_sign_out(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse>) {
    let account = state.account.clone();
    let result = tokio::task::spawn_blocking(move || account.sign_out()).await.unwrap_or_else(|error| Err(account::AccountError::Service(error.to_string())));
    state.notify_status();
    match result {
        Ok(()) => (StatusCode::OK, Json(ApiResponse { accepted: true, message: "signed out".to_owned() })),
        Err(error) => account_refusal(&error),
    }
}

async fn delete_account(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse>) {
    let account = state.account.clone();
    let result = tokio::task::spawn_blocking(move || account.delete()).await.unwrap_or_else(|error| Err(account::AccountError::Service(error.to_string())));
    state.notify_status();
    match result {
        Ok(()) => (StatusCode::OK, Json(ApiResponse { accepted: true, message: "account deleted".to_owned() })),
        Err(error) => account_refusal(&error),
    }
}

fn account_refusal(error: &account::AccountError) -> (StatusCode, Json<ApiResponse>) {
    let status = match error {
        account::AccountError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        account::AccountError::NotSignedIn => StatusCode::CONFLICT,
        account::AccountError::UnknownSignIn => StatusCode::BAD_REQUEST,
        account::AccountError::Service(_) => StatusCode::BAD_GATEWAY,
    };
    (status, Json(ApiResponse { accepted: false, message: error.to_string() }))
}

fn no_state_directory() -> axum::response::Response {
    use axum::response::IntoResponse;
    (StatusCode::SERVICE_UNAVAILABLE, Json(ApiResponse { accepted: false, message: "no state directory".to_owned() })).into_response()
}

#[derive(serde::Deserialize)]
struct HistoryQuery {
    limit: Option<usize>,
}

/// The last voice commands and their spoken replies, read from the resident thread (default 5, at most 50).
async fn get_resident_history(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<HistoryQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(resident) = state.resident.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(ApiResponse { accepted: false, message: "no state directory".to_owned() }))
            .into_response();
    };
    match resident.history(query.limit.unwrap_or(5).clamp(1, 50)).await {
        Ok(exchanges) => Json(serde_json::json!({"thread_id": resident.thread_id(), "exchanges": exchanges})).into_response(),
        Err(error) => {
            let status = match error {
                resident::ResidentError::Disabled => StatusCode::NOT_FOUND,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            };
            (status, Json(ApiResponse { accepted: false, message: error.to_string() })).into_response()
        }
    }
}

/// One voice command, typed for now: waits for the resident session's spoken replies.
async fn post_resident_say(State(state): State<AppState>, Json(request): Json<SayRequest>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(resident) = state.resident.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(ApiResponse { accepted: false, message: "no state directory".to_owned() }))
            .into_response();
    };
    let text = request.text.trim();
    if text.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(ApiResponse { accepted: false, message: "text is empty".to_owned() })).into_response();
    }
    let started = std::time::Instant::now();
    match resident.say(text, &wearing(&state).await).await {
        Ok(answer) => {
            info!(elapsed_ms = started.elapsed().as_millis() as u64, replies = answer.replies.len(), "voice command answered");
            Json(serde_json::json!({
                "thread_id": answer.thread_id,
                "replies": answer.replies,
                "elapsed_ms": started.elapsed().as_millis() as u64,
            }))
            .into_response()
        }
        Err(error) => {
            warn!(%error, "voice command failed");
            let status = match error {
                resident::ResidentError::Disabled => StatusCode::NOT_FOUND,
                resident::ResidentError::Timeout => StatusCode::GATEWAY_TIMEOUT,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            };
            (status, Json(ApiResponse { accepted: false, message: error.to_string() })).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct RecordingTransport {
        frames: Mutex<Vec<Vec<u8>>>,
    }

    impl Transport for RecordingTransport {
        fn send(&self, frame: Vec<u8>) -> Result<(), TransportError> {
            self.frames.lock().expect("mutex should not be poisoned").push(frame);
            Ok(())
        }
    }

    impl RecordingTransport {
        fn events(&self) -> Vec<Event> {
            self.frames
                .lock()
                .expect("mutex should not be poisoned")
                .iter()
                .map(|frame| serde_json::from_slice(frame).expect("frame should be a valid event"))
                .collect()
        }
    }

    fn test_state(transport: Arc<RecordingTransport>) -> AppState {
        AppState::new(
            transport,
            ActivityTracker::default(),
            SessionTitles::disabled(),
            None,
        )
    }

    #[tokio::test]
    async fn the_account_routes_answer_without_a_service() {
        let state = test_state(Arc::new(RecordingTransport::default()));
        let response = post_sign_in(State(state.clone())).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let (status, _) = post_sign_out(State(state.clone())).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(!state.snapshot().await.account.available);
    }

    #[tokio::test]
    async fn a_callback_this_daemon_did_not_start_shows_a_refusal_page() {
        let mut state = test_state(Arc::new(RecordingTransport::default()));
        let dir = std::env::temp_dir().join(format!("vibebuddy-callback-{}", std::process::id()));
        state.account = Arc::new(account::Account::new(Some("http://127.0.0.1:9".to_owned()), Some(&dir), 7331));
        let callback = |code: Option<&str>, sign_in: Option<&str>| {
            get_account_callback(State(state.clone()), axum::extract::Query(CallbackQuery { code: code.map(Into::into), state: sign_in.map(Into::into) }))
        };
        let (status, page) = callback(Some("c"), Some("forged")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(page.0.contains("Not signed in"));
        assert_eq!(callback(None, None).await.0, StatusCode::BAD_REQUEST);
        assert!(!state.snapshot().await.account.signed_in);
    }

    #[tokio::test(start_paused = true)]
    async fn a_flash_is_done_only_once_the_new_firmware_reports_ready() {
        let state = test_state(Arc::new(RecordingTransport::default()));
        let waiting = tokio::spawn({
            let state = state.clone();
            async move { await_first_boot(&state, Duration::from_secs(15)).await }
        });

        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(state.operation.lock().await.as_ref().map(|operation| operation.state), Some(OperationState::Running));

        publish_device_message(&state, DeviceMessage::Line("DISPLAY READY BUILD v9 2026-10-01".to_owned())).await;
        waiting.await.unwrap();
        assert_eq!(state.operation.lock().await.as_ref().map(|operation| operation.state), Some(OperationState::Done));
    }

    #[tokio::test(start_paused = true)]
    async fn a_box_that_stays_dark_asks_for_a_replug_then_finishes_after_it() {
        let state = test_state(Arc::new(RecordingTransport::default()));
        let waiting = tokio::spawn({
            let state = state.clone();
            async move { await_first_boot(&state, Duration::from_secs(15)).await }
        });

        tokio::time::sleep(Duration::from_secs(20)).await;
        assert_eq!(state.operation.lock().await.as_ref().map(|operation| operation.state), Some(OperationState::Replug));
        // No deadline on the replug: an hour later it is still waiting, not stuck in a state nothing watches.
        tokio::time::sleep(Duration::from_secs(3600)).await;
        assert_eq!(state.operation.lock().await.as_ref().map(|operation| operation.state), Some(OperationState::Replug));

        publish_device_message(&state, DeviceMessage::Line("DISPLAY READY BUILD v9 2026-10-01".to_owned())).await;
        waiting.await.unwrap();
        assert_eq!(state.operation.lock().await.as_ref().map(|operation| operation.state), Some(OperationState::Done));
    }

    #[tokio::test]
    async fn a_box_that_reports_its_key_is_paired_with_this_computer() {
        let transport = Arc::new(RecordingTransport::default());
        let mut state = test_state(transport.clone());
        let dir = std::env::temp_dir().join(format!("vibebuddy-pair-test-{}", std::process::id()));
        let identity = identity::Identity::load_or_create(&dir.join("identity"), "dragon's MacBook".to_owned()).unwrap();
        let key = identity.public_key();
        state.identity = Some(Arc::new(identity));

        publish_device_message(&state, DeviceMessage::Line("DISPLAY READY BUILD v9 2026-10-01".to_owned())).await;
        let pairs = || transport.events().into_iter().filter(|event| event.event == "device.pair").collect::<Vec<_>>();
        assert!(pairs().is_empty(), "firmware that can't pair gets no pair event");
        publish_device_message(&state, DeviceMessage::Line("BOX KEY 1Uc4Vh8Aby1bV4I08TphnmbwAm0AzHkw5RelizUGdRE".to_owned())).await;
        let pair = pairs().pop().expect("a pair event");
        assert_eq!(pair.event, "device.pair");
        assert_eq!(pair.extra["key"], key.as_str());
        assert_eq!(pair.extra["name"], "dragon's MacBook");
        assert!(
            !transport.events().iter().any(|event| event.event == "device.relay.set"),
            "nothing to give the box without a Relay"
        );
        state.relay = Some(relay::RelayConfig::new(state.identity.clone().unwrap(), None, false));
        publish_device_message(&state, DeviceMessage::Line("BOX KEY 1Uc4Vh8Aby1bV4I08TphnmbwAm0AzHkw5RelizUGdRE".to_owned())).await;
        let set = transport.events().into_iter().find(|event| event.event == "device.relay.set").expect("the Relay for the box");
        assert_eq!(set.extra["url"], relay::STAGING_URL);
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn only_the_cable_changes_the_remembered_box_key() {
        let transport = Arc::new(RecordingTransport::default());
        let mut state = test_state(transport.clone());
        let dir = std::env::temp_dir().join(format!("vibebuddy-box-key-test-{}", std::process::id()));
        state.identity = Some(Arc::new(identity::Identity::load_or_create(&dir.join("identity"), "a".to_owned()).unwrap()));
        state.state_dir = Some(dir.clone());
        let remembered = || std::fs::read_to_string(lan::box_key_file(&dir)).ok().map(|key| key.trim().to_owned());
        let old = "1Uc4Vh8Aby1bV4I08TphnmbwAm0AzHkw5RelizUGdRE";
        let new = "pm3f5yT3yi4s2P90TalP1NcUZD3xBTfETdlii-KnvWM";

        publish_device_message(&state, DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: false, usb_serial: None }).await;
        publish_device_message(&state, DeviceMessage::Line(format!("BOX KEY {old}"))).await;
        assert_eq!(remembered().as_deref(), Some(old));

        // A box on the network or the Relay saying it has a new identity is not believed.
        for linked in [
            DeviceMessage::ConnectedNetwork { address: "192.168.1.23:7340".to_owned() },
            DeviceMessage::ConnectedRelay { url: "wss://relay.example".to_owned() },
        ] {
            publish_device_message(&state, linked).await;
            publish_device_message(&state, DeviceMessage::Line(format!("BOX KEY {new}"))).await;
            assert_eq!(remembered().as_deref(), Some(old));
        }

        // The box itself, on the cable, after a reset.
        publish_device_message(&state, DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: false, usb_serial: None }).await;
        publish_device_message(&state, DeviceMessage::Line(format!("BOX KEY {new}"))).await;
        assert_eq!(remembered().as_deref(), Some(new));
        std::fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn wifi_is_set_only_over_the_cable_on_firmware_that_can() {
        let transport = Arc::new(RecordingTransport::default());
        let state = test_state(transport.clone());
        let request = || Json(WifiRequest { ssid: "Home".to_owned(), password: "secret".to_owned() });
        let (status, _) = post_wifi(State(state.clone()), request()).await;
        assert_eq!(status, StatusCode::CONFLICT, "nothing connected");

        publish_device_message(&state, DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: false, usb_serial: None }).await;
        let (status, _) = post_wifi(State(state.clone()), request()).await;
        assert_eq!(status, StatusCode::CONFLICT, "firmware that can't pair would show it as agent activity");

        publish_device_message(&state, DeviceMessage::Line("BOX KEY 1Uc4Vh8Aby1bV4I08TphnmbwAm0AzHkw5RelizUGdRE".to_owned())).await;
        publish_device_message(&state, DeviceMessage::Line("WIFI NETWORK Home".to_owned())).await;
        publish_device_message(&state, DeviceMessage::Line("WIFI FAILED FourWayHandshakeTimeout".to_owned())).await;
        let (status, _) = post_wifi(State(state.clone()), request()).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(state.device.lock().await.wifi_error, None, "joining again forgets the old password's failure");
        let set = transport.events().into_iter().find(|event| event.event == "device.wifi.set").expect("sent");
        assert_eq!((set.extra["ssid"].as_str(), set.extra["password"].as_str()), (Some("Home"), Some("secret")));

        publish_device_message(&state, DeviceMessage::ConnectedNetwork { address: "192.168.1.23:7340".to_owned() }).await;
        publish_device_message(&state, DeviceMessage::Line("BOX KEY 1Uc4Vh8Aby1bV4I08TphnmbwAm0AzHkw5RelizUGdRE".to_owned())).await;
        let (status, _) = post_wifi_forget(State(state.clone())).await;
        assert_eq!(status, StatusCode::CONFLICT, "not over the network");
    }

    fn device_event(json: &str) -> DeviceMessage {
        DeviceMessage::Event(serde_json::from_str(json).expect("test event should parse"))
    }

    /// Wait for the `index`-th event to show up in the recording transport.
    async fn nth_event(transport: &RecordingTransport, index: usize) -> Event {
        for _ in 0..200 {
            if let Some(event) = transport.events().get(index) {
                return event.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("event {index} never showed up");
    }

    #[tokio::test]
    async fn the_resident_sessions_own_hooks_make_no_card_announcement_or_stats() {
        let dir = std::env::temp_dir().join(format!("vibebuddy-main-resident-hooks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let transport = Arc::new(RecordingTransport::default());
        let mut state = test_state(transport.clone());
        state.resident = Some(Arc::new(resident::Resident::new(
            false,
            &dir,
            resident::Launcher::Codex,
            state.activities.clone(),
        )));
        let resident_cwd = dir.join("resident").display().to_string();
        for event in ["UserPromptSubmit", "PostToolUse", "Stop"] {
            let hook: CodexHook = serde_json::from_value(serde_json::json!({
                "session_id": "resident-thread",
                "turn_id": "t1",
                "hook_event_name": event,
                "cwd": resident_cwd,
            }))
            .unwrap();
            let _ = post_codex_hook(State(state.clone()), Json(hook)).await;
        }
        assert!(transport.events().is_empty());
        assert!(state.activities.lock().await.summaries().is_empty());
        assert_eq!(state.activities.lock().await.today().done, 0);

        // Any other Codex session still counts.
        let hook: CodexHook = serde_json::from_value(serde_json::json!({
            "session_id": "work",
            "turn_id": "t1",
            "hook_event_name": "UserPromptSubmit",
            "cwd": "/work/vibe-buddy",
        }))
        .unwrap();
        let _ = post_codex_hook(State(state.clone()), Json(hook)).await;
        assert_eq!(state.activities.lock().await.summaries().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_resident_claude_sessions_hooks_make_no_card_and_the_key_is_never_sent_back() {
        let dir = resident::tests::temp_dir("main-claude");
        let transport = Arc::new(RecordingTransport::default());
        let mut state = test_state(transport.clone());
        let claude = resident_claude::tests::script();
        state.resident = Some(Arc::new(
            resident::tests::resident(&dir, &resident::tests::logged_in(), state.activities.clone())
                .with_claude(resident_claude::tests::brain(&dir, &claude, state.activities.clone())),
        ));

        let request = AnthropicKeyRequest { api_key: " ".to_owned() };
        assert_eq!(put_anthropic_key(State(state.clone()), Json(request)).await.status(), StatusCode::BAD_REQUEST);
        let request = AnthropicKeyRequest { api_key: "sk-ant-secret".to_owned() };
        let response = put_anthropic_key(State(state.clone()), Json(request)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = http_body_util::BodyExt::collect(response.into_body()).await.unwrap().to_bytes();
        let info: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(info["anthropic_key_set"], true);
        assert!(!String::from_utf8_lossy(&body).contains("sk-ant-secret"));
        let request = BrainRequest { brain: resident::BrainChoice::Claude };
        let response = put_resident_brain(State(state.clone()), Json(request)).await;
        let body = http_body_util::BodyExt::collect(response.into_body()).await.unwrap().to_bytes();
        let info: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!((info["brain_setting"].as_str(), info["brain"].as_str(), info["ready"].as_bool()), (Some("claude"), Some("claude"), Some(true)));

        let resident = state.resident.clone().unwrap();
        resident.say("hi", &persona::Wearing::default()).await.unwrap();
        let session = resident.thread_id().unwrap();
        for event in ["UserPromptSubmit", "Stop"] {
            let hook: ClaudeHook = serde_json::from_value(serde_json::json!({
                "session_id": session,
                "prompt_id": "p1",
                "hook_event_name": event,
                "cwd": dir.join("resident").display().to_string(),
            }))
            .unwrap();
            let _ = post_claude_hook(State(state.clone()), Json(hook)).await;
        }
        assert!(transport.events().is_empty());
        assert!(state.activities.lock().await.summaries().is_empty());

        let response = delete_anthropic_key(State(state.clone())).await;
        let body = http_body_util::BodyExt::collect(response.into_body()).await.unwrap().to_bytes();
        let info: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!((info["anthropic_key_set"].as_bool(), info["problem"].as_str()), (Some(false), Some("anthropic_key_missing")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_app_version_leads_the_build_identity() {
        assert_eq!(
            build_identity_from(Some("0.3.0"), "abc1234", "2026-09-16 10:50"),
            "0.3.0 abc1234 2026-09-16 10:50"
        );
        assert_eq!(build_identity_from(None, "abc1234", ""), "abc1234");
    }

    /// A recognizer that is never called: these tests only look at whether voice is offered.
    #[test]
    fn a_voice_token_is_due_only_for_a_box_on_the_relay_that_can_use_it() {
        let device = DeviceState { connected: true, relay: true, voice_direct: true, box_key: Some("BOX".to_owned()), ..Default::default() };
        assert_eq!(voice_token_box(&device, true, true).as_deref(), Some("BOX"));
        assert_eq!(voice_token_box(&device, false, true), None, "signed out");
        assert_eq!(voice_token_box(&device, true, false), None, "the user's own recognizer: audio comes over the link");
        assert_eq!(voice_token_box(&DeviceState { relay: false, ..device.clone() }, true, true), None, "on the cable or the LAN");
        assert_eq!(voice_token_box(&DeviceState { voice_direct: false, ..device.clone() }, true, true), None, "firmware that can't use one");
        assert_eq!(voice_token_box(&DeviceState { connected: false, ..device }, true, true), None);
    }

    /// Recognition by the voice service, as far as the token handout is concerned.
    struct ServiceRecognizer;

    impl speech::Recognizer for ServiceRecognizer {
        fn recognize(&self, _wav: &[u8]) -> Result<speech::Transcript, speech::RecognitionError> {
            Ok(speech::Transcript::default())
        }

        fn is_voice_service(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn a_box_on_the_relay_gets_a_voice_token_and_another_after_it_restarts() {
        use std::io::{BufRead, BufReader, Read, Write};
        // A fake service handing out numbered tokens.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let service = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for (n, stream) in listener.incoming().enumerate() {
                let Ok(stream) = stream else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                reader.read_exact(&mut vec![0; length]).unwrap();
                let body = format!(r#"{{"token":"vbt1.t{n}","expires_at":"later"}}"#);
                let mut stream = stream;
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        let transport = Arc::new(RecordingTransport::default());
        let mut state = test_state(transport.clone());
        let dir = std::env::temp_dir().join(format!("vibebuddy-token-handout-{}", std::process::id()));
        state.account = Arc::new(account::signed_in_at(&service, &dir, "tok"));
        state.voice = Arc::new(voice::Voice::new(Some(Arc::new(ServiceRecognizer)), None));
        *state.device.lock().await = DeviceState { connected: true, relay: true, voice_direct: true, box_key: Some("BOX".to_owned()), ..Default::default() };
        tokio::spawn(hand_out_voice_tokens(state.clone()));
        let tokens = || transport.events().into_iter().filter(|event| event.event == "voice.token").map(|event| event.extra["token"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        for _ in 0..200 {
            if !tokens().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(tokens(), ["vbt1.t0"]);
        // The box restarted on the Relay: it says VOICE DIRECT again and needs a new token at once.
        let _ = state.device_bus.send(DeviceMessage::Line("VOICE DIRECT".to_owned()));
        for _ in 0..200 {
            if tokens().len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(tokens(), ["vbt1.t0", "vbt1.t1"]);
    }

    struct SilentRecognizer;

    impl speech::Recognizer for SilentRecognizer {
        fn recognize(&self, _wav: &[u8]) -> Result<speech::Transcript, speech::RecognitionError> {
            Ok(speech::Transcript::default())
        }
    }

    #[tokio::test]
    async fn voice_is_offered_after_each_build_report_but_never_over_the_relay_or_the_uart_bridge() {
        let dir = std::env::temp_dir().join(format!("vibebuddy-voice-offer-{}", std::process::id()));
        let offers = |transport: &RecordingTransport| {
            transport.events().iter().filter(|event| event.event == "voice.available").map(|event| event.extra["on"].clone()).collect::<Vec<_>>()
        };
        let build = || DeviceMessage::Line("DISPLAY READY BUILD v9 2026-10-01".to_owned());
        for enabled in [true, false] {
            let transport = Arc::new(RecordingTransport::default());
            let mut state = test_state(transport.clone());
            let resident = Arc::new(resident::Resident::new(enabled, &dir, resident::Launcher::Codex, Arc::default()));
            resident.note(None);
            state.voice = Arc::new(voice::Voice::new(Some(Arc::new(SilentRecognizer)), Some(resident)));

            publish_device_message(&state, DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: false, usb_serial: None }).await;
            assert!(offers(&transport).is_empty(), "a new link starts with voice off");
            publish_device_message(&state, build()).await;
            // The box rebooting on the same link reports its build again, and forgets.
            publish_device_message(&state, build()).await;
            publish_device_message(&state, DeviceMessage::ConnectedNetwork { address: "192.168.1.23:7340".to_owned() }).await;
            publish_device_message(&state, build()).await;
            let expected = if enabled { vec![serde_json::json!(true); 3] } else { Vec::new() };
            assert_eq!(offers(&transport), expected);

            publish_device_message(&state, DeviceMessage::ConnectedRelay { url: "wss://relay.example".to_owned() }).await;
            publish_device_message(&state, build()).await;
            assert_eq!(offers(&transport), expected, "not over the Relay");
            // The box sends no audio over the UART bridge.
            publish_device_message(&state, DeviceMessage::Connected { port: "/dev/cu.wchusbserial1".to_owned(), bridge: true, usb_serial: None }).await;
            publish_device_message(&state, build()).await;
            assert_eq!(offers(&transport), expected, "not over the UART bridge");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_box_is_told_each_time_the_resident_session_becomes_ready_or_stops_being() {
        let dir = resident::tests::temp_dir("main-readiness-offer");
        let transport = Arc::new(RecordingTransport::default());
        let mut state = test_state(transport.clone());
        let resident = Arc::new(resident::tests::resident(&dir, &resident::tests::logged_in(), Arc::default()));
        state.resident = Some(resident.clone());
        state.voice = Arc::new(voice::Voice::new(Some(Arc::new(SilentRecognizer)), Some(resident.clone())));
        let offers = || {
            transport.events().iter().filter(|event| event.event == "voice.available").map(|event| event.extra["on"].clone()).collect::<Vec<_>>()
        };
        let settle = || tokio::time::sleep(Duration::from_millis(50));
        let build = || DeviceMessage::Line("DISPLAY READY BUILD v9 2026-10-01".to_owned());

        publish_device_message(&state, DeviceMessage::Connected { port: "/dev/cu.x".to_owned(), bridge: false, usb_serial: None }).await;
        publish_device_message(&state, build()).await;
        assert!(offers().is_empty(), "not offered before the session is found ready");

        // The first check finds Codex installed and logged in.
        follow_resident_readiness(&state, resident.clone());
        settle().await;
        assert_eq!(offers(), [serde_json::json!(true)]);
        resident.note(Some(resident::Problem::CodexLoggedOut));
        settle().await;
        assert_eq!(offers(), [serde_json::json!(true), serde_json::json!(false)], "withdrawn when the user logs out");
        resident.note(Some(resident::Problem::CodexMissing));
        settle().await;
        assert_eq!(offers().len(), 2, "still not ready: nothing new to tell");

        // Never over the UART bridge, where voice isn't offered at all.
        publish_device_message(&state, DeviceMessage::Connected { port: "/dev/cu.wchusbserial1".to_owned(), bridge: true, usb_serial: None }).await;
        publish_device_message(&state, build()).await;
        resident.note(None);
        settle().await;
        assert_eq!(offers().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_resident_info_carries_a_revision_without_starting_the_app_server() {
        let dir = resident::tests::temp_dir("main-revision");
        let script = resident::tests::logged_in();
        let mut state = test_state(Arc::new(RecordingTransport::default()));
        state.resident = Some(Arc::new(resident::tests::resident(&dir, &script, Arc::default())));

        let Json(info) = get_resident(State(state.clone())).await;
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            serde_json::json!({
                "enabled": true,
                "thread_id": null,
                "revision": 0,
                "ready": true,
                "problem": null,
                "brain": "codex",
                "brain_setting": "auto",
                "anthropic_key_set": false,
            })
        );
        assert_eq!(script.lock().unwrap().starts, 0, "asking never starts the app-server");

        state.resident.as_ref().unwrap().say("hi", &persona::Wearing::default()).await.unwrap();
        let Json(info) = get_resident(State(state.clone())).await;
        assert_eq!(info.thread_id.as_deref(), Some("new-thread"));
        assert_eq!(info.revision, 2, "a new thread and one turn");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn status_reflects_device_lines_hooks_and_config() {
        let transport = Arc::new(RecordingTransport::default());
        let state = test_state(transport);
        publish_device_message(&state, DeviceMessage::Connected { port: "/dev/cu.test".to_owned(), bridge: true, usb_serial: None }).await;
        publish_device_message(&state, DeviceMessage::Line("MODE LEISURE".to_owned())).await;
        publish_device_message(&state, DeviceMessage::Line("VOICES hsiaoyu".to_owned())).await;
        state.hooks_seen.lock().await.codex = Some(chrono::Local::now());

        let Json(status) = get_status(State(state.clone())).await;
        assert!(status.device.connected);
        assert_eq!(status.device.mode.as_deref(), Some("leisure"));
        assert_eq!(status.device.voice.as_deref(), Some("hsiaoyu"));
        assert!(status.hooks.codex.is_some());
        assert!(status.hooks.claude.is_none());
        assert_eq!(status.config, Config::default());
        assert!(status.operation.is_none());
    }

    #[tokio::test]
    async fn the_status_stream_pushes_a_snapshot_first_and_again_on_change() {
        use axum::body::to_bytes;
        use axum::response::IntoResponse;
        use http_body_util::BodyExt;

        let transport = Arc::new(RecordingTransport::default());
        let state = test_state(transport);
        let response = status_stream(State(state.clone())).await.into_response();
        let mut body = response.into_body();
        let first = body.frame().await.expect("first snapshot is pushed").expect("frame is readable");
        let first = String::from_utf8_lossy(first.data_ref().expect("data frame")).into_owned();
        assert!(first.starts_with("event: status\n"), "{first}");
        assert!(first.contains("\"connected\":false"), "{first}");

        publish_device_message(&state, DeviceMessage::Connected { port: "/dev/cu.s".to_owned(), bridge: false, usb_serial: None }).await;
        let second = body.frame().await.expect("another snapshot is pushed after the status changes").expect("frame is readable");
        let second = String::from_utf8_lossy(second.data_ref().expect("data frame")).into_owned();
        assert!(second.contains("\"connected\":true"), "{second}");
        let _ = to_bytes; // only the frame interface is used
    }

    #[tokio::test]
    async fn config_put_is_persisted_to_the_file() {
        let dir = std::env::temp_dir().join(format!("vibebuddyd-config-{}", std::process::id()));
        let path = dir.join("config.json");
        let transport = Arc::new(RecordingTransport::default());
        let state = AppState::new(
            transport,
            ActivityTracker::default(),
            SessionTitles::disabled(),
            Some(path.clone()),
        );
        let wanted = Config { voice: Some("hsiaochen".to_owned()), notify_link: false, check_updates: Some(true), relay: false };
        let Json(returned) = put_config(State(state.clone()), Json(wanted.clone())).await;
        assert_eq!(returned, wanted);
        assert_eq!(Config::load(&path), wanted);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_screenshot_is_assembled_from_shot_lines_into_a_png() {
        let transport = Arc::new(RecordingTransport::default());
        let state = test_state(transport.clone());
        let handle = tokio::spawn({
            let state = state.clone();
            async move { post_screenshot(State(state)).await }
        });
        nth_event(&transport, 0).await;
        assert_eq!(transport.events()[0].event, "device.screenshot");
        publish_device_message(&state, DeviceMessage::Line("SHOT BEGIN 320x240 BACKLIGHT ON".to_owned())).await;
        for _ in 0..240 {
            publish_device_message(&state, DeviceMessage::Line("SHOT 0000:320".to_owned())).await;
        }
        publish_device_message(&state, DeviceMessage::Line("SHOT END".to_owned())).await;
        let response = handle.await.expect("screenshot task should not panic");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "image/png");
    }

    #[tokio::test]
    async fn identify_sends_the_device_command() {
        let transport = Arc::new(RecordingTransport::default());
        let (status, _) = post_identify(State(test_state(transport.clone()))).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(transport.events()[0].event, "device.identify");
    }

    #[tokio::test]
    async fn only_an_offered_device_can_be_chosen_as_the_box() {
        let state = test_state(Arc::new(RecordingTransport::default()));
        let request = ChoiceRequest { usb_serial: "98:88:E0:06:8B:CC".to_owned() };
        let (status, Json(response)) = post_choice(State(state), Json(request)).await;
        assert_eq!(status, StatusCode::CONFLICT, "nothing was offered");
        assert!(!response.accepted);
    }

    #[tokio::test]
    async fn setting_the_volume_sends_the_level_to_the_device() {
        let transport = Arc::new(RecordingTransport::default());
        let request = VolumeRequest { level: 40, preview: true };
        let (status, _) = post_volume(State(test_state(transport.clone())), Json(request)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let event = &transport.events()[0];
        assert_eq!(event.event, "device.volume");
        assert_eq!(event.extra["level"], 40);
        assert_eq!(event.extra["preview"], true);
    }

    #[tokio::test]
    async fn a_volume_outside_the_range_is_refused_before_reaching_the_device() {
        let transport = Arc::new(RecordingTransport::default());
        let request = VolumeRequest { level: 10, preview: false };
        let (status, Json(response)) = post_volume(State(test_state(transport.clone())), Json(request)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(!response.accepted);
        assert!(transport.events().is_empty());
    }

    fn no_query() -> axum::extract::Query<PackQuery> {
        axum::extract::Query(PackQuery::default())
    }

    fn sample_pack(voice: &str, payload_len: usize) -> Vec<u8> {
        let mut pack = vec![0_u8; 256 + payload_len];
        pack[0..4].copy_from_slice(b"VBVP");
        pack[16..16 + voice.len()].copy_from_slice(voice.as_bytes());
        for (index, byte) in pack[256..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        pack
    }

    #[tokio::test]
    async fn writing_a_voice_pack_waits_for_each_acknowledgement() {
        let transport = Arc::new(RecordingTransport::default());
        let state = test_state(transport.clone());
        let pack = sample_pack("custom", 1000); // 1256 bytes → two chunks

        let query = PackQuery { lender: Some("ada".to_owned()), address: Some("boss".to_owned()) };
        let (status, _) = post_voice_pack(State(state.clone()), axum::extract::Query(query), Bytes::from(pack.clone())).await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let begin = nth_event(&transport, 0).await;
        assert_eq!(begin.event, "voice.begin");
        assert_eq!(begin.extra["size"], 1256);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(transport.events().len(), 1, "no chunk may be sent before ready arrives");

        publish_device_message(&state, device_event(r#"{"version":1,"event":"voice.ready","seq":-1}"#)).await;
        let first = nth_event(&transport, 1).await;
        assert_eq!(first.event, "voice.chunk");
        assert_eq!(first.extra["seq"], 0);
        assert_eq!(first.extra["crc"], crc32fast::hash(&pack[..672]));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(transport.events().len(), 2, "the next chunk may not be sent before the ack arrives");

        publish_device_message(&state, device_event(r#"{"version":1,"event":"voice.ack","seq":0}"#)).await;
        let second = nth_event(&transport, 2).await;
        assert_eq!(second.extra["seq"], 1);
        assert_eq!(second.extra["crc"], crc32fast::hash(&pack[672..]));
        publish_device_message(&state, device_event(r#"{"version":1,"event":"voice.ack","seq":1}"#)).await;
        let end = nth_event(&transport, 3).await;
        assert_eq!(end.event, "voice.end");

        publish_device_message(&state, device_event(r#"{"version":1,"event":"voice.written","voice":"custom"}"#)).await;
        for _ in 0..200 {
            if state.operation.lock().await.as_ref().map(|op| op.state) == Some(OperationState::Done) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let operation = state.operation.lock().await.clone().expect("an operation should be recorded");
        assert_eq!(operation.state, OperationState::Done);
        assert_eq!(state.config.lock().await.voice.as_deref(), Some("custom"));
        // What the App said about it is kept: the custom Character speaks with Ada's persona and calls the user boss.
        assert_eq!(
            wearing(&state).await,
            persona::Wearing { character: Some("ada".to_owned()), address: Some("boss".to_owned()) }
        );
    }

    #[tokio::test]
    async fn a_device_error_fails_the_voice_pack_operation() {
        let transport = Arc::new(RecordingTransport::default());
        let state = test_state(transport.clone());
        let pack = sample_pack("hsiaoyu", 100);
        let _ = post_voice_pack(State(state.clone()), no_query(), Bytes::from(pack)).await;
        nth_event(&transport, 0).await;
        publish_device_message(
            &state,
            device_event(r#"{"version":1,"event":"voice.error","seq":-1,"message":"ESP_ERR_INVALID_SIZE"}"#),
        )
        .await;
        for _ in 0..200 {
            if state.operation.lock().await.as_ref().map(|op| op.state) == Some(OperationState::Failed) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let operation = state.operation.lock().await.clone().expect("an operation should be recorded");
        assert_eq!(operation.state, OperationState::Failed);
        assert!(operation.message.contains("ESP_ERR_INVALID_SIZE"), "{}", operation.message);
        assert_eq!(state.config.lock().await.voice, None);
    }

    #[tokio::test]
    async fn a_second_write_is_refused_while_one_is_running_and_garbage_is_rejected() {
        let transport = Arc::new(RecordingTransport::default());
        let state = test_state(transport.clone());
        let (status, _) = post_voice_pack(State(state.clone()), no_query(), Bytes::from_static(b"not a pack")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let _ = post_voice_pack(State(state.clone()), no_query(), Bytes::from(sample_pack("a", 10))).await;
        let (status, _) = post_voice_pack(State(state.clone()), no_query(), Bytes::from(sample_pack("b", 10))).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn post_event_accepts_and_frames_valid_event() {
        let transport = Arc::new(RecordingTransport::default());
        let event: Event =
            serde_json::from_str(r#"{"version":1,"event":"task.done","title":"Hello"}"#)
                .expect("test message should parse");

        let (status, Json(response)) =
            post_event(State(test_state(transport.clone())), Json(event)).await;

        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(response.accepted);
        assert_eq!(
            transport.frames.lock().expect("mutex should not be poisoned").as_slice(),
            [b"{\"version\":1,\"event\":\"task.done\",\"title\":\"Hello\"}\n"]
        );
    }

    #[test]
    fn heartbeat_carries_build_local_hour_and_day() {
        let frame = heartbeat_event("abc1234 2026-09-15 12:00", 23, 20260915)
            .to_ndjson()
            .expect("heartbeat should encode");
        let text = String::from_utf8(frame).expect("heartbeat must be UTF-8");
        assert!(text.contains(r#""event":"device.heartbeat""#));
        assert!(text.contains(r#""build":"abc1234 2026-09-15 12:00""#));
        assert!(text.contains(r#""hour":23"#));
        assert!(text.contains(r#""day":20260915"#));
    }

    #[test]
    fn local_day_packs_year_month_and_day() {
        use chrono::TimeZone;
        let now = chrono::Local.with_ymd_and_hms(2026, 9, 15, 23, 59, 0).unwrap();
        assert_eq!(local_day(&now), 20260915);
    }

    #[test]
    fn a_focus_event_names_the_tapped_card() {
        let event: Event = serde_json::from_str(r#"{"version":1,"event":"focus","task":"0d4e77c3"}"#).expect("focus event should parse");
        assert_eq!(tapped_card(&event), Some("0d4e77c3"));
        let button: Event = serde_json::from_str(r#"{"version":1,"event":"button","button":"K2","action":"press"}"#).expect("button event should parse");
        assert_eq!(tapped_card(&button), None);
    }

    #[test]
    fn only_a_k2_press_requests_source_opening() {
        let event: Event = serde_json::from_str(
            r#"{"version":1,"event":"button","button":"K2","action":"press"}"#,
        )
        .expect("button event should parse");
        assert!(is_k2_press(&event));

        let release: Event = serde_json::from_str(
            r#"{"version":1,"event":"button","button":"K2","action":"release"}"#,
        )
        .expect("release event should parse");
        assert!(!is_k2_press(&release));
    }
}
