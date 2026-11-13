#!/usr/bin/env bash
# Build Meta's Muse Gadget firmware for a Vibe Buddy box, to live next to Vibe Buddy (docs/muse.md).
# The source is the official SDK, unchanged: this only adds sdkconfig.vibebuddy, which moves Muse in flash.
#
#   tools/muse/build.sh <path to a muse-gadget-sdk checkout>
#
# Needs ESP-IDF v6.0.3 ($IDF_PATH, else ~/esp/esp-idf-v6.0.3). The result has no SDK token; flash.sh sends yours.
# Output: build/muse/muse-gadget.bin and build/muse/partition_table/partition-table.bin.
set -euo pipefail

sdk="$(cd "${1:?usage: tools/muse/build.sh <muse-gadget-sdk checkout>}" && pwd)/esp32"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${here}/../.." && pwd)"
out="${repo_root}/build/muse"
[ -f "${sdk}/devices/sdkconfig.muse-atk-dnesp32s3-box" ] || { echo "${sdk}: no ATK-DNESP32S3-BOX profile; use a newer SDK" >&2; exit 1; }

export IDF_PATH="${IDF_PATH:-${HOME}/esp/esp-idf-v6.0.3}"
# shellcheck disable=SC1091
. "${IDF_PATH}/export.sh" >/dev/null

mkdir -p "${out}"
# ESP-IDF applies the defaults only to a new sdkconfig, so a left-over one would keep old settings.
rm -f "${out}/sdkconfig"
# The table's path must be absolute: ESP-IDF reads it relative to the SDK's project directory.
{ cat "${here}/sdkconfig.vibebuddy"; echo "CONFIG_PARTITION_TABLE_CUSTOM_FILENAME=\"${here}/partitions.csv\""; } > "${out}/sdkconfig.vibebuddy"
defaults="${sdk}/sdkconfig.defaults;${sdk}/devices/sdkconfig.muse;${sdk}/devices/sdkconfig.muse-atk-dnesp32s3-box;${out}/sdkconfig.vibebuddy"

# Like the SDK's tools/muse/board.sh: boards share managed_components, so start and end clean.
clean() { rm -rf "${sdk}/managed_components" "${sdk}/dependencies.lock"; }
clean
trap clean EXIT
log="${out}/build.log"
if ! idf.py -C "${sdk}" -B "${out}" -DIDF_TARGET=esp32s3 -DSDKCONFIG="${out}/sdkconfig" -DSDKCONFIG_DEFAULTS="${defaults}" build > "${log}" 2>&1; then
    grep -E "error:|Error|undefined reference" "${log}" | head -20
    echo "build failed; see ${log}" >&2
    exit 1
fi
echo "$(git -C "${sdk}" describe --always --dirty), $(wc -c < "${out}/muse-gadget.bin" | tr -d ' ') B -> ${out}/muse-gadget.bin"
