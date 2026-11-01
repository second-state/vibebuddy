"""Drive the real daemon's XGGF stop-request path through an emulated serial device."""
import base64
import io
import json
import os
import pty
import secrets
import select
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import wave

DAEMON = sys.argv[1] if len(sys.argv) > 1 else "target/debug/vibebuddyd"
BASE = "http://127.0.0.1:17331"
TOKEN = secrets.token_urlsafe(24)
HEADERS = {"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"}


def call(path, method="GET", data=None, authorized=True):
    headers = HEADERS if authorized else {"Content-Type": "application/json"}
    body = json.dumps(data).encode() if data is not None else None
    request = urllib.request.Request(BASE + path, body, headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=3) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def wait_status(predicate, seconds=20):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            code, body = call("/v1/status", authorized=False)
            if code == 200:
                device = json.loads(body)["device"]
                if predicate(device):
                    return device
        except (urllib.error.URLError, TimeoutError):
            pass
        time.sleep(0.1)
    raise AssertionError("timed out waiting for emulated device status")


with tempfile.TemporaryDirectory(prefix="vibebuddy-xggf-stop-emulated-") as directory:
    master, slave = pty.openpty()
    os.set_blocking(master, False)
    port = os.ttyname(slave)
    env = os.environ.copy()
    env.update(
        XDG_CONFIG_HOME=directory,
        XDG_STATE_HOME=directory,
        VIBEBUDDY_BIND="127.0.0.1:17331",
        VIBEBUDDY_SERIAL_PORT=port,
        VIBEBUDDY_AUDIO_PROBE_TOKEN=TOKEN,
    )
    log_path = os.path.join(directory, "daemon.log")
    buffered = b""
    commands = []

    def send(line):
        os.write(master, (line + "\n").encode())

    def read_until(predicate, seconds=10):
        nonlocal_buffer = None
        global buffered
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.1)
            if ready:
                try:
                    buffered += os.read(master, 65536)
                except BlockingIOError:
                    pass
            while b"\n" in buffered:
                raw, buffered = buffered.split(b"\n", 1)
                line = raw.decode("utf-8", "replace").strip()
                if not line:
                    continue
                if line.startswith("{"):
                    try:
                        commands.append(json.loads(line))
                    except json.JSONDecodeError:
                        pass
                if predicate(line):
                    return line
        raise AssertionError("timed out waiting for daemon command")

    with open(log_path, "w") as log:
        process = subprocess.Popen([DAEMON], env=env, stdout=log, stderr=log)
        try:
            send("BOARD xggf-64x8")
            send("DISPLAY READY BUILD xggf-experimental")
            send("FIRMWARE VERSION 0.1.0")
            send("MODE duty")
            send("LINK NOISE")
            send("BOX KEY " + "A" * 43)
            pair = read_until(lambda line: '"event":"device.pair"' in line)
            key = json.loads(pair)["key"]
            send("PAIRED " + key)
            send("PAIRS 1")
            send("PAIR " + key + " emulated")
            wait_status(lambda device: device.get("paired") and device.get("unsupported_board") == "xggf-64x8")

            code, body = call("/v1/device/audio/start", "POST", {"sample_rate": 8000, "duration_ms": 10000})
            assert code == 202, (code, body)
            read_until(lambda line: '"event":"device.audio.start"' in line)
            send("AUDIO BEGIN rate=8000 samples=160")
            send("AUDIO FRAME 0 320 1 " + base64.b64encode(bytes(320)).decode())
            send('{"version":1,"event":"audio.stop.request"}')
            send('{"version":1,"event":"audio.stop.request"}')
            read_until(lambda line: '"event":"device.audio.stop"' in line)
            send("AUDIO END 1 0 320")

            wav_data = None
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline and wav_data is None:
                code, candidate = call("/v1/device/audio")
                if code == 200:
                    wav_data = candidate
                else:
                    assert code == 409, (code, candidate[:200])
                    time.sleep(0.05)
            assert wav_data is not None
            with wave.open(io.BytesIO(wav_data), "rb") as recording:
                assert recording.getframerate() == 8000
                assert recording.getnframes() == 160
            code, stats_body = call("/v1/device/audio/stats")
            assert code == 200
            stats = json.loads(stats_body)
            assert stats["duration_ms"] == 20, stats
            assert stats["timing_padding_frames"] == 0, stats
            assert stats["lost_frames"] == 0, stats
            time.sleep(0.2)
            assert sum(command.get("event") == "device.audio.stop" for command in commands) == 1
        finally:
            process.terminate()
            process.wait(timeout=5)
        os.close(master)
        os.close(slave)

    with open(log_path) as log:
        transcript = log.read()
    assert "AUDIO FRAME" not in transcript, "PCM frames appeared in the daemon log"

print("PASS: emulated XGGF stop request is idempotent and returns a 20 ms WAV without padding or PCM logs")
