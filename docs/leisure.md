# Leisure mode

The buddy has three modes, and each name says what it is doing for you at that moment: **On duty** watches the agents and calls you when something happens; **Pomodoro** times you; **Leisure** is when it's had nothing to do on duty for long enough and goes off to play by itself. K1 toggles between On duty and Pomodoro; Leisure needs no toggle — it comes by itself and leaves by itself.

## Boredom

Leisure is not a switch; it is a tiny emotional model: the longer it's idle, the more bored it gets, and anything happening resets it to zero. Behavior goes by tiers:

| Idle time | Tier | Behavior |
| --- | --- | --- |
| 0 to 5 minutes | Standby | The existing breathing, blinking, and today's stats rotation on the On duty screen |
| 5 to 30 minutes | Bored | Enters Leisure mode; performs a random skit every 20 to 40 seconds, and stands breathing the rest of the time |
| Over 30 minutes | Sleepy | Mostly sleeping, the whole screen dims, and every two to five minutes it talks in its sleep or startles itself awake |
| 23:00 to 07:00 at night, idle for over 90 minutes | Lights out | Backlight off, screen fully black |

**Idle is measured by state, not by messages.** vibebuddyd deduplicates, so a task running for twenty minutes may send the device no messages at all, even though it is clearly working. So idle time only accumulates when all of these hold: the buddy's main state is idle with no task cards, no Pomodoro is running, the link is up, and no button has been pressed recently. A Pomodoro that is ready to start or paused does not count as running. A static `25:00` screen waiting to start is as boring as On duty idle, so after five minutes it switches back to On duty and then goes into Leisure; a paused one is something the user deliberately left there, so it's only treated as the person having left after thirty minutes untouched. The first version waited thirty minutes for both; after actually using it, dragon found that sitting in front of a ready-to-start Pomodoro the sprite never came out to play, so it was changed to the current behavior.

**Come back the moment something happens.** Any agent state message and any button press resets boredom to zero and returns to On duty. A button press wakes and acts, not just wakes: Leisure doesn't hide anything you need to look at before deciding, and making K2 take two presses to open the source would just make the device feel sluggish. A short press of K1 goes from Leisure straight to Pomodoro; K0 starts a focus straight from Leisure.

**No Leisure while disconnected.** A moving screen looks alive, which is the opposite of `NO LINK`.

**Leisure is silent.** Voice on this device is for notifications; a device on your desk humming for no reason is harassment, not cute.

**Leisure doesn't change the at-a-glance state semantics.** The top status bar keeps the On duty idle blue, the footer stays as it is, and a sleeping sprite simply means "nothing is waiting for you".

## Skits

In the Bored tier they're picked at random, never the same one twice in a row. All are drawn with the existing rectangles, lines, and glyphs, at 8 fps.

| Skit | What happens |
| --- | --- |
| Patrol | Walks to the right, stops to glance at you, walks to the left, then walks back. Feet lift alternately |
| Kick the ball | A ball rolls in from the left to its feet, it kicks it away, the ball bounces and rolls back, then another kick sends it off screen. The ball always stays on the left of the body and never passes through it |
| Reading | Holds up a book and scans it line by line, turns a page every five seconds, and gets startled by the plot partway through |
| Counting stars | A few stars twinkle in the night sky; it looks up and counts to seven, slower and slower, and falls asleep mid-count |
| Hide and seek | Slips off the right edge of the screen until only one waving hand is left, leans half its body out to take a look, ducks back, and finally walks back |
| Startled awake | Asleep, suddenly an exclamation mark jumps up, it looks left and right, yawns, and goes back to sleep |
| Sleep talk | Asleep, a string of bubbles rises above its head, and the bubbles rotate through today's stats |

The baseline of the Sleepy tier is sleeping: eyes closed, slow breathing, a `Z` floating up, and every channel of the whole screen halved to dim it.

## Day, night, and stats

The heartbeat carries the Mac side's local hour (the device has no clock, and shouldn't connect to Wi-Fi just for this). At night the Bored tier performs counting stars and sleep talk more often and patrol and kick the ball less; if it doesn't know the time it assumes daytime — better to stay lit than to turn the lights off in the afternoon.

The done count in today's stats shapes its mood: if nothing got done today, kick the ball gets double weight, as it idly kicks pebbles; with five or more done, it's tired and dreams a lot, and in the Sleepy tier it no longer startles itself awake. What the sleep-talk bubbles recite is the stats themselves.

## Long press K1

Hold K1 for one second and the buddy goes into Leisure immediately. This is for demos and acceptance testing: no need to wait five minutes.

## Implementation

- The director is in [`agent_leisure.c`](../firmware/main/agent_leisure.c): boredom, tiers, skit selection, day/night, stats weighting, and the lights-out decision, in pure C without touching hardware. The host test `./tools/test-leisure.sh` covers tier transitions, reset on activity, no back-to-back repeats, Sleepy only sleeping, lights out only at night and lighting up as soon as something happens, and counter wraparound.
- Skit drawing is in `agent_display.c`, where the buddy is parameterized into poses that can specify position, gaze, eyes, mouth, arms, and feet. `./tools/preview-display.sh` renders each skit to a GIF to check before flashing.
- Hardware acceptance uses time-compressed firmware: `idf.py -B firmware/build-fast -DTIME_SCALE=60 build` compresses five minutes into five seconds (the Pomodoro is compressed by the same factor), so Bored, Sleepy, lights out, and being woken can all be walked through in a few minutes; flash it with `BUILD_DIR=firmware/build-fast tools/flash-bridge.sh <serial port> partition app`. Flash the production firmware again after acceptance.
- Diagnostic lines reported by the device: `MODE DUTY` / `POMODORO` / `LEISURE`, `LEISURE ALERT` / `BORED` / `SLEEPY`, `LEISURE SKIT <name>`, `LEISURE LIGHTS OUT` / `ON`, `CLOCK HOUR <n>`.
