#!/usr/bin/env bash
# Install the Disk Space app (Rust + Granite 7) and its launcher from this checkout.
# Development route; customers get it from the signed repository once it ships as a package.
#
#   bash mseries/diskspace-app/install.sh                 build, then install under /usr/local (asks for sudo)
#   bash mseries/diskspace-app/install.sh --uninstall
#   DESTDIR=/tmp/root bash mseries/diskspace-app/install.sh --no-build   stage into a directory (CI uses this)
#
# The app only reads the filesystem and removes what the person selects, as that person. It installs no
# service and no policy, and restarts nothing (no Wingpanel, dock or compositor). A new launcher may show a
# generic icon until the dock next restarts.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
dest=${DESTDIR:-}
prefix=${PREFIX:-/usr/local}
launcher=/usr/share/applications/org.techmore.mseries.DiskSpace.desktop

as_root() { if [[ -n $dest || $EUID -eq 0 ]]; then "$@"; else sudo "$@"; fi; }

if [[ ${1:-} == --uninstall ]]; then
  as_root rm -f "$dest$prefix/bin/mseries-diskspace" "$dest$launcher"
  echo "Removed Disk Space."
  exit 0
fi

target=${CARGO_TARGET_DIR:-$repo/rust/target}
if [[ ${1:-} != --no-build ]]; then
  CARGO_TARGET_DIR=$target cargo build --locked --release --manifest-path "$repo/rust/Cargo.toml" -p mseries-diskspace
fi
[[ -x $target/release/mseries-diskspace ]] || { echo "error: $target/release/mseries-diskspace is missing; build first" >&2; exit 1; }

as_root install -Dm755 "$target/release/mseries-diskspace" "$dest$prefix/bin/mseries-diskspace"
as_root install -Dm644 "$repo/rust/mseries-diskspace/data/org.techmore.mseries.DiskSpace.desktop" "$dest$launcher"
echo "Installed Disk Space to $dest$prefix/bin/mseries-diskspace. Launch it from Applications, or run: mseries-diskspace [FOLDER]"
