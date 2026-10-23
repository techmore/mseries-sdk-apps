#!/usr/bin/env bash
# Exercise real GTK widgets without the owner's display, services or settings.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for tool in cargo xvfb-run dbus-run-session xdotool python3; do
  command -v "$tool" >/dev/null || { echo "Missing test tool: $tool" >&2; exit 2; }
done
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/config" "$scratch/cache" "$scratch/data" "$scratch/runtime"
chmod 700 "$scratch/runtime"
cat >"$scratch/dbus.conf" <<'BUS'
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <policy context="default">
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
    <allow own="*"/>
  </policy>
</busconfig>
BUS
export XDG_CONFIG_HOME="$scratch/config" XDG_CACHE_HOME="$scratch/cache"
export XDG_DATA_HOME="$scratch/data" XDG_RUNTIME_DIR="$scratch/runtime"
export MSERIES_MESSAGES_ISOLATED_GUI=1 GSK_RENDERER=cairo GTK_A11Y=test
export GIO_USE_VFS=local GDK_BACKEND=x11
unset WAYLAND_DISPLAY DBUS_SESSION_BUS_ADDRESS
xvfb-run -a dbus-run-session --config-file="$scratch/dbus.conf" -- \
  cargo test --manifest-path "$here/../Cargo.toml" --locked -p mseries-messages \
    --lib offline_gui -- --ignored --test-threads=1
# Badge wire contracts need a bus but no display or owner dock.
dbus-run-session --config-file="$scratch/dbus.conf" -- \
  cargo test --manifest-path "$here/../Cargo.toml" --locked -p mseries-pantheon-kit \
    --lib dock_badge_private_bus -- --ignored --test-threads=1
