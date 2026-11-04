"""Acceptance checks for a TIME_SCALE=60, BUTTON_AUTOPILOT build.

Run under DEVICE.lock with the desktop daemon stopped. Requires no GPIO presses;
physical GPIO, audible loudness and power-off RTC retention remain manual gates.
"""
import json
import re
import sys
import time
import queue
import threading
import serial

with serial.Serial(sys.argv[1], 115200, timeout=.2) as port:
    lines=queue.Queue()
    def read_lines():
        try:
            while port.is_open:
                line=port.readline().decode(errors='replace').strip()
                if line: lines.put(line)
        except (serial.SerialException, TypeError):
            pass
    threading.Thread(target=read_lines,daemon=True).start()
    def read_line():
        try: return lines.get(timeout=.2)
        except queue.Empty: return ''
    def send(event, **fields):
        port.write((json.dumps(dict(version=1, event=event, **fields))+'\n').encode())
        port.flush()

    def snapshot():
        send('device.test.status')
        deadline=time.monotonic()+4
        while time.monotonic()<deadline:
            line=read_line()
            if line.startswith('BUTTON TEST STATUS '):
                values=re.search(r'PAGE (\d+) PHASE (\d+) RUN (\d+) LEFT (\d+) COUNT (\d+) CLOCK (\d+) (\d+):(\d+) VOLUME (\d+) MUTED (\d+)',line)
                assert values, line
                return tuple(map(int,values.groups()))
        raise AssertionError('No test snapshot; use the autopilot acceptance build')

    def key(gesture):
        send('device.test.buttons.gesture',gesture=gesture,effects=True)
        time.sleep(.1)

    def page(target):
        for _ in range(5):
            if snapshot()[0]==target: return
            key('K1 SHORT')
        raise AssertionError('Page cycle failed')

    def menu_row(name):
        key('K1 LONG')
        for _ in range(9):
            # Row transitions are reported by the same state machine as GPIO input.
            send('device.test.buttons.gesture',gesture='K1 SHORT')
            deadline=time.monotonic()+2
            while time.monotonic()<deadline:
                line=read_line()
                if line.startswith('BUTTON TEST ACTION MENU NEXT ROW'):
                    if ' TO MENU/'+name+'/0/' in line:
                        key('K0 SHORT')
                        return
                    break
        raise AssertionError('Missing menu row '+name)

    send('device.hello')
    send('device.heartbeat',hour=12,minute=34,day=20261103)
    time.sleep(.3)
    assert snapshot()[5:8]==(1,12,34)
    # MIC remains reachable from the menu and toggles both directions.
    key('K1 LONG'); key('K0 SHORT')
    deadline=time.monotonic()+3
    while time.monotonic()<deadline:
        if read_line().startswith('MIC mean='): break
    else: raise AssertionError('MIC row did not start the meter')
    key('K0 SHORT')
    deadline=time.monotonic()+3
    while time.monotonic()<deadline:
        if read_line()=='MIC DONE': break
    else: raise AssertionError('MIC row did not stop the meter')
    key('K2 SHORT')
    page(0)
    before=snapshot()
    key('K0 SHORT')
    assert snapshot()[1:3]==before[1:3]
    page(3)
    key('K0 SHORT'); assert snapshot()[2]==1
    key('K0 SHORT'); paused=snapshot(); assert paused[2]==2
    time.sleep(.6); assert snapshot()[3]==paused[3]
    key('K0 SHORT'); assert snapshot()[2]==1
    menu_row('STOP FOCUS'); assert snapshot()[1:3]==(0,0)
    page(3); count=snapshot()[4]
    key('K0 SHORT'); page(1)
    send('agent.input_required'); assert snapshot()[0]==1
    deadline=time.monotonic()+30
    while time.monotonic()<deadline:
        send('device.heartbeat',hour=12,minute=34,day=20261103)
        state=snapshot()
        if state[1:3]==(1,0): break
        time.sleep(.25)
    else: raise AssertionError('Focus did not end with compressed durations')
    assert state[0]==3 and state[4]==count+1, state
    key('K0 SHORT'); assert snapshot()[1:3]==(1,1)
    deadline=time.monotonic()+8
    while time.monotonic()<deadline:
        state=snapshot()
        if state[1:3]==(0,0): break
        time.sleep(.25)
    else: raise AssertionError('Break did not finish')
    # A phase ending over an open menu must close it so K0 starts the break.
    key('K0 SHORT'); key('K1 LONG')
    deadline=time.monotonic()+30
    while time.monotonic()<deadline:
        state=snapshot()
        if state[1:3]==(1,0): break
        time.sleep(.25)
    else: raise AssertionError('Focus did not end over the menu')
    key('K0 SHORT'); assert snapshot()[1:3]==(1,1)
    menu_row('STOP FOCUS'); assert snapshot()[1:3]==(0,0)
    levels=set()
    for _ in range(6):
        menu_row('VOLUME'); levels.add(snapshot()[8]); key('K2 SHORT'); key('K0 LONG')
    assert levels=={20,35,50,65,80,100}
    menu_row('VOLUME'); assert snapshot()[8] in (20,35,50,65,80,100)
    key('K2 SHORT')
    key('K2 LONG'); assert snapshot()[9]==1
    key('K2 LONG'); assert snapshot()[9]==0
    send('device.heartbeat',hour=0,minute=0,day=20261104)
    time.sleep(.1); assert snapshot()[4]==0
    send('device.heartbeat',hour=12,minute=34,day=20261103)
    local=time.localtime()
    send('device.heartbeat',hour=local.tm_hour,minute=local.tm_min,day=local.tm_year*10000+local.tm_mon*100+local.tm_mday)
    send('agent.idle')
    for target in range(4):
        page(target)
        send('device.audio.start',sample_rate=16000,duration_ms=2000)
        time.sleep(.2)
        key('K1 SHORT')
        assert snapshot()[0]==(target+1)%4
        key('K0 SHORT')
        deadline=time.monotonic()+3
        stopped=False
        while time.monotonic()<deadline:
            line=read_line()
            if '"event":"audio.stop.request"' in line:
                stopped=True; break
        assert stopped, 'K0 did not request recording stop on page '+str(target)
        send('device.audio.stop'); time.sleep(.2)
    print('PASS: recording-stop precedence and page cycling during real capture on every page')
    print('PASS: clock sync, page isolation, start/pause/resume/abandon, focus/break completion, volume, mute, daily reset')
