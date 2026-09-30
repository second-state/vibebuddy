# The buddy: Vibe Buddy's physical pet

## Product definition

The buddy is not a copy of the Codex pet; it is Vibe Buddy's own original pixel character. Codex, other local agents, training jobs, and CI can all drive the same pet. The character's animation, expressions, and voice run on the box; the Mac only sends sparse state events.

## State language

| State | Screen | Voice |
| --- | --- | --- |
| Idle | Breathing, blinking | None |
| Working | Floating, `>` and a looping dot pattern | None |
| Needs input | `!?`, orange accent | "Hey, I need you for a sec." once |
| Done | Happy, bouncing, green | "All done!" once |
| Failed/blocked | X eyes, sinking, red | "Uh-oh, something went wrong." once |
| Disconnected | Eyes closed, screen goes gray, `NO LINK` | None |

Voice is a supplement for attention, not a continuous state channel, so working and idle stay silent. Holding K2 for one second mutes all voice (including the Pomodoro chime), with `MUTE` shown in the top-left corner of the screen; hold again to unmute. Sound comes back after a reboot.

Disconnected is not the same as failed: failed is the outcome of a task, while disconnected means the buddy itself doesn't know the outcome. It has to be able to say "I don't know"; otherwise, after the daemon crashes or USB is unplugged, it would keep asserting a long-stale state in bright colors. Disconnection is not announced by voice, because unplugging the cable is usually the user's own action, and when the link genuinely drops the user is most likely not nearby.

## Idle screen

Idle is the screen that shows up most often, so it can't be just one fixed line. While idle, the buddy does two things:

- **Rotates through today's stats**: how many things got done, how many times you were asked for input, and how long it was busy in total, switching lines every 3 seconds.
- **Does a little move now and then**: roughly every 20 seconds it takes turns dozing (eyes closed with a `Z` floating up), glancing left and right, and stretching.

The little moves are not decoration. A device that is always lit and always on the same frame looks stuck rather than standing by; the buddy has to prove it's awake first for `NO LINK` to mean anything. Conversely, when disconnected it neither rotates stats nor makes little moves: a moving screen looks alive, which is exactly the opposite of what it needs to convey.

## Footer: build identifiers

The footer always shows two lines: the box's own firmware, and the Mac side as reported in the heartbeat. The format is `git describe` plus the build time:

```text
FW     9b642af 2026-09-14 17:41
APP    9b642af 2026-09-14 17:43
```

The second line's label used to be `DAEMON`; once the App bundled the daemon inside it, the two became the same build, and since 2026-09-16 the label has been `APP`. When the link drops, these two lines are not cleared; they go gray along with the rest of the screen: when disconnected, the most valuable piece of information is exactly "which version was I last connected to".

Build identifiers are used instead of semantic version numbers: both sides can say `0.1.0` and still be days apart, with nothing to show for it.

**Display only, don't judge.** Flashing the firmware requires plugging in USB and stopping the daemon, while the daemon restarts on a one-line source change, so the two sides are on different commits most of the time anyway; treat "mismatch" as a warning and within days it will be ignored completely. What actually causes trouble is a protocol capability mismatch — the firmware gains a requirement whose other half the Mac side doesn't yet satisfy — and that is not a question a commit can answer.

The information on its own is enough: see `NO LINK`, glance down, notice the `APP` line is three days old, and the diagnosis is over. The two lines are left-aligned to the same column, because character-by-character comparison relies on alignment, not color. When the Mac side is too old to send this field at all, it shows `?`, which is itself the answer.

## Pomodoro badge

On duty is not the buddy's only mode: K1 switches to Pomodoro (see [`pomodoro.md`](pomodoro.md)), and after being idle on duty long enough it wanders off into Leisure on its own (see [`leisure.md`](leisure.md)). When a Pomodoro is running and the user switches back to On duty, a small badge sits in the top-right corner, `F 18:21` (focus) or `B 04:59` (break), blinking while paused. It takes up a single line of small text and doesn't change any of the buddy's expressions.

## Multiple task cards

- At most 3 active task cards are shown, with the most recently active task on top.
- The bottom-right corner of each card shows how long it has been in its current state, collapsing from seconds to minutes to hours to fit a three-character width. While waiting for input, the number lights up in the same color as the state; otherwise it is grayed out — this column answers exactly "how long has it been waiting for me".
- Card order expresses time; the buddy's global expression expresses the state that most needs the user's attention.
- Global priority is "needs input > working". So when an older task is waiting for input, the buddy still shows needs input even if a newer working task appears above it.
- The first line of a card is the session name; the second line is the state, project name, and duration. When several things run under the same project at once, the project name can't tell them apart; the session name comes from the agent itself: the title the Claude App auto-generates for each session, the name the user gave a thread in Codex, then the branch recorded on the thread, then the branch of the working directory. Only if none of these exist does it fall back to the project name. It still does not read prompts or transcripts: these names are already shown in the sidebars of both apps. The `title` column in Codex's thread table is the raw first message and is not used.
- The device font has only uppercase letters and digits. If a session name has fewer than three alphanumerics left after filtering out characters that can't be drawn (typically a Chinese title), it is treated as unusable and falls back to the next source.

This split suits a small screen better than simply copying the desktop UI: the cards answer "what tasks are there", and the pet answers "what do I most need you to look at right now".
