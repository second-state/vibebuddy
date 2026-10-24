"""Run through tools/test-linux-host.sh; ordinary tool discovery skips this integration suite."""

from __future__ import annotations

import base64
from concurrent.futures import ThreadPoolExecutor
import json
import os
import re
import struct
import subprocess
import sys
import time
import unittest
from pathlib import Path

from host_test_support import HostHarness, REPO, wait_for
from make_voice_pack import build as build_voice_pack
from character_pack import build as build_character_pack

ENABLED = sys.platform.startswith("linux") and os.environ.get("VIBEBUDDY_HOST_TESTS") == "1"
FIXTURES = Path(__file__).with_name("fixtures") / "device"


@unittest.skipUnless(ENABLED, "run tools/test-linux-host.sh on Linux")
class HostTestCase(unittest.TestCase):
    firmware = True

    def setUp(self):
        self.host = HostHarness(self._testMethodName)
        self.addCleanup(self.host.close)
        self.host.use_session(self.session_environment())
        self.host.start(firmware=self.firmware, environment=self.daemon_environment())

    def daemon_environment(self):
        """Variables the daemon starts with; a test of the daemon's own evidence overrides this."""
        return {}

    def session_environment(self):
        """The agent's terminal's environment, which K2 reads the window system from: a live Hyprland by default;
        a desktop test overrides this."""
        signature = "test_instance"
        self.host.listening_socket(f"runtime/hypr/{signature}/.socket.sock")
        return {"HYPRLAND_INSTANCE_SIGNATURE": signature, "XDG_RUNTIME_DIR": str(Path(self.host.control_dir.name) / "runtime"),
                "WAYLAND_DISPLAY": "wayland-1", "XDG_SESSION_TYPE": "wayland"}

    def post(self, path, body=None, expected=202):
        code, response = self.host.request(path, body, method="POST")
        self.assertEqual(code, expected, response)
        return response

    def event(self, event_name, **fields):
        return self.post("/v1/events", {"version": 1, "event": event_name, **fields})

    def hook(self, name, session="test-session", pids=None):
        return self.post("/v1/claude-hooks", {
            "session_id": session, "prompt_id": "test-turn", "hook_event_name": name,
            "surface": "window", "host_pids": [self.host.pids.get(pid, pid) for pid in pids or [4242, 4000]],
        })


class LinuxHostTests(HostTestCase):
    def test_boot_identity_pairing_and_isolated_config(self):
        snapshot = self.host.status()
        device = snapshot["device"]
        self.assertFalse(device["foreign_firmware"])
        # PTYs have no USB metadata; the existing explicit-port path conservatively enables pacing.
        self.assertTrue(device["bridge"])
        self.assertFalse(device["network"])
        self.assertEqual(device["firmware_build"], "simulator")
        version = re.search(r'^version = "([^"]+)"', (REPO / "Cargo.toml").read_text(), re.MULTILINE)[1]
        self.assertEqual(device["firmware_version"], version)
        self.assertEqual(device["voice"], "builtin")
        # Box firmware names no board.
        self.assertIsNone(device["unsupported_board"])
        self.assertEqual(len(device["box_key"]), 43)
        self.assertTrue(any(computer["key"] == device["computer_key"] for computer in device["paired_computers"]))
        self.assertFalse(snapshot["config"]["notify_link"])
        self.assertFalse(snapshot["config"]["check_updates"])
        self.assertFalse(snapshot["config"]["relay"])

    def test_explicit_board_diagnostic_reaches_host(self):
        # A build for other hardware names its board; the host follows what the device says.
        self.host.emit("BOARD goouuu-s3-spi\n")
        self.host.wait_device(lambda device: device["unsupported_board"] == "goouuu-s3-spi", "board diagnostic")

    def test_volume_identify_and_device_menu_updates(self):
        self.post("/v1/device/identify")
        self.host.wait_line("IDENTIFY\n")
        self.post("/v1/device/volume", {"level": 40, "preview": True})
        self.host.wait_device(lambda device: device["volume"] == 40, "volume acknowledgment")
        self.host.wait_line("AUDIO QUEUED DONE")
        self.post("/v1/device/volume", {"level": 0}, expected=400)
        self.host.emit("VOLUME ERROR\r\nVOLUME 75\r\n")
        self.host.wait_device(lambda device: device["volume"] == 75, "unsolicited volume")

    def test_http_to_firmware_all_activity_states(self):
        for event, title, output in (
            ("task.start", "WORK", "DISPLAY STATE WORKING"),
            ("agent.input_required", "ASK", "DISPLAY STATE INPUT REQUIRED"),
            ("task.error", "FAIL", "DISPLAY STATE FAILED"),
            ("task.done", "DONE", "DISPLAY STATE DONE"),
        ):
            with self.subTest(event=event):
                self.event(event, title=title)
                self.host.wait_line(output)
        self.host.wait_line("AUDIO QUEUED INPUT_REQUIRED")
        self.host.wait_line("AUDIO QUEUED FAILED")

    def test_k2_routes_active_completed_and_persisted_source(self):
        self.host.command_reply("hyprctl", clients=[{"pid": 4242, "address": "0xabc"}])
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: self.host.commands("hyprctl"), "K2 opens window")
        wait_for(lambda: any("0xabc" in " ".join(command["args"]) for command in self.host.commands("hyprctl")),
                 "correct source focused")
        self.hook("Stop")
        before = len(self.host.commands("hyprctl"))
        self.host.press("K2")
        wait_for(lambda: len(self.host.commands("hyprctl")) > before, "completed source still opens")
        self.host.stop_daemon()
        self.host.start_daemon()
        self.host.wait_device(lambda device: device["paired"], "daemon restart handshake", timeout=12)
        before = len(self.host.commands("hyprctl"))
        self.host.press("K2")
        wait_for(lambda: len(self.host.commands("hyprctl")) > before, "source survives daemon restart")

    def test_k2_prefers_needs_input_over_other_work(self):
        self.host.command_reply("hyprctl", clients=[
            {"pid": 4242, "address": "0xaaa"}, {"pid": 4343, "address": "0xbbb"},
        ])
        self.hook("UserPromptSubmit", session="needs-input")
        self.hook("PermissionRequest", session="needs-input")
        self.hook("UserPromptSubmit", session="other-work", pids=[4343])
        self.host.press("K2")
        wait_for(lambda: any("0xaaa" in " ".join(command["args"]) for command in self.host.commands("hyprctl")),
                 "K2 selects needs input")
        self.assertFalse(any("0xbbb" in " ".join(command["args"]) for command in self.host.commands("hyprctl")))

    def test_k2_uses_nearest_ancestor_and_legacy_hyprland_fallback(self):
        self.host.command_reply("hyprctl", clients=[
            {"pid": 4000, "address": "0xaaa"}, {"pid": 4242, "address": "0xbbb"},
        ], lua_exit=1, lua_stdout="unknown Lua dispatch")
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: any(command["args"] == ["dispatch", "focuswindow", "address:0xbbb"]
                             for command in self.host.commands("hyprctl")), "legacy focus of nearest ancestor")

    def test_k2_failed_desktop_command_recovers_on_next_press(self):
        self.host.command_reply("hyprctl", exit=1, stdout="compositor unavailable")
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: "compositor unavailable" in (self.host.root / "daemon.log").read_text(), "focus failure logged")
        self.host.command_reply("hyprctl", clients=[{"pid": 4242, "address": "0xabc"}])
        self.host.press("K2")
        wait_for(lambda: any("0xabc" in " ".join(command["args"]) for command in self.host.commands("hyprctl")),
                 "next K2 recovers")

    def test_long_press_and_menu_consume_k2_locally(self):
        self.hook("UserPromptSubmit")
        self.host.press("K2", long=True)
        self.host.wait_line("MUTE ON")
        self.host.press("K1", long=True)
        self.host.wait_line("MENU OPEN")
        self.host.press("K2")
        self.host.emit("VOLUME 55\n")
        self.host.wait_device(lambda device: device["volume"] == 55, "device output drained")
        self.assertEqual(self.host.commands("hyprctl"), [])
        self.assertNotIn('"event":"button"', self.host.lines())

    def test_k0_k1_pomodoro_and_clock_are_local(self):
        self.host.press("K1")
        self.host.wait_device(lambda device: device["mode"] == "pomodoro", "K1 mode")
        self.host.press("K0")
        self.host.wait_line("POMODORO FOCUS START")
        self.host.press("K0")
        self.host.wait_line("POMODORO PAUSED")
        self.host.press("K0")
        self.host.wait_line("POMODORO RESUMED")
        self.host.control("advance", ms=25 * 60 * 1000 + 100)
        self.host.wait_line("POMODORO FOCUS END")
        self.assertEqual(self.host.commands("hyprctl"), [])

    def test_unplug_replug_relearns_state_and_keeps_device_storage(self):
        self.post("/v1/device/volume", {"level": 45})
        old = self.host.wait_device(lambda device: device["volume"] == 45, "volume stored")["device"]["box_key"]
        self.host.disconnect()
        self.host.wait_device(lambda device: not device["connected"], "unplug")
        self.host.connect()
        self.host.wait_device(lambda device: device["connected"], "replug")
        self.host.control("reboot")
        snapshot = self.host.wait_device(lambda device: device["paired"] and device["volume"] == 45, "reboot restores NVS")
        self.assertEqual(snapshot["device"]["box_key"], old)

    def test_pair_list_and_unpair_round_trip(self):
        other = base64.urlsafe_b64encode(bytes(range(32))).decode().rstrip("=")
        self.event("device.pair", key=other, name="Simulated second computer")
        self.event("device.pairs")
        self.host.wait_device(lambda device: any(pair["key"] == other for pair in device["paired_computers"]), "second pair")
        self.post("/v1/device/unpair", {"key": other})
        self.host.wait_device(lambda device: all(pair["key"] != other for pair in device["paired_computers"]), "unpaired")
        own = self.host.status()["device"]["computer_key"]
        self.post("/v1/device/unpair", {"key": own}, expected=409)

    def test_wifi_device_layer_signals_reach_host(self):
        fixture = json.loads((FIXTURES / "wifi.json").read_text())
        self.post("/v1/device/wifi", {"ssid": "Test network", "password": "test-only"})
        self.host.wait_line("WIFI NETWORK Test network")
        self.post("/v1/device/wifi/scan")
        self.host.emit(fixture["scan"])
        snapshot = self.host.wait_device(lambda device: len(device["wifi_seen"]) == 2, "scan results")
        self.assertEqual([item["ssid"] for item in snapshot["device"]["wifi_seen"]], ["Office test", "Test network"])
        self.host.emit(fixture["failed"])
        self.host.wait_device(lambda device: bool(device["wifi_error"]), "Wi-Fi failure")
        self.host.emit(fixture["joined"])
        self.host.wait_device(lambda device: device["wifi_address"] == "192.0.2.10" and not device["wifi_error"], "Wi-Fi recovered")
        self.host.emit("WIFI DISCONNECTED\n")
        self.host.wait_device(lambda device: device["wifi_address"] is None, "Wi-Fi disconnect")
        self.post("/v1/device/wifi/forget")
        self.host.wait_device(lambda device: device["wifi_network"] is None, "Wi-Fi forgotten")

    @staticmethod
    def pack():
        return build_voice_pack("test-voice", [bytes([index, 0, index, 0]) * 64 for index in range(5)])

    def test_voice_pack_acknowledgments_and_persistence(self):
        self.post("/v1/device/voice-pack", self.pack())
        snapshot = self.host.wait_operation("done")
        self.assertEqual(snapshot["config"]["voice"], "test-voice")
        self.host.wait_device(lambda device: device["voice"] == "test-voice", "written voice reported")
        self.host.control("reboot")
        self.host.wait_device(lambda device: device["paired"] and device["voice"] == "test-voice", "voice survives reset")

    def test_character_pack_uses_the_same_write_protocol(self):
        pack = build_character_pack("test-character", {"done": [[0] * 128]})
        self.post("/v1/device/voice-pack", pack)
        snapshot = self.host.wait_operation("done")
        self.assertEqual(snapshot["config"]["voice"], "test-character")
        self.host.wait_device(lambda device: device["voice"] == "test-character", "Character installed")

    # Firmware that reports TRANSFER 1, as the firmware core does, takes packs as xfer.* chunks (ADR-0016,
    # docs/protocol.md "Firmware and Character packs over the link").
    def big_pack(self):
        """A pack of 19 chunks of 3072 bytes, just past the daemon's window of 16, so a transfer can stop partway. The
        PTY counts as a UART bridge, whose pacing makes each chunk take about half a second."""
        return build_voice_pack("test-voice", [bytes([index, 0, index, 0]) * 2816 for index in range(5)])

    def ack(self, offset):
        return f'{{"version":1,"event":"xfer.ack","offset":{offset}}}\n'

    def test_pack_larger_than_the_voice_partition_is_refused(self):
        self.post("/v1/device/voice-pack", self.pack() + bytes(0x200000))
        snapshot = self.host.wait_operation("failed")
        self.assertIn("the box refused the image: too_big", snapshot["operation"]["message"])
        self.assertNotIn('"event":"xfer.chunk"', self.host.lines("host"))

    def test_a_refusal_partway_fails_the_write_one_at_a_time_and_a_retry_succeeds(self):
        self.host.filter(drop=('{"version":1,"event":"xfer.ack"',))
        self.post("/v1/device/voice-pack", self.big_pack())
        self.host.wait_line('"event":"xfer.ack"', timeout=30)  # the box acknowledged; the daemon didn't hear it
        self.post("/v1/device/voice-pack", self.pack(), expected=409)
        self.host.emit('{"version":1,"event":"xfer.error","code":"crc"}\n')
        snapshot = self.host.wait_operation("failed")
        self.assertIn("the box refused the image: crc", snapshot["operation"]["message"])
        self.assertIsNone(snapshot["config"]["voice"])
        self.host.filter()
        self.post("/v1/device/voice-pack", self.pack())
        self.assertEqual(self.host.wait_operation("done")["config"]["voice"], "test-voice")

    def test_the_daemon_keeps_at_most_a_window_of_chunks_unacknowledged(self):
        self.host.filter(drop=('{"version":1,"event":"xfer.ack"',))
        pack = self.big_pack()
        self.post("/v1/device/voice-pack", pack)
        wait_for(lambda: self.host.lines("host").count('"event":"xfer.chunk"') >= 16, "a window of chunks", timeout=30)
        time.sleep(1)  # 40 chunks a second: had the daemon not stopped, more would have gone by now
        self.assertEqual(self.host.lines("host").count('"event":"xfer.chunk"'), 16)
        self.host.filter()
        self.host.emit(self.ack(16 * 3072))
        self.assertEqual(self.host.wait_operation("done", timeout=30)["config"]["voice"], "test-voice")

    def test_a_lost_acknowledgement_asks_the_box_where_it_got_to(self):
        self.host.filter(drop=('{"version":1,"event":"xfer.ack"',))
        self.post("/v1/device/voice-pack", self.pack())
        self.host.wait_line('"event":"xfer.ack"')
        self.host.filter()
        # 20 s without an acknowledgement: the daemon sends xfer.begin again, and the box answers with its offset.
        wait_for(lambda: self.host.lines("host").count('"event":"xfer.begin"') >= 2, "the daemon asks again", timeout=30)
        snapshot = self.host.wait_operation("done", timeout=30)
        self.assertEqual(snapshot["config"]["voice"], "test-voice")
        self.assertEqual(self.host.lines("host").count('"event":"xfer.chunk"'), 1, "the chunk the box had isn't sent again")

    def test_stale_and_repeated_acknowledgements_change_nothing(self):
        self.host.filter(drop=('{"version":1,"event":"xfer.ack"',))
        pack = self.pack()
        self.post("/v1/device/voice-pack", pack)
        self.host.wait_line('"event":"xfer.ack"')
        self.host.filter()
        # Older offsets, repeated, before the one that completes it: the daemon keeps the furthest.
        self.host.emit(self.ack(0) + self.ack(0) + self.ack(len(pack)))
        snapshot = self.host.wait_operation("done")
        self.assertEqual(snapshot["config"]["voice"], "test-voice")
        self.assertEqual(self.host.lines("host").count('"event":"xfer.end"'), 1)

    def test_an_unplug_that_restarts_the_box_partway_fails_the_write_and_a_retry_succeeds(self):
        # What the daemon does today, not what ADR-0016 promises. The cable powers the box, so it restarts before the
        # replug and forgets the transfer. The daemon then sends the chunks it had in flight, the box answers
        # `xfer.error` `sequence`, and the daemon gives up instead of starting over with `xfer.begin`. The rest of the
        # queued chunks keep drawing `sequence`, and the daemon takes any `xfer.error` as the answer to its current
        # transfer, so a retry started before they stop fails too. Suspected bug in daemon/src/transfer.rs, reported on
        # #153: when it resumes, assert "done" here instead and drop the wait.
        self.post("/v1/device/voice-pack", self.big_pack())
        wait_for(lambda: self.host.lines("host").count('"event":"xfer.chunk"') >= 4, "the write under way", timeout=30)
        self.host.disconnect()
        self.host.wait_device(lambda device: not device["connected"], "unplug")
        self.assertEqual(self.host.status()["operation"]["state"], "running", "the write waits for the box")
        self.host.control("reboot")
        self.host.connect()
        snapshot = self.host.wait_operation("failed", timeout=60)
        self.assertIn("the box refused the image: sequence", snapshot["operation"]["message"])
        errors = lambda: self.host.lines("device").count('"event":"xfer.error"')
        wait_for(lambda: (before := errors()) and (time.sleep(2) or errors() == before), "the stale chunks to stop", timeout=30)
        self.post("/v1/device/voice-pack", self.pack())
        self.assertEqual(self.host.wait_operation("done", timeout=30)["config"]["voice"], "test-voice")

    def test_sse_delivers_device_changes_and_config(self):
        request = self.host.opener.open(self.host.base + "/v1/status/stream", timeout=5)
        with request:
            self.post("/v1/device/volume", {"level": 35})
            for _ in range(100):
                line = request.readline()
                if line.startswith(b"data:"):
                    snapshot = json.loads(line[5:])
                    if snapshot["device"]["volume"] == 35:
                        self.assertFalse(snapshot["config"]["relay"])
                        break
            else:
                self.fail("SSE never delivered the volume acknowledgment")

    def test_linux_app_status_and_tray_model_consumes_live_snapshots(self):
        snapshots = [self.host.status()]
        self.host.press("K1")
        snapshots.append(self.host.wait_device(lambda device: device["mode"] == "pomodoro", "mode for Linux app"))
        self.host.disconnect()
        snapshots.append(self.host.wait_device(lambda device: not device["connected"], "offline for Linux app"))
        snapshots.append(None)  # What the app derives when the daemon stops answering.
        probe = os.environ.get("VIBEBUDDY_TEST_DESKTOP_PROBE", str(REPO / "target/debug/examples/host_status_probe"))
        result = subprocess.run([probe], input="".join(json.dumps(snapshot) + "\n" for snapshot in snapshots),
                                text=True, capture_output=True, env=self.host.env, timeout=8, check=True)
        menus = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual([menu["icon"] for menu in menus], ["online", "online", "offline", "daemon_down"])
        self.assertEqual(menus[1]["mode_line"], "Mode: Pomodoro")
        self.assertIn("click to restart", menus[3]["device_line"])

    def test_status_names_the_supervisor_and_restart_refuses_without_one(self):
        # The harness runs the daemon by hand, so it isn't the systemd service, and exiting would only stop it.
        self.assertEqual(self.host.status()["daemon"]["supervisor"], "other")
        code, response = self.host.request("/v1/daemon/restart", method="POST")
        self.assertEqual(code, 409, response)
        self.assertIn(NOT_THE_SERVICE, response["message"])
        time.sleep(0.5)  # longer than the daemon waits before exiting on a restart it accepts
        self.assertIsNone(self.host.daemon.poll())
        self.assertEqual(self.host.status()["daemon"]["supervisor"], "other")

    def test_malformed_fragmented_and_coalesced_device_output(self):
        self.host.filter(fragment=3)
        self.host.emit(b'not-json\n{"version":99,"event":"button","button":"K2","action":"press"}\n'
                       b'{broken json}\nVOLUME ERROR\nVOLUME 2')
        # Separate writes exercise the daemon's partial-line buffer across reads.
        self.host.emit(b'5\r\nMODE POMODORO\r\n')
        self.host.wait_device(lambda device: device["volume"] == 25 and device["mode"] == "pomodoro", "fragmented lines")
        self.assertEqual(self.host.commands("hyprctl"), [])
        self.host.emit(b'x' * 9000 + b'\nVOLUME 60\n')
        self.host.wait_device(lambda device: device["volume"] == 60, "oversized diagnostic recovery")

    def test_incomplete_screenshot_is_rejected(self):
        self.host.filter(drop=("SHOT",))
        with ThreadPoolExecutor(max_workers=1) as executor:
            capture = executor.submit(self.host.request, "/v1/device/screenshot", method="POST")
            self.host.wait_line("SHOT END")
            self.host.emit("SHOT BEGIN 320x240 BACKLIGHT ON\nSHOT 0000:10\nSHOT END\n")
            code, response = capture.result(timeout=8)
        self.assertEqual(code, 502, response)
        self.assertIn(b"incomplete frame: 10 pixels", response)

    def test_malformed_screenshot_becomes_an_http_error(self):
        self.host.filter(drop=("SHOT",))
        with ThreadPoolExecutor(max_workers=1) as executor:
            capture = executor.submit(self.host.request, "/v1/device/screenshot", method="POST")
            self.host.wait_line("SHOT END")
            self.host.emit("SHOT BEGIN 320x240 BACKLIGHT ON\nSHOT invalid:12\nSHOT END\n")
            code, response = capture.result(timeout=8)
        self.assertEqual(code, 502, response)
        self.assertIn(b"bad color", response)

    def test_box_screenshot_is_a_320_by_240_png(self):
        code, png = self.host.request("/v1/device/screenshot", method="POST")
        self.host.wait_line("SHOT END")
        self.assertEqual(code, 200, png)
        self.assertEqual(png[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual(struct.unpack(">II", png[16:24]), (320, 240))


NOTIFY_ARGS = ["--app-name=VibeBuddy", "VibeBuddy",
               "The daemon stopped and didn't come back. Restart it from the tray icon or on the Advanced tab."]
NOTIFY_YOURSELF_ARGS = ["--app-name=VibeBuddy", "VibeBuddy",
                        "The daemon stopped and didn't come back. It didn't run as the vibebuddyd systemd service, "
                        "so start it again the way you started it."]
NOT_THE_SERVICE = "doesn't run as the vibebuddyd systemd service"
# The status stream retries every 2 s, so a brief restart must fit inside one retry plus the grace period.
PROBE_GRACE_MS = 4000
PROBE_WAIT = PROBE_GRACE_MS / 1000 + 8


@unittest.skipUnless(ENABLED, "run tools/test-linux-host.sh on Linux")
class DaemonOutageTests(HostTestCase):
    """The Linux app's daemon-failure notification: the real status stream drives the outage policy via the probe."""
    firmware = False

    def daemon_environment(self):
        # As installed: the daemon runs as the systemd service, so the unit's state is about it.
        return {"VIBEBUDDY_SUPERVISOR": "systemd"}

    def setUp(self):
        super().setUp()
        self.decision_log = self.host.root / "outage-decisions.log"
        # The harness keeps notifications off for the other tests; the app's toggle is on for these.
        config = self.host.status()["config"]
        config["notify_link"] = True
        code, response = self.host.request("/v1/config", config, method="PUT")
        self.assertEqual(code, 200, response)

    def start_probe(self, grace_ms=PROBE_GRACE_MS):
        probe = os.environ.get("VIBEBUDDY_TEST_OUTAGE_PROBE", str(REPO / "target/debug/examples/daemon_outage_probe"))
        with self.decision_log.open("w") as log:
            process = subprocess.Popen([probe, self.host.base, str(grace_ms)], cwd=self.host.root,
                                       env=self.host.env, stdout=log, stderr=subprocess.STDOUT)

        def stop():
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=3)

        self.addCleanup(stop)

    def decisions(self):
        return self.decision_log.read_text().splitlines() if self.decision_log.exists() else []

    def count(self, text):
        return sum(text in line for line in self.decisions())

    def wait_count(self, text, n, timeout=12):
        return wait_for(lambda: self.count(text) >= n, f"{n} decision(s) containing {text!r}", timeout)

    def assert_no_check_or_notify(self):
        self.assertFalse(any("CheckService" in line or "Notify" in line for line in self.decisions()), self.decisions())

    def vibebuddy_notifications(self):
        # The daemon's own box alert also goes through notify-send, under another title.
        return [command["args"] for command in self.host.commands("notify-send") if command["args"][1:2] == ["VibeBuddy"]]

    def await_notifications(self, n):
        return wait_for(lambda: (sent if len(sent := self.vibebuddy_notifications()) >= n else None),
                        f"{n} VibeBuddy notification(s)", 8)

    def test_a_brief_restart_within_the_grace_period_stays_silent(self):
        self.start_probe()
        self.wait_count("Status { notify_link: true, supervisor: Some(Systemd) } -> None", 1)
        self.host.stop_daemon()
        self.wait_count("Down -> Some(Wait", 1)
        self.host.start_daemon()
        self.wait_count("Status { notify_link: true, supervisor: Some(Systemd) } -> None", 2)
        self.wait_count("GraceElapsed -> None", 1, timeout=PROBE_WAIT)
        self.assert_no_check_or_notify()
        self.assertEqual(self.vibebuddy_notifications(), [])

    def test_a_sustained_outage_notifies_once(self):
        self.start_probe()
        self.wait_count("Status {", 1)
        self.host.stop_daemon()
        self.wait_count("-> Some(Notify(Here))", 1, timeout=PROBE_WAIT)
        # Retries keep failing after the notification; none of them may notify again.
        self.wait_count("Down -> None", 3, timeout=PROBE_WAIT)
        self.assertEqual(self.count("-> Some(Notify(Here))"), 1)
        self.assertEqual(self.await_notifications(1), [NOTIFY_ARGS])
        self.assertEqual([command["args"] for command in self.host.commands("systemctl")],
                         [["--user", "is-active", "vibebuddyd"]])

    def test_recovery_rearms_the_notification_for_the_next_outage(self):
        self.start_probe()
        self.wait_count("Status {", 1)
        self.host.stop_daemon()
        self.wait_count("-> Some(Notify(Here))", 1, timeout=PROBE_WAIT)
        self.host.start_daemon()
        self.wait_count("Status {", 2)
        self.host.stop_daemon()
        self.wait_count("-> Some(Notify(Here))", 2, timeout=PROBE_WAIT)
        self.assertEqual(self.await_notifications(2), [NOTIFY_ARGS, NOTIFY_ARGS])

    def test_the_notify_toggle_silences_an_outage_after_the_daemon_is_gone(self):
        self.start_probe()
        self.wait_count("Status {", 1)
        config = self.host.status()["config"]
        config["notify_link"] = False
        code, response = self.host.request("/v1/config", config, method="PUT")
        self.assertEqual(code, 200, response)
        self.wait_count("Status { notify_link: false, supervisor: Some(Systemd) } -> None", 1)
        self.host.stop_daemon()
        self.wait_count("GraceElapsed -> None", 1, timeout=PROBE_WAIT)
        self.assert_no_check_or_notify()
        self.assertEqual(self.vibebuddy_notifications(), [])

    def test_without_any_status_the_notification_is_on(self):
        self.host.stop_daemon()
        self.start_probe()
        self.wait_count("-> Some(Notify(Here))", 1, timeout=PROBE_WAIT)
        self.assertEqual(self.await_notifications(1), [NOTIFY_ARGS])

    def test_a_unit_stopped_on_purpose_stays_silent(self):
        self.host.command_reply("systemctl", stdout="inactive\n", exit=3)
        self.start_probe()
        self.wait_count("Status {", 1)
        self.host.stop_daemon()
        self.wait_count("Service(Inactive) -> Some(Wait", 2, timeout=PROBE_WAIT + PROBE_GRACE_MS / 1000)
        self.assertEqual(self.vibebuddy_notifications(), [])

    def test_a_failed_restart_after_a_stop_notifies_without_recovering(self):
        self.host.command_reply("systemctl", stdout="inactive\n", exit=3)
        self.start_probe()
        self.wait_count("Status {", 1)
        self.host.stop_daemon()
        self.wait_count("Service(Inactive) -> Some(Wait", 1, timeout=PROBE_WAIT)
        self.host.command_reply("systemctl", stdout="failed\n", exit=3)
        self.wait_count("Service(Failed) -> Some(Notify(Here))", 1, timeout=PROBE_WAIT)
        self.assertEqual(self.await_notifications(1), [NOTIFY_ARGS])

    def test_restart_uses_systemd_when_the_daemon_is_down_and_reports_failure(self):
        self.host.stop_daemon()
        probe = os.environ.get("VIBEBUDDY_TEST_OUTAGE_PROBE", str(REPO / "target/debug/examples/daemon_outage_probe"))
        self.host.command_reply("systemctl", stdout="", exit=0)
        result = subprocess.run([probe, "--restart"], env=self.host.env, capture_output=True, text=True, timeout=12)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.host.commands("systemctl")[-1]["args"], ["--user", "restart", "vibebuddyd"])
        self.host.command_reply("systemctl", stderr="restart refused", exit=1)
        result = subprocess.run([probe, "--restart", "systemd"], env=self.host.env, capture_output=True, text=True, timeout=12)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("restart refused", result.stderr)

    def test_restart_refuses_a_daemon_outside_the_service(self):
        # systemctl would restart the unit, not this daemon; the daemon's own endpoint would only stop it.
        self.host.stop_daemon()
        probe = os.environ.get("VIBEBUDDY_TEST_OUTAGE_PROBE", str(REPO / "target/debug/examples/daemon_outage_probe"))
        result = subprocess.run([probe, "--restart", "other"], env=self.host.env, capture_output=True, text=True, timeout=12)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(NOT_THE_SERVICE, result.stderr)
        self.assertEqual(self.host.commands("systemctl"), [])

    def test_a_daemon_outside_the_service_notifies_without_asking_systemd(self):
        # Started by hand, the unit is `inactive` whatever this daemon does; that must not read as a deliberate stop.
        self.host.command_reply("systemctl", stdout="inactive\n", exit=3)
        self.host.stop_daemon()
        self.host.env["VIBEBUDDY_SUPERVISOR"] = "other"
        self.host.start_daemon()
        self.start_probe()
        self.wait_count("supervisor: Some(Other) } -> None", 1)
        self.host.stop_daemon()
        self.wait_count("GraceElapsed -> Some(Notify(Yourself))", 1, timeout=PROBE_WAIT)
        self.assertEqual(self.await_notifications(1), [NOTIFY_YOURSELF_ARGS])
        self.assertEqual(self.host.commands("systemctl"), [])

    def test_a_crash_looping_unit_notifies(self):
        self.host.command_reply("systemctl", stdout="failed\n", exit=3)
        self.start_probe()
        self.wait_count("Status {", 1)
        self.host.stop_daemon()
        self.wait_count("Service(Failed) -> Some(Notify(Here))", 1, timeout=PROBE_WAIT)
        self.assertEqual(self.await_notifications(1), [NOTIFY_ARGS])

    def test_an_unreadable_unit_state_still_notifies(self):
        # No user bus or missing systemd: systemctl prints nothing and exits non-zero.
        self.host.command_reply("systemctl", stdout="", exit=1)
        self.start_probe()
        self.wait_count("Status {", 1)
        self.host.stop_daemon()
        self.wait_count("Service(Unknown) -> Some(Notify(Here))", 1, timeout=PROBE_WAIT)
        self.assertEqual(self.await_notifications(1), [NOTIFY_ARGS])

    def test_a_slow_systemctl_times_out_and_still_notifies(self):
        self.host.command_reply("systemctl", stdout="active\n", sleep=3)
        self.start_probe()
        self.wait_count("Status {", 1)
        self.host.stop_daemon()
        self.wait_count("Service(Unknown) -> Some(Notify(Here))", 1, timeout=PROBE_WAIT + 2)
        self.assertEqual(self.await_notifications(1), [NOTIFY_ARGS])


WINDOW_FIXTURES = Path(__file__).with_name("fixtures") / "linux-windows"
SWAY_TREE = (WINDOW_FIXTURES / "sway-get-tree.json").read_text()
WMCTRL_LIST = (WINDOW_FIXTURES / "wmctrl-list.txt").read_text()
NO_WINDOW = "has no host window"
NO_WINDOW_SYSTEM = "runs no supported window system"


@unittest.skipUnless(ENABLED, "run tools/test-linux-host.sh on Linux")
class SwayFocusTests(HostTestCase):
    """K2 on Sway: the daemon lists windows with `swaymsg -t get_tree` and focuses one by container id."""

    def session_environment(self):
        # The daemon only believes in a Sway socket that accepts a connection.
        self.sway_socket = self.host.listening_socket("sway.sock")
        return {"SWAYSOCK": str(self.sway_socket), "WAYLAND_DISPLAY": "wayland-1", "XDG_SESSION_TYPE": "wayland"}

    def restart_daemon_with(self, **variables):
        self.host.stop_daemon()
        self.host.env.update(variables)
        self.host.start_daemon()
        self.host.wait_device(lambda device: device["paired"], "daemon restart handshake", timeout=12)

    def answer_sway(self, tree=SWAY_TREE, focus_reply='[{"success": true}]'):
        self.host.command_reply("swaymsg", args=["-t", "get_tree"], stdout=tree)
        self.host.command_reply("swaymsg", args=["[con_id=17] focus"], stdout=focus_reply)
        self.host.command_reply("swaymsg", args=["[con_id=9] focus"], stdout=focus_reply)

    def focus_calls(self):
        return [command["args"] for command in self.host.commands("swaymsg") if command["args"][:1] != ["-t"]]

    def log_text(self):
        return (self.host.root / "daemon.log").read_text()

    def test_k2_focuses_the_nearest_ancestor_window_by_container(self):
        self.answer_sway()
        self.hook("UserPromptSubmit")  # host pids 4242, then 4000
        self.host.press("K2")
        wait_for(lambda: self.focus_calls() == [["[con_id=17] focus"]], "Sway focuses the nearest ancestor")
        self.assertEqual(self.host.commands("hyprctl"), [])

    def test_k2_prefers_needs_input_over_other_work(self):
        self.answer_sway()
        self.hook("UserPromptSubmit", session="needs-input", pids=[4000])
        self.hook("PermissionRequest", session="needs-input", pids=[4000])
        self.hook("UserPromptSubmit", session="other-work", pids=[4242])
        self.host.press("K2")
        wait_for(lambda: self.focus_calls() == [["[con_id=9] focus"]], "K2 selects needs input")

    def test_a_completed_source_survives_a_daemon_restart(self):
        self.answer_sway()
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: len(self.focus_calls()) == 1, "first focus")
        self.hook("Stop")
        self.host.press("K2")
        wait_for(lambda: len(self.focus_calls()) == 2, "completed source still focused")
        self.restart_daemon_with()
        self.host.press("K2")
        wait_for(lambda: len(self.focus_calls()) == 3, "source survives daemon restart")
        self.assertEqual(self.focus_calls(), [["[con_id=17] focus"]] * 3)

    def test_a_window_missing_from_the_tree_focuses_nothing(self):
        self.answer_sway(tree='{"id": 1, "type": "root", "nodes": [], "floating_nodes": []}')
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: NO_WINDOW in self.log_text(), "missing window logged")
        self.assertEqual(self.focus_calls(), [])
        self.assertEqual(self.host.commands("hyprctl"), [])

    def test_malformed_tree_is_treated_as_no_window(self):
        self.answer_sway(tree="not json")
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: NO_WINDOW in self.log_text(), "malformed tree logged")
        self.assertEqual(self.focus_calls(), [])
        self.assertEqual(self.host.commands("hyprctl"), [])

    def test_a_refused_focus_is_logged_and_the_next_press_recovers(self):
        self.answer_sway(focus_reply='[{"success": false, "error": "No node with id 17"}]')
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: "No node with id 17" in self.log_text(), "refused focus logged")
        self.host.command_reply("swaymsg", args=["[con_id=17] focus"], stdout='[{"success": true}]')
        self.host.press("K2")
        wait_for(lambda: len(self.focus_calls()) == 2, "next K2 recovers")

    def test_a_failing_swaymsg_is_logged_with_its_stderr(self):
        self.host.command_reply("swaymsg", args=["-t", "get_tree"], exit=1, stderr="unable to connect to the IPC socket")
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: "unable to connect to the IPC socket" in self.log_text(), "swaymsg stderr logged")
        self.assertEqual(self.focus_calls(), [])

    def test_a_slow_swaymsg_times_out_and_is_logged(self):
        self.host.command_reply("swaymsg", args=["-t", "get_tree"], stdout=SWAY_TREE, sleep=6)
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: "swaymsg timed out" in self.log_text(), "timeout logged", timeout=12)
        self.assertEqual(self.focus_calls(), [])

    def test_a_stale_sway_socket_focuses_nothing(self):
        stale = self.host.root / "gone.sock"
        stale.touch()  # what a crashed compositor leaves: the file, with nobody listening
        self.host.use_session({"SWAYSOCK": str(stale), "WAYLAND_DISPLAY": "wayland-1", "XDG_SESSION_TYPE": "wayland"})
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: NO_WINDOW_SYSTEM in self.log_text(), "no window system logged")
        self.assertEqual(self.host.commands("swaymsg"), [])
        self.assertEqual(self.host.commands("hyprctl"), [])

    def test_the_agents_session_wins_over_a_hyprland_left_in_systemd(self):
        # An earlier Hyprland login left its variables in the user manager (linger, say), and the daemon has them too.
        self.host.command_reply("systemctl", args=["--user", "show-environment"],
                                stdout="HYPRLAND_INSTANCE_SIGNATURE=old_instance\nXDG_RUNTIME_DIR=/run/user/1000\n")
        self.restart_daemon_with(HYPRLAND_INSTANCE_SIGNATURE="old_instance")
        self.answer_sway()
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: self.focus_calls() == [["[con_id=17] focus"]], "Sway, from the agent's own session")
        self.assertEqual(self.host.commands("hyprctl"), [])
        self.assertEqual(self.host.commands("systemctl"), [])  # the agent's session settles it; nothing else is asked

    def test_a_hyprland_left_in_systemd_alone_focuses_nothing(self):
        # The agent's processes say nothing graphical (as over SSH), and the only Hyprland sign is a stale one.
        self.host.use_session({"XDG_RUNTIME_DIR": str(self.host.root)})
        self.host.command_reply("systemctl", args=["--user", "show-environment"],
                                stdout=f"HYPRLAND_INSTANCE_SIGNATURE=old_instance\nXDG_RUNTIME_DIR={self.host.root}\n")
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: NO_WINDOW_SYSTEM in self.log_text(), "no window system logged")
        self.assertEqual(self.host.commands("hyprctl"), [])
        self.assertEqual(self.host.commands("swaymsg"), [])

    def test_without_a_readable_session_systemd_still_finds_sway(self):
        # A daemon systemd started before the desktop imported its variables, and agent processes that don't say.
        self.host.use_session({})
        self.host.command_reply("systemctl", args=["--user", "show-environment"],
                                stdout=f"SWAYSOCK={self.sway_socket}\nXDG_SESSION_TYPE=wayland\n")
        self.answer_sway()
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: self.focus_calls() == [["[con_id=17] focus"]], "Sway, from systemd's user manager")


@unittest.skipUnless(ENABLED, "run tools/test-linux-host.sh on Linux")
class X11FocusTests(HostTestCase):
    """K2 on X11: `wmctrl -l -p` lists the windows with their pids, and `wmctrl -i -a` raises one by its id."""

    def session_environment(self):
        return {"XDG_SESSION_TYPE": "x11", "DISPLAY": ":0"}

    def raise_calls(self):
        return [command["args"] for command in self.host.commands("wmctrl") if command["args"][:1] == ["-i"]]

    def log_text(self):
        return (self.host.root / "daemon.log").read_text()

    def test_k2_raises_the_nearest_ancestor_window_by_id(self):
        self.host.command_reply("wmctrl", args=["-l", "-p"], stdout=WMCTRL_LIST)
        self.hook("UserPromptSubmit")  # host pids 4242, then 4000
        self.host.press("K2")
        wait_for(lambda: self.raise_calls() == [["-i", "-a", "0x01400007"]], "X11 raises the nearest ancestor")
        self.assertEqual(self.host.commands("hyprctl"), [])
        self.assertEqual(self.host.commands("swaymsg"), [])

    def test_a_window_missing_from_the_listing_raises_nothing(self):
        self.host.command_reply("wmctrl", args=["-l", "-p"], stdout="0x01600009 -1 5151 host Volume Control\n")
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: NO_WINDOW in self.log_text(), "missing window logged")
        self.assertEqual(self.raise_calls(), [])
        self.assertEqual(self.host.commands("hyprctl"), [])

    def test_malformed_and_invalid_rows_are_treated_as_no_window(self):
        self.host.command_reply("wmctrl", args=["-l", "-p"],
                                stdout="garbage\n\n0x0140000g 0 4242 host Claude\n0x01400007 0 host Claude\n")
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: NO_WINDOW in self.log_text(), "malformed listing logged")
        self.assertEqual(self.raise_calls(), [])
        self.assertEqual(self.host.commands("hyprctl"), [])

    def test_a_refused_raise_is_logged_and_the_next_press_recovers(self):
        self.host.command_reply("wmctrl", args=["-l", "-p"], stdout=WMCTRL_LIST)
        self.host.command_reply("wmctrl", args=["-i", "-a", "0x01400007"], exit=1,
                                stderr="wmctrl: window 0x01400007 not found")
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: "not found" in self.log_text(), "refused raise logged")
        self.host.command_reply("wmctrl", args=["-i", "-a", "0x01400007"], stdout="")
        self.host.press("K2")
        wait_for(lambda: len(self.raise_calls()) == 2, "next K2 recovers")

    def test_a_failing_listing_is_logged_with_its_stderr(self):
        self.host.command_reply("wmctrl", args=["-l", "-p"], exit=1, stderr="Cannot open display: :0")
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: "Cannot open display: :0" in self.log_text(), "listing stderr logged")
        self.assertEqual(self.raise_calls(), [])

    def test_a_slow_wmctrl_times_out_and_is_logged(self):
        self.host.command_reply("wmctrl", args=["-l", "-p"], stdout=WMCTRL_LIST, sleep=6)
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: "wmctrl timed out" in self.log_text(), "timeout logged", timeout=12)
        self.assertEqual(self.raise_calls(), [])

    def test_gnome_or_kde_on_wayland_focuses_nothing_despite_an_x11_left_in_systemd(self):
        # Xwayland sets DISPLAY, and an earlier X11 login left its variables in the user manager.
        self.host.use_session({"XDG_SESSION_TYPE": "wayland", "WAYLAND_DISPLAY": "wayland-0", "DISPLAY": ":0"})
        self.host.command_reply("systemctl", args=["--user", "show-environment"], stdout="XDG_SESSION_TYPE=x11\nDISPLAY=:0\n")
        self.hook("UserPromptSubmit")
        self.host.press("K2")
        wait_for(lambda: NO_WINDOW_SYSTEM in self.log_text(), "no window system logged")
        self.assertEqual(self.host.commands("wmctrl"), [])
        self.assertEqual(self.host.commands("hyprctl"), [])


@unittest.skipUnless(ENABLED, "run tools/test-linux-host.sh on Linux")
class ForeignFirmwareTests(HostTestCase):
    firmware = False

    def test_foreign_firmware_receives_no_probe_or_heartbeat(self):
        self.host.emit("I (1234) factory: waiting for keyboard\r\n")
        self.host.wait_device(lambda device: device["foreign_firmware"], "foreign firmware detected")
        # Longer than one heartbeat and the listen-first interval: no hello, no JSON key presses.
        time.sleep(6.2)
        self.assertEqual(self.host.lines("host"), "")
        self.host.emit("ESP-ROM:esp32s3-20210327\r\nDISPLAY READY BUILD recovered\nVOLUME 65\n")
        self.host.wait_device(lambda device: not device["foreign_firmware"] and device["firmware_build"] == "recovered",
                              "reset reclassifies firmware")
        self.host.wait_line('"event":"device.heartbeat"', direction="host")


if __name__ == "__main__":
    unittest.main()
