#!/usr/bin/env bash
# Builds Vibe Buddy from this checkout and installs it for the current Linux user: the binaries in ~/.local/bin,
# the daemon as a systemd user service, the tray app in the launcher and at login, and the hooks for whichever of
# Claude Code and Codex this machine has. Run it again to upgrade. To remove: `vibebuddy-hook uninstall`,
# `systemctl --user disable --now vibebuddyd`, then delete the files this script installs.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
bin="${HOME}/.local/bin"
config="${XDG_CONFIG_HOME:-${HOME}/.config}"
units="${config}/systemd/user"
data="${XDG_DATA_HOME:-${HOME}/.local/share}"

cargo build --release --locked --manifest-path "${repo}/Cargo.toml" -p vibebuddyd -p vibebuddy-hook -p vibebuddy-desktop

# Stop first so the running daemon lets go of its binary and the serial port.
systemctl --user stop vibebuddyd.service 2>/dev/null || true
install -Dm755 "${repo}/target/release/vibebuddyd" "${bin}/vibebuddyd"
install -Dm755 "${repo}/target/release/vibebuddy-hook" "${bin}/vibebuddy-hook"
install -Dm644 "${repo}/packaging/linux/vibebuddyd.service" "${units}/vibebuddyd.service"
# The app has no lifecycle of its own to supervise: the launcher and the login autostart both just run it.
pkill -f -x "${bin}/vibebuddy-desktop" || true
install -Dm755 "${repo}/target/release/vibebuddy-desktop" "${bin}/vibebuddy-desktop"
# An absolute Exec, since autostart may run before ~/.local/bin is on PATH.
for entry in "${data}/applications/vibebuddy.desktop" "${config}/autostart/vibebuddy.desktop"; do
    mkdir -p "$(dirname "${entry}")"
    sed "s|^Exec=.*|Exec=${bin}/vibebuddy-desktop|" "${repo}/packaging/linux/vibebuddy.desktop" > "${entry}"
done
install -Dm644 "${repo}/packaging/linux/vibebuddy.svg" "${data}/icons/hicolor/scalable/apps/vibebuddy.svg"
systemctl --user daemon-reload
systemctl --user enable --now vibebuddyd.service

"${bin}/vibebuddy-hook" install

# Start the tray now rather than at the next login. systemd-run hands it the graphical session's environment,
# which a shell over SSH doesn't have.
if systemctl --user show-environment | grep -q '^WAYLAND_DISPLAY='; then
    systemd-run --user --collect --quiet "${bin}/vibebuddy-desktop"
fi

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
