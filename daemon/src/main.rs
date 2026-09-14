mod codex_hooks;
mod serial_transport;

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use beacon_protocol::Event;
use codex_hooks::{CodexActivityTracker, CodexHook};
use serde::Serialize;
use serial_transport::{SerialConfig, SerialTransport, Transport, TransportError};
use tokio::sync::Mutex;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

/// 最后一个会话僵死后不会再有 Hook 事件，只能靠定时扫描释放画面。
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct AppState {
    transport: Arc<dyn Transport>,
    codex_activities: Arc<Mutex<CodexActivityTracker>>,
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
    let state = AppState {
        transport,
        codex_activities: Arc::new(Mutex::new(CodexActivityTracker::default())),
    };
    tokio::spawn(sweep_expired_activities(state.clone()));
    let app = app(state);
    let listener = tokio::net::TcpListener::bind(bind_address)
        .await
        .unwrap_or_else(|error| panic!("无法监听 {bind_address}：{error}"));

    info!(address = %bind_address, "beacond 已启动");
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|error| panic!("HTTP server 失败：{error}"));
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/v1/events", post(post_event))
        .route("/v1/codex-hooks", post(post_codex_hook))
        .with_state(state)
}

async fn sweep_expired_activities(state: AppState) {
    let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
    loop {
        ticker.tick().await;
        let Some(event) = state.codex_activities.lock().await.sweep_expired() else {
            continue;
        };
        info!(event = %event.event, "清除过期的 Codex 活动");
        match event.to_ndjson() {
            Ok(frame) => {
                if let Err(error) = state.transport.send(frame) {
                    warn!(?error, "过期状态未能进入发送队列");
                }
            }
            Err(error) => warn!(%error, "过期状态编码失败"),
        }
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
    let event = state.codex_activities.lock().await.apply(hook);
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
                codex_activities: Arc::new(
                    tokio::sync::Mutex::new(CodexActivityTracker::default()),
                ),
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
