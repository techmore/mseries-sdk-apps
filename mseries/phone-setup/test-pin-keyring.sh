#!/bin/bash
set -euo pipefail
here=$(cd -- "$(dirname -- "$0")" && pwd)
private=$(mktemp -d)
trap 'rm -rf -- "$private"' EXIT
mkdir -p "$private/home" "$private/config" "$private/data" "$private/cache" "$private/run"
chmod 700 "$private/run"
env HOME="$private/home" XDG_CONFIG_HOME="$private/config" XDG_DATA_HOME="$private/data" \
  XDG_CACHE_HOME="$private/cache" XDG_RUNTIME_DIR="$private/run" MSERIES_PRIVATE_PIN_TEST=1 \
  dbus-run-session --config-file="$here/test-bus.conf" -- bash -c '
    eval "$(printf %s fixture-only-password | gnome-keyring-daemon --unlock --components=secrets)"
    # GNOME Keyring 50.0 does not register a login collection on creation.
    # Reload the isolated daemon after its first creation so D-Bus sees it.
    eval "$(printf %s fixture-only-password | gnome-keyring-daemon --replace --unlock --components=secrets)"
    exec python3 "$1"
  ' bash "$here/test_pin_keyring.py"
