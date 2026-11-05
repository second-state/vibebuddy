#!/usr/bin/env python3
"""Run the installed Kimi CLI against a scripted model, with the hook posting to a loopback receiver.

Usage: kimi_cli_smoke.py <vibebuddy-hook> <kimi>. Port 7331 must be free: the hook has that address built in.
No credentials, no real model, no user configuration: Kimi runs with KIMI_CODE_HOME in a temporary directory.

Each scenario runs in its own project directory, so no other scenario's events can match it. A scenario passes only
when all of these hold:
  - the events for its one session are exactly the expected normalized sequence (a duplicate or a miss fails);
  - for the tool scenarios, the scripted model received a successful tool result naming the project directory,
    so `pwd` really ran;
  - the session the hook reports is the session Kimi reports: the ACP session ID, or the session directory Kimi
    wrote under its home;
  - no posted payload carries a prompt or any field outside the allowlist.
The receiver stands in for vibebuddyd here; kimi_daemon_smoke.py runs the same kind of prompts through the real
daemon and checks its status.
"""
import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time

from kimi_harness import CONFIG, PROMPTS, WAIT_SECONDS, Acp, Model, held, release, start_model

HOOK_PORT = 7331
HOOK = KIMI = None  # the vibebuddy-hook and kimi binaries, set from the arguments
SETTLE_SECONDS = 1.0  # late deliveries get this long to land before a scenario's sequence is judged
ALLOWED_KEYS = {
    "agent", "session_id", "event", "cwd", "surface", "host_bundle_id", "host_ttys",
    "host_focus_url", "tmux_socket", "tmux_pane", "host_pids",
}


class Ledger(http.server.BaseHTTPRequestHandler):
    """The hook's receiver: every POST, with its path and body, in arrival order."""

    posts = []
    condition = threading.Condition()

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        with Ledger.condition:
            Ledger.posts.append((self.path, body))
            Ledger.condition.notify_all()
        self.send_response(202)
        self.end_headers()

    def log_message(self, *_):
        pass


def project_events(project):
    """Kimi's payloads posted for one project directory, in arrival order. Claude Code and Codex post to this port
    while they run; their posts are not this scenario's and are ignored."""
    with Ledger.condition:
        posts = list(Ledger.posts)
    return [body for path, body in posts
            if path == "/v1/agent-hooks" and body.get("agent") == "kimi" and body.get("cwd") == str(project)]


def settle(project, at_least):
    """Wait, bounded, for a scenario's events; then give late deliveries a moment and return everything posted."""
    with Ledger.condition:
        Ledger.condition.wait_for(lambda: len(project_events(project)) >= at_least, WAIT_SECONDS)
    time.sleep(SETTLE_SECONDS)
    return project_events(project)


def check_sequence(name, project, expected, events):
    """The scenario's events must be exactly `expected`, for one session; returns that session's ID."""
    sessions = {event["session_id"] for event in events}
    assert len(sessions) == 1, f"{name}: expected one session, got {sorted(sessions)}"
    names = [event["event"] for event in events]
    assert names == expected, f"{name}: got {names}, expected {expected}"
    for event in events:
        assert event["agent"] == "kimi", event
        assert event["cwd"] == str(project), event
        assert set(event) <= ALLOWED_KEYS, f"{name}: unexpected fields {sorted(set(event) - ALLOWED_KEYS)}"
        for prompt in PROMPTS:
            assert prompt not in json.dumps(event), f"{name}: a prompt was forwarded"
    return sessions.pop()


def check_pwd(name, project):
    """The model must have been sent a successful `pwd` result that names the project directory."""
    result = Model.pwd_result()
    assert result is not None, f"{name}: the model was never sent the tool result"
    assert not result["is_error"], f"{name}: pwd failed: {result['text'][-500:]}"
    assert str(project) in result["text"], f"{name}: pwd did not print the project directory: {result['text'][-500:]}"


def sessions_on_disk(home):
    """The session directories Kimi has written under its home, named as the hook reports them."""
    return {path.name for path in (home / "sessions").glob("wd_*/session_*") if path.is_dir()}


def new_project(root, name):
    project = (root / f"project-{name}").resolve()
    project.mkdir()
    return project


def run_print(home, root, name, prompt, expected, *, env, mode="text", succeeds=True):
    project = new_project(root, name)
    Model.begin(mode)
    before = sessions_on_disk(home)
    result = subprocess.run([KIMI, "-p", prompt], cwd=project, env=env, text=True, capture_output=True, timeout=60)
    assert (result.returncode == 0) == succeeds, (name, result.returncode, result.stderr[-1000:], result.stdout[-1000:])
    events = settle(project, len(expected))
    session = check_sequence(name, project, expected, events)
    created = sessions_on_disk(home) - before
    assert created == {session}, f"{name}: the hook reported {session}, Kimi wrote {sorted(created)}"
    if mode == "tool":
        check_pwd(name, project)
    print(f"{name}: {[event['event'] for event in events]}")


def acp_approval(root, env):
    project = new_project(root, "acp-approval")
    Model.begin("tool")
    client = Acp(KIMI, project, env)
    try:
        client.rpc(1, "initialize", {"protocolVersion": 1, "clientCapabilities": {}})
        created = client.rpc(2, "session/new", {"cwd": str(project), "mcpServers": []})
        client.send({"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {
            "sessionId": created["sessionId"], "prompt": [{"type": "text", "text": "Run pwd and reply OK."}]}})
        outcome = client.receive(3)
    finally:
        client.close()
    events = settle(project, 4)
    session = check_sequence("acp approval", project, ["working", "needs_input", "working", "done"], events)
    assert session == created["sessionId"], f"the hook reported {session}, ACP reported {created['sessionId']}"
    assert outcome["stopReason"] == "end_turn", outcome
    assert len(client.approvals) == 1, client.approvals
    assert client.approvals[0]["toolCall"]["title"] == "Bash", client.approvals[0]["toolCall"]
    check_pwd("acp approval", project)
    print(f"acp approval: {[event['event'] for event in events]}")


def acp_interrupt(root, env):
    project = new_project(root, "acp-interrupt")
    Model.begin("interrupt")
    held.clear()
    release.clear()
    client = Acp(KIMI, project, env)
    try:
        client.rpc(1, "initialize", {"protocolVersion": 1, "clientCapabilities": {}})
        created = client.rpc(2, "session/new", {"cwd": str(project), "mcpServers": []})
        client.send({"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {
            "sessionId": created["sessionId"], "prompt": [{"type": "text", "text": "Wait for cancellation."}]}})
        assert held.wait(timeout=WAIT_SECONDS), "the model request never started"
        client.send({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": created["sessionId"]}})
        outcome = client.receive(3)
        assert outcome["stopReason"] == "cancelled", outcome
    finally:
        release.set()
        client.close()
    events = settle(project, 2)
    session = check_sequence("acp interrupt", project, ["working", "stopped"], events)
    assert session == created["sessionId"], f"the hook reported {session}, ACP reported {created['sessionId']}"
    print(f"acp interrupt: {[event['event'] for event in events]}")


def main():
    with tempfile.TemporaryDirectory(prefix="vibebuddy-kimi-cli-") as temporary:
        root = Path(temporary)
        home = root / "home"
        home.mkdir(mode=0o700)
        env = {**os.environ, "KIMI_CODE_HOME": str(home), "NO_PROXY": "127.0.0.1,localhost"}
        model = start_model()
        try:
            original = CONFIG.format(port=model.server_port)
            (home / "config.toml").write_text(original)
            subprocess.run([HOOK, "install", "kimi"], env=env, capture_output=True, check=True)
            installed = (home / "config.toml").read_text()
            assert installed.count("vibebuddy-hook") == 7, installed
            # The installer only adds hook tables: every setting already there is untouched, byte for byte.
            assert installed.startswith(original), "installing changed settings that were already in the config"

            run_print(home, root, "text", "Reply with OK.", ["working", "done"], env=env)
            run_print(home, root, "tool", "Run pwd and then reply OK.", ["working", "working", "done"], env=env, mode="tool")
            # Print mode runs an "ask" rule without prompting, so no needs_input is expected here; ACP covers approval.
            (home / "config.toml").write_text(installed + '\n[permission]\nrules = [{ decision = "ask", pattern = "Bash" }]\n')
            run_print(home, root, "ask-rule", "Run pwd and then reply OK.", ["working", "working", "done"], env=env, mode="tool")
            (home / "config.toml").write_text(installed)
            run_print(home, root, "failure", "Reply OK.", ["working", "stopped"], env=env, mode="failure", succeeds=False)
            acp_approval(root, env)
            acp_interrupt(root, env)

            subprocess.run([HOOK, "uninstall", "kimi"], env=env, capture_output=True, check=True)
            remaining = (home / "config.toml").read_text()
            assert "vibebuddy-hook" not in remaining and "[providers.smoke]" in remaining, remaining
        finally:
            release.set()
            model.shutdown()
            model.server_close()
    print("Kimi CLI smoke passed: print and ACP scenarios, exact sequences, verified pwd results, approval and cancellation")


if __name__ == "__main__":
    hook_path, kimi_path = map(os.path.abspath, sys.argv[1:3])
    try:
        receiver = http.server.ThreadingHTTPServer(("127.0.0.1", HOOK_PORT), Ledger)
    except OSError as error:
        sys.exit(f"port {HOOK_PORT} must be free for this test: {error}")
    worker = threading.Thread(target=receiver.serve_forever, daemon=True)
    worker.start()
    try:
        HOOK, KIMI = hook_path, kimi_path
        main()
    finally:
        receiver.shutdown()
        receiver.server_close()
        worker.join()
