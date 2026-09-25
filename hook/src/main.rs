//! `vibebuddy-hook codex` / `vibebuddy-hook claude`: reads the hook payload from stdin, minimizes it and
//! POSTs it to the local daemon; a missing daemon, invalid input or network failure all exit 0 silently, so it never
//! holds up the agent. The app copies it into the bin directory under Application Support and the hook config points
//! there (ADR-0005), so it doesn't depend on python3 and survives the app being moved.

mod claude;
mod codex;
mod filter;
mod surface;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use serde_json::{Map, Value};

const TIMEOUT: Duration = Duration::from_millis(500);

fn post_json(endpoint: &str, payload: &Map<String, Value>) -> std::io::Result<()> {
    // The endpoint looks like http://127.0.0.1:7331/path; only that form is accepted, no full URL parsing.
    let rest = endpoint.strip_prefix("http://").unwrap_or(endpoint);
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let address: SocketAddr = host.parse().map_err(std::io::Error::other)?;
    let body = serde_json::to_vec(payload)?;
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT)?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    let request = format!(
        "POST /{path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(request.as_bytes())?;
    stream.write_all(&body)?;
    let mut sink = [0_u8; 256];
    let _ = stream.read(&mut sink);
    Ok(())
}

fn main() {
    let agent = std::env::args().nth(1).unwrap_or_default();
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Ok(source) = serde_json::from_str::<Value>(&input) else {
        return;
    };
    let (endpoint, payload) = match agent.as_str() {
        "codex" => {
            let Some(payload) = codex::sanitized_payload(&source) else {
                return;
            };
            if let Some(home) = std::env::var_os("HOME") {
                let log_dir = std::path::PathBuf::from(home).join("Library/Logs/VibeBuddy");
                codex::trace(&source, &payload, &log_dir);
            }
            (codex::ENDPOINT, payload)
        }
        "claude" => match claude::sanitized_payload(&source) {
            Some(payload) => (claude::ENDPOINT, payload),
            None => return,
        },
        _ => {
            eprintln!("用法: vibebuddy-hook codex|claude  （从 stdin 读 Hook 载荷）");
            return;
        }
    };
    let _ = post_json(endpoint, &payload);
}
