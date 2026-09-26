#!/usr/bin/env bash
# 装出 Vibe Buddy.app：Rust 的两个 helper、Swift 的 App、固件三件套、五个语音包。
#
# 用法: app/scripts/build-app.sh [--debug] [--install]
#   --debug    允许没有固件（设备页隐藏「更新」），Swift 用 debug 配置。
#   --install  装完拷到 /Applications 并从那里启动。日常使用的 App 必须装在这里：
#              worktree 里的 build 目录随时会被删，登录项与 Hook 绑在那上面，
#              下次开机就什么都不剩（2026-09-17 出过一次）。
# Release 构建要求 firmware-rs/device/build 里有三件套，缺了就失败并提示先构建固件。
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

echo "== App 图标"
# 菜单栏的像素脸同一份源码渲染成 App 图标，脸只在 PixelFace.swift 里定义一次。
icon_tool="${app_dir}/.build/make-app-icon"
swiftc -O "${app_dir}/Sources/VibeBuddy/PixelFace.swift" "${app_dir}/scripts/make-app-icon.swift" -o "${icon_tool}" 2>&1 | grep -v "warning:" || true
[[ -x "${icon_tool}" ]] || { echo "图标生成器编译失败" >&2; exit 1; }
"${icon_tool}" "${app_dir}/build/AppIcon.iconset"
iconutil -c icns "${app_dir}/build/AppIcon.iconset" -o "${contents}/Resources/AppIcon.icns"
cp "${repo_root}/target/release/vibebuddyd" "${contents}/MacOS/vibebuddyd"
cp "${repo_root}/target/release/vibebuddy-hook" "${contents}/MacOS/vibebuddy-hook"

echo "== 固件"
# Rust 固件的三件套与构建标识，由 tools/build-firmware.sh 打到这里。build.txt
# 与盒子页脚、DISPLAY READY BUILD 那一行逐字相同，App 靠它判断要不要更新。
fw="${repo_root}/firmware-rs/device/build"
if [[ -f "${fw}/bootloader.bin" && -f "${fw}/partition-table.bin" && -f "${fw}/vibebuddy-fw.bin" && -f "${fw}/build.txt" ]]; then
    for file in bootloader.bin partition-table.bin vibebuddy-fw.bin build.txt; do
        cp "${fw}/${file}" "${contents}/Resources/firmware/${file}"
    done
    echo "附带固件 $(cat "${contents}/Resources/firmware/build.txt")"
elif [[ ${debug} -eq 1 ]]; then
    echo "没有固件构建产物，Debug 构建不附带固件"
else
    echo "Release 构建需要 firmware-rs/device/build 里的三件套，先跑 tools/build-firmware.sh" >&2
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
