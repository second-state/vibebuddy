"""Physical recovery matrix for the XGGF audio probe.

Each scenario breaks one link mid-recording and checks the three Gate 4 expectations: the
recording ends with a terminal reason on the stats endpoint (409), no partial WAV is kept, and
the next recording after the link returns is complete again.

  daemon-restart  scripted; no hands needed
  unplug-usb      cue: unplug the device's USB cable mid-recording, plug it back on cue
  wifi-loss       cue: switch the Wi-Fi off mid-recording (router off or device out of range)
  reboot-device   cue: power-cycle the device mid-recording

Usage: recovery_matrix.py DAEMON PORT SCENARIO [CONNECTION]
CONNECTION (a NetworkManager name, credentials never printed) is only needed for the Wi-Fi
scenarios. Run under DEVICE.lock with an outer timeout, like audio_session.sh does.
"""
import json
import os
import secrets
import struct
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import wave
import serial

DAEMON, PORT, SCENARIO = sys.argv[1:4]
CONNECTION = sys.argv[4] if len(sys.argv) > 4 else None
assert SCENARIO in ("daemon-restart", "unplug-usb", "wifi-loss", "reboot-device"), SCENARIO
WIFI = SCENARIO in ("wifi-loss", "reboot-device")
assert not WIFI or CONNECTION, "the Wi-Fi scenarios need a NetworkManager connection name"
BASE = "http://127.0.0.1:17331"
TOKEN = secrets.token_urlsafe(24)
HEADERS = {"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"}
ARM_MS = 10000


def cue_tone(path, frequency):
    import math
    with wave.open(path, "wb") as tone_file:
        tone_file.setnchannels(1)
        tone_file.setsampwidth(2)
        tone_file.setframerate(8000)
        frames = []
        for _ in range(3):
            frames.extend(int(12000 * math.sin(2 * math.pi * frequency * t / 8000)) for t in range(1200))
            frames.extend(0 for _ in range(400))
        tone_file.writeframes(struct.pack(f"<{len(frames)}h", *frames))


def cue(text, kind=None):
    """Prints the cue and, for the physical ones, plays a sound the operator can react to."""
    print(f"\n*** >>> {text} <<< ***\n", flush=True)
    if kind:
        try:
            path = f"/tmp/xggf-cue-{kind}.wav"
            if not os.path.exists(path):
                cue_tone(path, 880 if kind == "act" else 440)
            subprocess.Popen(["aplay", "-q", path])
        except OSError:
            pass


def call(path, method="GET", data=None):
    body = json.dumps(data).encode() if data is not None else None
    request = urllib.request.Request(BASE + path, body, HEADERS, method=method)
    try:
        with urllib.request.urlopen(request, timeout=4) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def wait_device(predicate, seconds, note):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(BASE + "/v1/status", timeout=1) as response:
                device = json.load(response)["device"]
            last = device
            if predicate(device):
                return device
        except (urllib.error.URLError, TimeoutError):
            pass
        time.sleep(0.25)
    raise AssertionError(f"timed out waiting for the device ({note}): last seen {last}")


def start_daemon(env, log):
    process = subprocess.Popen([DAEMON], env=env, stdout=log, stderr=log)
    time.sleep(0.3)
    if process.poll() is not None:
        raise AssertionError("daemon exited")
    return process


def wait_relink(log_path, after_size, seconds=150):
    """`connected` in /v1/status stays stale while the daemon works its LAN retry, so wait for the
    log line that proves a live re-handshaked session instead."""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        time.sleep(1.0)
        try:
            with open(log_path) as source:
                if source.read().find("linked over the local network", after_size) != -1:
                    return
        except OSError:
            pass
    raise AssertionError("the daemon did not re-link to the device after the network returned")


def wait_terminal(note, seconds=30):
    """The disrupted recording must end in a terminal 409 with a reason, never a WAV."""
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        code, body = call("/v1/device/audio/stats")
        if code == 409:
            code, wav = call("/v1/device/audio")
            assert code == 409 and not wav.startswith(b"RIFF"), (code, wav[:80])
            print(f"terminal state reached: {body.decode()}", flush=True)
            return body.decode()
        time.sleep(0.3)
    raise AssertionError(f"no terminal recording state after the disruption ({note})")


def wait_device_recording_done(armed_at, arm_ms):
    """The device ends an undeliverable recording at most 3 s past its duration."""
    remaining = armed_at + arm_ms / 1000 + 4 - time.monotonic()
    if remaining > 0:
        time.sleep(remaining)


def record_ok(label, rate, duration_ms):
    code, body = call("/v1/device/audio/start", "POST", {"sample_rate": rate, "duration_ms": duration_ms})
    assert code == 202, (code, body)
    started = time.monotonic()
    wav_data = None
    deadline = started + duration_ms / 1000 + 15
    while time.monotonic() < deadline:
        code, candidate = call("/v1/device/audio")
        if code == 200:
            wav_data = candidate
            break
        assert code == 409, (code, candidate[:200])
        time.sleep(0.2)
    assert wav_data is not None, "recovery recording did not complete"
    path = os.path.join(OUTPUT, label + ".wav")
    with open(path, "wb") as output:
        output.write(wav_data)
    with wave.open(path, "rb") as recording:
        assert recording.getnchannels() == 1 and recording.getsampwidth() == 2
        assert recording.getframerate() == rate
        count = recording.getnframes()
        samples = struct.unpack(f"<{count}h", recording.readframes(count))
    assert abs(count / rate - duration_ms / 1000) < 0.1, count
    code, stats = call("/v1/device/audio/stats")
    stats = json.loads(stats)
    assert stats["lost_frames"] == stats["duplicate_frames"] == stats["reordered_frames"] == 0, stats
    assert stats["device_overruns"] == 0, stats
    print(f"recovery recording complete: {label} ({count} samples, peak {max(abs(v) for v in samples)})", flush=True)


OUTPUT = tempfile.mkdtemp(prefix="vibebuddy-xggf-recovery-")
print(f"scenario: {SCENARIO}", flush=True)
with tempfile.TemporaryDirectory(prefix="vibebuddy-xggf-recovery-") as directory:
    env = os.environ.copy()
    env.update(
        XDG_CONFIG_HOME=directory,
        XDG_STATE_HOME=directory,
        VIBEBUDDY_BIND="127.0.0.1:17331",
        VIBEBUDDY_SERIAL_PORT=PORT,
        VIBEBUDDY_AUDIO_PROBE_TOKEN=TOKEN,
    )
    process = None
    computer_key = None
    log = open(os.path.join(directory, "daemon.log"), "w+")
    try:
        # A daemon that opens the port mid-announce hears the build marker but can miss the leading
        # BOARD line, and nothing re-requests it; a fresh connection listens first and says hello,
        # which re-announces from BOARD. Retry the connection until both pairing and board land.
        for attempt in range(3):
            process = start_daemon(env, log)
            try:
                device = wait_device(lambda d: d.get("paired") and d.get("unsupported_board") == "xggf-64x8", 20, "USB pairing")
                break
            except AssertionError:
                if attempt == 2:
                    raise
                print("pairing landed without the board identity; restarting the daemon to re-announce", flush=True)
                process.terminate(); process.wait(timeout=5); process = None
        computer_key = device["computer_key"]
        if WIFI:
            secret = subprocess.check_output(
                ["nmcli", "--show-secrets", "-g", "802-11-wireless-security.psk", "connection", "show", CONNECTION],
                text=True, stderr=subprocess.DEVNULL).rstrip("\n")
            ssid = subprocess.check_output(
                ["nmcli", "-g", "802-11-wireless.ssid", "connection", "show", CONNECTION], text=True).rstrip("\n")
            assert secret
            code, _ = call("/v1/device/wifi", "POST", {"ssid": ssid, "password": secret})
            assert code == 202
            secret = None
            wait_device(lambda d: d.get("wifi_address"), 45, "Wi-Fi provisioning")
            process.terminate(); process.wait(timeout=5); process = None
            os.remove(os.path.join(directory, "vibebuddy", "box-address"))
            time.sleep(17)
            env["VIBEBUDDY_SERIAL_PORT"] = "/dev/vibebuddy-no-usb"
            process = start_daemon(env, log)
            wifi_ready = lambda d: (d.get("connected") and d.get("network") and not d.get("relay")
                                    and d.get("unsupported_board") == "xggf-64x8")
            try:
                wait_device(wifi_ready, 60, "encrypted Wi-Fi session")
            except AssertionError:
                # The announce race again: connected but no BOARD line; a fresh session re-announces.
                print("Wi-Fi session landed without the board identity; restarting the daemon", flush=True)
                process.terminate(); process.wait(timeout=5); process = None
                process = start_daemon(env, log)
                wait_device(wifi_ready, 60, "encrypted Wi-Fi session retry")

        code, _ = call("/v1/device/audio/start", "POST", {"sample_rate": 16000, "duration_ms": ARM_MS})
        assert code == 202, code
        armed_at = time.monotonic()
        print("recording armed; disruption window open", flush=True)

        if SCENARIO == "daemon-restart":
            time.sleep(1.5)
            cue("killing and restarting the daemon (no action needed)", None)
            process.kill(); process.wait(timeout=5); process = None
            time.sleep(1.0)
            process = start_daemon(env, log)
            wait_device(lambda d: d.get("connected") and d.get("unsupported_board") == "xggf-64x8", 30, "daemon restart")
            code, body = call("/v1/device/audio/stats")
            assert code == 409, (code, body)
            print(f"fresh daemon holds no recording state: {body.decode()}", flush=True)
            code, wav = call("/v1/device/audio")
            assert code == 409 and not wav.startswith(b"RIFF"), (code, wav[:80])
        elif SCENARIO == "unplug-usb":
            cue("UNPLUG THE DEVICE'S USB CABLE NOW, AND LEAVE IT OUT", "act")
            wait_device(lambda d: not d.get("connected"), 20, "link loss after unplugging")
            reason = wait_terminal("unplugged USB")
            assert "disconnected" in reason, reason
            cue("PLUG THE DEVICE BACK IN NOW", "restore")
            wait_device(lambda d: d.get("connected") and d.get("paired"), 60, "device returning")
        elif SCENARIO == "wifi-loss":
            cue("TURN THE WI-FI OFF NOW (router off, or carry the device out of range), LEAVE IT OFF", "act")
            wait_terminal("Wi-Fi loss", seconds=45)
            cue("TURN THE WI-FI BACK ON (or bring the device back)", "restore")
            wait_relink(log.name, os.path.getsize(log.name))
            wait_device(wifi_ready, 30, "Wi-Fi returning")
        else:  # reboot-device
            cue("POWER-CYCLE THE DEVICE NOW: UNPLUG ITS USB CABLE FOR A SECOND, THEN PLUG IT BACK", "act")
            wait_terminal("device reboot", seconds=45)
            wait_relink(log.name, os.path.getsize(log.name))
            wait_device(wifi_ready, 30, "device rebooting")

        wait_device_recording_done(armed_at, ARM_MS)
        record_ok(f"recovery-{SCENARIO}", 16000, 5000)
        print(f"PASS: {SCENARIO}", flush=True)
    finally:
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait()
        if computer_key and os.path.exists(PORT):
            try:
                with serial.Serial(PORT, 115200, timeout=0.5) as link:
                    event = {"version": 1, "event": "device.unpair", "key": computer_key}
                    link.write((json.dumps(event) + "\n").encode()); link.flush()
                    time.sleep(0.5)
            except serial.SerialException:
                print("note: could not unpair (serial port unavailable)", flush=True)
        log.flush()
        with open(os.path.join(OUTPUT, "daemon.log"), "w") as kept:
            log.seek(0)
            kept.write(log.read())
        log.close()
