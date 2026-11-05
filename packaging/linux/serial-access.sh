#!/usr/bin/env bash
# Run by install.sh: makes sure the daemon can open the box's USB port.
#
# The way to do that is the udev rule next to this script: it gives whoever is logged in at the computer access, with
# no group to join and no reboot, and the daemon, which keeps retrying the port, connects by itself. Installing the rule
# takes root, so this asks first and shows the commands. The serial group is the fallback for anyone who declines, and
# for machines only reached over SSH: uaccess follows a local session at a seat, and SSH has none.
#
# Nothing here stops the install: every path ends in telling the user where things stand.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
rule="${here}/70-vibebuddy.rules"
# Tests point this at a scratch directory; nothing else sets it.
root="${VIBEBUDDY_TEST_ROOT:-}"
local_rule="${root}/etc/udev/rules.d/70-vibebuddy.rules"
packaged_rule="${root}/usr/lib/udev/rules.d/70-vibebuddy.rules"
user="${USER:-$(id -un)}"
# dialout first: Debian and Ubuntu have both groups, and dialout is the one that owns the port there. Arch only has
# uucp. getent fails when either is missing.
serial_group="$({ getent group dialout uucp || true; } | head -n1 | cut -d: -f1)"

# The rule VibeBuddy ships is in force. A package's copy is the package's to keep up to date.
rule_is_current() {
    if [[ -e "${local_rule}" ]]; then
        cmp -s "${rule}" "${local_rule}"
    else
        [[ -e "${packaged_rule}" ]]
    fi
}

in_serial_group() {
    [[ -n "${serial_group}" && " $(id -nG) " == *" ${serial_group} "* ]]
}

# The daemon's own groups, not this shell's: it inherits them from the systemd user manager, which only picks up a new
# group when it restarts (LESSONS.md). With no daemon to ask, there is nothing to contradict the shell.
daemon_has_serial_group() {
    in_serial_group || return 1
    local gid pid status groups group
    gid="$(getent group "${serial_group}" | cut -d: -f3)"
    pid="$(systemctl --user show -p MainPID --value vibebuddyd.service 2>/dev/null || true)"
    status="${root}/proc/${pid}/status"
    [[ -r "${status}" ]] || return 0
    groups="$(grep '^Groups:' "${status}" || true)"
    for group in ${groups#Groups:}; do
        [[ "${group}" == "${gid}" ]] && return 0
    done
    return 1
}

at_the_seat() {
    local session
    session="$(loginctl show-user "${user}" -p Display --value 2>/dev/null || true)"
    [[ -n "${session}" && -n "$(loginctl show-session "${session}" -p Seat --value 2>/dev/null || true)" ]]
}

# The rule reaches a box that is already plugged in only when udev looks at it again. By USB ID, so that the computer's
# other serial ports are left alone.
retrigger() {
    sudo udevadm control --reload-rules \
        && sudo udevadm trigger --subsystem-match=tty --property-match=ID_VENDOR_ID=303a --property-match=ID_VENDOR_ID=1a86
}

explain_over_ssh() {
    at_the_seat && return 0
    daemon_has_serial_group && return 0
    [[ -n "${serial_group}" ]] || return 0
    echo
    echo "The udev rule covers whoever is logged in at this computer's screen, and this session isn't."
    echo "If you only reach this computer remotely, join the group instead, then reboot:"
    echo "  sudo usermod -aG ${serial_group} ${user}"
}

explain_the_group() {
    [[ -n "${serial_group}" ]] || return 0
    echo
    if ! in_serial_group; then
        echo "Without the udev rule, the daemon needs the ${serial_group} group to open the box. To join it, run this, then reboot:"
        echo "  sudo usermod -aG ${serial_group} ${user}"
    else
        echo "You are in ${serial_group}, but your systemd user session started before you were added,"
        echo "so the daemon cannot open the box yet. Reboot (or log out of every session) once."
    fi
}

show_the_commands() {
    local quoted
    printf -v quoted '%q' "${rule}"
    echo "  install -Dm644 ${quoted} ${local_rule}"
    echo "  udevadm control --reload-rules"
    echo "  udevadm trigger --subsystem-match=tty --property-match=ID_VENDOR_ID=303a --property-match=ID_VENDOR_ID=1a86"
}

if rule_is_current; then
    explain_over_ssh
    exit 0
fi
# Already working through the group: an upgrade doesn't need root for anything.
if daemon_has_serial_group; then
    exit 0
fi

# Without a terminal there is nobody to ask, and sudo would have nobody to prompt.
if [[ ! -t 0 ]] || ! command -v sudo >/dev/null || ! command -v udevadm >/dev/null; then
    echo
    echo "The daemon cannot open the box yet. To give whoever is logged in at this computer access, run these as root:"
    show_the_commands
    explain_the_group
    exit 0
fi

echo
echo "The daemon cannot open the box over USB yet. A udev rule gives whoever is logged in at this computer access,"
echo "with no group to join and no reboot. These run as root, through sudo:"
show_the_commands
read -r -p "Install the rule? [Y/n] " answer || answer=n
case "${answer}" in
    "" | [Yy]*) ;;
    *)
        explain_the_group
        exit 0
        ;;
esac

if ! sudo install -Dm644 "${rule}" "${local_rule}"; then
    echo "The udev rule was not installed."
    explain_the_group
    exit 0
fi
echo "Installed the udev rule."
if retrigger; then
    echo "The daemon connects to the box by itself, no reboot needed."
else
    echo "If the box is plugged in, replug it so the rule reaches it."
fi
explain_over_ssh
