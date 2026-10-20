#!/bin/bash
set -euo pipefail
umask 077
cd -- "$(dirname -- "$0")"
if [ "$(uname -s)" != Darwin ]; then
  echo 'Run this helper on your native macOS partition.'; exit 1
fi
printf '%s\n' 'In Mac Hardware Info, turn OFF Prevent sharing, then click Copy Activation Code.'
read -r -p 'Once the full code is copied, press Return here. ' _
tmp=$(mktemp ./hardware-code.XXXXXX)
trap 'rm -f "$tmp"' EXIT
pbpaste > "$tmp"
magic=$(base64 -D < "$tmp" 2>/dev/null | head -c 5 | od -An -tx1 | tr -d ' \n' || true)
if [ "$magic" != 4f41425300 ]; then
  echo 'Clipboard is not a full unshared OABS hardware blob. Turn OFF Prevent sharing and copy again.'
  exit 1
fi
if [ -e mac-hardware.txt ]; then
  echo 'mac-hardware.txt already exists; move it aside if you intend to replace it.'; exit 1
fi
mv "$tmp" mac-hardware.txt
chmod 600 mac-hardware.txt
printf '%s\n' 'Saved mac-hardware.txt without displaying its contents. Keep this private.'
firmware=/System/Library/Frameworks/Virtualization.framework/Resources/AVPBooter.vmapple2.bin
if [ -r "$firmware" ] && [ ! -e AVPBooter.vmapple2.bin ]; then
  cp "$firmware" AVPBooter.vmapple2.bin
  chmod 600 AVPBooter.vmapple2.bin
  shasum -a 256 AVPBooter.vmapple2.bin > AVPBooter.sha256
  echo 'Also copied this macOS installation AVPBooter for the separate VM experiment.'
fi
read -r -p 'Press Return to close. ' _
