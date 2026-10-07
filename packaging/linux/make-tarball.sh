#!/usr/bin/env bash
# Builds the Linux release package, VibeBuddy-<label>-linux-x86_64.tar.gz: the three binaries, the voice packs,
# the systemd unit, launcher entry and icon, the licenses, and install.sh, which installs from these files without
# building or downloading anything. Firmware isn't in it: it is released on its own and the daemon downloads it.
#
# Usage: packaging/linux/make-tarball.sh <label>   (e.g. v0.3.0)
set -euo pipefail

label="${1:?usage: make-tarball.sh <label>}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
name="VibeBuddy-${label}-linux-x86_64"
stage="${repo}/target/package/${name}"

cargo build --release --locked --manifest-path "${repo}/Cargo.toml" -p vibebuddyd -p vibebuddy-hook -p vibebuddy-desktop

rm -rf "${stage}"
mkdir -p "${stage}/bin" "${stage}/share/voices"
for binary in vibebuddyd vibebuddy-hook vibebuddy-desktop; do
    install -m755 "${repo}/target/release/${binary}" "${stage}/bin/${binary}"
done
for file in vibebuddyd.service vibebuddy.desktop vibebuddy.svg 70-vibebuddy.rules; do
    install -m644 "${repo}/packaging/linux/${file}" "${stage}/share/${file}"
done
for pack in "${repo}"/characters/*/pack.bin; do
    id="$(basename "$(dirname "${pack}")")"
    cp "${pack}" "${stage}/share/voices/${id}.bin"
    # Its lines said with each form of address; the app swaps in the one picked.
    for variant in "$(dirname "${pack}")"/address/*.bin; do
        [[ -f "${variant}" ]] && cp "${variant}" "${stage}/share/voices/${id}.$(basename "${variant}")"
    done
done
python3 "${repo}/tools/package-licenses.py" "${stage}/share" x86_64-unknown-linux-gnu
mv "${stage}/share/licenses" "${stage}/licenses"
install -m755 "${repo}/packaging/linux/install.sh" "${stage}/install.sh"

tar -C "$(dirname "${stage}")" -czf "${repo}/${name}.tar.gz" "${name}"
echo "${repo}/${name}.tar.gz"
