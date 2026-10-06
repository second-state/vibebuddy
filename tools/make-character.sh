#!/usr/bin/env bash
# Synthesize a Character's lines and build its Character pack (docs/characters.md).
#
# Reads characters/<id>/voice.env and characters/<id>/lines.tsv:
# - voice.env sets ENGINE (volc or elevenlabs) and VOICE (the engine's voice id), and for a Doubao
#   2.0 voice VOLC_RESOURCE_ID=seed-tts-2.0. The API keys stay in your environment, never in the repo:
#   VOLC_API_KEY or ELEVENLABS_API_KEY (a paid plan; the free one has no commercial license).
# - lines.tsv has one line per row: occasion<TAB>text. Blank rows and rows starting with # are skipped.
#
# Each line is synthesized once and cached in characters/<id>/audio/<occasion>/, named by a hash of
# its text, so editing a few rows only re-synthesizes those; audio for rows that are gone is deleted.
# Lines are converted to 16 kHz mono and normalized to a -1 dBFS peak one by one. A line longer than
# 3 seconds is reported: every line has to be short enough to catch from across the desk.
#
# Usage: VOLC_API_KEY=... tools/make-character.sh wanwanxiaohe
#        ELEVENLABS_API_KEY=... tools/make-character.sh jessica
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
id="${1:?usage: tools/make-character.sh <character id>}"
dir="${repo_root}/characters/${id}"
[[ -f "${dir}/voice.env" && -f "${dir}/lines.tsv" ]] || { echo "${dir} needs voice.env and lines.tsv" >&2; exit 2; }
# shellcheck source=/dev/null
source "${dir}/voice.env"
case "${ENGINE:-}" in
    volc | elevenlabs) ;;
    *) echo "voice.env: ENGINE must be volc or elevenlabs" >&2; exit 2 ;;
esac
[[ -n "${VOICE:-}" ]] || { echo "voice.env: VOICE is not set" >&2; exit 2; }
if [[ -n "${VOLC_RESOURCE_ID:-}" ]]; then
    export VOLC_RESOURCE_ID
fi

audio="${dir}/audio"
work="$(mktemp -d -t character)"
trap 'rm -rf "${work}"' EXIT
mkdir -p "${audio}"
: > "${work}/keep"
too_long=0

synth() {
    local occasion="$1" text="$2" name source
    name="$(printf '%s' "${text}" | shasum -a 1 | cut -c1-10)"
    local target="${audio}/${occasion}/${name}.pcm"
    echo "${target}" >> "${work}/keep"
    if [[ ! -f "${target}" ]]; then
        mkdir -p "${audio}/${occasion}"
        source="${work}/${name}.wav"
        case "${ENGINE}" in
            volc) "${repo_root}/tools/volc-tts.py" "${VOICE}" "${text}" "${source}" ;;
            elevenlabs) "${repo_root}/tools/elevenlabs-tts.py" "${VOICE}" "${text}" "${source}" ;;
        esac
        ffmpeg -loglevel error -y -i "${source}" -ar 16000 -ac 1 -f s16le -acodec pcm_s16le "${work}/${name}.raw"
        local peak gain
        peak="$(ffmpeg -f s16le -ar 16000 -ac 1 -i "${work}/${name}.raw" -af volumedetect -f null - 2>&1 \
            | sed -n 's/.*max_volume: \(-*[0-9.]*\) dB.*/\1/p')"
        gain="$(python3 -c "print(f'{-1.0 - float(\"${peak}\"):.2f}')")"
        ffmpeg -loglevel error -y -f s16le -ar 16000 -ac 1 -i "${work}/${name}.raw" \
            -af "volume=${gain}dB" -f s16le -acodec pcm_s16le "${target}"
        printf '%s\n' "${text}" > "${target%.pcm}.txt"
        echo "synthesized ${occasion}: ${text}"
    fi
    local seconds
    seconds="$(python3 -c "import os; print(f'{os.path.getsize(\"${target}\") / 32000:.1f}')")"
    if python3 -c "import sys; sys.exit(0 if ${seconds} > 3.0 else 1)"; then
        echo "too long (${seconds} s) ${occasion}: ${text}" >&2
        too_long=1
    fi
}

while IFS=$'\t' read -r occasion text || [[ -n "${occasion}" ]]; do
    [[ -z "${occasion}" || "${occasion}" == \#* ]] && continue
    [[ -n "${text}" ]] || { echo "lines.tsv: no text for ${occasion}" >&2; exit 2; }
    synth "${occasion}" "${text}"
done < "${dir}/lines.tsv"

# Drop the audio of rows that were deleted or reworded.
find "${audio}" -name '*.pcm' | while read -r file; do
    grep -qxF "${file}" "${work}/keep" || rm -f "${file}" "${file%.pcm}.txt"
done
find "${audio}" -type d -empty -delete

"${repo_root}/tools/character_pack.py" "${id}" "${audio}" "${dir}/pack.bin"
if [[ "${too_long}" == 1 ]]; then
    echo "some lines are longer than 3 seconds; shorten them in lines.tsv" >&2
    exit 1
fi
