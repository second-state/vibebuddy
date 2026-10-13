# Resident session

The resident session is the one long-running Codex thread every voice command goes to ([ADR-0017](adr/0017-voice-commands-go-to-a-resident-session-that-speaks-as-the-buddy.md); see "Voice commands" in [`CONTEXT.md`](../CONTEXT.md)). This is the first, command-line-only cut: the box can't hear yet, so a voice command is typed.

It is experimental and off by default. Start `vibebuddyd` with `VIBEBUDDY_RESIDENT_SESSION=1` to turn it on. There is no App UI for it, on the Mac or on Linux.

## Using it

```sh
vibebuddy say "What are my agents doing?"   # prints the spoken reply(ies); the time taken goes to stderr
vibebuddy resident                          # prints codex://threads/<id>, to open the thread in the Codex App
```

`vibebuddy` is a second binary of the daemon package (`daemon/src/bin/vibebuddy.rs`). Like every client, it only talks to the daemon's HTTP API, at `VIBEBUDDY_BIND` (default `127.0.0.1:7331`):

| Endpoint | Does |
| --- | --- |
| `POST /v1/resident/say` `{"text"}` | Runs one turn; answers `{"thread_id","replies":[...],"elapsed_ms"}`. 404 when off, 503 when Codex is missing or not logged in, 504 after two minutes. |
| `GET /v1/resident` | `{"enabled","thread_id","url"}` |

## How it works

`daemon/src/app_server.rs` is a newline-delimited JSON-RPC client for `codex app-server` on stdio; `daemon/src/resident.rs` is the session on top of it.

- **Starting.** On the first voice command, and on the next one after the app-server dies, the daemon runs the `codex` it finds (`VIBEBUDDY_CODEX`, else PATH, else `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, since launchd's PATH is bare). It sends `initialize` (`experimentalApi: true`) and `initialized`, then `account/read`: with no account it stops with "not logged in". It never starts a login.
- **One thread for good.** `thread/start` once, with `cwd` set to `resident/` in the daemon's state directory, `sandbox: "read-only"`, `approvalPolicy: "on-request"`, the developer instructions and the dynamic tools. The thread id is kept in `resident.json` beside it, and every later start sends `thread/resume` with the same settings (the tools survive a resume; the instructions are sent again). If the thread can't be resumed, a new one is started and saved, and the log names both the old and the new thread id.
- **Only our tools.** The app-server is started with the user's own Codex config, minus everything that gives a thread tools of its own: `-c features.<name>=false` for plugins, apps (connectors), the shell and exec tools, sub-agents, goals, memories, image generation and viewing, the browser and computer use, sleep, tool suggestions and skill search, plus `web_search="disabled"`. An override can't empty `mcp_servers` (it merges), so the daemon first asks `codex mcp list --json`, under the same overrides, which servers would be on, and adds `-c mcp_servers.<name>.enabled=false` for each. Overrides are `-c` rather than `--disable`, which refuses a feature name another Codex version doesn't know. The user's global `AGENTS.md` and hooks stay: the hooks are filtered (below). Checked on 0.162.0: `mcpServerStatus/list` goes from seven servers (one with 365 app tools) to two, both off with no tools, and the thread's tool list is `speak`, `list_activities` and what Codex always adds and has no switch for: `apply_patch` (refused by the read-only sandbox, its approval declined), a clock, and the MCP resource readers (no server is on to read from).
- **Tools.** `speak({text})` is the spoken reply: the daemon collects it and hands it back to the caller (later it goes to the box). `list_activities({})` returns the Activities the aggregator already holds, as JSON: agent, task-card title, project, `working`/`needs_input`, host, seconds in that state and since the last event. There are no tools that run commands or write files.
- **Declining.** Every approval or question the session raises is declined: command and file-change approvals (`decline`, or `denied` on the legacy methods), permission requests (nothing granted), MCP elicitations (`decline`) and user-input requests (no answers). `currentTime/read` is answered; anything else gets a JSON-RPC "method not found".
- **Turns.** `turn/start` with the text, then the daemon answers tool calls until `turn/completed` for that turn. One turn at a time: a command that arrives during another waits for it, in order, rather than being rejected, since two quick sentences are both meant to be answered. A turn that takes over two minutes is interrupted (`turn/interrupt`).
- **Persona.** `instructions()` in `resident.rs` takes the Character's persona; until Characters carry one, it is `None` and the buddy speaks plainly.

## Never an Activity

The user's Codex hooks fire for the resident thread too: its `UserPromptSubmit`, `PostToolUse` and `Stop` reach `/v1/codex-hooks` like any session's. The Codex adapter drops a hook whose session or thread is the resident thread, or whose `cwd` is inside the resident directory (which also covers the thread opened in the Codex App), before anything is recorded: no card, announcement, stats, CI watch or K2 target. This works even with the resident session switched off.

## Versions

app-server and `dynamicTools` are experimental. This was built and checked against codex-cli 0.162.0 (`TESTED_CODEX_VERSION`); another version logs a warning and is still tried. The version last used is written to `resident.json`.

## Privacy

`list_activities` sends the task-card titles and project names the daemon holds, with each Activity's agent, state, host app and ages, to the model provider the user's Codex uses, as part of the resident thread. Until now these stayed on the computer and the box. Prompts and replies are not sent: the daemon never holds them.

## Known gaps

- A reply is returned when the turn completes, which in testing was 4 to 6 seconds after `speak` was called. The box should play a spoken reply the moment `speak` arrives, not at `turn/completed`.
