//! A client for `codex app-server`'s protocol: newline-delimited JSON-RPC over the child's stdio (ADR-0015).
//!
//! Requests are `{"id","method","params"}` and get `{"id","result"|"error"}` back; the server also sends
//! notifications and requests of its own. This module only moves messages: what they mean is up to
//! [`crate::resident`]. It takes any reader and writer, so tests drive it with an in-memory fake server.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, mpsc, oneshot};

/// Something the server sent that isn't an answer to one of our requests.
#[derive(Debug)]
pub enum Incoming {
    Notification { method: String, params: Value },
    /// A server-to-client request: it must be answered with [`Client::respond`] or [`Client::respond_error`].
    Request { id: Value, method: String, params: Value },
}

#[derive(Debug, PartialEq)]
pub enum RpcError {
    /// The server answered with an error object.
    Server { code: i64, message: String },
    /// The server is gone: its output ended, or writing to it failed.
    Closed,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Server { code, message } => write!(formatter, "app-server error {code}: {message}"),
            Self::Closed => write!(formatter, "app-server exited"),
        }
    }
}

type Pending = Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;

pub struct Client {
    writer: Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    pending: Pending,
    next_id: AtomicU64,
    closed: Arc<AtomicBool>,
}

impl Client {
    /// Starts reading the server's output. Everything that isn't a response comes out of the returned receiver,
    /// which ends when the server's output does.
    pub fn spawn(
        reader: impl AsyncRead + Send + Unpin + 'static,
        writer: impl AsyncWrite + Send + Unpin + 'static,
    ) -> (Self, mpsc::UnboundedReceiver<Incoming>) {
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        let (incoming, receiver) = mpsc::unbounded_channel();
        tokio::spawn(read_loop(reader, pending.clone(), closed.clone(), incoming));
        let client = Self {
            writer: Mutex::new(Box::new(writer)),
            pending,
            next_id: AtomicU64::new(1),
            closed,
        };
        (client, receiver)
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(id, sender);
        if self.closed.load(Ordering::Relaxed) {
            self.pending.lock().expect("pending lock").remove(&id);
            return Err(RpcError::Closed);
        }
        if let Err(error) = self.write(&json!({"id": id, "method": method, "params": params})).await {
            self.pending.lock().expect("pending lock").remove(&id);
            return Err(error);
        }
        receiver.await.unwrap_or(Err(RpcError::Closed))
    }

    pub async fn notify(&self, method: &str) -> Result<(), RpcError> {
        self.write(&json!({"method": method})).await
    }

    pub async fn respond(&self, id: Value, result: Value) -> Result<(), RpcError> {
        self.write(&json!({"id": id, "result": result})).await
    }

    pub async fn respond_error(&self, id: Value, code: i64, message: &str) -> Result<(), RpcError> {
        self.write(&json!({"id": id, "error": {"code": code, "message": message}})).await
    }

    async fn write(&self, message: &Value) -> Result<(), RpcError> {
        let mut line = serde_json::to_vec(message).map_err(|_| RpcError::Closed)?;
        line.push(b'\n');
        let mut writer = self.writer.lock().await;
        let written = async {
            writer.write_all(&line).await?;
            writer.flush().await
        }
        .await;
        written.map_err(|error| {
            tracing::warn!(%error, "writing to the app-server failed");
            self.closed.store(true, Ordering::Relaxed);
            RpcError::Closed
        })
    }
}

async fn read_loop(
    reader: impl AsyncRead + Unpin,
    pending: Pending,
    closed: Arc<AtomicBool>,
    incoming: mpsc::UnboundedSender<Incoming>,
) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(mut message) = serde_json::from_str::<Value>(&line) else {
            tracing::warn!("app-server sent a line that isn't JSON");
            continue;
        };
        let method = message.get("method").and_then(Value::as_str).map(str::to_owned);
        let params = message.get_mut("params").map(Value::take).unwrap_or(Value::Null);
        match (method, message.get_mut("id").map(Value::take)) {
            (Some(method), Some(id)) => {
                let _ = incoming.send(Incoming::Request { id, method, params });
            }
            (Some(method), None) => {
                let _ = incoming.send(Incoming::Notification { method, params });
            }
            (None, Some(id)) => {
                let Some(sender) = id.as_u64().and_then(|id| pending.lock().expect("pending lock").remove(&id)) else {
                    tracing::warn!(%id, "app-server answered a request we didn't send");
                    continue;
                };
                let result = match message.get_mut("error") {
                    Some(error) => Err(RpcError::Server {
                        code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                        message: error.get("message").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    }),
                    None => Ok(message.get_mut("result").map(Value::take).unwrap_or(Value::Null)),
                };
                let _ = sender.send(result);
            }
            (None, None) => tracing::warn!("app-server sent a message with neither id nor method"),
        }
    }
    closed.store(true, Ordering::Relaxed);
    // Dropping the senders fails every request still waiting with `Closed`.
    pending.lock().expect("pending lock").clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn responses_reach_their_request_and_the_rest_comes_out_as_incoming() {
        let (client_side, server_side) = tokio::io::duplex(4096);
        let (client_read, client_write) = tokio::io::split(client_side);
        let (server_read, mut server_write) = tokio::io::split(server_side);
        let (client, mut incoming) = Client::spawn(client_read, client_write);

        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_read).lines();
            let request: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], "ping");
            let id = request["id"].clone();
            let out = format!(
                "{}\n{}\n{}\n",
                json!({"method": "note", "params": {"n": 1}}),
                json!({"id": "srv-1", "method": "ask", "params": {}}),
                json!({"id": id, "result": {"pong": true}}),
            );
            server_write.write_all(out.as_bytes()).await.unwrap();
            let answer: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(answer, json!({"id": "srv-1", "result": {"ok": 1}}));
        });

        assert_eq!(client.request("ping", json!({})).await, Ok(json!({"pong": true})));
        assert!(matches!(incoming.recv().await, Some(Incoming::Notification { method, .. }) if method == "note"));
        let Some(Incoming::Request { id, method, .. }) = incoming.recv().await else { panic!("expected a request") };
        assert_eq!(method, "ask");
        client.respond(id, json!({"ok": 1})).await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_server_that_goes_away_fails_waiting_requests() {
        let (client_side, server_side) = tokio::io::duplex(4096);
        let (client_read, client_write) = tokio::io::split(client_side);
        let (client, mut incoming) = Client::spawn(client_read, client_write);
        let waiting = tokio::spawn(async move { client.request("slow", json!({})).await });
        tokio::task::yield_now().await;
        drop(server_side);
        assert_eq!(waiting.await.unwrap(), Err(RpcError::Closed));
        assert!(incoming.recv().await.is_none());
    }

    #[tokio::test]
    async fn server_errors_come_back_as_errors() {
        let (client_side, server_side) = tokio::io::duplex(4096);
        let (client_read, client_write) = tokio::io::split(client_side);
        let (server_read, mut server_write) = tokio::io::split(server_side);
        let (client, _incoming) = Client::spawn(client_read, client_write);
        tokio::spawn(async move {
            let mut lines = BufReader::new(server_read).lines();
            let request: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            let reply = json!({"id": request["id"], "error": {"code": -32600, "message": "no such thread"}});
            server_write.write_all(format!("{reply}\n").as_bytes()).await.unwrap();
        });
        assert_eq!(
            client.request("thread/resume", json!({})).await,
            Err(RpcError::Server { code: -32600, message: "no such thread".to_owned() })
        );
    }
}
