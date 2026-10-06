#!/usr/bin/env bash
# Installs Vibe Buddy for the current Linux user: the binaries in ~/.local/bin, the daemon as a systemd user service,
# the tray app in the launcher and at login, the hooks for whichever of Claude Code and Codex this machine has, and the
# voice packs and firmware the app offers to write to the box. Run it again to upgrade.
#
# It works in two places. In an unpacked release (VibeBuddy-vX.Y.Z-linux-x86_64.tar.gz) everything is prebuilt and
# nothing is downloaded. In a source checkout (packaging/linux/install.sh) it builds with cargo, makes the voice packs
# with python3 and fetches this version's firmware from the GitHub release.
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

# What the Mac app carries in its bundle lives here instead: Character packs and firmware for the Sound and Device tabs.
assets="${data}/vibebuddy"
mkdir -p "${assets}/voices"
if [[ ${release} -eq 1 ]]; then
    cp "${share}"/voices/*.bin "${assets}/voices/"
    rm -rf "${assets}/firmware"
    cp -r "${share}/firmware" "${assets}/firmware"
    echo "Firmware for the Device tab: $(cat "${assets}/firmware/build.txt")"
else
    for pack in "${repo}"/characters/*/pack.bin; do
        cp "${pack}" "${assets}/voices/$(basename "$(dirname "${pack}")").bin"
    done

    # Firmware isn't built here (that takes the Xtensa toolchain): it comes from the GitHub release of this version,
    # checked against the sha256 GitHub records for the asset. Without it the Device tab simply offers no update.
    version="$(sed -n 's/^version = "\(.*\)"$/\1/p' "${repo}/Cargo.toml" | head -n1)"
    zip="VibeBuddy-firmware-v${version}.zip"
    download="$(mktemp -d)"
    trap 'rm -rf "${download}"' EXIT
    if curl -fsSL -o "${download}/${zip}" "https://github.com/second-state/vibebuddy/releases/download/v${version}/${zip}"; then
        expected="$(curl -fsSL "https://api.github.com/repos/second-state/vibebuddy/releases/tags/v${version}" \
            | python3 -c 'import json, sys; print(next(a["digest"] for a in json.load(sys.stdin)["assets"] if a["name"] == sys.argv[1]))' "${zip}" || true)"
        actual="sha256:$(sha256sum "${download}/${zip}" | cut -d' ' -f1)"
        if [[ "${expected}" == "${actual}" ]]; then
            rm -rf "${assets}/firmware"
            python3 -m zipfile -e "${download}/${zip}" "${assets}/firmware"
            echo "Firmware for the Device tab: $(cat "${assets}/firmware/build.txt")"
        else
            echo "warning: ${zip} doesn't match the checksum GitHub lists (${expected}); firmware left out" >&2
        fi
    else
        echo "No firmware release for v${version} on GitHub; the Device tab won't offer updates."
    fi
fi

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
