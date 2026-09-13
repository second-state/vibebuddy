#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 || ! "$1" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "usage: $0 <snapshot-name>" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_dir="$repo_root/.probe/$1"

if [[ -e "$output_dir" ]]; then
  echo "refusing to overwrite existing snapshot: $output_dir" >&2
  exit 1
fi

mkdir -p "$output_dir"

{
  echo "captured_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "hostname=$(hostname)"
  echo "macos_version=$(sw_vers -productVersion)"
  echo "macos_build=$(sw_vers -buildVersion)"
  echo "architecture=$(uname -m)"
} >"$output_dir/metadata.txt"

command_status="$output_dir/command-status.txt"

if system_profiler SPUSBDataType >"$output_dir/system-profiler-usb.txt" 2>"$output_dir/system-profiler-usb.stderr.txt"; then
  echo "system_profiler=0" >>"$command_status"
else
  echo "system_profiler=$?" >>"$command_status"
fi

if ioreg -p IOUSB -l -w 0 >"$output_dir/ioreg-usb.txt" 2>"$output_dir/ioreg-usb.stderr.txt"; then
  echo "ioreg=0" >>"$command_status"
else
  echo "ioreg=$?" >>"$command_status"
fi

if ls /dev/cu.* >"$output_dir/cu-devices.txt" 2>"$output_dir/cu-devices.stderr.txt"; then
  echo "cu_devices=0" >>"$command_status"
else
  echo "cu_devices=$?" >>"$command_status"
fi

if ls /dev/tty.* >"$output_dir/tty-devices.txt" 2>"$output_dir/tty-devices.stderr.txt"; then
  echo "tty_devices=0" >>"$command_status"
else
  echo "tty_devices=$?" >>"$command_status"
fi

echo "saved USB snapshot: $output_dir"
