//! Navigates from an aggregated activity back to its source window: through LaunchServices on the Mac, through
//! the compositor (Hyprland) on Linux.
//!
//! All arguments go straight to the process API, never through a shell. Source data comes from hooks and is still treated
//! as untrusted input: Codex thread ids, Claude session ids and GitHub repos are narrowed to a safe character set first.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;

use crate::activity::{ActivitySource, Surface, TmuxPane};

const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const CODEX_BUNDLE_ID: &str = "com.openai.codex";
const CLAUDE_BUNDLE_ID: &str = "com.anthropic.claudefordesktop";
const GHOSTTY_BUNDLE_ID: &str = "com.mitchellh.ghostty";
/// One `id<TAB>title` line per Ghostty terminal. `tab` would name Ghostty's own tab class inside the tell block,
/// hence the character ids.
const GHOSTTY_LIST_TERMINALS: [&str; 7] = [
    "tell application id \"com.mitchellh.ghostty\"",
    "set out to \"\"",
    "repeat with t in terminals",
    "set out to out & (id of t) & (character id 9) & (name of t) & (character id 10)",
    "end repeat",
    "return out",
    "end tell",
];
/// The daemon starts with launchd's bare PATH, so tmux is looked for where package managers put it.
const TMUX_PATHS: [&str; 4] = ["/opt/homebrew/bin/tmux", "/usr/local/bin/tmux", "/usr/bin/tmux", "/run/current-system/sw/bin/tmux"];
/// How long Ghostty gets to read a title off the tty before the listing is retried.
const GHOSTTY_TITLE_POLLS: u32 = 10;
const GHOSTTY_TITLE_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// The terminal id comes in as an argument, never spliced into the script.
const GHOSTTY_FOCUS_TERMINAL: [&str; 6] = [
    "on run argv",
    "tell application id \"com.mitchellh.ghostty\"",
    "focus terminal id (item 1 of argv)",
    "activate",
    "end tell",
    "end run",
];
/// Where Claude App keeps a record of each Code session, one subdirectory per account.
const DESKTOP_SESSIONS_DIR: &str = "Library/Application Support/Claude/claude-code-sessions";
/// Prefix of desktop session ids; the deep link only accepts this form.
const DESKTOP_ID_PREFIX: &str = "local_";

#[derive(Debug, PartialEq, Eq)]
struct CommandSpec {
    program: &'static str,
    args: Vec<String>,
}

/// On success returns the link actually opened, so the log can say where K2 went.
pub async fn open(source: ActivitySource) -> Result<String, String> {
    if let Some(Surface::Window { pids }) = surface_of(&source) {
        return focus_window(pids).await;
    }
    if let Some(Surface::Host { bundle_id, tty, tmux }) = surface_of(&source) {
        // Inside tmux, first switch the user's tmux client to the session's pane; the tab to find is then the
        // one that client runs in.
        let tty = match tmux {
            Some(tmux) => match focus_tmux_pane(tmux).await {
                Ok(client_tty) => Some(client_tty),
                Err(error) => {
                    tracing::info!(%error, "cannot switch tmux to the session's pane");
                    None
                }
            },
            None => tty.clone(),
        };
        // Ghostty can be asked about its terminals, so go to that tab. Failing that, Ghostty is still brought
        // forward below, as any other host is.
        if bundle_id == GHOSTTY_BUNDLE_ID
            && let Some(tty) = tty
        {
            match focus_ghostty_terminal(&tty).await {
                Ok(target) => return Ok(target),
                Err(error) => tracing::info!(%error, "cannot find the session's Ghostty tab, bringing Ghostty forward"),
            }
        }
    }
    let desktop = reported_desktop_session(&source)
        .map(str::to_owned)
        .or_else(|| fallback_desktop_session(&source));
    let spec = command_for(&source, desktop.as_deref())?;
    let link = spec.args.last().cloned().unwrap_or_default();
    let output = tokio::time::timeout(
        OPEN_TIMEOUT,
        Command::new(spec.program).args(&spec.args).output(),
    )
    .await
    .map_err(|_| "opening the source timed out".to_owned())?
    .map_err(|error| format!("cannot start the open command: {error}"))?;
    if output.status.success() {
        return Ok(link);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("failed to open the source: {}", stderr.trim()))
}

fn command_for(source: &ActivitySource, desktop: Option<&str>) -> Result<CommandSpec, String> {
    // First answer one question: is the user inside this agent's app? If not, don't use the deeplink;
    // it would import a terminal session into the app as a copy.
    match surface_of(source) {
        Some(Surface::Host { bundle_id, .. }) => return activate(bundle_id),
        Some(Surface::Headless) => return Err("session has no host window (SSH or background process)".to_owned()),
        Some(Surface::Window { .. }) => return Err("a window is focused through the compositor, not a command".to_owned()),
        _ => {}
    }
    match source {
        ActivitySource::Codex { thread_id, .. } => {
            if !valid_identifier(thread_id) {
                return Err("Codex thread id contains invalid characters".to_owned());
            }
            Ok(CommandSpec {
                program: "/usr/bin/open",
                args: vec![
                    "-b".to_owned(),
                    CODEX_BUNDLE_ID.to_owned(),
                    format!("codex://threads/{thread_id}"),
                ],
            })
        }
        ActivitySource::ClaudeCode { session_id, .. } => {
            // When we already know the window, focus it directly. `claude://resume` takes a different
            // route: it claims a transcript on disk by CLI session id. When the same id has a copy in several
            // project directories it can only pick one, and picking wrong opens a stale
            // shadow session, re-importing the whole transcript on every press.
            let link = match desktop {
                Some(desktop) => {
                    if !valid_desktop_id(desktop) {
                        return Err("invalid desktop session id".to_owned());
                    }
                    format!("claude://code/continue?session={desktop}")
                }
                // CLI sessions running in a terminal have no matching window; importing is the only way to open them.
                None => {
                    if !valid_identifier(session_id) {
                        return Err("Claude session id contains invalid characters".to_owned());
                    }
                    format!("claude://resume?session={session_id}")
                }
            };
            Ok(CommandSpec {
                program: "/usr/bin/open",
                args: vec!["-b".to_owned(), CLAUDE_BUNDLE_ID.to_owned(), link],
            })
        }
        ActivitySource::GitHubActions { repo, run_id } => {
            if !valid_repo(repo) {
                return Err("invalid GitHub repo".to_owned());
            }
            Ok(CommandSpec {
                program: if cfg!(target_os = "macos") { "/usr/bin/open" } else { "xdg-open" },
                args: vec![format!("https://github.com/{repo}/actions/runs/{run_id}")],
            })
        }
    }
}

/// Code sessions started by Claude App put the desktop session id in the process environment, and the hook reports it verbatim.
/// It's an identity the app itself assigned and can be used directly, with no need to claim a
/// transcript on disk by CLI session id, which is one-to-many and opens a stale shadow session (see LESSONS.md).
fn reported_desktop_session(source: &ActivitySource) -> Option<&str> {
    match source {
        ActivitySource::ClaudeCode { surface: Surface::App { desktop_session_id }, .. } => desktop_session_id.as_deref(),
        _ => None,
    }
}

/// Old hooks and old state files don't report the desktop session id, so disambiguate by cwd in the session index.
fn fallback_desktop_session(source: &ActivitySource) -> Option<String> {
    let ActivitySource::ClaudeCode { session_id, cwd, surface: Surface::App { desktop_session_id: None } } = source else {
        return None;
    };
    desktop_session_by_cli(&desktop_sessions_dir()?, session_id, cwd.as_deref())
}

fn surface_of(source: &ActivitySource) -> Option<&Surface> {
    match source {
        ActivitySource::Codex { surface, .. } | ActivitySource::ClaudeCode { surface, .. } => Some(surface),
        ActivitySource::GitHubActions { .. } => None,
    }
}

/// Focuses the Ghostty terminal on the agent's tty. Ghostty doesn't tell which terminal owns which tty, but it
/// does report titles, and anything written to a tty is read by the terminal that owns it: so the tab is
/// briefly given a one-off title, found by it, and handed its own title back. Ghostty has no title stack to
/// restore from, so the original comes from a listing taken first.
async fn focus_ghostty_terminal(tty: &str) -> Result<String, String> {
    if !valid_tty(tty) {
        return Err("invalid tty".to_owned());
    }
    let before = run_osascript(&GHOSTTY_LIST_TERMINALS, &[]).await?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let marker = format!("vibebuddy-{nanos}");
    set_title(tty, &marker)?;
    let mut found = None;
    for _ in 0..GHOSTTY_TITLE_POLLS {
        tokio::time::sleep(GHOSTTY_TITLE_POLL_INTERVAL).await;
        let listing = run_osascript(&GHOSTTY_LIST_TERMINALS, &[]).await;
        if let Some(id) = listing.ok().and_then(|listing| terminal_titled(&listing, &marker)) {
            found = Some(id);
            break;
        }
    }
    // Hand the title back before anything else can fail. If no terminal showed the marker, the tty isn't a
    // Ghostty tab of its own (tmux, for one, keeps titles to itself), and clearing is all that's left.
    let original = found.as_deref().and_then(|id| title_of(&before, id)).unwrap_or_default();
    set_title(tty, &original)?;
    let id = found.ok_or_else(|| format!("no Ghostty terminal is on {tty}"))?;
    run_osascript(&GHOSTTY_FOCUS_TERMINAL, &[&id]).await?;
    Ok(format!("ghostty terminal {id}"))
}

fn terminals(listing: &str) -> impl Iterator<Item = (&str, &str)> {
    listing
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(id, _)| valid_identifier(id))
}

fn terminal_titled(listing: &str, title: &str) -> Option<String> {
    terminals(listing).find(|(_, name)| *name == title).map(|(id, _)| id.to_owned())
}

fn title_of(listing: &str, id: &str) -> Option<String> {
    terminals(listing).find(|(terminal, _)| *terminal == id).map(|(_, name)| name.to_owned())
}

/// Writes an OSC 2 title sequence to the tty. Control characters are dropped so a title read back from the
/// terminal can't end the sequence early and smuggle in one of its own.
fn set_title(tty: &str, title: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let title: String = title.chars().filter(|character| !character.is_control()).collect();
    std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
        .open(format!("/dev/{tty}"))
        .and_then(|mut device| device.write_all(format!("\x1b]2;{title}\x07").as_bytes()))
        .map_err(|error| format!("cannot write to {tty}: {error}"))
}

/// `ttys016`: a pseudo-terminal name, nothing that could point elsewhere under /dev.
fn valid_tty(tty: &str) -> bool {
    tty.strip_prefix("tty")
        .is_some_and(|rest| !rest.is_empty() && rest.len() <= 8 && rest.bytes().all(|byte| byte.is_ascii_alphanumeric()))
}

/// Switches a tmux client to the session's pane, across sessions and windows, and returns that client's tty:
/// the terminal tab the user sees tmux in. A client already showing the pane's session is preferred, so a
/// client watching some other session isn't pulled away from it; otherwise the one used most recently.
async fn focus_tmux_pane(tmux: &TmuxPane) -> Result<String, String> {
    if !valid_tmux_pane(&tmux.pane) {
        return Err("invalid tmux pane".to_owned());
    }
    check_own_socket(&tmux.socket)?;
    let program = TMUX_PATHS
        .into_iter()
        .find(|path| Path::new(path).exists())
        .ok_or_else(|| "tmux not found".to_owned())?;
    let socket = tmux.socket.as_str();
    let pane = tmux.pane.as_str();
    let session = run(program, &["-S", socket, "display-message", "-p", "-t", pane, "#{session_id}"]).await?;
    let clients = run(program, &["-S", socket, "list-clients", "-F", "#{client_activity} #{client_tty} #{session_id}"]).await?;
    let client = tmux_client_for(&clients, session.trim()).ok_or_else(|| "no tmux client is attached".to_owned())?;
    run(program, &["-S", socket, "switch-client", "-c", &client, "-t", pane]).await?;
    Ok(client.trim_start_matches("/dev/").to_owned())
}

fn tmux_client_for(clients: &str, session: &str) -> Option<String> {
    clients
        .lines()
        .filter_map(|line| {
            // tmux turns tabs in the format into `_`; none of these fields can hold a space.
            let mut fields = line.split(' ');
            let activity: u64 = fields.next()?.parse().ok()?;
            let tty = fields.next()?;
            let same_session = fields.next()? == session;
            tty.starts_with("/dev/").then_some((same_session, activity, tty))
        })
        .max_by_key(|&(same_session, activity, _)| (same_session, activity))
        .map(|(_, _, tty)| tty.to_owned())
}

fn valid_tmux_pane(pane: &str) -> bool {
    pane.strip_prefix('%')
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
}

/// The socket path comes from the hook; only a socket this user owns is handed to tmux.
fn check_own_socket(socket: &str) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let metadata = std::fs::metadata(socket).map_err(|error| format!("cannot read tmux socket: {error}"))?;
    let own = metadata.uid() == unsafe { libc::getuid() };
    if socket.starts_with('/') && metadata.file_type().is_socket() && own {
        Ok(())
    } else {
        Err("not a tmux socket of this user".to_owned())
    }
}

/// Runs a fixed AppleScript. The first run against an app asks the user for Automation access; until they answer,
/// the call waits and then times out here, and the caller falls back to bringing the app forward.
async fn run_osascript(script: &[&str], args: &[&str]) -> Result<String, String> {
    let mut command_line: Vec<&str> = script.iter().flat_map(|line| ["-e", *line]).collect();
    command_line.extend_from_slice(args);
    run("/usr/bin/osascript", &command_line).await
}

/// Runs a helper with arguments passed straight through, never a shell, and returns its stdout.
async fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = tokio::time::timeout(OPEN_TIMEOUT, Command::new(program).args(args).kill_on_drop(true).output())
        .await
        .map_err(|_| format!("{program} timed out"))?
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{program} failed: {}", stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Brings the host app to the front. Whether we recognise the bundle id doesn't matter: unfamiliar terminals take
/// this same path, so supporting a new terminal needs no code change. The value comes from the hook and is still
/// narrowed to a safe character set as untrusted input.
fn activate(bundle_id: &str) -> Result<CommandSpec, String> {
    if !valid_bundle_id(bundle_id) {
        return Err("invalid host bundle id".to_owned());
    }
    Ok(CommandSpec {
        program: "/usr/bin/open",
        args: vec!["-b".to_owned(), bundle_id.to_owned()],
    })
}

/// Focuses the window that owns the nearest of the agent's ancestors. Only Hyprland is supported: it's the one
/// compositor here, and it can both list windows with their pids and focus one by address.
async fn focus_window(pids: &[u32]) -> Result<String, String> {
    let clients = run_hyprctl(&["clients", "-j"]).await?;
    let address = window_for(&clients, pids)
        .ok_or_else(|| "session has no host window (SSH, tmux or a background process)".to_owned())?;
    let target = format!("address:{address}");
    // Hyprland 0.56 made dispatch take Lua and rejects the old syntax; older releases only know the old one.
    // The address is plain hex (checked above), so it can't break out of the Lua string.
    let lua = format!("hl.dsp.focus({{ window = \"{target}\" }})");
    if let Err(lua_error) = run_hyprctl(&["dispatch", &lua]).await {
        run_hyprctl(&["dispatch", "focuswindow", &target])
            .await
            .map_err(|legacy_error| format!("{lua_error}; legacy syntax: {legacy_error}"))?;
    }
    Ok(target)
}

/// hyprctl reports dispatch errors on stdout, sometimes with a zero exit code; a dispatch that worked says `ok`.
async fn run_hyprctl(args: &[&str]) -> Result<String, String> {
    let output = tokio::time::timeout(OPEN_TIMEOUT, Command::new("hyprctl").args(args).output())
        .await
        .map_err(|_| "hyprctl timed out".to_owned())?
        .map_err(|error| format!("cannot run hyprctl (is this Hyprland?): {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let failed = !output.status.success() || (args.first() == Some(&"dispatch") && stdout.trim() != "ok");
    if failed {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("hyprctl failed: {} {}", stdout.trim(), stderr.trim()).trim_end().to_owned());
    }
    Ok(stdout)
}

/// Picks the window owned by the nearest ancestor. When one process owns several windows (a single-instance
/// terminal), the first one Hyprland lists wins: pids alone can't tell them apart.
fn window_for(clients_json: &str, pids: &[u32]) -> Option<String> {
    #[derive(Deserialize)]
    struct Client {
        address: String,
        pid: i64,
    }
    let clients: Vec<Client> = serde_json::from_str(clients_json).ok()?;
    pids.iter().find_map(|&pid| {
        clients
            .iter()
            .find(|client| client.pid == i64::from(pid) && valid_address(&client.address))
            .map(|client| client.address.clone())
    })
}

/// Hyprland window addresses look like `0x618656fe7aa0`; anything else never reaches the dispatcher.
fn valid_address(address: &str) -> bool {
    address
        .strip_prefix("0x")
        .is_some_and(|hex| !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// Claude App stores one record per Code session. Only the fields needed for navigation are read here.
#[derive(Debug, Deserialize)]
struct DesktopSession {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "cliSessionId")]
    cli_session_id: Option<String>,
    cwd: Option<String>,
    #[serde(default, rename = "isArchived")]
    is_archived: bool,
    #[serde(default, rename = "lastActivityAt")]
    last_activity_at: i64,
    /// The session title the app generated; the task card's first line uses it.
    #[serde(default)]
    title: Option<String>,
}

fn desktop_sessions_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join(DESKTOP_SESSIONS_DIR))
}

/// Finds which desktop window this CLI session belongs to right now.
///
/// `cliSessionId` isn't a unique key: a session moving back to the main repository after its worktree is deleted, a fork, or
/// a `claude://resume` import all give one CLI session several records. The working directory
/// tells the real one from the shadows: the cwd the hook reports is where that process actually is.
fn desktop_session_by_cli(dir: &Path, cli_session_id: &str, cwd: Option<&str>) -> Option<String> {
    desktop_session(dir, cli_session_id, cwd).map(|session| session.session_id)
}

/// This CLI session's title in Claude App. Records are picked by the same rule as when opening the window.
pub(crate) fn desktop_session_title(cli_session_id: &str, cwd: Option<&str>) -> Option<String> {
    let dir = desktop_sessions_dir()?;
    desktop_session(&dir, cli_session_id, cwd)
        .and_then(|session| session.title)
        .filter(|title| !title.trim().is_empty())
}

fn desktop_session(dir: &Path, cli_session_id: &str, cwd: Option<&str>) -> Option<DesktopSession> {
    let mut candidates = Vec::new();
    collect_desktop_sessions(dir, 0, &mut candidates);
    candidates.retain(|session| {
        !session.is_archived && session.cli_session_id.as_deref() == Some(cli_session_id)
    });
    if let Some(cwd) = cwd
        && let Some(index) = candidates
            .iter()
            .position(|session| session.cwd.as_deref() == Some(cwd))
    {
        return Some(candidates.swap_remove(index));
    }
    candidates.sort_by_key(|session| session.last_activity_at);
    candidates.pop()
}

/// Records are stored as `<account>/<org>/local_*.json`; the tree is shallow, so read it level by level.
fn collect_desktop_sessions(dir: &Path, depth: u32, out: &mut Vec<DesktopSession>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_desktop_sessions(&path, depth + 1, out);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Ok(session) = serde_json::from_str::<DesktopSession>(&text) {
                out.push(session);
            }
        }
    }
}

/// Bundle ids allow the same characters as session ids, but must be dotted, with no paths or whitespace.
fn valid_bundle_id(value: &str) -> bool {
    valid_identifier(value) && value.contains('.')
}

fn valid_desktop_id(value: &str) -> bool {
    value.starts_with(DESKTOP_ID_PREFIX) && valid_identifier(value)
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    matches!((parts.next(), parts.next(), parts.next()), (Some(owner), Some(name), None) if valid(owner) && valid(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ghostty_terminal_showing_the_marker_is_found_and_its_title_kept() {
        let before = "1CF0AA92-F606-4D99-BA8C-B686DF838744\t✳ Fix the build\nD7394CAC-B8EA-44DF-A4E5-C0676CF3F0E4\tdragon@mac:~/notes\n";
        let after = "1CF0AA92-F606-4D99-BA8C-B686DF838744\tvibebuddy-42\nD7394CAC-B8EA-44DF-A4E5-C0676CF3F0E4\tdragon@mac:~/notes\n";
        let id = terminal_titled(after, "vibebuddy-42").expect("marked terminal");
        assert_eq!(id, "1CF0AA92-F606-4D99-BA8C-B686DF838744");
        assert_eq!(title_of(before, &id).as_deref(), Some("✳ Fix the build"));
        assert_eq!(terminal_titled(before, "vibebuddy-42"), None);
    }

    #[test]
    fn a_ghostty_terminal_id_that_could_break_out_is_ignored() {
        let listing = "x\" & do shell script \"y\tvibebuddy-42\n";
        assert_eq!(terminal_titled(listing, "vibebuddy-42"), None);
    }

    #[test]
    fn the_tmux_client_on_the_panes_session_is_switched_before_a_busier_one() {
        let clients = "1791453500 /dev/ttys044 $1\n1791453400 /dev/ttys016 $0\n";
        assert_eq!(tmux_client_for(clients, "$0").as_deref(), Some("/dev/ttys016"));
        // Nobody watches the pane's session: the most recently used client is moved there.
        assert_eq!(tmux_client_for(clients, "$7").as_deref(), Some("/dev/ttys044"));
        assert_eq!(tmux_client_for("", "$0"), None);
    }

    #[test]
    fn only_tmux_pane_ids_reach_tmux() {
        assert!(valid_tmux_pane("%3"));
        for bogus in ["", "%", "3", "%3;kill-server", "%-1"] {
            assert!(!valid_tmux_pane(bogus), "{bogus}");
        }
    }

    #[test]
    fn only_pseudo_terminal_names_are_written_to() {
        assert!(valid_tty("ttys016"));
        for bogus in ["", "tty", "ttys016/../disk0", "disk0", "console", "ttys 1"] {
            assert!(!valid_tty(bogus), "{bogus}");
        }
    }

    #[test]
    fn a_host_tty_survives_a_state_file_round_trip() {
        let surface = Surface::Host { bundle_id: GHOSTTY_BUNDLE_ID.to_owned(), tty: Some("ttys016".to_owned()), tmux: None };
        let json = serde_json::to_string(&surface).unwrap();
        assert_eq!(serde_json::from_str::<Surface>(&json).unwrap(), surface);
        // State files from before the tty was reported still load.
        let old: Surface = serde_json::from_str(r#"{"surface":"host","bundle_id":"com.mitchellh.ghostty"}"#).unwrap();
        assert_eq!(old, Surface::Host { bundle_id: GHOSTTY_BUNDLE_ID.to_owned(), tty: None, tmux: None });
    }

    #[test]
    fn a_reported_desktop_session_skips_the_disk_lookup() {
        // The app's own identity wins; no more claiming transcripts on disk by CLI session id.
        let source = ActivitySource::ClaudeCode {
            session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
            cwd: Some("/work/vibe-buddy".to_owned()),
            surface: Surface::App {
                desktop_session_id: Some("local_b65a60de-9adb-48b0-85c6-f9a178971322".to_owned()),
            },
        };

        assert_eq!(reported_desktop_session(&source), Some("local_b65a60de-9adb-48b0-85c6-f9a178971322"));
        assert!(fallback_desktop_session(&source).is_none());
    }

    #[test]
    fn a_terminal_session_never_looks_for_a_desktop_window() {
        let source = ActivitySource::ClaudeCode {
            session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
            cwd: Some("/work/vibe-buddy".to_owned()),
            surface: Surface::Host { tty: None, tmux: None, bundle_id: "com.mitchellh.ghostty".to_owned() },
        };

        assert!(reported_desktop_session(&source).is_none());
        assert!(fallback_desktop_session(&source).is_none());
    }

    #[test]
    fn a_terminal_session_activates_its_host_instead_of_importing() {
        // The user runs claude in Ghostty: the deeplink would import the session into the app as a copy;
        // the right move is to bring that terminal to the front.
        let spec = command_for(
            &ActivitySource::ClaudeCode {
                session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
                cwd: Some("/work/vibe-buddy".to_owned()),
                surface: Surface::Host { tty: None, tmux: None, bundle_id: "com.mitchellh.ghostty".to_owned() },
            },
            None,
        )
        .expect("host should be activatable");

        assert_eq!(spec.program, "/usr/bin/open");
        assert_eq!(spec.args, ["-b", "com.mitchellh.ghostty"]);
    }

    #[test]
    fn an_unknown_host_needs_no_code_change() {
        // Unfamiliar terminals take the same path as known ones, so switching terminals needs no code change.
        let spec = command_for(
            &ActivitySource::Codex {
                thread_id: "019c6e27-e55b-73d1-87d8-4e01f1f75043".to_owned(),
                surface: Surface::Host { tty: None, tmux: None, bundle_id: "net.example.SomeNewTerminal".to_owned() },
            },
            None,
        )
        .expect("an unknown host should still be activated");

        assert_eq!(spec.args, ["-b", "net.example.SomeNewTerminal"]);
    }

    #[test]
    fn a_headless_session_is_skipped_rather_than_opened_wrong() {
        // Sessions started over SSH or by a daemon have no window at all; return an error so K2 tries the next candidate.
        let spec = command_for(
            &ActivitySource::Codex {
                thread_id: "019c6e27-e55b-73d1-87d8-4e01f1f75043".to_owned(),
                surface: Surface::Headless,
            },
            None,
        );

        assert!(spec.is_err());
    }

    #[test]
    fn a_host_id_that_is_not_a_bundle_id_is_refused() {
        // The bundle id comes from the hook and is treated as untrusted input.
        for bogus in ["../../evil", "com.example.a b", "no-dots", ""] {
            assert!(
                command_for(
                    &ActivitySource::Codex {
                        thread_id: "t".to_owned(),
                        surface: Surface::Host { bundle_id: bogus.to_owned(), tty: None, tmux: None },
                    },
                    None,
                )
                .is_err(),
                "{bogus} should not be accepted"
            );
        }
    }

    #[test]
    fn a_host_without_a_bundle_id_has_nowhere_to_go() {
        // The hook says there's a host but gave no target: skip it, rather than fall back to importing the session into the app.
        assert_eq!(Surface::from_hook(Some("host"), None, None, None, None, None), Surface::Headless);
    }

    #[test]
    fn an_older_hook_keeps_the_desktop_behaviour() {
        // Old hooks and old state files report no surface; back then only the desktop app was supported.
        assert_eq!(Surface::from_hook(None, None, None, None, None, None), Surface::default());
        assert!(matches!(Surface::default(), Surface::App { .. }));
    }

    #[test]
    fn codex_thread_uses_the_native_deep_link() {
        let spec = command_for(
            &ActivitySource::Codex {
                thread_id: "019c6e27-e55b-73d1-87d8-4e01f1f75043".to_owned(),
                surface: Surface::default(),
            },
            None,
        )
        .expect("UUID should open");

        assert_eq!(spec.program, "/usr/bin/open");
        assert_eq!(spec.args[0..2], ["-b", CODEX_BUNDLE_ID]);
        assert_eq!(
            spec.args[2],
            "codex://threads/019c6e27-e55b-73d1-87d8-4e01f1f75043"
        );
    }

    #[test]
    fn a_known_window_is_focused_instead_of_reimported() {
        let spec = command_for(
            &ActivitySource::ClaudeCode {
                session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
                cwd: Some("/work/vibe-buddy".to_owned()),
                surface: Surface::default(),
            },
            Some("local_b65a60de-9adb-48b0-85c6-f9a178971322"),
        )
        .expect("known window should open");

        assert_eq!(spec.program, "/usr/bin/open");
        assert_eq!(spec.args[0..2], ["-b", CLAUDE_BUNDLE_ID]);
        assert_eq!(
            spec.args[2],
            "claude://code/continue?session=local_b65a60de-9adb-48b0-85c6-f9a178971322"
        );
    }

    #[test]
    fn a_cli_session_without_a_window_falls_back_to_importing() {
        let spec = command_for(
            &ActivitySource::ClaudeCode {
                session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
                cwd: None,
                surface: Surface::default(),
            },
            None,
        )
        .expect("a session in a terminal should still open");

        assert_eq!(
            spec.args[2],
            "claude://resume?session=19b63622-e3e0-4cd0-a37e-dc8d71253155"
        );
    }

    #[test]
    fn a_forged_desktop_id_is_rejected() {
        assert!(
            command_for(
                &ActivitySource::ClaudeCode {
                    session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
                    cwd: None,
                    surface: Surface::default(),
                },
                Some("local_../../tmp"),
            )
            .is_err()
        );
        assert!(
            command_for(
                &ActivitySource::ClaudeCode {
                    session_id: "19b63622-e3e0-4cd0-a37e-dc8d71253155".to_owned(),
                    cwd: None,
                    surface: Surface::default(),
                },
                Some("19b63622-e3e0-4cd0-a37e-dc8d71253155"),
            )
            .is_err(),
            "an id without the local_ prefix is not a desktop session"
        );
    }

    #[test]
    fn malformed_remote_identifiers_are_rejected() {
        assert!(
            command_for(
                &ActivitySource::Codex {
                    thread_id: "../../tmp".to_owned(),
                    surface: Surface::default(),
                },
                None,
            )
            .is_err()
        );
        assert!(
            command_for(
                &ActivitySource::ClaudeCode {
                    session_id: "../../tmp".to_owned(),
                    cwd: None,
                    surface: Surface::default(),
                },
                None,
            )
            .is_err()
        );
        assert!(
            command_for(
                &ActivitySource::GitHubActions {
                    repo: "owner/repo/issues/1".to_owned(),
                    run_id: 1,
                },
                None,
            )
            .is_err()
        );
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("vibebuddy-open-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        base
    }

    fn write_session(dir: &Path, id: &str, cli: &str, cwd: &str, archived: bool, activity: i64) {
        let record = format!(
            r#"{{"sessionId":"{id}","cliSessionId":"{cli}","cwd":"{cwd}","isArchived":{archived},"lastActivityAt":{activity}}}"#
        );
        std::fs::write(dir.join(format!("{id}.json")), record).expect("write session record");
    }

    /// A session whose worktree was deleted leaves two records with the same `cliSessionId` in the index:
    /// the one moved back to the main repository, and the shadow an earlier K2 imported. cwd must pick the real one, or K2
    /// opens a stale session whose content stopped yesterday.
    #[test]
    fn the_working_directory_picks_the_live_window() {
        let root = temp_dir("split");
        let dir = root.join("account").join("org");
        std::fs::create_dir_all(&dir).expect("create test dir");
        let cli = "19b63622-e3e0-4cd0-a37e-dc8d71253155";
        // The shadow has the later activity time, since importing itself refreshes it, so "take the newest" isn't enough.
        write_session(
            &dir,
            "local_19b63622-e3e0-4cd0-a37e-dc8d71253155",
            cli,
            "/work/vibe-buddy",
            false,
            1_789_437_619_665,
        );
        write_session(
            &dir,
            "local_b65a60de-9adb-48b0-85c6-f9a178971322",
            cli,
            "/work/vibe-buddy/.claude/worktrees/git-status",
            false,
            1_789_437_616_240,
        );

        let picked = desktop_session_by_cli(
            &root,
            cli,
            Some("/work/vibe-buddy/.claude/worktrees/git-status"),
        );
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(
            picked.as_deref(),
            Some("local_b65a60de-9adb-48b0-85c6-f9a178971322"),
            "K2 should return to the window working in this directory"
        );
    }

    #[test]
    fn an_archived_window_is_never_reopened() {
        let root = temp_dir("archived");
        let dir = root.join("account").join("org");
        std::fs::create_dir_all(&dir).expect("create test dir");
        write_session(
            &dir,
            "local_aaaaaaaa-0000-0000-0000-000000000000",
            "cli-a",
            "/work/alpha",
            true,
            1,
        );

        let picked = desktop_session_by_cli(&root, "cli-a", Some("/work/alpha"));
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(picked, None, "K2 should not bring back an archived session");
    }

    /// When a CLI session maps to a single window, a cwd mismatch still mustn't fall back to importing.
    #[test]
    fn a_single_window_wins_even_when_the_directory_moved() {
        let root = temp_dir("moved");
        let dir = root.join("account").join("org");
        std::fs::create_dir_all(&dir).expect("create test dir");
        write_session(
            &dir,
            "local_cccccccc-0000-0000-0000-000000000000",
            "cli-c",
            "/work/new-place",
            false,
            5,
        );

        let picked = desktop_session_by_cli(&root, "cli-c", Some("/work/old-place"));
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(
            picked.as_deref(),
            Some("local_cccccccc-0000-0000-0000-000000000000")
        );
    }

    const CLIENTS: &str = r#"[
        {"address": "0x618656fe7aa0", "pid": 3534, "class": "org.omarchy.agent"},
        {"address": "0x618657e07910", "pid": 5487, "class": "org.omarchy.agent"},
        {"address": "not-an-address", "pid": 9000, "class": "evil"}
    ]"#;

    #[test]
    fn the_nearest_ancestor_with_a_window_wins() {
        // The hook's chain on Omarchy: claude, then the Ghostty that owns the window, then the user's systemd.
        assert_eq!(window_for(CLIENTS, &[3620, 3534, 1017]).as_deref(), Some("0x618656fe7aa0"));
        assert_eq!(window_for(CLIENTS, &[5576, 5487]).as_deref(), Some("0x618657e07910"));
    }

    #[test]
    fn a_chain_without_a_window_has_nowhere_to_go() {
        // tmux and SSH sessions climb to a server or sshd, never to a window.
        assert_eq!(window_for(CLIENTS, &[4000, 3999, 1017]), None);
        assert_eq!(window_for("not json", &[3534]), None);
    }

    #[test]
    fn a_malformed_address_never_reaches_the_dispatcher() {
        assert_eq!(window_for(CLIENTS, &[9000]), None);
    }

    #[test]
    fn a_window_surface_is_never_turned_into_a_command() {
        let source = ActivitySource::ClaudeCode {
            session_id: "s".to_owned(),
            cwd: None,
            surface: Surface::Window { pids: vec![3534] },
        };
        assert!(command_for(&source, None).is_err());
    }

    #[test]
    fn github_run_uses_the_exact_run_url() {
        let spec = command_for(
            &ActivitySource::GitHubActions {
                repo: "longzhi/vibe-buddy".to_owned(),
                run_id: 42,
            },
            None,
        )
        .expect("canonical repo name should open");

        assert_eq!(
            spec.args,
            ["https://github.com/longzhi/vibe-buddy/actions/runs/42"]
        );
    }
}
