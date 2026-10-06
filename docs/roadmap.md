# Vibe Buddy roadmap

## Stage gates

### Stage 0 — Hardware Probe (done, 2026-09-14)

Exit criteria:

- The current development host is confirmed to be a Mac Studio.
- Development tool paths and versions are recorded, with missing items and install plans spelled out.
- USB, IOKit, and serial enumeration of the ESP32S3-BOX are saved before and after plugging it in, and diffed.
- VID/PID, device name, serial node, USB type, flashing path, and runtime path are confirmed, or anything not yet provable is explicitly marked as pending verification.
- The PCB model/revision is identified precisely, and matching official schematics, BSP, and examples are researched; for any material that can't be obtained, the evidence boundary must be recorded, and the peripheral stages that depend on it stay locked.

Stage 0 does not pass until the minimal USB/flashing path is confirmed.

Hardware findings: the ATK-DNESP32S3-BOX V1.1 enumerates via `303A:1001` native USB Serial/JTAG; ROM probing, flashing, and runtime communication have all been done over the same `USB-SLAVE` connection. The vendor schematic/BSP for the old BOX V1.1 has still not been obtained, so peripheral GPIOs for Stages 3–5 must not be guessed from similar boards.

### Stage 1 — Serial Hello (done, 2026-09-14)

Implements only line-by-line serial reading and JSON parsing, no LCD/audio. Acceptance must be the real hardware path Mac → USB → ESP32 → JSON parse; local simulation or merely compiling is not a substitute.

Hardware findings: ESP-IDF v5.5.3 built and flashed successfully; after the Mac sent `{"version":1,"event":"task.done","title":"Hello"}`, the ESP32 actually returned `EVENT task.done` and `TITLE Hello`. The stock 16 MB flash was backed up locally with restricted permissions before writing.

### Stage 2 — `vibebuddyd` (done, 2026-09-14)

Implements `POST /v1/events`, a minimal Transport abstraction, `SerialTransport`, and reconnect on disconnect; hardware acceptance is done with HTTP requests.

Hardware findings: `vibebuddyd` auto-discovers `/dev/cu.usbmodem8401` by `303A:1001`; HTTP events reach the ESP32 and produce the corresponding diagnostic output. After physically unplugging `USB-SLAVE`, `Device not configured` was logged; after plugging it back in it reconnected automatically, and a `task.done` event after reconnecting reached the device successfully.

### Stage 3 — LCD (done, 2026-09-14)

After confirming the 320×240 ST7789 i80 and XL9555 backlight control on the hardware, implement the buddy's Ready, Working, Input Required, Done, and Failed animations. The Mac-side multi-task snapshot can draw up to 3 cards, newest on top.

### Stage 4 — Audio (done, 2026-09-14)

The ES8311 was detected on the hardware, and the I2S and speaker-enable chain confirmed. Needs input, done, and failed use short Chinese voice lines built into the firmware as 24 kHz PCM; working and idle stay silent. The user has confirmed by ear hearing "需要你确认" ("I need you to confirm").

### Stage 5 — Buttons (K2 done, 2026-09-14)

K2's "open the current source" button and the Mac activation path have passed hardware acceptance: a probe confirmed K2 is XL9555 P0.3, active-low (`P0: 0xFF → 0xF7 → 0xFF`); the device reports a single press over NDJSON, and after the user presses it `vibebuddyd` brings up the target app. Precise routing was fixed in two places: Codex sub-agents map to their parent thread; Claude Code no longer focuses Ghostty, but opens the corresponding Code session in the Claude App. On 2026-09-15 a targeting bug in this was fixed: CLI `session_id` to desktop session is not one-to-one, and the old `claude://resume` opened a shadow session with stale content; it now resolves the desktop session id first and then jumps with `claude://code/continue`. On 2026-09-22 the destination was changed to dispatch by where the session runs, and a short-press hardware acceptance was completed: a Code session in the Claude App jumps to `claude://code/continue?session=local_44d42f48…` (the desktop session id is taken from `CLAUDE_CODE_HOST_SESSION_ID`, no longer guessed on disk by cwd); the claude CLI and codex in Ghostty both land on `com.mitchellh.ghostty`; codex running inside the Claude App lands on `com.anthropic.claudefordesktop`. Terminal sessions are no longer imported as copies into the App. Sessions with no host (SSH, background processes) skip to the next candidate; this branch has only unit tests and hook end-to-end verification, with no hardware scenario.

On 2026-09-15 a second scene was added: Pomodoro (25-minute focus, 5-minute break, dial modeled on Focus To-Do). Each of the three buttons does one thing: K0 short press starts/pauses/resumes and long press abandons, K1 switches scenes, and K2 still opens the source; the end of a phase plays a chime plus voice line and switches to Pomodoro automatically, and the next phase waits for the user to press K0. The state machine has host tests and the screen has a host preview. The same day, after flashing over the UART bridge, all three buttons were confirmed working on the hardware: K0 is GPIO0, K1 was pinned down by the candidate-bit probe as XL9555 P0.4, and K2 still opens the source; scene switching, start, pause, and resume all produce device reports. The first hardware run also revealed that quick light taps were being dropped by 100 ms sampling plus two-sample-agreement debouncing; this was changed to 20 ms sampling that takes effect on the first flip. The chime, voice line, and automatic scene switch at the end of focus and the end of break still await one full 25-minute hardware acceptance run. Design in [`pomodoro.md`](pomodoro.md).

The same day a third mode was added: Leisure. After 5 minutes idle on duty it enters the Bored tier and randomly performs seven skits; at 30 minutes it enters the Sleepy tier, dims, and sleeps; between 23:00 and 07:00 at night, after 90 minutes of sleep the backlight turns off; any agent activity, any button press, or a link drop brings it straight back to On duty; a Pomodoro left ready to start for five minutes or paused for half an hour is treated as the person having left. The heartbeat gained the local hour. The director has host tests and the skits have GIF previews. Hardware acceptance used firmware time-compressed 60× to walk the whole chain: bored, skit rotation, sleepy, lights out, woken and lit up by an agent event, and falling asleep again, with device reports matching expectations. Design in [`leisure.md`](leisure.md).

Stage 5 as a whole is not yet done: K2's extended behavior in the no-activity/done state is still unimplemented; the originally planned K0 mute has given way to the Pomodoro.

### Stage 6 — `beacon`

Implement `start`, `done`, `error`, and `run`; the CLI only calls `vibebuddyd`.

### Stage 7 — App (implemented, 2026-09-16; UI awaits a human walkthrough)

A native SwiftUI menu bar App that supervises `vibebuddyd`, handles first-run onboarding, connects agents, picks the announcement voice and writes it to the device's `voices` partition, updates firmware, and shows the device screen. Design in [`app.md`](app.md).

Hardware findings: the firmware gained a 2 MB `voices` partition; voice packs are written in chunks over the serial protocol and read back for verification, so changing voices doesn't require reflashing firmware; the `device.hello`, `device.identify`, and `device.echo` events are in place. The daemon has a status endpoint and SSE, a config file, screenshots, voice pack writing, and ROM-protocol flashing (256-byte blocks, segmented for the bridge, hard reset after MD5 verification; about 38 ms per block under the App's supervision, 1.5 MB in four minutes). `vibebuddy-hook` replaces the two Python scripts. On dragon's Mac the App has replaced the LaunchAgent: it launches the daemon, drives the menu via SSE, and the daemon exits along with it under both SIGTERM and SIGKILL. Verified end-to-end through the App: writing voice packs, flashing firmware, and hook forwarding. What hasn't yet been looked over by a human is the UI itself (six onboarding steps, five settings pages, notifications).

Along the way it turned out that the CH343 UART bridge can't swallow more than about two hundred bytes of continuous data (see `LESSONS.md`), so the daemon was changed to send in segments at line rate, which also fixed the old problem of long event lines occasionally producing `invalid_json`.

### Stage 8 — Characters

The buddy becomes a role that wears one Character at a time: a look, a voice, a persona and a pool of lines. Phase 1 gives the four existing voices personas and line pools for twelve occasions on the robot look, in a new Character pack format; phase 2 adds sprite-sheet looks and importing custom ones. Design in [`characters.md`](characters.md).

## Later

Optional task titles, microphone, voice interaction, long-term task history, Wi-Fi, WebSocket transport, progress, multiple devices. Real-time three-task cards for multiple agents have already moved forward into Stage 4.
