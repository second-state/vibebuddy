#!/usr/bin/env bash
# Screenshot the device: briefly stop vibebuddyd to get the serial port, grab a frame, then bring vibebuddyd back.
# Usage: tools/screenshot.sh /dev/cu.usbmodemXXXX output.png
set -euo pipefail

serial_port="${1:?usage: screenshot.sh <serial-port> <output.png>}"
output="${2:?usage: screenshot.sh <serial-port> <output.png>}"
python="${HOME}/.espressif/tools/python/v5.5.3/venv/bin/python"
label="gui/$(id -u)/com.vibebuddy.vibebuddyd"
plist="${HOME}/Library/LaunchAgents/com.vibebuddy.vibebuddyd.plist"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

launchctl bootout "${label}" 2>/dev/null || true
trap 'launchctl bootstrap "gui/$(id -u)" "${plist}" 2>/dev/null || true' EXIT
sleep 1
"${python}" "${repo_root}/tools/device-screenshot.py" "${serial_port}" "${output}"
