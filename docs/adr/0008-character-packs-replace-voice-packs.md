---
status: accepted
---

# Character packs replace voice packs, in the same partition, with ADPCM audio

A Character's look, voice and lines always travel together, so they go into a single Character pack that fills the existing 2 MB `voices` partition and is written whole, as voice packs were (ADR-0003). Splitting looks and lines into two partitions would let a half-finished switch pair one Character's face with another's voice.

About 55 lines of speech don't fit in the old format: 24 kHz stereo PCM is 96 KB a second, so the pool would be about 10 MB. We store each line as 16 kHz mono IMA ADPCM, 8 KB a second, which puts the pool at about 0.9 MB and keeps the write over the UART bridge at about three minutes. The firmware decodes and upsamples to the codec's 24 kHz; the built-in lines stay PCM.

## Considered options

- **16 kHz mono PCM, uncompressed.** About 3.5 MB: the partition would have to grow, which means reflashing the partition table, and the bridge write would take about twelve minutes.
- **Two partitions, one for looks and one for lines.** Changing only the look would be faster, but the two halves could disagree.

## Consequences

The pack has a new magic, `VBCP`, so firmware from before this change rejects it and falls back to the built-in voice. The Pomodoro chime moves into the firmware and is played before the line instead of being baked into it. Changing only a Character's look still rewrites the whole pack.
