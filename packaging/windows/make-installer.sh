#!/usr/bin/env bash
# Builds the Windows installer, VibeBuddy-<label>-windows-x86_64-setup.exe: the three binaries, the voice packs and
# the licenses, staged as make-tarball.sh does on Linux, then wrapped by Inno Setup 6 (vibebuddy.iss). Run it in Git
# Bash on Windows, with Python and cargo-about on the PATH.
#
# Usage: packaging/windows/make-installer.sh <label>   (e.g. v0.7.0)
set -euo pipefail

label="${1:?usage: make-installer.sh <label>}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
here="${repo}/packaging/windows"
stage="${repo}/target/package/VibeBuddy-${label}-windows-x86_64"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "${repo}/Cargo.toml" | head -n 1)"
iscc="${ISCC:-/c/Program Files (x86)/Inno Setup 6/ISCC.exe}"
# Python on Windows reads and writes text in the ANSI code page unless told otherwise; the notices are UTF-8.
export PYTHONUTF8=1

cargo build --release --locked --manifest-path "${repo}/Cargo.toml" -p vibebuddyd -p vibebuddy-hook -p vibebuddy-desktop

rm -rf "${stage}"
mkdir -p "${stage}/voices"
for binary in vibebuddyd vibebuddy-hook vibebuddy-desktop; do
    cp "${repo}/target/release/${binary}.exe" "${stage}/"
done
# The app looks for the packs in voices\ beside itself (desktop/src/assets.rs).
python "${repo}/tools/make_voice_pack.py" "${repo}/voices/jessica" robot "${stage}/voices/robot.bin" >/dev/null
for pack in "${repo}"/characters/*/pack.bin; do
    id="$(basename "$(dirname "${pack}")")"
    cp "${pack}" "${stage}/voices/${id}.bin"
    for variant in "$(dirname "${pack}")"/address/*.bin; do
        [[ -f "${variant}" ]] && cp "${variant}" "${stage}/voices/${id}.$(basename "${variant}")"
    done
done
python "${repo}/tools/package-licenses.py" "${stage}" x86_64-pc-windows-msvc

# Inno Setup reads Windows paths.
"${iscc}" /Qp "/DAppVersion=${version}" "/DLabel=${label}" "/DStage=$(cygpath -w "${stage}")" "/O$(cygpath -w "${repo}")" "$(cygpath -w "${here}/vibebuddy.iss")"
echo "${repo}/VibeBuddy-${label}-windows-x86_64-setup.exe"
