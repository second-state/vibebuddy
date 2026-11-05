#!/usr/bin/env python3
"""Run the Kimi hook binary the way Kimi runs it, against a loopback receiver. No Kimi CLI and no daemon needed.

Usage: kimi_smoke.py <vibebuddy-hook>. Port 7331 must be free: the hook has that address built in. No user
configuration is read or changed.

Every event is checked against what it must produce: exactly one POST (or none, for events that are ignored),
to the agent endpoint, carrying exactly the allowed fields with the session and directory forwarded unchanged.
Each payload also carries a marker in every sensitive field; the marker must never reach the receiver.
"""
import http.server
import json
import socket
import subprocess
import sys
import threading
import time

HOOK_PORT = 7331
SECRET = "SECRET-NEVER-FORWARDED"
# Identity and the event, the directory, and the host the agent runs in (hook/src/surface.rs).
ALLOWED_KEYS = {
    "agent", "session_id", "event", "cwd", "surface", "host_bundle_id", "host_ttys",
    "host_focus_url", "tmux_socket", "tmux_pane", "host_pids",
}
HOST_KEYS = ALLOWED_KEYS - {"agent", "session_id", "event", "cwd"}
# Kimi's events, and the normalized event each one must become.
MAPPED = [
    ("UserPromptSubmit", "working"), ("PostToolUse", "working"), ("PermissionRequest", "needs_input"),
    ("Stop", "done"), ("StopFailure", "stopped"), ("Interrupt", "stopped"), ("SessionEnd", "session_end"),
]
IGNORED = ["Notification", "SessionStart", "PreCompact", "SubagentStop", "SubagentStart", "NoSuchEvent"]

binary = sys.argv[1]
posts = []  # (path, body) for every POST the receiver got, in arrival order
lock = threading.Lock()


class Receiver(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        with lock:
            posts.append((self.path, body))
        self.send_response(202)
        self.end_headers()

    def log_message(self, *_):
        pass


def kimi_posts():
    """Kimi's POSTs to the agent endpoint. Claude Code and Codex post to this same port while they run, so their
    posts are not counted."""
    with lock:
        return [(path, body) for path, body in posts if path == "/v1/agent-hooks" and body.get("agent") == "kimi"]


def posted():
    return len(kimi_posts())


def run_hook(payload):
    text = payload if isinstance(payload, str) else json.dumps(payload)
    started = time.monotonic()
    result = subprocess.run([binary, "kimi"], input=text, text=True, capture_output=True, timeout=5)
    elapsed = time.monotonic() - started
    assert result.returncode == 0 and not result.stdout and not result.stderr, (result.returncode, result.stdout, result.stderr)
    return elapsed


def kimi_payload(event, session="kimi-smoke"):
    # What Kimi sends for a hook: the event name, the session and directory, and content that must not leave.
    return {
        "hook_event_name": event, "session_id": session, "cwd": "/work/app",
        "prompt": SECRET, "tool_name": "Bash", "tool_input": {"command": SECRET},
        "tool_output": SECRET, "tool_response": SECRET, "error": SECRET, "error_message": SECRET,
        "session_title": SECRET, "transcript_path": SECRET, "last_assistant_message": SECRET,
        "messages": [SECRET],
    }


server = None
try:
    server = http.server.HTTPServer(("127.0.0.1", HOOK_PORT), Receiver)
except OSError as error:
    sys.exit(f"port {HOOK_PORT} must be free for this test: {error}")
worker = threading.Thread(target=server.serve_forever, daemon=True)
worker.start()
try:
    for event, expected in MAPPED:
        before = posted()
        run_hook(kimi_payload(event))
        assert posted() == before + 1, f"{event}: {posted() - before} POSTs, expected exactly one"
        _, body = kimi_posts()[-1]
        assert set(body) <= ALLOWED_KEYS, f"{event}: unexpected fields {sorted(set(body) - ALLOWED_KEYS)}"
        assert {key: body[key] for key in body if key not in HOST_KEYS} == {
            "agent": "kimi", "session_id": "kimi-smoke", "event": expected, "cwd": "/work/app",
        }, (event, body)
        assert SECRET not in json.dumps(body), f"{event}: sensitive content was forwarded"

    for event in IGNORED:
        before = posted()
        run_hook(kimi_payload(event))
        assert posted() == before, f"{event} should be ignored but was posted"

    for label, payload in [
        ("no session", {"hook_event_name": "Stop", "cwd": "/work/app"}),
        ("empty session", kimi_payload("Stop", session="")),
        ("malformed JSON", "{not json"),
        ("not an object", "[]"),
    ]:
        before = posted()
        run_hook(payload)
        assert posted() == before, f"{label} should post nothing"
finally:
    server.shutdown()
    server.server_close()
    worker.join()

# The daemon is down: the hook still exits quietly and quickly, so Kimi never waits on VibeBuddy.
elapsed = run_hook(kimi_payload("Stop"))
assert elapsed < 2, f"hook took {elapsed:.2f}s with no daemon"

# The daemon accepts the connection and never answers: the hook still gives up within its own timeout.
hung = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
hung.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
hung.bind(("127.0.0.1", HOOK_PORT))
hung.listen()
held = []
accepting = threading.Thread(target=lambda: held.append(hung.accept()), daemon=True)
accepting.start()
try:
    elapsed = run_hook(kimi_payload("Stop"))
    assert elapsed < 2, f"hook waited {elapsed:.2f}s on a daemon that never answers"
finally:
    for connection, _ in held:
        connection.close()
    hung.close()

print(
    "Kimi hook smoke passed: seven mapped events, one POST each, exact allowed fields, "
    "sensitive content withheld, ignored and malformed input posts nothing, "
    "no daemon and a silent daemon both return within two seconds"
)
