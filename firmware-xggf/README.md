# XGGF experimental firmware

This is experimental ESP32-C3 VibeBuddy firmware for the XGGF PixelClock. It displays Agent status over USB or a paired, encrypted local Wi-Fi link, provides an identify animation and short tone, and cycles through offline status, RTC time, temperature and humidity while the link is down. A `device.mic` command opens a ten-second sound-reactive buddy and ten-bar level display; Agent events take priority and only another explicit microphone command restarts the visualizer after preemption. It does not provide speech recognition or voice support. Other stock features remain under investigation. Sensor readings are used as reported without calibration. Do not use the normal box firmware installer on this device.

## Speech recording

The microphone also records speech on request: the daemon arms a bounded recording (`POST /v1/device/audio/start`), the firmware streams numbered 20 ms base64 frames over USB and the encrypted Wi-Fi link, and the daemon rebuilds the WAV in memory. From the apps' Device page this needs `VIBEBUDDY_XGGF_AUDIO=1` on this computer (record, play and save); the token-gated endpoints stay for the probe. The protocol, measurements and failure analysis are in [AUDIO_PROBE_REPORT.md](AUDIO_PROBE_REPORT.md), and the gated plan in [AUDIO_PROBE_PLAN.md](AUDIO_PROBE_PLAN.md).

## Buttons

All three buttons are active-low digital inputs with 40 ms debounce and a one-second long-press threshold. The panel shows `K0 DOWN`, `K1 DOWN` or `K2 DOWN` while a button is held and changes to `LONG` when its long press fires.

| Physical button | GPIO | Short press | Long press |
|---|---:|---|---|
| Left (K0) | 8 | Start or stop the microphone visualizer | Identify animation and tone |
| Middle (K1) | 9 | Cycle Agent status, clock and environment pages | Return to Agent status |
| Right (K2) | 2 | Send the standard `button` event; the daemon opens the current Agent source | Toggle buzzer mute |

Agent events return the display to Agent status. Identify respects mute. K2's short-press JSON event uses the same daemon path over USB and encrypted Wi-Fi.

The daemon and apps reject released box firmware for an identified unsupported board, and the ROM flasher independently requires an ESP32-S3 before writing. The XGGF reports its board before the build marker. Preserve all of these gates, including protection for an unknown or foreign board, when changing installation or recovery paths.

The panel tests in [the plan](../PLAN_XGGF_64X8.md) established the visual mapping on the examined device; see [working notes](WORK_NOTES.md). The initial test used GPIO 21 as data and showed no visible change. Inspection of the stock app, which matches this device's backup, showed that its SPI bus configuration puts GPIO 7 at MOSI offset 0 and GPIO 21 at SCLK offset 8. The current firmware uses GPIO 7 as data, leaves the SCLK pin unassigned, and caps each RGB channel at 8/255. The console is configured on native USB Serial/JTAG, not UART0. A measured waveform remains outstanding; a 15-minute runtime soak passed.

The framebuffer occupies 1,536 bytes of static RAM. The 6,324-byte SPI transfer buffer is allocated from DMA-capable memory at startup. At the nominal 3.2 MHz SPI rate the measured transfer time was stable within 15.861–15.865 ms during the soak. Minute diagnostics report heap, main-stack headroom and transfer time; the microphone uses a separate static 512-byte sample buffer plus its I2S DMA buffers.

## Before any flash

Identify the connected unit and record chip model, flash size, USB serial, boot output, eFuse security settings, and partition table. Use the appropriate `esptool` and `espefuse` commands for the installed version. The known dump describes one unit, not all units. Stop if the unit is not an ESP32-C3 with 4 MB flash or if its security settings prevent backup and restoration.

Read the entire 4 MB flash twice to a private directory outside this repository. For an esptool version supporting these commands, with the correct port substituted:

```sh
esptool --chip esp32c3 --port PORT read_flash 0 0x400000 xggf-backup-a.bin
esptool --chip esp32c3 --port PORT read_flash 0 0x400000 xggf-backup-b.bin
sha256sum xggf-backup-a.bin xggf-backup-b.bin
```

Confirm both files are exactly 4,194,304 bytes and have the same hash. They contain device-specific settings and may contain credentials. Keep them private and retain at least one separate copy. Record the actual chip, port, command and hash. Consult the installed esptool help if its command spelling differs.

The stock firmware may change NVS each time it boots. If full reads differ, identify every changed sector without displaying its contents. On the examined unit only NVS changed; the remaining regions of both full reads matched. Two identical reads of the first `0x10000` bytes with the firmware held in download mode, combined with the independently matching remainder, produced a full image that `esptool verify_flash` confirmed against the stopped device. Do not assume any other difference is harmless.

The recovery procedure for the verified, unencrypted 4 MB image is to write that unit's complete backup at offset `0x0` in ROM download mode, read the entire flash again, compare its hash with the backup, then boot and check the stock functions. For example, on a compatible esptool version:

```sh
esptool --chip esp32c3 --port PORT write_flash 0x0 xggf-backup-a.bin
esptool --chip esp32c3 --port PORT read_flash 0 0x400000 xggf-restored.bin
sha256sum xggf-backup-a.bin xggf-restored.bin
```

This procedure has **not been tested on the current target unit**. Do not proceed without a verified backup and access to ROM download mode. Flash operations replace device contents; do not erase eFuses or assume another unit's backup is suitable.

## Build and test

ESP-IDF 5.5.4 with ESP32-C3 tools and Rust's `riscv32imc-unknown-none-elf` target are required. The Espressif mDNS component is resolved by the ESP-IDF component manager; the checked-in lockfile pins the component version. From this directory after sourcing the ESP-IDF environment:

```sh
idf.py build
```

The firmware has been built with ESP-IDF 5.5.4 and tested on the examined hardware. The sibling template used ESP-IDF 5.2.x, but this project has not yet been built against that version. Its partition table preserves the original NVS area and places its own NVS in part of the former stock coredump region; it does not preserve the stock application layout. Follow the backup gate before any flash. Use a suitable power supply and watch the unit for resets or heating.

The host tests require a C compiler:

```sh
cc -std=c11 -Wall -Wextra -Werror -Ifirmware-xggf/main firmware-xggf/main/panel.c firmware-xggf/tests/panel_test.c -o /tmp/xggf-panel-test
/tmp/xggf-panel-test
cc -std=c11 -Wall -Wextra -Werror -Ifirmware-xggf/main firmware-xggf/main/panel.c firmware-xggf/main/font.c firmware-xggf/main/status.c firmware-xggf/tests/status_test.c -o /tmp/xggf-status-test
/tmp/xggf-status-test
cc -std=c11 -Wall -Wextra -Werror -Ifirmware-xggf/main firmware-xggf/main/panel.c firmware-xggf/main/font.c firmware-xggf/main/status.c firmware-xggf/tests/font_test.c -o /tmp/xggf-font-test
/tmp/xggf-font-test
cc -std=c11 -Wall -Wextra -Werror -Ifirmware-xggf/main firmware-xggf/main/buttons.c firmware-xggf/tests/buttons_test.c -o /tmp/xggf-buttons-test
/tmp/xggf-buttons-test
cc -std=c11 -Wall -Wextra -Werror -Ifirmware-xggf/main firmware-xggf/main/panel.c firmware-xggf/main/font.c firmware-xggf/main/status.c firmware-xggf/tests/preview.c -o /tmp/xggf-preview
/tmp/xggf-preview /tmp/xggf-previews
```

The status labels draw with a 5x7 font covering printable ASCII. `firmware-xggf/main/font.{h,c}` are generated from the vendored petabyt 7x5 font by `tools/make-xggf-font.py` (run it after touching `fonts/petabyt-5x7/`); the generator's tests run with the other tools unit tests, and `tests/font_test.c` checks the table through the renderer. `tests/preview.c` writes every label scenario as 1x and 8x PBM files for reviewing the panel output before flashing.

The previous panel probe and its results are described in the working notes. The current build starts in `OFFLINE` and displays Agent status once it receives events. Hold the repository-root `DEVICE.lock` for every complete flash or physical test, including data collection, and release it immediately afterward. From `firmware-xggf`, flash with `flock ../DEVICE.lock -c 'idf.py -p PORT flash'` after sourcing ESP-IDF; from the repository root, use `DEVICE.lock` as shown below. The lock file is ignored by Git.

With the device already running this firmware, these physical tests are available:

```sh
flock DEVICE.lock -c 'python3 firmware-xggf/tests/physical_serial.py PORT'
flock DEVICE.lock -c 'python3 firmware-xggf/tests/soak.py PORT 15'
flock DEVICE.lock -c 'python3 firmware-xggf/tests/microphone.py PORT'
flock DEVICE.lock -c 'python3 firmware-xggf/tests/microphone.py PORT /tmp/xggf-mic-tone.wav'
flock DEVICE.lock -c 'python3 firmware-xggf/tests/audio_roundtrip.py PORT'
flock DEVICE.lock -c 'python3 firmware-xggf/tests/wifi_smoke.py target/debug/vibebuddyd PORT CONNECTION'
flock DEVICE.lock -c 'python3 firmware-xggf/tests/buttons.py PORT action K0'
flock DEVICE.lock -c 'python3 firmware-xggf/tests/buttons.py PORT action K1'
flock DEVICE.lock -c 'python3 firmware-xggf/tests/buttons.py PORT action K2'
```

The button tests print a gesture sequence, wait up to 120 seconds and finish as soon as the required events arrive. The panel identifies the held key and shows when the long-press threshold fires. `map left|middle|right` replaces `action K0|K1|K2` for repeated physical-mapping checks.

The serial test sends every status state. The soak rotates states and checks diagnostics for the requested number of minutes. The microphone test activates the visualizer and reports only aggregate amplitude; with a WAV path it takes a baseline, plays the tone through `aplay`, and requires a clear response during a second probe. The round-trip test records six identify tones with `arecord`, checks their burst count and 1 kHz dominance, replays the recording through `aplay`, and requires the device microphone to detect it above the later baseline. Temporary recordings are deleted automatically. The identify command emits an 80 ms tone and remains covered by the serial test. The Wi-Fi test scans and provisions over USB, removes the cached address, reconnects through mDNS and drives eight encrypted controls; `CONNECTION` is an active NetworkManager connection whose credentials remain outside the repository.
