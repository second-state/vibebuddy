#!/usr/bin/env bash
# Packs the Vibe Buddy.app built by build-app.sh into a drag-to-install DMG.
#
# Usage: app/scripts/make-dmg.sh <output.dmg>
# The architecture follows the build machine: CI uses an Apple silicon runner, so the result is arm64.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bundle="${repo_root}/app/build/Vibe Buddy.app"
output="${1:?usage: make-dmg.sh <output.dmg>}"

[[ -d "${bundle}" ]] || { echo "${bundle} not found; run app/scripts/build-app.sh first" >&2; exit 1; }

staging="$(mktemp -d)"
trap 'rm -rf "${staging}"' EXIT
ditto "${bundle}" "${staging}/Vibe Buddy.app"
ditto "${bundle}/Contents/Resources/licenses" "${staging}/Licenses"
ln -s /Applications "${staging}/Applications"

rm -f "${output}"
hdiutil create -volname "VibeBuddy" -srcfolder "${staging}" -fs HFS+ -format UDZO -ov "${output}" >/dev/null
echo "== Done: ${output}"
