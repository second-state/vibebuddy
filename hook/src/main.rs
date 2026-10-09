//! `vibebuddy-hook codex` / `vibebuddy-hook claude`: reads the hook payload from stdin, minimizes it and
//! POSTs it to the local daemon; a missing daemon, invalid input or network failure all exit 0 silently, so it never
//! holds up the agent. The app copies it into the bin directory under Application Support and the hook config points
//! there (ADR-0005), so it doesn't depend on python3 and survives the app being moved. Where there is no app (Linux),
//! `vibebuddy-hook install` writes the hook config itself.

mod claude;
mod cli_agents;
mod codex;
mod filter;
mod install;
mod surface;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
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

/// `~/Library/Logs/VibeBuddy` on macOS; elsewhere the daemon's XDG state directory.
fn log_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|home| home.join("Library/Logs/VibeBuddy"));
    }
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.map(|home| home.join(".local/state")))
        .map(|dir| dir.join("vibebuddy"))
}

fn main() {
    let agent = std::env::args().nth(1).unwrap_or_default();
    if let "install" | "uninstall" = agent.as_str() {
        match install::run(agent == "install") {
            Ok(()) => return,
            Err(error) => {
                eprintln!("vibebuddy-hook: {error}");
                std::process::exit(1);
            }
        }
    }
    // `vibebuddy-hook agent-file opencode [hook path]`: prints the file Vibe Buddy owns in that agent's config, for
    // the Mac app to show and write. The hook path defaults to this binary.
    if agent == "agent-file" {
        let mut args = std::env::args().skip(2);
        let Some(target) = args.next().as_deref().and_then(cli_agents::Agent::from_argument) else {
            eprintln!("usage: vibebuddy-hook agent-file opencode|copilot|pi [hook path]");
            std::process::exit(2);
        };
        let hook = args.next().or_else(|| std::env::current_exe().ok().map(|path| path.display().to_string()));
        print!("{}", target.file_contents(&hook.unwrap_or_default()));
        return;
    }
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
            if let Some(log_dir) = log_dir() {
                codex::trace(&source, &payload, &log_dir);
            }
            (codex::ENDPOINT, payload)
        }
        "claude" => match claude::sanitized_payload(&source) {
            Some(payload) => (claude::ENDPOINT, payload),
            None => return,
        },
        "opencode" | "pi" => {
            let plugin = cli_agents::Agent::from_argument(&agent).expect("a plugin agent");
            match cli_agents::plugin_payload(plugin, &source) {
                Some(payload) => (cli_agents::ENDPOINT, payload),
                None => return,
            }
        }
        "copilot" => match std::env::args().nth(2).and_then(|event| cli_agents::copilot_payload(&event, &source)) {
            Some(payload) => (cli_agents::ENDPOINT, payload),
            None => return,
        },
        _ => {
            eprintln!("usage: vibebuddy-hook codex|claude|opencode|copilot|pi <event>  (reads the hook payload from stdin)");
            eprintln!("       vibebuddy-hook agent-file opencode|copilot|pi [hook path]  (prints the file Vibe Buddy owns there)");
            eprintln!("       vibebuddy-hook install|uninstall  (adds or removes the hooks, where there is no app)");
            return;
        }
    };
    let _ = post_json(endpoint, &payload);
}
