"""Provision and test a paired Wi-Fi link without printing network credentials.

The network and password are read from the active NetworkManager connection.
The temporary computer identity is removed from the device after the test.
"""
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import serial

daemon, port, connection = sys.argv[1:4]
secret = subprocess.check_output(
    ["nmcli", "--show-secrets", "-g", "802-11-wireless-security.psk", "connection", "show", connection],
    text=True, stderr=subprocess.DEVNULL).rstrip("\n")
if not secret:
    raise SystemExit("NetworkManager did not provide a Wi-Fi password")
ssid = subprocess.check_output(
    ["nmcli", "-g", "802-11-wireless.ssid", "connection", "show", connection], text=True).rstrip("\n")
base = "http://127.0.0.1:17331"

def post(path, data):
    request = urllib.request.Request(base + path, json.dumps(data).encode(), {"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=4) as response:
        return response.status

def wait_for(predicate, seconds):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        try:
            with urllib.request.urlopen(base + "/v1/status", timeout=1) as response:
                state = json.load(response)
            if predicate(state["device"]):
                return state["device"]
        except (urllib.error.URLError, TimeoutError):
            pass
        time.sleep(.5)
    raise AssertionError("timed out waiting for the device link")

with tempfile.TemporaryDirectory(prefix="vibebuddy-xggf-wifi-") as directory:
    env = os.environ.copy()
    env.update(XDG_CONFIG_HOME=directory, XDG_STATE_HOME=directory, VIBEBUDDY_BIND="127.0.0.1:17331", VIBEBUDDY_SERIAL_PORT=port)
    process = None
    computer_key = None
    try:
        with open(os.path.join(directory, "daemon.log"), "w+") as log:
            process = subprocess.Popen([daemon], env=env, stdout=log, stderr=log)
            state = wait_for(lambda d: d.get("box_key") and d.get("paired"), 30)
            computer_key = state["computer_key"]
            assert post("/v1/device/wifi/scan", {}) == 202
            wait_for(lambda d: any(network.get("ssid") == ssid for network in d.get("wifi_seen", [])), 20)
            print("PASS: device scan populated the daemon's network picker", flush=True)
            assert post("/v1/device/wifi", {"ssid": ssid, "password": secret}) == 202
            secret = None
            state = wait_for(lambda d: d.get("wifi_address"), 45)
            print("PASS: paired over USB and obtained a Wi-Fi address", flush=True)
            process.terminate(); process.wait(timeout=5); process = None
            # Keep the paired identity, but force the next daemon to discover the address through mDNS.
            os.remove(os.path.join(directory, "vibebuddy", "box-address"))
            # USB retains priority for 15 seconds after its last valid line.
            time.sleep(17)
            env["VIBEBUDDY_SERIAL_PORT"] = "/dev/vibebuddy-no-usb"
            process = subprocess.Popen([daemon], env=env, stdout=log, stderr=log)
            wait_for(lambda d: d.get("connected") and d.get("network") and not d.get("relay"), 45)
            events = (
                "task.start", "agent.input_required", "task.done", "task.error",
                "agent.blocked", "device.identify", "device.mic", "agent.idle",
            )
            for event in events:
                log.flush(); log.seek(0, os.SEEK_END); offset = log.tell()
                assert post("/v1/events", {"version": 1, "event": event}) == 202
                end = time.monotonic() + 8
                while time.monotonic() < end:
                    log.flush(); log.seek(offset)
                    if f"STATE {event}" in log.read():
                        break
                    time.sleep(.25)
                else:
                    raise AssertionError(f"device did not acknowledge {event} over Wi-Fi")
            print(f"PASS: encrypted Wi-Fi link delivered and acknowledged {len(events)} device controls", flush=True)
    finally:
        if process is not None:
            process.terminate()
            try: process.wait(timeout=5)
            except subprocess.TimeoutExpired: process.kill(); process.wait()
        if computer_key:
            with serial.Serial(port, 115200, timeout=.5) as link:
                event = {"version": 1, "event": "device.unpair", "key": computer_key}
                link.write((json.dumps(event) + "\n").encode()); link.flush()
                time.sleep(.5)
