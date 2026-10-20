#!/bin/bash
# Read-only private-framework export. macOS already includes JXA/Foundation.
set -euo pipefail
umask 077
[ "$(uname -s)" = Darwin ] || { echo 'Run this on your macOS partition.' >&2; exit 1; }
here=$(cd -- "$(dirname -- "$0")" && pwd)
destination=${1:-"$HOME/Downloads/MSeries-Message-Pins"}
mkdir -p "$destination"
target="$destination/apple-message-pins.json"
receipt="$destination/MSeries-Message-Pins-COMPLETE.txt"
rm -f "$receipt"
trap 'printf "\nPIN EXPORT NOT COMPLETE. Previous exports are retained; resolve the reported error before returning to Linux.\n" >&2' ERR
if [ -e "$target" ]; then
  cp -p "$target" "$destination/apple-message-pins-before-$(date -u +%Y%m%dT%H%M%SZ).json"
fi
printf '%s\n' 'Open Messages on this Mac and wait until its pinned conversations match your iPhone.'
printf '%s\n' 'This helper reads those pins through IMCore. It does not send messages or change pins.'
printf '%s\n' 'Keep this small private export on storage accessible from Linux, including your NAS if mounted.'
open -a Messages
read -r -p 'When the Mac’s pins match your iPhone, press Return to export: ' _
/usr/bin/osascript -l JavaScript "$here/Export-Message-Pins.js" --output "$target"
(cd "$destination" && shasum -a 256 apple-message-pins.json > APPLE-PINS-SHA256SUMS)
(cd "$destination" && shasum -a 256 -c APPLE-PINS-SHA256SUMS)
printf 'Pin snapshot and checksum verified: %s\nSource: Apple Messages local pin configuration. Linux import remains to be confirmed.\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$receipt"
printf 'Saved private pin snapshot: %s\n' "$target"
printf '%s\n' 'Before returning to Linux, keep this folder on your private NAS or other Linux-accessible storage and verify APPLE-PINS-SHA256SUMS there.'
printf '%s\n' 'A snapshot left only in macOS Downloads has not been transferred.'
printf '%s\n' 'On Linux: Messages → menu → Import iPhone Pins. Check the conversation matches before importing.'
