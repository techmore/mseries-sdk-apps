#!/usr/bin/env bash
# Render Apple Dev's pages to PNGs (light and dark) under Xvfb with the fixture project and a
# fixture iPhone, for visual review and pull requests. Uses the debug build.
#   rust/mseries-apple-dev/snapshot.sh OUT_DIR
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out=$(mkdir -p "$1" && cd "$1" && pwd)
(cd "$here/.." && cargo build -q -p mseries-apple-dev)
bin="$(cd "$here/.." && cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/mseries-apple-dev"
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
for page in toolchain devices project; do
  for scheme in light dark; do
    selected=$page; [[ $page == project ]] && selected="project:$here/fixtures/HelloOmarchy"
    printf '{"projects":["%s"],"selected":"%s","width":1180,"height":800}\n' "$here/fixtures/HelloOmarchy" "$selected" >"$work/settings.json"
    xvfb-run -a -s "-screen 0 1400x960x24" env GSK_RENDERER=cairo GDK_BACKEND=x11 \
      MSERIES_APPLE_DEV_CONFIG="$work/settings.json" MSERIES_APPLE_DEV_FIXTURE_DEVICES="$here/fixtures/devices.json" \
      MSERIES_APPLE_DEV_DARK=$([[ $scheme == dark ]] && echo 1 || echo 0) \
      MSERIES_APPLE_DEV_SNAPSHOT="$out/$page-$scheme.png" MSERIES_APPLE_DEV_SNAPSHOT_DELAY=${DELAY:-4000} \
      dbus-run-session -- "$bin" 2>&1 | grep -viE "egl|mesa|dri3|dbus" || true
  done
done
ls -la "$out"
