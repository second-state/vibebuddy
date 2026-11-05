# Kimi Code CLI Adapter

Kimi Code CLI connects through the shared `vibebuddy-hook kimi` binary and
`POST /v1/agent-hooks`. It uses the same activity tracker and host navigation as
the other CLI Agents. Its task-card prefix is `KM:`. Both Apps use the generic
Agent icon for Kimi.

## Setup

Run Kimi once, then open Settings → Agents → Kimi Code CLI → Connect. Review
the target path and hook changes before writing. Repair restores the seven
subscriptions and replaces commands pointing at an older VibeBuddy binary path.
Remove deletes VibeBuddy's hook entries while preserving the shared config.
Start a new Kimi session after changing hooks.

On Linux the package installer also connects Kimi when its user directory exists.
For manual management, use `vibebuddy-hook plan install kimi`,
`vibebuddy-hook install kimi`, and `vibebuddy-hook uninstall kimi`.

Configuration defaults to `~/.kimi-code/config.toml`. An absolute
`KIMI_CODE_HOME` inherited by the App or hook selects another directory. A GUI
launched from the desktop may not inherit shell-only exports; start VibeBuddy
with the same environment or configure the graphical login environment. Always
check the displayed path. The daemon's environment does not select this path.

## Events and limits

| Kimi hook | VibeBuddy event |
| --- | --- |
| UserPromptSubmit, PostToolUse | working |
| PermissionRequest | needs input |
| Stop | done |
| StopFailure, Interrupt | stopped |
| SessionEnd | session end |

Session starts, subagent events and background notifications are not subscribed.
A plain conversational question at Stop is announced as done because this
adapter does not read replies or transcripts. A denied or failed tool can lack
PostToolUse; the next Stop, StopFailure, Interrupt or user prompt updates the
card. PermissionResult is not currently subscribed. Overlapping or very short
events have not been checked against a live Kimi.

K2 uses the host terminal/editor and tmux metadata. Exact-session navigation in
the Kimi desktop App is not provided. On Linux, the normal Hyprland, Sway, X11,
Wayland and SSH limitations described in the README apply.

## Privacy and config safety

Only `agent`, `session_id`, normalized `event`, `cwd`, and locally detected host
metadata leave the hook. Prompts, replies, tool inputs/outputs, errors, titles,
and transcript paths are not forwarded. HTTP delivery has short timeouts;
invalid input and an unreachable daemon return success without output.

TOML editing preserves unrelated settings and hooks. Writes retain a `.bak`,
create a private temporary file and atomically replace the config, preserving
existing permissions. The installer refuses a config changed between its read
and write. Previews omit other configuration values, which may include keys.

## Validation status

The generated config was accepted by Kimi Code CLI **2.4.1** and **2.5.0** using
`kimi doctor`. These are the tested versions, not a claim of support for all Kimi
releases, and Kimi updates itself, so the installed version can move on its own.
Hooks may change between releases; recheck against the installed version.

The Rust tests cover normalized events, the allowlisted payload (with a marker in
every sensitive field), rejected input, TOML preservation (every other setting
unchanged, not just a comment), repair, missing configs, shell quoting,
permissions, the private backup, and refusal of a config changed before or after
the new text is staged. The daemon's Rust test posts the hook's Kimi payloads
through its real HTTP router and checks the `working`, `needs_input` and `working`
states as each one lands, one announcement per turn, and cleanup at session end.

Kimi-backed checks run by hand; CI does not run them, because the Kimi CLI is
installed from a self-updating CDN and the repository pins no installer. All of
them pass on 2.4.1 and 2.5.0. Each uses the fixed hook address, port 7331, and
needs it free. The temporary directory must not sit under a `.git`: the daemon
names a project after its Git root, so a stray `/tmp/.git` fails the daemon smoke's
project check. Set `TMPDIR` elsewhere if you have one.

- `python3 hook/tests/kimi_smoke.py <hook-binary>` needs no Kimi and runs in CI.
  It checks one POST per mapped event with exactly the allowed fields, that
  ignored and malformed input posts nothing, and that a missing or silent daemon
  returns within two seconds.
- `python3 hook/tests/kimi_cli_smoke.py <hook-binary> <kimi-binary>` runs Kimi
  in print and ACP modes against a scripted local model with dummy
  credentials. Each scenario must produce its exact event sequence for one
  session; the tool scenarios must receive a successful `pwd` result naming the
  project; the reported session ID must match the ACP session or the session
  directory Kimi wrote; and the installer must only add hook tables.
- `python3 hook/tests/kimi_daemon_smoke.py <hook-binary> <vibebuddyd> <kimi-binary>`
  runs the same kind of turns through a real `vibebuddyd` and reads its `/v1/status`:
  each finished turn is counted once, `needs_input` and `working` are shown while
  they hold, and failed and cancelled turns are not announced. The daemon's box
  search is pinned to a missing port, so it cannot pair with a box on the cable.

What these runs do not establish:

- Claude Code and Codex post to port 7331 too. The CLI and hook smokes count only
  Kimi's posts. The daemon smoke refuses to run if another agent reports during it.
- Neither version sent `SessionEnd` in these runs, including when ACP was terminated.
  Session-end cleanup is covered by the tracker and HTTP tests only, and a graceful
  ACP shutdown was not tested.
- Print mode did not emit `PermissionRequest`: an `ask` rule ran Bash without a
  prompt. Approval is exercised only through ACP.
- `stopped` comes from either `Interrupt` or `StopFailure`; these runs do not
  show which one fired.
- The installer's last check and the final rename are still separated by a few
  microseconds. POSIX offers no compare-and-swap for files.

Not checked yet: a session with a real model, graphical K2 focus, clicking through
Connect, Repair and Remove in either App, and Kimi on macOS. CI covers the Mac
App's `SelfTest`, not its interface. The scripted endpoint checks actual runtime
integration; it does not measure external-model quality or graphical behavior.
