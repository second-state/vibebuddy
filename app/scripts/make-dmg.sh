#!/usr/bin/env bash
# 把 build-app.sh 装出的 Vibe Buddy.app 打成拖拽安装的 DMG。
#
# 用法: app/scripts/make-dmg.sh <输出.dmg>
# 架构跟着构建机走：CI 用 Apple 芯片的 runner，出的就是 arm64。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bundle="${repo_root}/app/build/Vibe Buddy.app"
output="${1:?用法: make-dmg.sh <输出.dmg>}"

[[ -d "${bundle}" ]] || { echo "没有 ${bundle}，先跑 app/scripts/build-app.sh" >&2; exit 1; }

staging="$(mktemp -d)"
trap 'rm -rf "${staging}"' EXIT
ditto "${bundle}" "${staging}/Vibe Buddy.app"
ln -s /Applications "${staging}/Applications"

rm -f "${output}"
hdiutil create -volname "Vibe Buddy" -srcfolder "${staging}" -fs HFS+ -format UDZO -ov "${output}" >/dev/null
echo "== 完成: ${output}"
