#!/usr/bin/env bash
# Build and run the leisure director tests on the Mac; no ESP-IDF needed.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="$(mktemp -t leisure_test)"
trap 'rm -f "${output}"' EXIT

cc -std=c11 -Wall -Wextra -Werror \
    -I "${repo_root}/firmware/main" \
    "${repo_root}/firmware/main/agent_leisure.c" \
    "${repo_root}/firmware/host_tests/leisure_test.c" \
    -o "${output}"
"${output}"
