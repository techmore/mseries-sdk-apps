#!/bin/bash
set -euo pipefail
umask 077
cd -- "$(dirname -- "$0")"
# This optional helper provisions a separate VM; it never changes boot settings.
[ "$(uname -s)" = Darwin ] || { echo 'Run on native macOS.'; exit 1; }
export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
if ! command -v macosvm >/dev/null; then
  echo 'Install the native macosvm tool first: brew install macosvm'
  echo 'Official project: https://github.com/s-u/macosvm'
  read -r -p 'Press Return to close. ' _
  exit 1
fi
printf '%s\n' 'Optional Machine Lab guest preparation (experimental).'
printf '%s\n' 'Use the Ventura 13.6 (22G120) UniversalMac restore IPSW from Apple.'
printf '%s\n' 'Choose a local APFS/exFAT folder with at least 80 GiB available; FAT32 cannot store these files.'
read -r -p 'Full path to the downloaded IPSW: ' ipsw
read -r -p 'Full path to a NEW guest output folder: ' guest
[ -f "$ipsw" ] || { echo 'IPSW not found.'; exit 1; }
[ "${guest#/}" != "$guest" ] || { echo 'Use an absolute output path.'; exit 1; }
[ ! -e "$guest" ] || { echo 'Output folder already exists; refusing to overwrite it.'; exit 1; }
actual=$(shasum -a 1 "$ipsw" | awk '{print $1}')
[ "$actual" = a1675f2c8412122a5e796981571b0269a966708e ] || { echo 'IPSW does not match the pinned Ventura 13.6 image.'; exit 1; }
parent=$(dirname -- "$guest")
[ -d "$parent" ] || { echo 'Output parent folder does not exist.'; exit 1; }
free_kib=$(df -Pk "$parent" | awk 'END {print $4}')
[ "$free_kib" -ge 83886080 ] || { echo 'Need at least 80 GiB free on the output filesystem.'; exit 1; }
mkdir "$guest"
firmware=/System/Library/Frameworks/Virtualization.framework/Resources/AVPBooter.vmapple2.bin
[ -r "$firmware" ] || { echo 'AVPBooter not found in this macOS install.'; exit 1; }
cp "$firmware" "$guest/AVPBooter.vmapple2.bin"
shasum -a 256 "$guest/AVPBooter.vmapple2.bin" > "$guest/AVPBooter.sha256"
macosvm --disk "$guest/disk.img,size=64g" --aux "$guest/aux.img" \
  --restore "$ipsw" --net nat --mac 52:54:00:76:61:70 -c 4 -r 8g "$guest/vm.json"
dd if="$guest/aux.img" of="$guest/aux.img.trimmed" bs=16384 skip=1
printf '%s\n' 'Bundle created. Keep disk.img, aux.img, aux.img.trimmed, vm.json and firmware together.'
printf '%s\n' 'The installed host booter may differ from the upstream proof. We will check it on Linux.'
printf '%s\n' 'This does not prove Linux boot or iMessage activation. Never use this VM identity for Bubbles.'
read -r -p 'Press Return to close. ' _
