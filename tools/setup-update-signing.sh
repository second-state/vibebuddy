#!/usr/bin/env bash
# Sets up the key that signs the update manifest (ADR-0010):
#   - the private key lives in ~/.vibebuddy-signing/manifest.key (back it up) and in the MANIFEST_SIGNING_KEY secret
#   - the public key is written to manifest/manifest-key.pub, which the daemon trusts; commit it
# Rerunning reuses the existing key. Making a new one means every shipped daemon stops trusting the manifest, so it
# is never done silently: delete the old key by hand first.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="${HOME}/.vibebuddy-signing"
private="${work}/manifest.key"
public="${work}/manifest.pub"
mkdir -p "${work}"
chmod 700 "${work}"

cd "${repo_root}"
if [[ -f "${private}" ]]; then
    echo "Reusing ${private}"
    [[ -f "${public}" ]] || { echo "${public} is missing; restore it from your backup" >&2; exit 1; }
else
    cargo run -q -p vibebuddy-manifest -- keygen "${private}" > "${public}"
    echo "New key in ${private}; back up ${work}"
fi

cp "${public}" manifest/manifest-key.pub
echo "Public key: $(cat manifest/manifest-key.pub) (written to manifest/manifest-key.pub; commit it)"

if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    gh secret set MANIFEST_SIGNING_KEY < "${private}"
    echo "Set the MANIFEST_SIGNING_KEY secret"
else
    echo "gh isn't signed in; set the MANIFEST_SIGNING_KEY secret to the contents of ${private} by hand" >&2
fi
