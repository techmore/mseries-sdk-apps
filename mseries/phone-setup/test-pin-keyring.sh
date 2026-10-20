#!/bin/bash
set -euo pipefail
here=$(cd -- "$(dirname -- "$0")" && pwd)
private=$(mktemp -d)
trap 'rm -rf -- "$private"' EXIT
mkdir -p "$private/home" "$private/config" "$private/data" "$private/cache" "$private/run"
chmod 700 "$private/run"
env HOME="$private/home" XDG_CONFIG_HOME="$private/config" XDG_DATA_HOME="$private/data" \
  XDG_CACHE_HOME="$private/cache" XDG_RUNTIME_DIR="$private/run" MSERIES_PRIVATE_PIN_TEST=1 \
  dbus-run-session --config-file="$here/test-bus.conf" -- python3 "$here/test_pin_keyring.py"
