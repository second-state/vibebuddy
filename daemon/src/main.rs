mod activity;
mod ci;
mod claude_hooks;
mod codex_hooks;
mod config;
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
use beacon_protocol::{Event, VERSION};
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

/// 最后一个会话僵死后不会再有 Hook 事件，只能靠定时扫描释放画面。
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);
/// 心跳间隔。设备按这个节奏判断链路是否还活着，固件的超时是它的三倍。
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
/// 构建时的 git 描述，由 `build.rs` 写入。
const BUILD_REVISION: &str = env!("BEACON_BUILD");

#[derive(Clone)]
struct AppState {
    transport: Arc<dyn Transport>,
    /// 所有 Agent 共享一个聚合器：设备只有一块屏幕和一只小灯灵。
    activities: Arc<Mutex<ActivityTracker>>,
    /// 会话标题的查询与缓存：Claude App 的会话标题、Codex 的线程名。
    titles: Arc<Mutex<SessionTitles>>,
    /// 设备此刻的样子，从诊断行里拼出来。
    device: Arc<Mutex<DeviceState>>,
    hooks_seen: Arc<Mutex<HooksSeen>>,
    config: Arc<Mutex<Config>>,
    config_path: Option<PathBuf>,
    /// 正在写语音包或烧固件；同一时刻只有一个。
    operation: Arc<Mutex<Option<Operation>>>,
    /// 设备消息的广播：写语音包、截图这些要等回执的操作各自订阅。
    device_bus: broadcast::Sender<DeviceMessage>,
    /// 状态变了就叫一声，状态流据此推一份新快照。
    status_changed: broadcast::Sender<()>,
    /// App 的版本，随心跳报给设备；没有 App 时为 None。
    app_version: Option<String>,
    /// 真正的串口 worker，烧固件时要让它让出端口；测试里没有。
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
            app_version: env::var("BEACON_APP_VERSION").ok().filter(|value| !value.is_empty()),
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
            warn!(%error, path = %path.display(), "配置保存失败");
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
        .unwrap_or_else(|| EnvFilter::new("beacond=info"));
    tracing_subscriber::fmt().with_env_filter(log_filter).init();

    let bind_address = env::var("BEACON_BIND")
        .unwrap_or_else(|_| "127.0.0.1:7331".to_owned())
        .parse::<SocketAddr>()
        .unwrap_or_else(|error| panic!("BEACON_BIND 无效：{error}"));
    let serial_config = SerialConfig::from_env();
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
    let app = app(state);
    let listener = tokio::net::TcpListener::bind(bind_address)
        .await
        .unwrap_or_else(|error| panic!("无法监听 {bind_address}：{error}"));

    info!(address = %bind_address, "beacond 已启动");
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|error| panic!("HTTP server 失败：{error}"));
}

/// 设备到 Mac 的事件目前只开放 K2 单击。优先打开当前活动；空闲时返回最近
/// 一次可定位的 Agent/CI 来源。
async fn handle_device_events(
    state: AppState,
    mut events: tokio::sync::mpsc::Receiver<DeviceMessage>,
) {
    while let Some(message) = events.recv().await {
        publish_device_message(&state, message).await;
    }
}

/// 每条设备消息都走这里：更新设备状态、广播给等回执的操作，K2 则去开来源。
/// 测试也从这里注入设备消息，所以它不能依赖串口。
async fn publish_device_message(state: &AppState, message: DeviceMessage) {
    if state.device.lock().await.apply(&message) {
        state.notify_status();
    }
    let _ = state.device_bus.send(message.clone());
    // 刚连上先问一声，设备会把模式、固件构建号、音色重报一遍。
    if matches!(message, DeviceMessage::Connected { .. }) {
        send_event(state, device_command("device.hello"));
    }
    let DeviceMessage::Event(event) = message else {
        return;
    };
    if is_k2_press(&event) {
        open_k2_source(state).await;
    } else if !event.event.starts_with("voice.") && event.event != "echo" {
        info!(event = %event.event, "忽略未绑定的设备事件");
    }
}

async fn open_k2_source(state: &AppState) {
    {
        let sources = state.activities.lock().await.focus_sources();
        if sources.is_empty() {
            info!("K2 已按下，但当前没有可打开的活动");
            return;
        }
        for source in sources {
            // Codex 线程要先确认还在：打开一个不存在的线程得到的是空白会话。
            if let ActivitySource::Codex { thread_id } = &source
                && state.titles.lock().await.codex_thread_known(thread_id) == Some(false)
            {
                warn!(%thread_id, "K2 跳过不存在的 Codex 线程");
                continue;
            }
            match source_opener::open(source).await {
                // 记下链接本身：跳错地方时，日志要能直接说出跳去了哪儿。
                Ok(link) => {
                    info!(%link, "K2 已打开当前活动来源");
                    break;
                }
                Err(error) => warn!(%error, "K2 打开来源失败，试下一个候选"),
            }
        }
    }
}

fn is_k2_press(event: &Event) -> bool {
    event.event == "button"
        && event.extra.get("button").and_then(|value| value.as_str()) == Some("K2")
        && event.extra.get("action").and_then(|value| value.as_str()) == Some("press")
}

/// App 看管时它把自己的 pid 放在 BEACON_PARENT_PID 里。App 被强杀后 daemon
/// 会被 launchd 收养，父 pid 变成 1；那就跟着退出，别占着串口和端口等下一个
/// App 起不来。macOS 没有 prctl(PR_SET_PDEATHSIG)，只能轮询。
async fn watch_parent() {
    let Some(expected) = env::var("BEACON_PARENT_PID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
    else {
        return;
    };
    let mut ticker = tokio::time::interval(Duration::from_secs(2));
    loop {
        ticker.tick().await;
        if std::os::unix::process::parent_id() != expected {
            info!(expected, "看管我的 App 已经不在，跟着退出");
            std::process::exit(0);
        }
    }
}

/// daemon 的构建标识：git 描述加上二进制自己的时间戳。
///
/// 时间戳取可执行文件的 mtime，不用编译期常量。`build.rs` 只在它声明的依赖
/// 变化时才重跑；改一行源码重新链接时，编译期写下的时刻不会更新，正好在你
/// 最需要它准的时候骗你。
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

/// App 在时它的版本号排最前：设备页脚那一行就是 App 版本加构建号。
fn build_identity_from(app_version: Option<&str>, revision: &str, built: &str) -> String {
    let mut parts = Vec::new();
    if let Some(version) = app_version {
        parts.push(version);
    }
    parts.push(revision);
    parts.push(built);
    parts.join(" ").trim_end().to_owned()
}

/// 当日战绩的存放位置。缺少 `HOME` 时退回内存计数，不让 daemon 起不来。
fn stats_file() -> Option<PathBuf> {
    if let Ok(path) = env::var("BEACON_STATS_FILE") {
        return Some(PathBuf::from(path));
    }
    let home = env::var("HOME").ok()?;
    Some(PathBuf::from(home).join("Library/Application Support/AgentBeacon/stats.json"))
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
        .route(
            "/v1/device/voice-pack",
            post(post_voice_pack).layer(DefaultBodyLimit::max(4 * 1024 * 1024)),
        )
        .route("/v1/device/screenshot", post(post_screenshot))
        .route("/v1/device/firmware", post(post_firmware))
        .route("/v1/daemon/restart", post(post_restart))
        .with_state(state)
}

/// 截一张盒子当前画面，回 PNG。写语音包或烧固件时不截：截图行会挤掉回执。
async fn post_screenshot(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    if matches!(&*state.operation.lock().await, Some(current) if current.state == OperationState::Running) {
        return (StatusCode::CONFLICT, "设备上有操作在进行").into_response();
    }
    let bus = state.device_bus.subscribe();
    match screenshot::capture(state.transport.clone(), bus).await {
        Ok(frame) => match screenshot::encode_png(&frame) {
            Ok(png) => ([(axum::http::header::CONTENT_TYPE, "image/png")], png).into_response(),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
        },
        Err(error) => (StatusCode::BAD_GATEWAY, error).into_response(),
    }
}

async fn get_status(State(state): State<AppState>) -> Json<Status> {
    Json(state.snapshot().await)
}

/// SSE：连上先推一份，之后状态一变再推一份完整快照。
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
    Event {
        version: VERSION,
        event: name.to_owned(),
        id: None,
        title: None,
        message: None,
        extra: Default::default(),
    }
}

async fn post_identify(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse>) {
    post_event(State(state), Json(device_command("device.identify"))).await
}

/// 写语音包：请求体就是包本身。写入在后台跑，进度在状态流里。
async fn post_voice_pack(
    State(state): State<AppState>,
    body: Bytes,
) -> (StatusCode, Json<ApiResponse>) {
    let Some(voice) = voice_writer::voice_id_of(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse { accepted: false, message: "请求体不是语音包".to_owned() }),
        );
    };
    {
        let mut operation = state.operation.lock().await;
        if matches!(&*operation, Some(current) if current.state == OperationState::Running) {
            return (
                StatusCode::CONFLICT,
                Json(ApiResponse { accepted: false, message: "设备上已有操作在进行".to_owned() }),
            );
        }
        *operation = Some(Operation {
            kind: OperationKind::VoicePack,
            state: OperationState::Running,
            progress: 0.0,
            message: format!("正在写入 {voice}"),
        });
    }
    state.notify_status();
    let bus = state.device_bus.subscribe();
    let pack = body.to_vec();
    let task_state = state.clone();
    tokio::spawn(async move {
        let progress_state = task_state.clone();
        let result = voice_writer::write_pack(task_state.transport.clone(), bus, pack, move |fraction| {
            let state = progress_state.clone();
            tokio::spawn(async move {
                if let Some(operation) = state.operation.lock().await.as_mut() {
                    operation.progress = fraction;
                }
                state.notify_status();
            });
        })
        .await;
        match result {
            Ok(written) => {
                info!(voice = %written, "语音包已写入设备");
                task_state.save_config(|config| config.voice = Some(written.clone())).await;
                task_state
                    .set_operation(Some(Operation {
                        kind: OperationKind::VoicePack,
                        state: OperationState::Done,
                        progress: 1.0,
                        message: format!("已写入 {written}"),
                    }))
                    .await;
            }
            Err(error) => {
                warn!(%error, "语音包写入失败");
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
        Json(ApiResponse { accepted: true, message: format!("开始写入 {voice}") }),
    )
}

#[derive(serde::Deserialize)]
struct FirmwareRequest {
    bootloader: PathBuf,
    partition_table: PathBuf,
    app: PathBuf,
}

/// 烧固件：三件套的路径由 App 给出（都在它的包里）。串口 worker 让出端口，
/// ROM 协议逐段写并校验，完了硬复位、worker 重连。进度走状态流。
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
                    Json(ApiResponse { accepted: false, message: format!("读不到固件文件 {}", path.display()) }),
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
                    Json(ApiResponse { accepted: false, message: "没有连着的盒子".to_owned() }),
                );
            }
        }
    };
    let Some(serial) = state.serial.clone() else {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(ApiResponse { accepted: false, message: "没有串口 worker".to_owned() }),
        );
    };
    {
        let mut operation = state.operation.lock().await;
        if matches!(&*operation, Some(current) if current.state == OperationState::Running) {
            return (
                StatusCode::CONFLICT,
                Json(ApiResponse { accepted: false, message: "设备上已有操作在进行".to_owned() }),
            );
        }
        *operation = Some(Operation {
            kind: OperationKind::Firmware,
            state: OperationState::Running,
            progress: 0.0,
            message: "让出串口".to_owned(),
        });
    }
    state.notify_status();
    let task_state = state.clone();
    tokio::spawn(async move {
        serial.set_suspended(true);
        // 等 worker 真把端口放掉。
        tokio::time::sleep(Duration::from_millis(800)).await;
        let progress_state = task_state.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut report = |fraction: f32, message: &str| {
                let state = progress_state.clone();
                let message = message.to_owned();
                tokio::spawn(async move {
                    if let Some(operation) = state.operation.lock().await.as_mut() {
                        operation.progress = fraction;
                        operation.message = message;
                    }
                    state.notify_status();
                });
            };
            rom_flasher::flash(&port, bridge, &segments, &mut report)
        })
        .await
        .unwrap_or_else(|error| Err(format!("烧录任务崩溃：{error}")));
        serial.set_suspended(false);
        let operation = match result {
            Ok(()) => {
                info!("固件已烧录，等设备重启");
                Operation { kind: OperationKind::Firmware, state: OperationState::Done, progress: 1.0, message: "烧录完成，设备重启中".to_owned() }
            }
            Err(error) => {
                warn!(%error, "固件烧录失败");
                Operation { kind: OperationKind::Firmware, state: OperationState::Failed, progress: 0.0, message: error }
            }
        };
        task_state.set_operation(Some(operation)).await;
    });
    (StatusCode::ACCEPTED, Json(ApiResponse { accepted: true, message: "开始烧录".to_owned() }))
}

/// App 看管 daemon：退出即重启。先把响应发出去再退。
async fn post_restart() -> (StatusCode, Json<ApiResponse>) {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        info!("按 App 的要求退出，等它重新拉起");
        std::process::exit(0);
    });
    (StatusCode::ACCEPTED, Json(ApiResponse { accepted: true, message: "daemon 即将重启".to_owned() }))
}

/// 定期告诉设备链路还活着。
///
/// 没有心跳时，daemon 崩溃或串口断开后设备会一直显示最后一个状态，
/// 看上去任务仍在进行。状态设备最严重的失败是显示过时状态而不自知。
async fn send_heartbeats(state: AppState) {
    let build = build_identity(state.app_version.as_deref());
    info!(build = %build, "Mac 端构建标识");
    let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        ticker.tick().await;
        let now = chrono::Local::now();
        let heartbeat = heartbeat_event(&build, now.hour(), local_day(&now));
        match heartbeat.to_ndjson() {
            // 队列满意味着设备已经收不到东西，这时心跳没有意义，丢弃即可。
            Ok(frame) => drop(state.transport.send(frame)),
            Err(error) => warn!(%error, "心跳编码失败"),
        }
    }
}

/// 心跳捎带三样东西：构建标识、本地小时数、本地日期。都随每次心跳重复发，
/// 因为设备可能随时重启，一次性的握手会丢。小时数让小灯灵知道现在是白天
/// 还是夜里，日期让番茄钟知道什么时候算新的一天：设备没有时钟，也不该
/// 为了这个去连 Wi-Fi。
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

/// 本地日期压成一个整数 YYYYMMDD：设备只需要比较它变没变。
fn local_day(now: &chrono::DateTime<chrono::Local>) -> u32 {
    use chrono::Datelike;
    now.year() as u32 * 10_000 + now.month() * 100 + now.day()
}

/// 轮询 GitHub Actions。没有配置仓库时它什么也不做。
async fn poll_ci(state: AppState) {
    let mut watcher = CiWatcher::default();
    let mut ticker = tokio::time::interval(ci::POLL_INTERVAL);
    loop {
        ticker.tick().await;
        // 先取数据再上锁：`gh` 可能跑上几秒，持锁等它会把 Hook 全堵住。
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
        for event in events {
            info!(event = %event.event, "CI 状态变化");
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
        info!(event = %event.event, "清除过期的活动");
        send_event(&state, event);
    }
}

/// 后台任务发事件的共用路径。队列满或编码失败只记日志，不影响下一轮。
fn send_event(state: &AppState, event: Event) {
    match event.to_ndjson() {
        Ok(frame) => {
            if let Err(error) = state.transport.send(frame) {
                warn!(?error, "状态未能进入发送队列");
            }
        }
        Err(error) => warn!(%error, "状态编码失败"),
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
                message: "事件已进入设备发送队列".to_owned(),
            }),
        ),
        Err(TransportError::QueueFull) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse {
                accepted: false,
                message: "设备发送队列已满".to_owned(),
            }),
        ),
        Err(TransportError::Closed) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiResponse {
                accepted: false,
                message: "串口 worker 已停止".to_owned(),
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
    let Some(event) = event else {
        return (
            StatusCode::ACCEPTED,
            Json(ApiResponse {
                accepted: true,
                message: "Hook 已接收，可见状态未变化".to_owned(),
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
            self.frames.lock().expect("mutex 不应中毒").push(frame);
            Ok(())
        }
    }

    impl RecordingTransport {
        fn events(&self) -> Vec<Event> {
            self.frames
                .lock()
                .expect("mutex 不应中毒")
                .iter()
                .map(|frame| serde_json::from_slice(frame).expect("帧应是合法事件"))
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

    fn device_event(json: &str) -> DeviceMessage {
        DeviceMessage::Event(serde_json::from_str(json).expect("测试事件应可解析"))
    }

    /// 等记录型 Transport 里出现第 `index` 条事件。
    async fn nth_event(transport: &RecordingTransport, index: usize) -> Event {
        for _ in 0..200 {
            if let Some(event) = transport.events().get(index) {
                return event.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("第 {index} 条事件迟迟没有出现");
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
        publish_device_message(&state, DeviceMessage::Connected { port: "/dev/cu.test".to_owned(), bridge: true }).await;
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
    async fn config_put_is_persisted_to_the_file() {
        let dir = std::env::temp_dir().join(format!("beacond-config-{}", std::process::id()));
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
        let response = handle.await.expect("截图任务不该崩");
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
        let pack = sample_pack("hsiaoyu", 1000); // 1256 字节 → 两块

        let (status, _) = post_voice_pack(State(state.clone()), Bytes::from(pack.clone())).await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let begin = nth_event(&transport, 0).await;
        assert_eq!(begin.event, "voice.begin");
        assert_eq!(begin.extra["size"], 1256);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(transport.events().len(), 1, "没收到 ready 之前不能发块");

        publish_device_message(&state, device_event(r#"{"version":1,"event":"voice.ready","seq":-1}"#)).await;
        let first = nth_event(&transport, 1).await;
        assert_eq!(first.event, "voice.chunk");
        assert_eq!(first.extra["seq"], 0);
        assert_eq!(first.extra["crc"], crc32fast::hash(&pack[..672]));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(transport.events().len(), 2, "没收到 ack 之前不能发下一块");

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
        let operation = state.operation.lock().await.clone().expect("应有操作记录");
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
        let operation = state.operation.lock().await.clone().expect("应有操作记录");
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
                .expect("测试消息应可解析");

        let (status, Json(response)) =
            post_event(State(test_state(transport.clone())), Json(event)).await;

        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(response.accepted);
        assert_eq!(
            transport.frames.lock().expect("mutex 不应中毒").as_slice(),
            [b"{\"version\":1,\"event\":\"task.done\",\"title\":\"Hello\"}\n"]
        );
    }

    #[test]
    fn heartbeat_carries_build_local_hour_and_day() {
        let frame = heartbeat_event("abc1234 2026-09-15 12:00", 23, 20260915)
            .to_ndjson()
            .expect("心跳应可编码");
        let text = String::from_utf8(frame).expect("心跳必须是 UTF-8");
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
        .expect("按钮事件应可解析");
        assert!(is_k2_press(&event));

        let release: Event = serde_json::from_str(
            r#"{"version":1,"event":"button","button":"K2","action":"release"}"#,
        )
        .expect("释放事件应可解析");
        assert!(!is_k2_press(&release));
    }
}
