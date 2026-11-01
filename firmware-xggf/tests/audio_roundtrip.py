"""Loop the XGGF buzzer through the laptop mic/speaker back to the XGGF mic."""
import math
import statistics
import struct
import subprocess
import sys
import tempfile
import time
import wave
import serial

RATE = 16000
FRAME = RATE // 50


def send(link, event):
    link.write((f'{{"version":1,"event":"{event}"}}\n').encode())
    link.flush()


def goertzel(samples, frequency):
    coefficient = 2 * math.cos(2 * math.pi * frequency / RATE)
    first = second = 0.0
    for sample in samples:
        value = sample + coefficient * first - second
        second, first = first, value
    return (first * first + second * second - coefficient * first * second) / (len(samples) * len(samples))


def analyze(path):
    with wave.open(path, "rb") as recording:
        assert recording.getframerate() == RATE and recording.getnchannels() == 1
        count = recording.getnframes()
        samples = struct.unpack(f"<{count}h", recording.readframes(count))
    frames = [samples[index:index + FRAME] for index in range(0, len(samples) - FRAME + 1, FRAME)]
    tone = [goertzel(frame, 1000) for frame in frames]
    strongest = sorted(range(len(frames)), key=tone.__getitem__, reverse=True)[:12]
    powers = {
        frequency: statistics.mean(goertzel(frames[index], frequency) for index in strongest)
        for frequency in (500, 1000, 1500, 2000)
    }
    background = statistics.median(tone)
    peak = max(tone)
    threshold = background + (peak - background) * 0.25
    active = [value >= threshold for value in tone]
    bursts = sum(value and not previous for value, previous in zip(active, [False] + active[:-1]))
    assert peak > max(1, background * 16), f"no distinct buzzer bursts: background {background}, peak {peak}"
    assert powers[1000] > 4 * max(powers[500], powers[1500], powers[2000]), powers
    assert bursts >= 3, f"only {bursts} buzzer bursts detected"
    return bursts, peak / max(background, 1), powers


def replay_to_device(link, path):
    link.reset_input_buffer()
    send(link, "device.mic")
    player = subprocess.Popen(["aplay", "-q", path])
    values = []
    try:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            line = link.readline().decode("utf-8", "replace").strip()
            if "Guru Meditation" in line or line.startswith("MIC ERROR"):
                raise AssertionError(line)
            if line == "MIC DONE":
                break
            if line.startswith("MIC mean="):
                fields = dict(item.split("=", 1) for item in line[4:].split())
                values.append(int(fields["mean"]))
        else:
            raise AssertionError("device microphone probe did not finish")
    finally:
        try:
            player.wait(timeout=6)
        except subprocess.TimeoutExpired:
            player.terminate()
            player.wait()
    assert len(values) >= 30, f"only {len(values)} device microphone reports"
    returning = max(values[:20])
    baseline = max(values[20:])
    assert returning > max(1000, baseline * 2), (returning, baseline)
    return returning, baseline


port = sys.argv[1]
with tempfile.TemporaryDirectory(prefix="vibebuddy-xggf-roundtrip-") as directory:
    recording = directory + "/buzzer.wav"
    with serial.Serial(port, 115200, timeout=0.3) as link:
        link.reset_input_buffer()
        capture = subprocess.Popen([
            "arecord", "-q", "-f", "S16_LE", "-r", str(RATE), "-c", "1", "-d", "4", recording,
        ])
        time.sleep(0.4)
        for _ in range(6):
            send(link, "device.identify")
            time.sleep(0.5)
        assert capture.wait(timeout=7) == 0, "laptop microphone recording failed"
        bursts, ratio, powers = analyze(recording)
        print(
            f"Laptop captured {bursts} buzzer bursts; 1 kHz peak/background {ratio:.1f}; "
            f"band powers {powers}"
        )
        returning, baseline = replay_to_device(link, recording)

print(
    f"PASS: device buzzer -> laptop microphone -> laptop speaker -> device microphone; "
    f"returning/baseline mean amplitudes {returning}/{baseline}"
)
