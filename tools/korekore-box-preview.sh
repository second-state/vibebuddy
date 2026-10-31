#!/usr/bin/env bash
# Draw every korekore box screen on the Mac (ADR-0021) and save them as PNGs at 2×, to set beside the design's box
# boards in docs/design/korekore. A look sheet (characters/<id>/look.png) shows the screens wearing that Character.
#
# Usage: tools/korekore-box-preview.sh <output directory> [look.png]
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="${1:?usage: tools/korekore-box-preview.sh <output directory> [look.png]}"
mkdir -p "${out}"
work="$(mktemp -d -t korekore-box)"
trap 'rm -rf "${work}"' EXIT

look_args=()
if [[ -n "${2:-}" ]]; then
    "${repo_root}/tools/make-look.py" "$2" "${work}/look.bin"
    look_args=("${work}/look.bin")
fi
cargo run --quiet --manifest-path "${repo_root}/Cargo.toml" -p vibebuddy-firmware-core --example korekore_preview -- "${work}" "${look_args[@]}"
python3 -I - "${work}" "${out}" <<'EOF'
import sys
from pathlib import Path
from PIL import Image

source, target = Path(sys.argv[1]), Path(sys.argv[2])
for ppm in sorted(source.glob("*.ppm")):
    image = Image.open(ppm)
    image.resize((image.width * 2, image.height * 2), Image.NEAREST).save(target / f"{ppm.stem}.png")
    print(target / f"{ppm.stem}.png")
EOF
