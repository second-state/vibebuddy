---
status: accepted; the pack format is superseded by ADR-0008
---

# Voice packs live in their own partition and are written over the serial protocol

Users want to pick the announcement voice on the Mac, but the five PCM lines had always been compiled into the firmware's app partition. We decided to add a `voices` data partition to hold the voice pack; the firmware reads it at boot and falls back to the built-in voice if it's empty. To change the voice, the Mac sends it in chunks over the existing serial protocol and the firmware writes the partition itself, with no reset and no esptool.

## Rejected options

Build one firmware per voice, so changing the voice means flashing firmware: this entangles firmware versions with voices, adding a voice means re-releasing every firmware, and changing the voice becomes a reflash. Stream audio from the Mac in real time at playback: this goes against the existing decision that "voice assets live in the firmware, and Pomodoro doesn't depend on the Mac", and 115200 baud over the bridge can't carry it anyway.

## Consequences

The partition table has to change once, and old firmware checkouts need `sdkconfig` deleted and rebuilt. Writing takes three minutes over the bridge, so the pack header is written last; unplugging halfway is the same as never having written it. Firmware upgrades must not erase the `voices` partition.
