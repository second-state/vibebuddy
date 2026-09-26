#!/usr/bin/env bash
# 构建 Rust 固件，打出烧录用的三件套与构建标识到 firmware-rs/device/build：
#   bootloader.bin      → 0x0
#   partition-table.bin → 0x8000
#   vibebuddy-fw.bin    → 0x10000
#   build.txt           与盒子页脚、DISPLAY READY BUILD 那一行逐字相同
# 三段都不碰 0x9000 的设置区和 0x410000 的 voices 分区。
#
# FAST_CLOCK=1 构建验收用的时间压缩固件（休闲五分钟压成五秒）。
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
work="$(mktemp -d -t vibebuddy-firmware)"
trap 'rm -rf "${work}"' EXIT

# espflash 自带的 ESP-IDF 二级 bootloader；--flash-size 把 16 MB 写进它的
# 镜像头，否则 bootloader 会把 4 MB 以外的 voices 分区当成越界。
espflash save-image -S --chip esp32s3 --flash-size 16mb "${elf}" "${out}/vibebuddy-fw.bin"
espflash save-image -S --chip esp32s3 --flash-size 16mb --merge --skip-padding "${elf}" "${work}/merged.bin"
# espflash 解析不了自定义子类型的分区表（voices 是 0x40），分区表自己编。
python3 "${repo_root}/tools/make-partition-table.py" "${repo_root}/firmware/partitions.csv" "${out}/partition-table.bin" >/dev/null

python3 - "${work}/merged.bin" "${out}" <<'PY'
import sys
from pathlib import Path

merged = Path(sys.argv[1]).read_bytes()
out = Path(sys.argv[2])
# 合并镜像的前 32 KB 是 bootloader 加补齐的 0xFF；只留 bootloader 本身。
bootloader = merged[:0x8000].rstrip(b"\xff")
bootloader += b"\xff" * (-len(bootloader) % 4)
assert bootloader[:1] == b"\xe9", "bootloader 镜像头不对"
(out / "bootloader.bin").write_bytes(bootloader)

app = (out / "vibebuddy-fw.bin").read_bytes()
assert merged[0x10000:0x10000 + len(app)] == app, "合并镜像里的 app 与单独导出的不一致"
marker = b"VIBEBUDDY-BUILD:"
start = app.index(marker) + len(marker)
build = app[start:app.index(b"\0", start)].decode()
(out / "build.txt").write_text(build + "\n")
print(f"bootloader {len(bootloader)} B, app {len(app)} B, 构建标识 {build}")
PY
ls -l "${out}"
