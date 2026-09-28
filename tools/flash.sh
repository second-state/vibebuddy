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

"${repo_root}/tools/build-firmware.sh"
# Stay in the bootloader after the first two images and reset only after the last one, so a half-updated firmware never runs.
espflash write-bin -S --chip esp32s3 --port "${serial_port}" --after no-reset 0x0 "${build_dir}/bootloader.bin"
espflash write-bin -S --chip esp32s3 --port "${serial_port}" --after no-reset 0x8000 "${build_dir}/partition-table.bin"
espflash write-bin -S --chip esp32s3 --port "${serial_port}" 0x10000 "${build_dir}/vibebuddy-fw.bin"
