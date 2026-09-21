#!/usr/bin/env bash
# 装出 Vibe Buddy.app：Rust 的两个 helper、Swift 的 App、固件三件套、五个语音包。
#
# 用法: app/scripts/build-app.sh [--debug] [--install]
#   --debug    允许没有固件（设备页隐藏「更新」），Swift 用 debug 配置。
#   --install  装完拷到 /Applications 并从那里启动。日常使用的 App 必须装在这里：
#              worktree 里的 build 目录随时会被删，登录项与 Hook 绑在那上面，
#              下次开机就什么都不剩（2026-09-17 出过一次）。
# Release 构建要求 firmware/build 里有三件套，缺了就失败并提示先构建固件。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
app_dir="${repo_root}/app"
debug=0
install=0
for arg in "$@"; do
    case "${arg}" in
        --debug) debug=1 ;;
        --install) install=1 ;;
        *) echo "未知参数: ${arg}" >&2; exit 2 ;;
    esac
done

# 语义版本取 Cargo 工作区的 version（仓库还没有 tag，git describe 只是哈希）；
# 构建号是 git 描述，App「关于」里两者都显示，心跳里只带语义版本。
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "${repo_root}/Cargo.toml" | head -n 1)"
version="${version:-0.0.0}"
build_number="$(git -C "${repo_root}" describe --tags --always --dirty 2>/dev/null || echo dev)"

echo "== Rust helper"
cargo build --release --manifest-path "${repo_root}/Cargo.toml" -p vibebuddyd -p vibebuddy-hook
echo "== Swift App"
# macOS 27 的 SDK 把 SwiftUI 的 @State 做成了宏，实现它的 SwiftUIMacros 插件只随
# Xcode 发，命令行工具里没有，用默认 SDK 编译必败。只装了命令行工具时退回它
# 自带的上一版 SDK；装了 Xcode 或自己设了 SDKROOT 的不动。
clt="/Library/Developer/CommandLineTools"
if [[ -z "${SDKROOT:-}" && "$(xcode-select -p 2>/dev/null)" == "${clt}" \
      && ! -e "${clt}/usr/lib/swift/host/plugins/libSwiftUIMacros.dylib" \
      && -d "${clt}/SDKs/MacOSX26.sdk" ]]; then
    export SDKROOT="${clt}/SDKs/MacOSX26.sdk"
    echo "命令行工具缺 SwiftUIMacros 插件，改用 ${SDKROOT}"
fi
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
cp "${repo_root}/target/release/vibebuddyd" "${contents}/MacOS/vibebuddyd"
cp "${repo_root}/target/release/vibebuddy-hook" "${contents}/MacOS/vibebuddy-hook"

echo "== 固件"
fw="${repo_root}/firmware/build"
if [[ -f "${fw}/bootloader/bootloader.bin" && -f "${fw}/partition_table/partition-table.bin" && -f "${fw}/vibebuddy-fw.bin" ]]; then
    cp "${fw}/bootloader/bootloader.bin" "${contents}/Resources/firmware/bootloader.bin"
    cp "${fw}/partition_table/partition-table.bin" "${contents}/Resources/firmware/partition-table.bin"
    cp "${fw}/vibebuddy-fw.bin" "${contents}/Resources/firmware/vibebuddy-fw.bin"
    # 构建标识要和盒子页脚报的一模一样：镜像里 esp_app_desc 的 version 加上
    # 本次构建的时刻戳，格式与固件 describe_firmware_build 一致。
    python3 - "${fw}" > "${contents}/Resources/firmware/build.txt" <<'PY'
import re, struct, sys
from pathlib import Path
build = Path(sys.argv[1])
image = (build / "vibebuddy-fw.bin").read_bytes()
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

# 登录项与通知都认包身份，未签名的包每次改动都像换了个 App。本机默认 ad-hoc；
# 发布时 CI 用 CODESIGN_IDENTITY 给出 Developer ID，公证要求 hardened runtime
# 与时间戳，这条路上签名失败就是构建失败。
if [[ -n "${CODESIGN_IDENTITY:-}" ]]; then
    codesign --force --deep --options runtime --timestamp --sign "${CODESIGN_IDENTITY}" "${bundle}"
else
    codesign --force --deep --sign - "${bundle}" 2>/dev/null || echo "codesign 不可用，跳过签名"
fi
echo "== 完成: ${bundle} (${version}, ${build_number})"

if [[ ${install} -eq 1 ]]; then
    installed="/Applications/Vibe Buddy.app"
    echo "== 安装到 ${installed}"
    # 正在跑的实例先请它退出（顺带退 daemon），换包之后再拉起。
    if pgrep -xq VibeBuddy; then
        osascript -e 'quit app id "com.vibebuddy.app"' >/dev/null 2>&1 || true
        for _ in $(seq 1 50); do pgrep -xq VibeBuddy || break; sleep 0.2; done
    fi
    rm -rf "${installed}"
    ditto "${bundle}" "${installed}"
    open "${installed}"
    echo "== 已启动 ${installed}"
fi
