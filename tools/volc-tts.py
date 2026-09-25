#!/usr/bin/env python3
"""Synthesize one line with Volcano Engine Doubao TTS and save it as a 24 kHz, 16-bit, mono WAV.

Usage:
    VOLC_API_KEY=<API key from the Doubao Speech console> tools/volc-tts.py <voice> <text> <output.wav>

Uses the V3 HTTP chunked one-way streaming API: the server returns JSON chunk by chunk, each chunk's data is
base64 PCM; code 20000000 means synthesis is finished. X-Api-Resource-Id selects the model version
and billing item: the default is seed-tts-1.0 (Wanwan Xiaohe and other 1.0 voices); 2.0 voices need
VOLC_RESOURCE_ID=seed-tts-2.0. Either version must first be enabled (the per-character plan) in the console.
"""

from __future__ import annotations

import base64
import json
import os
import sys
import urllib.error
import urllib.request
import uuid
import wave

URL = "https://openspeech.bytedance.com/api/v3/tts/unidirectional"
SAMPLE_RATE = 24000


def synthesize(key: str, resource_id: str, speaker: str, text: str) -> bytes:
    body = json.dumps(
        {
            "user": {"uid": "vibe-buddy"},
            "req_params": {
                "text": text,
                "speaker": speaker,
                "audio_params": {"format": "pcm", "sample_rate": SAMPLE_RATE},
            },
        }
    ).encode("utf-8")
    request = urllib.request.Request(
        URL,
        data=body,
        headers={
            "X-Api-Key": key,
            "X-Api-Resource-Id": resource_id,
            "X-Api-Request-Id": str(uuid.uuid4()),
            "Content-Type": "application/json",
        },
    )
    try:
        response = urllib.request.urlopen(request, timeout=60)
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")[:300]
        sys.exit(f"火山引擎拒绝请求 HTTP {error.code}: {detail}")

    pcm = bytearray()
    decoder = json.JSONDecoder()
    buffer = ""
    for chunk in response:
        buffer += chunk.decode("utf-8")
        while buffer.strip():
            stripped = buffer.lstrip()
            try:
                message, end = decoder.raw_decode(stripped)
            except json.JSONDecodeError:
                break
            buffer = stripped[end:]
            code = message.get("code")
            if code == 0:
                if message.get("data"):
                    pcm += base64.b64decode(message["data"])
            elif code != 20000000:
                sys.exit(f"火山引擎返回错误 {code}: {message.get('message')}")
    if not pcm:
        sys.exit("火山引擎没有返回音频数据")
    return bytes(pcm)


def main(argv: list[str]) -> None:
    if len(argv) != 4:
        sys.exit(__doc__)
    speaker, text, output_path = argv[1:]
    key = os.environ.get("VOLC_API_KEY")
    if not key:
        sys.exit("需要环境变量 VOLC_API_KEY")
    resource_id = os.environ.get("VOLC_RESOURCE_ID", "seed-tts-1.0")
    pcm = synthesize(key, resource_id, speaker, text)
    with wave.open(output_path, "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(SAMPLE_RATE)
        output.writeframes(pcm)
    print(f"{output_path}: {len(pcm) / 2 / SAMPLE_RATE:.2f} 秒")


if __name__ == "__main__":
    main(sys.argv)
