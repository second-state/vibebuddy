---
status: accepted
---

# Firmware moves to an OTA partition layout with rollback, which costs every box one last USB flash

A box reached only over Wi-Fi must still get firmware updates, and the Relay is meant to carry them. The single `factory` app in `firmware/partitions.csv` was a choice, not a hardware limit: 16 MB of flash, about 6 MB used. We decided to switch to `ota_0` and `ota_1` (4 MB each) plus `otadata`, keeping `voices` where it is (0x410000) so a box keeps its Character pack across the switch, and to ship our own ESP-IDF bootloader built with app rollback enabled. The firmware writes the other slot with `esp-bootloader-esp-idf`'s `OtaUpdater` and confirms a new image early in boot; one that doesn't is rolled back. Firmware, the only thing written over the air, travels over any way of carrying the link, the Relay included, and still needs the user's confirmation (ADR-0010).

## Considered options

- **Keep the factory layout and update over USB only.** Rules out boxes away from the computer.
- **The espflash bootloader as is.** It picks OTA slots but has rollback off, so an image that crashes before marking itself invalid boot-loops.
- **Switch later, together with Wi-Fi.** Every box shipped in between would need the same USB migration.

## Consequences

The bootloader and partition table can't be safely rewritten over the air (ESP-IDF calls it unsafe, and the ESP32-S3 has no recovery bootloader), so every box already shipped needs one USB flash that writes the new bootloader, the new table and an erased `otadata`. After that they never change. The switch ships before Wi-Fi does. A new image gets one boot to confirm itself, and losing power during it also rolls back. `esp-bootloader-esp-idf` and `esp-storage` are 0.x, outside esp-hal's stability promise. Whether the C fallback firmware still runs on the new table must be checked.
