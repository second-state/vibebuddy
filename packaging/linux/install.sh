#!/usr/bin/env bash
# Builds Vibe Buddy from this checkout and installs it for the current Linux user: both binaries in ~/.local/bin,
# the daemon as a systemd user service, and the hooks for whichever of Claude Code and Codex this machine has.
# Run it again to upgrade. To remove: `vibebuddy-hook uninstall`, then `systemctl --user disable --now vibebuddyd`.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bin="${HOME}/.local/bin"
units="${XDG_CONFIG_HOME:-${HOME}/.config}/systemd/user"

cargo build --release --locked --manifest-path "${repo}/Cargo.toml" -p vibebuddyd -p vibebuddy-hook

# Stop first so the running daemon lets go of its binary and the serial port.
systemctl --user stop vibebuddyd.service 2>/dev/null || true
install -Dm755 "${repo}/target/release/vibebuddyd" "${bin}/vibebuddyd"
install -Dm755 "${repo}/target/release/vibebuddy-hook" "${bin}/vibebuddy-hook"
install -Dm644 "${repo}/packaging/linux/vibebuddyd.service" "${units}/vibebuddyd.service"
systemctl --user daemon-reload
systemctl --user enable --now vibebuddyd.service

"${bin}/vibebuddy-hook" install

# The box shows up as /dev/ttyACM*, owned by uucp on Arch and dialout on Debian and Ubuntu.
# getent fails when either group is missing, and each distro has only one of them.
serial_group="$({ getent group uucp dialout || true; } | head -n1 | cut -d: -f1)"
if [[ -n "${serial_group}" ]]; then
    gid="$(getent group "${serial_group}" | cut -d: -f3)"
    status="/proc/$(systemctl --user show -p MainPID --value vibebuddyd.service)/status"
    if ! id -nG | tr ' ' '\n' | grep -qx "${serial_group}"; then
        echo
        echo "You are not in the ${serial_group} group, so the daemon cannot open the box. Run this, then reboot:"
        echo "  sudo usermod -aG ${serial_group} ${USER}"
    # The daemon inherits its groups from the systemd user manager, which only picks up a new group when it restarts.
    elif [[ -r "${status}" ]] && ! grep '^Groups:' "${status}" | grep -qw "${gid}"; then
        echo
        echo "You are in ${serial_group}, but your systemd user session started before you were added,"
        echo "so the daemon cannot open the box yet. Reboot (or log out of every session) once."
    fi
fi

echo
echo "Installed. Daemon log: journalctl --user -u vibebuddyd -f"
