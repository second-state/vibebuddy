#!/usr/bin/env python3
"""Draft a Character's look with an image model: four expressions of one figure, as a look sheet.

Usage:
    tools/draft-look.py <character id> [--redraw]

Reads the figure's description from characters/<id>/look.txt and asks an OpenAI-compatible image
endpoint (IMAGE_BASE_URL, default http://127.0.0.1:8001/v1, a local Qwen-Image server; IMAGE_API_KEY if
it needs one) for the normal figure, then edits that image into eyes closed, happy and sad. The four
raw images are kept in characters/<id>/look-raw/ (not committed), so changing how they are processed
needs no new drawing; --redraw draws them again.

Processing: the white background is flood-filled away from the edges, each figure is cropped,
scaled by the same factor so the normal one stands 60 pixels tall, and set bottom-center in its
48 × 64 cell. The result is characters/<id>/look.png, a draft for a person to approve (or touch up)
before tools/make-character.sh puts it in the pack. Needs Pillow.
"""

from __future__ import annotations

import base64
import io
import json
import os
import sys
import urllib.request
import uuid
from collections import deque
from pathlib import Path

from PIL import Image

REPO = Path(__file__).resolve().parent.parent
CELL_W, CELL_H, FIGURE_H = 48, 64, 60
STYLE = (
    "pixel art game sprite, chibi proportions, standing, front view, full body, centered, arms down, "
    "flat colors, thick dark outline, limited 16-color palette, plain solid white background, "
    "16-bit retro game style"
)
EXPRESSIONS = {
    "normal": None,
    "closed": "the same character in exactly the same pose, clothes, colors and framing, with the eyes closed and a calm face",
    "happy": "the same character in exactly the same pose, clothes, colors and framing, with a big happy smile and joyful eyes",
    "sad": "the same character in exactly the same pose, clothes, colors and framing, with a sad face, frowning, eyes looking down",
}


def endpoint() -> tuple[str, dict[str, str]]:
    base = os.environ.get("IMAGE_BASE_URL", "http://127.0.0.1:8001/v1").rstrip("/")
    key = os.environ.get("IMAGE_API_KEY")
    return base, ({"Authorization": f"Bearer {key}"} if key else {})


def image_of(reply: dict) -> Image.Image:
    return Image.open(io.BytesIO(base64.b64decode(reply["data"][0]["b64_json"])))


def generate(prompt: str, seed: int) -> Image.Image:
    base, headers = endpoint()
    body = json.dumps({"prompt": prompt, "size": "1024x1024", "steps": 30, "seed": seed}).encode()
    request = urllib.request.Request(base + "/images/generations", body, {**headers, "Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=1800) as response:
        return image_of(json.load(response))


def edit(source: Image.Image, prompt: str, seed: int) -> Image.Image:
    base, headers = endpoint()
    boundary = uuid.uuid4().hex
    png = io.BytesIO()
    source.save(png, "PNG")
    parts = []
    for name, value in (("prompt", prompt), ("seed", str(seed)), ("steps", "30"), ("size", "1024x1024")):
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode())
    parts.append(
        f'--{boundary}\r\nContent-Disposition: form-data; name="image"; filename="figure.png"\r\n'
        f"Content-Type: image/png\r\n\r\n".encode() + png.getvalue() + b"\r\n"
    )
    parts.append(f"--{boundary}--\r\n".encode())
    request = urllib.request.Request(
        base + "/images/edits", b"".join(parts), {**headers, "Content-Type": f"multipart/form-data; boundary={boundary}"}
    )
    with urllib.request.urlopen(request, timeout=1800) as response:
        return image_of(json.load(response))


def cut_out(image: Image.Image) -> Image.Image:
    """Removes the background reachable from the edges: near-white pixels, flood-filled, so white
    inside the figure (eyes, shoes) stays."""
    rgba = image.convert("RGBA")
    pixels = rgba.load()
    width, height = rgba.size
    background = lambda x, y: min(pixels[x, y][:3]) > 225
    seen = bytearray(width * height)
    queue = deque((x, y) for x in range(width) for y in (0, height - 1))
    queue.extend((x, y) for y in range(height) for x in (0, width - 1))
    while queue:
        x, y = queue.popleft()
        if not (0 <= x < width and 0 <= y < height) or seen[y * width + x] or not background(x, y):
            continue
        seen[y * width + x] = 1
        pixels[x, y] = (0, 0, 0, 0)
        queue.extend(((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)))
    return rgba


def cell(figure: Image.Image, scale: float) -> Image.Image:
    box = figure.getbbox()
    cropped = figure.crop(box)
    size = (max(1, round(cropped.width * scale)), max(1, round(cropped.height * scale)))
    # Shrink color and coverage separately, then keep only pixels that are mostly figure: no halo.
    color = cropped.convert("RGB").resize(size, Image.Resampling.BOX)
    alpha = cropped.getchannel("A").resize(size, Image.Resampling.BOX).point(lambda a: 255 if a >= 140 else 0)
    small = color.convert("RGBA")
    small.putalpha(alpha)
    out = Image.new("RGBA", (CELL_W, CELL_H), (0, 0, 0, 0))
    out.paste(small, ((CELL_W - size[0]) // 2, CELL_H - size[1]), small)
    return out


def main(argv: list[str]) -> None:
    if len(argv) not in (2, 3) or (len(argv) == 3 and argv[2] != "--redraw"):
        sys.exit(__doc__)
    character = REPO / "characters" / argv[1]
    description = (character / "look.txt").read_text().strip()
    raw = character / "look-raw"
    raw.mkdir(exist_ok=True)
    seed = sum(argv[1].encode())
    images = {}
    for name, change in EXPRESSIONS.items():
        path = raw / f"{name}.png"
        if path.exists() and len(argv) == 2:
            images[name] = Image.open(path)
            continue
        print(f"drawing {name}…", flush=True)
        images[name] = generate(f"{description}, {STYLE}", seed) if change is None else edit(images["normal"], change, seed)
        images[name].save(path)
    figures = {name: cut_out(image) for name, image in images.items()}
    normal_box = figures["normal"].getbbox()
    scale = FIGURE_H / (normal_box[3] - normal_box[1])
    sheet = Image.new("RGBA", (CELL_W * 4, CELL_H), (0, 0, 0, 0))
    for index, name in enumerate(EXPRESSIONS):
        sheet.paste(cell(figures[name], scale), (index * CELL_W, 0))
    sheet.save(character / "look.png")
    sheet.resize((sheet.width * 4, sheet.height * 4), Image.Resampling.NEAREST).save(raw / "preview.png")
    print(f"{character / 'look.png'}: a draft; preview at {raw / 'preview.png'}")


if __name__ == "__main__":
    main(sys.argv)
