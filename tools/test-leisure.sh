#!/usr/bin/env bash
# 在 Mac 上编译并运行休闲导演的测试；不需要 ESP-IDF。
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
