#!/usr/bin/env bash
# Flash through the BOX's CH343 UART bridge. On this path, with esptool's default settings the stub upload
# hits a checksum error, and --no-stub also fails on the first block and erases the flash; what works is
# --no-stub with 256-byte write blocks: slow, but every block passes verification (see LESSONS.md).
# Write the partition table alone first to test the path, then the app; the bootloader is only written when explicitly asked for.
#
# Usage: tools/flash-bridge.sh /dev/cu.usbmodemXXXX partition|app|bootloader ...
# Flashes firmware/build by default; set BUILD_DIR to flash another build directory, e.g. the acceptance
# firmware/build-fast。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="${BUILD_DIR:-${repo_root}/firmware/build}"
serial_port="${1:?用法: flash-bridge.sh <串口> partition|app|bootloader ...}"
shift
if [[ $# -eq 0 ]]; then
    echo "至少给一个要写的目标：partition、app 或 bootloader" >&2
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
        partition) segments+=(0x8000 "${build_dir}/partition_table/partition-table.bin") ;;
        app) segments+=(0x10000 "${build_dir}/vibebuddy-fw.bin") ;;
        bootloader) segments+=(0x0 "${build_dir}/bootloader/bootloader.bin") ;;
        *) echo "未知目标: ${target}" >&2; exit 1 ;;
    esac
done

# flash_mode / flash_size / flash_freq come from the build output rather than being copied here by hand.
read -r -a flash_args <<< "$(head -n 1 "${build_dir}/flash_args")"

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
