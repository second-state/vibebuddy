# Voice Library

The same set of five prompts, synthesized once in each of several voices, for users to choose from in the macOS app. Each subdirectory is a complete set of firmware assets in exactly the same format as `firmware/main/assets/` (24 kHz, 16-bit, stereo, little-endian PCM, -1 dBFS peak, with a chime prepended to the two Pomodoro lines); copy one over and rebuild to change the voice. `jessica/` is byte-for-byte identical to the current firmware assets.

| Directory | Voice | Engine | Notes |
|---|---|---|---|
| `wanwanxiaohe/` | Wanwan Xiaohe (湾湾小何) `zh_female_wanwanxiaohe_moon_bigtts` | Volcano Engine Doubao Speech 1.0 (`seed-tts-1.0`) | Taiwanese accent, same voice as the Xiaozhi speaker; the firmware's built-in voice until 2026-09-29 |
| `xiaohe2/` | Xiaohe 2.0 (小何 2.0) `zh_female_xiaohe_uranus_bigtts` | Volcano Engine Doubao Speech 2.0 (`seed-tts-2.0`) | Mandarin, the 2.0 version of the same character |

The five lines are fixed as: 需要你确认 ("I need your confirmation") / 任务完成 ("task done") / 任务遇到问题 ("the task ran into a problem") / 专注结束，休息一下 ("focus is over, take a break") / 休息结束 ("break is over"), corresponding to `input_required` / `done` / `failed` / `focus_done` / `break_done`.

The five English lines are `Hey, I need you for a sec.` / `All done!` / `Uh-oh, something went wrong.` / `Nice work. Time for a break!` / `Break's over. Back to it!`: conversational, short, and easy to hear from across the desk. `VOICE_LANG` defaults to `en`; generating a Chinese voice requires `VOICE_LANG=zh`.

## English voices

English voices are synthesized with ElevenLabs, using its own premade voices (not ones from the community Voice Library, which may come with their creators' terms). Checked-in audio must come from a paid plan: the free plan doesn't include a commercial license.

| Directory | Voice | Engine | Notes |
|---|---|---|---|
| `jessica/` | Jessica `cgSgspJ2msm6clMCkdW9` | ElevenLabs (`eleven_multilingual_v2`) | American female, bright and warm; the firmware's built-in voice |
| `chris/` | Chris `iP95p4xoKVk53GoZ742B` | ElevenLabs (`eleven_multilingual_v2`) | American male, easygoing and natural |

```bash
# The key is an API key from the ElevenLabs console and stays out of the repo; first confirm the voice ids are available on this account
ELEVENLABS_API_KEY=... tools/elevenlabs-tts.py --list
ELEVENLABS_API_KEY=... VOICE_LANG=en OUT_DIR=voices/jessica tools/make-voices.sh
ELEVENLABS_API_KEY=... VOICE_LANG=en OUT_DIR=voices/chris ELEVENLABS_VOICE=iP95p4xoKVk53GoZ742B tools/make-voices.sh
```

Once a directory has PCM in it, `app/scripts/build-app.sh` builds a voice pack from it, and these two entries show up in the app's voice list. To use a different English voice, the directory name must match the id in `VoiceCatalogEntry.all` in `app/Sources/VibeBuddyCore/VoicePack.swift`, with `language: .en` set there. English lines are longer than Chinese ones, so a full set is about 1.5 MB, still within the 2 MB `voices` partition.

When both the Volcano Engine and ElevenLabs keys are set, `make-voices.sh` uses Volcano Engine first; with neither set it uses edge-tts. To choose explicitly, set `TTS_ENGINE=volc|elevenlabs|edge`.

## Regenerating

```bash
# Volcano Engine voices (the key is an API key from the Doubao Speech console and stays out of the repo)
VOLC_API_KEY=... VOICE_LANG=zh OUT_DIR=voices/wanwanxiaohe tools/make-voices.sh
VOLC_API_KEY=... VOICE_LANG=zh OUT_DIR=voices/xiaohe2 VOLC_VOICE=zh_female_xiaohe_uranus_bigtts VOLC_RESOURCE_ID=seed-tts-2.0 tools/make-voices.sh
```

edge-tts (Microsoft Edge's read-aloud service) needs no key and is handy for local previews, but it isn't a public API and there's no word on whether the audio it generates can be redistributed, so it isn't checked in. On 2026-09-29 the three edge-tts voices that used to be checked in were removed: HsiaoYu (晓雨) `hsiaoyu`, HsiaoChen (晓臻) `hsiaochen` and Xiaoxiao (晓晓) `xiaoxiao`.

## Building voice packs

What gets written into the device's `voices` partition is a voice pack (ADR-0003), which a script builds from a voice directory:

```bash
tools/make_voice_pack.py voices/wanwanxiaohe wanwanxiaohe build/wanwanxiaohe.bin
```

Packs aren't checked in; they're built fresh when the app is built. The format is in `firmware/main/agent_voice_pack.h`, and each side has its own tests: `tools/test-voice-pack.sh` (firmware parsing) and `python3 tools/test_make_voice_pack.py` (pack layout).

Doubao Speech 2.0 and ElevenLabs produce slightly different output on each synthesis; Doubao 1.0 is essentially stable. To preview, convert to WAV:

```bash
ffmpeg -f s16le -ar 24000 -ac 2 -i voices/jessica/done.pcm done.wav
```
