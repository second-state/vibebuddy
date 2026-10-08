#!/usr/bin/env bash
set -euo pipefail

# 两种硬件的构建缓存与 sdkconfig 分开；此命令只编译，不烧录。
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
board="${1:-alientek}"
case "$board" in
  alientek|breadboard) ;;
  *) echo "用法: $0 [alientek|breadboard]" >&2; exit 1 ;;
esac
activation_script="${HOME}/.espressif/tools/activate_idf_v5.5.3.sh"
caller_path="$PATH"
while IFS='=' read -r key value; do
  if [[ "$key" == PATH ]]; then
    export PATH="${value}:${caller_path}"
  elif [[ "$key" != SYSTEM_PATH ]]; then
    export "${key}=${value}"
  fi
done < <("${activation_script}" -e)
build_dir="${repo_root}/firmware/build-${board}"
"${IDF_PYTHON_ENV_PATH}/bin/python" "${IDF_PATH}/tools/idf.py" \
  -C "${repo_root}/firmware" -B "$build_dir" \
  -DSDKCONFIG="${build_dir}/sdkconfig" -DVIBE_BOARD="$board" build
