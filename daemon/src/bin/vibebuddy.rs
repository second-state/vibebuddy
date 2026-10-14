//! `vibebuddy`: the command line for the local daemon. It only talks to `vibebuddyd` over its HTTP API and never
//! opens the serial port itself (docs/architecture.md, decision 2).
//!
//! - `vibebuddy say <text>`: hands a voice command, typed, to the resident session and prints its spoken replies.
//! - `vibebuddy resident [n]`: prints the resident thread's id and its last `n` (default 5) voice commands with the
//!   spoken replies, read from the thread (or, on Claude Code, the session). Both live in a home of the daemon's
//!   own, so neither Agent's App can open them.
//!
//! The resident session is experimental: the daemon has it only when started with `VIBEBUDDY_RESIDENT_SESSION=1`.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};

/// A voice command waits for a whole Codex turn; the daemon gives up on it after two minutes.
const SAY_TIMEOUT: Duration = Duration::from_secs(180);
/// Reading the history may first have to start the app-server and wait for a turn in progress.
const HISTORY_TIMEOUT: Duration = Duration::from_secs(180);
const QUICK_TIMEOUT: Duration = Duration::from_secs(5);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("say") if args.len() > 1 => say(&args[1..].join(" ")),
        Some("resident") if args.len() == 1 => resident(5),
        Some("resident") if let [_, count] = args.as_slice()
            && let Ok(count) = count.parse() => resident(count),
        _ => {
            eprintln!("usage: vibebuddy say <text>   (a voice command for the resident session, typed)");
            eprintln!("       vibebuddy resident [n] (prints the resident session's last n voice commands and replies)");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("vibebuddy: {error}");
            ExitCode::FAILURE
        }
    }
}

fn say(text: &str) -> Result<(), String> {
    let answer = call("POST", "/v1/resident/say", Some(&json!({"text": text})), SAY_TIMEOUT)?;
    let replies = answer["replies"].as_array().cloned().unwrap_or_default();
    if replies.is_empty() {
        println!("(no spoken reply)");
    }
    for reply in replies {
        println!("{}", reply.as_str().unwrap_or_default());
    }
    if let Some(elapsed) = answer["elapsed_ms"].as_u64() {
        eprintln!("({:.1} s)", elapsed as f64 / 1000.0);
    }
    Ok(())
}

fn resident(limit: usize) -> Result<(), String> {
    let info = call("GET", "/v1/resident", None, QUICK_TIMEOUT)?;
    if info["enabled"] != true {
        return Err("the resident session is off; start vibebuddyd with VIBEBUDDY_RESIDENT_SESSION=1".to_owned());
    }
    match info["problem"].as_str() {
        Some("codex_missing") => eprintln!("voice is off: Codex isn't installed (https://learn.chatgpt.com/docs/codex/cli)"),
        Some("codex_logged_out") => eprintln!("voice is off: Codex isn't logged in; run `codex login`"),
        Some("claude_missing") => eprintln!("voice is off: Claude Code isn't installed (https://code.claude.com/docs/en/setup)"),
        Some("anthropic_key_missing") => eprintln!("voice is off: no Anthropic API key is set for Claude Code (Settings → Conversation)"),
        _ => {}
    }
    if let Some(brain) = info["brain"].as_str() {
        println!("brain {brain}");
    }
    let Some(thread) = info["thread_id"].as_str() else {
        println!("no resident thread yet; it starts with the first `vibebuddy say`");
        return Ok(());
    };
    println!("thread {thread}");
    let history = call("GET", &format!("/v1/resident/history?limit={limit}"), None, HISTORY_TIMEOUT)?;
    for exchange in history["exchanges"].as_array().cloned().unwrap_or_default() {
        println!("\n> {}", exchange["said"].as_str().unwrap_or_default());
        for reply in exchange["replies"].as_array().cloned().unwrap_or_default() {
            println!("  {}", reply.as_str().unwrap_or_default());
        }
    }
    Ok(())
}

/// One request to the daemon at `VIBEBUDDY_BIND` (default 127.0.0.1:7331), like the hook's: no HTTP library.
fn call(method: &str, path: &str, body: Option<&Value>, timeout: Duration) -> Result<Value, String> {
    let address = std::env::var("VIBEBUDDY_BIND").unwrap_or_else(|_| "127.0.0.1:7331".to_owned());
    let socket: SocketAddr = address.parse().map_err(|error| format!("invalid VIBEBUDDY_BIND: {error}"))?;
    let unreachable = |error: std::io::Error| format!("vibebuddyd is not reachable at {address}: {error}");
    let mut stream = TcpStream::connect_timeout(&socket, QUICK_TIMEOUT).map_err(unreachable)?;
    stream.set_read_timeout(Some(timeout)).map_err(unreachable)?;
    let body = body.map(Value::to_string).unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).map_err(unreachable)?;
    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|error| format!("no answer from vibebuddyd: {error}"))?;
    let (head, payload) = response.split_once("\r\n\r\n").ok_or("malformed answer from vibebuddyd")?;
    let status: u16 = head.split_whitespace().nth(1).and_then(|code| code.parse().ok()).unwrap_or(0);
    let value: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let message = value["message"].as_str().map(str::to_owned).unwrap_or_else(|| format!("HTTP {status}"));
        return Err(message);
    }
    Ok(value)
}
