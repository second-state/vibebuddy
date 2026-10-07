#!/usr/bin/env bash
# Build the Rust firmware and write the three flash images plus the build ID to firmware-rs/device/build:
#   bootloader.bin      → 0x0
#   partition-table.bin → 0x8000
#   vibebuddy-fw.bin    → 0x10000
#   build.txt           matches the box's DISPLAY READY BUILD line character for character
#   version.txt         the firmware version (firmware-rs/device/Cargo.toml), the box's FIRMWARE VERSION line
# None of the three touches the settings area at 0x9000 or the voices partition at 0x410000.
#
# FAST_CLOCK=1 builds the time-compressed acceptance firmware (five minutes of leisure become five seconds).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
device="${repo_root}/firmware-rs/device"
out="${device}/build"

if ! command -v xtensa-esp32s3-elf-gcc >/dev/null 2>&1 && [[ -f "${HOME}/export-esp.sh" ]]; then
    # shellcheck disable=SC1091
    . "${HOME}/export-esp.sh"
fi
features=()
if [[ "${FAST_CLOCK:-0}" == "1" ]]; then
    features=(--features fast-clock)
fi
(cd "${device}" && cargo build --release ${features[@]+"${features[@]}"})
elf="${device}/target/xtensa-esp32s3-none-elf/release/vibebuddy-firmware"

mkdir -p "${out}"
work="$(mktemp -d "${TMPDIR:-/tmp}/vibebuddy-firmware.XXXXXX")"
trap 'rm -rf "${work}"' EXIT

# The ESP-IDF second-stage bootloader bundled with espflash. --flash-size writes 16 MB into its image header;
# otherwise the bootloader treats the voices partition beyond 4 MB as out of bounds.
espflash save-image -S --chip esp32s3 --flash-size 16mb "${elf}" "${out}/vibebuddy-fw.bin"
espflash save-image -S --chip esp32s3 --flash-size 16mb --merge --skip-padding "${elf}" "${work}/merged.bin"
# espflash can't parse a partition table with a custom subtype (voices is 0x40), so we build it ourselves.
python3 "${repo_root}/tools/make-partition-table.py" "${repo_root}/firmware/partitions.csv" "${out}/partition-table.bin" >/dev/null

python3 - "${work}/merged.bin" "${out}" <<'PY'
import sys
from pathlib import Path

merged = Path(sys.argv[1]).read_bytes()
out = Path(sys.argv[2])
# The first 32 KB of the merged image is the bootloader padded with 0xFF; keep only the bootloader.
bootloader = merged[:0x8000].rstrip(b"\xff")
bootloader += b"\xff" * (-len(bootloader) % 4)
assert bootloader[:1] == b"\xe9", "unexpected bootloader image header"
(out / "bootloader.bin").write_bytes(bootloader)

app = (out / "vibebuddy-fw.bin").read_bytes()
assert merged[0x10000:0x10000 + len(app)] == app, "the app in the merged image differs from the one saved on its own"
marker = b"VIBEBUDDY-BUILD:"
start = app.index(marker) + len(marker)
build = app[start:app.index(b"\0", start)].decode()
(out / "build.txt").write_text(build + "\n")
print(f"bootloader {len(bootloader)} B, app {len(app)} B, build ID {build}")
PY
sed -n 's/^version = "\(.*\)"$/\1/p' "${device}/Cargo.toml" | head -n 1 > "${out}/version.txt"
echo "firmware version $(cat "${out}/version.txt")"
ls -l "${out}"
