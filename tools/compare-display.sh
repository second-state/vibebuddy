#!/usr/bin/env bash
# Rust 固件的画面必须与 C 固件逐像素一致：两边各渲染同一组场景（含全部休闲
# 剧目的每一帧），逐个文件比对。C 固件删掉之前，改绘制代码后都跑一遍。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d -t compare-display)"
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
        echo "不一致: ${name}"
    fi
done
echo "比对 ${total} 帧，不一致 ${different} 帧"
[[ ${different} -eq 0 ]]
