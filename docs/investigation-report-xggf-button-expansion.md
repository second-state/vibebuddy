# Investigation Report: Expanding the XGGF Three-Button Frontend

**Date:** 2026-11-01
**Scope:** Read-only inspection of the current XGGF firmware, daemon audio path, protocol, Mac app, Linux app, and existing device-menu conventions.

## 1. Objective

Find existing capabilities that can make fuller use of the XGGF's three buttons, and separate safe near-term frontend work from actions that need more protocol, security, or transport evidence.

## 2. Methodology

Traced button gestures through `main.c` and `status.c`; inspected microphone visualization, speech recording, Wi-Fi/pairing state, panel brightness, daemon authorization, and Mac/Linux XGGF capability text. Compared possible interactions with the established three-key menu in `docs/device-menu.md`.

## 3. Key Findings

### 3.1 The current six gestures work, but there is no discoverable menu

The firmware binds K0 short/long, K1 short/long, K2 short/long directly in `firmware-xggf/main/main.c:116-147`. K1 long only calls `status_home`; there is no menu state or action selection. The status model exposes only Agent, clock, and environment pages (`firmware-xggf/main/status.h:6-7,48-58`).

The main VibeBuddy box already has a suitable convention: K1 long opens a menu, K1 moves, K0 selects, and K2 closes (`docs/device-menu.md:26-45`). This maps directly onto the XGGF without new hardware assumptions.

**Recommendation:** Make K1 long open a one-row-at-a-time menu. Inside it, use K1 short = next, K0 short = select, K2 short = close; preserve K2 long = mute. Agent events should close the menu, as they do on the main box (`docs/device-menu.md:116-121`).

Suggested initial rows: **MIC**, **LINK**, **BRIGHTNESS**, **IDENTIFY**, **MUTE**, **CLOSE**. These expose existing direct actions and leave room for later settings without overloading more gestures.

### 3.2 A read-only LINK page can use data the firmware already owns

`wifi_link.c` already tracks the SSID, paired-computer count, TCP peer/client state, Wi-Fi connection state, IP address, RSSI, and failure reasons (`firmware-xggf/main/wifi_link.c:38-68,256-265,292-306,377-447`). None of this is exposed to `status_model`; its public header only supplies link initialization, events, reporting, and diagnostics (`firmware-xggf/main/wifi_link.h:1-16`).

**Recommendation:** Add a small read-only status getter and a LINK menu/page cycling only non-secret fields:

- transport: USB, LAN, or offline;
- Wi-Fi SSID and joined/connecting/failed state;
- IP address when joined;
- paired-computer count.

Do not display passwords, private key material, or full pairing keys. Network provisioning should remain in the apps, which already have scan/password flows.

### 3.3 An active recording cannot currently be stopped by a button

K0 short checks `audio_active()` and only prints `MIC BUSY` when speech recording is active (`firmware-xggf/main/main.c:122-130`). Firmware already has bounded start/stop and visible `REC` (`firmware-xggf/main/audio.c:32-48,95-140`; `firmware-xggf/main/status.c:78-82`). The daemon's `stop()` marks an armed recording as allowed to finish short (`daemon/src/audio.rs:89-93`), which is required for an early end to validate (`daemon/src/audio.rs:252-270`).

**Recommendation:** Let K0 short stop an active recording instead of merely saying BUSY. Add a device-to-daemon `audio.stop.request` event. When a recording is active, the daemon should call its existing `AudioIngestor::stop()` and send `device.audio.stop`; otherwise it should ignore the request. This is a safe stop operation and does not bypass authorization to start recording.

A stop-only physical control should be implemented before any button-based recording start.

### 3.4 Button-based recording start or push-to-talk is not ready

The current recording probe is explicitly token-gated at every HTTP operation, including start, stop, delete, WAV retrieval, and stats (`daemon/src/main.rs:822-833,839-900`). Its production boundary requires explicit start/stop, bounded duration, visible indication, no default retention, and confirmed understandable speech (`firmware-xggf/AUDIO_PROBE_PLAN.md:96-98`). Wi-Fi 16 kHz completion remains intermittent and no 30-second stress run has passed (`firmware-xggf/AUDIO_PROBE_PLAN.md:42-49`).

The full VibeBuddy voice-command protocol is a different, richer flow with `voice.available`, push-to-talk, recognition, replies, and turn state (`docs/protocol.md:234-270`). The XGGF's PCM `AUDIO FRAME` probe does not implement that flow.

**Recommendation:** Do not bind recording start or voice push-to-talk yet. First finish the audio reliability/fidelity gates and make an explicit product decision about how physical presence relates to the token authorization boundary.

### 3.5 Brightness is a feasible local setting, but needs scaling and persistence

Panel output currently has a fixed per-channel cap of 8 (`firmware-xggf/main/panel.c:4-5,17-25`). Renderer colors are already small constants, and no runtime brightness setting exists in `status_model`.

**Recommendation:** A BRIGHTNESS menu row is reasonable after LINK. Implement a bounded global scale in `panel_encode`, with a few tested levels and the existing cap of 8 as maximum. Persist it only if there is a clear NVS ownership boundary; avoid putting incidental settings into the link credential store. Verify readability and power behavior on the physical panel.

### 3.6 Pairing changes should remain out of the first menu

The firmware supports pairing, unpairing, Wi-Fi set/forget, and up to 16 paired computers (`firmware-xggf/main/wifi_link.c:377-447`). These operations modify persistent credentials and can disconnect computers. The main box uses dedicated views and explicit confirmation for comparable actions (`docs/device-menu.md:57-67`).

**Recommendation:** Show paired count read-only. Do not add forget-network, unpair, or new-identity actions until there is a specific confirmation design and user request.

### 3.7 Mac and Linux capability text is stale

Both platforms still state that “device buttons are not yet supported”:

- `app/Sources/VibeBuddy/Korekore/Pages/CharacterPage.swift:17-20`
- `app/Sources/VibeBuddy/SettingsView.swift:167-170`
- `desktop/src/korekore/pages.rs:53-56`
- `desktop/src/main.rs:1546-1549`
- translation: `app/Localization/zh-Hans.lproj/Localizable.strings:635`

Comments also claim XGGF “has no K0” in `app/Sources/VibeBuddy/ConversationView.swift:15-16`, `app/Sources/KorekoreUI/ConversationPage.swift:27-28`, and `desktop/src/korekore/pages.rs:465-466`. The visible conversation text correctly says voice commands are unsupported; it is the button claim in comments and capability text that is wrong.

**Recommendation:** Update both apps in the same change: remove buttons from the unsupported list and add a short XGGF button guide to the Device page. Keep Voice, Characters, Relay, and timers listed as unsupported. Update the shared zh-Hans strings and run `tools/check-localization.py`.

## 4. Recommended Interaction Model

```text
Normal display
  K0 short: mic visualizer on/off; stop active recording if one is running
  K0 long: identify (retain until voice availability explicitly changes it)
  K1 short: Agent -> clock -> environment
  K1 long: open menu
  K2 short: open current Agent source through the existing daemon path
  K2 long: mute/unmute

Menu
  K1 short: next row
  K0 short: select/act
  K2 short: close
  K2 long: mute/unmute
  Rows: MIC, LINK, BRIGHTNESS, IDENTIFY, MUTE, CLOSE
```

This keeps all current useful shortcuts while making them discoverable. It also follows the existing VibeBuddy menu mental model rather than inventing a separate XGGF navigation scheme.

## 5. Risk & Impact

- **Privacy:** stop-recording is safe and desirable; start-recording needs an authorization decision.
- **Security:** pairing and identity operations are persistent and destructive; keep them read-only until confirmed.
- **Transport:** do not promote Wi-Fi speech recording to a button feature while its reliability gates remain open.
- **Power/readability:** brightness needs physical validation; retain the existing channel cap.
- **Parity:** Mac and Linux text and guidance must change together with zh-Hans translations.

## 6. Gaps & Uncertainties

| # | Gap | Impact | Follow-up |
|---|-----|--------|-----------|
| 1 | No public Wi-Fi status getter currently exists. | LINK needs a small firmware API, not just renderer work. | Return a bounded struct containing only non-secret fields. |
| 2 | No device-to-daemon recording-stop request exists. | K0 cannot safely complete an early recording without daemon state changes. | Add protocol handling and tests for active/idle sessions. |
| 3 | Brightness persistence ownership is undecided. | Settings could be lost at reboot or mixed with credentials. | Choose a separate NVS key/namespace and migration behavior. |
| 4 | Subjective speech intelligibility and Wi-Fi reliability are unresolved. | Push-to-talk is not ready for production. | Finish AUDIO_PROBE_PLAN gates 2–5. |

## 7. Conclusions

The highest-value safe expansion is **K1-long menu + LINK diagnostics + stop-active-recording + corrected cross-platform button guidance**. These reuse existing device and daemon capabilities without weakening audio authorization or pairing safety.

Brightness is a reasonable follow-up. Button-based recording start, push-to-talk, network forgetting, unpairing, and new identity should remain deferred until their specific reliability, privacy, and confirmation requirements are addressed. Confidence is high because each recommendation is based on existing call paths and explicitly identified state boundaries.
