# Stage 1 — Serial Hello acceptance record

Accepted: 2026-09-14 (Asia/Singapore)

## Conclusion

Stage 1 **has passed on-device acceptance**. The evidence chain is Mac Studio → `/dev/cu.usbmodem8401` → ESP32-S3 native USB Serial/JTAG → `vibebuddy-fw` → cJSON parse → result returned over USB; not a local simulation, and not merely a successful build.

This stage did not initialize the LCD, audio, microphone, buzzer or buttons. After flashing, the LCD kept showing the Wi-Fi setup page last left by the original Xiaozhi firmware. This does not mean the old firmware is still running: the LCD controller's frame memory and the backlight persist across an ESP32 software reset, and the Stage 1 firmware does not overwrite the screen. Repeating Serial Hello afterwards still passed, which directly proves the VibeBuddy firmware is what is running. Clearing the LCD and the new UI must wait for Stage 3 and vendor hardware evidence.

## Build

- ESP-IDF: v5.5.3.
- Target: `esp32s3`.
- Firmware project name: `vibebuddy-fw`.
- Application image: 183,616 bytes (`0x2cd40`).
- Default application partition: 1 MiB, 82% free.
- The first build uncovered and removed an invalid Kconfig symbol and a C qualifier warning; the rebuild has no compiler warnings.

Build and flash entry point:

```bash
./tools/flash.sh /dev/cu.usbmodem8401
```

## Factory firmware backup

Before writing VibeBuddy, the full 16 MiB Flash was read out with `esptool read_flash`:

- Local file: `.probe/factory/atk-dnesp32s3-box-v1.1-xiaozhi-1.9.4-2026-09-14.bin`
- Size: 16,777,216 bytes.
- SHA-256: `7e0ae33002423eaca46a6e6c1f8cc6d92a2a04babd99fe7aa2a4ede788a01ec2`.
- Permissions: `600`.
- Git status: `.probe/` is ignored and does not enter the repository.

The full-chip backup may contain the original firmware's Wi-Fi setup or device state, so it is treated as sensitive local evidence and is not copied into docs or any remote repository.

## Flashing

`idf.py flash` wrote successfully over the same `/dev/cu.usbmodem8401`:

| Offset | Content | Bytes written |
| --- | --- | --- |
| `0x0000` | bootloader | 20,832 bytes |
| `0x8000` | partition table | 3,072 bytes |
| `0x10000` | `vibebuddy-fw` | 183,616 bytes |

esptool reported hash verified for all three segments and then performed a hard reset; the device re-enumerated as `/dev/cu.usbmodem8401` again.

## Mac → USB → ESP32 acceptance

Run:

```bash
uv run --with pyserial python tools/serial-hello.py /dev/cu.usbmodem8401
```

What the Mac actually sent:

```json
{"version":1,"event":"task.done","title":"Hello"}
```

What the ESP32 actually returned:

```text
EVENT task.done
TITLE Hello
PASS Stage 1 Mac -> USB -> ESP32-S3 -> JSON parse
```

After the user observed that the old Xiaozhi screen was still showing, the same command produced exactly the same PASS again, ruling out "the flash didn't take, or the old firmware is still running". Hello has no `id`, which verifies in practice that Stage 1 accepts an optional `id`. NDJSON framing, version policy, unknown fields/events and the input limit are covered in [`protocol.md`](protocol.md).

On the same device, invalid JSON, `version: 2`, `custom.event` with unknown fields, CRLF framing and a 1025-byte oversized input were also tested, giving in order:

```text
ERROR invalid_json
ERROR unsupported_version
EVENT custom.event
ERROR input_too_large
PASS protocol error, version, extension, CRLF, and size handling
```

## Not proven

- This acceptance does not prove that VibeBuddy can drive the LCD, audio, buzzer, buttons or TF card.
- This acceptance did not implement `vibebuddyd`, the HTTP API or serial reconnect; those belong to Stage 2.
- The serial node number may change with macOS enumeration, so the future daemon must not hard-code `usbmodem8401`; it should discover the device by VID/PID and USB serial.
