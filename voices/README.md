# Voice Library

Since the Characters (see [`docs/characters.md`](../docs/characters.md)), the voices the App offers are Characters in [`characters/`](../characters/), each with its own pool of lines. What is left here is the source of the firmware's built-in voice.

| Directory | Voice | Engine | Notes |
|---|---|---|---|
| `jessica/` | Jessica `cgSgspJ2msm6clMCkdW9` | ElevenLabs (`eleven_multilingual_v2`) | American female; byte for byte the firmware's built-in five lines in `firmware/main/assets/` |

The five files are 24 kHz, 16-bit, stereo, little-endian PCM at a -1 dBFS peak, with a chime in front of the two Pomodoro lines. The lines are `Hey, I need you for a sec.` / `All done!` / `Uh-oh, something went wrong.` / `Nice work. Time for a break!` / `Break's over. Back to it!`. Checked-in ElevenLabs audio must come from a paid plan: the free plan doesn't include a commercial license.

```bash
ELEVENLABS_API_KEY=... VOICE_LANG=en OUT_DIR=voices/jessica tools/make-voices.sh
cp voices/jessica/*.pcm firmware/main/assets/
```

`tools/make_voice_pack.py` still builds the older five-line voice pack (ADR-0003) from a directory like this one; the firmware keeps playing such packs until the App writes a Character pack.

The voices that used to live here were retired on 2026-10-06: `chris` (ElevenLabs) became the Character Jackson with a Doubao voice (himself retired on 2026-10-08), `wanwanxiaohe` became the Character of the same name, and `xiaohe2` was dropped.
