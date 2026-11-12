#!/usr/bin/env bash
# The release wasm must draw what the host tests draw: `vibeplay shot` takes each scene from the wasm, with fixed data
# and a key script, and tests/wasm_shots.rs replays them on the host and compares. It needs a vibeplay CLI with
# 640 × 480 frames (vibeplay-sdk 9e8e424 or later); VIBEPLAY=<path> picks one. With KEEP=<folder> the shots stay there.
set -euo pipefail

app_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
vibeplay="${VIBEPLAY:-vibeplay}"
work="${KEEP:-$(mktemp -d "${TMPDIR:-/tmp}/compare-wasm-shots.XXXXXX")}"
[[ -n "${KEEP:-}" ]] || trap 'rm -rf "${work}"' EXIT
mkdir -p "${work}"

cargo build --quiet --release --target wasm32-unknown-unknown --locked --manifest-path "${app_root}/Cargo.toml"
wasm="${app_root}/target/wasm32-unknown-unknown/release/vibeplay_app.wasm"

# scene <screens> <name> <frame> <key script>: a VibePlay paired with one computer, none linked.
scene() {
    local dir="${work}/$1-$2"
    mkdir -p "${dir}/data"
    head -c 32 /dev/zero | tr '\0' '\7' > "${dir}/data/identity"
    printf '[{"key":"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE","name":"Desk Mac"}]' > "${dir}/data/computers"
    printf '{"muted":false,"screens":"%s"}' "$1" > "${dir}/data/settings"
    printf '%s\n' "$3" > "${dir}/frame"
    printf '%b' "$4" > "${dir}/input"
    rm -rf "${dir}/wasm-data" && cp -R "${dir}/data" "${dir}/wasm-data"
    "${vibeplay}" shot "${wasm}" --frames "$3" --input "${dir}/input" --data "${dir}/wasm-data" --out "${dir}/wasm.png" > /dev/null 2>&1
}

for screens in robot korekore; do
    scene "${screens}" looking 40 ''
    scene "${screens}" looking-later 75 ''
    scene "${screens}" menu 40 '30 x\n'
    scene "${screens}" menu-screens 50 '30 x\n34 down\n38 down\n'
    scene "${screens}" menu-forget 50 '30 x\n34 down\n38 a\n'
done

VIBEPLAY_WASM_SHOTS="${work}" cargo test --quiet --locked --manifest-path "${app_root}/Cargo.toml" --test wasm_shots -- --nocapture
