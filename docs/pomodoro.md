# Pomodoro mode

Pomodoro is the buddy's second mode. The first is On duty: it watches the agents and answers "how are the agents doing right now"; Pomodoro answers "how much is left of my own focus session". The two share one screen, which is why the concept of a mode is needed; they share one speaker, so voice is not mode-specific. The third mode is Leisure; see [`leisure.md`](leisure.md).

## Durations

Focus is 25 minutes and break is 5 minutes, fixed for now. Both phases are started by the user with a button press and don't chain automatically: when focus ends, it stops at "break ready to start" (`BREAK 05:00`), and the break timer only starts when you press K0; when the break ends, it returns to idle, and the next focus only starts when you press K0. The first version started the break automatically, on the grounds that the point of a break is to be timed; after actually using it, dragon asked for it to be manual — at the moment focus ends you're often still wrapping up, and the break is only accurate if it's counted from when you get up. While the break is ready to start, holding K0 skips the break and goes straight back to idle.

## Screen

It replicates the Focus To-Do dial: a ring of 60 ticks, the elapsed portion colored in the phase color, a longer hand resting at the current position, and the countdown in the center. When idle, the hand rests at 12 o'clock, all ticks are gray, and it shows `25:00`. Focus is tomato red, break is green, and idle uses the buddy's READY blue.

The right-hand column has three groups of information:

- Phase and button hints: `FOCUS` / `BREAK` / `READY`, with the buttons currently available underneath. While paused, the countdown digits blink every half second and `PAUSED` is shown as well — the old stopwatch convention.
- Today's record: under `TODAY`, each completed focus adds one square, and a further line such as `3 FOCUS 1H15` spells out the count and total focus time. Only completed focus sessions count; abandoned ones don't, and paused time isn't deducted. It resets on the local date carried by the Mac-side heartbeat and is stored in NVS, so it survives a reboot; if the Mac side isn't running across midnight, it only resets once the heartbeat comes back with the new date. The today's stats rotation on the On duty idle screen includes this line too.
- Agent summary: the buddy's main state and the title of the top task card. In Pomodoro mode the agents give up the screen but not your attention: on needs input this line still turns yellow and the voice line plays as usual.

When a phase ends, the ring goes off like an alarm clock: for the first two seconds the ring and the countdown digits shake left and right, switching sides every 0.1 seconds with an amplitude of 3 pixels, and all the ticks light up in the next phase's color (green for break when focus ends, READY blue when the break ends); after that the whole ring pulses between the phase color and its half-brightness at one beat per second, like a heartbeat rather than a hard flash between gray and lit. The pulsing continues until the user does something: pressing K0 to start the next phase, holding K0 to abandon or skip, or pressing K1 to switch the screen away (Leisure taking over also counts). The chime and voice line are momentary, and once muted they can never be heard; this visual means that at any time after the timer goes off, a glance shows it "hasn't been dismissed yet". This is not part of the Pomodoro state machine: the display module records the phase that just ended and stops as soon as the view changes.

When switching back to On duty, the countdown doesn't disappear: there is a small badge in the top-right corner, `F 18:21` / `B 04:59`, which also blinks while paused.

When disconnected, only the agent column and the top status bar go gray. The Pomodoro is the device's own fact, and it doesn't become untrustworthy because the Mac side is disconnected; the ring keeps its colors.

## Buttons

The case has three buttons, K0, K1, and K2 (plus RST). Each button does one thing, regardless of the current mode:

| Button | Short press | Long press (1 second) |
| --- | --- | --- |
| K0 | Pomodoro: start / pause / resume | Abandon the current phase and return to idle; when the break is ready to start, this skips the break |
| K1 | Toggle between On duty and Pomodoro | Send the buddy to Leisure right now |
| K2 | Open the current source (reported to the Mac, behavior unchanged) | Mute toggle |

Each button has only one meaning, so there's no need to remember "what to press in which mode". If an agent needs input in the middle of a focus session, one press of K2 takes you there without switching modes first. Starting a focus with K0 switches to Pomodoro automatically, and the ring starting to move is the feedback; pause and resume don't switch modes, and the badge in the top-right of the On duty screen blinks along.

A short press only counts on release, because only on release do you know it wasn't the start of a long press. K2's report to the Mac is therefore delayed from press to release, which has no noticeable effect on "open source".

The wiring of all three buttons has been confirmed on the hardware: K0 is the BOOT button on GPIO0 (silkscreened `B0` on the PCB), K1 is XL9555 P0.4, K2 is XL9555 P0.3, all active-low. K1's bit was pinned down on the first flash on 2026-09-15 by a candidate-bit probe in the firmware: the upstream firmware configures P0.0, P0.1, P0.4, and P1.1–P1.7 as inputs, so the firmware initially treated any of those bits going low as K1; pressing it on the hardware reported `BUTTON RAW P0=0xEF`, after which it was narrowed to a single bit and the probe was removed.

## Mode switching rules

- The user actively toggles between On duty and Pomodoro with K1, and the device remembers the choice.
- Starting a focus with K0 switches to Pomodoro.
- The end of a phase brings Pomodoro to the front. That is exactly the moment the user should take a look: to know it's time to press the button to start the break, or to start the next focus.
- If it sits ready to start for five minutes, or paused for thirty minutes, with no one touching it, it switches back to On duty, and then Leisure takes over (see [`leisure.md`](leisure.md)).
- Agent events don't take over the mode. If an agent needs input during a focus session, the voice line and the yellow line in the right-hand column do the alerting; switching the screen away would decide on the user's behalf that "the agent matters more than your focus", which is not a judgment the device should make.

## Sound

The end of focus and the end of the break each play once: first a chime, then a spoken line: "Nice work. Time for a break!" and "Break's over. Back to it!" in the built-in voice. The chimes are additively synthesized ([`tools/make-pomodoro-audio.py`](../tools/make-pomodoro-audio.py)): end of focus is a descending "ding–dong", end of break is a rising three-note figure. The chime pulls your attention in, and the voice line makes clear which one it is.

Start, pause, resume, and abandon are silent: these are actions the user pressed themselves, and the screen already gives feedback.

Holding K2 for one second is a global mute: neither agent announcements nor the Pomodoro chime sound; hold again to unmute, and `MUTE` stays in the top-left corner of the screen. Mute isn't persisted, and sound comes back after a reboot — muting for a meeting and forgetting to turn it back on, leaving the device silent for days, is worse than one extra sound. Automatic do-not-disturb during focus and automatic night-time mute are not done yet.

## State ownership

The Pomodoro state lives entirely in the firmware; `vibebuddyd` is not involved. It is device I/O and deterministic rendering, which is exactly the firmware's job; and while the user is using it as a timer, the Mac side may not be running at all. The firmware only sends transitions to the Mac as diagnostic lines (`POMODORO FOCUS START`, `FOCUS END`, `BREAK START`, `BREAK END`, `PAUSED`, `RESUMED`, `STOPPED`, `BREAK SKIPPED`, and `POMODORO TODAY <count> <seconds>S DAY <date>` when the record changes), and `vibebuddyd` logs them and does nothing else. Today's record also stays on the device and isn't sent back: the number on the panel should change the moment a focus ends, and a round trip through the Mac side would only add latency and inconsistency.

## Verification

- The state machine has host-side tests: `./tools/test-pomodoro.sh`, covering start, pause, resume, abandon, reporting a phase end only once, and millisecond-counter wraparound.
- The screen has a host-side preview: `./tools/preview-display.sh` renders the same drawing code from the firmware to PNG so the layout can be checked before flashing; the first three seconds of the phase-end alarm are stitched into `pomodoro_alarm.gif`.
- Hardware acceptance checklist: press K1 and see the `25:00` dial, press K1 again to return to On duty; press K0 to start, the screen switches to Pomodoro automatically and the hand moves, press K1 to switch back to On duty and see the badge in the top-right corner; press K0 again to pause, and the digits blink; hold K0 to return to `25:00`; wait for focus to end, hear the chime plus voice line, the screen switches automatically to `BREAK 05:00` and doesn't move, the ring shakes for two seconds and then pulses green all round; press K0 and the break starts moving and the pulsing stops; when the break ends, hear the second chime, return to `READY`, and the ring pulses blue all round until K0 is pressed; hold K2 to mute and repeat the whole thing, confirming the timer going off is noticeable from the screen alone; during a focus, have an agent ask for input, the right-hand column line turns yellow and the voice line plays as usual, and pressing K2 opens the source.
