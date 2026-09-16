#!/usr/bin/env bash
# 用神经网络语音重新生成固件里的五句话，写进 firmware/main/assets/。
#
# 语音来自 edge-tts（微软 Edge 的朗读接口，免费、不用密钥；不是正式公开的
# API，只用来一次性生成这几句）。默认音色是台湾女声 HsiaoYu；换音色改 VOICE。
# 每一句各自归一化到 -1 dBFS 峰值；番茄钟的两句前面拼上钟声。
#
# 用法: VOICE=zh-TW-HsiaoYuNeural tools/make-voices.sh
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
assets="${repo_root}/firmware/main/assets"
voice="${VOICE:-zh-TW-HsiaoYuNeural}"
work="$(mktemp -d -t voices)"
trap 'rm -rf "${work}"' EXIT

# 文件名 -> 台词。
lines=(
    "input_required|需要你确认"
    "done|任务完成"
    "failed|任务遇到问题"
    "focus_voice|专注结束，休息一下"
    "break_voice|休息结束"
)

synth() {
    local name="$1" text="$2"
    uvx --from edge-tts edge-tts --voice "${voice}" --text "${text}" \
        --write-media "${work}/${name}.mp3" >/dev/null
    # 先量峰值再补增益到 -1 dBFS；单声道转双声道会掉 3 dB，量的是转换后的结果。
    ffmpeg -loglevel error -y -i "${work}/${name}.mp3" -ar 24000 -ac 2 \
        -f s16le -acodec pcm_s16le "${work}/${name}.raw"
    local peak
    peak="$(ffmpeg -f s16le -ar 24000 -ac 2 -i "${work}/${name}.raw" -af volumedetect -f null - 2>&1 \
        | sed -n 's/.*max_volume: \(-*[0-9.]*\) dB.*/\1/p')"
    local gain
    gain="$(python3 -c "print(f'{-1.0 - float(\"${peak}\"):.2f}')")"
    ffmpeg -loglevel error -y -f s16le -ar 24000 -ac 2 -i "${work}/${name}.raw" \
        -af "volume=${gain}dB" -f s16le -acodec pcm_s16le "${work}/${name}.pcm"
    echo "${name}: 峰值 ${peak} dB，增益 ${gain} dB"
}

for entry in "${lines[@]}"; do
    synth "${entry%%|*}" "${entry#*|}"
done

cp "${work}/input_required.pcm" "${work}/done.pcm" "${work}/failed.pcm" "${assets}/"
"${repo_root}/tools/make-pomodoro-audio.py" focus "${work}/focus_voice.pcm" "${assets}/focus_done.pcm"
"${repo_root}/tools/make-pomodoro-audio.py" break "${work}/break_voice.pcm" "${assets}/break_done.pcm"
ls -la "${assets}"/*.pcm
