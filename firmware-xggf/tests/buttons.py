"""Independent guided tests for XGGF physical-button mapping and actions."""
import sys
import time
import serial

if len(sys.argv) != 4 or sys.argv[2] not in ("map", "action"):
    raise SystemExit("usage: buttons.py PORT 'map left|middle|right' or 'action K0|K1|K2'")
port, mode, subject = sys.argv[1:]
if mode == "map" and subject not in ("left", "middle", "right"):
    raise SystemExit("map subject must be left, middle or right")
if mode == "action" and subject not in ("K0", "K1", "K2"):
    raise SystemExit("action subject must be K0, K1 or K2")

if mode == "map":
    duration = 120
    instruction = f"Tap {subject.upper()} 3 times, then hold it twice for 1.5 seconds each; touch no other button"
else:
    instructions = {
        "K0": "Tap K0 twice to start/stop mic, then hold K0 once for 1.5 seconds",
        "K1": "Tap K1 3 times to cycle all pages, then hold K1 once for 1.5 seconds",
        "K2": "Tap K2 once, then hold K2 twice for 1.5 seconds each to mute/unmute",
    }
    duration = 120
    instruction = instructions[subject]

with serial.Serial(port, 115200, timeout=0.1) as link:
    link.reset_input_buffer()
    lines = []
    print(instruction, flush=True)
    try:
        deadline = time.monotonic() + duration
        next_heartbeat = time.monotonic()
        while time.monotonic() < deadline:
            now = time.monotonic()
            if now >= next_heartbeat:
                link.write(b'{"version":1,"event":"device.heartbeat"}\n')
                link.flush()
                next_heartbeat = now + 4
            line = link.readline().decode("utf-8", "replace").strip()
            if not line:
                continue
            if line.startswith(("BUTTON ", "MUTE ", "MIC DONE")) or '"event":"button"' in line:
                lines.append(line)
                print(line, flush=True)
                short_now = [item.split()[1] for item in lines if item.startswith("BUTTON K") and item.endswith(" SHORT")]
                long_now = [item.split()[1] for item in lines if item.startswith("BUTTON K") and item.endswith(" LONG")]
                if mode == "map":
                    done = len(short_now) >= 3 and len(long_now) >= 2 and len(set(short_now + long_now)) == 1
                elif subject == "K0":
                    done = short_now.count("K0") >= 2 and long_now.count("K0") >= 1 and "MIC DONE" in lines
                elif subject == "K1":
                    done = short_now.count("K1") >= 3 and long_now.count("K1") >= 1
                else:
                    mute_states = [item for item in lines if item.startswith("MUTE ")]
                    done = (short_now.count("K2") >= 1 and long_now.count("K2") >= 2
                            and any('"event":"button"' in item for item in lines)
                            and "MUTE ON" in mute_states and "MUTE OFF" in mute_states
                            and mute_states[-1] == "MUTE OFF")
                if done:
                    break

        short = [line.split()[1] for line in lines if line.startswith("BUTTON K") and line.endswith(" SHORT")]
        long = [line.split()[1] for line in lines if line.startswith("BUTTON K") and line.endswith(" LONG")]
        if mode == "map":
            assert len(short) >= 3, f"expected 3 short presses, got {len(short)}"
            assert len(long) >= 2, f"expected 2 long presses, got {len(long)}"
            keys = set(short + long)
            assert len(keys) == 1, f"expected only {subject}, got {sorted(keys)}"
            print(f"PASS: {subject}={short[0]}, repeated short/long detection and restored state")
        elif subject == "K0":
            assert short.count("K0") >= 2 and long.count("K0") >= 1, (short, long)
            assert any(line == "MIC DONE" for line in lines), "mic did not stop"
            print("PASS: K0 mic start/stop and identify")
        elif subject == "K1":
            assert short.count("K1") >= 3 and long.count("K1") >= 1, (short, long)
            print("PASS: K1 page cycle and return to Agent status")
        else:
            assert short.count("K2") >= 1 and long.count("K2") >= 2, (short, long)
            assert any('"event":"button"' in line and '"button":"K2"' in line for line in lines), "missing source event"
            mute_states = [line for line in lines if line.startswith("MUTE ")]
            assert "MUTE ON" in mute_states, "missing mute"
            assert "MUTE OFF" in mute_states and mute_states[-1] == "MUTE OFF", "mute was not restored"
            print("PASS: K2 source event and mute/unmute restored off")
    finally:
        if mode == "action" and subject == "K2":
            time.sleep(2.5)
        link.write(b'{"version":1,"event":"agent.idle"}\n')
        link.flush()
