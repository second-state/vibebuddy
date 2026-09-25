#!/usr/bin/env bash
# 用神经网络语音重新生成固件里的五句话，写进 firmware/main/assets/。
#
# 两个引擎二选一：
# - 设了 VOLC_API_KEY 就用火山引擎豆包语音（tools/volc-tts.py），默认音色
#   湾湾小何（1.0 模型），换音色改 VOLC_VOICE；2.0 音色还要把 VOLC_RESOURCE_ID
#   设成 seed-tts-2.0；
# - 否则用 edge-tts（微软 Edge 的朗读接口，免费、不用密钥；不是正式公开的
#   API，只用来一次性生成这几句），默认音色台湾女声 HsiaoYu，换音色改 VOICE。
# 每一句各自归一化到 -1 dBFS 峰值；番茄钟的两句前面拼上钟声。
# VOICE_LANG picks the language of the five lines: zh (default) or en. English
# defaults the edge-tts voice to en-US-JennyNeural; the Doubao path just speaks
# whatever text it is given, so pair VOICE_LANG=en with an English VOLC_VOICE.
#
# 用法: tools/make-voices.sh
#       VOLC_API_KEY=... tools/make-voices.sh
#       OUT_DIR=voices/hsiaochen VOICE=zh-TW-HsiaoChenNeural tools/make-voices.sh
#       VOICE_LANG=en OUT_DIR=voices/jenny tools/make-voices.sh
#       VOICE_LANG=en OUT_DIR=voices/guy VOICE=en-US-GuyNeural tools/make-voices.sh
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# 默认直接覆盖固件资产；归档到音色库时用 OUT_DIR 指到 voices/<音色>/。
assets="${OUT_DIR:-${repo_root}/firmware/main/assets}"
mkdir -p "${assets}"
language="${VOICE_LANG:-zh}"
case "${language}" in
    zh) default_voice="zh-TW-HsiaoYuNeural" ;;
    en) default_voice="en-US-JennyNeural" ;;
    *) echo "VOICE_LANG must be zh or en, got ${language}" >&2; exit 2 ;;
esac
voice="${VOICE:-${default_voice}}"
volc_voice="${VOLC_VOICE:-zh_female_wanwanxiaohe_moon_bigtts}"
work="$(mktemp -d -t voices)"
trap 'rm -rf "${work}"' EXIT

# 文件名 -> 台词。Both languages say the same five things, in the same order as
# the voice pack's clips (firmware/main/agent_voice_pack.h).
if [[ "${language}" == "en" ]]; then
    lines=(
        "input_required|Need your input."
        "done|Task complete."
        "failed|Task hit a problem."
        "focus_voice|Focus time's up. Take a break."
        "break_voice|Break's over."
    )
else
    lines=(
        "input_required|需要你确认"
        "done|任务完成"
        "failed|任务遇到问题"
        "focus_voice|专注结束，休息一下"
        "break_voice|休息结束"
    )
fi

synth() {
    local name="$1" text="$2" source
    if [[ -n "${VOLC_API_KEY:-}" ]]; then
        source="${work}/${name}.wav"
        "${repo_root}/tools/volc-tts.py" "${volc_voice}" "${text}" "${source}"
    else
        source="${work}/${name}.mp3"
        uvx --from edge-tts edge-tts --voice "${voice}" --text "${text}" \
            --write-media "${source}" >/dev/null
    fi
    # 先量峰值再补增益到 -1 dBFS；单声道转双声道会掉 3 dB，量的是转换后的结果。
    ffmpeg -loglevel error -y -i "${source}" -ar 24000 -ac 2 \
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
