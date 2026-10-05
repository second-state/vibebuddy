#!/usr/bin/env bash
# Flash a box that Vibe Buddy shares with Muse (https://github.com/longzhi/muse-gadget-sdk,
# branch atk-dnesp32s3-box): Vibe Buddy in ota_0, Muse in ota_1, and holding K1 and K2 for
# 3 s in either switches to the other. The partition table and bootloader come from the Muse
# build; see partitions_atk_box_dual.csv there.
#
#   tools/flash-dual.sh [--fresh] [PORT]
#
# Build Muse first: tools/muse/board.sh build atk-box-dual in the SDK's esp32 directory, or
# point MUSE_BUILD at its build directory. Without --fresh only the two apps, the bootloader
# and the table are written: Muse's pairing, Vibe Buddy's settings, the voice pack and which
# app boots are kept. --fresh is for the first install on a box: it also erases Muse's NVS
# (Muse aborts on one it can't read), Vibe Buddy's settings and otadata, so the box boots
# Vibe Buddy and Muse has to be paired again.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="${BUILD_DIR:-${repo_root}/firmware-rs/device/build}"
muse_build="${MUSE_BUILD:-${HOME}/workspace/muse-gadget-sdk/esp32/build-muse-atk-dnesp32s3-box-dual}"

fresh=0
if [[ "${1:-}" == "--fresh" ]]; then
    fresh=1
    shift
fi
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

for file in bootloader/bootloader.bin partition_table/partition-table.bin muse-gadget.bin; do
    if [[ ! -f "${muse_build}/${file}" ]]; then
        echo "Missing ${muse_build}/${file}: build Muse with tools/muse/board.sh build atk-box-dual." >&2
        exit 1
    fi
done
if ! grep -q '^CONFIG_MUSE_ATK_BOX_DUAL_BOOT=y' "${muse_build}/sdkconfig"; then
    echo "${muse_build} is not a dual-boot build (CONFIG_MUSE_ATK_BOX_DUAL_BOOT)." >&2
    exit 1
fi

"${repo_root}/tools/build-firmware.sh"

# One esptool session writes everything: a second connection right after one that left the
# chip in its loader (as chained espflash calls with --after no-reset do) hangs on this box
# until it is unplugged. ESP-IDF's esptool, which the Muse build needed anyway, is the default.
esptool="${ESPTOOL:-$(command -v esptool || ls "${HOME}"/.espressif/python_env/idf6*/bin/esptool 2>/dev/null | head -1)}"
if [[ -z "${esptool}" ]]; then
    echo "esptool not found: install ESP-IDF v6 or set ESPTOOL." >&2
    exit 1
fi
images=(
    0x0 "${muse_build}/bootloader/bootloader.bin"
    0x8000 "${muse_build}/partition_table/partition-table.bin"
    0x20000 "${build_dir}/vibebuddy-fw.bin"
    0x420000 "${muse_build}/muse-gadget.bin"
)
if [[ "${fresh}" == "1" ]]; then
    # nvs, otadata, phy_init and vb_cfg (0x9000 up to ota_0), written blank.
    blank="$(mktemp "${TMPDIR:-/tmp}/flash-dual-blank.XXXXXX")"
    trap 'rm -f "${blank}"' EXIT
    python3 -c 'import sys; sys.stdout.buffer.write(b"\xff" * 0x17000)' > "${blank}"
    images+=(0x9000 "${blank}")
fi
"${esptool}" --chip esp32s3 -p "${serial_port}" -b 460800 --before default-reset --after hard-reset \
    write-flash --flash-mode keep --flash-freq keep --flash-size keep "${images[@]}"
