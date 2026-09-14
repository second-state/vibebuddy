#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
firmware_dir="${repo_root}/firmware"
serial_port="${1:-}"

if command -v idf.py >/dev/null 2>&1; then
    idf_command=(idf.py)
else
    activation_script="${HOME}/.espressif/tools/activate_idf_v5.5.3.sh"
    if [[ ! -f "${activation_script}" ]]; then
        echo "ESP-IDF v5.5.3 activation script not found: ${activation_script}" >&2
        exit 1
    fi

    while IFS='=' read -r key value; do
        export "${key}=${value}"
    done < <("${activation_script}" -e)

    idf_command=("${IDF_PYTHON_ENV_PATH}/bin/python" "${IDF_PATH}/tools/idf.py")
fi

if [[ -z "${serial_port}" ]]; then
    shopt -s nullglob
    ports=(/dev/cu.usbmodem*)
    shopt -u nullglob
    if [[ ${#ports[@]} -ne 1 ]]; then
        echo "Expected exactly one /dev/cu.usbmodem* device; pass the port explicitly." >&2
        exit 1
    fi
    serial_port="${ports[0]}"
fi

if [[ ! -c "${serial_port}" ]]; then
    echo "Serial port is not a character device: ${serial_port}" >&2
    exit 1
fi

"${idf_command[@]}" -C "${firmware_dir}" -p "${serial_port}" build flash
