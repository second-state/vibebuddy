#!/usr/bin/env python3
"""Run the Kimi CLI through a real vibebuddyd and check what the daemon's own status reports.

Usage: kimi_daemon_smoke.py <vibebuddy-hook> <vibebuddyd> <kimi>.

The chain under test is Kimi -> hook -> vibebuddyd -> /v1/status: the daemon's activities and done counts, not a
stand-in receiver. Port 7331 (the hook's address) must be free. Every path lives under a temporary directory.

The box search is pinned to a port that does not exist. An unpinned daemon connects to a VibeBuddy box on the cable
and pairs this computer with it, so this test must never run without the pin. The test also refuses to continue
if the daemon reports a connected box anyway.
"""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

from kimi_harness import CONFIG, WAIT_SECONDS, Acp, Model, held, release, start_model

PORT = 7331
SETTLE_SECONDS = 1.0
HOOK = KIMI = DAEMON = None  # binaries, set from the arguments


OTHER_AGENTS = ["codex", "claude", "opencode", "copilot", "pi"]


def status():
    """The daemon's status. The daemon started for this test has seen no hooks from any other agent, so one that
    has reported would have its turns counted as Kimi's: refuse, rather than misattribute the counts."""
    with urllib.request.urlopen(f"http://127.0.0.1:{PORT}/v1/status", timeout=5) as response:
        current = json.load(response)
    busy = [agent for agent in OTHER_AGENTS if current["hooks"][agent] is not None]
    assert not busy, (f"{', '.join(busy)} reported hooks to port {PORT} during this test; its turns would be counted "
                      "as Kimi's. Quit those agents and run the test again.")
    return current


def wait_status(predicate, what):
    """Poll the daemon's status until `predicate` holds; bounded. Refused connections are retried: the daemon may
    still be starting."""
    deadline = time.monotonic() + WAIT_SECONDS
    current = None
    while time.monotonic() < deadline:
        try:
            current = status()
        except OSError:
            time.sleep(0.05)
            continue
        if predicate(current):
            return current
        time.sleep(0.05)
    raise AssertionError(f"timed out waiting for {what}; today={current and current['today']}, "
                         f"activities={current and current['activities']}")


def kimi_activities(current):
    return [activity for activity in current["activities"] if activity["agent"] == "kimi"]


def done_count(current):
    return current["today"]["done"]


def start_daemon(root):
    home = root / "daemon-home"
    home.mkdir()
    env = {
        "PATH": os.environ.get("PATH", ""),
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(root / "config"),
        "XDG_STATE_HOME": str(root / "state"),
        "XDG_DATA_HOME": str(root / "data"),
        "VIBEBUDDY_STATS_FILE": str(root / "stats.json"),
        "VIBEBUDDY_SERIAL_PORT": "/nonexistent/vibebuddy-smoke-no-box",
        # Nothing reaches the Relay from here: a closed local port.
        "VIBEBUDDY_RELAY_URL": "ws://127.0.0.1:9",
        "VIBEBUDDY_SUPERVISOR": "other",
        "RUST_LOG": "warn",
    }
    log = open(root / "daemon.log", "w")
    process = subprocess.Popen([DAEMON], env=env, stdout=log, stderr=subprocess.STDOUT)
    try:
        current = wait_status(lambda _: True, "the daemon to answer on its status endpoint")
        assert current["device"]["connected"] is False, "the daemon found a box despite the pin"
    except BaseException:
        process.terminate()
        process.wait(timeout=10)
        log.close()
        raise
    return process, log


def run_print(name, project, prompt, *, env, mode, succeeds=True):
    project.mkdir()
    Model.begin(mode)
    result = subprocess.run([KIMI, "-p", prompt], cwd=project, env=env, text=True, capture_output=True, timeout=60)
    assert (result.returncode == 0) == succeeds, (name, result.returncode, result.stderr[-1000:])


def main():
    with tempfile.TemporaryDirectory(prefix="vibebuddy-kimi-daemon-") as temporary:
        root = Path(temporary)
        kimi_home = root / "kimi-home"
        kimi_home.mkdir(mode=0o700)
        model = start_model()
        daemon, log = start_daemon(root)
        try:
            (kimi_home / "config.toml").write_text(CONFIG.format(port=model.server_port))
            kimi_env = {**os.environ, "KIMI_CODE_HOME": str(kimi_home), "NO_PROXY": "127.0.0.1,localhost"}
            subprocess.run([HOOK, "install", "kimi"], env=kimi_env, capture_output=True, check=True)

            before = status()
            assert before["hooks"]["kimi"] is None and done_count(before) == 0, before["hooks"]

            # Finished turns: each adds exactly one announcement to the daemon's count.
            run_print("text", root / "text", "Reply with OK.", env=kimi_env, mode="text")
            wait_status(lambda current: done_count(current) == 1, "the text turn to be counted")
            time.sleep(SETTLE_SECONDS)
            current = status()
            assert done_count(current) == 1, f"text turn counted {done_count(current)} times"
            assert current["hooks"]["kimi"] is not None, "the daemon did not record Kimi's hooks"
            assert kimi_activities(current) == [], kimi_activities(current)

            run_print("tool", root / "tool", "Run pwd and then reply OK.", env=kimi_env, mode="tool")
            wait_status(lambda current: done_count(current) == 2, "the tool turn to be counted")
            time.sleep(SETTLE_SECONDS)
            assert done_count(status()) == 2

            # A failed turn is stopped, not announced as done.
            run_print("failure", root / "failure", "Reply OK.", env=kimi_env, mode="failure", succeeds=False)
            time.sleep(SETTLE_SECONDS)
            current = status()
            assert done_count(current) == 2, f"a failed turn was announced as done: {current['today']}"
            assert kimi_activities(current) == [], kimi_activities(current)

            # Approval, held open: the daemon must show the Kimi activity as needing input while the request is
            # pending, and only then is the request answered.
            project = (root / "acp-approval").resolve()
            project.mkdir()
            Model.begin("tool")
            seen_waiting = []

            def approve(params):
                current = wait_status(lambda current: any(a["state"] == "needs_input" for a in kimi_activities(current)),
                                      "the Kimi activity to need input while approval is pending")
                waiting = [a for a in kimi_activities(current) if a["state"] == "needs_input"]
                seen_waiting.append(waiting[0])
                return "approve_once"

            client = Acp(KIMI, project, kimi_env)
            try:
                client.rpc(1, "initialize", {"protocolVersion": 1, "clientCapabilities": {}})
                created = client.rpc(2, "session/new", {"cwd": str(project), "mcpServers": []})
                client.send({"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {
                    "sessionId": created["sessionId"], "prompt": [{"type": "text", "text": "Run pwd and reply OK."}]}})
                outcome = client.receive(3, approve=approve)
                assert outcome["stopReason"] == "end_turn", outcome
            finally:
                client.close()
            assert len(seen_waiting) == 1, seen_waiting
            # The status carries the card's display title, uppercase for the device's font.
            assert seen_waiting[0]["project"] == project.name.upper(), seen_waiting[0]
            wait_status(lambda current: done_count(current) == 3, "the approved turn to be counted")
            time.sleep(SETTLE_SECONDS)
            current = status()
            assert done_count(current) == 3, f"approved turn counted {done_count(current)} times"
            assert kimi_activities(current) == [], kimi_activities(current)

            # Cancellation, held open: the daemon must show the Kimi activity as working while the model is
            # waiting, and a cancelled turn must not be announced.
            project = (root / "acp-interrupt").resolve()
            project.mkdir()
            Model.begin("interrupt")
            held.clear()
            release.clear()
            client = Acp(KIMI, project, kimi_env)
            try:
                client.rpc(1, "initialize", {"protocolVersion": 1, "clientCapabilities": {}})
                created = client.rpc(2, "session/new", {"cwd": str(project), "mcpServers": []})
                client.send({"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {
                    "sessionId": created["sessionId"], "prompt": [{"type": "text", "text": "Wait for cancellation."}]}})
                assert held.wait(timeout=WAIT_SECONDS), "the model request never started"
                wait_status(lambda current: any(a["state"] == "working" for a in kimi_activities(current)),
                            "the Kimi activity to show as working")
                client.send({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": created["sessionId"]}})
                outcome = client.receive(3)
                assert outcome["stopReason"] == "cancelled", outcome
            finally:
                release.set()
                client.close()
            wait_status(lambda current: not kimi_activities(current), "the cancelled activity to be cleared")
            time.sleep(SETTLE_SECONDS)
            assert done_count(status()) == 3, "a cancelled turn was announced as done"

            subprocess.run([HOOK, "uninstall", "kimi"], env=kimi_env, capture_output=True, check=True)
            assert "vibebuddy-hook" not in (kimi_home / "config.toml").read_text()
        finally:
            release.set()
            daemon.terminate()
            try:
                daemon.wait(timeout=10)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.wait()
            log.close()
            model.shutdown()
            model.server_close()
        panics = [line for line in (root / "daemon.log").read_text().splitlines() if "panicked" in line]
        assert not panics, panics
    print("Kimi daemon smoke passed: CLI turns counted once each by vibebuddyd, needs_input and working shown in "
          "/v1/status, failed and cancelled turns not announced as done, approval and cancellation through ACP")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit("usage: kimi_daemon_smoke.py <vibebuddy-hook> <vibebuddyd> <kimi>")
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)  # connections left in TIME_WAIT don't count
        try:
            probe.bind(("127.0.0.1", PORT))
        except OSError as error:
            sys.exit(f"port {PORT} must be free for this test: {error}")
    HOOK, DAEMON, KIMI = (os.path.abspath(path) for path in sys.argv[1:4])
    main()
