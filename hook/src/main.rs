//! `beacon-hook codex` / `beacon-hook claude`：从 stdin 读 Hook 载荷，最小化后
//! POST 给本机 daemon；daemon 不在、输入无效、网络失败都静默退出 0，不能
//! 拖住 Agent。App 把它复制到 Application Support 的 bin 目录，Hook 配置指向
//! 那里（ADR-0005），因此不依赖 python3，也不怕 App 挪位置。

mod claude;
mod codex;
mod filter;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use serde_json::{Map, Value};

const TIMEOUT: Duration = Duration::from_millis(500);

fn post_json(endpoint: &str, payload: &Map<String, Value>) -> std::io::Result<()> {
    // endpoint 形如 http://127.0.0.1:7331/path；只认这种，不做完整的 URL 解析。
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
                let log_dir = std::path::PathBuf::from(home).join("Library/Logs/AgentBeacon");
                codex::trace(&source, &payload, &log_dir);
            }
            (codex::ENDPOINT, payload)
        }
        "claude" => match claude::sanitized_payload(&source) {
            Some(payload) => (claude::ENDPOINT, payload),
            None => return,
        },
        _ => {
            eprintln!("用法: beacon-hook codex|claude  （从 stdin 读 Hook 载荷）");
            return;
        }
    };
    let _ = post_json(endpoint, &payload);
}
