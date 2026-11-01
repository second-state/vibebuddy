"""Run an isolated daemon against one explicitly selected physical device."""
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

daemon, port = sys.argv[1:3]
with tempfile.TemporaryDirectory(prefix="vibebuddy-xggf-smoke-") as directory:
    env = os.environ.copy()
    env.update(XDG_CONFIG_HOME=directory, XDG_STATE_HOME=directory,
               VIBEBUDDY_BIND="127.0.0.1:17331", VIBEBUDDY_SERIAL_PORT=port)
    process = subprocess.Popen([daemon], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        base = "http://127.0.0.1:17331"
        deadline = time.monotonic() + 30
        while True:
            if process.poll() is not None:
                raise AssertionError("daemon exited")
            try:
                with urllib.request.urlopen(base + "/v1/status", timeout=1) as response:
                    status = json.load(response)
                device = status["device"]
                if device.get("firmware_build") and device.get("unsupported_board") == "xggf-64x8":
                    break
            except (urllib.error.URLError, TimeoutError):
                pass
            if time.monotonic() >= deadline:
                raise AssertionError("daemon did not identify the XGGF")
            time.sleep(0.25)
        def post(path, data):
            request = urllib.request.Request(base + path, json.dumps(data).encode(), {"Content-Type": "application/json"})
            try:
                with urllib.request.urlopen(request, timeout=3) as response:
                    return response.status
            except urllib.error.HTTPError as error:
                return error.code
        assert post("/v1/events", {"version": 1, "event": "task.start"}) == 202
        assert post("/v1/device/firmware", {"bootloader": "/missing", "partition_table": "/missing", "app": "/missing"}) == 409
        print("PASS: daemon discovered XGGF, accepted an event and refused box firmware")
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
