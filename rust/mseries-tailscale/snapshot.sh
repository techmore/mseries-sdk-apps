#!/usr/bin/env bash
# Render the Tailscale window to PNGs (light and dark) under Xvfb from the fixture status, for review and PRs.
#   rust/mseries-tailscale/snapshot.sh OUT_DIR [FIXTURE_DIR]
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out=$(mkdir -p "$1" && cd "$1" && pwd)
fixtures=${2:-$here/fixtures}
(cd "$here/.." && cargo build -q -p mseries-tailscale)
bin="$(cd "$here/.." && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/mseries-tailscale"
# Icons come from a private data dir so the snapshot does not depend on what is installed.
cache=${XDG_CACHE_HOME:-$HOME/.cache}; mkdir -p "$cache"  # CI runners have no ~/.cache yet
data=$(mktemp -d "$cache/tailscale-snapshot.XXXXXX"); trap 'rm -rf "$data"' EXIT
for f in "$here"/data/*.svg; do
  case $f in *-symbolic.svg) dir=symbolic ;; *) dir=scalable ;; esac
  install -Dm644 "$f" "$data/icons/hicolor/$dir/apps/$(basename "$f")"
done
for scheme in light dark; do
  xvfb-run -a -s "-screen 0 1000x1700x24" env GSK_RENDERER=cairo GDK_BACKEND=x11 \
    XDG_DATA_DIRS="$data:/usr/local/share:/usr/share" \
    MSERIES_TAILSCALE_FIXTURES="$fixtures" ${SIZE:+MSERIES_TAILSCALE_SNAPSHOT_SIZE=$SIZE} MSERIES_TAILSCALE_DARK=$([[ $scheme == dark ]] && echo 1 || echo 0) \
    MSERIES_TAILSCALE_SNAPSHOT="$out/${PREFIX:-main}-$scheme.png" MSERIES_TAILSCALE_SNAPSHOT_DELAY=${DELAY:-2500} \
    dbus-run-session -- "$bin" 2>&1 | grep -viE "egl|mesa|dri3|dbus" || true
done
ls -la "$out"
