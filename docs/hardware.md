# VibeBuddy hardware notes

This document records only hardware facts with an evidence boundary. Similar names or boards from the same series cannot serve as the basis for GPIO, codec, LCD, or USB paths.

## Provided by the user, not yet verified against the hardware or a schematic

- Name: ALIENTEK ESP32S3-BOX.
- Module: ATK-MWS3S / ESP32-S3.
- Resources: 16 MB flash, 8 MB PSRAM.
- Peripherals: LCD, speaker, microphone, buzzer, K0/K1/K2, TF/microSD, USB-C, USB-A host, UART.
- Expectation: a single USB-C cable handles power, flashing, and runtime communication at once.

Using the same USB-C cable for power, the runtime serial port, ROM download, writing the VibeBuddy firmware, and re-enumeration after writing has been verified in practice.

## Current host confirmed

- Hostname: `Michaels-Mac-Studio.local`.
- Model: Mac Studio, Model Identifier `Mac14,14`, Apple M2 Ultra.
- Evidence: `hostname` and `system_profiler SPHardwareDataType` run on this machine on 2026-09-13.

## Confirmed from official sources

Detailed sources and differences between candidate boards are in [`hardware-research.md`](hardware-research.md). What can currently be confirmed:

- `ATK-MWS3S` is a module identifier and isn't enough to identify the carrier board; both ALIENTEK's DNESP32S3 development board and the old ESP32S3 BOX may use this module.
- The official `ATK-DNESP32S3-Board` repository corresponds to the DNESP32S3 development board, not a general BSP for the BOX. What it publishes is the `ATK_DNESP32S3 V1.2` schematic, KEY0–KEY3 + BOOT, ES8388, XL9555, CH340C, and two flashing paths: CH340C and native USB Serial/JTAG.
- What the current official Wiki fully covers is the `ATK-DNESP32S3B3 V1` (BOX3): K0 wired directly to GPIO0, K1/K2 via an AW9523B, a 320×240 ST7789V2 screen, a CHSC5432 touch controller, an audio chain including ES8311, ES7210, and NS4150B, and published native USB/TinyUSB material.
- The hardware combination the user gave doesn't fully match any of the candidates above. The official documentation entry point for the old BOX currently can't be read, and BOX3 or DNESP32S3 development board pinouts can't be used to fill the gaps.

Therefore, no BSP is chosen and no GPIOs are frozen until the PCB silkscreen is confirmed. ESP-IDF v5.5.3 was installed based on the v5.5 build info of the firmware currently on the hardware and the requirements of the corresponding upstream source; that choice does not mean any candidate board's GPIO definitions have been accepted.

## Confirmed on the hardware

- After the user confirmed the board was not yet connected, an unconnected USB baseline of the Mac Studio was saved on 2026-09-13; the baseline showed no ESP32, no common USB-UART bridge, and no new USB modem serial port.
- After connecting the board on 2026-09-14, a new Espressif `USB JTAG/serial debug unit` appeared, VID:PID `303A:1001`, USB serial `98:88:E0:06:8B:CC`.
- The new nodes are `/dev/cu.usbmodem8401` and `/dev/tty.usbmodem8401`, proving the target board currently enumerates via the ESP32-S3's native USB Serial/JTAG rather than an external CH340/CP210/FTDI bridge.
- The runtime log confirms ESP32-S3 revision v0.2, 16 MB QIO flash, 8 MB Octal PSRAM, and the current firmware's board identifier `atk-dnesp32s3-box`.
- A read-only `esptool flash_id` successfully connected over the ROM download path and rechecked the 16 MB flash, 8 MB embedded PSRAM, and USB-Serial/JTAG mode; flash was neither erased nor written.
- Opening the native USB serial port triggers `USB_UART_CHIP_RESET`, so the runtime reconnect design must tolerate device resets and re-enumeration.
- A photo of the back taken on 2026-09-14 shows the PCB silkscreen `V1.1`, an ATK-MWS3S `N16R8` module, `B0/K1/K2`, `USB-SLAVE`, `HOST`, `UART`, the microphone, speaker, buzzer, and TF card slot.
- The physical layout together with the current firmware's self-reported identifier confirm the board is the **ATK-DNESP32S3-BOX V1.1**; it is not the DNESP32S3 development board, BOX0, BOX2, or BOX3.
- In Stage 2, after physically unplugging `USB-SLAVE`, `vibebuddyd` logged `Device not configured`; after plugging it back in, it automatically rediscovered the same serial node and resumed communication.
- After fully cutting USB power, the leftover screen from the original xiaozhi firmware disappeared and the screen stayed black; this proves the old image was residual LCD state, not a sign the old firmware was still running, nor proof that VibeBuddy already had an LCD driver.
- The LCD is now driven by VibeBuddy on the hardware and has passed visual acceptance: 320×240 ST7789, 8-bit i80, bus data on GPIO40/39/38/12/11/10/9/46, CS/DC/RD/WR on GPIO1/2/41/42, backlight controlled by XL9555 P0.7.
- The ES8311 was detected over I2C at 7-bit address `0x18`; audio I2S BCLK/WS/DOUT are GPIO21/13/14, sample rate 24 kHz, and speaker enable is controlled by XL9555 P0.5.
- Firmware startup has reported `AUDIO READY` and `AUDIO CODEC ES8311`; the user has actually heard the "需要你确认" ("I need you to confirm") voice line.
- On 2026-09-14 the disconnection indicator was verified on the hardware: after stopping `vibebuddyd` for more than 15 seconds, the device closed its eyes, the screen went gray, and it showed `NO LINK`, with the task cards kept but also grayed out; after the daemon was restored it left that state automatically. Confirmed by the user on the hardware.
- On 2026-09-14 announcement loudness was raised: the voice assets were normalized with a uniform gain to about 90% of full scale (+5.5 dB, no clipping), and the codec output volume was raised from 45 to 65. The user confirmed on the hardware that the volume is now sufficient.
- On 2026-09-14 a hardware probe confirmed K2 is wired to XL9555 P0.3, active-low: `P0=0xFF` when released, `P0=0xF7` when pressed, then back to `0xFF`, with P1 staying at `0xFF` throughout. After writing the production firmware, when the user short-pressed K2, the device reported a K2 press event and the Mac-side `vibebuddyd` brought up the target app; the fix for precise routing from Codex sub-agents to the parent session still awaits one final hardware confirmation.
- On 2026-09-15 the user confirmed the buttons on the case are K0, K1, K2, and RST. `B0` on the PCB silkscreen corresponds to K0 on the case: it is the ESP32-S3's BOOT button, wired directly to GPIO0, active-low, with an internal pull-up. The basis is that the same board's upstream firmware `xiaozhi` defines `BOOT_BUTTON_GPIO GPIO_NUM_0` in `atk-dnesp32s3-box/config.h`; a press on the hardware the same day confirmed it: after pressing K0 the device reported `POMODORO PAUSED` / `RESUMED`. Pressing K0 at runtime doesn't affect the boot mode; only holding it at the instant of reset enters download mode. `vibebuddyd` opening the serial port triggers a reset, and the chance of K0 being held at exactly that instant is negligible, but it's worth knowing this can happen.
- On 2026-09-15 a hardware probe confirmed K1 is wired to XL9555 P0.4, active-low: `P0=0xEF` when pressed, back to `0xFF` on release, with P1 always `0xFF`. The probe's candidate range came from the direction register writes in the same board's upstream firmware `atk_dnesp32s3_box.cc` (`0x06=0x1B`, `0x07=0xFE`, i.e. P0.0, P0.1, P0.4, and P1.1–P1.7 as inputs); on the hardware only P0.4 changed with K1.

## Corroboration from the currently running firmware

The hardware runs `xiaozhi` 1.9.4. The source at that pinned version contains an `atk-dnesp32s3-box` board directory that records the ST7789 i80, XL9555, ES8311, and a set of board-level pins. This corroborates the boot log and can be used as a candidate for cross-checking, but that repository is not ALIENTEK's official schematic/BSP and cannot bypass the PCB revision gate to freeze pins directly.

- [`xiaozhi-esp32` v1.9.4 pinned commit](https://github.com/78/xiaozhi-esp32/tree/3ced7709c65a39494f5684e99111854a5bcbd8c7)
- [`atk-dnesp32s3-box/config.h`](https://github.com/78/xiaozhi-esp32/blob/3ced7709c65a39494f5684e99111854a5bcbd8c7/main/boards/atk-dnesp32s3-box/config.h)

## Pending verification

- Vendor schematic, BSP, and examples matching the exact PCB revision.
- Touch controller (if any).
- Microphone input chain.
- Buzzer GPIO and active level.
