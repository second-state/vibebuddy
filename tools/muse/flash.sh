#!/usr/bin/env bash
# Flash Muse next to Vibe Buddy on a box, over the USB-SLAVE port (docs/muse.md). Vibe Buddy, its Character, its
# settings and its pairings stay. Build Muse first with tools/muse/build.sh. Quit the Vibe Buddy app first: it holds
# the port.
#
#   tools/muse/flash.sh <port> [sdk token]
#
# Without a token it only flashes; send the token later with tools/muse/send-token.py <port> <token>.
set -euo pipefail

port="${1:?usage: tools/muse/flash.sh <port> [sdk token]}"
token="${2:-}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${here}/../.." && pwd)"
muse="${repo_root}/build/muse"
[ -f "${muse}/muse-gadget.bin" ] || { echo "no Muse build; run tools/muse/build.sh first" >&2; exit 1; }

export IDF_PATH="${IDF_PATH:-${HOME}/esp/esp-idf-v6.0.3}"
# shellcheck disable=SC1091
. "${IDF_PATH}/export.sh" >/dev/null

# Vibe Buddy's partition table with the Muse partitions, and the bootloader that reads boot_sel. A box flashed
# before them has neither.
table="${muse}/vibebuddy-partition-table.bin"
python3 "${repo_root}/tools/make-partition-table.py" "${repo_root}/firmware/partitions.csv" "${table}" >/dev/null
# boot_sel set to Muse ("MUSE"), so the box starts Muse when the flash is done.
boot_sel="${muse}/boot_sel-muse.bin"
python3 -c 'import sys; sys.stdout.buffer.write(b"MUSE" + b"\xff" * 4092)' > "${boot_sel}"

python3 -m esptool --chip esp32s3 -p "${port}" -b 460800 --after hard-reset \
    write-flash --flash-mode keep --flash-freq keep --flash-size keep \
    0x0 "${repo_root}/firmware-rs/bootloader/bootloader.bin" \
    0x8000 "${table}" \
    0xa14000 "${muse}/partition_table/partition-table.bin" \
    0xa21000 "${boot_sel}" \
    0xa30000 "${muse}/muse-gadget.bin"

if [ -n "${token}" ]; then
    python3 "${here}/send-token.py" "${port}" "${token}"
fi
