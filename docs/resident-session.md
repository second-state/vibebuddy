# Resident session

The resident session is the one long-running Codex thread every voice command goes to ([ADR-0017](adr/0017-voice-commands-go-to-a-resident-session-that-speaks-as-the-buddy.md); see "Voice commands" in [`CONTEXT.md`](../CONTEXT.md)). A voice command reaches it spoken to a box linked over USB or the local network (below), or typed with `vibebuddy say`.

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
- **Tools.** `speak({text})` is the spoken reply: the daemon hands it on the moment it is called (`Resident::say_streaming`), to the box as `voice.reply`, or collected for `POST /v1/resident/say`. `list_activities({})` returns the Activities the aggregator already holds, as JSON: agent, task-card title, project, `working`/`needs_input`, host, seconds in that state and since the last event. There are no tools that run commands or write files.
- **Declining.** Every approval or question the session raises is declined: command and file-change approvals (`decline`, or `denied` on the legacy methods), permission requests (nothing granted), MCP elicitations (`decline`) and user-input requests (no answers). `currentTime/read` is answered; anything else gets a JSON-RPC "method not found".
- **Turns.** `turn/start` with the text, then the daemon answers tool calls until `turn/completed` for that turn. One turn at a time: a command that arrives during another waits for it, in order, rather than being rejected, since two quick sentences are both meant to be answered. A turn that takes over two minutes is interrupted (`turn/interrupt`).
- **Persona.** `instructions()` in `resident.rs` takes the Character's persona; until Characters carry one, it is `None` and the buddy speaks plainly.

## From the box

The box side of push-to-talk is in [`protocol.md`](protocol.md), "Voice commands (experimental)"; `daemon/src/voice.rs` is the computer side and `daemon/src/speech.rs` the recognizers.

- **Offered.** Voice is on when the resident session is on and a speech recognizer is configured. The daemon then sends `voice.available` `on: true` every time the box reports its build (after `device.hello` on each new link, and after a reboot). Never over the Relay, where the box will talk to the voice service itself, and never over the UART bridge, which is too slow for the audio and gets none from the box (use the box's native USB port); the log says which kept voice off. Nothing switches either condition at run time yet, so `on: false` is never sent.
- **Recording.** `VOICE DATA` lines are decoded as they arrive, each from the IMA ADPCM state it carries (the same decoder as `firmware-rs/core/src/adpcm.rs`, checked against it in the tests). On `VOICE END` the lines are put in seq order; a lost line becomes silence as long as a line, audio lost at the end is filled up to the sample count `END` gives, and the missing seqs are logged. A turn's audio is kept only in memory and only until it is recognised; `VOICE DATA` lines stay out of the log.
- **A turn.** The audio goes to the recognizer as a 24 kHz 16-bit mono WAV file; the text goes to the box as `voice.heard` and to the resident session as a voice command; each `speak` goes to the box as `voice.reply` as soon as it is called, and its audio follows as `voice.audio` (see "Speech synthesis" below); `voice.done` follows `turn/completed` and the last reply's audio `end`. Text is cut at a character, with an ellipsis, to fit the 1024-byte line. Errors: `not_heard` (no audio arrived, or the recognizer heard nothing), `unavailable` (voice is off, recognition failed, or the resident session failed, with the reason as `message`), `busy` (a turn started while another is still being answered: sent right after its `VOICE START`, so the box stops recording, and the rest of that turn is dropped; unlike `vibebuddy say`, a voice turn doesn't wait). A typed command and a voice turn still queue behind each other in the resident session.
- **Timing.** A 5 s recording, checked end to end on 2026-10-13 (recognition by Workers AI, codex-cli 0.162.0, app-server started cold): `voice.heard` 2.2 s after `VOICE END`, the spoken reply at 10.1 s, `voice.done` at 14.0 s.

The end-to-end check is an ignored test, `e2e_a_real_recording_through_the_real_recognizer_and_resident_session` in `voice.rs`: it turns the left channel of a WAV file into `VOICE` lines with firmware-core's own voice command code (as do the other voice tests, so they hold the daemon to what the box writes) and feeds them through the real recognizer and the real resident session, in a temporary state directory. Its doc comment says how to run it; give it a `VIBEBUDDY_CODEX` that adds `-c features.hooks=false`, or the turn's hooks reach the daemon on 7331 and show up on the box.

### Speech recognition

Behind the `Recognizer` trait, so the planned voice service in `vibebuddy-cloud` (called with the account session) can take over. For now there are two backends, picked at start:

| Backend | Settings | Used |
| --- | --- | --- |
| The user's own recognizer: any OpenAI-compatible `POST {base}/audio/transcriptions` | `VIBEBUDDY_STT_BASE_URL`, `VIBEBUDDY_STT_API_KEY` (optional), `VIBEBUDDY_STT_MODEL` (default `whisper-1`) | When set; it wins over Cloudflare |
| Cloudflare Workers AI, `@cf/openai/whisper-large-v3-turbo` | `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_AI_TOKEN` | Otherwise; for development |

`VIBEBUDDY_STT=cloudflare|openai|off` forces the choice. `VIBEBUDDY_STT_LANGUAGE` (`zh`, `en`, …) tells the recognizer the language; unset, it detects it, and the log names what it detected.

Each setting is taken from the environment, else from `speech.json` in the config directory (`{"base_url","api_key","model","language"}`, the user's own recognizer), else from a `KEY=VALUE` file named by `VIBEBUDDY_STT_ENV_FILE` (development credentials, so they stay out of the daemon's environment and out of the repository). All are read when the daemon starts. `speech.json` is a file of its own, not part of `config.json`, because `PUT /v1/config` replaces the whole config and an App that doesn't know the field would wipe the key; whoever writes it (an App setting, later) writes it with mode 0600. Credentials are never logged, and errors name only the recognizer's host.

### Speech synthesis

Each spoken reply is also synthesized and streamed to the box (`daemon/src/synthesis.rs`, `Speaker` in `voice.rs`), behind the `Synthesizer` trait so the backend can change. There is one backend, the voice service in `vibebuddy-cloud`: `POST {base}/v0/voice/speak` with `{"text","voice"}` (text 1 to 1000 characters, `voice` an Edge voice name), answering a chunked `audio/mpeg` stream (MPEG-2 Layer III, 24 kHz mono, 48 kbps) that starts as soon as there is audio. v0 takes no credentials; the account session's header goes in `VoiceService::speak` when it does.

| Setting | Effect |
| --- | --- |
| `VIBEBUDDY_TTS_BASE_URL` unset | Staging (`https://api-staging.korekore.ai`) in every build but a release, as for the Relay; a release has no voice service until production's is deployed, so its replies stay text |
| `VIBEBUDDY_TTS_BASE_URL=<url>` | That service |
| `VIBEBUDDY_TTS_BASE_URL=off` | No synthesis: replies stay text |

- **Order.** Replies are synthesized one after another, in the order they were said, on a task of their own: a reply's `voice.reply` goes out the moment it is said and never waits for an earlier reply's audio, so the text of reply 1 may come between reply 0's audio pieces. Each reply's audio is forwarded as the bytes arrive, cut into pieces of at most 576 bytes (768 base64 characters), `seq` from 0, then `end`. `voice.done` waits for the last reply's `end`, so the turn is over only once everything has been sent; it is still the last message of the turn.
- **Failures.** A status other than 200, or a stream cut before its end (the service closes the connection when it fails partway), ends that reply's audio at once with `"error":"synthesis"`, keeping what was sent; the box keeps the text. The next reply is still tried. When the resident session fails the turn, the reply being synthesized stops at its next piece, the rest are dropped, and `voice.error` follows.
- **Long replies.** Over 1000 characters (counted in UTF-16, as the service counts), a reply is split at the end of a sentence, else at a comma or space; the pieces are requested one after another and stream as one reply, since MP3 frames stand alone.
- **Voices.** For development, `CHARACTER_VOICES` in `synthesis.rs` gives each Character an Edge voice roughly matching its persona (Xiaohe `zh-TW-HsiaoChenNeural`, Ada `en-GB-SoniaNeural`, Luna `en-US-AvaNeural`, Mei `en-US-AriaNeural`, Hank `en-US-ChristopherNeural`, Kai `en-US-AndrewNeural`, Dave `en-GB-RyanNeural`, the robot and no pack `en-US-JennyNeural`). The Character is the one the box reports (`VOICES`), else the one last written to it. A Character speaks one language but the resident session answers in the user's, so a reply with Chinese in it gets `zh-CN-XiaoxiaoNeural` when the Character's voice isn't Chinese. Production voices are still to be decided; that table is the one place to change.
- **Links.** Only where voice is offered, native USB and the local network: nothing goes over the Relay or the UART bridge.
- **Timing.** Checked on 2026-10-13 against staging, from this computer through the daemon's own client (a new TLS connection each time): a Chinese sentence, first audio after 1.3 to 1.4 s and its `end` after 1.5 to 1.6 s for 4.5 s of speech; an English one, 0.5 to 1.8 s and 0.6 to 2.0 s for 3.8 s. `e2e_spoken_replies_through_the_real_voice_service` in `voice.rs` (ignored) repeats it and saves the reassembled MP3s.

## Never an Activity

The user's Codex hooks fire for the resident thread too: its `UserPromptSubmit`, `PostToolUse` and `Stop` reach `/v1/codex-hooks` like any session's. The Codex adapter drops a hook whose session or thread is the resident thread, or whose `cwd` is inside the resident directory (which also covers the thread opened in the Codex App), before anything is recorded: no card, announcement, stats, CI watch or K2 target. This works even with the resident session switched off.

## Versions

app-server and `dynamicTools` are experimental. This was built and checked against codex-cli 0.162.0 (`TESTED_CODEX_VERSION`); another version logs a warning and is still tried. The version last used is written to `resident.json`.

## Privacy

`list_activities` sends the task-card titles and project names the daemon holds, with each Activity's agent, state, host app and ages, to the model provider the user's Codex uses, as part of the resident thread. Until now these stayed on the computer and the box. Prompts and replies are not sent: the daemon never holds them.

A voice command's audio leaves the computer for the speech recognizer: in development that is Cloudflare Workers AI, under the developer's own account; with the user's own recognizer, only that endpoint (and its key goes nowhere else). The recognised text then goes to the user's Codex model provider as the voice command. The daemon keeps neither the audio nor the text once the turn is over, and logs neither.

Each spoken reply's text, which may name what the Agents are doing, goes to the voice service to be synthesized (in development, staging), with the voice name; the service passes it on to the speech engine behind it (Edge voices for now). The audio it returns goes only to the box. The daemon logs the voice, timings and byte counts, never the text.
