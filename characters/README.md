# Characters

Each directory is one Character (see [`docs/characters.md`](../docs/characters.md)):

| File | What it is |
| --- | --- |
| `persona.md` | Who they are and how they talk; the only thing a line writer gets about them |
| `voice.env` | The TTS engine and voice that speak their lines |
| `lines.tsv` | Their lines: `occasion<TAB>text`, one per row |
| `pack.bin` | The built Character pack the App ships, from `tools/make-character.sh <id>` |

`audio/` is the synthesis cache and is not committed.

## Writing lines

Lines are drafted by a language model from the persona and the brief below, then a person listens to the synthesized result and deletes or rewrites what doesn't work. Nothing in the App or the daemon writes lines.

### The brief

You are writing the spoken lines of a small desk companion that watches the user's AI coding agents and speaks up when something happens. Write in the persona's language and voice. Every line:

1. takes at most 3 seconds to say, and is easy to catch from across the desk;
2. never mentions any session, project, file or code: you know nothing about the work, only that something happened;
3. never mocks the user when something fails;
4. for `input_required` and `late_night_input`, is unmistakably a call for attention, and all of them start with the same call-out (for example "Hey" or "喂");
5. sounds like the persona talking, not like a notification;
6. is something a real person would say out loud, and makes sense on its own: no riddles, no poetry, no metaphors that need explaining.

The agent is "it" (or simply left out); the user is "you". A `milestone` line is said on the 5th, the 10th and the 20th done alike, so it never says a number. A late-night line never suggests drinking.

Occasions, and how many lines each needs:

| Occasion | When it is said | Lines |
| --- | --- | --- |
| `input_required` | An agent is waiting for the user to answer or approve something | 8 |
| `done` | An agent finished a task | 8 |
| `failed` | An agent's task failed or got stuck | 4 |
| `focus_done` | A 25-minute Pomodoro focus ended (a chime plays first); time for a break | 4 |
| `break_done` | The 5-minute break ended; back to work | 4 |
| `first_done` | The first task done today | 3 |
| `milestone` | The 5th, 10th or 20th task done today (don't say the number) | 3 |
| `late_night_done` | A task done between 23:00 and 05:00 | 3 |
| `late_night_input` | An agent needs the user between 23:00 and 05:00 | 3 |
| `greeting_morning` | The user's first time at the computer today, before noon | 3 |
| `greeting_afternoon` | The same, in the afternoon | 3 |
| `greeting_evening` | The same, in the evening or at night | 3 |

Output one line per row as `occasion<TAB>text`, nothing else.
