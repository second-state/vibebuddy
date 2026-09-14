mod activity;
mod ci;
mod claude_hooks;
mod codex_hooks;
mod serial_transport;

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use activity::ActivityTracker;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use beacon_protocol::{Event, VERSION};
use ci::CiWatcher;
use claude_hooks::ClaudeHook;
use codex_hooks::CodexHook;
use serde::Serialize;
use serial_transport::{SerialConfig, SerialTransport, Transport, TransportError};
use tokio::sync::Mutex;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

/// 最后一个会话僵死后不会再有 Hook 事件，只能靠定时扫描释放画面。
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);
/// 心跳间隔。设备按这个节奏判断链路是否还活着，固件的超时是它的三倍。
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct AppState {
    transport: Arc<dyn Transport>,
    /// 所有 Agent 共享一个聚合器：设备只有一块屏幕和一只小灯灵。
    activities: Arc<Mutex<ActivityTracker>>,
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
    let transport: Arc<dyn Transport> = Arc::new(SerialTransport::spawn(serial_config));
    let activities = match stats_file() {
        Some(path) => ActivityTracker::with_stats_file(path),
        None => ActivityTracker::default(),
    };
    let state = AppState {
        transport,
        activities: Arc::new(Mutex::new(activities)),
    };
    tokio::spawn(sweep_expired_activities(state.clone()));
    tokio::spawn(send_heartbeats(state.clone()));
    tokio::spawn(poll_ci(state.clone()));
    let app = app(state);
    let listener = tokio::net::TcpListener::bind(bind_address)
        .await
        .unwrap_or_else(|error| panic!("无法监听 {bind_address}：{error}"));

    info!(address = %bind_address, "beacond 已启动");
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|error| panic!("HTTP server 失败：{error}"));
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
        .with_state(state)
}

/// 定期告诉设备链路还活着。
///
/// 没有心跳时，daemon 崩溃或串口断开后设备会一直显示最后一个状态，
/// 看上去任务仍在进行。状态设备最严重的失败是显示过时状态而不自知。
async fn send_heartbeats(state: AppState) {
    let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        ticker.tick().await;
        let heartbeat = Event {
            version: VERSION,
            event: "device.heartbeat".to_owned(),
            id: None,
            title: None,
            message: None,
            extra: Default::default(),
        };
        match heartbeat.to_ndjson() {
            // 队列满意味着设备已经收不到东西，这时心跳没有意义，丢弃即可。
            Ok(frame) => drop(state.transport.send(frame)),
            Err(error) => warn!(%error, "心跳编码失败"),
        }
    }
}

/// 轮询 GitHub Actions。没有配置仓库时它什么也不做。
async fn poll_ci(state: AppState) {
    let mut watcher = CiWatcher::default();
    let mut ticker = tokio::time::interval(ci::POLL_INTERVAL);
    loop {
        ticker.tick().await;
        // 先取数据再上锁：`gh` 可能跑上几秒，持锁等它会把 Hook 全堵住。
        let fetched = watcher.fetch().await;
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
    let event = {
        let mut tracker = state.activities.lock().await;
        codex_hooks::apply(&mut tracker, hook).map(|mut event| {
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
    let event = {
        let mut tracker = state.activities.lock().await;
        claude_hooks::apply(&mut tracker, hook).map(|mut event| {
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

    #[tokio::test]
    async fn post_event_accepts_and_frames_valid_event() {
        let transport = Arc::new(RecordingTransport::default());
        let event: Event =
            serde_json::from_str(r#"{"version":1,"event":"task.done","title":"Hello"}"#)
                .expect("测试消息应可解析");

        let (status, Json(response)) = post_event(
            State(AppState {
                transport: transport.clone(),
                activities: Arc::new(tokio::sync::Mutex::new(ActivityTracker::default())),
            }),
            Json(event),
        )
        .await;

        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(response.accepted);
        assert_eq!(
            transport.frames.lock().expect("mutex 不应中毒").as_slice(),
            [b"{\"version\":1,\"event\":\"task.done\",\"title\":\"Hello\"}\n"]
        );
    }
}
