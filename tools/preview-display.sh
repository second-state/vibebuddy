#!/usr/bin/env bash
# 在 Mac 上把固件的画面渲染成 PNG（需要 ffmpeg），烧录前先看版式。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="${1:-${repo_root}/.preview}"
mkdir -p "${output_dir}"
binary="$(mktemp -t display_preview)"
trap 'rm -f "${binary}"' EXIT

cc -std=gnu11 -Wall -Wextra -Wno-unused-function -Wno-unused-parameter \
    -I "${repo_root}/firmware/host_tests/stubs" \
    -I "${repo_root}/firmware/main" \
    "${repo_root}/firmware/host_tests/display_preview.c" \
    "${repo_root}/firmware/main/agent_pomodoro.c" \
    -lm -o "${binary}"
"${binary}" "${output_dir}"
for ppm in "${output_dir}"/*.ppm; do
    ffmpeg -loglevel error -y -i "${ppm}" -vf scale=640:480:flags=neighbor "${ppm%.ppm}.png"
    rm -f "${ppm}"
done
ls "${output_dir}"
