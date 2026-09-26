#!/usr/bin/env bash
# 烧 Rust 固件（原生 USB 口）。三件套来自 tools/build-firmware.sh；
# voices 分区与设置区不动，换固件不丢音色。只接 UART 桥时用 tools/flash-bridge.sh。
# 要烧回 C 固件：tools/flash-c.sh。
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
# 前两段写完留在 bootloader 里，最后一段写完再复位，中途不跑半新半旧的固件。
espflash write-bin -S --chip esp32s3 --port "${serial_port}" --after no-reset 0x0 "${build_dir}/bootloader.bin"
espflash write-bin -S --chip esp32s3 --port "${serial_port}" --after no-reset 0x8000 "${build_dir}/partition-table.bin"
espflash write-bin -S --chip esp32s3 --port "${serial_port}" 0x10000 "${build_dir}/vibebuddy-fw.bin"
