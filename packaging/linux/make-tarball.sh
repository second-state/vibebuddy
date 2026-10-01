#!/usr/bin/env bash
# Builds the Linux release package, VibeBuddy-<label>-linux-x86_64.tar.gz: the three binaries, the voice packs,
# the firmware this release ships, the systemd unit, launcher entry and icon, the licenses, and install.sh, which
# installs from these files without building or downloading anything.
#
# Usage: packaging/linux/make-tarball.sh <label>   (e.g. v0.3.0)
# The firmware images must already be in firmware-rs/device/build (tools/build-firmware.sh, or CI's firmware job).
set -euo pipefail

label="${1:?usage: make-tarball.sh <label>}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
firmware="${repo}/firmware-rs/device/build"
name="VibeBuddy-${label}-linux-x86_64"
stage="${repo}/target/package/${name}"

for file in bootloader.bin partition-table.bin vibebuddy-fw.bin build.txt; do
    [[ -f "${firmware}/${file}" ]] || { echo "missing ${firmware}/${file}; build the firmware first" >&2; exit 1; }
done

cargo build --release --locked --manifest-path "${repo}/Cargo.toml" -p vibebuddyd -p vibebuddy-hook -p vibebuddy-desktop

rm -rf "${stage}"
mkdir -p "${stage}/bin" "${stage}/share/voices" "${stage}/share/firmware"
for binary in vibebuddyd vibebuddy-hook vibebuddy-desktop; do
    install -m755 "${repo}/target/release/${binary}" "${stage}/bin/${binary}"
done
for file in vibebuddyd.service vibebuddy.desktop vibebuddy.svg; do
    install -m644 "${repo}/packaging/linux/${file}" "${stage}/share/${file}"
done
for dir in "${repo}"/voices/*/; do
    if [[ -f "${dir}/done.pcm" ]]; then
        python3 "${repo}/tools/make_voice_pack.py" "${dir}" "$(basename "${dir}")" "${stage}/share/voices/$(basename "${dir}").bin" >/dev/null
    fi
done
for file in bootloader.bin partition-table.bin vibebuddy-fw.bin build.txt; do
    install -m644 "${firmware}/${file}" "${stage}/share/firmware/${file}"
done
# The script lays licenses out as the Mac bundle does, with a copy beside the firmware; here that is share/firmware,
# so the installed firmware carries them as the release's firmware zip does.
python3 "${repo}/tools/package-licenses.py" "${stage}/share" x86_64-unknown-linux-gnu
mv "${stage}/share/licenses" "${stage}/licenses"
install -m755 "${repo}/packaging/linux/install.sh" "${stage}/install.sh"

tar -C "$(dirname "${stage}")" -czf "${repo}/${name}.tar.gz" "${name}"
echo "${repo}/${name}.tar.gz"
