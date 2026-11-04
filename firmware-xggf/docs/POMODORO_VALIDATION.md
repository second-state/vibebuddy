# Pomodoro display validation — 2026-11-03

Implementation follows `PLAN_XGGF_POMODORO_DISPLAY.md`. The owner accepted D8–D14
recommendations and approved the layout preview. Hardware observations listed
below are still required before marking the whole plan accepted.

## Passed

- Eleven C host commands: existing panel/status/font/buttons/button-actions/audio
  checks, preview generation, calendar clock, volume table, buzzer sequences and
  RTC codec. The preview tool now produces 92 PBM and 92 color PPM images.
- Rust bridge: five default-feature tests, including real default durations,
  completed focus/break transitions, restored tally and abandon through the ABI.
  The accelerated feature build also passes.
- Firmware core: 172 unit tests, 92 firmware integration tests and five voicepack
  integration tests. The box accepts the added heartbeat `minute` field.
- Daemon: 434 passed, three ignored. Linux app: 83 passed, two ignored.
- Localization: 607 keys checked. Mac compilation remains unavailable on Linux.
- Default and accelerated ESP-IDF images build. The default CMake configuration
  is TIME_SCALE=1, BUTTON_AUTOPILOT=OFF and RTC_WRITE=OFF. Acceleration requires
  an explicit autopilot acceptance build.
- Connected ESP32-C3: MCU button transitions and protocol robustness checks.
- Accelerated device cycle: start/pause/resume/abandon, explicit break start,
  focus/break completion, page preservation on Agent input, clock sync, daily
  reset, real audio recording-stop precedence and page navigation on every page.
- Updated RTC build: MIC menu start/stop, all six volume steps, and a focus
  ending over an open menu (K0 then starts the break).
- RTC write/readback; time restored after reset before any heartbeat; daily tally
  and volume retained; running timer deliberately returns to idle on reset.
- Fifteen-minute soak: 15 diagnostic samples, heap change −52 bytes, minimum
  main-stack headroom 7,836 bytes, panel transfer 15,867–15,879 microseconds,
  zero audio overruns and no reset.
- Two-minute follow-up with the new RTC validation path: heap change +20 bytes,
  minimum stack headroom 7,788 bytes, zero audio overruns and no reset.
- Installed OpenCode plugin permission-event handler exercised through the real
  hook and rebuilt daemon: activity reached `needs_input` with XGGF connected.
  The event was injected into the plugin; a live permission prompt is still a
  separate manual gate. The test activity was deleted afterward.
- Final default image flashed, its hash verified by esptool, serial smoke passed,
  and the rebuilt daemon restored. Its status reports the XGGF connected.

## Artifacts

The flashed default image matches `build/vibebuddy_xggf_bringup.bin`:

```text
e4a108063cd6bbc22bc9ac1ab07287fae535a883d28b305d027c830781ae1c59
```

Private full backups and temporary test logs are outside the repository under
`/tmp/xggf-pomodoro-backup/` and `/tmp/xggf-pomo-*.log`. Two complete 4 MB reads
matched; secure boot and flash encryption were disabled, and the partition table
matched before writing the application. The daemon was stopped for serial
ownership and restored with its original arguments, directory and environment.

See [the approved preview](pomodoro-preview.png) and [RTC evidence](RTC.md).

## Remaining acceptance gates

- Actual GPIO button presses and visible behavior, including MIC menu access,
  Pomodoro controls and identify. MCU-injected gestures do not certify switches.
- Human confirmation that focus/break chimes are distinct, all VOLUME levels are
  audible as intended, mute silences them and identify, and start/pause are silent.
- Real OpenCode permission-prompt observation with the blinking alert visible on
  the panel. Injected protocol events do not certify this entire scenario.
- Power-off RTC retention with the daemon stopped. A reset does not prove backup
  power retention. The RX-8025-compatible protocol is inferred from the stock
  binary and manufacturer documentation; package marking remains unconfirmed.
- Separate Phase A/B/C PRs and their merge gates. No PR has been published or
  merged from this working tree; existing staged legacy-document moves and other
  pre-existing work have been preserved.
