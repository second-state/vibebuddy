# DEV-19: XGGF speech recording consent, integrity, and lifecycle

Category: Device tab · Platform observed: Mac and Linux apps (`app/`, `desktop/`)

## Where it lives

- Mac: `app/Sources/VibeBuddy/SettingsView.swift`, `Korekore/Pages/DevicePage.swift`, `AudioRecorder.swift`
- Linux: `desktop/src/main.rs`, `korekore/pages.rs`, `client.rs`
- Shared authorization and capture state: `daemon/src/main.rs`, `daemon/src/audio.rs`
- Device capture and recording indicator: `firmware-xggf/main/audio.c`, `main/status.c`

## Initial state (input)

- The paired XGGF is connected and reports board `xggf-64x8`.
- The daemon listens on loopback. Audio capture is off unless the operator explicitly starts the daemon with `VIBEBUDDY_XGGF_AUDIO=1`.
- An isolated test daemon and temporary device pairing are used. Test audio is synthetic or explicitly consented speech; no user's private audio, Wi-Fi password, device key, or real recording is used as a fixture.

## Workflow

1. With audio capture disabled, both apps identify the XGGF and explain that recording requires the computer-level opt-in. Neither app offers Start/Record, and device notifications continue normally over encrypted Wi-Fi.
2. Restart the daemon with audio opt-in while it remains loopback-only. Both apps show equivalent recorder controls on the Device page in standard and Korekore views.
3. Start a bounded recording. The device shows its recording indicator, each app shows progress, and the daemon returns the recording only in memory.
4. Stop early and verify a usable partial WAV. Play it, save it to a test path, then discard it. Confirm Discard removes the in-memory WAV and stops any active device capture.
5. Start another recording and navigate away from the recorder page or close the window while it is active. Capture is cancelled/discarded within the documented bound; a later poll or reconnect must not restore the old recording UI or WAV.
6. Exercise nonzero `lost_frames`, a device overrun/discontinuity, and a terminal link error. All views show the same loss/error information; no view labels a lossy recording as clean.
7. Attempt to enable app-opt-in recording with a non-loopback daemon bind. The daemon refuses the unsafe combination or otherwise follows the documented fail-closed policy. Probe-token mode continues to require the bearer token on every audio route.

## Final state (output)

- Capture is explicitly started and visually indicated, bounded by the configured duration, and controllable from either platform.
- Page exit, window close, Stop, Discard, and link loss have distinct, documented outcomes.
- No audio endpoint is reachable without the expected authorization policy, and no audio payload appears in logs or default disk storage.
- Mac, Linux standard, and Linux Korekore expose equivalent progress, loss counts, errors, and playback/save/discard behavior.
- Core Agent notifications remain available over the paired encrypted Wi-Fi link.

## Check on Mac

- [ ] Disabled-by-default behavior and opt-in controls
- [ ] Clean, early-stop, lossy, and failed recordings
- [ ] Page/window exit cancels an active recording
- [ ] Play, save, discard, and no default disk retention
- [ ] Non-loopback bind policy

## Check on Linux

- [ ] Same behaviors in standard and Korekore views
- [ ] Korekore displays nonzero frame loss
- [ ] Page/window exit cancels an active recording
- [ ] Daemon service receives the documented opt-in configuration
- [ ] Non-loopback bind policy

## Results

- Status: not checked
- Checked on: (date, commit, operating system and desktop)
- Evidence: (daemon test, app test, physical test, log, or screenshot)
- Notes:
