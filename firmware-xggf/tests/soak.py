"""Exercise the XGGF display and check runtime diagnostics over an extended soak."""
import json
import sys
import time
import serial

port = sys.argv[1]
minutes = float(sys.argv[2]) if len(sys.argv) > 2 else 15
if minutes <= 0:
    raise SystemExit("soak minutes must be positive")

states = (
    "task.start",
    "agent.input_required",
    "task.done",
    "task.error",
    "agent.blocked",
    "agent.idle",
    "device.identify",
)

def send(link, event, **fields):
    link.write((json.dumps({"version": 1, "event": event, **fields}) + "\n").encode())
    link.flush()

with serial.Serial(port, 115200, timeout=0.2) as link:
    send(link, "device.hello")
    time.sleep(1)
    link.reset_input_buffer()
    started = time.monotonic()
    deadline = started + minutes * 60
    next_heartbeat = started
    next_state = started
    state_index = 0
    next_page = started + 20
    page_index = 0
    samples = []
    while time.monotonic() < deadline:
        now = time.monotonic()
        if now >= next_heartbeat:
            local=time.localtime()
            send(link, "device.heartbeat",hour=local.tm_hour,minute=local.tm_min,day=local.tm_year*10000+local.tm_mon*100+local.tm_mday)
            next_heartbeat = now + 5
        # Requires the autopilot acceptance build to exercise timer pages.
        if now >= next_page:
            send(link,"device.test.buttons.gesture",gesture="K1 SHORT",effects=True)
            page_index=(page_index+1)%4
            if page_index==3: send(link,"device.test.buttons.gesture",gesture="K0 SHORT",effects=True)
            next_page=now+20
        if now >= next_state:
            send(link, states[state_index % len(states)])
            state_index += 1
            next_state = now + 10
        line = link.readline().decode("utf-8", "replace").strip()
        if line.startswith("DISPLAY READY BUILD"):
            raise AssertionError("device restarted during the soak")
        if "Guru Meditation" in line or "Stack protection fault" in line:
            raise AssertionError(line)
        if line.startswith(("PANEL ERROR", "MDNS ERROR", "WIFI UNAVAILABLE", "IDENTITY MIGRATION REQUIRED", "RTC ERROR")):
            raise AssertionError(line)
        if not line.startswith("DIAG "):
            continue
        values = {}
        for item in line[5:].split():
            key, value = item.split("=", 1)
            values[key] = int(value)
        samples.append(values)
        print(line, flush=True)

    send(link, "agent.idle")

if not samples:
    raise AssertionError("no diagnostics arrived during the soak")
heap_change = samples[-1]["heap_free"] - samples[0]["heap_free"]
if heap_change < -16384:
    raise AssertionError(f"free heap fell by {-heap_change} bytes during the soak")
panel_times = [sample["panel_us"] for sample in samples]
stack_minimum = min(sample["stack_free_min"] for sample in samples)
print(
    f"PASS: {minutes:g}-minute soak, {len(samples)} diagnostic samples, "
    f"heap change {heap_change} bytes, minimum stack headroom {stack_minimum} bytes, "
    f"panel transfer {min(panel_times)}-{max(panel_times)} us"
)
