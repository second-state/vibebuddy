"""Exercise the token-gated XGGF audio probe and save explicitly requested WAV results."""
import io
import json
import math
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

DAEMON, PORT, CONNECTION, OUTPUT = sys.argv[1:5]
QUICK = "--quick" in sys.argv
WIFI_ONLY = "--wifi-only" in sys.argv
BUTTON_STOP = "--button-stop" in sys.argv
REPEAT = next((int(value.split("=", 1)[1]) for value in sys.argv if value.startswith("--repeat=")), 1)
assert 1 <= REPEAT <= 100
LONG_MS = next((int(value.split("=", 1)[1]) for value in sys.argv if value.startswith("--long-ms=")), 30000)
assert 1000 <= LONG_MS <= 60000
BASE = "http://127.0.0.1:17331"
TOKEN = secrets.token_urlsafe(24)
HEADERS = {"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"}
os.makedirs(OUTPUT, exist_ok=True)


def action_beep():
    path = "/tmp/xggf-action-beep.wav"
    with wave.open(path, "wb") as tone:
        tone.setnchannels(1)
        tone.setsampwidth(2)
        tone.setframerate(16000)
        samples = (int(12000 * math.sin(2 * math.pi * 1000 * index / 16000)) for index in range(8000))
        tone.writeframes(b"".join(struct.pack("<h", sample) for sample in samples))
    try:
        subprocess.run(["aplay", path], check=True)
    except FileNotFoundError:
        print("\a", end="", flush=True)


def call(path, method="GET", data=None, authorized=True):
    headers = HEADERS if authorized else {"Content-Type": "application/json"}
    body = json.dumps(data).encode() if data is not None else None
    request = urllib.request.Request(BASE + path, body, headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=4) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def wait_device(predicate, seconds):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(BASE + "/v1/status", timeout=1) as response:
                device = json.load(response)["device"]
            if predicate(device):
                return device
        except (urllib.error.URLError, TimeoutError):
            pass
        time.sleep(0.25)
    raise AssertionError("timed out waiting for the device")


def start_daemon(env, log):
    process = subprocess.Popen([DAEMON], env=env, stdout=log, stderr=log)
    time.sleep(0.3)
    if process.poll() is not None:
        raise AssertionError("daemon exited")
    return process


def record(label, rate, duration_ms, stimulus):
    code, body = call("/v1/device/audio/start", "POST", {"sample_rate": rate, "duration_ms": duration_ms})
    assert code == 202, (code, body)
    player = subprocess.Popen(["aplay", "-q", stimulus]) if stimulus else None
    wav_data = None
    try:
        deadline = time.monotonic() + duration_ms / 1000 + 15
        while time.monotonic() < deadline:
            code, candidate = call("/v1/device/audio")
            if code == 200:
                wav_data = candidate
                break
            assert code == 409, (code, candidate[:200])
            status_code, status_body = call("/v1/status", authorized=False)
            if status_code == 200:
                device = json.loads(status_body)["device"]
                if not device.get("connected"):
                    raise AssertionError(f"device link lost during {label}")
            time.sleep(0.2)
        if wav_data is None:
            progress = call("/v1/device/audio/stats")
            raise AssertionError(f"recording did not complete: {progress}")
    finally:
        if player:
            try:
                player.wait(timeout=7)
            except subprocess.TimeoutExpired:
                player.terminate(); player.wait()
    code, stats_body = call("/v1/device/audio/stats")
    assert code == 200, (code, stats_body)
    stats = json.loads(stats_body)
    path = os.path.join(OUTPUT, label + ".wav")
    with open(path, "wb") as output:
        output.write(wav_data)
    with wave.open(path, "rb") as recording:
        assert recording.getnchannels() == 1 and recording.getsampwidth() == 2
        assert recording.getframerate() == rate
        count = recording.getnframes()
        samples = struct.unpack(f"<{count}h", recording.readframes(count))
    duration = count / rate
    assert abs(duration - duration_ms / 1000) < 0.1, (duration, duration_ms)
    assert stats["pcm_bytes"] == count * 2
    assert stats["lost_frames"] == stats["duplicate_frames"] == stats["reordered_frames"] == 0, stats
    assert stats["device_overruns"] == 0, stats
    peak = max(abs(value) for value in samples)
    clipped = sum(abs(value) >= 32700 for value in samples)
    result = {
        "label": label,
        "sample_rate": rate,
        "duration_seconds": duration,
        "samples": count,
        "peak": peak,
        "clipped_percent": clipped * 100 / count,
        "stats": stats,
        "wav": path,
    }
    print(json.dumps(result), flush=True)
    log.flush(); log.seek(0)
    diagnostics = [line[line.index("LINK DIAG "):].strip() for line in log if "LINK DIAG " in line]
    if diagnostics:
        print(diagnostics[-1], flush=True)
    return result


def record_button_stop(label, rate):
    code, body = call("/v1/device/audio/start", "POST", {"sample_rate": rate, "duration_ms": 10000})
    assert code == 202, (code, body)
    time.sleep(0.5)
    action_beep()
    print(f"Press K0 once to stop {label}; waiting for the shortened WAV", flush=True)
    wav_data = None
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        code, candidate = call("/v1/device/audio")
        if code == 200:
            wav_data = candidate
            break
        assert code == 409, (code, candidate[:200])
        time.sleep(0.1)
    assert wav_data is not None, "K0 did not finish the recording"
    code, stats_body = call("/v1/device/audio/stats")
    assert code == 200, (code, stats_body)
    stats = json.loads(stats_body)
    with wave.open(io.BytesIO(wav_data), "rb") as recording:
        assert recording.getnchannels() == 1 and recording.getsampwidth() == 2
        assert recording.getframerate() == rate
        duration = recording.getnframes() / rate
    assert 0 < duration < 9.5, (duration, stats)
    assert stats["timing_padding_frames"] == 0, stats
    assert stats["lost_frames"] == stats["duplicate_frames"] == stats["reordered_frames"] == 0, stats
    assert stats["device_overruns"] == 0, stats
    path = os.path.join(OUTPUT, label + ".wav")
    with open(path, "wb") as output:
        output.write(wav_data)
    result = {"label": label, "sample_rate": rate, "duration_seconds": duration, "stats": stats, "wav": path}
    print(json.dumps(result), flush=True)
    return result


secret = subprocess.check_output(
    ["nmcli", "--show-secrets", "-g", "802-11-wireless-security.psk", "connection", "show", CONNECTION],
    text=True, stderr=subprocess.DEVNULL).rstrip("\n")
ssid = subprocess.check_output(
    ["nmcli", "-g", "802-11-wireless.ssid", "connection", "show", CONNECTION], text=True).rstrip("\n")
assert secret
results = []
if BUTTON_STOP:
    print("Starting the USB/Wi-Fi K0-stop probe; later beeps request a K0 press", flush=True)
    action_beep()
with tempfile.TemporaryDirectory(prefix="vibebuddy-xggf-audio-") as directory:
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
    try:
        with open(os.path.join(directory, "daemon.log"), "w+") as log:
            process = start_daemon(env, log)
            device = wait_device(lambda d: d.get("paired") and d.get("unsupported_board") == "xggf-64x8", 60)
            computer_key = device["computer_key"]
            assert call("/v1/device/audio/start", "POST", {"sample_rate": 8000, "duration_ms": 1000}, False)[0] == 401
            code, _ = call("/v1/device/wifi", "POST", {"ssid": ssid, "password": secret})
            assert code == 202
            secret = None
            wait_device(lambda d: d.get("wifi_address"), 45)
            if BUTTON_STOP and not WIFI_ONLY:
                results.append(record_button_stop("usb-button-stop", 8000))
            elif not WIFI_ONLY:
                results.append(record("usb-8k-speech", 8000, 5000, "/tmp/xggf-speech.wav"))
                results.append(record("usb-16k-speech", 16000, 5000, "/tmp/xggf-speech.wav"))
                if not QUICK:
                    results.append(record("usb-8k-30s", 8000, 30000, None))
                code, _ = call("/v1/device/audio/start", "POST", {"sample_rate": 8000, "duration_ms": 5000})
                assert code == 202
                time.sleep(0.5)
                assert call("/v1/device/audio", "DELETE")[0] == 202
                assert call("/v1/device/audio")[0] == 409
            process.terminate(); process.wait(timeout=5); process = None
            os.remove(os.path.join(directory, "vibebuddy", "box-address"))
            time.sleep(17)
            env["VIBEBUDDY_SERIAL_PORT"] = "/dev/vibebuddy-no-usb"
            process = start_daemon(env, log)
            wait_device(lambda d: d.get("connected") and d.get("network") and not d.get("relay") and d.get("unsupported_board") == "xggf-64x8", 45)
            if BUTTON_STOP:
                results.append(record_button_stop("wifi-button-stop", 16000))
            else:
                for index in range(REPEAT):
                    label = "wifi-16k-speech" if REPEAT == 1 else f"wifi-16k-speech-{index + 1:02d}"
                    results.append(record(label, 16000, 5000, "/tmp/xggf-speech.wav"))
                if not QUICK:
                    results.append(record(f"wifi-16k-{LONG_MS // 1000}s", 16000, LONG_MS, None))
    except Exception:
        # The `with` above has already closed `log` by the time this runs, so read the file by path; the
        # temporary directory is still alive here and vanishes right after, taking the transcript with it.
        log_path = os.path.join(directory, "daemon.log")
        if os.path.isfile(log_path):
            with open(log_path) as source:
                transcript = source.read()
            for line in transcript.splitlines():
                if any(text in line for text in ("audio probe rejected", "LINK DIAG ", "ending the session", "reconnecting",
                                                 "encrypted link")):
                    print(line.strip(), flush=True)
            with open(os.path.join(OUTPUT, "daemon.log"), "w") as kept:
                kept.write(transcript)
        raise
    finally:
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill(); process.wait()
        if computer_key:
            with serial.Serial(PORT, 115200, timeout=0.5) as link:
                event = {"version": 1, "event": "device.unpair", "key": computer_key}
                link.write((json.dumps(event) + "\n").encode()); link.flush()
                time.sleep(0.5)

with open(os.path.join(OUTPUT, "results.json"), "w") as output:
    json.dump(results, output, indent=2)
print(f"PASS: {len(results)} recordings fetched and validated; results are in {OUTPUT}", flush=True)
