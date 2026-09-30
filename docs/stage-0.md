# Stage 0 — Hardware Probe record

Updated: 2026-09-14 (Asia/Singapore)

## Current conclusion

Stage 0 **has passed**. The development host, the USB plug/unplug diff, the runtime serial port and the ROM download link have been confirmed on the device; the chip, Flash/PSRAM capacity, PCB revision and the board identifier of the currently running firmware are also confirmed.

Photos of the hardware taken on 2026-09-14 show the PCB silkscreen `V1.1`, an ATK-MWS3S `N16R8` module, `B0/K1/K2`, the `USB-SLAVE`, `HOST` and `UART` ports, plus a microphone, speaker, buzzer and TF card slot. Combined with the `atk-dnesp32s3-box` identifier the current firmware reports, the device is identified as the **ALIENTEK ATK-DNESP32S3-BOX V1.1**, not the DNESP32S3 development board, BOX0, BOX2 or BOX3.

The vendor schematic/BSP for this older BOX V1.1 still has not been obtained from the current official site or GitHub organization. This gap does not block Stage 1, which uses only the chip's native USB Serial/JTAG and touches no peripheral GPIOs; it still blocks the LCD, audio, buzzer and button implementation. Until vendor evidence is in hand, the GPIOs for these peripherals stay unfrozen.

The minimal USB prerequisites for Stage 1 are met: a single USB-C cable can supply power, read runtime logs and get `esptool` into the ESP32-S3 ROM download link. "Download link confirmed" here does not mean "Vibe Buddy firmware flashed"; this stage neither erased nor wrote Flash.

## Host identity

| Item | Measured result |
| --- | --- |
| Hostname | `Michaels-Mac-Studio.local` |
| Model Name | Mac Studio |
| Model Identifier | `Mac14,14` |
| Chip | Apple M2 Ultra |
| Architecture | `arm64` |
| macOS | 26.6.2 (25G83) |

The host meets the prerequisite "create the project only on the Mac Studio".

## Development environment

### Initial probe

| Tool | Initial state | Initial version/path | Impact on the current stage |
| --- | --- | --- | --- |
| Homebrew | Installed | 6.0.22, `/opt/homebrew/bin/brew` | Enough to install dependencies |
| Python 3 | Installed | 3.14.5, `/opt/homebrew/bin/python3` | Meets ESP-IDF 6.x's official minimum of Python 3.10 |
| CMake | Not installed | Not on PATH, no installed Homebrew keg | Must be added before building firmware |
| Ninja | Not installed | Not on PATH, no installed Homebrew keg | EIM/ESP-IDF prerequisite, must be added |
| Rust | Installed | 1.98.0, `/Users/dragon/.cargo/bin/rustc` | Enough for later Mac daemon development |
| Cargo | Installed | 1.98.0, `/Users/dragon/.cargo/bin/cargo` | Enough for later Rust workspace builds |
| ESP-IDF / `idf.py` | Not found | Not found on PATH, `~/esp` or `~/.espressif`, the usual locations | Must be installed before building Stage 1 firmware |
| `esptool` / `esptool.py` | Not found | Neither the command nor the current Python environment has it | Must come from the chosen IDF environment before chip identification/flashing |

### Installed and verified

Following Espressif's current official macOS installation route, the following were installed via Homebrew:

- CMake 4.4.3
- Ninja 1.13.2
- dfu-util 0.11
- libslirp 4.9.4
- ESP-IDF Installation Manager (EIM) 0.19.0

While installing EIM, Homebrew auto-updated from 6.0.22 to 7.0.0 and required explicit trust for third-party taps. Trust was granted only to Espressif's official `espressif/eim` tap; no other tap was touched or trusted.

The device log shows the current firmware was built with ESP-IDF v5.5; the `xiaozhi` v1.9.4 source for the same firmware requires ESP-IDF 5.4 or later. On these two grounds, EIM was used to install the pinned stable ESP-IDF v5.5.3, rather than tracking `master` or switching to 6.x.

The install printed warnings about a missing `compote cooking` command and failed component cache downloads, so the installer's success message was not taken as acceptance. After re-activating the environment, independent verification gave:

| Tool | Measured version/path |
| --- | --- |
| ESP-IDF | v5.5.3, `/Users/dragon/.espressif/v5.5.3/esp-idf` |
| `idf.py` | ESP-IDF v5.5.3 |
| `esptool` | v4.12.0 |
| Xtensa GCC | 14.2.0_20251107 |
| IDF CMake | 3.30.2 |
| IDF Ninja | 1.12.1 |

EIM generated an `eim_config.toml` in the repository root containing absolute paths on this machine. It is local install state, not project build configuration; it has been added to `.gitignore` and is not committed.

Official sources:

- [ESP-IDF v6.0 macOS installation guide](https://docs.espressif.com/projects/esp-idf/en/stable/esp32p4/get-started/macos-setup.html)
- [EIM official docs and macOS Homebrew installation](https://docs.espressif.com/projects/idf-im-ui/en/latest/)
- [EIM prerequisites](https://docs.espressif.com/projects/idf-im-cli/en/latest/prerequisites.html)
- [ESP-IDF v5.5.3 official release](https://github.com/espressif/esp-idf/releases/tag/v5.5.3)

## USB baseline, not connected

After the user confirmed the board was not yet connected to the Mac Studio, `.probe/baseline` was saved. In that snapshot:

- `system_profiler SPUSBDataType` exits with code 0 but returns empty output on macOS 26.6.2.
- `ioreg -p IOUSB -l -w 0` returns the USB tree normally.
- `/dev/cu.*` and `/dev/tty.*` show only Bluetooth, Bose QC Earbuds and system debug nodes.
- No device or serial port with ESP, CP210, CH34, FTDI, USB Serial/JTAG or CDC in its name was seen.

This is the formal baseline for the board being disconnected, but it still says nothing about which USB implementation the target board uses. Raw `ioreg` output contains constantly changing statistics counters; the probe script therefore also produces a normalized summary that keeps only the device tree, VID/PID, product name, vendor name, serial number and location ID. Later diffs rely primarily on the summary, with the raw output as supporting evidence.

## USB diff after connecting

On 2026-09-14, after the user connected the board to the Mac Studio with the USB-C cable planned for use, `.probe/connected` was saved and compared with the baseline. An iPhone also appeared during the connection; it did not match the ESP/serial filter and was excluded from the target device judgment.

| Item | Measured result |
| --- | --- |
| USB product | `USB JTAG/serial debug unit` |
| Manufacturer | Espressif |
| VID:PID | `303A:1001` (decimal `12346:4097`) |
| USB serial | `98:88:E0:06:8B:CC` |
| Location ID | `138412032` |
| Runtime callout device | `/dev/cu.usbmodem8401` |
| Runtime tty device | `/dev/tty.usbmodem8401` |
| USB implementation | ESP32-S3 native USB Serial/JTAG; not a CH340, CP210 or FTDI bridge |

`system_profiler SPUSBDataType` still returns empty output with exit code 0 on this machine, so this conclusion rests on the `ioreg` plug/unplug diff together with the newly added serial device node.

## Runtime log and ROM download link

Opening `/dev/cu.usbmodem8401` at 115200 baud caused a `USB_UART_CHIP_RESET` on the device, triggered by opening the native USB serial port. This is not a fully passive read, but no Flash was erased or written. The boot log confirms:

- ESP32-S3 ROM identifier `esp32s3-20210327`, chip revision v0.2.
- 16 MB QIO Flash, 80 MHz.
- AP 64 Mbit (8 MB) Octal PSRAM, 80 MHz.
- Current application project `xiaozhi`, version 1.9.4, ESP-IDF v5.5.
- Current firmware board identifier `atk-dnesp32s3-box`.
- The current firmware successfully initialized LCD/LVGL, the ES8311 codec and Wi-Fi; these logs only prove the existing firmware can drive the device, not that Vibe Buddy can reuse all of its board parameters.

A read-only `esptool flash_id` probe followed:

- Connected successfully to an ESP32-S3 QFN56 revision v0.2.
- Confirmed USB mode `USB-Serial/JTAG`, 40 MHz crystal, 8 MB embedded PSRAM.
- Flash manufacturer/device `68:4018`, detected capacity 16 MB, 3.3 V.
- RAM stub upload, switch to 460800 baud and hard reset all succeeded.

This probe proves the ROM download path works, but no erase, write or Vibe Buddy firmware flash was performed. Re-enumeration after an actual write and runtime communication remain part of Stage 1 on-device acceptance.

## Corroboration from the current firmware source

The `xiaozhi` 1.9.4 and board identifier in the boot log map to a pinned source version of that firmware. Its `atk-dnesp32s3-box` board directory records configuration such as ST7789 i80, XL9555 and ES8311, which can be used to cross-check the hardware next. But it is the upstream source of the currently running firmware, not ALIENTEK's official schematic/BSP, and cannot on its own serve as the final GPIO source.

- [`xiaozhi-esp32` v1.9.4 pinned commit](https://github.com/78/xiaozhi-esp32/tree/3ced7709c65a39494f5684e99111854a5bcbd8c7)
- [`atk-dnesp32s3-box/config.h` in that version](https://github.com/78/xiaozhi-esp32/blob/3ced7709c65a39494f5684e99111854a5bcbd8c7/main/boards/atk-dnesp32s3-box/config.h)

## Stage 0 gate

- [x] User-confirmed USB baseline with the board disconnected.
- [x] Snapshot after connecting and the USB diff before and after plugging in.
- [x] VID/PID and device name.
- [x] Evidence of native USB versus a USB-UART bridge.
- [x] ROM download path and the current firmware's runtime serial path; must be re-verified after actually flashing Vibe Buddy.
- [x] Full PCB model/revision: ATK-DNESP32S3-BOX V1.1.
- [x] Collected the two candidate sets of official material, DNESP32S3 development board and BOX3, and made clear they must not be mixed.
- [x] Determined the device revision; the vendor schematic, BSP and examples are still missing and continue to block the peripheral stages.
- [ ] Official pin/part evidence for the LCD, touch, audio, microphone, buzzer and K0/K1/K2.
