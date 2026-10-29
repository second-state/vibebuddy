#!/usr/bin/env bash
# Exercise the real Linux daemon against the simulated box (the firmware core) without a box or desktop.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != Linux ]]; then
    echo "These host integration tests require Linux."
    exit 1
fi

cargo build --locked -p vibebuddyd --target-dir target
cargo build --locked -p vibebuddy-desktop --example host_status_probe --example daemon_outage_probe --target-dir target
cargo build --locked -p vibebuddy-firmware-core --example simulator --target-dir target

export VIBEBUDDY_HOST_TESTS=1
python3 -m unittest discover -s tools -p test_linux_host.py -v "$@"
