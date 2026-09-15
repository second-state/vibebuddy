#!/usr/bin/env bash
# 经 BOX 的 CH343 UART 桥烧录。这条路在 esptool 默认参数下 stub 上传会
# Checksum error，--no-stub 也会在写第一块时失败并把 flash 擦掉；能过的是
# --no-stub 加 256 字节写块，慢，但每一块都过校验（见 LESSONS.md）。
# 先单独写分区表试路，再写 app；bootloader 只在明确要求时才写。
#
# 用法: tools/flash-bridge.sh /dev/cu.usbmodemXXXX partition|app|bootloader ...
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="${repo_root}/firmware/build"
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
        app) segments+=(0x10000 "${build_dir}/agent-beacon-fw.bin") ;;
        bootloader) segments+=(0x0 "${build_dir}/bootloader/bootloader.bin") ;;
        *) echo "未知目标: ${target}" >&2; exit 1 ;;
    esac
done

# flash_mode / flash_size / flash_freq 取自构建产物，不在这里手抄。
read -r -a flash_args <<< "$(head -n 1 "${build_dir}/flash_args")"

"${IDF_PYTHON_ENV_PATH}/bin/python" - --no-stub --chip esp32s3 --port "${serial_port}" \
    --baud 115200 --before default_reset --after hard_reset \
    write_flash "${flash_args[@]}" "${segments[@]}" <<'PY'
import sys

import esptool
from esptool.loader import ESPLoader

# ROM loader 默认 1024 字节一块，这条桥只能可靠通过 256 字节。
ESPLoader.FLASH_WRITE_SIZE = 0x100
esptool.main(sys.argv[1:])
PY
