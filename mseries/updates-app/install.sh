#!/usr/bin/env bash
# Install the Software Updates app (Rust + Granite 7) and its polkit action from this checkout.
# Development route; customers get it from the signed repository once it ships as a package.
#
#   bash mseries/updates-app/install.sh                 build, then install under /usr/local (asks for sudo)
#   bash mseries/updates-app/install.sh --uninstall
#   DESTDIR=/tmp/root bash mseries/updates-app/install.sh --no-build   stage into a directory (CI uses this)
#
# The app only reads `mseries-update status --json`; every change goes through pkexec to the helper
# under the polkit action org.techmore.mseries.update. Needs mseries-update installed
# (`sudo bash mseries/update/install.sh`). Nothing is enabled or started, and no Wingpanel, dock or
# compositor restart happens. A new launcher may show a generic icon until the dock next restarts.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
dest=${DESTDIR:-}
prefix=${PREFIX:-/usr/local}
libexec=$prefix/libexec/mseries-updates
helper=$libexec/mseries-update-helper
policy=/usr/share/polkit-1/actions/org.techmore.mseries.update.policy

as_root() { if [[ -n $dest || $EUID -eq 0 ]]; then "$@"; else sudo "$@"; fi; }

if [[ ${1:-} == --uninstall ]]; then
  as_root rm -f "$dest$prefix/bin/mseries-updates" "$dest$helper" "$dest$policy" \
    "$dest/usr/share/applications/org.techmore.mseries.Updates.desktop"
  as_root rmdir "$dest$libexec" 2>/dev/null || true
  echo "Removed Software Updates. Update state (/var/lib/mseries-update) is untouched."
  exit 0
fi

target=${CARGO_TARGET_DIR:-$repo/rust/target}
if [[ ${1:-} != --no-build ]]; then
  # The helper path is fixed at build time so the app and the polkit action cannot disagree.
  MSERIES_UPDATE_HELPER=$helper CARGO_TARGET_DIR=$target \
    cargo build --locked --release --manifest-path "$repo/rust/Cargo.toml" -p mseries-updates -p mseries-update-core
fi
for bin in mseries-updates mseries-update-helper; do
  [[ -x $target/release/$bin ]] || { echo "error: $target/release/$bin is missing; build first" >&2; exit 1; }
done

data=$repo/rust/mseries-updates/data
as_root install -Dm0755 "$target/release/mseries-updates" "$dest$prefix/bin/mseries-updates"
# Runs as root through pkexec: root-owned, not writable by anyone else.
as_root install -Dm0755 -o root -g root "$target/release/mseries-update-helper" "$dest$helper" 2>/dev/null \
  || as_root install -Dm0755 "$target/release/mseries-update-helper" "$dest$helper"
sed "s|@HELPER@|$helper|" "$data/org.techmore.mseries.update.policy.in" > "${TMPDIR:-/tmp}/mseries-update.policy.$$"
as_root install -Dm0644 "${TMPDIR:-/tmp}/mseries-update.policy.$$" "$dest$policy"
rm -f "${TMPDIR:-/tmp}/mseries-update.policy.$$"
as_root install -Dm0644 "$data/org.techmore.mseries.Updates.desktop" "$dest/usr/share/applications/org.techmore.mseries.Updates.desktop"
echo "Installed Software Updates ($prefix/bin/mseries-updates). Run mseries-updates or open it from the launcher."
