#!/usr/bin/env bash
# Draw each decoration (ADR-0022) the way the firmware does, over both sets of box screens, and save the previews
# the apps play on the Character page: decorations/<screens>-<decoration>.png, the frames side by side, 320 × 240
# each (one frame for off). Run it again after changing firmware-rs/core/src/decoration.rs or the ready screens.
#
# Usage: tools/decoration-previews.sh [directory for the state shots too]
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="${1:-$(mktemp -d -t decoration-previews)}"
[[ -n "${1:-}" ]] || trap 'rm -rf "${work}"' EXIT
mkdir -p "${work}"
cargo run --quiet --manifest-path "${repo_root}/Cargo.toml" -p vibebuddy-firmware-core --example decoration_preview -- "${work}"
python3 -I - "${work}" "${repo_root}/decorations" <<'EOF'
import sys
from collections import defaultdict
from pathlib import Path
from PIL import Image

source, target = Path(sys.argv[1]), Path(sys.argv[2])
target.mkdir(exist_ok=True)
strips = defaultdict(list)
for ppm in sorted(source.glob("frame-*.ppm")):
    _, screens, decoration, _ = ppm.stem.split("-")
    strips[f"{screens}-{decoration}"].append(Image.open(ppm).convert("RGB"))
for name, frames in sorted(strips.items()):
    width, height = frames[0].size
    strip = Image.new("RGB", (width * len(frames), height))
    for index, frame in enumerate(frames):
        strip.paste(frame, (index * width, 0))
    strip.save(target / f"{name}.png", optimize=True)
    print(target / f"{name}.png")
EOF
