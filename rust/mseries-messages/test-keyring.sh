#!/usr/bin/env bash
# Cross-client pin persistence on an isolated bus/home/keyring, never the owner bus.
set -euo pipefail
here=$(cd -- "$(dirname -- "$0")" && pwd)
repo=$(cd -- "$here/../.." && pwd)
for tool in cargo dbus-run-session gnome-keyring-daemon secret-tool python3 xvfb-run; do
  command -v "$tool" >/dev/null || { echo "Missing test tool: $tool" >&2; exit 2; }
done
private=$(mktemp -d)
trap 'rm -rf -- "$private"' EXIT
mkdir -p "$private/home" "$private/config" "$private/data" "$private/cache" "$private/run"
chmod 700 "$private/run"
env HOME="$private/home" XDG_CONFIG_HOME="$private/config" XDG_DATA_HOME="$private/data" \
  XDG_CACHE_HOME="$private/cache" XDG_RUNTIME_DIR="$private/run" \
  CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
  MSERIES_PRIVATE_PIN_TEST=1 MSERIES_MESSAGES_PYTHON_SOURCE="$repo/mseries/phone-setup" \
  dbus-run-session --config-file="$repo/mseries/phone-setup/test-bus.conf" -- bash -c '
    set -euo pipefail
    eval "$(printf %s fixture-only-password | gnome-keyring-daemon --unlock --components=secrets)"
    eval "$(printf %s fixture-only-password | gnome-keyring-daemon --replace --unlock --components=secrets)"
    cargo test --manifest-path "$1/rust/Cargo.toml" --locked -p mseries-pantheon-kit \
      --lib phone_pin_private_keyring -- --ignored --test-threads=1
    export MSERIES_MESSAGES_ISOLATED_GUI=1 GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none GIO_USE_VFS=local
    unset WAYLAND_DISPLAY
    exec xvfb-run -a cargo test --manifest-path "$1/rust/Cargo.toml" --locked -p mseries-messages \
      --lib favorites_private_keyring_gui -- --ignored --test-threads=1
  ' bash "$repo"
