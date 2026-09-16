#!/usr/bin/env bash
# 装出 Vibe Buddy.app：Rust 的两个 helper、Swift 的 App、固件三件套、五个语音包。
#
# 用法: app/scripts/build-app.sh [--debug]
#   --debug  允许没有固件（设备页隐藏「更新」），Swift 用 debug 配置。
# Release 构建要求 firmware/build 里有三件套，缺了就失败并提示先构建固件。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
app_dir="${repo_root}/app"
debug=0
if [[ "${1:-}" == "--debug" ]]; then
    debug=1
fi

# 语义版本取 Cargo 工作区的 version（仓库还没有 tag，git describe 只是哈希）；
# 构建号是 git 描述，App「关于」里两者都显示，心跳里只带语义版本。
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "${repo_root}/Cargo.toml" | head -n 1)"
version="${version:-0.0.0}"
build_number="$(git -C "${repo_root}" describe --tags --always --dirty 2>/dev/null || echo dev)"

echo "== Rust helper"
cargo build --release --manifest-path "${repo_root}/Cargo.toml" -p beacond -p beacon-hook
echo "== Swift App"
if [[ ${debug} -eq 1 ]]; then
    swift build --package-path "${app_dir}" --product VibeBuddy
    swift_bin="${app_dir}/.build/debug/VibeBuddy"
else
    swift build --package-path "${app_dir}" -c release --product VibeBuddy
    swift_bin="${app_dir}/.build/release/VibeBuddy"
fi

bundle="${app_dir}/build/Vibe Buddy.app"
contents="${bundle}/Contents"
rm -rf "${bundle}"
mkdir -p "${contents}/MacOS" "${contents}/Resources/firmware" "${contents}/Resources/voices"

sed -e "s/__VERSION__/${version}/" -e "s/__BUILD__/${build_number}/" "${app_dir}/Info.plist" > "${contents}/Info.plist"
cp "${swift_bin}" "${contents}/MacOS/VibeBuddy"
cp "${repo_root}/target/release/beacond" "${contents}/MacOS/beacond"
cp "${repo_root}/target/release/beacon-hook" "${contents}/MacOS/beacon-hook"

echo "== 固件"
fw="${repo_root}/firmware/build"
if [[ -f "${fw}/bootloader/bootloader.bin" && -f "${fw}/partition_table/partition-table.bin" && -f "${fw}/agent-beacon-fw.bin" ]]; then
    cp "${fw}/bootloader/bootloader.bin" "${contents}/Resources/firmware/bootloader.bin"
    cp "${fw}/partition_table/partition-table.bin" "${contents}/Resources/firmware/partition-table.bin"
    cp "${fw}/agent-beacon-fw.bin" "${contents}/Resources/firmware/agent-beacon-fw.bin"
    # 构建标识要和盒子页脚报的一模一样：镜像里 esp_app_desc 的 version 加上
    # 本次构建的时刻戳，格式与固件 describe_firmware_build 一致。
    python3 - "${fw}" > "${contents}/Resources/firmware/build.txt" <<'PY'
import re, struct, sys
from pathlib import Path
build = Path(sys.argv[1])
image = (build / "agent-beacon-fw.bin").read_bytes()
# esp_app_desc_t 在镜像偏移 0x20：magic(4) secure_version(4) reserv1(8) version[32]
magic, = struct.unpack_from("<I", image, 0x20)
assert magic == 0xABCD5432, "找不到 esp_app_desc"
version = image[0x30:0x50].split(b"\0", 1)[0].decode()[:24]
stamp_header = next(build.rglob("agent_build_stamp.h"))
stamp = re.search(r'"([^"]+)"', stamp_header.read_text()).group(1)[:16]
print(f"{version} {stamp}")
PY
    echo "附带固件 $(cat "${contents}/Resources/firmware/build.txt")"
elif [[ ${debug} -eq 1 ]]; then
    echo "没有固件构建产物，Debug 构建不附带固件"
else
    echo "Release 构建需要 firmware/build 里的三件套，先跑 idf.py -C firmware build" >&2
    exit 1
fi

echo "== 语音包"
for dir in "${repo_root}"/voices/*/; do
    id="$(basename "${dir}")"
    if [[ -f "${dir}/done.pcm" ]]; then
        "${repo_root}/tools/make_voice_pack.py" "${dir}" "${id}" "${contents}/Resources/voices/${id}.bin" >/dev/null
        echo "  ${id}"
    fi
done

# ad-hoc 签名：登录项与通知都认包身份，未签名的包每次改动都像换了个 App。
codesign --force --deep --sign - "${bundle}" 2>/dev/null || echo "codesign 不可用，跳过签名"
echo "== 完成: ${bundle} (${version}, ${build_number})"
