#!/usr/bin/env bash
# Build and run the pomodoro state machine tests on the Mac; no ESP-IDF needed.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="$(mktemp -t pomodoro_test)"
trap 'rm -f "${output}"' EXIT

cc -std=c11 -Wall -Wextra -Werror \
    -I "${repo_root}/firmware/main" \
    "${repo_root}/firmware/main/agent_pomodoro.c" \
    "${repo_root}/firmware/host_tests/pomodoro_test.c" \
    -o "${output}"
"${output}"
