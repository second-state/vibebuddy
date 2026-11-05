# Investigation Report: XGGF USB Communication

**Date:** 2026-11-05  
**Scope:** Current working tree based on `eeaf6f4`, including the uncommitted XGGF pairing and Linux serial-access changes made during the break. Assessment only; implementation files were not changed.

## 1. Objective

Determine whether the desktop apps and XGGF can exchange device commands and replies entirely over a USB serial port, without requiring Wi-Fi, and assess the intervening changes.

**Assessment:** The implementation supports communication over USB without a Wi-Fi connection. The new work improves pairing consistency and Linux port access. Approval of the changes is qualified by one reproducible pairing parity failure and the absence of a fresh physical acceptance run. There is also no strict USB-only transport policy: network fallback remains enabled.

## 2. Methodology

Reviewed the working-tree diffs, traced both apps through the daemon to USB input/output and the C/Rust pairing bridge, inspected storage migration and installer wiring, and ran the relevant host tests. Unrelated Kimi adapter changes were excluded from the review. Read the project context, architecture, lessons, and ADR-0012 for the existing USB > LAN > Relay policy.

## 3. Key Findings

### 3.1 USB carries the protocol directly; Wi-Fi association is unnecessary

- **Evidence:** `app/Sources/VibeBuddy/DaemonClient.swift:6` and `desktop/src/client.rs:12` both address the local daemon at `http://127.0.0.1:7331`.
- **Evidence:** `firmware-xggf/main/main.c:446` installs USB Serial/JTAG and directs console output through its driver. At line 494 it calls `usb_serial_jtag_read_bytes`; complete lines are dispatched with `receive_line(..., true)` at line 498.
- **Evidence:** `firmware-xggf/main/main.c:485` reports `WIFI UNAVAILABLE` on initialization failure and continues toward the same USB processing loop. `firmware-xggf/main/wifi_link.c:402` only configures a saved SSID when one exists; USB commands are not gated on station association or an IP address.
- **Interpretation:** App-to-daemon HTTP uses the computer's loopback interface. Daemon-to-device communication uses the cable directly when USB is selected. No router, hotspot, account, or device IP address is needed for that path. Pairing identity initialization still shares the `wifi_link_init` function; a Wi-Fi-free link does not mean that the firmware skips all network initialization code.

### 3.2 USB takes priority, but the product does not enforce USB-only operation

- **Evidence:** `daemon/src/serial_transport.rs:284` enters LAN/Relay fallback when no serial candidate exists. At line 981 an absent explicitly pinned serial port deliberately returns `Found::None`, allowing that fallback.
- **Evidence:** `daemon/src/main.rs:275` attaches LAN and Relay configurations when the computer has an identity and state directory.
- **Evidence:** `firmware-xggf/main/main.c:373` calls `wifi_link_usb_activity()` before handling USB commands. The bridge test at `firmware-xggf/link/src/lib.rs:191` asserts that USB closes the network peer and that the closed peer can no longer deliver notifications.
- **Interpretation:** The original requirement of USB communication without depending on Wi-Fi is supported. A stronger requirement that the link must never use Wi-Fi, including after unplugging USB, is not implemented. `VIBEBUDDY_SERIAL_PORT` narrows discovery; it is not a network-disable switch. This matches ADR-0012's existing fallback policy.

### 3.3 The new pairing implementation removes duplicated state and handles legacy storage

- **Evidence:** `firmware-xggf/link/src/pairing.rs:138` saves the old list before mutation, saves the resulting bytes, and restores the previous list on a failed save. Network-origin pairing commands return `ERROR cable_only` at line 117.
- **Evidence:** `firmware-xggf/main/pair_store.c:18` stores the list under one `paired` blob. Its legacy loader at line 34 migrates the earlier `pairs`/`names` arrays, removing them only after a successful new-format save. The new format takes precedence, including when its list is empty.
- **Evidence:** `firmware-xggf/main/wifi_link.c:393` loads the bridge's list before starting the station. The C layer no longer owns a second pairing list.
- **Interpretation:** Pairing commands over USB now use shared Rust domain rules, with explicit storage callbacks. Host tests verify restart persistence, migration, invalid names, failed writes, and prevention of removed computers reappearing from old storage. These tests do not establish physical flash durability under power loss or a failed real NVS commit.

### 3.4 [P2] Malformed unpair keys incorrectly receive a success reply

- **Evidence:** `firmware-xggf/link/src/pairing.rs:130` special-cases an invalid key of length 43 and replies `UNPAIRED` before normal validation. `firmware-rs/core/src/firmware.rs:2031` instead rejects an undecodable key with `PAIR INVALID`.
- **Reproduction:** The new `every_reply_matches_the_boxs_and_so_does_the_list_after_a_restart` test fails for:

  ```json
  {"version":1,"event":"device.unpair","key":"AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAg!"}
  ```

  XGGF returns `UNPAIRED <key>`; the shared firmware returns `PAIR INVALID`.
- **Impact:** Malformed requests report success without removing a pairing. This contradicts the claimed protocol parity and leaves the new test suite failing. It does not demonstrate a failure of valid-key pairing or USB transport.
- **Recommended correction:** Remove the invalid-key success special case and use the common key validation, then rerun the entire parity script. The failing test stops at this case, so later cases in that script are not yet verified in this run.

### 3.5 Linux setup now addresses port permissions more effectively

- **Evidence:** `packaging/linux/serial-access.sh:38` checks the daemon's inherited groups; line 60 reloads udev and retriggers matching devices. The script offers installation of the shipped rule, retains group-based guidance, and explains the seat-access limitation for SSH sessions.
- **Evidence:** `packaging/linux/install.sh:91` invokes the helper. `packaging/linux/make-tarball.sh:21` ships both the helper and rule. `packaging/linux/70-vibebuddy.rules` grants `uaccess` to the supported USB IDs.
- **Interpretation:** The work addresses a Linux access problem, without changing the serial protocol or requiring Wi-Fi. Tests cover the installer and permission decision paths using stand-ins; real udev/ACL behavior was not exercised here. macOS continues to use its existing accessory-permission path.

### 3.6 Validation still needs stronger automation and physical evidence

- **Evidence:** `.github/workflows/ci.yml:31` runs the root workspace tests, but `firmware-xggf/link/Cargo.toml:7` declares its own workspace. No explicit XGGF Rust or C storage test invocation was found in that workflow or `tools/test-linux-host.sh`. The Python serial-access tests are covered by CI's tools test discovery.
- **Evidence:** `firmware-xggf/tests/daemon_smoke.py:27` waits for board identification, then at line 41 asserts only HTTP `202` for an event. It neither requires the device's corresponding `STATE task.start` reply nor asserts `network == false`.
- **Interpretation:** A green root CI run can miss the reproduced parity failure. The physical daemon smoke is insufficient on its own to certify bidirectional USB-only communication. Add the standalone Rust suite and C storage harness to CI, and strengthen physical acceptance around device replies and transport identity.

## 4. Verification Results

| Check | Result |
|---|---|
| Daemon serial transport tests, `cargo test --offline --locked -p vibebuddyd serial_transport -- --test-threads=1` | 26 passed |
| XGGF bridge unit tests | 7 passed, including USB preemption and network pairing refusal |
| XGGF pairing integration tests | 8 passed, 1 failed as detailed above |
| C `pair_store_test.c` linked against the freshly built Rust static library | All 10 scenarios passed |
| `python3 -B -m unittest tools.test_serial_access -v` | 20 passed |
| Scoped `git diff --check` | Passed |
| Fresh ESP32-C3 build, flash, physical USB test, real udev ACL test | Not performed |

The XGGF tests were built offline with their lockfile into `/tmp/vibebuddy-usb-assessment`. The C harness used `-std=c11 -Wall -Wextra -Werror` and the actual Rust static library. The daemon tests initially encountered the sandbox's read-only external build cache; the approved host retry passed.

## 5. Gaps & Uncertainties

| Gap | Impact | Follow-up |
|---|---|---|
| No serial device node was visible in this assessment's sandbox | No fresh hardware certification; visibility does not establish whether hardware is plugged into the host | Run physical acceptance with host port access and `DEVICE.lock` held |
| No fresh target build or resource measurements | Host tests do not prove C3 linking, memory headroom, or boot stability | Build the complete firmware and inspect resources before deploying |
| No complete Wi-Fi-unavailable acceptance run on the updated firmware | Existing historical USB measurements do not certify the new storage path | Test without an available AP and verify both directions over USB |
| No transport policy disabling network fallback | Cable removal can permit a network link | Only if strict USB exclusivity is required, add an explicit policy with Mac/Linux parity |

## 6. Conclusions

The architecture supports the requested Wi-Fi-independent USB link, and the intervening changes improve pairing and Linux setup. The work is not ready for an unconditional validation sign-off because the new parity suite has a reproducible failure and the updated firmware has not been physically tested in this assessment.

After correcting the reply mismatch, acceptance should use the installed app's daemon, with no competing port owner and `DEVICE.lock` held. Without an available AP, require `/v1/status` to report the actual USB port, `network: false`, `relay: false`, and the current XGGF build. Send a command and wait for the device's corresponding reply, verify a device-origin button event, then check daemon restart and cable reconnect. Confirm that pairing persists across a device reboot. HTTP acceptance alone is insufficient evidence of delivery. Preserve existing Wi-Fi credentials unless changing them is explicitly part of the test.
