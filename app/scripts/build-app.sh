#!/usr/bin/env bash
# Assembles VibeBuddy.app: the two Rust helpers, the Swift app with Sparkle, and the Character packs. Firmware isn't bundled: it is
# released on its own and the daemon downloads it (ADR-0010).
#
# Usage: app/scripts/build-app.sh [--debug] [--install]
#   --debug    Swift uses the debug configuration.
#   --install  Copy the result to /Applications and launch it from there. The app you use day to day must live there:
#              a worktree's build directory can be deleted at any time, and the login item and hooks point into it,
#              so after the next reboot nothing is left (this happened once on 2026-09-17).
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

bundle="${app_dir}/build/VibeBuddy.app"
contents="${bundle}/Contents"
rm -rf "${bundle}"
mkdir -p "${contents}/MacOS" "${contents}/Frameworks" "${contents}/Resources/voices"

sed -e "s/__VERSION__/${version}/" -e "s/__BUILD__/${build_number}/" "${app_dir}/Info.plist" > "${contents}/Info.plist"
# Sparkle trusts only downloads signed with the key in app/sparkle-key.pub (tools/setup-update-signing.sh); a build
# without it has no SUPublicEDKey, and the app then leaves Sparkle off.
if [[ -s "${app_dir}/sparkle-key.pub" ]]; then
    /usr/libexec/PlistBuddy -c "Set :SUPublicEDKey $(tr -d '[:space:]' < "${app_dir}/sparkle-key.pub")" "${contents}/Info.plist"
else
    /usr/libexec/PlistBuddy -c "Delete :SUPublicEDKey" "${contents}/Info.plist"
fi

echo "== Sparkle"
ditto "${app_dir}/.build/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64/Sparkle.framework" \
    "${contents}/Frameworks/Sparkle.framework"
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

# Each Character's pack is built by tools/make-character.sh and committed as characters/<id>/pack.bin.
echo "== Character packs"
# The robot: the built-in voice's five lines as a pack without a look, and its face for the card.
"${repo_root}/tools/make_voice_pack.py" "${repo_root}/voices/jessica" robot "${contents}/Resources/voices/robot.bin" >/dev/null
cp "${repo_root}/characters/robot/face.png" "${contents}/Resources/robot-face.png"
echo "  robot"
for pack in "${repo_root}"/characters/*/pack.bin; do
    id="$(basename "$(dirname "${pack}")")"
    cp "${pack}" "${contents}/Resources/voices/${id}.bin"
    # The same Character's lines said with each form of address; the app swaps in the one picked.
    for variant in "$(dirname "${pack}")"/address/*.bin; do
        [[ -f "${variant}" ]] && cp "${variant}" "${contents}/Resources/voices/${id}.$(basename "${variant}")"
    done
    echo "  ${id}"
done

# The firmware zip carries its own notices (release-firmware.yml).
echo "== Licenses and corresponding source"
python3 "${repo_root}/tools/package-licenses.py" "${contents}/Resources"
# Sparkle ships inside the bundle, with the notices of what it bundles in turn in its own LICENSE.
cp "${app_dir}/.build/checkouts/Sparkle/LICENSE" "${contents}/Resources/licenses/THIRD-PARTY-SPARKLE.txt"

# Login items and notifications key on the bundle's identity, so an unsigned bundle looks like a new app after every change. Locally we sign ad hoc;
# for releases CI passes a Developer ID in CODESIGN_IDENTITY, and notarization requires the hardened runtime
# and a timestamp, so on that path a signing failure fails the build.
# Inside out and without --deep, as Sparkle asks: --deep would re-sign its XPC services and drop the Downloader's
# entitlements. The helpers, then each of Sparkle's executables, the framework, and the app last.
sign() {
    if [[ -n "${CODESIGN_IDENTITY:-}" ]]; then
        codesign --force --options runtime --timestamp --sign "${CODESIGN_IDENTITY}" "$@"
    else
        codesign --force --sign - "$@"
    fi
}
if command -v codesign >/dev/null 2>&1; then
    sparkle="${contents}/Frameworks/Sparkle.framework/Versions/B"
    sign "${contents}/MacOS/vibebuddyd" "${contents}/MacOS/vibebuddy-hook"
    sign "${sparkle}/XPCServices/Installer.xpc"
    sign --preserve-metadata=entitlements "${sparkle}/XPCServices/Downloader.xpc"
    sign "${sparkle}/Autoupdate" "${sparkle}/Updater.app"
    sign "${contents}/Frameworks/Sparkle.framework"
    # The app asks for Automation access itself (Agents tab), which the hardened runtime only allows with this entitlement.
    sign --entitlements "${app_dir}/VibeBuddy.entitlements" "${bundle}"
else
    echo "codesign unavailable, skipping signing"
fi
echo "== Done: ${bundle} (${version}, ${build_number})"

if [[ ${install} -eq 1 ]]; then
    # A copy installed under the bundle's old name is replaced where it is: the app renames itself on launch and
    # moves the login item along (BundleName in VibeBuddyCore).
    installed="/Applications/VibeBuddy.app"
    [[ -d "/Applications/Vibe Buddy.app" && ! -d "${installed}" ]] && installed="/Applications/Vibe Buddy.app"
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
