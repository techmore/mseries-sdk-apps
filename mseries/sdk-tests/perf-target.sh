#!/usr/bin/env bash
# perf-target.sh [SECONDS]: startup time, RSS and idle CPU of SDK apps and the native T3 client on this host.
# Everything runs in a private Xvfb + D-Bus session with an empty HOME, offline, so no real desktop,
# server, credential or provider is touched. Needs release builds of msdk-run and claude-pantheon-{rs,sdk}
# (start-up time: `claude-pantheon-rs --quit-after-frame`). Needs release builds in ${CARGO_TARGET_DIR:-~/.cache/cargo-target}/release. Prints Markdown rows.
set -uo pipefail
secs=${1:-20}
here="$(cd "$(dirname "$0")/../.." && pwd)"
bin="${CARGO_TARGET_DIR:-$HOME/.cache/cargo-target}/release"
bench="$here/rust/claude-pantheon-rs/bench.sh"
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
echo "| App | processes | RSS avg MB | RSS peak MB | idle CPU % | note |"
echo "|---|---:|---:|---:|---:|---|"

measure() { # label command...
  local label=$1; shift
  mkdir -p "$work/h"
  local tag="perfapp-$$-$(echo "$label" | tr -c "A-Za-z0-9\n" _)"
  setsid env HOME="$work/h" XDG_CONFIG_HOME="$work/h/c" XDG_CACHE_HOME="$work/h/k" XDG_DATA_HOME="$work/h/d" XDG_STATE_HOME="$work/h/s" \
    CLAUDE_PANTHEON_OFFLINE=1 GSK_RENDERER=cairo GDK_BACKEND=x11 \
    xvfb-run -a dbus-run-session -- bash -c 'exec -a "$0" "$@"' "$tag" "$@" >/dev/null 2>&1 &
  local pid=$!
  # wait for the app process, then let it settle
  for _ in $(seq 1 100); do pgrep -f -- "$tag" >/dev/null && break; sleep 0.1; done
  sleep 4
  local row
  row=$("$bench" "^$tag" "$secs" "$label")
  echo "$row"
  # stop only what this function started: the session leader's whole process group
  kill -- "-$pid" 2>/dev/null; wait "$pid" 2>/dev/null
}

measure "native T3 client (claude-pantheon-rs)" "$bin/claude-pantheon-rs"
measure "SDK T3 client (claude-pantheon-sdk)" "$bin/claude-pantheon-sdk"
echo '{"calls":{}}' > "$work/fixture.json"
measure "SDK Home Storage (msdk-run)" "$bin/msdk-run" "$here/rust/msdk/examples/home-storage.ui.json" --fixture "$work/fixture.json"
