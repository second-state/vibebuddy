#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
activation_script="${HOME}/.espressif/tools/activate_idf_v5.5.3.sh"
caller_path="${PATH}"
while IFS='=' read -r key value; do
    if [[ "${key}" == "PATH" ]]; then
        export PATH="${value}:${caller_path}"
    elif [[ "${key}" != "SYSTEM_PATH" ]]; then
        export "${key}=${value}"
    fi
done < <("${activation_script}" -e)

# 此脚本只构建独立探针，不选择串口，也不写入任何设备。
"${IDF_PYTHON_ENV_PATH}/bin/python" "${IDF_PATH}/tools/idf.py" \
    -C "${repo_root}/firmware-probes/spi-screen" build
