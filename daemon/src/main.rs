mod serial_transport;

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use beacon_protocol::Event;
use serde::Serialize;
use serial_transport::{SerialConfig, SerialTransport, Transport, TransportError};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState {
    transport: Arc<dyn Transport>,
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
    let app = app(transport);
    let listener = tokio::net::TcpListener::bind(bind_address)
        .await
        .unwrap_or_else(|error| panic!("无法监听 {bind_address}：{error}"));

    info!(address = %bind_address, "beacond 已启动");
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|error| panic!("HTTP server 失败：{error}"));
}

fn app(transport: Arc<dyn Transport>) -> Router {
    Router::new()
        .route("/v1/events", post(post_event))
        .with_state(AppState { transport })
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
