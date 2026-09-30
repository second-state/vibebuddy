#!/usr/bin/env python3
"""Synthesize one line with ElevenLabs text-to-speech and save it as a 24 kHz, 16-bit, mono WAV.

Usage:
    ELEVENLABS_API_KEY=<API key from the ElevenLabs console> tools/elevenlabs-tts.py <voice id> <text> <output.wav>
    ELEVENLABS_API_KEY=... tools/elevenlabs-tts.py --list

Asks for output_format=pcm_24000, which is raw little-endian 16-bit mono PCM at the rate the
firmware plays, so no resampling happens here. ELEVENLABS_MODEL picks the model (default
eleven_multilingual_v2). Audio shipped in the repo must come from a paid plan: the free plan
carries no commercial license. --list prints the voices the key can use (id, name, category),
to check a voice id before generating; "premade" voices are ElevenLabs' own, the ones to ship.
"""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
import wave

API = "https://api.elevenlabs.io/v1"
SAMPLE_RATE = 24000
DEFAULT_MODEL = "eleven_multilingual_v2"


def request(key: str, path: str, body: dict | None = None) -> bytes:
    data = json.dumps(body).encode("utf-8") if body is not None else None
    headers = {"xi-api-key": key}
    if data is not None:
        headers["Content-Type"] = "application/json"
    try:
        with urllib.request.urlopen(urllib.request.Request(f"{API}{path}", data=data, headers=headers), timeout=60) as response:
            return response.read()
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")[:300]
        sys.exit(f"ElevenLabs rejected the request: HTTP {error.code}: {detail}")


def synthesize(key: str, model: str, voice_id: str, text: str) -> bytes:
    query = urllib.parse.urlencode({"output_format": f"pcm_{SAMPLE_RATE}"})
    pcm = request(key, f"/text-to-speech/{urllib.parse.quote(voice_id)}?{query}", {"text": text, "model_id": model})
    if not pcm:
        sys.exit("ElevenLabs returned no audio data")
    return pcm


def list_voices(key: str) -> None:
    voices = json.loads(request(key, "/voices"))["voices"]
    for voice in sorted(voices, key=lambda v: (v.get("category") != "premade", v["name"])):
        labels = ", ".join(f"{k}={v}" for k, v in (voice.get("labels") or {}).items())
        print(f"{voice['voice_id']}  {voice['name']:<12} {voice.get('category', '?'):<10} {labels}")


def main(argv: list[str]) -> None:
    key = os.environ.get("ELEVENLABS_API_KEY")
    if not key:
        sys.exit("ELEVENLABS_API_KEY must be set")
    if argv[1:] == ["--list"]:
        list_voices(key)
        return
    if len(argv) != 4:
        sys.exit(__doc__)
    voice_id, text, output_path = argv[1:]
    pcm = synthesize(key, os.environ.get("ELEVENLABS_MODEL", DEFAULT_MODEL), voice_id, text)
    with wave.open(output_path, "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(SAMPLE_RATE)
        output.writeframes(pcm)
    print(f"{output_path}: {len(pcm) / 2 / SAMPLE_RATE:.2f} s")


if __name__ == "__main__":
    main(sys.argv)
