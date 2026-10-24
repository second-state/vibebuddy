"""Linux host integration harness: real daemon + real firmware core, joined through a PTY.

The control socket and wire filters exist only in the test harness. No physical serial ports,
desktop services, user configuration or remote services are needed.
"""

from __future__ import annotations

import json
import fcntl
import os
import pty
import re
import select
import shutil
import socket
import subprocess
import sys
import tempfile
import termios
import threading
import time
import tty
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
# The agent's ancestors in the scenarios and the window listings. Each stands for a real process the harness starts
# with the session's environment, since the daemon reads the window system from `/proc/<pid>/environ`.
PLACEHOLDER_PIDS = (4242, 4000, 4343, 5151)
PLACEHOLDER = re.compile(r"(?<![\w.])(" + "|".join(map(str, PLACEHOLDER_PIDS)) + r")(?![\w.])")
SLEEP = shutil.which("sleep") or "/bin/sleep"


def wait_for(predicate, description, timeout=8):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        last = predicate()
        if last:
            return last
        time.sleep(0.025)
    raise AssertionError(f"Timed out: {description}; last result: {last!r}")


class HostHarness:
    def __init__(self, name):
        artifacts = Path(os.environ.get("VIBEBUDDY_TEST_ARTIFACTS", REPO / "target/host-tests")).resolve()
        artifacts.mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix=name + "-", dir=artifacts))
        # AF_UNIX paths are short even if the checkout/artifact path is very long.
        self.control_dir = tempfile.TemporaryDirectory(prefix="vb-control-")
        self.control_path = Path(self.control_dir.name) / "control.sock"
        self.daemon = None
        self.simulator = None
        self.master = self.slave = None
        self.pump = None
        self.stop_pump = threading.Event()
        self.wire_lock = threading.Lock()
        self.record_lock = threading.Lock()
        self.filter_lock = threading.Lock()
        self.drop_prefixes = ()
        self.replacements = {}
        self.fragment = 0
        self.records = []
        self.pump_errors = []
        self.files = []
        self.sessions = []
        self.pids = {}
        self.listeners = []
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self._prepare_environment()

    def _prepare_environment(self):
        self.bin = self.root / "bin"
        self.bin.mkdir()
        stub = self.bin / "desktop-command"
        stub.write_text(f"#!{sys.executable}\n" + '''import json, os, sys, time
from pathlib import Path
name = Path(sys.argv[0]).name
root = Path(os.environ["VIBEBUDDY_TEST_ROOT"])
argv = sys.argv[1:]
with (root / "commands.jsonl").open("a") as log:
    log.write(json.dumps({"program": name, "args": argv}) + "\\n")
reply = root / (name + ".json")
data = json.loads(reply.read_text()) if reply.exists() else {}
# A reply for leading arguments takes over from the default for the calls that start with them.
for match in data.get("matches", []):
    if argv[:len(match["args"])] == match["args"]:
        data = {**data, **{key: value for key, value in match.items() if key != "args"}}
        break
time.sleep(data.get("sleep", 0))
sys.stderr.write(data.get("stderr", ""))
if name == "hyprctl" and argv[:1] == ["clients"]:
    print(data.get("stdout", json.dumps(data.get("clients", []))))
elif name == "hyprctl" and argv[:1] == ["dispatch"] and argv[1].startswith("hl."):
    print(data.get("lua_stdout", data.get("stdout", "ok")))
    sys.exit(data.get("lua_exit", data.get("exit", 0)))
else:
    print(data.get("stdout", "ok"))
sys.exit(data.get("exit", 0))
''')
        stub.chmod(0o755)
        for name in ("hyprctl", "swaymsg", "wmctrl", "xdg-open", "notify-send", "systemctl", "gh"):
            (self.bin / name).symlink_to(stub.name)
        # Build a fresh environment instead of inheriting agent hooks, desktop buses or production
        # VIBEBUDDY_* overrides. Omit HOME so title lookup cannot read real agent sessions;
        # daemon-owned files use explicit XDG paths.
        self.env = {key: value for key, value in os.environ.items()
                    if key in ("LANG", "LC_ALL", "LD_LIBRARY_PATH", "SYSTEMROOT")}
        self.env.update({
            "PATH": str(self.bin),
            "LANG": "C.UTF-8",
            "LC_ALL": "C.UTF-8",
            "XDG_CONFIG_HOME": str(self.root / "config"),
            "XDG_STATE_HOME": str(self.root / "state"),
            "XDG_CACHE_HOME": str(self.root / "cache"),
            "VIBEBUDDY_CONFIG_FILE": str(self.root / "config.json"),
            "VIBEBUDDY_TEST_ROOT": str(self.root),
            "VIBEBUDDY_CI_REPOS": "",
            "VIBEBUDDY_GH": str(self.bin / "gh"),
            "VIBEBUDDY_LAN_ADDR": "127.0.0.1:1",
            "VIBEBUDDY_RELAY_URL": "ws://127.0.0.1:1",
            "VIBEBUDDY_MANIFEST_URL": "http://127.0.0.1:1/manifest.json",
            "RUST_LOG": "vibebuddyd=debug",
        })
        (self.root / "config.json").write_text(json.dumps({
            "notify_link": False, "check_updates": False, "relay": False,
        }))

    def _log_file(self, name):
        stream = (self.root / name).open("ab", buffering=0)
        self.files.append(stream)
        return stream

    def start(self, firmware=True, environment=None):
        self.firmware = firmware
        # Session variables for the daemon, such as the desktop's compositor; the rest of the environment stays fresh.
        self.env.update(environment or {})
        if firmware:
            executable = Path(os.environ.get("VIBEBUDDY_TEST_SIMULATOR", REPO / "target/debug/examples/simulator"))
            self.simulator = subprocess.Popen(
                [str(executable), "--control", str(self.control_path), "--build", "simulator"], cwd=self.root,
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self._log_file("simulator.log"),
                env=self.env, bufsize=0,
            )
            wait_for(lambda: self.control_path.exists(), "simulator control socket")
        self.connect()
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            self.port = reservation.getsockname()[1]
        self.base = f"http://127.0.0.1:{self.port}"
        self.env["VIBEBUDDY_BIND"] = f"127.0.0.1:{self.port}"
        self.start_daemon()
        self.wait_device(lambda device: device["connected"], "PTY connected")
        if firmware:
            self.control("reboot")
            self.wait_device(lambda device: device["paired"] and device["volume"] is not None
                             and any(pair["key"] == device["computer_key"] for pair in device["paired_computers"]),
                             "firmware boot and automatic pairing")
        return self

    def start_daemon(self):
        executable = Path(os.environ.get("VIBEBUDDY_TEST_DAEMON", REPO / "target/debug/vibebuddyd"))
        self.daemon = subprocess.Popen([str(executable)], cwd=self.root, env=self.env,
                                       stdout=self._log_file("daemon.log"), stderr=subprocess.STDOUT)

    def stop_daemon(self):
        self._stop(self.daemon)
        self.daemon = None
        if self.slave is not None:
            # We retain the slave to keep the PTY alive between daemon processes. Unlike a real
            # unplug, this prevents last-close from clearing tokio-serial's TIOCEXCL automatically.
            fcntl.ioctl(self.slave, termios.TIOCNXCL)

    def connect(self):
        self.master, self.slave = pty.openpty()
        tty.setraw(self.slave)
        link = self.root / "device"
        link.unlink(missing_ok=True)
        link.symlink_to(os.ttyname(self.slave))
        self.env["VIBEBUDDY_SERIAL_PORT"] = str(link)
        self.stop_pump = threading.Event()
        self.pump = threading.Thread(target=self._pump, daemon=True)
        self.pump.start()

    def disconnect(self):
        self.stop_pump.set()
        if self.pump:
            self.pump.join(timeout=2)
        for name in ("master", "slave"):
            descriptor = getattr(self, name)
            if descriptor is not None:
                os.close(descriptor)
                setattr(self, name, None)
        (self.root / "device").unlink(missing_ok=True)

    def record(self, direction, data):
        item = {"direction": direction, "data": data.decode("utf-8", errors="backslashreplace")}
        with self.record_lock:
            self.records.append(item)
            with (self.root / "wire.jsonl").open("a") as log:
                log.write(json.dumps(item) + "\n")

    def _pump(self):
        buffer = b""
        try:
            while not self.stop_pump.is_set():
                readers = [self.master]
                if self.simulator:
                    readers.append(self.simulator.stdout.fileno())
                ready, _, _ = select.select(readers, [], [], 0.05)
                for descriptor in ready:
                    chunk = os.read(descriptor, 8192)
                    if not chunk:
                        raise RuntimeError("simulator or PTY closed unexpectedly")
                    if descriptor == self.master:
                        self.record("host", chunk)
                        if self.simulator:
                            piece = memoryview(chunk)
                            while piece:
                                written = self.simulator.stdin.write(piece)
                                if not written:
                                    raise RuntimeError("simulator stopped reading")
                                piece = piece[written:]
                    else:
                        buffer += chunk
                        while b"\n" in buffer:
                            line, buffer = buffer.split(b"\n", 1)
                            self.record("device", line + b"\n")
                            with self.filter_lock:
                                dropped = any(line.startswith(prefix) for prefix in self.drop_prefixes)
                                replacement = next((value for prefix, value in self.replacements.items()
                                                    if line.startswith(prefix)), line + b"\n")
                            if not dropped:
                                self.emit(replacement)
        except Exception as error:
            if not self.stop_pump.is_set():
                self.pump_errors.append(repr(error))

    def filter(self, drop=(), replace=None, fragment=0):
        with self.filter_lock:
            self.drop_prefixes = tuple(prefix.encode() for prefix in drop)
            self.replacements = {key.encode(): value.encode() for key, value in (replace or {}).items()}
            self.fragment = fragment

    def emit(self, data):
        if isinstance(data, str):
            data = data.encode()
        with self.wire_lock:
            self.record("delivered", data)
            size = self.fragment or max(len(data), 1)
            for at in range(0, len(data), size):
                piece = memoryview(data)[at:at + size]
                while piece:
                    written = os.write(self.master, piece)
                    piece = piece[written:]

    def control(self, action, **fields):
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(5)
            client.connect(str(self.control_path))
            client.sendall(json.dumps({"action": action, **fields}).encode() + b"\n")
            with client.makefile("rb") as reply:
                result = json.loads(reply.readline())
        if not result["ok"]:
            raise AssertionError(result)
        return result

    def press(self, key, long=False):
        self.control("advance", ms=50)
        self.control("buttons", **{key.lower(): True})
        self.control("advance", ms=1100 if long else 80)
        self.control("buttons")
        self.control("advance", ms=50)

    def request(self, path, body=None, method=None, timeout=12):
        headers = {}
        if body is not None:
            if isinstance(body, bytes):
                headers["Content-Type"] = "application/octet-stream"
            else:
                body = json.dumps(body).encode()
                headers["Content-Type"] = "application/json"
        request = urllib.request.Request(self.base + path, data=body, headers=headers,
                                         method=method or ("POST" if body is not None else "GET"))
        try:
            response = self.opener.open(request, timeout=timeout)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            raw = response.read()
            if "application/json" in response.headers.get("Content-Type", ""):
                return response.status, json.loads(raw)
            return response.status, raw

    def status(self):
        if self.daemon and self.daemon.poll() is not None:
            raise AssertionError(f"daemon exited: {self.daemon.returncode}; logs: {self.root}")
        if self.pump_errors:
            raise AssertionError(f"wire pump failed: {self.pump_errors}")
        try:
            return self.request("/v1/status", timeout=0.5)[1]
        except (OSError, urllib.error.URLError):
            return None

    def wait_device(self, predicate, description, timeout=8):
        return wait_for(lambda: (snapshot if (snapshot := self.status()) and predicate(snapshot["device"]) else None),
                        description, timeout)

    def wait_operation(self, state, timeout=8):
        return wait_for(lambda: (snapshot if (snapshot := self.status()) and snapshot["operation"]
                                and snapshot["operation"]["state"] == state else None),
                        f"operation {state}", timeout)

    def lines(self, direction="device"):
        with self.record_lock:
            return "".join(item["data"] for item in self.records if item["direction"] == direction)

    def wait_line(self, text, direction="device", timeout=8):
        return wait_for(lambda: text in self.lines(direction), f"{direction} line {text!r}", timeout)

    def commands(self, program=None):
        log = self.root / "commands.jsonl"
        if not log.exists():
            return []
        # A process may be halfway through appending the final line.
        commands = [json.loads(line) for line in log.read_text().splitlines(keepends=True) if line.endswith("\n")]
        return [command for command in commands if program is None or command["program"] == program]

    def use_session(self, environment):
        """Starts a process for each placeholder pid with exactly `environment`, as the agent's terminal would have it,
        and maps the placeholders to them from now on; the processes started before are stopped."""
        for process in self.sessions:
            self._stop(process)
        self.sessions = [subprocess.Popen([SLEEP, "600"], env=environment, stdin=subprocess.DEVNULL,
                                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                         for _ in PLACEHOLDER_PIDS]
        self.pids = {placeholder: process.pid for placeholder, process in zip(PLACEHOLDER_PIDS, self.sessions)}

    def real_pids(self, text):
        """`text` with each placeholder pid replaced by the process that stands for it."""
        return PLACEHOLDER.sub(lambda match: str(self.pids.get(int(match[1]), match[1])), text)

    def listening_socket(self, name):
        """A Unix socket that accepts connections, as a running compositor's does, under a short path."""
        path = Path(self.control_dir.name) / name
        path.parent.mkdir(parents=True, exist_ok=True)
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(path))
        listener.listen(16)
        self.listeners.append(listener)
        return path

    def command_reply(self, name, args=None, **reply):
        """Without `args`, the reply answers every call. With `args`, it answers calls whose leading arguments match, and
        replaces an earlier reply for the same arguments; the default reply is kept. Placeholder pids in the reply
        become the processes that stand for them."""
        reply = json.loads(self.real_pids(json.dumps(reply)))
        path = self.root / f"{name}.json"
        data = json.loads(path.read_text()) if path.exists() else {}
        if args is None:
            data = {"matches": data.get("matches", []), **reply}
        else:
            matches = [match for match in data.get("matches", []) if match["args"] != list(args)]
            data["matches"] = matches + [{"args": list(args), **reply}]
        path.write_text(json.dumps(data))

    @staticmethod
    def _stop(process):
        if process is None:
            return
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=3)

    def close(self):
        self.stop_daemon()
        self.disconnect()
        self._stop(self.simulator)
        if self.simulator:
            self.simulator.stdin.close()
            self.simulator.stdout.close()
        for process in self.sessions:
            self._stop(process)
        for listener in self.listeners:
            listener.close()
        for stream in self.files:
            stream.close()
        self.control_dir.cleanup()
