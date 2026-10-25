#!/usr/bin/env bash
# Render actual SDK status rows using inert tools and a private HOME/config.
# Usage: snapshot-sdk.sh APP_BINARY OUTPUT_DIRECTORY
# Reuses a compiled app; never installs a real SDK or contacts Apple.
set -euo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
bin=$(realpath "${1:?app binary required}")
mkdir -p "${2:?output directory required}"
out=$(realpath "$2")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/home" "$work/config/swiftpm/swift-sdks/darwin.artifactbundle" "$work/bin" "$work/toolkit"
printf 'XTOOL_SDK_VERSION="epoch=4,darwinTools=1.1.0,oam=1.4.1"\n' >"$work/toolkit/install-toolchain.sh"
: >"$work/toolkit/device-run.sh"
cat >"$work/bin/swift" <<'STUB'
#!/bin/sh
case "$1" in
  --version) echo 'Swift version 6.4.1' ;;
  sdk) echo darwin ;;
  *) exit 1 ;;
esac
STUB
cat >"$work/bin/xtool" <<'STUB'
#!/bin/sh
[ "$1" = --version ] || exit 1
echo 'xtool 1.23.0'
STUB
chmod +x "$work/bin/swift" "$work/bin/xtool"
printf 'export PATH=%q:$PATH\n' "$work/bin" >"$work/home/.bash_profile"
for state in current stale; do
  stamp='epoch=4,darwinTools=1.1.0,oam=1.4.1'
  [[ $state != stale ]] || stamp='epoch=3,darwinTools=1.0.0,oam=1.3.0'
  printf '%s\n' "$stamp" >"$work/config/swiftpm/swift-sdks/darwin.artifactbundle/darwin-sdk-version.txt"
  for scheme in light dark; do
    for width in 780 1180; do
      python3 - "$work/settings.json" "$work/toolkit" "$width" <<'PY'
import json, sys
with open(sys.argv[1], 'w') as out:
    json.dump(dict(toolkit_path=sys.argv[2], selected='toolchain', width=int(sys.argv[3]), height=900), out)
PY
      image="$out/sdk-$state-$scheme-$width.png"
      xvfb-run -a -s '-screen 0 1400x1100x24' env \
        HOME="$work/home" XDG_CONFIG_HOME="$work/config" XDG_DATA_HOME="$work/data" \
        PATH="$work/bin:$PATH" GSK_RENDERER=cairo GDK_BACKEND=x11 \
        MSERIES_APPLE_DEV_CONFIG="$work/settings.json" \
        MSERIES_APPLE_DEV_FIXTURE_DEVICES="$here/fixtures/devices.json" \
        MSERIES_APPLE_DEV_DARK=$([[ $scheme == dark ]] && echo 1 || echo 0) \
        MSERIES_APPLE_DEV_SNAPSHOT="$image" MSERIES_APPLE_DEV_SNAPSHOT_DELAY=5000 \
        dbus-run-session -- "$bin" >"$image.log" 2>&1
      test -s "$image"
    done
  done
done
