#!/usr/bin/env bash
# The VibePlay's screens must match the box's pixel for pixel: the firmware's previews draw their scenes, and
# tests/box_screens.rs reaches the same scenes through the app and compares each one. Run it after any change to the
# app's screens or to the firmware's drawing code. With VIBEPLAY_SHOTS=<folder> it also saves the app's shots.
set -euo pipefail

app_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo_root="$(cd "${app_root}/.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/compare-box-screens.XXXXXX")"
trap 'rm -rf "${work}"' EXIT

cargo build --quiet --locked --manifest-path "${repo_root}/Cargo.toml" -p vibebuddy-firmware-core --example preview --example korekore_preview
"${repo_root}/target/debug/examples/preview" "${work}/robot"
"${repo_root}/target/debug/examples/preview" "${work}/robot" leisure
"${repo_root}/target/debug/examples/korekore_preview" "${work}/korekore"

VIBEPLAY_BOX_PREVIEW="${work}" cargo test --locked --manifest-path "${app_root}/Cargo.toml" --test box_screens -- --nocapture
