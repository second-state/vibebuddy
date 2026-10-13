#!/usr/bin/env bash
# Rebuild firmware-rs/device/certs/relay-roots.pem: the roots the box trusts for the Relay's TLS.
# The Relay sits on Cloudflare's Universal SSL, whose CA can be Let's Encrypt, Google Trust Services or
# SSL.com, with a backup certificate from Sectigo deployed on revocation (#110). Each CA's ECDSA and RSA
# roots are taken from macOS's system root store. Replacing the bundle needs a firmware release.
#
# Roots the system store doesn't have yet are fetched from their CA and checked against a pinned SHA-256
# fingerprint. Let's Encrypt's ISRG Root YE and YR are among them: today their chains are cross-signed to
# ISRG Root X2/X1, and a box that trusts them directly keeps the Relay when those cross-signs end (#138).
# Move a root to the system list once macOS ships it. GTS Root R2 and R3 are Google's reserve roots, there
# for the same reason.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="${repo_root}/firmware-rs/device/certs/relay-roots.pem"
store=/System/Library/Keychains/SystemRootCertificates.keychain
roots=(
    "GTS Root R1"
    "GTS Root R2"
    "GTS Root R3"
    "GTS Root R4"
    "ISRG Root X1"
    "ISRG Root X2"
    "SSL.com TLS ECC Root CA 2022"
    "SSL.com TLS RSA Root CA 2022"
    "SSL.com Root Certification Authority ECC"
    "SSL.com Root Certification Authority RSA"
    "Sectigo Public Server Authentication Root E46"
    "Sectigo Public Server Authentication Root R46"
    "USERTrust ECC Certification Authority"
    "USERTrust RSA Certification Authority"
)
# name|url|sha256 fingerprint
fetched=(
    "ISRG Root YE|https://letsencrypt.org/certs/gen-y/root-ye.pem|E14FFCAD5B0025731006CAA43A121A22D8E9700F4FB9CF852F02A708AA5D5666"
    "ISRG Root YR|https://letsencrypt.org/certs/gen-y/root-yr.pem|E57B7E6F150C419102E8D5C055729FF967B9D1A829BF00CEC89CA604EBF4A86F"
)

mkdir -p "$(dirname "${out}")"
: > "${out}"
for name in "${roots[@]}"; do
    pem="$(security find-certificate -a -c "${name}" -p "${store}")"
    if [[ "$(grep -c 'BEGIN CERTIFICATE' <<< "${pem}")" -ne 1 ]]; then
        echo "expected exactly one root named ${name}" >&2
        exit 1
    fi
    printf '# %s\n%s\n' "${name}" "${pem}" >> "${out}"
done
for entry in "${fetched[@]}"; do
    IFS='|' read -r name url fingerprint <<< "${entry}"
    pem="$(curl -fsSL "${url}")"
    actual="$(openssl x509 -noout -fingerprint -sha256 <<< "${pem}" | sed 's/.*=//; s/://g')"
    if [[ "${actual}" != "${fingerprint}" ]]; then
        echo "${name} from ${url} has fingerprint ${actual}, expected ${fingerprint}" >&2
        exit 1
    fi
    printf '# %s\n%s\n' "${name}" "${pem}" >> "${out}"
done
echo "$(( ${#roots[@]} + ${#fetched[@]} )) roots -> ${out}"
