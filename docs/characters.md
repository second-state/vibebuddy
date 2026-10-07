# Characters

The buddy is a role: the one presence on the box that watches the agents and speaks up for you. A **Character** is who it is right now: a look, a voice, a persona and a set of lines, always shipped together in one Character pack. The original pixel robot is the default Character, Vibe Buddy. Vocabulary is in [`CONTEXT.md`](../CONTEXT.md); the decisions behind this design are ADR-0007, ADR-0008 and ADR-0009.

The work comes in two phases:

1. **Lines.** Each Character gets a persona and a pool of lines for each occasion, written in advance. The look stays the robot. This is what makes the buddy feel like someone rather than a chime.
2. **Looks.** Sprite-sheet looks for other Characters, and importing a look the user made themselves.

## Phase 1: lines

### Occasions

An announcement speaks one line, drawn from the pool for its occasion. There are five ordinary occasions and seven special ones:

| Occasion id | When | Falls back to |
| --- | --- | --- |
| `input_required` | An Activity needs input | (built-in line) |
| `done` | An Activity is done | (built-in line) |
| `failed` | An Activity failed or is blocked | (built-in line) |
| `focus_done` | Pomodoro focus ended (after the chime) | (built-in line) |
| `break_done` | Pomodoro break ended (after the chime) | (built-in line) |
| `first_done` | The first done of the local day | `done` |
| `milestone` | The 5th, 10th or 20th done of the day | `done` |
| `late_night_done` | The first done between 23:00 and 05:00 | `done` |
| `late_night_input` | The first needs input between 23:00 and 05:00 | `input_required` |
| `greeting_morning` | First link of the day, 05:00 to 12:00 | silence |
| `greeting_afternoon` | First link of the day, 12:00 to 18:00 | silence |
| `greeting_evening` | First link of the day, 18:00 to 05:00 | silence |
| `long_session` | The first done after today's busy time passes another whole hour (up to three) | `done` |
| `welcome_back` | The first activity after three quiet hours, on a day already greeted | silence |

Rules:

- **One edge, one line.** A special occasion replaces the ordinary line rather than adding a second one.
- **The rarest wins.** When several special occasions fit the same done, late night beats first done, which beats a long session, which beats a milestone.
- **Late night is once a night**, shared between done and needs input. A night runs from 23:00 to 05:00 the next morning.
- **The greeting follows the link and the work, not the power.** The box restarts every time the daemon opens the serial port, so "booted" happens many times a day; and a box left plugged in overnight never sees the link come up in the morning. The greeting fires once a local calendar day, the first time the link comes up or an Agent does something, whichever comes first. A welcome back is the same kind of line for coming back to work later in a day: an Agent doing something after three quiet hours, never right after a greeting.
- **The buddy still never speaks up on its own.** Each special occasion rides an edge that would have made a sound anyway; the daily greeting is the one exception, and it happens once a day at the moment the user plugs in or starts working.
- **Mute wins.** A muted box swallows every line, special or not. The daemon still counts the occasion as used.

The daemon decides the occasion for agent announcements, because it owns today's stats, the clock and the link. The firmware raises the two Pomodoro occasions itself, as before; they get no late-night variants.

### Drawing a line

Each occasion has a pool: about 8 lines for `input_required` and `done`, 3 to 4 for the rest, around 55 in all. The firmware picks one at random, skipping the last 3 it played for that occasion (or all but one, when the pool is smaller than four). If the occasion has no lines in the current pack, it follows the fallback column; if the pack has none of those either, it plays the built-in line.

### Rules every line follows

Whatever the persona, every line:

1. lasts at most 3 seconds, and is easy to make out from across the desk;
2. never mentions a session, a project or any code. The persona never sees them;
3. never mocks the user when something fails;
4. for `input_required` and `late_night_input`, is unmistakably a call for attention. All of a Character's needs-input lines start with the same call-out, such as "Hey".

### Forms of address

Some lines call the user something: 老板, 大佬, 哥, 姐 or 亲 in Chinese, boss, captain or buddy in English, or nothing. The user picks one per language in the App; every Character of that language then says it. The forms are a fixed list because every line is synthesized in advance (ADR-0007): a name the user typed would need speech synthesis at the moment they type it.

A line in `lines.tsv` that can carry a form of address has `{address}` where it goes. `tools/make-character.sh` synthesizes it once without a form (the `{address}` and the comma that joins it dropped) into `pack.bin`, and once with each form of the Character's language into `address/<form>.bin`, which holds only those lines, first in their pools in row order. When it writes a Character, the App swaps the lines of the picked form in for the plain ones, so the box only ever holds one form.

### Characters at launch

Four Characters, a woman and a man in each language, share the robot look, each with its own persona and pool, all spoken by Volcano Engine Doubao voices. A Character speaks one language; the App suggests Characters in the UI language first, as it did for voices.

| Character id | Language | Voice |
| --- | --- | --- |
| `amanda` | en | Doubao `en_female_amanda_mars_bigtts` |
| `jackson` | en | Doubao `en_male_jackson_mars_bigtts` |
| `wanwanxiaohe` | zh | Doubao Wanwan Xiaohe |
| `ahu` | zh | Doubao `zh_male_wennuanahu_moon_bigtts` |

They replace the four voices the App used to offer: Jessica and Chris (ElevenLabs) gave way to Amanda and Jackson, since the built-in voice is still ElevenLabs Jessica and two different voices both called Jessica would only confuse; Xiaohe 2.0 was dropped, and Ahu, a steady senior colleague, joined so Chinese has a man's voice too.

With no Character pack on the box, the built-in default Character speaks Jessica's original five fixed lines.

### Where the lines come from

Lines are written in advance, never when they are spoken (ADR-0007). For each Character the repo holds:

```text
characters/<id>/
  persona.md   who they are and how they talk
  lines.tsv    occasion<TAB>line, one line per row
```

A language model drafts `lines.tsv` from `persona.md` and the rules above (`tools/draft-lines.py` asks a local one); a person listens to the synthesized result and deletes what doesn't work. Nothing in the App or the daemon calls a language model. `tools/make-character.sh` synthesizes each line with the Character's TTS voice, normalizes it and builds the Character pack, which the App bundles.

### Character pack format

One Character pack fills the `voices` partition (2 MB, unchanged), written whole over the serial protocol exactly as voice packs were (`voice.begin` / `voice.chunk` / `voice.end`). Old firmware sees an unknown magic and falls back to the built-in voice.

1024-byte header, little endian:

```text
   0  magic "VBCP"
   4  u32 format version, currently 1
   8  u32 payload length (bytes after the header)
  12  u32 payload CRC32 (zlib)
  16  char[32] character id, NUL-terminated
  48  u32 sample rate, 16000
  52  u8  audio codec, 1 = IMA ADPCM, 4 bits, mono
  53  u8  occasion count (12)
  54  u16 line count, at most 111
  56  occasion table: per occasion, u16 first line index, u16 line count (0 = no pool)
 128  line table: per line, u32 offset from the start of the pack, u32 sample count
1020  u32 CRC32 of bytes 0..1020
```

Occasions are numbered in the order of the table above. Each line's audio is a standalone IMA ADPCM stream: predictor and step index start at zero, two samples per byte, low nibble first, `ceil(samples / 2)` bytes. The firmware upsamples 16 kHz to the codec's 24 kHz with linear interpolation and duplicates it to both channels.

At 16 kHz ADPCM a second of speech is 8 KB, so the whole pool is about 0.9 MB: writing it over the UART bridge takes about three minutes, as a voice pack did.

The Pomodoro chime is no longer baked into the two Pomodoro lines. The firmware keeps one copy of each chime and plays it before whichever `focus_done` or `break_done` line it picks.

### Protocol

Agent state events gain an optional `occasion` field naming a special occasion; it only changes which pool the announcement draws from:

```json
{"version":1,"event":"task.done","occasion":"first_done","tasks":[],"stats":["1 DONE","0 ASKS","0M BUSY"]}
```

The daily greeting is a new event that changes nothing on screen:

```json
{"version":1,"event":"buddy.say","occasion":"greeting_morning"}
```

Firmware that doesn't know `occasion` ignores it and plays the ordinary line; firmware that doesn't know `buddy.say` ignores the event.

## Phase 2: looks

Each of the four Characters gets a look of its own; the code-drawn robot stays the built-in default Character's look, shown when the box has no Character pack.

The robot is a Character the user can pick again at any time: it comes first on the Character tab. Its card picks who lends it voice and lines, by default the built-in voice's five fixed lines (`voices/jessica/`, packed as `robot`); with a preset lending them, the App writes that Character's pack, form of address included, without its look and under the id `robot`, and the box draws the robot. A box with no pack at all also counts as wearing the robot.

### Four key frames, moved by the firmware

A look is four still frames, not an animation: **normal**, **eyes closed**, **happy** and **sad**. The firmware does all the moving, so a look stays something an image model, or a person, can draw consistently:

| What the buddy is doing | Frame | Motion and marks |
| --- | --- | --- |
| Idle | normal, eyes closed for a blink now and then | breathes up and down |
| Working | normal | a slight sway, `...` above the head |
| Needs input | normal | `!?` above the head |
| Done | happy | hops |
| Failed | sad | sinks a little |
| Sleeping (lost link, sleepy tier) | eyes closed | `Z` floating up; on a lost link the whole screen is gray as before |
| Walking (leisure) | normal | slides sideways with a bob, mirrored when turning |

The skits with props (the ball, the book, the stars) stay with the robot; other Characters spend leisure walking, sleeping and resting. If four frames turn out to be too stiff, optional frames can be added on top later.

### Pixel spec

Each frame is **48 wide × 64 high**, drawn at 2× (96 × 128 on screen), with at most **16 colors** shared by the four frames, one of them transparent. A chibi figure is about twice as tall as it is wide; a square frame left half of it empty, and at 64 high on screen the head already reaches the title. A mockup of an image-model figure reduced to this spec on a real screenshot held up at 16 colors.

### State colors

A sprite is never recolored by state. The status bar and the task cards carry the state colors; on a lost link the whole Character is drawn gray, like the rest of the screen.

### Where looks come from

- **Preset looks.** A local image model (Qwen-Image) draws the base figure and edits it into the other three expressions; a script reduces the colors, removes the background and aligns the frames; a person approves the result. As with lines, the tool drafts and a person decides.
- **Custom looks.** The App's Character tab takes one to four images (normal, eyes closed, happy, sad, in file name order; one image stands in for all four) on a plain white or transparent background, made with any tool; "Copy a prompt" gives a starting prompt for an image model. The App removes the background, scales and aligns the figures and reduces the colors exactly as `tools/draft-look.py` does, and writes the result as the Character `custom`. It never calls an image model (ADR-0009). The Linux app doesn't offer this yet.
- **A custom Character is a custom look.** The user picks a preset Character to lend its voice, persona and lines.

### Storage

The look travels inside the same Character pack, so changing Character rewrites the whole pack (a look adds about 6 KB). The pack format gets a new version for it; packs without a look keep drawing the robot.
