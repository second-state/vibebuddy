#!/usr/bin/env bash
# Flash through the BOX's CH343 UART bridge. On this path, with esptool's default settings the stub upload
# hits a checksum error, and --no-stub also fails on the first block and erases the flash; what works is
# --no-stub with 256-byte write blocks: slow, but every block passes verification (see LESSONS.md).
# Write the partition table alone first to test the path, then the app; the bootloader is only written when explicitly asked for.
#
# Usage: tools/flash-bridge.sh /dev/cu.usbmodemXXXX partition|app|bootloader ...
# Flashes the Rust firmware images by default (firmware-rs/device/build; run tools/build-firmware.sh first).
# Set C_FIRMWARE=1 to flash the C firmware's firmware/build instead; BUILD_DIR points at another build directory.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "${C_FIRMWARE:-0}" == "1" ]]; then
    build_dir="${BUILD_DIR:-${repo_root}/firmware/build}"
    partition_bin="${build_dir}/partition_table/partition-table.bin"
    bootloader_bin="${build_dir}/bootloader/bootloader.bin"
    otadata_bin="${build_dir}/ota_data_initial.bin"
    # flash_mode / flash_size / flash_freq come from the build output rather than being copied here by hand.
    read -r -a flash_args <<< "$(head -n 1 "${build_dir}/flash_args")"
else
    build_dir="${BUILD_DIR:-${repo_root}/firmware-rs/device/build}"
    partition_bin="${build_dir}/partition-table.bin"
    bootloader_bin="${build_dir}/bootloader.bin"
    otadata_bin="${build_dir}/ota-data-initial.bin"
    # The Rust firmware's bootloader header already says 16 MB; write it as is and don't let esptool patch it.
    flash_args=(--flash_mode keep --flash_freq keep --flash_size keep)
fi
serial_port="${1:?usage: flash-bridge.sh <serial-port> partition|app|bootloader ...}"
shift
if [[ $# -eq 0 ]]; then
    echo "give at least one target to write: partition, app or bootloader" >&2
    exit 1
fi

activation_script="${HOME}/.espressif/tools/activate_idf_v5.5.3.sh"
caller_path="${PATH}"
while IFS='=' read -r key value; do
    if [[ "${key}" == "PATH" ]]; then
        export PATH="${value}:${caller_path}"
    elif [[ "${key}" != "SYSTEM_PATH" ]]; then
        export "${key}=${value}"
    fi
done < <("${activation_script}" -e)

segments=()
for target in "$@"; do
    case "${target}" in
        partition) segments+=(0x8000 "${partition_bin}") ;;
        # The app goes to ota_0, and blanking otadata makes the box boot it rather than an older ota_1.
        app) segments+=(0x10000 "${build_dir}/vibebuddy-fw.bin" 0xa10000 "${otadata_bin}") ;;
        bootloader) segments+=(0x0 "${bootloader_bin}") ;;
        *) echo "unknown target: ${target}" >&2; exit 1 ;;
    esac
done

"${IDF_PYTHON_ENV_PATH}/bin/python" - --no-stub --chip esp32s3 --port "${serial_port}" \
    --baud 115200 --before default_reset --after hard_reset \
    write_flash "${flash_args[@]}" "${segments[@]}" <<'PY'
import sys

import esptool
from esptool.loader import ESPLoader

# The ROM loader defaults to 1024-byte blocks; this bridge only passes 256 bytes reliably.
ESPLoader.FLASH_WRITE_SIZE = 0x100
esptool.main(sys.argv[1:])
PY
