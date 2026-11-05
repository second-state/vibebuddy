#!/usr/bin/env python3
"""Tests for how the Linux installer gets the daemon access to the box's USB port (packaging/linux/serial-access.sh).

The script runs sudo, so nothing here may reach the real one. Every run gets a PATH holding only links to the plain
tools it needs and stand-ins for sudo, udevadm, systemctl, loginctl, getent and id: the real sudo isn't on it at all.
Paths under /etc, /usr/lib and /proc are redirected to a scratch directory with VIBEBUDDY_TEST_ROOT.
"""

from __future__ import annotations

import os
from pathlib import Path
import pty
import shutil
import stat
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parent.parent
LINUX = REPO / "packaging/linux"
SCRIPT = LINUX / "serial-access.sh"
RULE = LINUX / "70-vibebuddy.rules"
TOOLS = ["bash", "cat", "cmp", "cp", "cut", "dirname", "grep", "head", "install", "mkdir", "rm", "sed", "touch"]
STAND_INS = {
    # Each logs its arguments, then does what the test set in the environment.
    "sudo": 'echo "sudo $*" >> "$LOG"\n[ -z "$SUDO_FAILS" ] || exit 1\nexec "$@"\n',
    "udevadm": 'echo "udevadm $*" >> "$LOG"\n[ -z "$UDEVADM_FAILS" ] || exit 1\n',
    "systemctl": 'case "$*" in *MainPID*) echo "$DAEMON_PID";; esac\n',
    "loginctl": 'case "$1" in show-user) echo "$SESSION";; show-session) echo "$SEAT";; esac\n',
    "id": 'case "$1" in -nG) echo "$GROUPS_OF_USER";; -un) echo me;; esac\n',
    # In the order asked, printing only the groups that exist, and failing if one doesn't: getent's own behaviour.
    "getent": 'shift\nstatus=0\nfor name in "$@"; do\n'
              '  case " $EXISTING_GROUPS " in\n'
              '    *" $name:"*) for pair in $EXISTING_GROUPS; do\n'
              '        case "$pair" in "$name":*) echo "$name:x:${pair#*:}:";; esac\n'
              '      done;;\n'
              '    *) status=2;;\n'
              '  esac\ndone\nexit $status\n',
    "pkill": "exit 1\n",
}
GROUP_ADVICE = "sudo usermod -aG"


class Sandbox:
    def __init__(self, base: Path, omit: tuple[str, ...] = ()) -> None:
        self.base = base
        self.root = base / "root"
        self.root.mkdir()
        self.log = base / "commands.log"
        self.log.touch()
        self.path = base / "path"
        self.path.mkdir()
        for tool in TOOLS:
            (self.path / tool).symlink_to(shutil.which(tool))
        for name, body in STAND_INS.items():
            if name in omit:
                continue
            stand_in = self.path / name
            stand_in.write_text("#!/bin/sh\n" + body)
            stand_in.chmod(0o755)
        self.env = {
            "PATH": str(self.path),
            "USER": "me",
            "HOME": str(base / "home"),
            "VIBEBUDDY_TEST_ROOT": str(self.root),
            "LOG": str(self.log),
            # Ubuntu's picture: both groups exist, dialout owns the port, and the user is in neither.
            "EXISTING_GROUPS": "uucp:10 dialout:20",
            "GROUPS_OF_USER": "me sudo",
            "DAEMON_PID": "4242",
            "SESSION": "2",
            "SEAT": "seat0",
            "SUDO_FAILS": "",
            "UDEVADM_FAILS": "",
        }
        self.daemon_groups("1000 27")

    def daemon_groups(self, groups: str) -> None:
        status = self.root / "proc/4242/status"
        status.parent.mkdir(parents=True, exist_ok=True)
        status.write_text(f"Name:\tvibebuddyd\nGroups:\t{groups}\n")

    @property
    def local_rule(self) -> Path:
        return self.root / "etc/udev/rules.d/70-vibebuddy.rules"

    @property
    def packaged_rule(self) -> Path:
        return self.root / "usr/lib/udev/rules.d/70-vibebuddy.rules"

    def commands(self) -> list[str]:
        return self.log.read_text().splitlines()

    def run(self, answer: str | None = None, script: Path = SCRIPT) -> subprocess.CompletedProcess[str]:
        """Runs the script at a terminal that answers `answer`; with None, with no terminal at all."""
        if answer is None:
            return subprocess.run(["bash", str(script)], env=self.env, stdin=subprocess.DEVNULL,
                                  capture_output=True, text=True, timeout=30)
        controller, terminal = pty.openpty()
        try:
            os.write(controller, answer.encode() + b"\n")
            return subprocess.run(["bash", str(script)], env=self.env, stdin=terminal,
                                  capture_output=True, text=True, timeout=30)
        finally:
            os.close(terminal)
            os.close(controller)


class SerialAccess(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="vibebuddy-serial-access-")
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.box = Sandbox(self.base)

    def test_accepting_installs_the_rule_and_reaches_a_box_already_plugged_in(self) -> None:
        for answer in ["", "y", "Yes"]:
            with self.subTest(answer=answer), tempfile.TemporaryDirectory() as other:
                box = Sandbox(Path(other))
                result = box.run(answer)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(box.local_rule.read_bytes(), RULE.read_bytes())
                self.assertEqual(stat.S_IMODE(box.local_rule.stat().st_mode), 0o644)
                self.assertEqual(box.commands(), [
                    f"sudo install -Dm644 {RULE} {box.local_rule}",
                    "sudo udevadm control --reload-rules",
                    "udevadm control --reload-rules",
                    "sudo udevadm trigger --subsystem-match=tty --property-match=ID_VENDOR_ID=303a --property-match=ID_VENDOR_ID=1a86",
                    "udevadm trigger --subsystem-match=tty --property-match=ID_VENDOR_ID=303a --property-match=ID_VENDOR_ID=1a86",
                ])
                self.assertIn("no reboot needed", result.stdout)
                self.assertNotIn(GROUP_ADVICE, result.stdout)

    def test_the_commands_are_shown_before_anything_runs_as_root(self) -> None:
        result = self.box.run("n")
        self.assertIn(f"install -Dm644 {RULE} {self.box.local_rule}", result.stdout)
        self.assertIn("udevadm trigger", result.stdout)
        self.assertEqual(self.box.commands(), [])

    def test_declining_runs_nothing_as_root_and_names_the_group(self) -> None:
        result = self.box.run("n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.box.local_rule.exists())
        self.assertIn(f"{GROUP_ADVICE} dialout me", result.stdout)

    def test_ubuntu_is_told_to_join_dialout_not_uucp(self) -> None:
        # Debian and Ubuntu have both groups; the port belongs to dialout.
        result = self.box.run("n")
        self.assertIn("dialout", result.stdout)
        self.assertNotIn("uucp", result.stdout)

    def test_arch_is_told_to_join_uucp(self) -> None:
        self.box.env["EXISTING_GROUPS"] = "uucp:14"
        result = self.box.run("n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"{GROUP_ADVICE} uucp me", result.stdout)

    def test_with_no_terminal_nothing_runs_as_root_and_the_way_forward_is_printed(self) -> None:
        result = self.box.run()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.box.commands(), [])
        self.assertFalse(self.box.local_rule.exists())
        self.assertIn(f"install -Dm644 {RULE} {self.box.local_rule}", result.stdout)
        self.assertIn(f"{GROUP_ADVICE} dialout me", result.stdout)

    def test_piped_input_is_not_consent(self) -> None:
        # `yes | ./install.sh` must not end up running anything as root.
        result = subprocess.run(["bash", str(SCRIPT)], env=self.box.env, input="y\ny\n",
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.box.commands(), [])
        self.assertFalse(self.box.local_rule.exists())

    def test_with_no_sudo_at_a_terminal_it_prints_the_commands_for_root(self) -> None:
        other = self.base / "no-sudo"
        other.mkdir()
        box = Sandbox(other, omit=("sudo",))
        result = box.run("y")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(box.commands(), [])
        self.assertIn("run these as root", result.stdout)

    def test_a_failed_sudo_falls_back_to_the_group_and_does_not_stop_the_install(self) -> None:
        self.box.env["SUDO_FAILS"] = "1"
        result = self.box.run("y")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.box.local_rule.exists())
        self.assertIn("was not installed", result.stdout)
        self.assertIn(f"{GROUP_ADVICE} dialout me", result.stdout)

    def test_a_failed_trigger_keeps_the_rule_and_says_to_replug(self) -> None:
        self.box.env["UDEVADM_FAILS"] = "1"
        result = self.box.run("y")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.box.local_rule.exists())
        self.assertIn("replug", result.stdout)
        self.assertNotIn("no reboot needed", result.stdout)

    def test_a_current_rule_asks_nothing_on_an_upgrade(self) -> None:
        self.box.local_rule.parent.mkdir(parents=True)
        shutil.copy(RULE, self.box.local_rule)
        result = self.box.run("n")
        self.assertEqual((result.returncode, result.stdout, self.box.commands()), (0, "", []))

    def test_a_packages_rule_counts_as_current(self) -> None:
        self.box.packaged_rule.parent.mkdir(parents=True)
        self.box.packaged_rule.write_text("# the package's version, whatever it is\n")
        result = self.box.run("n")
        self.assertEqual((result.returncode, result.stdout, self.box.commands()), (0, "", []))

    def test_an_outdated_rule_is_offered_again(self) -> None:
        self.box.local_rule.parent.mkdir(parents=True)
        self.box.local_rule.write_text("# an earlier version\n")
        result = self.box.run("y")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.box.local_rule.read_bytes(), RULE.read_bytes())

    def test_a_daemon_that_holds_the_group_needs_nothing(self) -> None:
        self.box.env["GROUPS_OF_USER"] = "me dialout"
        self.box.daemon_groups("1000 20")
        result = self.box.run("n")
        self.assertEqual((result.returncode, result.stdout, self.box.commands()), (0, "", []))

    def test_the_daemons_groups_count_not_this_shells(self) -> None:
        # In the group, but the user manager started before that: the rule is the way out with no reboot.
        self.box.env["GROUPS_OF_USER"] = "me dialout"
        declined = self.box.run("n")
        self.assertIn("started before you were added", declined.stdout)
        self.assertNotIn(GROUP_ADVICE, declined.stdout)
        accepted = self.box.run("y")
        self.assertTrue(self.box.local_rule.exists())
        self.assertNotIn("started before you were added", accepted.stdout)

    def test_a_group_with_a_longer_name_is_not_mistaken_for_the_serial_one(self) -> None:
        self.box.env["GROUPS_OF_USER"] = "me dialout-admins"
        result = self.box.run("n")
        self.assertIn(f"{GROUP_ADVICE} dialout me", result.stdout)

    def test_with_no_session_at_a_seat_the_rule_is_said_not_to_cover_it(self) -> None:
        self.box.env["SESSION"] = ""
        self.box.local_rule.parent.mkdir(parents=True)
        shutil.copy(RULE, self.box.local_rule)
        result = self.box.run()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("this session isn't", result.stdout)
        self.assertIn(f"{GROUP_ADVICE} dialout me", result.stdout)
        self.assertEqual(self.box.commands(), [])

    def test_a_system_with_neither_group_still_finishes(self) -> None:
        self.box.env["EXISTING_GROUPS"] = ""
        result = self.box.run("n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn(GROUP_ADVICE, result.stdout)


class InstallerWiring(unittest.TestCase):
    """install.sh itself, run from a stand-in for the unpacked release tarball in a scratch HOME."""

    def unpacked_release(self, base: Path) -> Path:
        release = base / "release"
        (release / "bin").mkdir(parents=True)
        (release / "share/voices").mkdir(parents=True)
        for binary in ["vibebuddyd", "vibebuddy-hook", "vibebuddy-desktop"]:
            stand_in = release / "bin" / binary
            stand_in.write_text("#!/bin/sh\nexit 0\n")
            stand_in.chmod(0o755)
        (release / "share/voices/robot.bin").write_bytes(b"pack")
        shutil.copy(LINUX / "install.sh", release / "install.sh")
        for file in ["vibebuddyd.service", "vibebuddy.desktop", "vibebuddy.svg", "70-vibebuddy.rules", "serial-access.sh"]:
            shutil.copy(LINUX / file, release / "share" / file)
        return release

    def test_the_installer_offers_the_rule_and_finishes_whatever_the_answer(self) -> None:
        for answer, installed in [("y", True), ("n", False), (None, False)]:
            with self.subTest(answer=answer), tempfile.TemporaryDirectory(prefix="vibebuddy-install-") as temporary:
                base = Path(temporary)
                box = Sandbox(base)
                release = self.unpacked_release(base)
                result = box.run(answer, script=release / "install.sh")
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("Installed. Daemon log", result.stdout)
                self.assertTrue((base / "home/.local/bin/vibebuddyd").exists())
                self.assertEqual(box.local_rule.exists(), installed)
                self.assertEqual(box.local_rule.exists() or GROUP_ADVICE in result.stdout, True)

    def test_the_release_tarball_carries_what_the_installer_runs(self) -> None:
        tarball = (LINUX / "make-tarball.sh").read_text()
        self.assertIn("serial-access.sh", tarball)
        self.assertIn("70-vibebuddy.rules", tarball)


if __name__ == "__main__":
    unittest.main()
