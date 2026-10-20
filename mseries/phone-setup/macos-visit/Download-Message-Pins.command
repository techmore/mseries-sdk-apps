#!/bin/bash
# Download the small, revision-pinned pin export kit. No VM payloads are fetched.
set -euo pipefail
umask 077
[ "$(uname -s)" = Darwin ] || { echo 'Run this on your macOS partition.' >&2; exit 1; }
revision=${1:-}
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || { echo 'Pass the full Git revision from the checklist.' >&2; exit 1; }
destination="$HOME/Downloads/MSeries-Message-Pins-Tools"
if [ -e "$destination" ] && [ ! -f "$destination/.mseries-pin-kit" ]; then
  echo 'An unrelated folder already exists at the download destination. Rename it before continuing.' >&2
  exit 1
fi
mkdir -p "$destination"
printf '%s\n' "$revision" > "$destination/.mseries-pin-kit"
base="https://raw.githubusercontent.com/techmore/ElementaryOS_MSeries/$revision/mseries/phone-setup/macos-visit"
for name in Export-Message-Pins.js Export-Message-Pins.command; do
  curl --fail --location --retry 3 "$base/$name" -o "$destination/$name.partial"
  mv "$destination/$name.partial" "$destination/$name"
done
exec /bin/bash "$destination/Export-Message-Pins.command" "${2:-$HOME/Downloads/MSeries-Message-Pins}"
