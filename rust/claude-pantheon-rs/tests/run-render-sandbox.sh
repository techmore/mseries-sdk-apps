#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
source_home="$HOME"
rustup_home="${RUSTUP_HOME:-$source_home/.rustup}"
cargo_home="${CARGO_HOME:-$source_home/.cargo}"

sandbox_root="$(mktemp -d "${TMPDIR:-/tmp}/claude-pantheon-render.XXXXXX")"
trap 'rm -rf -- "$sandbox_root"' EXIT
mkdir -p "$sandbox_root/home" "$sandbox_root/runtime" "$sandbox_root/config" \
  "$sandbox_root/cache" "$sandbox_root/data" "$sandbox_root/state"
chmod 700 "$sandbox_root/runtime"
render_args=()
if [[ ${CLAUDE_PANTHEON_RENDER_RELEASE:-0} == 1 ]]; then render_args+=(--release); fi

HOME="$sandbox_root/home" \
RUSTUP_HOME="$rustup_home" \
CARGO_HOME="$cargo_home" \
XDG_RUNTIME_DIR="$sandbox_root/runtime" \
XDG_CONFIG_HOME="$sandbox_root/config" \
XDG_CACHE_HOME="$sandbox_root/cache" \
XDG_DATA_HOME="$sandbox_root/data" \
XDG_STATE_HOME="$sandbox_root/state" \
dbus-run-session -- env \
  -u WAYLAND_DISPLAY \
  -u CLAUDE_PANTHEON_LIVE_RENDER \
  -u CLAUDE_PANTHEON_LIVE_SEND \
  -u CLAUDE_PANTHEON_LIVE_ROTATE \
  -u CLAUDE_PANTHEON_TEST_MICROPHONE \
  GDK_BACKEND=x11 \
  GSETTINGS_BACKEND=memory \
  GSK_RENDERER=cairo \
  CLAUDE_PANTHEON_RENDER_TEST_SANDBOX=1 \
  xvfb-run -a -s "-screen 0 1400x900x24" \
  cargo test "${render_args[@]}" --locked --manifest-path rust/Cargo.toml -p claude-pantheon-rs --test render
