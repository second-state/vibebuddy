mod activity;
mod ci;
mod claude_hooks;
mod codex_hooks;
mod config;
mod link_alert;
mod occasions;
mod rom_flasher;
mod screenshot;
mod serial_transport;
mod session_titles;
mod source_opener;
mod status;
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
use claude_hooks::ClaudeHook;
use codex_hooks::CodexHook;
use config::Config;
use serde::Serialize;
use serial_transport::{DeviceMessage, SerialConfig, SerialTransport, Transport, TransportError};
use session_titles::SessionTitles;
use status::{DaemonInfo, DeviceState, HooksSeen, Operation, OperationKind, OperationState, Status};
use tokio::sync::{Mutex, broadcast};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

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
        }
    }

    fn notify_status(&self) {
        let _ = self.status_changed.send(());
    }

    async fn snapshot(&self) -> Status {
        Status {
            daemon: DaemonInfo {
                build: build_identity(self.app_version.as_deref()),
                app_version: self.app_version.clone(),
            },
            device: self.device.lock().await.clone(),
            today: self.activities.lock().await.today(),
            hooks: self.hooks_seen.lock().await.clone(),
            operation: self.operation.lock().await.clone(),
            config: self.config.lock().await.clone(),
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
    let serial_config = SerialConfig::from_env(known_box_file());
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
    state.serial = Some(serial_transport);
    tokio::spawn(watch_parent());
    tokio::spawn(sweep_expired_activities(state.clone()));
    tokio::spawn(send_heartbeats(state.clone()));
    tokio::spawn(poll_ci(state.clone()));
    tokio::spawn(handle_device_events(state.clone(), device_events));
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
    let _ = state.device_bus.send(message.clone());
    // Ask right after connecting; the device reports its mode, firmware build and voice again.
    if matches!(message, DeviceMessage::Connected { .. }) {
        send_event(state, device_command("device.hello"));
    }
    let DeviceMessage::Event(event) = message else {
        return;
    };
    if is_k2_press(&event) {
        open_k2_source(state).await;
    } else if !event.event.starts_with("voice.") && event.event != "echo" {
        info!(event = %event.event, "ignoring unbound device event");
    }
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
        .route("/v1/status", get(get_status))
        .route("/v1/status/stream", get(status_stream))
        .route("/v1/config", get(get_config).put(put_config))
        .route("/v1/device/identify", post(post_identify))
        .route("/v1/device/volume", post(post_volume))
        .route(
            "/v1/device/voice-pack",
            post(post_voice_pack).layer(DefaultBodyLimit::max(4 * 1024 * 1024)),
        )
        .route("/v1/device/screenshot", post(post_screenshot))
        .route("/v1/device/firmware", post(post_firmware))
        .route("/v1/daemon/restart", post(post_restart))
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

async fn put_config(State(state): State<AppState>, Json(config): Json<Config>) -> Json<Config> {
    state.save_config(|current| *current = config).await;
    Json(state.config.lock().await.clone())
}

fn device_command(name: &str) -> Event {
    Event::named(name)
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

/// Write a voice pack: the request body is the pack itself. The write runs in the background with progress in the status stream.
async fn post_voice_pack(
    State(state): State<AppState>,
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
        let result = voice_writer::write_pack(task_state.transport.clone(), bus, pack, move |fraction| {
            report_progress(&progress_state, fraction, None);
        })
        .await;
        match result {
            Ok(written) => {
                info!(voice = %written, "voice pack written to device");
                task_state.save_config(|config| config.voice = Some(written.clone())).await;
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

/// Flash firmware: the app supplies the three image paths (all inside its bundle). The serial worker releases the port,
/// the ROM protocol writes and verifies each segment, then hard-resets and the worker reconnects. Progress goes through the status stream.
async fn post_firmware(
    State(state): State<AppState>,
    Json(request): Json<FirmwareRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    let mut segments = Vec::new();
    for (address, path) in [(0x0_u32, &request.bootloader), (0x8000, &request.partition_table), (0x10000, &request.app)] {
        match std::fs::read(path) {
            Ok(data) if !data.is_empty() => segments.push(rom_flasher::Segment { address, data }),
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

    #[test]
    fn the_app_version_leads_the_build_identity() {
        assert_eq!(
            build_identity_from(Some("0.3.0"), "abc1234", "2026-09-16 10:50"),
            "0.3.0 abc1234 2026-09-16 10:50"
        );
        assert_eq!(build_identity_from(None, "abc1234", ""), "abc1234");
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
        let wanted = Config { voice: Some("hsiaochen".to_owned()), notify_link: false };
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
        let pack = sample_pack("hsiaoyu", 1000); // 1256 bytes → two chunks

        let (status, _) = post_voice_pack(State(state.clone()), Bytes::from(pack.clone())).await;
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

        publish_device_message(&state, device_event(r#"{"version":1,"event":"voice.written","voice":"hsiaoyu"}"#)).await;
        for _ in 0..200 {
            if state.operation.lock().await.as_ref().map(|op| op.state) == Some(OperationState::Done) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let operation = state.operation.lock().await.clone().expect("an operation should be recorded");
        assert_eq!(operation.state, OperationState::Done);
        assert_eq!(state.config.lock().await.voice.as_deref(), Some("hsiaoyu"));
    }

    #[tokio::test]
    async fn a_device_error_fails_the_voice_pack_operation() {
        let transport = Arc::new(RecordingTransport::default());
        let state = test_state(transport.clone());
        let pack = sample_pack("hsiaoyu", 100);
        let _ = post_voice_pack(State(state.clone()), Bytes::from(pack)).await;
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
        let (status, _) = post_voice_pack(State(state.clone()), Bytes::from_static(b"not a pack")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let _ = post_voice_pack(State(state.clone()), Bytes::from(sample_pack("a", 10))).await;
        let (status, _) = post_voice_pack(State(state.clone()), Bytes::from(sample_pack("b", 10))).await;
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
