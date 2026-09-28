#!/usr/bin/env bash
# The Rust firmware's screens must match the C firmware's pixel for pixel: both render the same scenes (including
# every frame of every leisure skit) and each file is compared. Until the C firmware is removed, run this after
# any change to the drawing code.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/compare-display.XXXXXX")"
trap 'rm -rf "${work}"' EXIT

cc -std=gnu11 -O1 -Wno-unused-function -Wno-unused-parameter \
    -I "${repo_root}/firmware/host_tests/stubs" \
    -I "${repo_root}/firmware/main" \
    "${repo_root}/firmware/host_tests/display_preview.c" \
    "${repo_root}/firmware/main/agent_leisure.c" \
    "${repo_root}/firmware/main/agent_pomodoro.c" \
    -lm -o "${work}/c_preview"
cargo build --quiet --manifest-path "${repo_root}/Cargo.toml" -p vibebuddy-firmware-core --example preview
rust_preview="${repo_root}/target/debug/examples/preview"

mkdir -p "${work}/c" "${work}/rust"
for mode in "" leisure; do
    "${work}/c_preview" "${work}/c" ${mode}
    "${rust_preview}" "${work}/rust" ${mode}
done

total=0
different=0
for c_file in "${work}"/c/*.ppm; do
    name="$(basename "${c_file}")"
    total=$((total + 1))
    if ! cmp -s "${c_file}" "${work}/rust/${name}"; then
        different=$((different + 1))
        echo "differs: ${name}"
    fi
done
echo "compared ${total} frames, ${different} differ"
[[ ${different} -eq 0 ]]
