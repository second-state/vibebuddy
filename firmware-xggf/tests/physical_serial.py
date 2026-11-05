"""Exercise the experimental firmware over an explicitly selected serial port."""
import json
import sys
import time
import serial

port = sys.argv[1]
with serial.Serial(port, 115200, timeout=0.2) as link:
    # Opening native USB Serial/JTAG resets the ESP32-C3. Give app_main time to install its
    # USB reader before sending hello; otherwise the command can be lost during startup.
    time.sleep(2.5)
    link.reset_input_buffer()

    def send(event, **extra):
        link.write((json.dumps({"version": 1, "event": event, **extra}) + "\n").encode())
        link.flush()

    def until(expected, timeout=4):
        deadline = time.monotonic() + timeout
        lines = []
        while time.monotonic() < deadline:
            value = link.readline().decode("utf-8", "replace").strip()
            if value:
                lines.append(value)
                if expected in value:
                    return lines
        raise AssertionError(f"missing {expected!r}; got {lines!r}")

    send("device.hello")
    hello = until("DISPLAY READY BUILD")
    assert "BOARD xggf-64x8" in hello, hello
    for event in ("task.start", "agent.input_required", "task.done", "task.error", "agent.idle", "device.identify"):
        send(event)
        until("STATE " + event)
    link.write(b'{"version":2,"event":"task.start"}\nnot-json\n' + b'x' * 5000 + b'\n')
    link.flush()
    send("agent.idle")
    recovered = until("STATE agent.idle")
    assert not any("STATE task.start" in value for value in recovered), recovered
    print("PASS: board identity, hello, states, malformed and oversized input")
