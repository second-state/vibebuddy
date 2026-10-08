#!/usr/bin/env bash
# Render a look sheet on the real screens before it goes into a pack: every duty state and leisure skit
# as a GIF, in .preview/look/<name>/ (needs ffmpeg and Pillow).
# Usage: tools/preview-look.sh characters/ada/look.png
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sheet="${1:?usage: tools/preview-look.sh <look.png>}"
name="$(basename "$(dirname "${sheet}")")"
output="${repo_root}/.preview/look/${name}"
rm -rf "${output}" && mkdir -p "${output}"
"${repo_root}/tools/make-look.py" "${sheet}" "${output}/look.bin" >/dev/null
cargo run -q --manifest-path "${repo_root}/Cargo.toml" -p vibebuddy-firmware-core --example look_preview -- \
    "${output}/look.bin" "${output}"
for first in "${output}"/*_000.ppm; do
    scene="$(basename "${first}" _000.ppm)"
    ffmpeg -loglevel error -y -framerate 8 -i "${output}/${scene}_%03d.ppm" \
        -vf "scale=640:480:flags=neighbor,split[a][b];[a]palettegen[p];[b][p]paletteuse" "${output}/${scene}.gif"
done
rm -f "${output}"/*.ppm
ls "${output}"
