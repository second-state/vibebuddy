#!/usr/bin/env bash
# Installs Vibe Buddy for the current Linux user: the binaries in ~/.local/bin, the daemon as a systemd user service,
# the tray app in the launcher and at login, the hooks for whichever of Claude Code and Codex this machine has, and the
# voice packs the app offers to write to the box. Run it again to upgrade.
#
# It works in two places. In an unpacked release (VibeBuddy-vX.Y.Z-linux-x86_64.tar.gz) everything is prebuilt and
# nothing is downloaded. In a source checkout (packaging/linux/install.sh) it builds with cargo.
#
# To remove: `vibebuddy-hook uninstall`, `systemctl --user disable --now vibebuddyd`, then delete the files it installs.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bin="${HOME}/.local/bin"
config="${XDG_CONFIG_HOME:-${HOME}/.config}"
units="${config}/systemd/user"
data="${XDG_DATA_HOME:-${HOME}/.local/share}"

if [[ -x "${here}/bin/vibebuddyd" ]]; then
    release=1
    built="${here}/bin"
    share="${here}/share"
else
    release=0
    repo="$(cd "${here}/../.." && pwd)"
    cargo build --release --locked --manifest-path "${repo}/Cargo.toml" -p vibebuddyd -p vibebuddy-hook -p vibebuddy-desktop
    built="${repo}/target/release"
    share="${repo}/packaging/linux"
fi

# Stop first so the running daemon lets go of its binary and the serial port.
systemctl --user stop vibebuddyd.service 2>/dev/null || true
install -Dm755 "${built}/vibebuddyd" "${bin}/vibebuddyd"
install -Dm755 "${built}/vibebuddy-hook" "${bin}/vibebuddy-hook"
install -Dm644 "${share}/vibebuddyd.service" "${units}/vibebuddyd.service"
# The app has no lifecycle of its own to supervise: the launcher and the login autostart both just run it.
pkill -f -x "${bin}/vibebuddy-desktop" || true
install -Dm755 "${built}/vibebuddy-desktop" "${bin}/vibebuddy-desktop"
# An absolute Exec, since autostart may run before ~/.local/bin is on PATH.
for entry in "${data}/applications/vibebuddy.desktop" "${config}/autostart/vibebuddy.desktop"; do
    mkdir -p "$(dirname "${entry}")"
    sed "s|^Exec=.*|Exec=${bin}/vibebuddy-desktop|" "${share}/vibebuddy.desktop" > "${entry}"
done
install -Dm644 "${share}/vibebuddy.svg" "${data}/icons/hicolor/scalable/apps/vibebuddy.svg"
systemctl --user daemon-reload
systemctl --user enable --now vibebuddyd.service

"${bin}/vibebuddy-hook" install

# What the Mac app carries in its bundle lives here instead: the Character packs for the Character tab. Firmware isn't
# installed; the daemon downloads it (ADR-0010).
assets="${data}/vibebuddy"
mkdir -p "${assets}/voices"
if [[ ${release} -eq 1 ]]; then
    cp "${share}"/voices/*.bin "${assets}/voices/"
else
    for pack in "${repo}"/characters/*/pack.bin; do
        id="$(basename "$(dirname "${pack}")")"
        cp "${pack}" "${assets}/voices/${id}.bin"
        # Its lines said with each form of address; the app swaps in the one picked.
        for variant in "$(dirname "${pack}")"/address/*.bin; do
            [[ -f "${variant}" ]] && cp "${variant}" "${assets}/voices/${id}.$(basename "${variant}")"
        done
    done
fi
# Firmware from earlier versions of this script; the daemon keeps its own now.
rm -rf "${assets}/firmware"

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
