"""Verify tally, volume and RTC readback over a reset, without a daemon.

Requires BUTTON_AUTOPILOT=ON, TIME_SCALE=60, RTC_WRITE=ON. A reset is not
power-off retention: removing external power remains a separate hardware gate.
"""
import json
import re
import subprocess
import sys
import time
import serial

port_name=sys.argv[1]
def send(port,event,**fields):
    port.write((json.dumps(dict(version=1,event=event,**fields))+'\n').encode());port.flush()
def snapshot(port):
    send(port,'device.test.status')
    deadline=time.monotonic()+5
    while time.monotonic()<deadline:
        line=port.readline().decode(errors='replace').strip()
        if line.startswith('RTC ERROR'): raise AssertionError(line)
        if line.startswith('BUTTON TEST STATUS '):
            match=re.search(r'PAGE (\d+) PHASE (\d+) RUN (\d+) LEFT (\d+) COUNT (\d+) CLOCK (\d+) (\d+):(\d+) VOLUME (\d+) MUTED (\d+)',line)
            assert match,line
            return tuple(map(int,match.groups()))
    raise AssertionError('Missing snapshot')
def key(port,gesture):
    send(port,'device.test.buttons.gesture',gesture=gesture,effects=True);time.sleep(.1)

with serial.Serial(port_name,115200,timeout=.2) as port:
    now=time.localtime(); day=now.tm_year*10000+now.tm_mon*100+now.tm_mday
    send(port,'device.heartbeat',day=day,hour=now.tm_hour,minute=now.tm_min)
    synced=False; deadline=time.monotonic()+5
    while time.monotonic()<deadline:
        line=port.readline().decode(errors='replace').strip()
        if line=='RTC SYNC OK': synced=True;break
        if line.startswith('RTC ERROR'): raise AssertionError(line)
    assert synced,'RTC write/readback was not confirmed'
    before=snapshot(port)
    for _ in range(4):
        if snapshot(port)[0]==3: break
        key(port,'K1 SHORT')
    assert snapshot(port)[1:3]==(0,0)
    key(port,'K0 SHORT')
    deadline=time.monotonic()+30
    while time.monotonic()<deadline:
        completed=snapshot(port)
        if completed[1:3]==(1,0): break
        time.sleep(.2)
    else: raise AssertionError('Focus did not complete')
    assert completed[4]==before[4]+1

subprocess.run([sys.executable,'-m','esptool','--chip','esp32c3','--port',port_name,'run'],check=True)
time.sleep(3)
with serial.Serial(port_name,115200,timeout=.2) as port:
    # No hello or heartbeat before inspecting RTC-derived time.
    restored=snapshot(port)
    assert restored[1:3]==(0,0),'Running phase survived reboot'
    assert restored[4]==completed[4],'Completed record was not restored'
    assert restored[8]==completed[8],'Volume was not restored'
    expected=now.tm_hour*60+now.tm_min
    actual=restored[6]*60+restored[7]
    assert restored[5]==1 and (actual-expected)%1440<=1,restored
    print('PASS: RTC write/readback, clock after reset without heartbeat, persistent tally and volume, idle timer after reboot')
