# Plan: XGGF Pomodoro, clock, alert and button rework

**Status:** Implementation and automated host/device validation completed in the working tree. Owner accepted D8–D14 and approved the layout. Manual hardware and PR/merge gates remain open; see [validation results](firmware-xggf/docs/POMODORO_VALIDATION.md).
**Date:** 2026-11-03
**Branch:** `xggf-64x8`
**Source decisions:** the owner's review, recorded in [`PLAN_XGGF_64X8.md`](PLAN_XGGF_64X8.md) under "Decisions 2026-11-03". This document turns them into work items.

## 1. Goal

Bring three things to the XGGF 64×8 device that the Cardputer box already has, adapted to a 64×8 panel and three buttons:

1. A Pomodoro timer with its own page.
2. A clock that works offline, with its time taken from the desktop daemon.
3. A visible, blinking alert while an agent needs input.

Along the way, the mic meter loses its K0 short press, and the device gets a VOLUME row and buzzer chimes.

Non-goals: stock content (weather, ambient animations, scoreboard, messages), auto-brightness, Leisure (see D10), and speech on the device.

## 2. Baseline: what exists today

All paths are relative to the repository root.

| Area | Current behaviour | Where |
|---|---|---|
| Buttons, HOME | K0 short → mic toggle; K0 long → identify; K1 short → cycle views; K1 long → menu; K2 short → source; K2 long → mute | `firmware-xggf/main/button_actions.c` (`button_actions_handle`, HOME branch) |
| Buttons, recording | K0 short stops an active recording in every mode | `button_actions.c`, first branch of `button_actions_handle` |
| Menu rows | MIC, LINK, BRIGHTNESS, IDENTIFY, MUTE, CLOSE | `button_actions.h` (`button_menu_row`), `button_actions.c` (MENU branch) |
| Actions | 19 `button_action` values, with names in `button_action_name` | `button_actions.h` |
| Views | `STATUS_VIEW_AGENT`, `_CLOCK`, `_ENVIRONMENT` | `main/status.h` |
| Renderer | `status_render` draws the face, label and text; the steady recording dot sits at top-right (cols 61–62, rows 0–1); the connection bar is column 63 | `main/status.c` |
| Offline | Grey face with closed eyes; `OFFLINE` label; rotates clock and environment readings every 4 s | `status.c`, `STATUS_VIEW_AGENT` branch |
| Heartbeat | The daemon sends `device.heartbeat` to every device every 5 s, carrying `build`, `hour` and `day` (`YYYYMMDD`). The XGGF uses it only for link liveness and ignores the fields | `daemon/src/main.rs` (`send_heartbeats`, `heartbeat_event`); `firmware-xggf/main/main.c` |
| Time | The RTC at I²C `0x32` is read in `probe_sensors()` every 60 s. It is never written. Only a BCD range check is applied | `main/main.c` |
| Buzzer | 1 kHz LEDC on GPIO 3, fixed 80 ms tone, used only for identify. No volume control | `main/buzzer.c`, `main/buzzer.h` |
| Settings | NVS namespace `xggf_ui` holds brightness | `main/settings.c` |
| Pomodoro logic | Exists for the box as a clock-free Rust module: `Pomodoro::{toggle, stop, tick, view, tally, set_day}` | `firmware-rs/core/src/pomodoro.rs` |
| Rust bridge | Static library `libvibebuddy_xggf_link.a` already linked into the XGGF build, exporting C functions `vb_link_*` | `firmware-xggf/link/src/lib.rs`, `firmware-xggf/main/CMakeLists.txt` |
| Desktop guide | English strings "Left/K0: microphone or stop recording…" and similar | `desktop/src/main.rs` (`xggf_button_guide`) |
| Mac guide | "XGGF buttons" section | `app/Sources/VibeBuddy/Korekore/Pages/DevicePage.swift` (around line 126) |
| Tests | Button transitions, status rendering, panel, font, buttons, audio C tests; a preview tool that writes 56 PBM files; physical scripts | `firmware-xggf/tests/` |

## 3. Locked decisions

These come from the owner's review. Changing one needs a new decision.

- **D1 Offline shows the clock.** It is styled for the 64×8 panel. Until the time has been synced, it shows `NO TIME`. It never shows a guessed time.
- **D2 The mic meter loses its button, not its function.** K0 short no longer toggles the visualizer. The MIC row in the menu still starts it. K0 short still stops an active recording. The state machine change must be tested as described in §6, Phase A.
- **D3 Pomodoro chimes play on the buzzer.** Their loudness follows the VOLUME row. Start, pause and resume stay silent.
- **D4 The date and time come from the desktop daemon.** Source: the heartbeat, which the daemon already sends.
- **D5 Agent alert.** While input is required, a 2×2 dot blinks at the bottom-right corner (cols 61–62, rows 6–7). It mirrors the steady recording dot at top-right.
- **D6 Device display is English only.** No translation tables on the device. The desktop apps' guidance text is still translated, as `AGENTS.md` requires.
- **D7 The dial becomes a progress bar.** The countdown is shown in the 5×7 font.

## 4. Decisions still needed

Each has a recommendation. Phase A cannot start until D8 and D12 are answered. The others can wait for their phase.

- **D8 K0 long and abandon (blocks Phase A).** K0 long currently identifies. On the box, K0 long abandons a phase, and the box also has a menu row for it. *Recommended:* K0 long keeps identify everywhere, so nothing changes for users. Abandon becomes a menu row, STOP FOCUS, which appears while a phase is running, paused or waiting to start its break (the box's rule, `docs/device-menu.md`).
- **D9 Agent cue sounds (blocks Phase C4).** Should the buzzer chime when input is needed, a task is done, or a task fails? The box speaks these. *Recommended:* off by default, with a menu toggle later.
- **D10 Leisure.** *Recommended:* not included. K1 is used for page cycling.
- **D11 VOLUME steps.** *Recommended:* the box's levels, 20 → 35 → 50 → 65 → 80 → 100 → 20, for consistency across devices. The steps are percentages of the current buzzer duty cycle.
- **D12 Page order (blocks Phase A).** K1 short cycles Agent → Clock → Environment → Pomodoro → Agent. *Recommended:* accept this order.
- **D13 Pomodoro screen layout.** The 8-row panel has no room for two lines of 5×7 text. The layout is set in the preview: digits, progress bar, the TODAY record and the agent alert. *Recommended:* digits on rows 0–6, progress bar on row 7, and the TODAY count as a small number near the bar, to be confirmed in the preview.
- **D14 Acceptance time scale.** Testing 25 minutes on the bench is impractical. *Recommended:* a build option `XGGF_POMODORO_TIME_SCALE` that divides the durations by 60 for a test build only. The default build must always use 1, and a host test must assert that.

## 5. Target behaviour

### 5.1 Pages and buttons

K1 short cycles through the four pages. K1 long opens the menu, as now.

| Gesture | Agent / Clock / Environment | Pomodoro page | Recording active | Menu |
|---|---|---|---|---|
| K0 short | No action (was mic toggle) | Start focus, or pause/resume the running phase; in break-ready, start break | Stop recording | Activate row (MIC still toggles the visualizer) |
| K0 long | Identify (D8) | Abandon the phase (D8: the menu row replaces this) | Ignored | Ignored |
| K1 short | Next page | Next page | Next page | Next row |
| K1 long | Open menu | Open menu | Open menu | Ignored |
| K2 short | Open the Agent source | Open the Agent source | Open the Agent source | Close menu |
| K2 long | Toggle mute | Toggle mute | Toggle mute | Toggle mute |

Menu rows, once D8 and D11 are settled: MIC, LINK, BRIGHTNESS, VOLUME, IDENTIFY, STOP FOCUS (shown while a phase is running, paused or waiting to start its break), MUTE, CLOSE.

### 5.2 Clock and offline

- **Clock page** shows `HH:MM` from the synced time. Without a sync, it shows `NO TIME`.
- **Offline** shows the grey face with closed eyes, and then the clock if a time is known (D1). It no longer rotates the environment readings. The offline screen stays still apart from the minute changing.
- **Beautification** is a preview task. The current 9×8 face is kept at the left, the clock at the right, and the offline colour is grey.

### 5.3 Pomodoro state machine

Phases, taken from the box spec in `docs/pomodoro.md`:

- **Idle:** shows `25:00`, empty progress bar, READY blue.
- **Focus running / paused:** red progress bar fills over 25 minutes. Paused: digits blink every 0.5 s and the bar stops.
- **Break ready:** after focus ends, waits for K0. Shows `05:00`, green, no movement.
- **Break running / paused:** as focus, with green.
- **Alarm:** at the end of a phase, the chime plays once, the progress bar lights in the next phase's colour, and it pulses for the rest of the phase until K0. (The box shakes for 2 s first. On the 64×8 panel, use a blink for 2 s then the pulse, as a preview option.)
- **Return to idle:** after the break ends. The next focus starts only on K0.

Rules to keep from the box:

- K0 starts a focus and switches the device to the Pomodoro page.
- A focus that ends on a page other than Pomodoro brings the Pomodoro page to the front.
- Agent events never change the page.
- The agent alert dot (D5) stays visible on the Pomodoro page.
- If break-ready sits for 5 minutes, or paused for 30 minutes, with no key press, the device returns to the Agent page (the box's rule).
- The timer keeps running offline (the box's rule).

Implementation: call the existing `firmware-rs/core/src/pomodoro.rs` through new `vb_pomodoro_*` exports in `firmware-xggf/link/src/lib.rs`. Do not rewrite the logic in C. The module takes millisecond counts from the caller, which fits the firmware.

### 5.4 Alert dot (D5)

- Shown while the Agent state is `INPUT`, on every page.
- A 2×2 dot at cols 61–62, rows 6–7, blinking every 500 ms, in amber.
- Does not overlap the connection bar (col 63) or the recording dot (top-right).
- Stops at once when the state leaves `INPUT`.

### 5.5 VOLUME and sound

- VOLUME sets the buzzer loudness for chimes and identify. The steps follow D11.
- Chimes: focus end is a descending two-note figure; break end is a rising three-note figure. These follow the box's shapes in `tools/make-pomodoro-audio.py`, adapted to square tones at the buzzer's 1 kHz base frequency, or to a frequency sequence if the LEDC can set one per note.
- Mute (K2 long) silences everything, as on the box. Mute is not persisted.
- Start, pause, resume and abandon are silent.

### 5.6 Time and date

- The daemon's heartbeat already carries `hour` and `day`. Add `minute`. This is a backwards-compatible addition, and the box must be checked to ignore it (task B1).
- The firmware reads `hour`, `minute` and `day` on every heartbeat.
- Clock source: the last heartbeat plus milliseconds since then, with the RTC written from it so the time survives an offline reboot (task B4, which depends on the RTC chip being identified).
- `day` drives the Pomodoro daily reset.

### 5.7 Persistence

NVS namespace `xggf_ui`:
- `pomo_day`: the date of the stored record, `YYYYMMDD`.
- `pomo_count` and `pomo_secs`: completed focus count and total seconds.
- The running timer is not persisted. A reboot during a phase returns to idle. This is a deliberate simplification and is listed in §8.

## 6. Work breakdown

Each phase ends with a gate. Phase A, B and C are separate pull requests. Phase D lands with whichever phase first changes a user-visible string. This keeps the app guides in step with the device.

### Phase A: firmware UI and buttons (no protocol change)

Blocked by: D8, D12.

- **A1 Button state machine.**
  - Files: `firmware-xggf/main/button_actions.{h,c}`, `firmware-xggf/tests/button_actions_test.c`, `firmware-xggf/tests/button_autopilot.py`.
  - Remove `BUTTON_ACTION_MIC_TOGGLE` from HOME K0 short. Add a `BUTTON_UI_POMODORO`-style page handling, or a view-based rule, so K0 short on the Pomodoro page starts or pauses.
  - Keep recording-stop precedence over every mode.
  - Add the page to `BUTTON_ACTION_VIEW_NEXT` cycling (D12).
  - Add action names for every new action, because `button_action_name` is used in diagnostics.
  - Acceptance: host transition tests cover every row of the §5.1 table, including recording active on each page.
- **A2 Status model and renderer.**
  - Files: `main/status.{h,c}`, `tests/status_test.c`, `tests/preview.c`.
  - Add `STATUS_VIEW_POMODORO` and the alert-dot state. Draw the alert dot per §5.4.
  - Draw the progress bar on row 7.
  - Add preview scenarios: pomodoro idle, focus running, focus paused, break ready, break running, alarm, input-alert on each page, and recording with alert together.
  - Acceptance: every scenario renders in the 1× and 8× previews, and the owner approves the layout (D13).
- **A3 Pomodoro module.**
  - Files: `firmware-xggf/link/src/lib.rs`, `firmware-xggf/link/Cargo.toml` if needed, new `main/pomodoro.{h,c}`.
  - Export `vb_pomodoro_new`, `vb_pomodoro_toggle`, `vb_pomodoro_stop`, `vb_pomodoro_tick`, `vb_pomodoro_view`, `vb_pomodoro_tally`, `vb_pomodoro_set_day`, using the same pattern as `vb_link_*`.
  - Acceptance: a host test calls the same entry points through the Rust unit tests, and the box's `tools/test-pomodoro.sh` still passes.
- **A4 Menu.**
  - Files: `button_actions.{h,c}`, `status.c`.
  - Add the STOP FOCUS row (D8), shown while a phase is running, paused or waiting to start its break.
  - Acceptance: menu tests cover the row being absent when Pomodoro is idle, and present in each of the three states.
- **A5 Apps and docs in the same PR.**
  - Update the K0 wording in the guide strings in `desktop/src/main.rs` and in `DevicePage.swift`. Both platforms change together.
  - Add zh-Hans entries for the app strings and run `tools/check-localization.py`.
  - Update `firmware-xggf/README.md` (the Buttons table) and `docs/device-menu.md` (the XGGF variant section).
- **Gate A (physical):**
  - K0 short on Agent does nothing.
  - MIC row in the menu still starts and stops the visualizer.
  - Recording stops on K0 short from every page, and K1 still changes page during a recording.
  - Pomodoro start, pause, resume, abandon and break-start work by button.
  - Alert dot blinks during a real OpenCode `INPUT` (the permission-prompt test from earlier).
  - Identify still works (D8).

### Phase B: time and date from the daemon

Blocked by: Phase A merged; RTC identification (B4).

- **B1 Daemon heartbeat.**
  - Files: `daemon/src/main.rs` (`heartbeat_event`, `send_heartbeats`), test at `heartbeat_carries_build_local_hour_and_day`.
  - Add `minute`. Update the test.
  - Check that the box accepts unknown heartbeat fields. Check the box's parser in `firmware-rs/core`, and add a test that feeds it a heartbeat with `minute` present.
- **B2 Protocol documentation.** `docs/protocol.md`, "XGGF 64×8 experimental board": list the heartbeat fields the XGGF reads.
- **B3 Firmware time source.**
  - Files: `main/main.c` (heartbeat handling, `probe_sensors`), new `main/clock.{h,c}`.
  - Parse `hour`, `minute`, `day`. Keep the last time and milliseconds since then. Show `NO TIME` until the first valid heartbeat or an RTC read that is valid.
  - Acceptance: host tests for the clock module, covering rollover at midnight and minute carry.
- **B4 RTC write.**
  - Identify the chip. The address `0x32` and the 7-byte read at register 0 are consistent with an RX8010-family part, but this is unverified.
  - Read the stock `set_time` path. The stock `/api/time` handler is at `0x4200fc86` in the stock flash dump (`/mnt/dell-ext/Workspace/XGGF/flash_dumps/`). Disassemble it to get the register writes and the BCD format.
  - Implement the write behind a flag, and verify by reading back after power-off.
  - Acceptance: the RTC keeps the time across a reboot with the daemon stopped. If it does not, record that and keep the NVS fallback (§5.7) as the only offline source.
- **B5 Offline clock and beautification.**
  - Files: `main/status.c`, `tests/preview.c`.
  - Draw the offline clock per §5.2. Owner approves the preview.
- **B6 Pomodoro daily record.**
  - Files: `main/pomodoro.c`, `main/settings.c`.
  - Store `pomo_day`, `pomo_count`, `pomo_secs` in NVS. Reset when the heartbeat `day` changes.
  - Acceptance: a day change during a test resets the record; a reboot keeps it.
- **Gate B:**
  - Daemon test passes. Box tests pass.
  - Time on the device matches the desktop within a minute after sync.
  - Offline screen shows the clock, and the value is still correct after a daemon stop.
  - Pomodoro record resets on a simulated day change.

### Phase C: VOLUME row and chimes

Blocked by: Phase B for the record; D9 for C4.

- **C1 Buzzer.**
  - Files: `main/buzzer.{h,c}`.
  - Add a volume setting (D11) and a tone sequence API. Use `ledc_set_freq` between notes if the LEDC supports it on this chip.
  - Keep the existing identify tone as the default sequence.
  - Acceptance: a host-side test of the step table, and an audible check.
- **C2 VOLUME row.**
  - Files: `button_actions.{h,c}`, `status.c`, `settings.c`.
  - Add the row. K0 short cycles the steps. Store the level in `xggf_ui`.
- **C3 Pomodoro chimes.**
  - End of focus and end of break. Silent for start, pause and abandon. Respect mute.
- **C4 Agent cues (only if D9 is yes).** Chime on INPUT, DONE and FAILED, respecting mute and volume.
- **Gate C:**
  - Chimes audible at each VOLUME step.
  - Mute silences chimes and identify.
  - Start and pause produce no sound.

### Phase D: docs, apps and parity

Parts of D land with Phase A (A5). The rest land with B and C where they change.

- `firmware-xggf/README.md`: Buttons table, Pomodoro, alert dot, clock.
- `docs/device-menu.md`: XGGF variant section.
- `docs/protocol.md`: heartbeat fields.
- `docs/platform-parity.md`: XGGF Pomodoro, VOLUME and clock rows.
- `CONTEXT.md`: add any new domain terms, such as "alert dot" and "Pomodoro page", if the domain doc covers the XGGF.
- Device display strings stay English (D6). Only the app guides are translated.
- `PLAN_XGGF_64X8.md`: update the status of each section.

## 7. Test plan

| Level | What | Command |
|---|---|---|
| Firmware host C | Button transitions, status, panel, font, buttons, audio, preview | The `cc` lines in `firmware-xggf/README.md` |
| Firmware host Rust | Pomodoro through `vb_pomodoro_*` | `cargo test` in `firmware-rs/core` and `firmware-xggf/link` |
| Firmware build | Default build (TIME_SCALE 1, autopilot off) | `idf.py build` in `firmware-xggf` |
| Firmware test build | Autopilot on, for the button tests | `idf.py -DXGGF_BUTTON_AUTOPILOT=ON build`, then restore the default build before flashing |
| Daemon | Heartbeat test and full suite | `cargo test -p vibebuddyd` |
| Desktop | Compile and tests | `cargo check -p vibebuddy-desktop`, `cargo test -p vibebuddy-desktop` |
| Mac | Not available on this host | Needs a Swift toolchain; record the gap |
| Localization | Keys and placeholders | `python3 tools/check-localization.py` |
| Physical | Buttons, menu, alert, chimes, clock, record | `tests/buttons.py`, `tests/physical_serial.py`, `tests/soak.py` (update to rotate the new states), and a manual checklist per gate |

Soak: `tests/soak.py` must include the Pomodoro and alert states, so the 15-minute soak exercises the new code.

## 8. Risks

- **Button regressions.** K0 short is used for recording stop, and recording-stop precedence must survive the change. Mitigation: transition tests for every page with recording active (A1).
- **Timer lost on reboot.** A running Pomodoro phase is lost if the device restarts. Mitigation: documented as a simplification; the record and the day survive (§5.7).
- **RTC unknown.** The chip is not yet identified. Mitigation: B4 gates the RTC write, with the NVS fallback.
- **Protocol addition.** The new heartbeat field must not break the box. Mitigation: B1 tests the box parser.
- **Layout space.** Eight rows constrain every screen. Mitigation: previews are approved before code is merged (D13).
- **Parity drift.** The apps could describe the old K0 behaviour after A1. Mitigation: A5 lands in the same PR.
- **Chime timing.** The chimes must not block the render loop. Mitigation: the tone sequence is driven from `buzzer_tick`, not by sleeping.

## 9. Acceptance criteria (whole plan)

1. The mic meter is reachable only from the menu, and K0 short no longer changes it.
2. Pomodoro runs through a full focus and break on the device, with the record updated.
3. The alert dot blinks while an agent needs input, on every page.
4. The offline screen shows the synced clock, and `NO TIME` before any sync.
5. The VOLUME row changes chime and identify loudness, and mute silences both.
6. The default firmware build uses the real 25- and 5-minute durations.
7. Both desktop apps describe the same XGGF controls, with zh-Hans strings checked.
8. Host, daemon and physical tests pass, and the Mac gap is recorded.

## 10. Order of work

1. Owner answers D8 and D12. (Blocks Phase A.)
2. Phase A, one PR: A1 through A5, with the physical gate.
3. Phase B, one PR: B1 through B6. B4 may need a separate investigation first.
4. Phase C, one PR. C4 only if D9 is yes.
5. Phase D items not already done, then the status update in `PLAN_XGGF_64X8.md`.


## Execution notes (2026-11-03)

D8–D14 recommendations accepted by the owner. D9 agent cues stay disabled;
Leisure is excluded. The owner approved [the generated layout](firmware-xggf/docs/pomodoro-preview.png).

Implemented in the working tree: four-page controls and recording precedence,
shared Rust Pomodoro ABI, timer/progress/count rendering, alert dot, daemon
heartbeat minute and box compatibility coverage, validated monotonic calendar
clock, NVS daily tally, persistent volume, nonblocking chimes, mute, both app
guides and shared Chinese translations. Default builds use scale 1, autopilot
OFF and experimental RTC writes OFF. Acceptance builds use scale 60.

Automated host validation: all C host checks plus clock/volume tests; Rust bridge
5 tests; firmware core 172 unit, 92 firmware integration and 5 voicepack tests;
daemon 434 passed/3 ignored; Linux app 83 passed/2 ignored; localization 607
keys. The additive heartbeat minute passes the box tally/reboot integration test.
Both default and accelerated ESP-IDF images build. Mac compilation is not
available on this Linux host.

Device: ESP32-C3 QFN32 rev 0.4, 4 MB flash, encryption and secure boot disabled.
Two full backups match and the partition table matches the build. Private
backups and logs are outside the repository in `/tmp/xggf-pomodoro-backup/`.
MCU autopilot transitions and serial protocol tests pass. End-to-end timer, recording, reset persistence and soak checks passed; see the validation report for remaining manual gates.

RTC evidence: the existing stock analysis recovers the writer at `0x420018BE`
(register 0 plus seven packed-BCD bytes). The experimental implementation is
behind `XGGF_RTC_WRITE`; protocol evidence now indicates RX-8025 compatibility (see `firmware-xggf/docs/RTC.md`); package identity and power-off retention remain unverified.
No stale NVS timestamp is treated as a valid offline clock. A running phase is
intentionally lost on reboot. Preview approval does not close the audible,
physical GPIO, real OpenCode permission-prompt, or power-off RTC gates.

The plan's separate PR/merge gates remain open; these working-tree changes have
not been pushed or merged.
