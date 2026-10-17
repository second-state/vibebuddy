//! Brings the window of an agent's session forward on Linux, on the window system the session runs. Hyprland is the
//! default, as it always was; Sway and X11 are chosen only when the session says so. Each backend lists the windows
//! with their pids and focuses the one owned by the nearest of the agent's ancestors.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::source_opener::run_with_env;

pub const NO_HOST_WINDOW: &str = "session has no host window (SSH, tmux or a background process)";

/// The variables that say which window system the session runs and how its helpers reach it.
const SESSION_VARIABLES: [&str; 6] = ["HYPRLAND_INSTANCE_SIGNATURE", "SWAYSOCK", "XDG_SESSION_TYPE", "DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY"];

/// Those variables as the login session has them, by name; also the environment the backend's helper runs with.
pub type Session = BTreeMap<&'static str, String>;

/// The session's variables, from `systemctl --user show-environment` (`manager`, one `NAME=value` a line) where it
/// has them, otherwise from `process`, the daemon's own environment. systemd starts the daemon at its own start,
/// which can be before the desktop imports its variables; the manager has the ones the desktop imported since.
/// A value systemd had to quote (`$'…'`) is left to the process: none of these normally needs it.
pub fn session(manager: &str, process: impl Fn(&str) -> Option<String>) -> Session {
    let imported: BTreeMap<&str, &str> = manager.lines().filter_map(|line| line.split_once('=')).collect();
    SESSION_VARIABLES
        .into_iter()
        .filter_map(|name| {
            let value = imported
                .get(name)
                .filter(|value| !value.is_empty() && !value.starts_with("$'"))
                .map(|value| (*value).to_owned())
                .or_else(|| process(name))?;
            Some((name, value))
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Compositor {
    Hyprland,
    Sway,
    X11,
}

/// Which window system the session runs on. `var` reads an environment variable, `socket_exists` tells whether a path
/// is there. Both are parameters, so the choice is testable without the real environment.
///
/// Hyprland is answered first, as before. A Sway socket counts only while it exists, so one a dead compositor left
/// behind doesn't win. Then `XDG_SESSION_TYPE` decides when it is set: `x11` is X11, anything else stays on Hyprland.
/// Without a session type, a `DISPLAY` with no `WAYLAND_DISPLAY` is X11.
pub fn compositor(var: impl Fn(&str) -> Option<String>, socket_exists: impl Fn(&str) -> bool) -> Compositor {
    if var("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Compositor::Hyprland;
    }
    if var("SWAYSOCK").is_some_and(|socket| socket_exists(&socket)) {
        return Compositor::Sway;
    }
    match var("XDG_SESSION_TYPE").as_deref() {
        Some("x11") => Compositor::X11,
        Some(_) => Compositor::Hyprland,
        None if var("DISPLAY").is_some() && var("WAYLAND_DISPLAY").is_none() => Compositor::X11,
        None => Compositor::Hyprland,
    }
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

    fn choose(pairs: &[(&str, &str)], sockets: &[&str]) -> Compositor {
        compositor(|name| lookup(pairs, name), |path| sockets.contains(&path))
    }

    #[test]
    fn hyprland_is_chosen_when_nothing_else_is_known() {
        assert_eq!(choose(&[], &[]), Compositor::Hyprland);
    }

    #[test]
    fn a_hyprland_session_wins_over_every_other_sign() {
        let pairs = [
            ("HYPRLAND_INSTANCE_SIGNATURE", "abc"),
            ("SWAYSOCK", "/run/sway.sock"),
            ("XDG_SESSION_TYPE", "x11"),
        ];
        assert_eq!(choose(&pairs, &["/run/sway.sock"]), Compositor::Hyprland);
    }

    #[test]
    fn a_live_sway_socket_selects_sway() {
        assert_eq!(choose(&[("SWAYSOCK", "/run/sway.sock"), ("XDG_SESSION_TYPE", "wayland")], &["/run/sway.sock"]), Compositor::Sway);
    }

    #[test]
    fn a_stale_sway_socket_is_ignored() {
        // A socket path a dead compositor left behind doesn't exist any more.
        assert_eq!(choose(&[("SWAYSOCK", "/run/gone.sock"), ("XDG_SESSION_TYPE", "wayland")], &[]), Compositor::Hyprland);
        assert_eq!(choose(&[("SWAYSOCK", "/run/gone.sock"), ("XDG_SESSION_TYPE", "x11")], &[]), Compositor::X11);
    }

    #[test]
    fn an_x11_session_type_selects_x11() {
        assert_eq!(choose(&[("XDG_SESSION_TYPE", "x11"), ("DISPLAY", ":0")], &[]), Compositor::X11);
    }

    #[test]
    fn a_wayland_session_with_xwayland_stays_on_hyprland() {
        // Xwayland sets DISPLAY too, so the session type, not the display, decides.
        let pairs = [("XDG_SESSION_TYPE", "wayland"), ("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0")];
        assert_eq!(choose(&pairs, &[]), Compositor::Hyprland);
    }

    #[test]
    fn a_bare_display_without_wayland_is_x11_when_no_session_type_is_set() {
        assert_eq!(choose(&[("DISPLAY", ":0")], &[]), Compositor::X11);
        assert_eq!(choose(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0")], &[]), Compositor::Hyprland);
    }

    #[test]
    fn the_session_comes_from_systemd_first_then_the_daemon() {
        // systemd started the daemon before the desktop imported its variables, so only the manager has Sway's socket.
        let manager = "HOME=/home/me\nSWAYSOCK=/run/sway.sock\nXDG_SESSION_TYPE=wayland\nDISPLAY=\nXAUTHORITY=$'/tmp/odd\\npath'\n";
        let process = [("XDG_SESSION_TYPE", "tty"), ("DISPLAY", ":0"), ("XAUTHORITY", "/tmp/xauth"), ("HOME", "/root")];
        let found = session(manager, |name| lookup(&process, name));
        let expected = [("DISPLAY", ":0"), ("SWAYSOCK", "/run/sway.sock"), ("XAUTHORITY", "/tmp/xauth"), ("XDG_SESSION_TYPE", "wayland")];
        assert_eq!(found, Session::from_iter(expected.map(|(name, value)| (name, value.to_owned()))));
    }

    #[test]
    fn without_systemd_the_session_is_the_daemons_own() {
        assert_eq!(session("", |name| lookup(&[("SWAYSOCK", "/run/sway.sock")], name)).get("SWAYSOCK").map(String::as_str), Some("/run/sway.sock"));
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
