#!/usr/bin/env bash
# Sets up the two keys that sign updates (ADR-0010):
#   - the manifest key signs the update manifest, which the daemon trusts for firmware: private half in
#     ~/.vibebuddy-signing/manifest.key and the MANIFEST_SIGNING_KEY secret, public half in manifest/manifest-key.pub
#   - the Sparkle key signs the macOS App's DMG, which Sparkle checks before it swaps the bundle: private half in
#     ~/.vibebuddy-signing/sparkle.key and the SPARKLE_SIGNING_KEY secret, public half in app/sparkle-key.pub
# Commit both .pub files. Back up ~/.vibebuddy-signing.
# Rerunning reuses existing keys. Making a new one means every shipped copy stops trusting what it signs, so it is
# never done silently: delete the old key by hand first.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="${HOME}/.vibebuddy-signing"
mkdir -p "${work}"
chmod 700 "${work}"
cd "${repo_root}"

# setup <name> <public key in the repo> <secret>
setup() {
    local private="${work}/$1.key" public="${work}/$1.pub"
    if [[ -f "${private}" ]]; then
        echo "Reusing ${private}"
        [[ -f "${public}" ]] || { echo "${public} is missing; restore it from your backup" >&2; exit 1; }
    else
        cargo run -q -p vibebuddy-manifest -- keygen "${private}" > "${public}"
        echo "New key in ${private}; back up ${work}"
    fi
    cp "${public}" "$2"
    echo "Public key: $(cat "$2") (written to $2; commit it)"
    if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
        gh secret set "$3" < "${private}"
        echo "Set the $3 secret"
    else
        echo "gh isn't signed in; set the $3 secret to the contents of ${private} by hand" >&2
    fi
}

setup manifest manifest/manifest-key.pub MANIFEST_SIGNING_KEY
setup sparkle app/sparkle-key.pub SPARKLE_SIGNING_KEY
