#!/usr/bin/env python3
"""Draft a Character's lines with a language model, from its persona and the brief in characters/README.md.

Usage:
    LLM_API_KEY=... tools/draft-lines.py <character id> [--force]

Talks to any OpenAI-compatible chat endpoint: LLM_BASE_URL (default http://localhost:8000/v1, a local
oMLX server) and LLM_MODEL (default: the server's first model). Writes characters/<id>/lines.tsv,
refusing to overwrite one unless --force. This is a first draft: a person listens to the result and
edits lines.tsv before anything is committed (docs/characters.md).
"""

from __future__ import annotations

import json
import os
import re
import sys
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
COUNTS = {
    "input_required": 8,
    "done": 8,
    "failed": 4,
    "focus_done": 4,
    "break_done": 4,
    "first_done": 3,
    "milestone": 3,
    "late_night_done": 3,
    "late_night_input": 3,
    "greeting_morning": 3,
    "greeting_afternoon": 3,
    "greeting_evening": 3,
}


def request(base: str, key: str, path: str, body: dict | None = None) -> dict:
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Authorization": f"Bearer {key}", "Content-Type": "application/json"}
    with urllib.request.urlopen(urllib.request.Request(base + path, data, headers), timeout=600) as response:
        return json.load(response)


def brief() -> str:
    readme = (REPO / "characters" / "README.md").read_text()
    return readme[readme.index("### The brief") :].split("\n", 1)[1].strip()


def parse(text: str) -> dict[str, list[str]]:
    lines: dict[str, list[str]] = {occasion: [] for occasion in COUNTS}
    for row in text.splitlines():
        match = re.match(r"\s*`?([a-z_]+)`?\s*(?:\t|\s{2,}|:|\|)\s*(.+?)\s*$", row)
        if match and match.group(1) in lines:
            lines[match.group(1)].append(match.group(2).strip().strip('"“”'))
    return lines


def main(argv: list[str]) -> None:
    if len(argv) not in (2, 3) or (len(argv) == 3 and argv[2] != "--force"):
        sys.exit(__doc__)
    character = REPO / "characters" / argv[1]
    output = character / "lines.tsv"
    if output.exists() and len(argv) == 2:
        sys.exit(f"{output} exists; pass --force to replace it")
    key = os.environ.get("LLM_API_KEY", "")
    base = os.environ.get("LLM_BASE_URL", "http://localhost:8000/v1").rstrip("/")
    model = os.environ.get("LLM_MODEL") or request(base, key, "/models")["data"][0]["id"]
    persona = (character / "persona.md").read_text()
    prompt = f"{brief()}\n\n## The persona\n\n{persona}"
    reply = request(
        base,
        key,
        "/chat/completions",
        {
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0.9,
            "max_tokens": 4000,
            # Qwen's thinking would land in the reply text; the lines are short enough not to need it.
            "chat_template_kwargs": {"enable_thinking": False},
        },
    )
    lines = parse(reply["choices"][0]["message"]["content"])
    short = [f"{occasion} ({len(lines[occasion])}/{count})" for occasion, count in COUNTS.items() if len(lines[occasion]) < count]
    rows = [f"{occasion}\t{text}" for occasion, texts in lines.items() for text in texts[: COUNTS[occasion]]]
    output.write_text("\n".join(rows) + "\n")
    print(f"{output}: {len(rows)} lines from {model}")
    if short:
        print("fewer lines than asked for: " + ", ".join(short), file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv)
