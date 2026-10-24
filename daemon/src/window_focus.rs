//! Brings the window of an agent's session forward on Linux, on the window system that session runs. The evidence is
//! the environment of the processes being focused (`/proc/<pid>/environ`): the terminal an agent runs in carries the
//! variables of the graphical session its window belongs to, whatever any other session left behind. A window system
//! counts only on live evidence; with none, nothing is focused. Each backend lists the windows with their pids and
//! focuses the one owned by the nearest of the agent's ancestors.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::source_opener::run_with_env;

pub const NO_HOST_WINDOW: &str = "session has no host window (SSH, tmux or a background process)";
pub const NO_WINDOW_SYSTEM: &str = "the session runs no supported window system (Hyprland, Sway or X11)";

/// The variables that say which window system a session runs and how its helpers reach it.
const SESSION_VARIABLES: [&str; 7] = [
    "HYPRLAND_INSTANCE_SIGNATURE",
    "SWAYSOCK",
    "XDG_SESSION_TYPE",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "XDG_RUNTIME_DIR",
];

/// The ones only a graphical session sets: an environment with none of them says nothing about a window system.
const GRAPHICAL_VARIABLES: [&str; 4] = ["HYPRLAND_INSTANCE_SIGNATURE", "SWAYSOCK", "WAYLAND_DISPLAY", "DISPLAY"];

/// A session's variables, by name; also the environment the backend's helper runs with.
pub type Session = BTreeMap<&'static str, String>;

/// The session variables among `entries`, each `NAME=value`: a process's environment, or the lines of
/// `systemctl --user show-environment`. A value systemd had to quote (`$'…'`) is dropped: none of these normally needs it.
pub fn session<'a>(entries: impl IntoIterator<Item = &'a str>) -> Session {
    entries
        .into_iter()
        .filter_map(|entry| entry.split_once('='))
        .filter(|(_, value)| !value.is_empty() && !value.starts_with("$'"))
        .filter_map(|(name, value)| Some((*SESSION_VARIABLES.iter().find(|known| **known == name)?, value.to_owned())))
        .collect()
}

/// Whether an environment belongs to a graphical session at all.
pub fn graphical(session: &Session) -> bool {
    GRAPHICAL_VARIABLES.iter().any(|name| session.contains_key(name))
}

/// The session of the nearest process in `environments` (nearest first; None where one couldn't be read) whose
/// environment is graphical. It is taken whole, never merged with another source's variables.
pub fn source_session(environments: impl IntoIterator<Item = Option<Session>>) -> Option<Session> {
    environments.into_iter().flatten().find(graphical)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Compositor {
    Hyprland,
    Sway,
    X11,
}

/// Which window system a session runs, if any we support. `var` reads one of its variables; `live` tells whether a
/// Unix socket at a path accepts a connection. Both are parameters, so the choice is testable without a desktop.
///
/// Hyprland and Sway count only while their socket answers, so a variable an ended session left behind (in systemd's
/// user manager, say) never wins. Hyprland listens in `$XDG_RUNTIME_DIR/hypr/<signature>`, or in `/tmp/hypr` before
/// 0.40. X11 needs `XDG_SESSION_TYPE=x11`, or no session type and a `DISPLAY` without a `WAYLAND_DISPLAY`: Xwayland
/// sets `DISPLAY` too, and GNOME or KDE on Wayland are not X11.
pub fn compositor(var: impl Fn(&str) -> Option<String>, live: impl Fn(&str) -> bool) -> Option<Compositor> {
    if let Some(signature) = var("HYPRLAND_INSTANCE_SIGNATURE").filter(|signature| valid_signature(signature)) {
        let current = var("XDG_RUNTIME_DIR").map(|dir| format!("{dir}/hypr/{signature}/.socket.sock"));
        let legacy = format!("/tmp/hypr/{signature}/.socket.sock");
        if current.iter().chain([&legacy]).any(|socket| live(socket)) {
            return Some(Compositor::Hyprland);
        }
    }
    if var("SWAYSOCK").is_some_and(|socket| live(&socket)) {
        return Some(Compositor::Sway);
    }
    match var("XDG_SESSION_TYPE").as_deref() {
        Some("x11") => Some(Compositor::X11),
        Some(_) => None,
        None if var("DISPLAY").is_some() && var("WAYLAND_DISPLAY").is_none() => Some(Compositor::X11),
        None => None,
    }
}

/// Hyprland's instance signatures are its commit, a timestamp and a random part, joined by `_`; anything else never
/// becomes part of a path.
fn valid_signature(signature: &str) -> bool {
    !signature.is_empty() && signature.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// `swaymsg -t get_tree` lists the windows, then the nearest ancestor's window is focused by its container id.
pub async fn focus_sway(pids: &[u32], session: &Session) -> Result<String, String> {
    let tree = run_with_env("swaymsg", &["-t", "get_tree"], session).await?;
    let id = sway_container_for(&tree, pids).ok_or_else(|| NO_HOST_WINDOW.to_owned())?;
    let command = format!("[con_id={id}] focus");
    let reply = run_with_env("swaymsg", &[&command], session).await?;
    if !sway_succeeded(&reply) {
        return Err(format!("swaymsg did not focus the window: {}", reply.trim()));
    }
    Ok(command)
}

/// `wmctrl -l -p` lists the windows, then the nearest ancestor's window is raised by its id.
pub async fn focus_x11(pids: &[u32], session: &Session) -> Result<String, String> {
    let listing = run_with_env("wmctrl", &["-l", "-p"], session).await?;
    let id = x11_window_for(&listing, pids).ok_or_else(|| NO_HOST_WINDOW.to_owned())?;
    run_with_env("wmctrl", &["-i", "-a", &id], session).await?;
    Ok(id)
}

/// A node of `swaymsg -t get_tree`. Windows carry the pid of their process; containers and outputs don't.
#[derive(Deserialize)]
struct SwayNode {
    id: i64,
    pid: Option<i64>,
    #[serde(default)]
    nodes: Vec<SwayNode>,
    #[serde(default)]
    floating_nodes: Vec<SwayNode>,
}

impl SwayNode {
    /// Every window in tree order, tiled before floating, as (container id, owning pid).
    fn windows(&self, found: &mut Vec<(i64, i64)>) {
        if let Some(pid) = self.pid {
            found.push((self.id, pid));
        }
        for child in self.nodes.iter().chain(&self.floating_nodes) {
            child.windows(found);
        }
    }
}

/// The container of the first window owned by the nearest ancestor that owns one. Container ids are positive
/// numbers; anything else never reaches the command.
fn sway_container_for(tree_json: &str, pids: &[u32]) -> Option<i64> {
    let root: SwayNode = serde_json::from_str(tree_json).ok()?;
    let mut windows = Vec::new();
    root.windows(&mut windows);
    pids.iter().find_map(|&pid| {
        windows
            .iter()
            .find(|&&(id, owner)| owner == i64::from(pid) && id > 0)
            .map(|&(id, _)| id)
    })
}

/// The first window of the nearest ancestor that owns one. `wmctrl -l -p` prints one window per line: id, desktop (`-1`
/// for a sticky window), pid, host, title. Rows whose id or pid don't parse are skipped; the rest is ignored.
fn x11_window_for(listing: &str, pids: &[u32]) -> Option<String> {
    let windows: Vec<(&str, i64)> = listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let id = fields.next().filter(|id| valid_x11_id(id))?;
            fields.next()?;
            let owner = fields.next()?.parse().ok()?;
            Some((id, owner))
        })
        .collect();
    pids.iter().find_map(|&pid| {
        windows
            .iter()
            .find(|&&(_, owner)| owner == i64::from(pid))
            .map(|&(id, _)| id.to_owned())
    })
}

/// X window ids are hex with a `0x` prefix; anything else never reaches `wmctrl`.
fn valid_x11_id(id: &str) -> bool {
    id.strip_prefix("0x")
        .is_some_and(|hex| !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// `swaymsg` answers a command with one `success` flag per command. A focus counts only when every flag is true.
fn sway_succeeded(reply: &str) -> bool {
    #[derive(Deserialize)]
    struct Outcome {
        success: bool,
    }
    serde_json::from_str::<Vec<Outcome>>(reply)
        .is_ok_and(|outcomes| !outcomes.is_empty() && outcomes.iter().all(|outcome| outcome.success))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SWAY_TREE: &str = include_str!("../../tools/fixtures/linux-windows/sway-get-tree.json");

    fn lookup(pairs: &[(&str, &str)], name: &str) -> Option<String> {
        pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
    }

    /// The window system for these variables, with only the sockets in `live` answering.
    fn choose(pairs: &[(&str, &str)], live: &[&str]) -> Option<Compositor> {
        compositor(|name| lookup(pairs, name), |path| live.contains(&path))
    }

    const HYPRLAND: [(&str, &str); 2] = [("HYPRLAND_INSTANCE_SIGNATURE", "abc_123_456"), ("XDG_RUNTIME_DIR", "/run/user/1000")];
    const HYPRLAND_SOCKET: &str = "/run/user/1000/hypr/abc_123_456/.socket.sock";

    fn owned(pairs: &[(&'static str, &str)]) -> Session {
        pairs.iter().map(|&(name, value)| (name, value.to_owned())).collect()
    }

    #[test]
    fn nothing_known_is_no_window_system() {
        assert_eq!(choose(&[], &[]), None);
    }

    #[test]
    fn hyprland_counts_only_while_its_socket_answers() {
        assert_eq!(choose(&HYPRLAND, &[HYPRLAND_SOCKET]), Some(Compositor::Hyprland));
        // A signature an ended session left behind, in systemd's user manager say, is not a Hyprland session.
        assert_eq!(choose(&HYPRLAND, &[]), None);
        assert_eq!(choose(&[HYPRLAND[0], ("XDG_SESSION_TYPE", "x11")], &[]), Some(Compositor::X11));
    }

    #[test]
    fn hyprland_before_0_40_listens_in_tmp() {
        assert_eq!(choose(&HYPRLAND[..1], &["/tmp/hypr/abc_123_456/.socket.sock"]), Some(Compositor::Hyprland));
    }

    #[test]
    fn a_signature_that_could_leave_its_directory_is_ignored() {
        let pairs = [("HYPRLAND_INSTANCE_SIGNATURE", "../../sway"), ("XDG_RUNTIME_DIR", "/run/user/1000")];
        assert_eq!(compositor(|name| lookup(&pairs, name), |_| true), None);
    }

    #[test]
    fn a_live_hyprland_wins_over_every_other_sign() {
        let pairs = [HYPRLAND[0], HYPRLAND[1], ("SWAYSOCK", "/run/sway.sock"), ("XDG_SESSION_TYPE", "x11")];
        assert_eq!(choose(&pairs, &[HYPRLAND_SOCKET, "/run/sway.sock"]), Some(Compositor::Hyprland));
    }

    #[test]
    fn a_live_sway_socket_selects_sway() {
        assert_eq!(choose(&[("SWAYSOCK", "/run/sway.sock"), ("XDG_SESSION_TYPE", "wayland")], &["/run/sway.sock"]), Some(Compositor::Sway));
    }

    #[test]
    fn a_stale_sway_socket_is_ignored() {
        // A socket a dead compositor left behind no longer accepts a connection.
        assert_eq!(choose(&[("SWAYSOCK", "/run/gone.sock"), ("XDG_SESSION_TYPE", "wayland")], &[]), None);
        assert_eq!(choose(&[("SWAYSOCK", "/run/gone.sock"), ("XDG_SESSION_TYPE", "x11")], &[]), Some(Compositor::X11));
    }

    #[test]
    fn an_x11_session_type_selects_x11() {
        assert_eq!(choose(&[("XDG_SESSION_TYPE", "x11"), ("DISPLAY", ":0")], &[]), Some(Compositor::X11));
    }

    #[test]
    fn gnome_or_kde_on_wayland_is_no_supported_window_system() {
        // Xwayland sets DISPLAY too, so the session type, not the display, decides.
        let pairs = [("XDG_SESSION_TYPE", "wayland"), ("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0")];
        assert_eq!(choose(&pairs, &[]), None);
    }

    #[test]
    fn a_bare_display_without_wayland_is_x11_when_no_session_type_is_set() {
        assert_eq!(choose(&[("DISPLAY", ":0")], &[]), Some(Compositor::X11));
        assert_eq!(choose(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0")], &[]), None);
    }

    #[test]
    fn a_session_keeps_only_its_window_system_variables() {
        let environ = "HOME=/home/me\0SWAYSOCK=/run/sway.sock\0DISPLAY=\0XAUTHORITY=$'/tmp/odd\\npath'\0XDG_RUNTIME_DIR=/run/user/1000\0garbage";
        let expected = owned(&[("SWAYSOCK", "/run/sway.sock"), ("XDG_RUNTIME_DIR", "/run/user/1000")]);
        assert_eq!(session(environ.split('\0')), expected);
        // The same parser reads `systemctl --user show-environment`, one variable a line.
        assert_eq!(session("SWAYSOCK=/run/sway.sock\nXDG_RUNTIME_DIR=/run/user/1000\n".lines()), expected);
    }

    #[test]
    fn the_nearest_graphical_process_gives_the_whole_session() {
        let shell = owned(&[("XDG_RUNTIME_DIR", "/run/user/1000")]);
        let terminal = owned(&[("SWAYSOCK", "/run/sway.sock"), ("WAYLAND_DISPLAY", "wayland-1")]);
        let older = owned(&[("HYPRLAND_INSTANCE_SIGNATURE", "abc_123_456"), ("DISPLAY", ":0")]);
        // Unreadable and non-graphical environments are passed over; the first graphical one is taken as it is.
        let found = source_session([None, Some(shell), Some(terminal.clone()), Some(older)]);
        assert_eq!(found, Some(terminal));
        assert_eq!(source_session([None, Some(Session::new())]), None);
    }

    #[test]
    fn the_nearest_ancestor_with_a_window_is_focused_by_container() {
        assert_eq!(sway_container_for(SWAY_TREE, &[4242, 4000]), Some(17));
        assert_eq!(sway_container_for(SWAY_TREE, &[4000, 4242]), Some(9));
    }

    #[test]
    fn a_floating_window_is_found_too() {
        assert_eq!(sway_container_for(SWAY_TREE, &[5151]), Some(21));
    }

    #[test]
    fn an_absent_pid_or_a_missing_window_finds_nothing() {
        assert_eq!(sway_container_for(SWAY_TREE, &[9000]), None);
        assert_eq!(sway_container_for(r#"{"id": 1, "nodes": [], "floating_nodes": []}"#, &[4242]), None);
    }

    #[test]
    fn the_first_window_in_tree_order_wins_for_one_process() {
        let tree = r#"{"id": 1, "nodes": [
            {"id": 30, "pid": 7, "nodes": [], "floating_nodes": []},
            {"id": 31, "pid": 7, "nodes": [], "floating_nodes": []}
        ], "floating_nodes": []}"#;
        assert_eq!(sway_container_for(tree, &[7]), Some(30));
    }

    #[test]
    fn invalid_container_ids_never_reach_the_command() {
        let tree = r#"{"id": 1, "nodes": [
            {"id": 0, "pid": 7, "nodes": [], "floating_nodes": []},
            {"id": 31, "pid": 7, "nodes": [], "floating_nodes": []}
        ], "floating_nodes": []}"#;
        assert_eq!(sway_container_for(tree, &[7]), Some(31));
        assert_eq!(sway_container_for(r#"{"id": -4, "pid": 7}"#, &[7]), None);
    }

    #[test]
    fn malformed_tree_output_finds_nothing() {
        assert_eq!(sway_container_for("", &[4242]), None);
        assert_eq!(sway_container_for("not json", &[4242]), None);
        assert_eq!(sway_container_for(r#"{"id": "one", "nodes": []}"#, &[4242]), None);
    }

    #[test]
    fn a_focus_counts_only_when_every_command_succeeded() {
        assert!(sway_succeeded(r#"[{"success": true}]"#));
        assert!(!sway_succeeded(r#"[{"success": false, "error": "No node with id 17"}]"#));
        assert!(!sway_succeeded(r#"[{"success": true}, {"success": false}]"#));
        assert!(!sway_succeeded("[]"));
        assert!(!sway_succeeded(""));
        assert!(!sway_succeeded("not json"));
    }

    const WMCTRL_LIST: &str = include_str!("../../tools/fixtures/linux-windows/wmctrl-list.txt");

    #[test]
    fn the_nearest_ancestor_window_is_raised_by_id() {
        assert_eq!(x11_window_for(WMCTRL_LIST, &[4242, 4000]).as_deref(), Some("0x01400007"));
        assert_eq!(x11_window_for(WMCTRL_LIST, &[4000, 4242]).as_deref(), Some("0x01200003"));
    }

    #[test]
    fn a_sticky_window_is_found_too() {
        assert_eq!(x11_window_for(WMCTRL_LIST, &[5151]).as_deref(), Some("0x01600009"));
    }

    #[test]
    fn an_absent_pid_finds_nothing() {
        assert_eq!(x11_window_for(WMCTRL_LIST, &[9000]), None);
        assert_eq!(x11_window_for("", &[4242]), None);
    }

    #[test]
    fn the_first_listed_window_wins_for_one_process() {
        let listing = "0x00a00001 0 7 host one\n0x00a00002 0 7 host two\n";
        assert_eq!(x11_window_for(listing, &[7]).as_deref(), Some("0x00a00001"));
    }

    #[test]
    fn malformed_and_invalid_rows_are_skipped() {
        let listing = "garbage\n0x0140000g 0 4242 host bad id\n0x01400007 0 host no pid\n0x01400008 0 4242 host ok\n";
        assert_eq!(x11_window_for(listing, &[4242]).as_deref(), Some("0x01400008"));
        assert_eq!(x11_window_for("garbage\n\n", &[4242]), None);
    }

    #[tokio::test]
    async fn a_missing_helper_is_named_in_the_error() {
        let error = run_with_env("vibebuddy-no-such-helper", &[], &Session::new()).await.unwrap_err();
        assert!(error.starts_with("cannot run vibebuddy-no-such-helper"), "{error}");
    }
}
