#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 2 ]]; then
  echo "Usage: package-service.sh SERVICE_BINARY OUTPUT_DIRECTORY" >&2
  exit 2
fi

[[ -f "$1" && -x "$1" ]] || { echo "Service binary is missing or not executable: $1" >&2; exit 1; }
mkdir -p "$2"
install -m 755 "$1" "$2/karabiner-service-macos-arm64"
(
  cd "$2"
  shasum -a 256 karabiner-service-macos-arm64 > karabiner-service-macos-arm64.sha256
)
echo "$2/karabiner-service-macos-arm64"
