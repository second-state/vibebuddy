# Muse next to Vibe Buddy

You can put Meta's [Muse Gadget](https://github.com/facebookincubator/muse-gadget-sdk) firmware on your box next to Vibe Buddy. Both stay on the box. You choose which one starts with the K2 key.

Vibe Buddy is not part of Meta and Meta does not support this. We do not ship Muse on the box. You build it, flash it and give it your own SDK token. The token is for your own, personal use: do not share it, and do not put it in a box that you sell.

## How it works

- Muse uses free flash space. Vibe Buddy, its Character, its settings and its pairings stay where they are.
- Muse has its own partition table. It shows Muse only its own NVS and its app, so Muse can't change Vibe Buddy's data.
- The bootloader chooses the app. Neither app knows about the other.

| Partition | Offset | Size | For |
|---|---|---|---|
| `muse_tbl` | 0xc14000 | 4 KB | Muse's own partition table (`tools/muse/partitions.csv`) |
| `muse_nvs` | 0xc15000 | 48 KB | Muse's NVS: Wi-Fi, token, settings, BLE bonds |
| `boot_sel` | 0xc21000 | 4 KB | Which app starts: `MUSE` in the first word for Muse, anything else for Vibe Buddy |
| `muse` | 0xc30000 | 3.8 MB | The Muse app (`app`, subtype `test`) |

The bootloader (`firmware-rs/bootloader/bootloader_components/main/bootloader_start.c`) reads `boot_sel` at each start. When K2 is held for half a second from reset, it toggles `boot_sel`. It reads K2 only when the `muse` partition holds an image. A wake from deep sleep keeps the choice, because Muse powers off into deep sleep. When it chooses Muse, it does not read otadata, so a Vibe Buddy update waiting for its trial boot stays waiting. A damaged Muse image sets `boot_sel` back to Vibe Buddy.

When the Vibe Buddy app installs Vibe Buddy over USB, it blanks `boot_sel` with otadata, so the box starts Vibe Buddy. Updates over the link do not touch the bootloader or the partition table, so they keep Muse.

## Build

You need ESP-IDF v6.0.3 and a checkout of the official SDK. The Muse source is not changed: `tools/muse/sdkconfig.vibebuddy` only moves Muse in flash.

```sh
git clone https://github.com/facebookincubator/muse-gadget-sdk
tools/muse/build.sh muse-gadget-sdk
```

The build has no SDK token.

## Flash

1. Quit the Vibe Buddy app. It holds the box's port.
2. Connect the box with the USB-C port marked USB-SLAVE.
3. Get an SDK token at gadgets.muse.ai > Account > SDK tokens.
4. Flash Muse and send the token:

   ```sh
   tools/muse/flash.sh /dev/cu.usbmodem101 mgst_…
   ```

The box starts Muse. Pair it with the Muse app as Muse's docs say.

## Switch

- **To Vibe Buddy or to Muse:** push K2 down first and hold it. Then press and release RESET. Keep K2 down for 2 more seconds. The bootloader reads K2 only in the first second after reset, so K2 must already be down when you release RESET. Pulling the cable and plugging it in again while you hold K2 also works.
- Do not hold K0 at reset: K0 is BOOT, and it starts the chip's download mode.

## Remove Muse

Install Vibe Buddy again from the Vibe Buddy app. The box then starts Vibe Buddy. The Muse image stays in its partition, so holding K2 at reset still starts it. To remove it, erase the `muse` partition:

```sh
python -m esptool --chip esp32s3 -p PORT erase-region 0xc30000 0x3d0000
```
