#!/usr/bin/env bash
# 在 Mac 上把固件的画面渲染成 PNG，休闲剧目渲染成 GIF（需要 ffmpeg），
# 烧录前先看版式和动画。
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
    "${repo_root}/firmware/main/agent_leisure.c" \
    "${repo_root}/firmware/main/agent_pomodoro.c" \
    -lm -o "${binary}"

"${binary}" "${output_dir}"
ffmpeg -loglevel error -y -framerate 10 -i "${output_dir}/pomodoro_alarm_%03d.ppm" \
    -vf "scale=640:480:flags=neighbor,split[a][b];[a]palettegen[p];[b][p]paletteuse" \
    "${output_dir}/pomodoro_alarm.gif"
rm -f "${output_dir}"/pomodoro_alarm_[0-9]*.ppm
for ppm in "${output_dir}"/*.ppm; do
    ffmpeg -loglevel error -y -i "${ppm}" -vf scale=640:480:flags=neighbor "${ppm%.ppm}.png"
    rm -f "${ppm}"
done

"${binary}" "${output_dir}" leisure
for first in "${output_dir}"/skit_*_000.ppm; do
    name="$(basename "${first}" _000.ppm)"
    ffmpeg -loglevel error -y -framerate 8 -i "${output_dir}/${name}_%03d.ppm" \
        -vf "scale=640:480:flags=neighbor,split[a][b];[a]palettegen[p];[b][p]paletteuse" \
        "${output_dir}/${name}.gif"
    rm -f "${output_dir}/${name}"_*.ppm
done
ls "${output_dir}"
