#!/usr/bin/env bash
# Regenerate the firmware's five lines with neural TTS and write them to firmware/main/assets/.
#
# One of two engines:
# - with VOLC_API_KEY set, Volcano Engine Doubao TTS (tools/volc-tts.py), default voice
#   Wanwan Xiaohe (1.0 model); change voices with VOLC_VOICE, and for a 2.0 voice also set VOLC_RESOURCE_ID
#   to seed-tts-2.0;
# - otherwise edge-tts (Microsoft Edge's read-aloud service: free, no key; not an officially public
#   API, only used to generate these few lines once), default voice the Taiwanese female HsiaoYu; change with VOICE.
# Each line is normalized to a -1 dBFS peak on its own; the two pomodoro lines get a chime in front.
# VOICE_LANG picks the language of the five lines: zh (default) or en. English
# defaults the edge-tts voice to en-US-JennyNeural; the Doubao path just speaks
# whatever text it is given, so pair VOICE_LANG=en with an English VOLC_VOICE.
#
# Usage: tools/make-voices.sh
#       VOLC_API_KEY=... tools/make-voices.sh
#       OUT_DIR=voices/hsiaochen VOICE=zh-TW-HsiaoChenNeural tools/make-voices.sh
#       VOICE_LANG=en OUT_DIR=voices/jenny tools/make-voices.sh
#       VOICE_LANG=en OUT_DIR=voices/guy VOICE=en-US-GuyNeural tools/make-voices.sh
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Overwrites the firmware assets by default; to archive into the voice library, point OUT_DIR at voices/<voice>/.
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

# File name -> line. Both languages say the same five things, in the same order as
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
    # Measure the peak first, then add gain up to -1 dBFS; mono-to-stereo loses 3 dB, so measure after converting.
    ffmpeg -loglevel error -y -i "${source}" -ar 24000 -ac 2 \
        -f s16le -acodec pcm_s16le "${work}/${name}.raw"
    local peak
    peak="$(ffmpeg -f s16le -ar 24000 -ac 2 -i "${work}/${name}.raw" -af volumedetect -f null - 2>&1 \
        | sed -n 's/.*max_volume: \(-*[0-9.]*\) dB.*/\1/p')"
    local gain
    gain="$(python3 -c "print(f'{-1.0 - float(\"${peak}\"):.2f}')")"
    ffmpeg -loglevel error -y -f s16le -ar 24000 -ac 2 -i "${work}/${name}.raw" \
        -af "volume=${gain}dB" -f s16le -acodec pcm_s16le "${work}/${name}.pcm"
    echo "${name}: peak ${peak} dB, gain ${gain} dB"
}

for entry in "${lines[@]}"; do
    synth "${entry%%|*}" "${entry#*|}"
done

cp "${work}/input_required.pcm" "${work}/done.pcm" "${work}/failed.pcm" "${assets}/"
"${repo_root}/tools/make-pomodoro-audio.py" focus "${work}/focus_voice.pcm" "${assets}/focus_done.pcm"
"${repo_root}/tools/make-pomodoro-audio.py" break "${work}/break_voice.pcm" "${assets}/break_done.pcm"
ls -la "${assets}"/*.pcm
