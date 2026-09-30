# Vibe Buddy Stage 0: ALIENTEK ESP32-S3 hardware fact check

> Research date: 2026-09-13  
> Evidence scope: the ALIENTEK website, official wiki and official GitHub, plus Espressif's official docs and repositories. Third-party projects appear only under "Unofficial implementation references" and are not used to confirm any hardware fact.

## Conclusion first

At this point, **"ALIENTEK ESP32S3-BOX / ATK-MWS3S" alone is not enough to identify the carrier board, nor to freeze the GPIOs or the BSP**.

- ALIENTEK officially lists the "ESP32S3 development board" and the "ESP32S3 BOX" as two separate products, yet both use the ATK-MWS3S; ATK-MWS3S therefore identifies the module, not a unique carrier board. [ALIENTEK official article](https://www.cnblogs.com/zdyz/p/18715431)
- ALIENTEK currently has fully documented material for the **ATK-DNESP32S3B3 V1 (BOX3)**. It uses K0/K1/K2, ST7789V2, CHSC5432, ES8311, ES7210 and NS4150B; the official introduction lists a single USB Type-C OTG port and lists neither a buzzer nor a USB-A host port. [BOX3 official introduction](https://wiki.alientek.com/docs/Boards/IoT/DNESP32S3B3/start-guide/dnesp32s3-box3-introduction/)
- ALIENTEK's official `ATK-DNESP32S3-Board` repository covers the **DNESP32S3 development board**; it has not been shown to be a BSP equivalent to the BOX. That board has KEY0–KEY3, BOOT, a buzzer, ES8388, a CH340C USB-to-serial chip and native USB device/JTAG. [Official repository README](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/README.md)
- The user's description, "16 MB Flash, 8 MB PSRAM, LCD, speaker, microphone, buzzer, K0/K1/K2, TF, USB-C, USB-A Host, UART", does not fully match any of the published official materials above. In particular, no first-hand source yet closes the loop on the combination "buzzer + three buttons + USB-A Host".

The Stage 0 hardware conclusion is therefore: **the model gate has not passed**. Until we have clear photos of both sides of the carrier board, the PCB silkscreen and the port labels, we can only prepare candidate adaptation layers; no candidate board's GPIOs may be treated as facts about the actual device.

## Model ambiguity

### Three things that must not be mixed up

| Object | Official identity | Relation to the current description | Evidence boundary |
|---|---|---|---|
| ALIENTEK DNESP32S3 development board | `ATK-DNESP32S3-Board`; ATK-MWS3S, 16 MB Flash, 8 MB PSRAM | Has a buzzer, ES8388, MIC, speaker, TF, USB-to-serial and USB device/JTAG; but its buttons are KEY0–KEY3 + BOOT | Usable only as candidate material for the DNESP32S3 development board; cannot be applied directly to the BOX. [Official README](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/README.md) |
| ALIENTEK BOX3 | `ATK-DNESP32S3B3 V1`; ESP32S3-R8, 16 MB Flash | Has K0/K1/K2, a 2.4-inch touchscreen, MIC, speaker, TF and USB-C; the official introduction lists no buzzer or USB-A Host, and does not call the board ATK-MWS3S | The BOX3 pinout on this page may be used only after the actual device is confirmed to be `ATK-DNESP32S3B3 V1`. [Official introduction](https://wiki.alientek.com/docs/Boards/IoT/DNESP32S3B3/start-guide/dnesp32s3-box3-introduction/) |
| Espressif ESP32-S3-BOX / BOX-3 | Espressif's own product line | Similar name, but not an ALIENTEK board | Espressif's BSP serves only its own ESP-BOX series and cannot be used as a basis for ALIENTEK GPIOs or BSP. [Espressif esp-box](https://github.com/espressif/esp-box) · [ESP-BOX-3 in Espressif esp-bsp](https://github.com/espressif/esp-bsp/blob/master/bsp/esp-box-3/README.md) |

ALIENTEK's official 2025 article does give the older "ESP32S3 BOX" its own documentation entry, `ATK-DNESP32S3BVXX.html`, and gives the DNESP32S3 development board a separate `ATK-DNESP32S3.html`; this is further evidence that the two are not interchangeable. [Official article](https://www.cnblogs.com/zdyz/p/18715431) As of this research, the older documentation entries in that article no longer return content, so **the exact revision, schematic and BSP of the older BOX remain unconfirmed**.

### Current minimal credible judgment

1. If the PCB silkscreen reads `ATK-DNESP32S3B3 V1`, implement against the BOX3 material.
2. If the PCB silkscreen reads `ATK-DNESP32S3` and matches the official development board schematic, implement against the DNESP32S3 development board material.
3. If the silkscreen contains `ATK-DNESP32S3B`, `BOX` or another revision number that is not `B3 V1`, keep looking for the official package for that exact revision; do not fill the gaps with the BOX3 or DNESP32S3 pin tables.

## Officially confirmed: DNESP32S3 development board candidate

Everything below holds only for the development board covered by ALIENTEK's official `ATK-DNESP32S3-Board` repository. Links are pinned to official repository commit `c7434a3da5b9e6feda05added5d6a686f1c95f13` so later branch changes cannot alter the evidence.

### BSP, schematic and examples

- The official repository contains the schematic, ESP-IDF/Arduino/MicroPython examples, firmware and tools. [Repository README](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/README.md)
- The official schematic revision is `ATK_DNESP32S3 V1.2`. [Schematic PDF](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/1_docs/1_sch/ATK_DNESP32S3%20V1.2.pdf)
- The repository is not a centralized board package like those in Espressif's `esp-bsp`; its ESP-IDF examples keep the vendor drivers under each example's `components/BSP`. These can be extracted into a Vibe Buddy board adaptation layer, but we would have to settle versions, dependencies and test boundaries ourselves. [BSP directory of the I2C expander example](https://github.com/openedv/ATK-DNESP32S3-Board/tree/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/components/BSP)

### Buttons and buzzer

On this board, KEY0–KEY3 and the buzzer are not wired directly to ESP32-S3 GPIOs; they go to an XL9555 I/O expander, which uses ESP32-S3 GPIO41 (SDA) and GPIO42 (SCL). [Official schematic](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/1_docs/1_sch/ATK_DNESP32S3%20V1.2.pdf)

| Function | XL9555 bit | Electrical/software semantics | Official source |
|---|---:|---|---|
| Buzzer | P0_3 / `0x0008` | The example writes 0 for on and 1 for off, i.e. active low | [Definition](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/components/BSP/XL9555/xl9555.h) · [Example](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/main/main.c) |
| KEY3 | P1_4 / `0x1000` | Low when pressed | [Definition and scan implementation](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/components/BSP/XL9555/xl9555.c) |
| KEY2 | P1_5 / `0x2000` | Low when pressed | [Definition and scan implementation](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/components/BSP/XL9555/xl9555.c) |
| KEY1 | P1_6 / `0x4000` | Low when pressed | [Definition and scan implementation](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/components/BSP/XL9555/xl9555.c) |
| KEY0 | P1_7 / `0x8000` | Low when pressed | [Definition and scan implementation](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/09_iic_exio/components/BSP/XL9555/xl9555.c) |

Note: these official definitions do not use the names K0/K1/K2; they use KEY0–KEY3. If the actual enclosure is printed K0/K1/K2, the mapping cannot be done by button number alone.

### Audio

- The audio codec is ES8388, with an onboard MIC, speaker and headphone jack; the speaker enable is controlled by XL9555 P0_2. [Official README](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/README.md) · [XL9555 definition](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/30_music/components/BSP/XL9555/xl9555.h)
- The official music example's I2S pins are MCLK=GPIO3, BCLK=GPIO46, WS/LRCK=GPIO9, ESP→codec data=GPIO10, codec→ESP data=GPIO14. [Official I2S header](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/30_music/components/BSP/I2S/i2s.h)
- ES8388 is configured over I2C; the driver interface covers both the DAC and ADC paths. [Official ES8388 header](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/30_music/components/BSP/ES8388/es8388.h)

### LCD and touch

- The official SPI LCD example supports two panel configurations, 240×240 and 320×240, and defaults to 320×240 in its orientation; LCD CS=GPIO21, DC is routed to GPIO40 via a jumper, and SPI MOSI/SCLK/MISO are GPIO11/GPIO12/GPIO13. [LCD header](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/12_spilcd/components/BSP/LCD/lcd.h) · [LCD implementation](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/12_spilcd/components/BSP/LCD/lcd.c)
- The repository includes the ST7789VW datasheet, but the generic driver supports several sizes, so **the repository alone cannot confirm the LCD controller or resolution of the user's BOX**. [Official docs directory](https://github.com/openedv/ATK-DNESP32S3-Board/tree/c7434a3da5b9e6feda05added5d6a686f1c95f13/1_docs/2_chip_manual)
- The official `touch` example uses GT9xxx together with an RGB LCD; touch I2C is GPIO39/GPIO38 and INT is GPIO40. This shows the touch solution exists in the development board ecosystem, but does not show that the 2.4-inch screen on the user's BOX uses GT9xxx. [Official GT9xxx header](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/24_touch/components/BSP/TOUCH/gt9xxx.h)

### USB, flashing and runtime link

- The schematic connects the ESP32-S3 native USB D- / D+ to GPIO19 / GPIO20; UART0 TX/RX are GPIO43/GPIO44. [Official schematic](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/1_docs/1_sch/ATK_DNESP32S3%20V1.2.pdf)
- The development board offers two flashing paths: the CH340C USB-to-serial chip, and ESP32-S3 native USB Serial/JTAG. [Official README](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/README.md) · [Official USB-UART example notes](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/33_usb_uart/README.md)
- The official USB-UART example uses TinyUSB CDC ACM, letting the application expose a virtual serial port over native USB at runtime. [Official implementation](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/2_examples/1_ESP_IDF/1_basic_routines/33_usb_uart/main/APP/tud_usart.c)

Three things must be kept apart here: the external CH340C UART, the chip's fixed-function USB Serial/JTAG, and the TinyUSB CDC that the application starts itself. All of them can show up as "a serial port", but they differ in enumeration identity, driver, firmware lifecycle and failure recovery.

## Officially confirmed: BOX3 candidate

Everything below holds only if the actual device's silkscreen is confirmed as `ATK-DNESP32S3B3 V1`. Links are pinned to official wiki source repository commit `c6f9797b64aef2666547206b4b274017d6d28a3d`.

### Board composition

- The MCU is an ESP32S3-R8 with 16 MB external Flash; the board is powered from 5V USB. [Official introduction source](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/start-guide/dnesp32s3-box3-introduction.md)
- The screen is a 2.4-inch 320×240, 4-wire SPI ST7789V2; LCD CS=GPIO47, DC=GPIO48, SPI SCLK/MOSI/MISO=GPIO15/GPIO16/GPIO17. [Official LCD notes](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/lcd.md)
- The touch controller is a CHSC5432 at 7-bit I2C address `0x2E`. [Official touch notes](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/touch.md)
- Audio output uses ES8311, capture uses ES7210, the input is an analog microphone, and the amplifier is an NS4150B. [Official introduction source](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/start-guide/dnesp32s3-box3-introduction.md)
- Audio I2S is BCLK=GPIO38, WS=GPIO39, data out=GPIO40, data in=GPIO41, MCLK=GPIO21. [Official music notes](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/music.md)

### K0/K1/K2

| Button | Connection | Notes | Official source |
|---|---|---|---|
| K0 | ESP32-S3 GPIO0 | Active low | [Button example](https://wiki.alientek.com/docs/Boards/IoT/DNESP32S3B3/example-idf/key/) |
| K1 | AW9523B P0_0 | Read through the I/O expander | [Music example source](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/music.md) |
| K2 | AW9523B P0_1 | Read through the I/O expander | [Music example source](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/music.md) |

Neither the feature list nor the onboard resource table in the official BOX3 introduction lists a buzzer. So if the actual device turns out to have a buzzer, its pin cannot be guessed from the BOX3 docs. [Official introduction](https://wiki.alientek.com/docs/Boards/IoT/DNESP32S3B3/start-guide/dnesp32s3-box3-introduction/)

### USB, flashing and runtime link

- BOX3 has one USB Type-C OTG port, in Device mode by default, used for power, USB communication and connecting to VS Code; the official introduction lists no second USB-A Host port. [Official introduction source](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/start-guide/dnesp32s3-box3-introduction.md)
- The official USB Slave example uses the chip's native USB with TinyUSB CDC to provide a virtual serial port at runtime. [Official USB Slave notes](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/example-idf/usb_slave.md)
- The official flashing instructions use the same USB connection and allow choosing UART or JTAG download in the development environment. [Official flashing notes](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/set-up-development-environment/esp-flash-download.md)

## Espressif's official USB capability boundaries

These are ESP32-S3 chip capabilities; they do not by themselves prove that a given carrier board brings out the interface, power and jumpers the same way.

- ESP32-S3 fixed-function USB Serial/JTAG uses GPIO20 (D+) and GPIO19 (D-) and can provide a bidirectional serial port, flashing and JTAG at the same time; macOS usually exposes it as a `/dev/cu.*` device. [Espressif USB Serial/JTAG docs](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-guides/usb-serial-jtag-console.html)
- If the application reconfigures the USB pins or disables the USB Serial/JTAG controller, the port disappears; the official recovery methods include holding GPIO0 low and resetting into download mode. [Espressif USB Serial/JTAG docs](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-guides/usb-serial-jtag-console.html)
- ESP-IDF's TinyUSB device stack can implement CDC, HID, MIDI, MSC, Vendor and composite devices; CDC-ACM is an application-level USB serial implementation. [Espressif USB Device Stack docs](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-reference/peripherals/usb_device.html)
- When flashing over the chip's native USB for the first time, if automatic download does not kick in, the official procedure is to hold BOOT, press RESET once, then release BOOT. [Espressif serial connection docs](https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/get-started/establish-serial-connection.html)

For Vibe Buddy, "one USB-C cable handling power, flashing, logs and the runtime NDJSON serial link" is feasible as far as the chip goes, provided that:

1. the device's USB-C really connects to the native USB on GPIO19/20, not only to a CH340C;
2. the firmware picks one explicit link, either the USB Serial/JTAG console or TinyUSB CDC;
3. the firmware does not take over or shut down that USB controller at runtime;
4. cold boot, re-enumeration after flashing, crash reboots and BOOT recovery are all tested on macOS.

Until those tests are done, "one cable for every link" should not be written up as a confirmed capability.

## Unofficial implementation references

Neither repository in this section is official ALIENTEK or Espressif material, so **neither may be used as a basis for board type, GPIOs, BSP, schematic or electrical safety**.

### `second-state/echokit_box`

Worth borrowing:

- It organizes device firmware with Rust + ESP-IDF and selects an integrated BOX configuration through a Cargo feature; the README shows the `espflash` build/flash flow. [Project README](https://github.com/second-state/echokit_box/blob/4484efca885c2ffd01ffb1acdbb5817421583bd8/README.md)
- `atom_box.rs` gathers the screen, audio, buttons, I2C/I/O expander and so on into a single board implementation. Vibe Buddy can borrow this structure: business logic never references raw GPIOs, and board differences are encapsulated in a board profile. [Board implementation](https://github.com/second-state/echokit_box/blob/4484efca885c2ffd01ffb1acdbb5817421583bd8/src/boards/atom_box.rs)
- Its concurrency layout for the framebuffer, audio task and button task is worth referencing, but timing, buffers and pins must be re-verified against the target board's official material and on-device tests.

Not usable as a source: the HAL drivers the project bundles or modifies, and the I2C, I2S, LCD and I/O expander pins in `atom_box.rs`. Even where comments mention ALIENTEK or ESP32S3 BOX, these are only a third-party implementation's claims. They differ clearly from the confirmed BOX3/DNESP32S3 pins above, which is exactly why the name "ESP32S3 BOX" is not enough to identify the hardware.

### `second-state/echokit_server`

Worth borrowing: the device/server layering, WebSocket sessions, and the engineering organization of ASR→LLM→TTS orchestration with configurable providers. [Project README](https://github.com/second-state/echokit_server/blob/d1d976596f122976095b7da4df3e946baf152b96/README.md)

Not usable as a source: this is a higher-level voice agent server and provides no official hardware evidence for ALIENTEK carrier boards. If Vibe Buddy v1 uses a local daemon + USB NDJSON, WebSocket and a full cloud voice pipeline should not be brought in without validated requirements.

## Pending confirmation on the device / silkscreen

Collect all of the following evidence in one go, so we stop guessing the board from its product name:

1. Take high-resolution photos of both sides of the main board, opened up or through the vents; the PCB model, revision and date must be legible, e.g. whether it is `ATK-DNESP32S3B3 V1`.
2. Photograph the silkscreen on the module shield to confirm the exact `ATK-MWS3S` revision; also record the Flash/PSRAM lines from an on-device boot log.
3. Photograph the silkscreen next to every port clearly: USB-C, USB-A, UART, OTG, HOST, SLAVE, DOWNLOAD/JTAG.
4. Photograph the silkscreen for K0/K1/K2, the buzzer, MIC, speaker, TF and the LCD ribbon cable/daughterboard clearly.
5. On macOS, plug in each USB port separately and save the changes in `system_profiler SPUSBDataType`, `ioreg -p IOUSB` and `/dev/cu.*`; do not judge Host/Device from the connector shape alone.
6. Save the full ROM/bootloader log at power-on, recording chip revision, Flash, PSRAM, USB enumeration identity and firmware identifier.

Until the device is confirmed, keep the following items **unconfirmed**:

- exact carrier board model and hardware revision;
- LCD controller, resolution, bus and backlight control;
- touch controller, I2C address, interrupt/reset lines;
- codec, ADC, amplifier, MIC type and all I2S/I2C pins;
- whether the buzzer exists, whether it is wired directly or via an I/O expander, and its active level;
- whether K0/K1/K2 are direct GPIOs or expander bits;
- which link the USB-C port is: CH340C, USB Serial/JTAG, or USB OTG/TinyUSB;
- whether the USB-A port really supports Host, its VBUS power capability and overcurrent protection;
- whether the official BSP/documentation package has a separate version for the older BOX.

## ESP-IDF version selection gate

- **If confirmed as BOX3**: ALIENTEK's official environment docs explicitly require ESP-IDF `v5.3.x` or later for its ESP32-S3 examples, and the current documentation package ships a `v5.5.3` offline installer. [Official BOX3 installation notes](https://github.com/openedv/openedv-wiki-boards-dnesp32s3b3/blob/c6f9797b64aef2666547206b4b274017d6d28a3d/set-up-development-environment/esp-idf-install.md)
- **If confirmed as the DNESP32S3 development board or the older BOX**: the official DNESP32S3 repository we found does not declare a reproducible minimum or pinned ESP-IDF version, and the exact package for the older BOX has not been obtained. So BOX3's `>=5.3.x` requirement cannot be extrapolated to them. [Official DNESP32S3 development notes](https://github.com/openedv/ATK-DNESP32S3-Board/blob/c7434a3da5b9e6feda05added5d6a686f1c95f13/1_docs/Developing_With_ESP_IDF.md)
- **Vibe Buddy's selection rule**: do not track `latest` or `master`. First pick a pinned release tag within the exact board's official minimum supported line; a candidate version can be locked only after both the vendor's original examples and Vibe Buddy's minimal probe pass "build, flash, reboot, USB re-enumeration, LCD, touch, buttons, audio capture and playback". Espressif also explicitly recommends that projects depending on ESP-IDF follow that project's own compatibility notes first. [Espressif ESP-IDF versions](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/versions.html)
- **Current recommendation**: if the device is BOX3, use `v5.5.3` from the official material as the first reproduction baseline, then separately verify the latest bugfix tag in the same `release/v5.5` series; do not move to ESP-IDF 6.x without a full regression. If it is not BOX3, do not freeze the IDF version until the matching documentation package is in hand.

## Suggested next steps

1. **Pass the model gate first**: based on the PCB silkscreen and port photos, classify the device as BOX3, the DNESP32S3 development board, or "another older BOX".
2. **Get the exact official package**: if it is not BOX3, ask ALIENTEK technical support, citing the silkscreen model, for the schematic, examples and factory firmware of that exact revision; do not accept a similar model as a substitute.
3. **Do a read-only enumeration**: on the Mac Studio, record each USB port's VID/PID, product name, serial node and re-enumeration behavior, then decide on the flashing/runtime link.
4. **Build a minimal hardware probe firmware**: verify only serial, the three buttons, buzzer, LCD color blocks, touch coordinates, MIC level, speaker sine wave and TF mount; for each item, start with the candidate BSP, then cross-check with a logic analyzer/oscilloscope and on-device behavior.
5. **Set up board isolation**: the code must distinguish at least `dnesp32s3_devboard`, `dnesp32s3_box3` and `unknown_legacy_box`, and must refuse to pick a default GPIO table while the model is unconfirmed.
6. **Freeze the Vibe Buddy interface last**: only after the hardware probe passes, settle the flashing, logging and NDJSON lifecycle over the single USB cable, and transcribe the verified pin table into `docs/hardware.md`.
