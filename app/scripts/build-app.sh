#!/usr/bin/env bash
# Assembles Vibe Buddy.app: the two Rust helpers, the Swift app, the three firmware images, and the Character packs.
#
# Usage: app/scripts/build-app.sh [--debug] [--install]
#   --debug    Allow a build without firmware (the Device tab hides "Update"); Swift uses the debug configuration.
#   --install  Copy the result to /Applications and launch it from there. The app you use day to day must live there:
#              a worktree's build directory can be deleted at any time, and the login item and hooks point into it,
#              so after the next reboot nothing is left (this happened once on 2026-09-17).
# A release build requires the three images in firmware-rs/device/build; if any is missing it fails and says to build the firmware first.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
app_dir="${repo_root}/app"
debug=0
install=0
for arg in "$@"; do
    case "${arg}" in
        --debug) debug=1 ;;
        --install) install=1 ;;
        *) echo "unknown argument: ${arg}" >&2; exit 2 ;;
    esac
done

# The semantic version comes from the Cargo workspace version (the repo had no tags yet, so git describe is just a hash);
# the build number is the git description. The app's About shows both; the heartbeat carries only the semantic version.
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "${repo_root}/Cargo.toml" | head -n 1)"
version="${version:-0.0.0}"
build_number="$(git -C "${repo_root}" describe --tags --always --dirty 2>/dev/null || echo dev)"

echo "== Localization"
# A missing zh-Hans entry would silently show English to Chinese users; fail early.
python3 "${repo_root}/tools/check-localization.py"

echo "== Rust helper"
cargo build --release --manifest-path "${repo_root}/Cargo.toml" -p vibebuddyd -p vibebuddy-hook
echo "== Swift App"
# The macOS 27 SDK turns SwiftUI's @State into a macro, and the SwiftUIMacros plugin implementing it ships only
# with Xcode, not the command line tools, so compiling against the default SDK always fails. With only the command
# line tools installed, fall back to their bundled previous SDK; leave Xcode installs and an explicit SDKROOT alone.
clt="/Library/Developer/CommandLineTools"
if [[ -z "${SDKROOT:-}" && "$(xcode-select -p 2>/dev/null)" == "${clt}" \
      && ! -e "${clt}/usr/lib/swift/host/plugins/libSwiftUIMacros.dylib" \
      && -d "${clt}/SDKs/MacOSX26.sdk" ]]; then
    export SDKROOT="${clt}/SDKs/MacOSX26.sdk"
    echo "Command Line Tools lack the SwiftUIMacros plugin; using ${SDKROOT}"
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
# UI copy is keyed in English; each lproj holds one language's table, and macOS
# picks by the user's preferred languages (Chinese systems get zh-Hans).
for lproj in "${app_dir}"/Localization/*.lproj; do
    cp -R "${lproj}" "${contents}/Resources/"
done
# Traditional Chinese systems get the Simplified table: most readers manage it,
# which beats falling back to English. A real zh-Hant table would replace this copy.
cp -R "${app_dir}/Localization/zh-Hans.lproj" "${contents}/Resources/zh-Hant.lproj"

echo "== App icon"
# The menu bar pixel face is rendered into the app icon from the same source; the face is defined once, in PixelFace.swift.
icon_tool="${app_dir}/.build/make-app-icon"
swiftc -O "${app_dir}/Sources/VibeBuddy/PixelFace.swift" "${app_dir}/scripts/make-app-icon.swift" -o "${icon_tool}" 2>&1 | grep -v "warning:" || true
[[ -x "${icon_tool}" ]] || { echo "failed to compile the icon generator" >&2; exit 1; }
"${icon_tool}" "${app_dir}/build/AppIcon.iconset"
iconutil -c icns "${app_dir}/build/AppIcon.iconset" -o "${contents}/Resources/AppIcon.icns"
cp "${repo_root}/target/release/vibebuddyd" "${contents}/MacOS/vibebuddyd"
cp "${repo_root}/target/release/vibebuddy-hook" "${contents}/MacOS/vibebuddy-hook"

echo "== Firmware"
# The Rust firmware's three images and build ID, produced by tools/build-firmware.sh. build.txt matches the box
# footer and its DISPLAY READY BUILD line character for character; the app compares against it to offer updates.
fw="${repo_root}/firmware-rs/device/build"
if [[ -f "${fw}/bootloader.bin" && -f "${fw}/partition-table.bin" && -f "${fw}/vibebuddy-fw.bin" && -f "${fw}/build.txt" ]]; then
    for file in bootloader.bin partition-table.bin vibebuddy-fw.bin build.txt; do
        cp "${fw}/${file}" "${contents}/Resources/firmware/${file}"
    done
    echo "Bundled firmware $(cat "${contents}/Resources/firmware/build.txt")"
elif [[ ${debug} -eq 1 ]]; then
    echo "No firmware build output; debug build ships without firmware"
else
    echo "A release build needs the three firmware images in firmware-rs/device/build; run tools/build-firmware.sh first" >&2
    exit 1
fi

# A Character's pack (characters/<id>/pack.bin, from tools/make-character.sh) replaces the old voice
# pack of the same id; voices that have no Character yet still ship their five fixed lines.
echo "== Character packs"
for dir in "${repo_root}"/characters/*/; do
    id="$(basename "${dir}")"
    if [[ -f "${dir}/pack.bin" ]]; then
        cp "${dir}/pack.bin" "${contents}/Resources/voices/${id}.bin"
        echo "  ${id} (character)"
    fi
done
for dir in "${repo_root}"/voices/*/; do
    id="$(basename "${dir}")"
    if [[ -f "${dir}/done.pcm" && ! -f "${contents}/Resources/voices/${id}.bin" ]]; then
        "${repo_root}/tools/make_voice_pack.py" "${dir}" "${id}" "${contents}/Resources/voices/${id}.bin" >/dev/null
        echo "  ${id} (voice pack)"
    fi
done

# Both the App and its separately downloadable firmware carry these notices.
echo "== Licenses and corresponding source"
python3 "${repo_root}/tools/package-licenses.py" "${contents}/Resources"

# Login items and notifications key on the bundle's identity, so an unsigned bundle looks like a new app after every change. Locally we sign ad hoc;
# for releases CI passes a Developer ID in CODESIGN_IDENTITY, and notarization requires the hardened runtime
# and a timestamp, so on that path a signing failure fails the build.
if [[ -n "${CODESIGN_IDENTITY:-}" ]]; then
    codesign --force --deep --options runtime --timestamp --sign "${CODESIGN_IDENTITY}" "${bundle}"
else
    codesign --force --deep --sign - "${bundle}" 2>/dev/null || echo "codesign unavailable, skipping signing"
fi
echo "== Done: ${bundle} (${version}, ${build_number})"

if [[ ${install} -eq 1 ]]; then
    installed="/Applications/Vibe Buddy.app"
    echo "== Installing to ${installed}"
    # Ask a running instance to quit first (which also stops its daemon), swap the bundle, then relaunch.
    if pgrep -xq VibeBuddy; then
        osascript -e 'quit app id "com.vibebuddy.app"' >/dev/null 2>&1 || true
        for _ in $(seq 1 50); do pgrep -xq VibeBuddy || break; sleep 0.2; done
    fi
    rm -rf "${installed}"
    ditto "${bundle}" "${installed}"
    open "${installed}"
    echo "== Launched ${installed}"
fi
