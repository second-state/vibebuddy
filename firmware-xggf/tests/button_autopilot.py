"""Drive the test-only button state machine over USB without touching the board's buttons."""
import json
import sys
import time

import serial


port = sys.argv[1]
with serial.Serial(port, 115200, timeout=0.1) as link:
    time.sleep(2)
    link.reset_input_buffer()

    def send(event, **fields):
        message = {"version": 1, "event": event, **fields}
        link.write((json.dumps(message) + "\n").encode())
        link.flush()

    def response(expected, timeout=3):
        deadline = time.monotonic() + timeout
        lines = []
        while time.monotonic() < deadline:
            line = link.readline().decode("utf-8", "replace").strip()
            if not line:
                continue
            lines.append(line)
            if line.startswith("BUTTON TEST"):
                assert expected in line, f"expected {expected!r}; got {line!r}"
                print(line, flush=True)
                return line
        raise AssertionError(f"missing response {expected!r}; got {lines!r}")

    def gesture(name, expected):
        send("device.test.buttons.gesture", gesture=name)
        response(expected)

    def reset():
        send("device.test.buttons.reset")
        response("BUTTON TEST RESET HOME/MIC/0/REC0")

    def menu():
        reset()
        gesture("K1 LONG", "ACTION MENU OPEN FROM HOME/MIC/0/REC0 TO MENU/MIC/0/REC0")

    reset()
    gesture("K0 SHORT", "ACTION MIC TOGGLE FROM HOME/MIC/0/REC0 TO HOME/MIC/0/REC0")
    gesture("K0 LONG", "ACTION IDENTIFY FROM HOME/MIC/0/REC0 TO HOME/MIC/0/REC0")
    gesture("K1 SHORT", "ACTION VIEW NEXT FROM HOME/MIC/0/REC0 TO HOME/MIC/0/REC0")
    gesture("K2 SHORT", "ACTION SOURCE OPEN FROM HOME/MIC/0/REC0 TO HOME/MIC/0/REC0")
    gesture("K2 LONG", "ACTION MUTE TOGGLE FROM HOME/MIC/0/REC0 TO HOME/MIC/0/REC0")

    row_actions = [
        ("MIC", "MIC TOGGLE", "MENU/MIC/0/REC0"),
        ("LINK", "LINK OPEN", "LINK/LINK/0/REC0"),
        ("BRIGHTNESS", "BRIGHTNESS NEXT", "MENU/BRIGHTNESS/0/REC0"),
        ("IDENTIFY", "IDENTIFY", "HOME/MIC/0/REC0"),
        ("MUTE", "MUTE TOGGLE", "MENU/MUTE/0/REC0"),
        ("CLOSE", "MENU CLOSE", "HOME/MIC/0/REC0"),
    ]
    for index, (row, action, target) in enumerate(row_actions):
        menu()
        source = "MENU/MIC/0/REC0"
        for next_index in range(index):
            next_row = row_actions[next_index + 1][0]
            gesture("K1 SHORT", f"ACTION MENU NEXT ROW FROM {source} TO MENU/{next_row}/0/REC0")
            source = f"MENU/{next_row}/0/REC0"
        gesture("K0 SHORT", f"ACTION {action} FROM {source} TO {target}")

    menu()
    # Menu row navigation wraps from CLOSE back to MIC.
    for index in range(len(row_actions)):
        source_row = row_actions[index][0]
        target_row = row_actions[(index + 1) % len(row_actions)][0]
        if index:
            gesture("K1 SHORT", f"ACTION MENU NEXT ROW FROM MENU/{source_row}/0/REC0 TO MENU/{target_row}/0/REC0")
        else:
            gesture("K1 SHORT", f"ACTION MENU NEXT ROW FROM MENU/MIC/0/REC0 TO MENU/LINK/0/REC0")
    gesture("K2 SHORT", "ACTION MENU CLOSE FROM MENU/MIC/0/REC0 TO HOME/MIC/0/REC0")

    menu()
    for row in ("MIC", "LINK"):
        gesture("K1 SHORT", f"ACTION MENU NEXT ROW FROM MENU/{row}/0/REC0 TO MENU/{'LINK' if row == 'MIC' else 'BRIGHTNESS'}/0/REC0")
    gesture("K1 SHORT", "ACTION MENU NEXT ROW FROM MENU/BRIGHTNESS/0/REC0 TO MENU/IDENTIFY/0/REC0")
    gesture("K1 SHORT", "ACTION MENU NEXT ROW FROM MENU/IDENTIFY/0/REC0 TO MENU/MUTE/0/REC0")
    gesture("K1 SHORT", "ACTION MENU NEXT ROW FROM MENU/MUTE/0/REC0 TO MENU/CLOSE/0/REC0")
    gesture("K1 SHORT", "ACTION MENU NEXT ROW FROM MENU/CLOSE/0/REC0 TO MENU/MIC/0/REC0")
    gesture("K0 LONG", "ACTION IGNORED LONG PRESS FROM MENU/MIC/0/REC0 TO MENU/MIC/0/REC0")
    gesture("K1 LONG", "ACTION IGNORED LONG PRESS FROM MENU/MIC/0/REC0 TO MENU/MIC/0/REC0")
    gesture("K2 LONG", "ACTION MUTE TOGGLE FROM MENU/MIC/0/REC0 TO MENU/MIC/0/REC0")
    gesture("K1 SHORT", "ACTION MENU NEXT ROW FROM MENU/MIC/0/REC0 TO MENU/LINK/0/REC0")
    gesture("K0 SHORT", "ACTION LINK OPEN FROM MENU/LINK/0/REC0 TO LINK/LINK/0/REC0")
    for page in range(4):
        gesture("K1 SHORT", f"ACTION LINK NEXT PAGE FROM LINK/LINK/{page}/REC0 TO LINK/LINK/{(page + 1) % 4}/REC0")
    gesture("K0 LONG", "ACTION IGNORED LONG PRESS FROM LINK/LINK/0/REC0 TO LINK/LINK/0/REC0")
    gesture("K2 LONG", "ACTION MUTE TOGGLE FROM LINK/LINK/0/REC0 TO LINK/LINK/0/REC0")
    gesture("K0 SHORT", "ACTION LINK BACK FROM LINK/LINK/0/REC0 TO MENU/LINK/0/REC0")
    gesture("K0 SHORT", "ACTION LINK OPEN FROM MENU/LINK/0/REC0 TO LINK/LINK/0/REC0")
    gesture("K2 SHORT", "ACTION LINK BACK FROM LINK/LINK/0/REC0 TO MENU/LINK/0/REC0")

    # K0 stops recording in HOME, MENU and LINK without changing the active UI.
    for mode in ("HOME", "MENU", "LINK"):
        reset()
        if mode != "HOME":
            gesture("K1 LONG", "ACTION MENU OPEN FROM HOME/MIC/0/REC0 TO MENU/MIC/0/REC0")
        if mode == "LINK":
            gesture("K1 SHORT", "ACTION MENU NEXT ROW FROM MENU/MIC/0/REC0 TO MENU/LINK/0/REC0")
            gesture("K0 SHORT", "ACTION LINK OPEN FROM MENU/LINK/0/REC0 TO LINK/LINK/0/REC0")
        send("device.test.buttons.recording", active=True)
        response(f"ACTION RECORDING STARTED FROM {mode}/{'MIC' if mode != 'LINK' else 'LINK'}/0/REC0 TO {mode}/{'MIC' if mode != 'LINK' else 'LINK'}/0/REC1")
        gesture("K0 SHORT", f"ACTION RECORDING STOP FROM {mode}/{'MIC' if mode != 'LINK' else 'LINK'}/0/REC1 TO {mode}/{'MIC' if mode != 'LINK' else 'LINK'}/0/REC1")
        send("device.test.buttons.recording", active=False)
        response(f"ACTION RECORDING FINISHED FROM {mode}/{'MIC' if mode != 'LINK' else 'LINK'}/0/REC1 TO {mode}/{'MIC' if mode != 'LINK' else 'LINK'}/0/REC0")

    reset()
    gesture("K1 LONG", "ACTION MENU OPEN FROM HOME/MIC/0/REC0 TO MENU/MIC/0/REC0")
    send("device.test.buttons.tick", advance_ms=29999)
    response("ACTION NONE FROM MENU/MIC/0/REC0 TO MENU/MIC/0/REC0")
    send("device.test.buttons.tick", advance_ms=30000)
    response("ACTION MENU TIMEOUT FROM MENU/MIC/0/REC0 TO HOME/MIC/0/REC0")

    reset()
    gesture("K1 LONG", "ACTION MENU OPEN FROM HOME/MIC/0/REC0 TO MENU/MIC/0/REC0")
    gesture("K1 SHORT", "ACTION MENU NEXT ROW FROM MENU/MIC/0/REC0 TO MENU/LINK/0/REC0")
    gesture("K0 SHORT", "ACTION LINK OPEN FROM MENU/LINK/0/REC0 TO LINK/LINK/0/REC0")
    send("device.test.buttons.tick", advance_ms=30000)
    response("ACTION MENU TIMEOUT FROM LINK/LINK/0/REC0 TO HOME/MIC/0/REC0")

    reset()
    gesture("K1 LONG", "ACTION MENU OPEN FROM HOME/MIC/0/REC0 TO MENU/MIC/0/REC0")
    gesture("K1 SHORT", "ACTION MENU NEXT ROW FROM MENU/MIC/0/REC0 TO MENU/LINK/0/REC0")
    gesture("K0 SHORT", "ACTION LINK OPEN FROM MENU/LINK/0/REC0 TO LINK/LINK/0/REC0")
    send("device.test.buttons.reset")
    response("BUTTON TEST RESET HOME/MIC/0/REC0")

    reset()
    gesture("K1 LONG", "ACTION MENU OPEN FROM HOME/MIC/0/REC0 TO MENU/MIC/0/REC0")
    send("device.test.buttons.external_reset")
    response("ACTION EXTERNAL RESET FROM MENU/MIC/0/REC0 TO HOME/MIC/0/REC0")
    reset()
    gesture("K1 LONG", "ACTION MENU OPEN FROM HOME/MIC/0/REC0 TO MENU/MIC/0/REC0")
    gesture("K1 SHORT", "ACTION MENU NEXT ROW FROM MENU/MIC/0/REC0 TO MENU/LINK/0/REC0")
    gesture("K0 SHORT", "ACTION LINK OPEN FROM MENU/LINK/0/REC0 TO LINK/LINK/0/REC0")
    send("device.test.buttons.external_reset")
    response("ACTION EXTERNAL RESET FROM LINK/LINK/0/REC0 TO HOME/MIC/0/REC0")
    print("PASS: MCU button autopilot action and state transitions", flush=True)
