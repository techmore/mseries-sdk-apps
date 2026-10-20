#!/bin/bash
set -euo pipefail
umask 077
[ "$(uname -s)" = Darwin ] || { echo 'Run this on macOS.'; exit 1; }
revision=${1:-}
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || { echo 'Pass the full Git commit from the checklist.'; exit 1; }
destination="$HOME/Downloads/MSeries-Mac-Visit"
[ ! -e "$destination" ] || { echo "$destination already exists. Use that kit, or rename it first."; exit 1; }
mkdir "$destination"
cd "$destination"
base="https://raw.githubusercontent.com/techmore/ElementaryOS_MSeries/$revision/mseries/phone-setup/macos-visit"
for name in Save-Hardware-Code.command Prepare-Optional-VM.command README.md; do
  curl --fail --location --retry 3 "$base/$name" -o "$name"
done
curl --fail --location --retry 3 'https://github.com/OpenBubbles/Mac-Hardware-Info/releases/download/v1.2.0/Mac.Hardware.Info.zip' -o Mac.Hardware.Info.zip
printf '%s\n' '09db8d858d81a609c0a52e1b8a36625a68379a26e5e4e4d7c526ab64065c9986  Mac.Hardware.Info.zip' | shasum -a 256 -c -
unzip -q Mac.Hardware.Info.zip
chmod 700 ./*.command
printf '%s\n' 'Kit ready in Downloads/MSeries-Mac-Visit. Read README.md before rebooting.'
open .
