#!/usr/bin/env bash
# Render the Messages window with fixture conversations, light and dark, under Xvfb.
# Usage: screenshots.sh [output-dir]   (default: build/messages-screenshots)
# Uses the elementary stylesheet when installed, Adwaita otherwise.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="$(mkdir -p "${1:-$here/../../build/messages-screenshots}" && cd "${1:-$here/../../build/messages-screenshots}" && pwd)"
for tool in cargo xvfb-run dbus-run-session; do
  command -v "$tool" >/dev/null || { echo "Missing tool: $tool" >&2; exit 2; }
done
theme=Adwaita
[ -d /usr/share/themes/io.elementary.stylesheet.blueberry ] && theme=io.elementary.stylesheet.blueberry
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/config" "$scratch/cache" "$scratch/data" "$scratch/runtime"
chmod 700 "$scratch/runtime"
export XDG_CONFIG_HOME="$scratch/config" XDG_CACHE_HOME="$scratch/cache"
export XDG_DATA_HOME="$scratch/data" XDG_RUNTIME_DIR="$scratch/runtime"
export GSK_RENDERER=cairo GDK_BACKEND=x11 GIO_USE_VFS=local
export SIGNAL_CLI_PATH="$here/tests/fake-signal-cli.py" FAKE_SIGNAL_LOG="$scratch/signal.log"
export MSERIES_SIGNAL_CLIENT_NAME=no-such-signal-client MSERIES_SHOT_DIR="$out"
unset WAYLAND_DISPLAY DBUS_SESSION_BUS_ADDRESS
for variant in light dark; do
  gtk_theme="$theme"
  [ "$variant" = dark ] && gtk_theme="$theme:dark"
  GTK_THEME="$gtk_theme" MSERIES_SHOT_PREFIX="$variant-" \
    xvfb-run -a -s "-screen 0 1280x900x24" dbus-run-session -- \
    cargo test --manifest-path "$here/../Cargo.toml" --locked -p mseries-messages \
      --lib screenshot_fixture -- --ignored --test-threads=1
done
ls -1 "$out"
