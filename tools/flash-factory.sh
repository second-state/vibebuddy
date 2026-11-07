#!/usr/bin/env bash
# Flash a box of a company edition at the factory (ADR-0025), in one pass over the native USB port:
# the bootloader, the partition table, the firmware, a blank otadata, the company Character's pack in voices
# (so the box starts up wearing it) and the edition image in the edition partition. Then check that the box
# reports the edition.
#
# Usage: tools/flash-factory.sh <edition.bin> [port]
# The edition image comes from tools/make_edition.py. The firmware is built first, as tools/flash.sh does.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="${BUILD_DIR:-${repo_root}/firmware-rs/device/build}"
edition="${1:-}"
serial_port="${2:-}"

if [[ -z "${edition}" || ! -f "${edition}" ]]; then
    echo "Usage: tools/flash-factory.sh <edition.bin> [port]" >&2
    exit 2
fi
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
if ! command -v esptool >/dev/null 2>&1; then
    echo "esptool not found on PATH; install it (pip install esptool) or activate an ESP-IDF environment." >&2
    exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT
# Check the image as the firmware will, and take the pack out of it for voices.
edition_id="$(python3 - "${repo_root}/tools" "${edition}" "${work}/pack.bin" <<'EOF'
import sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
import make_edition
image = Path(sys.argv[2]).read_bytes()
edition_id = make_edition.check(image)
Path(sys.argv[3]).write_bytes(image[make_edition.PACK_OFFSET:])
print(edition_id)
EOF
)"

"${repo_root}/tools/build-firmware.sh"
# One connection writes every image and resets once at the end. The addresses are those of firmware/partitions.csv.
esptool --chip esp32s3 --port "${serial_port}" --before default-reset --after hard-reset \
    write-flash --flash-mode keep --flash-freq keep --flash-size keep \
    0x0 "${build_dir}/bootloader.bin" \
    0x8000 "${build_dir}/partition-table.bin" \
    0x10000 "${build_dir}/vibebuddy-fw.bin" \
    0x410000 "${work}/pack.bin" \
    0xa10000 "${build_dir}/ota-data-initial.bin" \
    0xa14000 "${edition}"

# The native USB port goes away during the reset; give it time to come back.
sleep 3
python3 "${repo_root}/tools/check-edition.py" "${serial_port}" "${edition_id}"
