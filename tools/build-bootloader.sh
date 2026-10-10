#!/usr/bin/env bash
# Rebuild firmware-rs/bootloader/bootloader.bin: the ESP-IDF second-stage bootloader with app rollback enabled
# (ADR-0013). The result is committed, so only run this when its configuration or ESP-IDF changes.
# Uses $IDF_PATH if set, else ~/esp/esp-idf-v6.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
project="${repo_root}/firmware-rs/bootloader"
export IDF_PATH="${IDF_PATH:-${HOME}/esp/esp-idf-v6}"

# shellcheck disable=SC1091
. "${IDF_PATH}/export.sh" >/dev/null
cd "${project}"
rm -rf build sdkconfig
idf.py set-target esp32s3 >/dev/null
idf.py bootloader >/dev/null
cp build/bootloader/bootloader.bin bootloader.bin
echo "$(idf.py --version), $(wc -c < bootloader.bin | tr -d ' ') B -> ${project}/bootloader.bin"
