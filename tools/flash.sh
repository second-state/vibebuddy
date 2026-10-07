#!/usr/bin/env bash
# Flash the Rust firmware over the native USB port. The three images come from tools/build-firmware.sh;
# the voices partition and the settings area are left alone, so a firmware change keeps the voice.
# With only the UART bridge connected, use tools/flash-bridge.sh. To go back to the C firmware: tools/flash-c.sh.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="${BUILD_DIR:-${repo_root}/firmware-rs/device/build}"
serial_port="${1:-}"

if [[ -z "${serial_port}" ]]; then
    shopt -s nullglob
    ports=(/dev/cu.usbmodem*)
    shopt -u nullglob
    if [[ ${#ports[@]} -ne 1 ]]; then
        echo "Expected exactly one /dev/cu.usbmodem* device; pass the port explicitly." >&2
        exit 1
    fi
    serial_port="${ports[0]}"
fi

# espflash write-bin takes a single address per connection, and chaining connections with --after no-reset
# can wedge the native USB port (see LESSONS.md), so the write goes through esptool.
if ! command -v esptool >/dev/null 2>&1; then
    echo "esptool not found on PATH; install it (pip install esptool) or activate an ESP-IDF environment." >&2
    exit 1
fi

"${repo_root}/tools/build-firmware.sh"
# One connection writes all three images and resets once at the end, so a half-updated firmware never runs.
# The bootloader header already says 16 MB; keep the flash settings rather than letting esptool patch them.
esptool --chip esp32s3 --port "${serial_port}" --before default-reset --after hard-reset \
    write-flash --flash-mode keep --flash-freq keep --flash-size keep \
    0x0 "${build_dir}/bootloader.bin" \
    0x8000 "${build_dir}/partition-table.bin" \
    0x10000 "${build_dir}/vibebuddy-fw.bin"
