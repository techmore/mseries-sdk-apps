#!/bin/bash
# Original MSeries collection wrapper. Apple payloads stay private and outside Git.
set -euo pipefail
umask 077
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:$PATH"
fail() { printf '\nNOT COMPLETE: %s\n' "$*" >&2; exit 1; }
trap 'printf "\nCollection interrupted or failed. Do not treat this as a completed transfer. Re-run the same script to resume.\n" >&2' ERR
[ "$(uname -s)" = Darwin ] || fail 'Run on your real macOS partition.'
[ "$(uname -m)" = arm64 ] || fail 'Use a native Apple Silicon Terminal (not Rosetta).'
version=$(sw_vers -productVersion)
[ "${version%%.*}" -ge 13 ] || fail 'Ventura provisioning requires macOS 13 or newer.'
stage="$HOME/Library/Application Support/MSeriesMacVisit"
mkdir -p "$stage"
# Keep the computer awake for downloads, restore and transfer, but never reboot it.
caffeinate -i -w $$ &
printf '%s\n' 'MSeries: collect Bubbles activation + Machine Lab guest in one visit.'
printf '%s\n' 'This can take hours. Keep AC power connected. No Apple Account login is needed.'
printf '%s\n' 'Choose an EXISTING folder on shared exFAT storage (internal or external, 100 GiB free). No partition is created or formatted.'
transfer=$(osascript -e 'POSIX path of (choose folder with prompt "Choose a shared exFAT folder (internal or external) for the private Linux return bundle")')
transfer=${transfer%/}
[ -d "$transfer" ] && [ -w "$transfer" ] || fail 'Transfer folder is not writable.'
device=$(df -P "$transfer" | awk 'END {print $1}')
diskutil info -plist "$device" > "$stage/transfer-volume.plist"
fs=$(/usr/libexec/PlistBuddy -c 'Print :FilesystemType' "$stage/transfer-volume.plist")
[ "$fs" = exfat ] || [ "$fs" = ExFAT ] || fail 'Choose exFAT: Linux cannot read this Mac APFS installation, and FAT32 cannot hold the guest.'
# Existing internal exFAT partitions are valid too. APFS and FAT32 remain rejected.
# Partition creation/resizing is deliberately outside this collector.
bundle="$transfer/MSeries-Linux-Return"
if [ -e "$bundle" ] && [ ! -f "$bundle/.mseries-collector-owner" ]; then
  fail 'An unrelated MSeries-Linux-Return folder exists. Rename it before continuing.'
fi
mkdir -p "$bundle"
printf '%s\n' 'MSeries macOS collector v1' > "$bundle/.mseries-collector-owner"
rm -f "$bundle/COLLECTION-COMPLETE.txt"
free_kib() { df -Pk "$1" | awk 'END {print $4}'; }
# On retries account for files already allocated by this collector.
used_stage=$(du -sk "$stage" | awk '{print $1}')
used_bundle=0
if [ -d "$bundle" ]; then used_bundle=$(du -sk "$bundle" | awk '{print $1}'); fi
[ $(( $(free_kib "$stage") + used_stage )) -ge 104857600 ] || fail 'Need 100 GiB available on native macOS, including this staging directory.'
[ $(( $(free_kib "$transfer") + used_bundle )) -ge 104857600 ] || fail 'Need 100 GiB available on the return drive, including this bundle.'
fetch() {
  local url=$1 target=$2
  if [ ! -f "$target" ]; then
    curl --fail --location --retry 3 -C - "$url" -o "$target.partial"
    mv "$target.partial" "$target"
  fi
}
check_sha256() { printf '%s  %s\n' "$1" "$2" | shasum -a 256 -c - >/dev/null; }
valid_hardware() {
  local magic
  magic=$(base64 -D < "$1" 2>/dev/null | head -c 5 | od -An -tx1 | tr -d ' \n' || true)
  [ "$magic" = 4f41425300 ] && base64 -D < "$1" >/dev/null 2>&1 && [ "$(wc -c < "$1")" -gt 100 ]
}
if [ ! "$0" -ef "$stage/Collect-Everything.command" ]; then
  cp "$0" "$stage/Collect-Everything.command"
fi
cd "$stage"
printf '\n1/6 Hardware export\n'
fetch 'https://github.com/OpenBubbles/Mac-Hardware-Info/releases/download/v1.2.0/Mac.Hardware.Info.zip' "$stage/Mac.Hardware.Info.zip"
check_sha256 09db8d858d81a609c0a52e1b8a36625a68379a26e5e4e4d7c526ab64065c9986 "$stage/Mac.Hardware.Info.zip" || fail 'Mac helper download checksum mismatch.'
if [ ! -d "$stage/hardware-tool" ]; then
  mkdir "$stage/hardware-tool"
  unzip -q "$stage/Mac.Hardware.Info.zip" -d "$stage/hardware-tool"
fi
if [ ! -s "$stage/mac-hardware.txt" ]; then
  app=$(find "$stage/hardware-tool" -name '*.app' -type d -prune -print | sed -n '1p')
  [ -n "$app" ] || fail 'Downloaded Mac Hardware Info app not found.'
  open "$app"
  printf '%s\n' 'In Mac Hardware Info: turn OFF Prevent sharing, then Copy Activation Code.'
  printf '%s\n' 'If blocked, use Privacy & Security → Open Anyway for this official app.'
  read -r -p 'After copying the full code, press Return here: ' _
  pbpaste > "$stage/hardware-code.pending"
  valid_hardware "$stage/hardware-code.pending" || fail 'Clipboard is not a complete base64 OABS export. Copy again and rerun.'
  mv "$stage/hardware-code.pending" "$stage/mac-hardware.txt"
fi
valid_hardware "$stage/mac-hardware.txt" || fail 'Saved hardware export is invalid. Preserve it and investigate before continuing.'
printf '\n2/6 Native firmware and provenance\n'
mkdir -p "$stage/firmware"
firmware=/System/Library/Frameworks/Virtualization.framework/Resources/AVPBooter.vmapple2.bin
[ -s "$firmware" ] || fail 'Native AVPBooter.vmapple2.bin is missing.'
if [ ! -f "$stage/firmware/AVPBooter.vmapple2.bin" ]; then
  cp "$firmware" "$stage/firmware/AVPBooter.vmapple2.bin"
fi
sw_vers > "$stage/HOST-VERSION.txt"
firmware_hash=$(shasum -a 256 "$stage/firmware/AVPBooter.vmapple2.bin" | awk '{print $1}')
cat > "$stage/FIRMWARE-COMPATIBILITY.txt" <<TXT
Collected native booter SHA-256: $firmware_hash
Published proof booter SHA-256: 513ca3fbb2cd5accb0a6da96a5e476d593ebae403d7edb33aebca88f505dac61
A different hash is not a proven failure, but compatibility must be tested on Linux.
Native startup uses Apple Virtualization.framework, not this extracted file in QEMU.
TXT
printf '\n3/6 Pinned macosvm and Ventura restore download\n'
fetch 'https://github.com/s-u/macosvm/releases/download/0.2-3/macosvm-0.2-3-darwin21.tar.gz' "$stage/macosvm.tar.gz"
check_sha256 777ec90adc1ba5a7c705ad0d5ce021a71a6f4c707f2f8c0b307fd07d6f9a8c6f "$stage/macosvm.tar.gz" || fail 'macosvm checksum mismatch.'
mkdir -p "$stage/tools"
tar -xzf "$stage/macosvm.tar.gz" -C "$stage/tools" macosvm
chmod 700 "$stage/tools/macosvm"
ipsw="$stage/UniversalMac_13.6_22G120_Restore.ipsw"
fetch 'https://updates.cdn-apple.com/2023FallFCS/fullrestores/042-55833/C0830847-A2F8-458F-B680-967991820931/UniversalMac_13.6_22G120_Restore.ipsw' "$ipsw"
[ "$(shasum -a 1 "$ipsw" | awk '{print $1}')" = a1675f2c8412122a5e796981571b0269a966708e ] || fail 'Ventura image checksum mismatch.'
printf '\n4/6 Restore guest and verify native startup\n'
guest="$stage/guest"
if [ ! -f "$stage/restore-complete" ]; then
  if [ -d "$guest" ]; then
    fail 'A previous restore was interrupted. Preserve guest/ for diagnosis; rename it to guest-incomplete, then rerun. It will not be overwritten.'
  fi
  mkdir "$guest"
  printf 'Restoring guest; diagnostic log: %s/restore.log\n' "$stage"
  # Relative paths make vm.json portable to the Linux return bundle.
  (cd "$guest" && "$stage/tools/macosvm" --disk disk.img,size=64g --aux aux.img --restore "$ipsw" --net nat --mac 52:54:00:76:61:70 -c 4 -r 8g vm.json) > "$stage/restore.log" 2>&1 || fail "Restore failed. See $stage/restore.log"
  touch "$stage/restore-complete"
fi
for name in disk.img aux.img vm.json; do
  [ -s "$guest/$name" ] || fail "Restored guest missing $name."
done
# Validate the actual generated identity and disk links, not just file presence.
validate_config() {
  /usr/bin/osascript -l JavaScript - "$1" <<'JXA'
ObjC.import('Foundation');
function run(args) {
  const raw = $.NSString.stringWithContentsOfFileEncodingError(args[0], $.NSUTF8StringEncoding, null);
  const c = JSON.parse(ObjC.unwrap(raw));
  if (!c.machineId || !c.hardwareModel || !Array.isArray(c.storage)) throw Error('Missing guest identity');
  const disks = c.storage.filter(x => x.type === 'disk');
  const aux = c.storage.filter(x => x.type === 'aux');
  if (disks.length !== 1 || aux.length !== 1 || disks[0].file !== 'disk.img' || aux[0].file !== 'aux.img') throw Error('Guest storage links are not portable');
  return 'Guest identity and relative storage links verified';
}
JXA
}
validate_config "$guest/vm.json" || fail 'Guest configuration invalid.'
[ "$(stat -f %z "$guest/disk.img")" -eq 68719476736 ] || fail 'Guest disk is not the requested 64 GiB.'
[ "$(stat -f %z "$guest/aux.img")" -gt 16384 ] || fail 'Guest AUX is too small.'
if [ ! -f "$stage/native-startup-confirmed" ]; then
  printf '%s\n' 'The guest will open. Wait for its graphical Setup Assistant.'
  printf '%s\n' 'Do not sign into Apple in this guest. Use the guest Apple menu → Shut Down.'
  printf '%s\n' 'Closing its window does NOT stop the VM; shut down inside the guest.'
  (cd "$guest" && "$stage/tools/macosvm" -g vm.json) > "$stage/native-startup.log" 2>&1 || fail 'Native guest startup failed; see native-startup.log.'
  read -r -p 'Did graphical Setup Assistant appear, followed by guest Shut Down? Type YES: ' answer
  [ "$answer" = YES ] || fail 'Native startup is not confirmed. Rerun to retry it.'
  touch "$stage/native-startup-confirmed"
fi
dd if="$guest/aux.img" of="$guest/aux.img.trimmed" bs=16384 skip=1 2>/dev/null
[ "$(stat -f %z "$guest/aux.img.trimmed")" -eq $(( $(stat -f %z "$guest/aux.img") - 16384 )) ] || fail 'Trimmed AUX size mismatch.'
printf '\n5/6 Private bundle, checksums and shared-volume transfer\n'
# A failed retry must never leave an old completion marker looking current.
mkdir -p "$bundle"
rm -f "$bundle/COLLECTION-COMPLETE.txt"
cat > "$stage/RETURN-TO-LINUX.txt" <<'TXT'
Private collection: keep outside Git and chat.
Bubbles: use mac-hardware.txt with Local Mac hardware; log in only on Linux.
Machine Lab: keep firmware/, guest/, tools/, IPSW and provenance together.
Use guest/ as MACOS_DIR for our read-only readiness helper.
Native GUI startup was operator-confirmed. Linux KVM boot, firmware compatibility,
iMessage activation, and dock save/restore remain unqualified.
The native booter can differ from the published M2 Pro/Ventura proof.
COLLECTION-COMPLETE certifies collection and transfer only, not Linux boot.
TXT
files=(Collect-Everything.command mac-hardware.txt HOST-VERSION.txt FIRMWARE-COMPATIBILITY.txt RETURN-TO-LINUX.txt firmware/AVPBooter.vmapple2.bin guest/disk.img guest/aux.img guest/aux.img.trimmed guest/vm.json tools/macosvm UniversalMac_13.6_22G120_Restore.ipsw macosvm.tar.gz Mac.Hardware.Info.zip restore.log native-startup.log restore-complete native-startup-confirmed)
# Including the 64 GiB disk makes both checksum passes slow; this is intentional.
shasum -a 256 "${files[@]}" > SHA256SUMS
for name in "${files[@]}" SHA256SUMS; do
  mkdir -p "$bundle/$(dirname "$name")"
  rsync -a --progress "$stage/$name" "$bundle/$name"
done
printf '\n6/6 Re-read every transferred file and verify its checksum\n'
(cd "$bundle" && shasum -a 256 -c SHA256SUMS) || fail 'Shared-volume copy did not verify. Keep macOS running and rerun.'
valid_hardware "$bundle/mac-hardware.txt" || fail 'Transferred hardware code invalid.'
validate_config "$bundle/guest/vm.json" || fail 'Transferred guest config invalid.'
sync
cat > "$bundle/COLLECTION-COMPLETE.txt" <<TXT
Collection and shared-volume transfer verified: $(date -u '+%Y-%m-%dT%H:%M:%SZ')
Native host macOS: $version
All manifest SHA-256 checks passed after transfer.
Native guest graphical startup and shutdown confirmed by owner.
Linux KVM boot and Bubbles activation remain to be tested.
TXT
sync
printf '\nCOLLECTION COMPLETE — ready to return to Linux. Eject the drive first if it is external.\n'
printf 'Verified bundle: %s\n' "$bundle"
printf '%s\n' 'This certifies file collection, native startup and transfer. Linux compatibility is still unverified.'
printf '%s\n' 'Keep the macOS staging copy until Linux activation and VM import are checked.'
open "$bundle"
read -r -p 'Press Return to finish (this script does not reboot): ' _
