#!/usr/bin/env bash
# 给设备截图：短暂停掉 beacond 独占串口，取帧，再把 beacond 拉起来。
# 用法: tools/screenshot.sh /dev/cu.usbmodemXXXX 输出.png
set -euo pipefail

serial_port="${1:?用法: screenshot.sh <串口> <输出.png>}"
output="${2:?用法: screenshot.sh <串口> <输出.png>}"
python="${HOME}/.espressif/tools/python/v5.5.3/venv/bin/python"
label="gui/$(id -u)/com.agentbeacon.beacond"
plist="${HOME}/Library/LaunchAgents/com.agentbeacon.beacond.plist"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

launchctl bootout "${label}" 2>/dev/null || true
trap 'launchctl bootstrap "gui/$(id -u)" "${plist}" 2>/dev/null || true' EXIT
sleep 1
"${python}" "${repo_root}/tools/device-screenshot.py" "${serial_port}" "${output}"
