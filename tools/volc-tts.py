#!/usr/bin/env python3
"""用火山引擎豆包语音合成一句话，存成 24 kHz、16-bit、单声道 WAV。

用法：
    VOLC_API_KEY=<豆包语音控制台的 API Key> tools/volc-tts.py <音色> <文本> <输出.wav>

走的是 V3 HTTP Chunked 单向流式接口：服务端逐块返回 JSON，每块的 data 是
base64 的 PCM；code 20000000 表示合成结束。X-Api-Resource-Id 决定模型版本
和计费项：默认 seed-tts-1.0（湾湾小何等 1.0 音色），2.0 音色要用
VOLC_RESOURCE_ID=seed-tts-2.0。两个版本都得先在控制台开通对应的字符版。
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
